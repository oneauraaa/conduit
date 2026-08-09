//! Key-name to macOS virtual keycode mapping.
//!
//! These are ANSI positional codes from `Carbon/Events.h`. They identify a
//! physical key, not a character — which is exactly what we want for shortcuts
//! (cmd+c is the same physical key on a Turkish layout as on a US one). Literal
//! text goes through `input::type_text`, which bypasses layouts entirely.

use objc2_core_graphics::CGEventFlags;

use crate::platform::types::Modifiers;

pub fn lookup(name: &str) -> Option<u16> {
    let n = name.trim().to_ascii_lowercase();
    Some(match n.as_str() {
        // letters
        "a" => 0, "s" => 1, "d" => 2, "f" => 3, "h" => 4, "g" => 5, "z" => 6,
        "x" => 7, "c" => 8, "v" => 9, "b" => 11, "q" => 12, "w" => 13, "e" => 14,
        "r" => 15, "y" => 16, "t" => 17, "o" => 31, "u" => 32, "i" => 34,
        "p" => 35, "l" => 37, "j" => 38, "k" => 40, "n" => 45, "m" => 46,

        // digits
        "1" => 18, "2" => 19, "3" => 20, "4" => 21, "5" => 23,
        "6" => 22, "7" => 26, "8" => 28, "9" => 25, "0" => 29,

        // punctuation
        "-" | "minus" => 27,
        "=" | "equal" => 24,
        "[" | "leftbracket" => 33,
        "]" | "rightbracket" => 30,
        "\\" | "backslash" => 42,
        ";" | "semicolon" => 41,
        "'" | "quote" => 39,
        "," | "comma" => 43,
        "." | "period" => 47,
        "/" | "slash" => 44,
        "`" | "grave" => 50,

        // editing & navigation
        "return" | "enter" => 36,
        "tab" => 48,
        "space" => 49,
        "delete" | "backspace" => 51,
        "escape" | "esc" => 53,
        "forwarddelete" => 117,
        "home" => 115,
        "end" => 119,
        "pageup" => 116,
        "pagedown" => 121,
        "left" => 123,
        "right" => 124,
        "down" => 125,
        "up" => 126,

        // function keys
        "f1" => 122, "f2" => 120, "f3" => 99, "f4" => 118, "f5" => 96,
        "f6" => 97, "f7" => 98, "f8" => 100, "f9" => 101, "f10" => 109,
        "f11" => 103, "f12" => 111,

        _ => return None,
    })
}

/// Packs modifiers into the flags field macOS carries on the event itself.
///
/// Not part of the cross-platform contract: Windows has no such field and must
/// press and release real modifier keys instead, so there is no honest shared
/// return type. Only the *names* are shared, via [`Modifiers::parse`].
pub(super) fn modifier_flags(modifiers: &[String]) -> Result<CGEventFlags, String> {
    let m = Modifiers::parse(modifiers)?;
    let mut flags = CGEventFlags::empty();

    // Command is both the shortcut modifier and the logo key on macOS, so the
    // two collapse here. They are Ctrl and Win respectively on Windows.
    if m.cmd || m.win {
        flags |= CGEventFlags::MaskCommand;
    }
    if m.shift {
        flags |= CGEventFlags::MaskShift;
    }
    if m.alt {
        flags |= CGEventFlags::MaskAlternate;
    }
    if m.ctrl {
        flags |= CGEventFlags::MaskControl;
    }
    if m.func {
        flags |= CGEventFlags::MaskSecondaryFn;
    }
    Ok(flags)
}
