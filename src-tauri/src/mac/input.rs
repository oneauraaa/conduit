//! Synthetic input.
//!
//! ## Why the real pointer moves
//!
//! macOS has exactly one hardware cursor. To make the AI's cursor land clicks
//! in *every* application — including ones that ignore events posted straight
//! to their process — conduit drives the real pointer and hides the system
//! arrow, drawing its own glowing cursor in the overlay instead. Visually
//! there is one cursor and it belongs to the AI; mechanically it's the pointer
//! macOS already trusts.
//!
//! [`InputBackend`] exists so a genuinely independent virtual cursor
//! (CGEventPostToPid, no pointer movement at all) can be added later without
//! touching a single tool handler.

use std::time::Duration;

use objc2_core_graphics::{
    CGDisplayHideCursor, CGDisplayShowCursor, CGEvent, CGEventFlags, CGEventSource,
    CGEventSourceStateID, CGEventTapLocation, CGEventType, CGMainDisplayID, CGMouseButton,
    CGScrollEventUnit, CGWarpMouseCursorPosition,
};
use objc2_core_foundation::CGPoint;

use super::keycodes;

/// Interval between tween steps. ~125 Hz: smooth to the eye, and comfortably
/// under the rate at which CGEvent posting starts to drop events.
const STEP: Duration = Duration::from_millis(8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

impl Button {
    fn cg(self) -> CGMouseButton {
        match self {
            Button::Left => CGMouseButton::Left,
            Button::Right => CGMouseButton::Right,
            Button::Middle => CGMouseButton::Center,
        }
    }

    fn events(self) -> (CGEventType, CGEventType, CGEventType) {
        match self {
            Button::Left => (
                CGEventType::LeftMouseDown,
                CGEventType::LeftMouseUp,
                CGEventType::LeftMouseDragged,
            ),
            Button::Right => (
                CGEventType::RightMouseDown,
                CGEventType::RightMouseUp,
                CGEventType::RightMouseDragged,
            ),
            Button::Middle => (
                CGEventType::OtherMouseDown,
                CGEventType::OtherMouseUp,
                CGEventType::OtherMouseDragged,
            ),
        }
    }
}

fn source() -> Option<objc2_core_foundation::CFRetained<CGEventSource>> {
    CGEventSource::new(CGEventSourceStateID::HIDSystemState)
}

fn post(event: Option<&CGEvent>) {
    CGEvent::post(CGEventTapLocation::HIDEventTap, event);
}

/// Where the pointer is now, in Quartz coordinates.
pub fn cursor_position() -> (f64, f64) {
    // Reading back an event with no source yields the current pointer location.
    let ev = CGEvent::new(None);
    let p = CGEvent::location(ev.as_deref());
    (p.x, p.y)
}

/// Jumps the pointer without emitting a move event. Used for each tween step;
/// the accompanying `MouseMoved` event is posted separately so applications
/// see a proper move stream rather than a teleport.
fn warp(x: f64, y: f64) {
    CGWarpMouseCursorPosition(CGPoint { x, y });
}

fn mouse_event(kind: CGEventType, x: f64, y: f64, button: Button) {
    let src = source();
    let ev = CGEvent::new_mouse_event(src.as_deref(), kind, CGPoint { x, y }, button.cg());
    post(ev.as_deref());
}

/// Eased, slightly arced pointer motion.
///
/// A straight linear slide reads as robotic and, more practically, some
/// hover-sensitive UIs need a few intermediate positions to fire their
/// handlers at all. `on_step` receives every intermediate point so the overlay
/// can draw the trail in lockstep with the real pointer.
pub async fn glide(to_x: f64, to_y: f64, mut on_step: impl FnMut(f64, f64)) {
    let (from_x, from_y) = cursor_position();
    let dx = to_x - from_x;
    let dy = to_y - from_y;
    let distance = (dx * dx + dy * dy).sqrt();

    if distance < 1.0 {
        warp(to_x, to_y);
        mouse_event(CGEventType::MouseMoved, to_x, to_y, Button::Left);
        on_step(to_x, to_y);
        return;
    }

    // Longer trips take longer, but sub-linearly — 40px and 2000px should not
    // differ by a factor of 50.
    let duration_ms = (160.0 + distance.sqrt() * 22.0).clamp(180.0, 620.0);
    let steps = ((duration_ms / STEP.as_millis() as f64).round() as usize).max(2);

    // Perpendicular bow, capped so long journeys don't sail off-screen.
    let bow = (distance * 0.06).min(38.0);
    let (nx, ny) = (-dy / distance, dx / distance);

    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        // easeInOutCubic
        let e = if t < 0.5 {
            4.0 * t * t * t
        } else {
            1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
        };
        // Arc peaks mid-flight and returns to zero at both ends.
        let arc = (e * std::f64::consts::PI).sin() * bow;

        let x = from_x + dx * e + nx * arc;
        let y = from_y + dy * e + ny * arc;

        warp(x, y);
        mouse_event(CGEventType::MouseMoved, x, y, Button::Left);
        on_step(x, y);

        tokio::time::sleep(STEP).await;
    }

    warp(to_x, to_y);
    mouse_event(CGEventType::MouseMoved, to_x, to_y, Button::Left);
    on_step(to_x, to_y);
}

