//! Synthetic input via `SendInput`.
//!
//! ## Why the real pointer moves
//!
//! Same reason as macOS: to make the AI's cursor land clicks in *every*
//! application, conduit drives the real pointer and hides the system arrow,
//! drawing its own glowing cursor in the overlay instead. Visually there is one
//! cursor and it belongs to the AI; mechanically it's the pointer Windows
//! already trusts.
//!
//! The easing curve lives in `platform::tween`, shared with macOS, so the
//! cursor moves with the same weight on both.
//!
//! ## UIPI
//!
//! `SendInput` returns the number of events it actually inserted. A short
//! return is not a transient blip — it is `ERROR_ACCESS_DENIED`, meaning the
//! foreground window belongs to a process at a higher integrity level than
//! conduit and Windows discarded the input. Unchecked, conduit would report
//! "clicked at 400, 300" while nothing happened, which is exactly the
//! silent-wrong-answer failure the capture path is designed to avoid. Every
//! send goes through [`send`], which notices.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
    MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK,
    MOUSEEVENTF_WHEEL, MOUSEEVENTF_HWHEEL, MOUSE_EVENT_FLAGS, MOUSEINPUT, SendInput, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, WHEEL_DELTA,
};

use super::keycodes;
use crate::platform::tween::{self, STEP};
use crate::platform::types::{Button, Modifiers};

/// Set once UIPI has eaten an event, so the failure is reported rather than
/// silently swallowed. Reset when a send succeeds again.
static INPUT_BLOCKED: AtomicBool = AtomicBool::new(false);

/// Why the last synthetic input did not reach its target, if it didn't.
///
/// `click`, `drag`, `scroll` and `type_text` cannot return an error — they are
/// fire-and-forget by signature — so without this an agent would be told
/// "clicked at 400, 300" for a click Windows threw away, and would go on to
/// reason about a state change that never happened. The tool layer checks this
/// after every input operation and refuses rather than lying.
///
/// Always `None` on macOS, where Quartz has no equivalent restriction.
pub fn blocked_reason() -> Option<String> {
    INPUT_BLOCKED.load(Ordering::Relaxed).then(|| {
        "windows discarded the input: the window in front belongs to an elevated \
         process and conduit is not running as administrator. ask the user to \
         restart conduit as admin from the Server tab, or work with a \
         non-elevated window."
            .to_string()
    })
}

/// Posts a batch of events, noticing when Windows discards them.
///
/// The whole batch goes in one call on purpose. `SendInput` guarantees the
/// events are not interleaved with the user's own physical input, which is what
/// keeps a surrogate pair or a modified keystroke from being torn apart.
fn send(events: &[INPUT]) -> bool {
    if events.is_empty() {
        return true;
    }
    let inserted = unsafe { SendInput(events, std::mem::size_of::<INPUT>() as i32) };
    let ok = inserted as usize == events.len();
    INPUT_BLOCKED.store(!ok, Ordering::Relaxed);
    if !ok {
        tracing::warn!(
            sent = events.len(),
            inserted,
            "windows discarded synthetic input — the foreground window is likely elevated"
        );
    }
    ok
}

/* ── the mouse ── */

impl Button {
    fn down_up(self) -> (MOUSE_EVENT_FLAGS, MOUSE_EVENT_FLAGS) {
        match self {
            Button::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
            Button::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
            Button::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
        }
    }
}

/// Where the pointer is now, in physical pixels on the virtual screen.
pub fn cursor_position() -> (f64, f64) {
    let mut p = windows::Win32::Foundation::POINT::default();
    if unsafe { GetCursorPos(&mut p) }.is_ok() {
        (p.x as f64, p.y as f64)
    } else {
        (0.0, 0.0)
    }
}

