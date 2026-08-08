//! Window management for the two pieces of chrome conduit shows over other
//! applications: the full-screen overlay (one per display) and the control pill.
//!
//! Both need native NSWindow treatment that Tauri doesn't expose, so this
//! module reaches through `ns_window()` for the last few properties.

use std::ffi::c_void;

use parking_lot::RwLock;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, Wry};

use crate::mac::screen::{self, Display};

/// Height of the pill window. Tall enough to hold the mode dropdown and the
/// approval card without clipping, since the window can't grow at runtime
/// without a visible resize.
const PILL_W: f64 = 420.0;
const PILL_H: f64 = 260.0;

/// Display geometry, cached because NSScreen may only be read on the main
/// thread and tool calls arrive on tokio workers.
static DISPLAYS: RwLock<Vec<Display>> = RwLock::new(Vec::new());

pub fn cached_displays() -> Vec<Display> {
    DISPLAYS.read().clone()
}

/// Re-reads display geometry. Must be called on the main thread.
pub fn refresh_displays() {
    let Some(mtm) = objc2_foundation::MainThreadMarker::new() else {
        return;
    };
    *DISPLAYS.write() = screen::displays(mtm);
}

pub fn overlay_label(index: usize) -> String {
    format!("overlay-{index}")
}

/// Creates the overlay and pill windows, hidden. Called once at startup so the
/// first takeover doesn't pay webview startup cost.
pub fn create_chrome(app: &AppHandle<Wry>) -> tauri::Result<()> {
    refresh_displays();

    for display in cached_displays() {
        let label = overlay_label(display.index);
        if app.get_webview_window(&label).is_some() {
            continue;
        }

        let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("overlay.html".into()))
            .title("conduit overlay")
            .inner_size(display.width, display.height)
            .position(display.x, display.y)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(true)
            .focused(false)
            .visible(false)
            .resizable(false)
            .skip_taskbar(true)
            .visible_on_all_workspaces(true)
            .build()?;

        // Belt and braces: Tauri's flag plus the native one. There are open
        // reports of full-screen transparent windows still swallowing events on
        // Tahoe with only the former set.
        let _ = window.set_ignore_cursor_events(true);
        configure_overlay_native(&window);
        // Bound to locals first: `display` is also a tracing field helper, so
        // `display.width` inside the macro resolves to the wrong thing.
        let (w, h) = (display.width, display.height);
        tracing::info!(label = %label, width = w, height = h, "overlay window created");
    }

    if app.get_webview_window("pill").is_none() {
        let window = WebviewWindowBuilder::new(app, "pill", WebviewUrl::App("pill.html".into()))
            .title("conduit")
            .inner_size(PILL_W, PILL_H)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(true)
            .focused(false)
            .visible(false)
            .resizable(false)
            .skip_taskbar(true)
            .visible_on_all_workspaces(true)
            .build()?;

        configure_pill_native(&window);
        position_pill(&window);
        tracing::info!("pill window created");
    }

    Ok(())
}

/// Raises the overlay above everything and makes it inert.
fn configure_overlay_native(window: &tauri::WebviewWindow<Wry>) {
    #[cfg(target_os = "macos")]
    unsafe {
        use objc2::msg_send;
        use objc2::runtime::AnyObject;

        let Ok(ptr) = window.ns_window() else { return };
        let ns_window = ptr as *mut AnyObject;
        if ns_window.is_null() {
            return;
        }

        // NSScreenSaverWindowLevel is 1000; one above keeps the glow over
        // full-screen apps and the Dock.
        let _: () = msg_send![ns_window, setLevel: 1001_isize];
        let _: () = msg_send![ns_window, setIgnoresMouseEvents: true];
        // CanJoinAllSpaces | Stationary | FullScreenAuxiliary
        let behavior: usize = (1 << 0) | (1 << 4) | (1 << 8);
        let _: () = msg_send![ns_window, setCollectionBehavior: behavior];
        let _: () = msg_send![ns_window, setHasShadow: false];
    }
}

