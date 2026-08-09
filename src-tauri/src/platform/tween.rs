//! Pointer motion easing, shared by both backends.
//!
//! A straight linear slide reads as robotic and, more practically, some
//! hover-sensitive UIs need a few intermediate positions to fire their handlers
//! at all. The curve lives here rather than in either backend so the AI cursor
//! *feels* the same on both platforms — the arc and timing are part of the
//! product, not an implementation detail of Quartz or SendInput.

use std::time::Duration;

/// Interval between tween steps. ~125 Hz: smooth to the eye, and comfortably
/// under the rate at which synthetic event posting starts to drop events on
/// either platform.
pub const STEP: Duration = Duration::from_millis(8);

/// The intermediate points of an eased, slightly arced glide.
///
/// Excludes the destination: callers sleep [`STEP`] after each point returned
/// here, then move to the exact destination once more without sleeping. A trip
/// shorter than a pixel returns empty, so the caller just jumps.
pub fn path(from_x: f64, from_y: f64, to_x: f64, to_y: f64) -> Vec<(f64, f64)> {
    let dx = to_x - from_x;
    let dy = to_y - from_y;
    let distance = (dx * dx + dy * dy).sqrt();

    if distance < 1.0 {
        return Vec::new();
    }

    // Longer trips take longer, but sub-linearly — 40px and 2000px should not
    // differ by a factor of 50.
    let duration_ms = (160.0 + distance.sqrt() * 22.0).clamp(180.0, 620.0);
    let steps = ((duration_ms / STEP.as_millis() as f64).round() as usize).max(2);

    // Perpendicular bow, capped so long journeys don't sail off-screen.
    let bow = (distance * 0.06).min(38.0);
    let (nx, ny) = (-dy / distance, dx / distance);

    (1..=steps)
        .map(|i| {
            let t = i as f64 / steps as f64;
            // easeInOutCubic
            let e = if t < 0.5 {
                4.0 * t * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
            };
            // Arc peaks mid-flight and returns to zero at both ends.
            let arc = (e * std::f64::consts::PI).sin() * bow;

            (from_x + dx * e + nx * arc, from_y + dy * e + ny * arc)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sub_pixel_trip_has_no_intermediate_steps() {
        assert!(path(100.0, 100.0, 100.4, 100.4).is_empty());
    }

    #[test]
    fn the_path_ends_at_the_destination() {
        let p = path(0.0, 0.0, 800.0, 600.0);
        let (x, y) = *p.last().unwrap();
        // The final eased step lands on the destination; the arc term is
        // sin(pi) == 0 there, so this is exact up to float noise.
        assert!((x - 800.0).abs() < 0.001, "x drifted to {x}");
        assert!((y - 600.0).abs() < 0.001, "y drifted to {y}");
    }

    #[test]
    fn the_path_bows_off_the_straight_line() {
        let p = path(0.0, 0.0, 1000.0, 0.0);
        // Mid-flight the cursor should be measurably off the y=0 axis.
        let mid = p[p.len() / 2];
        assert!(mid.1.abs() > 1.0, "expected a bow, got y={}", mid.1);
    }

    #[test]
    fn longer_trips_take_more_steps_but_sub_linearly() {
        let short = path(0.0, 0.0, 40.0, 0.0).len();
        let long = path(0.0, 0.0, 2000.0, 0.0).len();
        assert!(long > short);
        assert!(long < short * 50, "{long} vs {short}: should be sub-linear");
    }
}
