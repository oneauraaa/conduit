//! Screen capture via ScreenCaptureKit.
//!
//! The old Quartz path (`CGDisplayCreateImage`, `CGWindowListCreateImage`) is
//! not an option on Tahoe — it returns desktop wallpaper with no application
//! windows in it, which would silently give the agent a blank-looking screen
//! rather than an error. ScreenCaptureKit is the only supported route.
//!
//! Capture is synchronous and blocking, so callers run it on a blocking thread.

use image::{ImageEncoder, codecs::png::PngEncoder};
use screencapturekit::prelude::*;
use screencapturekit::screenshot_manager::{CGImageExt, SCScreenshotManager};

use crate::platform::types::{Display, Shot};

/// Captures `display`, optionally cropping to `region` and scaling the result.
///
/// `region` is in the same Quartz point space the rest of conduit uses, and is
/// relative to the display's own origin.
///
/// `scale` downsamples the final image. Screens are physically large and models
/// are billed per pixel, so the default already means "logical points, not
/// backing pixels" — see `screen::default_capture_scale`.
pub fn capture(
    display: &Display,
    region: Option<(f64, f64, f64, f64)>,
    scale: f64,
) -> Result<Shot, String> {
    let content = SCShareableContent::get().map_err(|e| {
        format!("could not enumerate displays for capture ({e}). is screen recording permission granted?")
    })?;

    let displays = content.displays();
    let sc_display = displays
        .get(display.index)
        // A display can vanish between enumeration and capture (hot-unplug).
        .or_else(|| displays.first())
        .ok_or_else(|| "no capturable displays".to_string())?;

    let (w, h) = (display.width, display.height);
    let (rx, ry, rw, rh) = region.unwrap_or((0.0, 0.0, w, h));

    // Clamp into the display so a bad region degrades to a smaller capture
    // rather than an opaque ScreenCaptureKit failure.
    let rx = rx.clamp(0.0, w);
    let ry = ry.clamp(0.0, h);
    let rw = rw.clamp(1.0, w - rx);
    let rh = rh.clamp(1.0, h - ry);

    let out_w = ((rw * scale).round() as u32).max(1);
    let out_h = ((rh * scale).round() as u32).max(1);

    let filter = SCContentFilter::create()
        .with_display(sc_display)
        .with_excluding_windows(&[])
        .build();

    let mut config = SCStreamConfiguration::new()
        .with_width(out_w)
        .with_height(out_h)
        // The agent's cursor is drawn by the overlay, and the overlay is a
        // window — so the captured frame would show a stale system arrow. Leave
        // it out; the agent already knows where it put the cursor.
        .with_shows_cursor(false);

    if region.is_some() {
        config = config.with_source_rect(CGRect::new(rx, ry, rw, rh));
    }

    let cg_image = SCScreenshotManager::capture_image(&filter, &config)
        .map_err(|e| format!("screen capture failed: {e}"))?;

    let rgba = cg_image
        .rgba_data()
        .map_err(|e| format!("could not read captured pixels: {e}"))?;

    // ScreenCaptureKit honours the requested width/height, but a mismatch here
    // would corrupt the encode, so derive dimensions from the buffer itself.
    let expected = (out_w as usize) * (out_h as usize) * 4;
    if rgba.len() < expected {
        return Err(format!(
            "captured buffer is smaller than expected ({} < {expected} bytes)",
            rgba.len()
        ));
    }

    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(&rgba[..expected], out_w, out_h, image::ExtendedColorType::Rgba8)
        .map_err(|e| format!("could not encode png: {e}"))?;

    Ok(Shot {
        png,
        width: out_w,
        height: out_h,
    })
}
