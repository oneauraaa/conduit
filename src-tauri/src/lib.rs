mod agents;
mod chrome;
mod commands;
mod hyprland;
mod mcp;
mod panic_stop;
mod platform;
mod state;
mod store;
mod tailscale;
mod tray;

use std::sync::Arc;

use tauri::{Manager, WindowEvent};

use state::{AppState, Shared};

/// The argument conduit's login entry passes to itself.
///
/// It is how the app tells "the system started me" from "a person started me".
/// A manual launch never carries it, so double-clicking conduit always shows a
/// window even when `start_hidden` is on — a launch that appears to do nothing
/// is worse than a window you have to dismiss.
const HIDDEN_FLAG: &str = "--hidden";

fn launched_at_login() -> bool {
    std::env::args().any(|arg| arg == HIDDEN_FLAG)
}

/// Turns off WebKitGTK's DMA-BUF renderer.
///
/// Without this, conduit dies at startup on KWin with
/// `wp_linux_drm_syncobj_surface_v1: explicit sync is used, but no acquire
/// point is set` — a protocol error the compositor answers by dropping the
/// connection, which surfaces as `Gdk-Message: Error 71 (Protocol error)` and
/// no window at all.
///
/// It is not conduit's bug and there is nothing to fix on this side: WebKitGTK
/// commits a buffer through the explicit-sync protocol without attaching an
/// acquire point, and KWin is right to refuse it. Every Tauri and GTK-webview
/// app on Wayland hits it. The SHM path this falls back to is slower, which for
/// four small mostly-static webviews is not a cost anyone can perceive.
///
/// Set before anything touches GTK, and only if the user has not chosen a value
/// — someone on a compositor where the DMA-BUF path works should be able to
/// keep it.
#[cfg(target_os = "linux")]
fn appease_webkit() {
    for key in ["WEBKIT_DISABLE_DMABUF_RENDERER"] {
        if std::env::var_os(key).is_none() {
            // SAFETY: single-threaded here — this runs as the first statement
            // of `run`, before the Tauri builder, any GTK call, or any thread
            // conduit spawns.
            unsafe { std::env::set_var(key, "1") };
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    appease_webkit();

    // Without a subscriber, every tracing::warn! in the app goes nowhere — which
    // is exactly the wrong thing when a window silently fails to create.
    // RUST_LOG overrides; default is warnings from conduit itself.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "conduit=info,warn".into()),
        )
        .with_target(false)
        .init();

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default();

    // Windows and Linux — see the Cargo.toml note. macOS gets this free from
    // Launch Services; neither of the other two has an equivalent rule, so
    // without it a second launch leaves a process that cannot bind the port and
    // shows a broken window. Because closing hides to the tray, they accumulate.
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            chrome::show_main(app);
        }));
    }

    builder
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        // `--hidden` is how the login entry tells the app it was started by the
        // system rather than by a person. A manual launch never carries it, so
        // double-clicking conduit always shows the window.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![HIDDEN_FLAG]),
        ))
        .invoke_handler(tauri::generate_handler![
            commands::get_server_state,
            commands::start_server,
            commands::stop_server,
            commands::restart_server,
            commands::set_port,
            commands::get_settings,
            commands::get_tool_catalog,
            commands::set_default_access,
            commands::set_tools_access,
            commands::set_tool_enabled,
            commands::set_cors_enabled,
            commands::set_cors_origins,
            commands::set_start_on_login,
            commands::set_start_hidden,
            commands::get_readiness,
            commands::request_accessibility,
            commands::request_screen_recording,
            commands::open_permission_settings,
            commands::relaunch_elevated,
            commands::get_control_state,
            commands::set_session_mode,
            commands::stop_control,
            commands::resume_control,
            commands::resolve_approval,
            commands::list_agents,
            commands::install_agent,
            commands::uninstall_agent,
            commands::get_tailscale_state,
            commands::get_hyprland_state,
            commands::enable_remote,
            commands::disable_remote,
            commands::regenerate_remote_token,
            commands::hide_to_tray,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let settings = store::load(&handle);
            let shared: Shared = Arc::new(AppState::new(handle.clone(), settings));
            app.manage(shared.clone());

            tray::build(&handle)?;

            // Logged once at startup because "conduit says it can't take
            // screenshots" is otherwise unanswerable from a bug report — and
            // has already been a wrong answer once, when a WinRT probe failed
            // for want of a COM apartment and was read as an old Windows.
            tracing::info!(readiness = ?platform::permissions::snapshot(), "readiness");

            // Wayland withholds input and capture behind a consent dialog that
            // cannot be made permanent, so it has to be asked for on every
            // launch. Asking *now* puts it beside the window that explains what
            // conduit is, rather than under whatever an agent is doing twenty
            // minutes in. No-op on the other two platforms.
            #[cfg(target_os = "linux")]
            platform::portal::warm_up();

            // Overlay and pill are built up front and kept hidden, so the first
            // takeover doesn't pay webview startup cost mid-action.
            if let Err(e) = chrome::create_chrome(&handle) {
                tracing::warn!("could not create the control chrome: {e}");
            }

            // The MCP server comes up with the app: that is the whole contract.
            // conduit open means agents can reach it, conduit quit means they
            // cannot.
            let boot = shared.clone();
            tauri::async_runtime::spawn(async move {
                mcp::server::start(boot).await;
            });

            // Extracting an app icon takes up to a second, and the Agents tab
            // needs several — warm them now so opening the tab is instant.
            // Must be the main thread: the macOS path is AppKit, which is not
            // safe to touch anywhere else.
            let icon_app = handle.clone();
            let _ = handle.run_on_main_thread(move || {
                let _ = icon_app;
                platform::appicon::warm_cache(&agents::icon_keys(), 64.0);
            });

            // Sharing is a persisted setting, so restore it on launch —
            // otherwise the Tailscale tab would claim to be sharing while no
            // listener existed behind the funnel.
            if shared.settings().remote_enabled {
                let resume = shared.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = mcp::server::start_remote(resume).await {
                        tracing::warn!("could not restore the sharing listener: {e}");
                    }
                });
            }

            // Hold-Escape panic stop. Every platform can withhold it: macOS
            // until Accessibility is granted, linux until the user is in the
            // `input` group. Retry rather than losing the shortcut for the whole
            // run — and until it lands, the pill's Stop button is the way out.
            let panic_state = shared.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    if panic_stop::install(panic_state.clone()) {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            });

            // Retires a session that has gone quiet, so a finished agent doesn't
            // leave the screen glowing indefinitely.
            let watchdog = shared.clone();
            tauri::async_runtime::spawn(async move {
                let mut tick = tokio::time::interval(std::time::Duration::from_secs(2));
                loop {
                    tick.tick().await;
                    if watchdog.is_idle() {
                        watchdog.end_control();
                    }
                }
            });

            // Re-assert the login entry. The setting is conduit's record of
            // what the user asked for; the registry key or LaunchAgent is the
            // OS's, and something else may have removed it since.
            //
            // In a development build this is where a stale entry written by an
            // earlier dev run gets torn out — see `apply_autostart` for why one
            // can only ever be broken. The setting survives, so a release build
            // registers the right binary here instead.
            if shared.settings().start_on_login {
                if let Err(e) = commands::apply_autostart(&handle, true) {
                    tracing::warn!("{e}");
                }
            }

            // The server is already starting above, whether or not a window is
            // shown — starting in the tray must not mean starting inert.
            if shared.settings().start_hidden && launched_at_login() {
                tracing::info!("started at login; staying in the tray");
            } else if let Some(main) = app.get_webview_window("main") {
                let _ = main.show();
                let _ = main.set_focus();
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            // Every close path — the custom `x`, cmd-W, the menu — funnels to
            // the tray rather than quitting. Quit is deliberately only in the
            // tray menu.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running conduit");
}
