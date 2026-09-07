//! Key-name to X11 keysym mapping.
//!
//! ## Why keysyms and not evdev scancodes
//!
//! The portal offers both `NotifyKeyboardKeycode` (positional, evdev) and
//! `NotifyKeyboardKeysym` (symbolic, X11). conduit uses keysyms, which is the
//! opposite of what the macOS and Windows backends do, and the difference is
//! deliberate.
//!
//! Those two backends send *positional* codes because they can also attach a
//! literal unicode string to an event for text — `CGEventKeyboardSetUnicodeString`
//! on macOS, `KEYEVENTF_UNICODE` on Windows — so shortcuts get the physical key
//! and typing bypasses the layout entirely. Wayland's portal has no unicode
//! escape hatch. A keycode here would be interpreted through whatever layout the
//! compositor has loaded, so typing `@` on a Turkish keyboard would emit `"`.
//!
//! Keysyms are the layout-independent route: `XK_at` means the at sign, and the
//! compositor finds whichever physical key and modifier produces it. That makes
//! [`super::input::type_text`] correct on every layout, which is the property
//! that actually matters.
//!
//! The trade-off is that a keysym for a *shortcut* is nominal rather than
//! positional — `cmd+z` asks for the key labelled `z`, not the key in `z`'s
//! place on a US board. On a Turkish Q layout those are the same key. On Dvorak
//! they are not, and the shortcut follows the label, which is what a user
//! reading their own keycaps expects.
//!
//! Values are from `X11/keysymdef.h` and are stable public API.

/// The keysym for a named key, or `None` if the name is not one conduit knows.
///
/// The return type is `u16` to match the shared contract, which was shaped by
/// macOS virtual keycodes. Every keysym conduit names fits — the printable
/// ASCII block is `0x20..0x7e` and the function block is `0xff00..0xffff` — and
/// [`keysym`] is the wider accessor used for arbitrary text.
pub fn lookup(name: &str) -> Option<u16> {
    let n = name.trim().to_ascii_lowercase();
    Some(match n.as_str() {
        // letters — lowercase keysyms, so an unmodified press types lowercase
        // and `shift` produces the capital the way a real keyboard does
        c if c.len() == 1 && c.chars().next().is_some_and(|c| c.is_ascii_lowercase()) => {
            c.chars().next().unwrap() as u16
        }

        // digits
        c if c.len() == 1 && c.chars().next().is_some_and(|c| c.is_ascii_digit()) => {
            c.chars().next().unwrap() as u16
        }

        // punctuation
        "-" | "minus" => 0x002d,
        "=" | "equal" => 0x003d,
        "[" | "leftbracket" => 0x005b,
        "]" | "rightbracket" => 0x005d,
        "\\" | "backslash" => 0x005c,
        ";" | "semicolon" => 0x003b,
        "'" | "quote" => 0x0027,
        "," | "comma" => 0x002c,
        "." | "period" => 0x002e,
        "/" | "slash" => 0x002f,
        "`" | "grave" => 0x0060,

        // editing & navigation
        "return" | "enter" => 0xff0d,
        "tab" => 0xff09,
        "space" => 0x0020,
        "delete" | "backspace" => 0xff08,
        "escape" | "esc" => 0xff1b,
        "forwarddelete" => 0xffff,
        "home" => 0xff50,
        "end" => 0xff57,
        "pageup" => 0xff55,
        "pagedown" => 0xff56,
        "left" => 0xff51,
        "up" => 0xff52,
        "right" => 0xff53,
        "down" => 0xff54,
        "insert" => 0xff63,

        // function keys
        "f1" => 0xffbe,
        "f2" => 0xffbf,
        "f3" => 0xffc0,
        "f4" => 0xffc1,
        "f5" => 0xffc2,
        "f6" => 0xffc3,
        "f7" => 0xffc4,
        "f8" => 0xffc5,
        "f9" => 0xffc6,
        "f10" => 0xffc7,
        "f11" => 0xffc8,
        "f12" => 0xffc9,

        _ => return None,
    })
}

/* ── modifiers ── */

