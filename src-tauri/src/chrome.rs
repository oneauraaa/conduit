//! Window management for the two pieces of chrome conduit shows over other
//! applications: the full-screen overlay (one per display) and the control pill.
//!
//! Both need native treatment Tauri doesn't expose, so this module reaches
//! through `ns_window()` / `hwnd()` for the last few properties.

use parking_lot::RwLock;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, Wry};

use crate::platform::screen;
use crate::platform::types::{Display, OwnWindows};

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
        //
        // Not on Linux, and not yet: tao implements this by combining an empty
        // input region onto the window's `GdkWindow`, which does not exist
        // until the widget is realized — and conduit builds all its chrome
        // hidden. It unwraps that `Option`, so calling this on an unshown
        // window aborts the process from inside the GTK main loop, where the
        // panic cannot even unwind into something legible. It is applied in
        // `show_control_chrome` instead, right after the window is shown.
        #[cfg(not(target_os = "linux"))]
        let _ = window.set_ignore_cursor_events(true);

        configure_overlay_native(&window, display.index);
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
fn configure_overlay_native(window: &tauri::WebviewWindow<Wry>, display_index: usize) {
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

    #[cfg(target_os = "linux")]
    linux_chrome::configure(window, linux_chrome::Role::Overlay, Some(display_index));

    #[cfg(not(target_os = "linux"))]
    let _ = display_index;
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

    #[cfg(target_os = "linux")]
    linux_chrome::configure(window, linux_chrome::Role::Pill, None);
}

/// Parks the pill just above whatever the Dock (or taskbar) leaves free.
///
/// On Wayland this is deliberately skipped: a client cannot position its own
/// window there, and the pill is a layer-shell surface whose placement comes
/// from its anchor and exclusive zone instead — which is strictly better, since
/// the compositor knows where the panel actually is and `pill_anchor` can only
/// guess. See `platform/linux/chrome`-side notes in `configure_pill_native`.
fn position_pill(window: &tauri::WebviewWindow<Wry>) {
    #[cfg(target_os = "linux")]
    if linux_chrome::active() {
        return;
    }

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
                let _ = w.show();
                // After `show`, never before: on Linux the window has no
                // `GdkWindow` to attach an empty input region to until it is
                // realized, and tao unwraps that. Harmless ordering on the
                // other two, where `create_chrome` already made it inert.
                let _ = w.set_ignore_cursor_events(true);
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
    match crate::platform::apps::own_windows() {
        OwnWindows::Rects(rects) => rects
            .iter()
            .any(|(rx, ry, rw, rh)| x >= *rx && x < rx + rw && y >= *ry && y < ry + rh),
        OwnWindows::AskTauri => tauri_rects(app)
            .iter()
            .any(|(rx, ry, rw, rh)| x >= *rx && x < rx + rw && y >= *ry && y < ry + rh),
        // Nothing can say where conduit is, so there is no rectangle to refuse.
        // Guarding a guessed one is not a weaker version of this protection, it
        // is a different bug: it refuses clicks the agent is entitled to make
        // while still leaving the real window exposed.
        OwnWindows::Unknown => {
            warn_unguarded();
            false
        }
    }
}

/// Conduit's own window rectangles as Tauri reports them.
///
/// Correct on macOS and Windows, where a process may ask the window server
/// where its own windows are. `platform::apps::own_windows` decides whether
/// this is trustworthy, so nothing here has to know which platform it is on.
fn tauri_rects(app: &AppHandle<Wry>) -> Vec<(f64, f64, f64, f64)> {
    let mut out = Vec::new();
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

        out.push((
            screen::physical_to_space(pos.x as f64, scale),
            screen::physical_to_space(pos.y as f64, scale),
            screen::physical_to_space(size.width as f64, scale),
            screen::physical_to_space(size.height as f64, scale),
        ));
    }
    out
}

