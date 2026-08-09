//! Window management for the two pieces of chrome conduit shows over other
//! applications: the full-screen overlay (one per display) and the control pill.
//!
//! Both need native treatment Tauri doesn't expose, so this module reaches
//! through `ns_window()` / `hwnd()` for the last few properties.

use parking_lot::RwLock;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, Wry};

use crate::platform::screen;
use crate::platform::types::Display;

/// Size of the pill window. Tall and wide enough to hold the mode dropdown and
/// the approval card without clipping, since the window can't grow at runtime
/// without a visible resize.
///
/// The margin around the content is not slack — it is what the glow needs. The
/// pill draws its aura as a blurred element inset 12px beyond itself with a
/// 24px blur, so roughly 40px past the pill's own edge still has ink on it.
/// The window clips at its bounds, so anything tighter than that turns the
/// soft falloff into a straight edge and visible corners. 480×300 around a
/// 352px pill leaves 64px on each side and, with the layout's bottom padding,
/// about 44px underneath.
const PILL_W: f64 = 480.0;
const PILL_H: f64 = 300.0;

/// Display geometry, cached because NSScreen may only be read on the main
/// thread and tool calls arrive on tokio workers.
static DISPLAYS: RwLock<Vec<Display>> = RwLock::new(Vec::new());

pub fn cached_displays() -> Vec<Display> {
    DISPLAYS.read().clone()
}

/// Re-reads display geometry. Must be called on the main thread — see the
/// note on `platform::screen::displays`.
pub fn refresh_displays() {
    let fresh = screen::displays();
    // An empty read means "called off the main thread" on macOS, not "no
    // monitors attached". Keeping the previous geometry beats blanking it and
    // destroying every overlay.
    if !fresh.is_empty() {
        *DISPLAYS.write() = fresh;
    }
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

        // Size and position are set after the build rather than through the
        // builder, because the builder only speaks logical units and conduit's
        // display bounds are physical pixels on Windows. `tauri_size` and
        // `tauri_position` tag them with the unit each platform actually uses.
        let _ = window.set_size(screen::tauri_size(display.width, display.height));
        let _ = window.set_position(screen::tauri_position(display.x, display.y));

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

    #[cfg(target_os = "windows")]
    win_chrome::configure(window, win_chrome::Role::Overlay);
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

    #[cfg(target_os = "windows")]
    win_chrome::configure(window, win_chrome::Role::Pill);
}

/// Parks the pill just above whatever the Dock (or taskbar) leaves free.
fn position_pill(window: &tauri::WebviewWindow<Wry>) {
    let (x, y) = screen::pill_anchor(PILL_W, PILL_H);
    let _ = window.set_position(screen::tauri_position(x, y));
}

/// The Windows counterpart of the `NSWindow` block above.
///
/// Tauri already covers most of it — `set_ignore_cursor_events` maps to
/// `WS_EX_TRANSPARENT | WS_EX_LAYERED`, `always_on_top` to `WS_EX_TOPMOST`, and
/// `transparent` to `WS_EX_NOREDIRECTIONBITMAP`. What is left is the part with
/// no Tauri equivalent.
#[cfg(target_os = "windows")]
mod win_chrome {
    use tauri::{Wry, WebviewWindow};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, SetWindowDisplayAffinity, SetWindowLongPtrW,
        WDA_EXCLUDEFROMCAPTURE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };

    #[derive(Clone, Copy)]
    pub enum Role {
        Overlay,
        Pill,
    }

    pub fn configure(window: &WebviewWindow<Wry>, role: Role) {
        let Ok(hwnd) = window.hwnd() else { return };
        let hwnd = HWND(hwnd.0 as *mut _);

        unsafe {
            let mut ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);

            // `skip_taskbar` removes the taskbar button but leaves the window
            // in Alt-Tab. WS_EX_TOOLWINDOW is what actually takes it out of
            // both — otherwise the glow and the pill are tab stops.
            ex |= WS_EX_TOOLWINDOW.0 as isize;

            // The overlay must never take focus from the app being driven.
            // The pill must, or its buttons could not be clicked.
            if matches!(role, Role::Overlay) {
                ex |= WS_EX_NOACTIVATE.0 as isize;
            }

            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex);

            // Keep conduit's own chrome out of conduit's own screenshots. On
            // macOS ScreenCaptureKit gets this from the content filter; here
            // the window itself has to opt out, or every capture would show
            // the glow and the AI cursor drawn over the real screen — and the
            // agent would start reasoning about its own UI.
            let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
        }
    }
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
        // The pill is shown *after* the overlays on purpose. macOS keeps them
        // apart by window level (1002 over 1001), but Windows orders topmost
        // windows by which was raised most recently — show them the other way
        // round there and the glow covers the stop button.
        match app.get_webview_window("pill") {
            Some(pill) => {
                position_pill(&pill);
                if let Err(e) = pill.show() {
                    tracing::warn!("could not show the pill: {e}");
                }
                let _ = pill.set_always_on_top(true);
                tracing::info!(pos = ?pill.outer_position().ok(), "pill shown");
            }
            None => tracing::warn!("pill window is missing"),
        }

        crate::platform::input::hide_system_cursor();

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

        crate::platform::input::show_system_cursor();
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
/// Coordinates are in conduit's own space, matching the rest of the tool
/// surface. Tauri always reports window geometry in *physical* units, so the
/// conversion back has to go through `screen::physical_to_space` — macOS
/// divides by the scale factor to reach points, Windows is already there.
/// Hardcoding either one would size this box wrongly on the other platform,
/// and since this is what stops an agent clicking conduit's own Tools tab, a
/// mis-sized box is a hole rather than a cosmetic bug.
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

        let left = screen::physical_to_space(pos.x as f64, scale);
        let top = screen::physical_to_space(pos.y as f64, scale);
        let right = left + screen::physical_to_space(size.width as f64, scale);
        let bottom = top + screen::physical_to_space(size.height as f64, scale);

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