pub const SHIFT: i32 = 0xffe1;
pub const CONTROL: i32 = 0xffe3;
pub const ALT: i32 = 0xffe9;
pub const SUPER: i32 = 0xffeb;

/// The keysyms to hold down for a parsed modifier set, in press order.
///
/// `cmd` maps to Control, not Super. That is the same decision the Windows
/// backend makes and for the same reason: `cmd` in conduit's tool surface means
/// *the shortcut modifier*, and on Linux that is Control. An agent that learned
/// `cmd+c` means copy drives a Linux box without relearning anything, and
/// `super` stays available for the logo key so `super+e` is still reachable.
///
/// `fn` has no keysym — it is a hardware-level modifier that never reaches the
/// compositor as a key — so it is accepted and ignored rather than rejected,
/// which keeps a macOS-shaped prompt from failing outright here.
pub(super) fn modifier_keysyms(modifiers: &[String]) -> Result<Vec<i32>, String> {
    let m = crate::platform::types::Modifiers::parse(modifiers)?;
    let mut out = Vec::new();

    // Order matters for compositors that latch: the shortcut modifier first,
    // shift last, which is the order a human's hand arrives in.
    if m.cmd || m.ctrl {
        out.push(CONTROL);
    }
    if m.win {
        out.push(SUPER);
    }
    if m.alt {
        out.push(ALT);
    }
    if m.shift {
        out.push(SHIFT);
    }
    Ok(out)
}

/// The keysym that types `c`, for [`super::input::type_text`].
///
/// Latin-1 maps straight through — the keysym *is* the codepoint below 0x100.
/// Everything else uses the Unicode escape: keysym `0x01000000 + codepoint`,
/// which every modern compositor understands and which is what makes typing
/// `こんにちは` work without a Japanese layout loaded.
pub(super) fn keysym_for_char(c: char) -> i32 {
    match c {
        '\n' => 0xff0d,
        '\t' => 0xff09,
        c if (c as u32) < 0x100 => c as i32,
        c => 0x0100_0000 + c as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_and_digits_are_their_own_keysyms() {
        assert_eq!(lookup("a"), Some(0x0061));
        assert_eq!(lookup("Z"), Some(0x007a));
        assert_eq!(lookup("5"), Some(0x0035));
    }

    #[test]
    fn named_keys_resolve_and_unknown_ones_do_not() {
        assert_eq!(lookup("return"), Some(0xff0d));
        assert_eq!(lookup("Escape"), Some(0xff1b));
        assert_eq!(lookup("f11"), Some(0xffc8));
        assert!(lookup("dpad-up").is_none());
    }

    /// The whole point of the keysym route: text that no US layout can produce
    /// still has to type correctly.
    #[test]
    fn non_latin_text_goes_through_the_unicode_escape() {
        // Latin-1 passes through unchanged.
        assert_eq!(keysym_for_char('ü'), 0x00fc);
        // Beyond it, the 0x01000000 escape.
        assert_eq!(keysym_for_char('こ'), 0x0100_0000 + 0x3053);
        assert_eq!(keysym_for_char('😀'), 0x0100_0000 + 0x1_f600);
    }

    /// `cmd` is the shortcut modifier, so it must land on Control here — an
    /// agent asking for `cmd+c` is asking to copy, not to press the logo key.
    #[test]
    fn cmd_is_control_and_super_is_separate() {
        assert_eq!(modifier_keysyms(&["cmd".into()]).unwrap(), vec![CONTROL]);
        assert_eq!(modifier_keysyms(&["ctrl".into()]).unwrap(), vec![CONTROL]);
        assert_eq!(modifier_keysyms(&["super".into()]).unwrap(), vec![SUPER]);
    }

    #[test]
    fn fn_is_accepted_and_dropped_rather_than_refused() {
        // It has no keysym, but refusing would fail a prompt written on a mac.
        assert_eq!(modifier_keysyms(&["fn".into()]).unwrap(), Vec::<i32>::new());
    }

    #[test]
    fn modifiers_press_in_a_stable_order() {
        let got = modifier_keysyms(&["shift".into(), "cmd".into(), "alt".into()]).unwrap();
        assert_eq!(got, vec![CONTROL, ALT, SHIFT]);
    }
}
