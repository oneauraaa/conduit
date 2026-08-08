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
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Display {
    pub index: usize,
    /// Top-left origin, Quartz space, logical points.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
    pub primary: bool,
}

impl Display {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

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
pub fn displays(mtm: MainThreadMarker) -> Vec<Display> {
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
pub fn pill_anchor(mtm: MainThreadMarker, pill_w: f64, pill_h: f64) -> (f64, f64) {
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

pub fn display_at(displays: &[Display], x: f64, y: f64) -> usize {
    displays
        .iter()
        .find(|d| d.contains(x, y))
        .map(|d| d.index)
        .unwrap_or(0)
}
