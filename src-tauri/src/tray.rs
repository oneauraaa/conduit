//! The menu-bar item.
//!
//! This is the only place conduit can actually be quit. Closing the window
//! hides it, because the app *is* the MCP server — quitting from the `x` would
//! silently revoke every agent's access with no obvious cause.

use std::sync::OnceLock;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::state::{ServerStatus, Shared};

/// Held so the label can track the server's actual state. A menu item reading
/// "stop server" while the server is already stopped is worse than no item.
static TOGGLE_ITEM: OnceLock<MenuItem<Wry>> = OnceLock::new();

pub fn build(app: &AppHandle<Wry>) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "open conduit", true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "stop server", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "quit conduit", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;

    let menu = Menu::with_items(app, &[&open, &toggle, &sep, &quit])?;
    let _ = TOGGLE_ITEM.set(toggle);

    // The menu bar wants a monochrome glyph, not the app icon. Marking it a
    // template image lets macOS invert it for light and dark menu bars, and for
    // the highlighted state when the menu is open.
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray@2x.png"))
        .unwrap_or_else(|_| {
            app.default_window_icon()
                .cloned()
                .expect("conduit ships a window icon")
        });

    TrayIconBuilder::with_id("conduit")
        .icon(icon)
        .icon_as_template(true)
        .tooltip("conduit")
        .menu(&menu)
        // Left-click should toggle the window, so the menu must not open on it.
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "open" => crate::windows::show_main(app),
            "toggle" => {
                let state = app.state::<Shared>().inner().clone();
                let running = matches!(state.server().status, ServerStatus::Running);
                tauri::async_runtime::spawn(async move {
                    if running {
                        crate::mcp::server::stop(state).await;
                    } else {
                        crate::mcp::server::start(state).await;
                    }
                });
            }
            "quit" => {
                // Tear the listener down before exiting so the port is released
                // deterministically rather than at process teardown.
                let state = app.state::<Shared>().inner().clone();
                if let Some(cancel) = state.server_cancel.write().take() {
                    cancel.cancel();
                }
                crate::mac::input::show_system_cursor();
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                let visible = app
                    .get_webview_window("main")
                    .and_then(|w| w.is_visible().ok())
                    .unwrap_or(false);

                if visible {
                    crate::windows::hide_main(app);
                } else {
                    crate::windows::show_main(app);
                }
            }
        })
        .build(app)?;

    Ok(())
}

/// Keeps the tray's toggle label matching the server's real state.
pub fn sync_label(running: bool) {
    if let Some(item) = TOGGLE_ITEM.get() {
        let _ = item.set_text(if running { "stop server" } else { "start server" });
    }
}
