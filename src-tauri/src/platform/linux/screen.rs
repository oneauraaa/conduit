//! Display geometry.
//!
//! ## The coordinate space
//!
//! Linux uses **logical pixels** in the compositor's layout space: origin
//! top-left of the primary output, y down, monitors to the left or above
//! carrying negative coordinates. That is the same shape as the macOS space and
//! unlike the Windows one, which is physical.
//!
//! Logical rather than physical because it is what the portal speaks. Absolute
//! pointer coordinates go to `NotifyPointerMotionAbsolute` in stream-relative
//! *logical* units, and stream sizes come back the same way. Reporting physical
//! pixels would mean converting at every input call site with a scale factor
//! that differs per monitor — exactly the class of bug the Windows notes warn
//! about, arrived at from the other direction.
//!
//! On a 200%-scaled 4K panel this means conduit reports a 1920×1080 display.
//! `Display.scale` carries the 2.0 so an agent can reason about sharpness, and
//! [`default_capture_scale`] leaves screenshots at logical size for the same
//! reason macOS does: models are billed per pixel.

use gtk::gdk;
use gtk::prelude::*;

use crate::platform::types::Display;

/// Every attached monitor, in the space described above. Index 0 is primary.
///
/// Returns empty when GDK has no display connection — off the main thread
/// before GTK is up, in practice. Callers reach this through
/// `chrome::refresh_displays`, which runs on main and caches the result for the
/// tokio workers that need it.
pub fn displays() -> Vec<Display> {
    let Some(gdk_display) = gdk::Display::default() else {
        return Vec::new();
    };

    let count = gdk_display.n_monitors();
    let mut out = Vec::new();

    for i in 0..count {
        let Some(monitor) = gdk_display.monitor(i) else {
            continue;
        };
        let geometry = monitor.geometry();
        out.push(Display {
            index: 0, // assigned below, after sorting
            x: geometry.x() as f64,
            y: geometry.y() as f64,
            width: geometry.width() as f64,
            height: geometry.height() as f64,
            scale: monitor.scale_factor() as f64,
            primary: monitor.is_primary(),
        });
    }

    // Top-to-bottom, left-to-right. This is the *same* order
    // `portal::parse_streams` sorts its streams into, and `capture` relies on
    // the two lining up by index — a stream and a display at the same index
    // must be the same monitor.
    out.sort_by(|a, b| {
        (a.y, a.x)
            .partial_cmp(&(b.y, b.x))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // GDK on Wayland does not always mark a primary monitor; nothing else in
    // conduit tolerates having none, so the first one takes the role.
    if !out.iter().any(|d| d.primary) {
        if let Some(first) = out.first_mut() {
            first.primary = true;
        }
    }

    for (index, display) in out.iter_mut().enumerate() {
        display.index = index;
    }
    out
}

/// Where the control pill should sit: centred on the primary display, just
/// above whatever the panel leaves free.
///
/// Unlike macOS's `visibleFrame` there is no portable "usable area" on Wayland
/// — panels are ordinary layer-shell clients and no protocol reports their
/// extent to another client. [`PANEL_ALLOWANCE`] is the honest approximation:
/// enough to clear a default Plasma panel, and harmless if there is none, since
/// the pill's own window is mostly transparent margin anyway.
pub fn pill_anchor(pill_w: f64, pill_h: f64) -> (f64, f64) {
    /// A default Plasma panel is 44–56px depending on scale; this clears it
    /// with a little air.
    const PANEL_ALLOWANCE: f64 = 68.0;

    let displays = displays();
    let Some(primary) = displays.iter().find(|d| d.primary).or_else(|| displays.first()) else {
        return (0.0, 0.0);
    };

    let x = primary.x + (primary.width - pill_w) / 2.0;
    let y = primary.y + primary.height - pill_h - PANEL_ALLOWANCE;
    (x, y)
}

/* ── bridging conduit's coordinate space to Tauri's ──────────── */

/// Converts a point in conduit's space to the units Tauri positions windows in.
///
/// Logical, like macOS — see the module header.
pub fn tauri_position(x: f64, y: f64) -> tauri::Position {
    tauri::Position::Logical(tauri::LogicalPosition::new(x, y))
}

pub fn tauri_size(w: f64, h: f64) -> tauri::Size {
    tauri::Size::Logical(tauri::LogicalSize::new(w, h))
}

/// Converts a value Tauri reports in physical units back to conduit's space.
///
/// Tauri's `outer_position`/`outer_size` are always physical; this space is
/// logical, so they divide by the window's scale factor.
pub fn physical_to_space(v: f64, scale: f64) -> f64 {
    v / scale
}

/// The screenshot scale to use when the caller didn't ask for one.
///
/// 1.0: bounds are already logical, so this means "logical pixels, not backing
/// pixels" — the same place macOS lands, reached the same way.
pub fn default_capture_scale(_display: &Display) -> f64 {
    1.0
}
