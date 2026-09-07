//! Synthetic input.
//!
//! Which of Wayland's two injection routes this actually uses is
//! [`super::sink`]'s decision — the portal on GNOME and KDE, the wlroots
//! virtual-input protocols on Hyprland, sway and the rest. Everything below is
//! written against the four operations both routes provide, because the
//! difference between them is about consent and availability rather than about
//! what a click is.
//!
//! ## Why conduit tracks the cursor itself
//!
//! Wayland has no "where is the pointer" call. There is no counterpart to
//! `CGEvent::location` or `GetCursorPos`: a client learns the pointer position
//! only when the pointer is over one of its own surfaces, and conduit's windows
//! are click-through overlays that never receive it. The compositor knows and
//! will not say, deliberately — a client that could poll the cursor could watch
//! the user work.
//!
//! So [`POSITION`] is conduit's own record of where it last *put* the pointer.
//! That is exact for every move conduit makes, which is every move that matters:
//! `glide` starts from it, `click` and `drag` act at it, and the overlay draws
//! the AI cursor there. It goes stale only when the human moves the mouse, and
//! the panic stop's whole job is to end the session when they do.
//!
//! The seed is the centre of the primary display, because there is no honest
//! way to ask. The first `glide` corrects it: it ends at a known absolute point
//! regardless of where the real pointer was.
//!
//! ## Why the real pointer moves
//!
//! Same reason as the other two backends: to land clicks in *every*
//! application, conduit drives the one real pointer and hides the system arrow,
//! drawing its own glowing cursor in the overlay instead.

use std::time::Duration;

use parking_lot::RwLock;

use super::{keycodes, sink};
use crate::platform::tween::{self, STEP};
use crate::platform::types::Button;

/// evdev button codes. Both routes speak these directly rather than an enum of
/// their own — `linux/input-event-codes.h`.
const BTN_LEFT: i32 = 0x110;
const BTN_RIGHT: i32 = 0x111;
const BTN_MIDDLE: i32 = 0x112;

/// Scroll axes, numbered as the portal numbers them; `sink` translates for the
/// wlroots route.
const AXIS_VERTICAL: u32 = 0;
const AXIS_HORIZONTAL: u32 = 1;

/// Where conduit last put the pointer. See the module header for why this is
/// bookkeeping rather than a query.
static POSITION: RwLock<Option<(f64, f64)>> = RwLock::new(None);

impl Button {
    fn evdev(self) -> i32 {
        match self {
            Button::Left => BTN_LEFT,
            Button::Right => BTN_RIGHT,
            Button::Middle => BTN_MIDDLE,
        }
    }
}

/// Where the pointer is, as far as conduit knows.
///
/// Seeded to the centre of the primary display on first call. Never fails —
/// the callers are motion paths that need a starting point more than they need
/// the truth, and the first move makes it true.
pub fn cursor_position() -> (f64, f64) {
    if let Some(p) = *POSITION.read() {
        return p;
    }

    let seed = crate::chrome::cached_displays()
        .into_iter()
        .find(|d| d.primary)
        .map(|d| (d.x + d.width / 2.0, d.y + d.height / 2.0))
        .unwrap_or((0.0, 0.0));

    *POSITION.write() = Some(seed);
    seed
}

fn move_to(x: f64, y: f64) -> Result<(), String> {
    sink::pointer_motion_absolute(x, y)?;
    *POSITION.write() = Some((x, y));
    Ok(())
}

/// Eased, slightly arced pointer motion.
///
/// A straight linear slide reads as robotic and, more practically, some
/// hover-sensitive UIs need intermediate positions to fire their handlers at
/// all. `on_step` receives every intermediate point so the overlay draws the
/// trail in lockstep.
pub async fn glide(to_x: f64, to_y: f64, mut on_step: impl FnMut(f64, f64)) {
    sink::begin_action();
    let (from_x, from_y) = cursor_position();

    for (x, y) in tween::path(from_x, from_y, to_x, to_y) {
        if move_to(x, y).is_err() {
            // A mid-glide failure means the session died. Bail rather than
            // spend the rest of the path logging the same error 60 times.
            return;
        }
        on_step(x, y);
        tokio::time::sleep(STEP).await;
    }

    let _ = move_to(to_x, to_y);
    on_step(to_x, to_y);
}

/// Clicks at the current pointer location.
///
/// Unlike macOS, there is no click-state field to mark a double-click with: the
/// portal has press and release and nothing else. Two clicks inside the
/// compositor's double-click interval *are* a double-click here, which is why
/// the gap below is short and fixed rather than merely "comfortably inside" it.
pub async fn click(button: Button, count: i64) {
    sink::begin_action();
    let code = button.evdev();

    for n in 1..=count {
        if sink::pointer_button(code, true).is_err() {
            return;
        }
        // A press and release with no gap at all is dropped by some toolkits as
        // a spurious event; this is the shortest reliably-seen press.
        tokio::time::sleep(Duration::from_millis(20)).await;
        if sink::pointer_button(code, false).is_err() {
            return;
        }
        if n < count {
            // Well inside the 400ms every desktop uses as its double-click
            // threshold, and slow enough that both clicks register.
            tokio::time::sleep(Duration::from_millis(60)).await;
        }
    }
}