/// Converts a virtual-screen pixel to the 0..65535 range `MOUSEEVENTF_ABSOLUTE`
/// wants.
///
/// Two off-by-ones live here and both produce bugs that look like something
/// else:
///
///   - The origin must be subtracted. A monitor to the left of the primary has
///     negative x, and `SM_XVIRTUALSCREEN` is where the virtual screen actually
///     starts. Forget it and every click lands on the wrong monitor.
///   - The divisor is `span - 1`, not `span`. Forget it and the last pixel
///     column and row are unreachable — which shows up as "the close button
///     doesn't work on a maximised window" long after you stopped looking here.
fn normalize(x: f64, y: f64) -> (i32, i32) {
    let (ox, oy, w, h) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN) as f64,
            GetSystemMetrics(SM_YVIRTUALSCREEN) as f64,
            GetSystemMetrics(SM_CXVIRTUALSCREEN) as f64,
            GetSystemMetrics(SM_CYVIRTUALSCREEN) as f64,
        )
    };

    let nx = ((x - ox) * 65535.0 / (w - 1.0).max(1.0)).round() as i32;
    let ny = ((y - oy) * 65535.0 / (h - 1.0).max(1.0)).round() as i32;
    (nx.clamp(0, 65535), ny.clamp(0, 65535))
}

