//! Key-name to scancode mapping.
//!
//! These are **set-1 scancodes**, the positional codes the keyboard controller
//! emits, deliberately not virtual-key codes. A scancode identifies a physical
//! key; a VK identifies the character that key currently produces. Shortcuts
//! want the former — ctrl+c must be the same physical key on a Turkish-F
//! layout as on a US one, and it is the key *position* that every application's
//! accelerator table is really keyed on.
//!
//! This is the same reasoning `platform/mac/keycodes.rs` gives for using
//! Carbon's ANSI positional codes over character translation. `MapVirtualKeyW`
//! would undo it: `MAPVK_VK_TO_VSC` maps through the *active layout*, so on a
//! Turkish-F keyboard `VK_A` returns the scancode of whichever key types 'a',
//! not the key in A's position.
//!
//! Literal text never comes through here — `input::type_text` attaches unicode
//! to the event directly and bypasses layouts entirely.
//!
//! ## Encoding
//!
//! Extended keys (the arrow cluster, navigation block, right-hand modifiers)
//! are prefixed with `0xE0` on the wire. That prefix is carried in the high
//! byte of the returned `u16`, and [`is_extended`] unpacks it.

/// Marks a scancode as needing the `0xE0` prefix and `KEYEVENTF_EXTENDEDKEY`.
const EXT: u16 = 0xE000;

pub fn lookup(name: &str) -> Option<u16> {
    let n = name.trim().to_ascii_lowercase();
    Some(match n.as_str() {
        // letters — QWERTY positions, per the module note above
        "q" => 0x10, "w" => 0x11, "e" => 0x12, "r" => 0x13, "t" => 0x14,
        "y" => 0x15, "u" => 0x16, "i" => 0x17, "o" => 0x18, "p" => 0x19,
        "a" => 0x1E, "s" => 0x1F, "d" => 0x20, "f" => 0x21, "g" => 0x22,
        "h" => 0x23, "j" => 0x24, "k" => 0x25, "l" => 0x26,
        "z" => 0x2C, "x" => 0x2D, "c" => 0x2E, "v" => 0x2F, "b" => 0x30,
        "n" => 0x31, "m" => 0x32,

        // digits
        "1" => 0x02, "2" => 0x03, "3" => 0x04, "4" => 0x05, "5" => 0x06,
        "6" => 0x07, "7" => 0x08, "8" => 0x09, "9" => 0x0A, "0" => 0x0B,

        // punctuation
        "-" | "minus" => 0x0C,
        "=" | "equal" => 0x0D,
        "[" | "leftbracket" => 0x1A,
        "]" | "rightbracket" => 0x1B,
        "\\" | "backslash" => 0x2B,
        ";" | "semicolon" => 0x27,
        "'" | "quote" => 0x28,
        "," | "comma" => 0x33,
        "." | "period" => 0x34,
        "/" | "slash" => 0x35,
        "`" | "grave" => 0x29,

        // editing & navigation
        "return" | "enter" => 0x1C,
        "tab" => 0x0F,
        "space" => 0x39,
        "delete" | "backspace" => 0x0E,
        "escape" | "esc" => 0x01,
        "forwarddelete" => EXT | 0x53,
        "home" => EXT | 0x47,
        "end" => EXT | 0x4F,
        "pageup" => EXT | 0x49,
        "pagedown" => EXT | 0x51,
        "insert" => EXT | 0x52,
        "left" => EXT | 0x4B,
        "right" => EXT | 0x4D,
        "down" => EXT | 0x50,
        "up" => EXT | 0x48,

        // function keys
        "f1" => 0x3B, "f2" => 0x3C, "f3" => 0x3D, "f4" => 0x3E, "f5" => 0x3F,
        "f6" => 0x40, "f7" => 0x41, "f8" => 0x42, "f9" => 0x43, "f10" => 0x44,
        "f11" => 0x57, "f12" => 0x58,

        _ => return None,
    })
}

/// Whether a code from [`lookup`] needs `KEYEVENTF_EXTENDEDKEY`.
pub(super) fn is_extended(code: u16) -> bool {
    code & 0xFF00 == EXT
}

/// The byte to put on the wire, with any marker stripped.
pub(super) fn scancode(code: u16) -> u16 {
    code & 0x00FF
}

/* ── modifier scancodes ── */

pub(super) const LSHIFT: u16 = 0x2A;
pub(super) const LCTRL: u16 = 0x1D;
pub(super) const LALT: u16 = 0x38;
/// Left Windows key — extended.
pub(super) const LWIN: u16 = EXT | 0x5B;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_are_extended_and_letters_are_not() {
        assert!(is_extended(lookup("up").unwrap()));
        assert!(is_extended(lookup("home").unwrap()));
        assert!(!is_extended(lookup("a").unwrap()));
        assert!(!is_extended(lookup("f1").unwrap()));
    }

    #[test]
    fn the_marker_never_leaks_into_the_wire_byte() {
        assert_eq!(scancode(lookup("up").unwrap()), 0x48);
        assert_eq!(scancode(lookup("a").unwrap()), 0x1E);
    }

    #[test]
    fn names_match_the_macos_table() {
        // Both backends must accept the same vocabulary, or a prompt written
        // against one silently fails against the other.
        for name in [
            "a", "z", "0", "9", "minus", "equal", "leftbracket", "backslash",
            "semicolon", "quote", "comma", "period", "slash", "grave", "return",
            "enter", "tab", "space", "delete", "backspace", "escape", "esc",
            "forwarddelete", "home", "end", "pageup", "pagedown", "left",
            "right", "up", "down", "f1", "f12",
        ] {
            assert!(lookup(name).is_some(), "{name} is missing from the table");
        }
    }
}