/// Says once, loudly, that conduit's own controls are reachable by an agent.
///
/// Once rather than per call: this is consulted on every click, and a warning
/// per click would bury the log it is trying to be found in.
fn warn_unguarded() {
    static SAID: std::sync::Once = std::sync::Once::new();
    SAID.call_once(|| {
        tracing::warn!(
            "this compositor will not say where conduit's own windows are, so an agent's              clicks cannot be kept off conduit's controls by coordinate. kwin (plasma) and              hyprland both answer; on anything else, quit conduit from the tray rather than              leaving its window open while an agent drives."
        );
    });
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

/// The Wayland counterpart of the `NSWindow` and `HWND` blocks above.
///
/// ## Why layer-shell
///
/// Neither of the two things conduit's chrome needs is possible for an ordinary
/// Wayland client. A client cannot place its own window at a screen coordinate,
/// and it cannot ask to be above other applications — `xdg_toplevel` has no
/// concept of either, deliberately, because a window that could do both is a
/// phishing surface.
///
/// `zwlr_layer_shell_v1` is the protocol for surfaces that legitimately live
/// outside that model: panels, notification daemons, lock screens, and this.
/// It replaces position with *anchors* and z-order with a small set of named
/// layers, which is a better fit anyway — the overlay wants "cover this whole
/// output" rather than a rectangle, and the pill wants "just above the panel"
/// rather than a y coordinate the compositor would have to be asked for.
///
/// ## Why it is loaded by hand
///
/// `dlopen` rather than linking. conduit ships as a zip with no installer and
/// no dependency resolution, so a hard link against `libgtk-layer-shell.so`
/// would turn a missing optional library into a binary that does not start at
/// all. Loaded this way, a machine without it gets a working MCP server and no
/// glow, which is the right way round.
#[cfg(target_os = "linux")]
mod linux_chrome {
    use std::ffi::{CString, c_char, c_int, c_void};
    use std::sync::OnceLock;

    use gtk::glib::translate::ToGlibPtr;
    use gtk::prelude::*;
    use tauri::{WebviewWindow, Wry};

    /// `GTK_LAYER_SHELL_LAYER_OVERLAY` — above panels and full-screen windows.
    const LAYER_OVERLAY: c_int = 3;

    /// `GtkLayerShellEdge`.
    const EDGE_LEFT: c_int = 0;
    const EDGE_RIGHT: c_int = 1;
    const EDGE_TOP: c_int = 2;
    const EDGE_BOTTOM: c_int = 3;

    /// `GtkLayerShellKeyboardMode`.
    const KEYBOARD_NONE: c_int = 0;
    const KEYBOARD_ON_DEMAND: c_int = 2;

    #[derive(Clone, Copy)]
    pub enum Role {
        Overlay,
        Pill,
    }

    // Named rather than written inline at each `transmute`, so the signature a
    // symbol is being cast to is stated once and is checkable against
    // gtk-layer-shell's header. An unannotated `transmute` in FFI infers
    // whatever the destination field happens to be, which is exactly how a
    // wrong signature gets in.
    type Window1 = unsafe extern "C" fn(*mut c_void);
    type WindowInt = unsafe extern "C" fn(*mut c_void, c_int);
    type WindowIntInt = unsafe extern "C" fn(*mut c_void, c_int, c_int);
    type WindowPtr = unsafe extern "C" fn(*mut c_void, *mut c_void);
    type WindowStr = unsafe extern "C" fn(*mut c_void, *const c_char);

    struct Api {
        init_for_window: Window1,
        set_layer: WindowInt,
        set_anchor: WindowIntInt,
        set_exclusive_zone: WindowInt,
        set_keyboard_mode: WindowInt,
        set_margin: WindowIntInt,
        set_monitor: WindowPtr,
        set_namespace: WindowStr,
    }

    // SAFETY: these are plain function pointers into a library that stays
    // loaded for the life of the process. They are only ever called on the main
    // thread, where GTK requires them to be.
    unsafe impl Send for Api {}
    unsafe impl Sync for Api {}

    static API: OnceLock<Option<Api>> = OnceLock::new();

    fn api() -> Option<&'static Api> {
        API.get_or_init(load).as_ref()
    }

    /// Whether layer-shell is usable, so the rest of `chrome.rs` knows whether
    /// its own positioning still applies.
    pub fn active() -> bool {
        api().is_some()
    }

    fn load() -> Option<Api> {
        // An escape hatch for a compositor whose layer-shell implementation
        // misbehaves. conduit still runs without it — the glow and the pill
        // just land wherever the compositor decides — so this is a far better
        // answer for a user than "it crashes on my machine".
        if std::env::var_os("CONDUIT_NO_LAYER_SHELL").is_some() {
            tracing::info!("layer-shell disabled by CONDUIT_NO_LAYER_SHELL");
            return None;
        }

        // Wayland only. Under X11 the library loads but its calls abort the
        // process on the first surface, and X11 does not need it — plain
        // override-redirect positioning works there.
        if !crate::platform::permissions::wayland() {
            return None;
        }

        // The versioned soname first: it is what a distribution package
        // installs, while the bare `.so` symlink comes from the -dev package
        // and is often absent on a user's machine.
        let handle = ["libgtk-layer-shell.so.0", "libgtk-layer-shell.so"]
            .iter()
            .find_map(|name| {
                let c = CString::new(*name).ok()?;
                // SAFETY: a valid NUL-terminated path and a documented flag.
                let h = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_LAZY) };
                (!h.is_null()).then_some(h)
            });

        let Some(handle) = handle else {
            tracing::warn!(
                "libgtk-layer-shell is not installed, so the overlay and pill \
                 cannot be placed over other windows. install gtk-layer-shell \
                 to get them back; everything else works without it."
            );
            return None;
        };

        // SAFETY: each symbol is looked up by its documented name and
        // transmuted to that function's documented signature. A missing symbol
        // yields null and aborts the whole load rather than being called.
        unsafe {
            let sym = |name: &str| -> Option<*mut c_void> {
                let c = CString::new(name).ok()?;
                let p = libc::dlsym(handle, c.as_ptr());
                (!p.is_null()).then_some(p)
            };

            Some(Api {
                init_for_window: std::mem::transmute::<*mut c_void, Window1>(sym(
                    "gtk_layer_init_for_window",
                )?),
                set_layer: std::mem::transmute::<*mut c_void, WindowInt>(sym(
                    "gtk_layer_set_layer",
                )?),
                set_anchor: std::mem::transmute::<*mut c_void, WindowIntInt>(sym(
                    "gtk_layer_set_anchor",
                )?),
                set_exclusive_zone: std::mem::transmute::<*mut c_void, WindowInt>(sym(
                    "gtk_layer_set_exclusive_zone",
                )?),
                set_keyboard_mode: std::mem::transmute::<*mut c_void, WindowInt>(sym(
                    "gtk_layer_set_keyboard_mode",
                )?),
                set_margin: std::mem::transmute::<*mut c_void, WindowIntInt>(sym(
                    "gtk_layer_set_margin",
                )?),
                set_monitor: std::mem::transmute::<*mut c_void, WindowPtr>(sym(
                    "gtk_layer_set_monitor",
                )?),
                set_namespace: std::mem::transmute::<*mut c_void, WindowStr>(sym(
                    "gtk_layer_set_namespace",
                )?),
            })
        }
    }

    pub fn configure(window: &WebviewWindow<Wry>, role: Role, display_index: Option<usize>) {
        let Some(api) = api() else { return };
        let Ok(gtk_window) = window.gtk_window() else {
            tracing::warn!("no gtk window behind {}", window.label());
            return;
        };

        let raw: *mut c_void = {
            let as_window: &gtk::Window = gtk_window.upcast_ref();
            let ptr: *mut gtk::ffi::GtkWindow = as_window.to_glib_none().0;
            ptr.cast()
        };

        // SAFETY: `raw` is a live GtkWindow for as long as the Tauri window is,
        // and every call below is the documented use of that pointer. GTK
        // requires the main thread, which is where `create_chrome` runs.
        unsafe {
            // Must come before anything else, and before the window is first
            // mapped — which is why every conduit window is built with
            // `.visible(false)` and shown later.
            (api.init_for_window)(raw);

            // The namespace is what a compositor matches window rules against,
            // so a user who wants to special-case conduit's glow has a handle.
            if let Ok(ns) = CString::new("conduit") {
                (api.set_namespace)(raw, ns.as_ptr());
            }

            (api.set_layer)(raw, LAYER_OVERLAY);

            match role {
                Role::Overlay => {
                    // Anchored to all four edges, which is how layer-shell
                    // spells "fill this output" — there is no set_size for a
                    // layer surface, and the compositor resizes it on hotplug.
                    for edge in [EDGE_LEFT, EDGE_RIGHT, EDGE_TOP, EDGE_BOTTOM] {
                        (api.set_anchor)(raw, edge, 1);
                    }
                    // -1 means "ignore other surfaces' exclusive zones", so the
                    // glow reaches under the panel instead of stopping at it.
                    (api.set_exclusive_zone)(raw, -1);
                    // The overlay must never take a keystroke from the app
                    // being driven.
                    (api.set_keyboard_mode)(raw, KEYBOARD_NONE);

                    if let Some(monitor) = display_index.and_then(gdk_monitor) {
                        (api.set_monitor)(raw, monitor);
                    }
                }
                Role::Pill => {
                    // Bottom edge only. Leaving left and right unanchored is
                    // what centres it horizontally.
                    (api.set_anchor)(raw, EDGE_BOTTOM, 1);
                    // 0, not -1: the pill should sit *above* the panel, and
                    // respecting the panel's exclusive zone is what puts it
                    // there without conduit having to know the panel's height.
                    (api.set_exclusive_zone)(raw, 0);
                    (api.set_margin)(raw, EDGE_BOTTOM, 8);
                    // On demand, not none: the stop button has to be clickable,
                    // but the pill must not steal focus by appearing.
                    (api.set_keyboard_mode)(raw, KEYBOARD_ON_DEMAND);
                }
            }
        }
    }

    /// The `GdkMonitor` for a display index, matching `screen::displays` order.
    fn gdk_monitor(index: usize) -> Option<*mut c_void> {
        let display = gtk::gdk::Display::default()?;

        // `screen::displays` sorts top-to-bottom, left-to-right; GDK's own
        // index order is arbitrary. Sorting the same way here is what keeps
        // overlay N on the monitor conduit calls display N.
        let mut monitors: Vec<(i32, i32, gtk::gdk::Monitor)> = (0..display.n_monitors())
            .filter_map(|i| display.monitor(i))
            .map(|m| {
                let g = m.geometry();
                (g.y(), g.x(), m)
            })
            .collect();
        monitors.sort_by_key(|(y, x, _)| (*y, *x));

        let monitor = monitors.into_iter().nth(index)?.2;
        let ptr: *mut gtk::gdk::ffi::GdkMonitor = monitor.to_glib_none().0;
        Some(ptr.cast())
    }
}