fn mouse_input(flags: MOUSE_EVENT_FLAGS, dx: i32, dy: i32, data: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data as u32,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Moves the pointer to an absolute point, emitting a real move event so
/// hover-sensitive UIs see the motion.
fn move_to(x: f64, y: f64) {
    let (nx, ny) = normalize(x, y);
    send(&[mouse_input(
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        nx,
        ny,
        0,
    )]);
}

/// Eased, slightly arced pointer motion. `on_step` receives every intermediate
/// point so the overlay can draw the trail in lockstep with the real pointer.
pub async fn glide(to_x: f64, to_y: f64, mut on_step: impl FnMut(f64, f64)) {
    let (from_x, from_y) = cursor_position();

    for (x, y) in tween::path(from_x, from_y, to_x, to_y) {
        move_to(x, y);
        on_step(x, y);
        tokio::time::sleep(STEP).await;
    }

    move_to(to_x, to_y);
    on_step(to_x, to_y);
}

/// Clicks at the current pointer location.
///
/// Unlike macOS, which carries a click-state field on the event, Windows infers
/// a double-click from two clicks arriving inside `GetDoubleClickTime` at
/// nearly the same position — so the gap below has to stay comfortably under
/// it, and the pointer must not move in between.
pub async fn click(button: Button, count: i64) {
    let (down, up) = button.down_up();

    for n in 1..=count.max(1) {
        send(&[mouse_input(down, 0, 0, 0), mouse_input(up, 0, 0, 0)]);
        if n < count {
            tokio::time::sleep(Duration::from_millis(60)).await;
        }
    }
}

/// Press, glide, release — the sequence sliders and drag-and-drop expect.
pub async fn drag(to_x: f64, to_y: f64, button: Button, mut on_step: impl FnMut(f64, f64)) {
    let (down, up) = button.down_up();

    send(&[mouse_input(down, 0, 0, 0)]);
    // Applications often need a beat between press and motion to enter a drag.
    tokio::time::sleep(Duration::from_millis(90)).await;

    glide(to_x, to_y, |px, py| on_step(px, py)).await;

    tokio::time::sleep(Duration::from_millis(60)).await;
    send(&[mouse_input(up, 0, 0, 0)]);
}

/// Scrolls the surface under the cursor.
///
/// `dy` is positive-scrolls-up, matching the macOS tool contract and Windows'
/// own wheel convention. Callers pass a distance in coordinate-space units;
/// `WHEEL_DELTA` (120) is one notch, and treating ~40px as a notch keeps a
/// given `dy` moving a comparable distance on both platforms.
pub fn scroll(dx: i32, dy: i32) {
    const PIXELS_PER_NOTCH: f64 = 40.0;

    let mut events = Vec::new();
    if dy != 0 {
        let amount = (dy as f64 / PIXELS_PER_NOTCH * WHEEL_DELTA as f64).round() as i32;
        events.push(mouse_input(MOUSEEVENTF_WHEEL, 0, 0, amount));
    }
    if dx != 0 {
        let amount = (dx as f64 / PIXELS_PER_NOTCH * WHEEL_DELTA as f64).round() as i32;
        events.push(mouse_input(MOUSEEVENTF_HWHEEL, 0, 0, amount));
    }
    send(&events);
}

/* ── the keyboard ── */

fn key_input(scan: u16, extended: bool, up: bool) -> INPUT {
    let mut flags = KEYEVENTF_SCANCODE;
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if up {
        flags |= KEYEVENTF_KEYUP;
    }

    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn unit_input(unit: u16, up: bool) -> INPUT {
    let mut flags = KEYEVENTF_UNICODE;
    if up {
        flags |= KEYEVENTF_KEYUP;
    }

    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: unit,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Types a string as literal unicode, sidestepping keyboard-layout translation
/// entirely — the same code types "hello", "merhaba" and "こんにちは" without
/// knowing anything about the active layout.
///
/// Two details are load-bearing:
///
///   - **Newlines and tabs are not unicode here.** `KEYEVENTF_UNICODE` with
///     `\n` inserts a line-feed character, which most Windows controls ignore
///     — it does not press Enter. They are translated to real key events.
///   - **A surrogate pair must go in one batch.** An emoji is two UTF-16 units,
///     and sending them as separate `SendInput` calls lets the target see two
///     lone surrogates and render two replacement glyphs.
pub async fn type_text(text: &str) {
    for chunk in text.chars().collect::<Vec<_>>().chunks(16) {
        let mut events = Vec::with_capacity(chunk.len() * 2);

        for ch in chunk {
            match ch {
                '\n' | '\r' => {
                    let scan = keycodes::lookup("return").unwrap();
                    events.push(key_input(keycodes::scancode(scan), false, false));
                    events.push(key_input(keycodes::scancode(scan), false, true));
                }
                '\t' => {
                    let scan = keycodes::lookup("tab").unwrap();
                    events.push(key_input(keycodes::scancode(scan), false, false));
                    events.push(key_input(keycodes::scancode(scan), false, true));
                }
                _ => {
                    let mut buf = [0u16; 2];
                    for unit in ch.encode_utf16(&mut buf) {
                        events.push(unit_input(*unit, false));
                        events.push(unit_input(*unit, true));
                    }
                }
            }
        }

        send(&events);

        // A human-ish cadence. Also gives slower text fields time to keep up.
        tokio::time::sleep(Duration::from_millis(12)).await;
    }
}

/// Presses a named key with optional modifiers, e.g. `("a", ["cmd"])`.
///
/// Modifiers are pressed and released around the key as real events; Windows
/// has no per-event modifier field like Quartz's `CGEventFlags`.
pub fn key_press(key: &str, modifiers: &[String]) -> Result<(), String> {
    let code = keycodes::lookup(key).ok_or_else(|| format!("unknown key: {key}"))?;
    let m = Modifiers::parse(modifiers)?;

    // `cmd` is the shortcut modifier, which is Ctrl here — so an agent's
    // `cmd+c` and `ctrl+c` are the same keystroke, as they should be.
    let mut mods: Vec<u16> = Vec::new();
    if m.cmd || m.ctrl {
        mods.push(keycodes::LCTRL);
    }
    if m.win {
        mods.push(keycodes::LWIN);
    }
    if m.shift {
        mods.push(keycodes::LSHIFT);
    }
    if m.alt {
        mods.push(keycodes::LALT);
    }
    // `fn` has no synthesizable equivalent on Windows — it is handled in
    // keyboard firmware, below anything SendInput can reach. Silently ignored
    // rather than refused, so a cross-platform prompt carrying it still works.

    let mut events = Vec::with_capacity(mods.len() * 2 + 2);
    for mc in &mods {
        events.push(key_input(
            keycodes::scancode(*mc),
            keycodes::is_extended(*mc),
            false,
        ));
    }
    events.push(key_input(
        keycodes::scancode(code),
        keycodes::is_extended(code),
        false,
    ));
    events.push(key_input(
        keycodes::scancode(code),
        keycodes::is_extended(code),
        true,
    ));
    // Release in reverse, the order a human's hand leaves the keyboard.
    for mc in mods.iter().rev() {
        events.push(key_input(
            keycodes::scancode(*mc),
            keycodes::is_extended(*mc),
            true,
        ));
    }

    if send(&events) {
        Ok(())
    } else {
        Err("windows discarded the keystroke — the window in front is running \
             elevated and conduit is not. restart conduit as administrator from \
             the Server tab."
            .into())
    }
}

/* ── the system cursor ── */

pub fn hide_system_cursor() {
    super::cursor::hide();
}

pub fn show_system_cursor() {
    super::cursor::restore();
}