/// The pill floats high but stays interactive.
fn configure_pill_native(window: &tauri::WebviewWindow<Wry>) {
    #[cfg(target_os = "macos")]
    unsafe {
        use objc2::msg_send;
        use objc2::runtime::AnyObject;

        let Ok(ptr) = window.ns_window() else { return };
        let ns_window = ptr as *mut AnyObject;
        if ns_window.is_null() {
            return;
        }

        // Above the overlay, so the stop button is never covered by the glow.
        let _: () = msg_send![ns_window, setLevel: 1002_isize];
        let behavior: usize = (1 << 0) | (1 << 4) | (1 << 8);
        let _: () = msg_send![ns_window, setCollectionBehavior: behavior];
        let _: () = msg_send![ns_window, setHasShadow: false];
    }
}

/// Parks the pill just above whatever the Dock leaves free.
fn position_pill(window: &tauri::WebviewWindow<Wry>) {
    let Some(mtm) = objc2_foundation::MainThreadMarker::new() else {
        return;
    };
    let (x, y) = screen::pill_anchor(mtm, PILL_W, PILL_H);
    let _ = window.set_position(tauri::LogicalPosition::new(x, y));
}

/// Shows the glow and the pill. Safe to call from any thread.
///
/// `state` is re-emitted *after* the windows are up. The overlay and pill are
/// created hidden at startup, and a hidden webview can still be mid-load when
/// the first `control:state` goes out — it would miss that one-shot event and
/// sit there rendering nothing while cursor updates streamed past it.
pub fn show_control_chrome(app: &AppHandle<Wry>, state: crate::state::ControlState) {
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        refresh_displays();

        for display in cached_displays() {
            if let Some(w) = app.get_webview_window(&overlay_label(display.index)) {
                let _ = w.set_ignore_cursor_events(true);
                let _ = w.show();
            }
        }
        match app.get_webview_window("pill") {
            Some(pill) => {
                position_pill(&pill);
                if let Err(e) = pill.show() {
                    tracing::warn!("could not show the pill: {e}");
                }
                tracing::info!(pos = ?pill.outer_position().ok(), "pill shown");
            }
            None => tracing::warn!("pill window is missing"),
        }

        crate::mac::input::hide_system_cursor();

        let _ = tauri::Emitter::emit(&app, "control:state", &state);
    });
}

pub fn hide_control_chrome(app: &AppHandle<Wry>) {
    let app = app.clone();
    let _ = app.clone().run_on_main_thread(move || {
        for display in cached_displays() {
            if let Some(w) = app.get_webview_window(&overlay_label(display.index)) {
                let _ = w.hide();
            }
        }
        if let Some(pill) = app.get_webview_window("pill") {
            let _ = pill.hide();
        }

        crate::mac::input::show_system_cursor();
    });
}

/// Whether a screen point lands on one of conduit's own windows.
///
/// This closes a real hole. `list_windows` already hides conduit from agents,
/// but nothing stopped one from taking a screenshot, spotting the Tools tab or
/// the pill's mode dropdown, and clicking them by coordinate — an agent could
/// quietly promote itself to full access, or switch every tool back on after
/// the user turned them off. Input tools refuse any point inside these bounds,
/// so conduit's own controls stay reachable only by a human.
///
/// Coordinates are Quartz points, matching everything else in the tool surface.
pub fn point_hits_conduit(app: &AppHandle<Wry>, x: f64, y: f64) -> bool {
    for label in ["main", "pill"] {
        let Some(window) = app.get_webview_window(label) else {
            continue;
        };
        // A hidden window can't be clicked, so it can't be a target.
        if !window.is_visible().unwrap_or(false) {
            continue;
        }

        let (Ok(pos), Ok(size), Ok(scale)) = (
            window.outer_position(),
            window.outer_size(),
            window.scale_factor(),
        ) else {
            continue;
        };

        let left = pos.x as f64 / scale;
        let top = pos.y as f64 / scale;
        let right = left + size.width as f64 / scale;
        let bottom = top + size.height as f64 / scale;

        if x >= left && x < right && y >= top && y < bottom {
            return true;
        }
    }
    false
}

/// Brings the main window back from the tray.
pub fn show_main(app: &AppHandle<Wry>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn hide_main(app: &AppHandle<Wry>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

/// Raw pointer helper kept for readability at the call sites above.
#[allow(dead_code)]
fn as_object(ptr: *mut c_void) -> *mut objc2::runtime::AnyObject {
    ptr as *mut objc2::runtime::AnyObject
}