/// Press, glide, release — the sequence sliders and drag-and-drop expect.
pub async fn drag(to_x: f64, to_y: f64, button: Button, on_step: impl FnMut(f64, f64)) {
    sink::begin_action();
    let code = button.evdev();

    if sink::pointer_button(code, true).is_err() {
        return;
    }
    // Applications often need a beat between press and motion to enter a drag.
    tokio::time::sleep(Duration::from_millis(90)).await;

    glide(to_x, to_y, on_step).await;

    tokio::time::sleep(Duration::from_millis(60)).await;
    let _ = sink::pointer_button(code, false);
}

/// Scrolls by whole wheel detents.
///
/// The tool surface speaks pixels, which is what Quartz and `SendInput` want.
/// The portal's pixel-accurate `NotifyPointerAxis` is widely implemented as a
/// no-op, while `NotifyPointerAxisDiscrete` is what compositors actually honour
/// — so pixels are converted to detents here. [`PIXELS_PER_DETENT`] matches the
/// conventional 15° step most toolkits treat as "three lines".
pub fn scroll(dx: i32, dy: i32) {
    sink::begin_action();
    const PIXELS_PER_DETENT: i32 = 50;

    // Magnitude first, sign last. Dividing a negative straight through and
    // then clamping to at least 1 collapses every upward scroll to a single
    // detent — `-120` would arrive as `-1` instead of `-2` — so the direction
    // is stripped off before the clamp and reapplied after.
    //
    // Rounded away from zero, so a scroll too small to make a whole detent
    // still moves rather than silently doing nothing. Capped so one call
    // cannot fling a page to its end.
    let detents = |v: i32| -> i32 {
        if v == 0 {
            return 0;
        }
        (v.abs() / PIXELS_PER_DETENT).clamp(1, 20) * v.signum()
    };

    let (dx, dy) = (detents(dx), detents(dy));

    if dy != 0 {
        // conduit's dy is "content moves up by this much", matching Quartz;
        // the portal's positive vertical axis scrolls down. Hence the negation.
        let _ = sink::pointer_axis_discrete(AXIS_VERTICAL, -dy);
    }
    if dx != 0 {
        let _ = sink::pointer_axis_discrete(AXIS_HORIZONTAL, -dx);
    }
}

/// Types a string, one keysym per character.
///
/// Layout-independent: see the header of [`super::keycodes`] for why keysyms
/// rather than scancodes are what make this correct on a Turkish or Japanese
/// keyboard.
pub async fn type_text(text: &str) {
    sink::begin_action();
    let mut remaining = text;
    while !remaining.is_empty() {
        let Ok(end) = sink::prepare_text(remaining) else { return };
        for (i, c) in remaining[..end].chars().enumerate() {
            let sym = keycodes::keysym_for_char(c);
            if sink::keyboard_keysym(sym, true).is_err() {
                return;
            }
            if sink::keyboard_keysym(sym, false).is_err() {
                return;
            }

            // Give slower text fields time to consume queued key events.
            if i % 4 == 3 {
                tokio::time::sleep(Duration::from_millis(12)).await;
            }
        }
        remaining = &remaining[end..];
        if !remaining.is_empty() {
            // Let the last events in this map drain before recycling codes.
            tokio::time::sleep(Duration::from_millis(12)).await;
        }
    }
}

/// Presses a named key with optional modifiers, e.g. `("a", ["cmd"])`.
///
/// Modifiers are pressed as real keys and released in reverse order. There is
/// no flags field to set as there is on macOS — the compositor tracks modifier
/// state from the key events themselves, so a leaked modifier would stay stuck
/// down for every subsequent keystroke on the machine. That is why the releases
/// run even when the main key fails.
pub fn key_press(key: &str, modifiers: &[String]) -> Result<(), String> {
    sink::begin_action();
    let code = keycodes::lookup(key).ok_or_else(|| format!("unknown key: {key}"))?;
    let mods = keycodes::modifier_keysyms(modifiers)?;
    sink::prepare_key(code as i32)?;

    let result = mods.iter().try_for_each(|m| sink::keyboard_keysym(*m, true))
        .and_then(|_| sink::keyboard_keysym(code as i32, true))
        .and_then(|_| sink::keyboard_keysym(code as i32, false));

    for m in mods.iter().rev() {
        // Deliberately not `?`: a modifier left held down would corrupt every
        // later keystroke system-wide, so every release is attempted even if
        // an earlier one failed.
        let _ = sink::keyboard_keysym(*m, false);
    }

    result
}

/// Why the last synthetic input did not reach its target, if it didn't.
///
/// On Wayland there is exactly one reason, and it is not per-window the way
/// Windows' UIPI is: either conduit's input route is up and every application is
/// reachable, or it is not and none are. There is no equivalent of an elevated
/// window conduit can see but not touch — the compositor injects at the seat,
/// below any such distinction. Which route that is, and so what the user has to
/// do about it, is [`super::sink`]'s to say.
pub fn blocked_reason() -> Option<String> {
    sink::blocked_reason()
}

/// Hiding the system cursor is not possible on Wayland, and pretending
/// otherwise would be worse than not trying.
///
/// A client cannot alter the pointer image except while the pointer is over its
/// own surface — and conduit's overlay is explicitly click-through, so it never
/// is. The overlay's glowing orb is therefore drawn *in addition to* the system
/// arrow rather than instead of it.
///
/// That is a real visual difference from macOS and Windows, and it is left
/// visible rather than papered over: the arrow sits inside the glow and still
/// reads as one cursor under conduit's control. The functions stay because the
/// contract has them and `chrome.rs` calls them on every takeover.
pub fn hide_system_cursor() {}

pub fn show_system_cursor() {}
