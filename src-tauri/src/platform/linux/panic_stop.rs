//! The Linux half of the panic stop: a passive evdev reader watching Escape.
//!
//! The hold threshold and what happens when it trips live in
//! `crate::panic_stop`; this file only reports the key's up/down edges.
//!
//! ## Why not a global shortcut
//!
//! `org.freedesktop.portal.GlobalShortcuts` exists, KDE implements it, and it
//! is the obvious answer. It is the wrong one, for the reason the shared
//! `panic_stop` module already states: a registered shortcut is **consumed**.
//! Binding Escape would take it away from every dialog, every vim session and
//! every video player on the machine for as long as conduit runs. Worse, it
//! would swallow the Escape keystrokes conduit itself injects, so an agent
//! could never send Escape to anything — and each one it tried would trip the
//! panic stop.
//!
//! So this reads the key without taking it, which on Linux means evdev.
//!
//! ## Why this needs a group and the other two backends don't
//!
//! Wayland has no passive keyboard listener by design — a client that could
//! watch keys it does not own is a keylogger. That is a deliberate and correct
//! restriction, and it applies to conduit too. The kernel's input devices are
//! below that line and readable with the right group membership, which is why
//! this is the one capability on Linux that needs a one-time `usermod`.
//!
//! When the group is missing this returns `false`, which is not fatal: `lib.rs`
//! retries every few seconds, the readiness card shows the exact command, and
//! the pill's Stop button remains the way out. That is the same contract the
//! macOS backend has before Accessibility is granted.
//!
//! Only *key codes* are read, never state, and only `KEY_ESC` is acted on.

use std::sync::atomic::{AtomicBool, Ordering};

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Whether any keyboard device is readable.
///
/// Distinguishes "no permission" from "no keyboard", which the readiness card
/// needs in order to say something true.
pub fn readable() -> bool {
    !keyboards().is_empty()
}

/// The one-time fix, for the readiness card.
pub fn how_to_enable() -> String {
    format!(
        "hold-escape needs read access to the keyboard device. run:\n  \
         sudo usermod -aG input {}\nthen log out and back in. \
         until then, the stop button on the pill is the way to take control back.",
        std::env::var("USER").unwrap_or_else(|_| "$USER".into())
    )
}

/// Every input device that looks like a keyboard.
///
/// Filtered by capability rather than by name: `/dev/input/event*` includes
/// mice, touchpads, power buttons and a lid switch, and matching on names like
/// "keyboard" misses most real ones. A device that reports `KEY_ESC` among its
/// keys is a keyboard for this purpose, whatever it calls itself.
fn keyboards() -> Vec<evdev::Device> {
    let Ok(entries) = std::fs::read_dir("/dev/input") else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("event"))
        {
            continue;
        }
        let Ok(device) = evdev::Device::open(&path) else {
            // Almost always EACCES — the missing group. Not logged per device,
            // or a machine with twenty of them logs twenty times a retry.
            continue;
        };
        let has_escape = device
            .supported_keys()
            .is_some_and(|keys| keys.contains(evdev::KeyCode::KEY_ESC));
        if has_escape {
            out.push(device);
        }
    }
    out
}

/// Starts one reader thread per keyboard.
///
/// Returns whether at least one could be opened. A `false` is not fatal — see
/// the module header.
pub fn install_hook() -> bool {
    if RUNNING.load(Ordering::SeqCst) {
        return true;
    }

    let devices = keyboards();
    if devices.is_empty() {
        return false;
    }

    RUNNING.store(true, Ordering::SeqCst);

    for mut device in devices {
        let name = device
            .name()
            .unwrap_or("keyboard")
            .to_string();

        let spawned = std::thread::Builder::new()
            .name("conduit-panic-stop".into())
            .spawn(move || {
                tracing::info!(device = %name, "watching for hold-escape");
                loop {
                    // Blocking read. One thread per keyboard rather than a poll
                    // set: there are rarely more than two, and a blocked thread
                    // costs nothing while the user is not typing.
                    let events = match device.fetch_events() {
                        Ok(events) => events,
                        Err(e) => {
                            // A device unplugged mid-session ends its thread
                            // cleanly; the others carry on.
                            tracing::info!(device = %name, "keyboard reader stopped: {e}");
                            return;
                        }
                    };

                    for event in events {
                        let evdev::EventSummary::Key(_, key, value) = event.destructure()
                        else {
                            continue;
                        };
                        if key != evdev::KeyCode::KEY_ESC {
                            continue;
                        }
                        match value {
                            // 1 is press, 2 is auto-repeat. `escape_down` is a
                            // compare-exchange, so repeats are harmless, but
                            // there is no reason to send them.
                            1 => crate::panic_stop::escape_down(),
                            0 => crate::panic_stop::escape_up(),
                            _ => {}
                        }
                    }
                }
            })
            .is_ok();

        if !spawned {
            tracing::warn!("could not start a keyboard reader thread");
        }
    }

    true
}

