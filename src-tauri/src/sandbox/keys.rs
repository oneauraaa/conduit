//! conduit's key names, as `xdotool key` spells them.
//!
//! The vocabulary is exactly the Linux backend's (`platform::linux::keycodes`)
//! so a prompt that works against a Linux host works against a sandbox, and a
//! test holds the two together. `cmd` is the shortcut modifier — Control here,
//! as on every non-Mac — and `fn` is accepted and dropped, since no keysym
//! exists for it.

use crate::platform::types::Modifiers;

/// The X keysym name for a conduit key name, or `None` if it is not one.
pub fn keysym_name(name: &str) -> Option<String> {
    let n = name.trim().to_ascii_lowercase();
    let mut chars = n.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            return Some(c.to_string());
        }
    }
    Some(
        match n.as_str() {
            "-" | "minus" => "minus",
            "=" | "equal" => "equal",
            "[" | "leftbracket" => "bracketleft",
            "]" | "rightbracket" => "bracketright",
            "\\" | "backslash" => "backslash",
            ";" | "semicolon" => "semicolon",
            "'" | "quote" => "apostrophe",
            "," | "comma" => "comma",
            "." | "period" => "period",
            "/" | "slash" => "slash",
            "`" | "grave" => "grave",
            "return" | "enter" => "Return",
            "tab" => "Tab",
            "space" => "space",
            "delete" | "backspace" => "BackSpace",
            "escape" | "esc" => "Escape",
            "forwarddelete" => "Delete",
            "home" => "Home",
            "end" => "End",
            "pageup" => "Prior",
            "pagedown" => "Next",
            "left" => "Left",
            "up" => "Up",
            "right" => "Right",
            "down" => "Down",
            "insert" => "Insert",
            "f1" => "F1",
            "f2" => "F2",
            "f3" => "F3",
            "f4" => "F4",
            "f5" => "F5",
            "f6" => "F6",
            "f7" => "F7",
            "f8" => "F8",
            "f9" => "F9",
            "f10" => "F10",
            "f11" => "F11",
            "f12" => "F12",
            _ => return None,
        }
        .to_string(),
    )
}

/// `ctrl+shift+t`, in the press order the Linux backend uses.
pub fn chord(key: &str, modifiers: &[String]) -> Result<String, String> {
    let sym = keysym_name(key).ok_or_else(|| format!("unknown key: {key}"))?;
    let m = Modifiers::parse(modifiers)?;
    let mut parts: Vec<&str> = Vec::new();
    if m.cmd || m.ctrl {
        parts.push("ctrl");
    }
    if m.win {
        parts.push("super");
    }
    if m.alt {
        parts.push("alt");
    }
    if m.shift {
        parts.push("shift");
    }
    let mut out = parts.join("+");
    if !out.is_empty() {
        out.push('+');
    }
    out.push_str(&sym);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_is_control_and_order_is_stable() {
        let m = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(chord("c", &m(&["cmd"])).unwrap(), "ctrl+c");
        assert_eq!(chord("t", &m(&["shift", "cmd"])).unwrap(), "ctrl+shift+t");
        assert_eq!(chord("e", &m(&["win"])).unwrap(), "super+e");
        assert_eq!(chord("return", &[]).unwrap(), "Return");
        assert_eq!(chord("f4", &m(&["alt"])).unwrap(), "alt+F4");
        assert_eq!(chord("a", &m(&["fn"])).unwrap(), "a");
        assert!(chord("dpad-up", &[]).is_err());
        assert!(chord("a", &m(&["hyper"])).is_err());
    }

    /// Every name the Linux host accepts, the sandbox accepts, and they mean
    /// the same key: the keysym *value* behind each name here must be the one
    /// `keycodes::lookup` sends.
    #[cfg(target_os = "linux")]
    #[test]
    fn agrees_with_the_linux_backend() {
        use crate::platform::keycodes::lookup;
        // keysymdef.h values for the names above.
        let value_of = |name: &str| -> u16 {
            match name {
                "minus" => 0x2d,
                "equal" => 0x3d,
                "bracketleft" => 0x5b,
                "bracketright" => 0x5d,
                "backslash" => 0x5c,
                "semicolon" => 0x3b,
                "apostrophe" => 0x27,
                "comma" => 0x2c,
                "period" => 0x2e,
                "slash" => 0x2f,
                "grave" => 0x60,
                "Return" => 0xff0d,
                "Tab" => 0xff09,
                "space" => 0x20,
                "BackSpace" => 0xff08,
                "Escape" => 0xff1b,
                "Delete" => 0xffff,
                "Home" => 0xff50,
                "End" => 0xff57,
                "Prior" => 0xff55,
                "Next" => 0xff56,
                "Left" => 0xff51,
                "Up" => 0xff52,
                "Right" => 0xff53,
                "Down" => 0xff54,
                "Insert" => 0xff63,
                f if f.starts_with('F') => 0xffbe + f[1..].parse::<u16>().unwrap() - 1,
                c if c.len() == 1 => c.as_bytes()[0] as u16,
                other => panic!("no value for {other}"),
            }
        };
        let names = [
            "a",
            "z",
            "0",
            "9",
            "-",
            "minus",
            "=",
            "[",
            "]",
            "\\",
            ";",
            "'",
            ",",
            ".",
            "/",
            "`",
            "return",
            "enter",
            "tab",
            "space",
            "delete",
            "backspace",
            "escape",
            "esc",
            "forwarddelete",
            "home",
            "end",
            "pageup",
            "pagedown",
            "left",
            "up",
            "right",
            "down",
            "insert",
            "f1",
            "f5",
            "f12",
            "quote",
            "grave",
            "comma",
        ];
        for name in names {
            let host = lookup(name).unwrap_or_else(|| panic!("host lost {name}"));
            let sym = keysym_name(name).unwrap_or_else(|| panic!("sandbox lost {name}"));
            assert_eq!(value_of(&sym), host, "{name} means different keys");
        }
        for unknown in ["dpad-up", "hyper", "f13", "capslock"] {
            assert_eq!(
                keysym_name(unknown).is_some(),
                lookup(unknown).is_some(),
                "{unknown}"
            );
        }
    }
}
