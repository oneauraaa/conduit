mod agents;
mod chrome;
mod commands;
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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

    // Windows only — see the Cargo.toml note. Launching conduit again just
    // brings the running one back from the tray, which is what the user meant.
    #[cfg(target_os = "windows")]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            chrome::show_main(app);
        }));
    }

    builder
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
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

            // Hold-Escape panic stop. On macOS this needs Accessibility, which
            // the user may not have granted yet — retry once it appears rather
            // than losing the shortcut for the whole run.
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

            if let Some(main) = app.get_webview_window("main") {
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
