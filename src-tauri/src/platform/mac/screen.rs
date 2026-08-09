//! Display geometry.
//!
//! Two coordinate spaces matter here and mixing them up is the classic source
//! of "the cursor is on the wrong monitor" bugs:
//!
//!   - **Cocoa** (`NSScreen`): origin bottom-left of the primary display, y up.
//!   - **Quartz** (`CGEvent`, `CGDisplay`): origin top-left of the primary
//!     display, y down.
//!
//! conduit speaks Quartz everywhere — it's what the MCP tools accept, what
//! screenshots are indexed in, and what CGEvent expects. Cocoa values are
//! converted the moment they're read.

use objc2_app_kit::NSScreen;
use objc2_foundation::MainThreadMarker;

use crate::platform::types::Display;

/// The full height of the primary display, which is the pivot for flipping
/// between Cocoa and Quartz y-coordinates.
fn primary_height(mtm: MainThreadMarker) -> f64 {
    NSScreen::screens(mtm)
        .iter()
        .next()
        .map(|s| s.frame().size.height)
        .unwrap_or(0.0)
}

/// Every attached display, in Quartz coordinates. Index 0 is always primary.
///
/// Returns empty off the main thread: `NSScreen` is not safe to read anywhere
/// else. Callers reach this through `chrome::refresh_displays`, which already
/// runs on main and caches the result for the tokio workers that need it.
pub fn displays() -> Vec<Display> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Vec::new();
    };
    let flip = primary_height(mtm);

    NSScreen::screens(mtm)
        .iter()
        .enumerate()
        .map(|(index, screen)| {
            let f = screen.frame();
            Display {
                index,
                x: f.origin.x,
                // Cocoa gives the bottom edge measured up from the primary's
                // bottom; Quartz wants the top edge measured down from its top.
                y: flip - (f.origin.y + f.size.height),
                width: f.size.width,
                height: f.size.height,
                scale: screen.backingScaleFactor(),
                primary: index == 0,
            }
        })
        .collect()
}

/// Where the control pill should sit: centred on the primary display's usable
/// width, just above whatever the Dock leaves free.
///
/// `visibleFrame` rather than `frame` is the whole point — it already excludes
/// the Dock and menu bar, so this lands correctly whether the Dock is on the
/// bottom, the left, or the right, and it follows the Dock when it auto-hides.
pub fn pill_anchor(pill_w: f64, pill_h: f64) -> (f64, f64) {
    let Some(mtm) = MainThreadMarker::new() else {
        return (0.0, 0.0);
    };
    let Some(screen) = NSScreen::screens(mtm).iter().next() else {
        return (0.0, 0.0);
    };

    let flip = screen.frame().size.height;
    let vis = screen.visibleFrame();

    let x = vis.origin.x + (vis.size.width - pill_w) / 2.0;
    // Bottom of the visible area, converted to Quartz, minus the pill's height
    // and a small breathing gap.
    let bottom_quartz = flip - vis.origin.y;
    let y = bottom_quartz - pill_h - 8.0;

    (x, y)
}

/* ── bridging conduit's coordinate space to Tauri's ──────────── */

/// Converts a point in conduit's space to the units Tauri positions windows in.
///
/// macOS points are logical, so this is a straight `Logical`. The Windows
/// backend returns `Physical` instead — the pair of them is what keeps
/// `chrome.rs` free of `#[cfg]`.
pub fn tauri_position(x: f64, y: f64) -> tauri::Position {
    tauri::Position::Logical(tauri::LogicalPosition::new(x, y))
}

pub fn tauri_size(w: f64, h: f64) -> tauri::Size {
    tauri::Size::Logical(tauri::LogicalSize::new(w, h))
}

/// Converts a value Tauri reports in physical units back to conduit's space.
///
/// Tauri's `outer_position`/`outer_size` are always physical; macOS coordinates
/// are logical points, so they divide by the window's scale factor.
pub fn physical_to_space(v: f64, scale: f64) -> f64 {
    v / scale
}

/// The screenshot scale to use when the caller didn't ask for one.
///
/// Screens are physically large and models are billed per pixel, so the default
/// means "logical points, not backing pixels" — a Retina display is captured at
/// its point size. macOS bounds are already in points, so this is 1.0; the
/// Windows backend divides by the monitor's DPI scale to reach the same place.
pub fn default_capture_scale(_display: &Display) -> f64 {
    1.0
}