/// Clicks at the current pointer location. `count` of 2 produces a real
/// double-click: macOS distinguishes it by the click-state field, not by two
/// independent clicks arriving close together.
pub async fn click(button: Button, count: i64) {
    let (x, y) = cursor_position();
    let (down, up, _) = button.events();

    for n in 1..=count {
        // The CGEventSource is created and dropped inside this scope on
        // purpose. CFRetained is not Send, and holding one across the await
        // below would make every calling tool future non-Send.
        {
            let src = source();
            for kind in [down, up] {
                let ev =
                    CGEvent::new_mouse_event(src.as_deref(), kind, CGPoint { x, y }, button.cg());
                if let Some(e) = ev.as_deref() {
                    CGEvent::set_integer_value_field(
                        Some(e),
                        objc2_core_graphics::CGEventField::MouseEventClickState,
                        n,
                    );
                }
                post(ev.as_deref());
            }
        }
        if n < count {
            // Comfortably inside the system double-click interval.
            tokio::time::sleep(Duration::from_millis(60)).await;
        }
    }
}

/// Press, glide, release — the sequence sliders and drag-and-drop expect.
pub async fn drag(to_x: f64, to_y: f64, button: Button, mut on_step: impl FnMut(f64, f64)) {
    let (x, y) = cursor_position();
    let (down, up, dragged) = button.events();

    mouse_event(down, x, y, button);
    // Applications often need a beat between press and motion to enter a drag.
    tokio::time::sleep(Duration::from_millis(90)).await;

    // A fresh source per step keeps this future Send — see `click`.
    glide(to_x, to_y, |px, py| {
        let src = source();
        let ev =
            CGEvent::new_mouse_event(src.as_deref(), dragged, CGPoint { x: px, y: py }, button.cg());
        post(ev.as_deref());
        on_step(px, py);
    })
    .await;

    tokio::time::sleep(Duration::from_millis(60)).await;
    mouse_event(up, to_x, to_y, button);
}

pub fn scroll(dx: i32, dy: i32) {
    let src = source();
    let ev = CGEvent::new_scroll_wheel_event2(
        src.as_deref(),
        CGScrollEventUnit::Pixel,
        2,
        dy,
        dx,
        0,
    );
    post(ev.as_deref());
}

/// Types a string by attaching unicode directly to synthetic key events, which
/// sidesteps keyboard-layout translation entirely — the same code types "hello",
/// "merhaba" and "こんにちは" without knowing anything about the active layout.
pub async fn type_text(text: &str) {
    for chunk in text.chars().collect::<Vec<_>>().chunks(16) {
        let utf16: Vec<u16> = chunk.iter().collect::<String>().encode_utf16().collect();

        // Scoped so no CFRetained is alive at the await below — see `click`.
        {
            let src = source();
            for down in [true, false] {
                let Some(ev) = CGEvent::new_keyboard_event(src.as_deref(), 0, down) else {
                    continue;
                };
                unsafe {
                    CGEvent::keyboard_set_unicode_string(
                        Some(&ev),
                        utf16.len() as u64,
                        utf16.as_ptr(),
                    );
                }
                post(Some(&ev));
            }
        }

        // A human-ish cadence. Also gives slower text fields time to keep up.
        tokio::time::sleep(Duration::from_millis(12)).await;
    }
}

/// Presses a named key with optional modifiers, e.g. `("a", ["cmd"])`.
pub fn key_press(key: &str, modifiers: &[String]) -> Result<(), String> {
    let code = keycodes::lookup(key).ok_or_else(|| format!("unknown key: {key}"))?;
    let flags = keycodes::modifier_flags(modifiers)?;
    let src = source();

    for down in [true, false] {
        let Some(ev) = CGEvent::new_keyboard_event(src.as_deref(), code, down) else {
            return Err("could not create a keyboard event".into());
        };
        if flags != CGEventFlags::empty() {
            CGEvent::set_flags(Some(&ev), flags);
        }
        post(Some(&ev));
    }
    Ok(())
}

/// Hides the system arrow so the overlay's glowing cursor is the only pointer
/// on screen.
///
/// This is best-effort by design: `CGDisplayHideCursor` is scoped to the
/// calling application's connection and macOS has historically ignored it when
/// the caller isn't frontmost — which conduit usually isn't, since the whole
/// point is that another app has focus. When it doesn't take, the system arrow
/// simply sits inside the glow, which still reads as intentional. Nothing else
/// depends on it succeeding.
pub fn hide_system_cursor() {
    CGDisplayHideCursor(CGMainDisplayID());
}

pub fn show_system_cursor() {
    CGDisplayShowCursor(CGMainDisplayID());
}
