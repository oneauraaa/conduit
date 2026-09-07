//! Which of the two input routes this machine actually has.
//!
//! Wayland has no single answer to "move the pointer", and the two that exist
//! are implemented by disjoint halves of the desktop:
//!
//! | route | who implements it | consent |
//! |---|---|---|
//! | [`super::wlroots`] — virtual-pointer + virtual-keyboard | hyprland, sway, river, wayfire | none needed |
//! | [`super::portal`] — `org.freedesktop.portal.RemoteDesktop` | gnome, kde | a dialog, every launch |
//!
//! Neither is a superset of the other and most machines have exactly one, so
//! this module picks and every caller in [`super::input`] goes through it.
//!
//! ## Why wlroots wins when both are there
//!
//! Because it costs the user nothing. The portal's grant cannot be made
//! permanent — a remote-desktop session is precisely the token upstream refuses
//! to persist — so every portal launch spends a dialog. The wlroots protocols
//! are advertised in the registry and need no prompt at all.
//!
//! That does *not* make the portal redundant. Screen capture has no wlroots
//! equivalent conduit can use, so `ScreenCast` is still asked for and the prompt
//! still appears at launch; what changes is that on a wlroots machine the
//! pointer keeps working whether or not the user answers it, and on a machine
//! whose portal has no `RemoteDesktop` at all — every wlroots desktop today —
//! input works where it previously could not work at all.
//!
//! ## Why the choice is made once
//!
//! A route that changed under a running session would be worse than either: the
//! two backends keep independent notions of modifier state and of which pointer
//! device is current, so a `key_press` that pressed Control through one and
//! released it through the other would leave Control stuck down for every
//! application on the machine. [`route`] resolves on first use and stays.

use std::sync::OnceLock;

use parking_lot::Mutex;

use super::{portal, wlroots};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// wlroots virtual-pointer and virtual-keyboard.
    Wlroots,
    /// The desktop portal's RemoteDesktop interface.
    Portal,
}

static ROUTE: OnceLock<Route> = OnceLock::new();

/// The first failure in an action survives cleanup releases. Clear it only
/// when the next high-level action starts, never on a successful key release.
#[derive(Default)]
struct ActionFailure(Mutex<Option<String>>);

impl ActionFailure {
    fn clear(&self) {
        *self.0.lock() = None;
    }

    fn record(&self, result: Result<(), String>) -> Result<(), String> {
        if let Err(why) = &result {
            self.0.lock().get_or_insert_with(|| why.clone());
        }
        result
    }

    fn reason(&self) -> Option<String> {
        self.0.lock().clone()
    }
}

static FAILURE: ActionFailure = ActionFailure(Mutex::new(None));

pub fn begin_action() {
    FAILURE.clear();
}

/// The route this machine uses, decided once.
///
/// Probing wlroots is cheap and silent — it binds two globals and uploads a
/// keymap, with no dialog and nothing the user can see — so it is safe to ask
/// first even on a desktop that will end up using the portal.
pub fn route() -> Route {
    *ROUTE.get_or_init(|| {
        if wlroots::available() {
            Route::Wlroots
        } else {
            Route::Portal
        }
    })
}

/// Whether input is usable, and why not when it isn't.
///
/// `None` means the route is working. The message is the one the readiness card
/// shows, so it names the route that was actually tried rather than a generic
/// "input is unavailable" that would send a hyprland user hunting for a portal
/// backend that does not exist.
pub fn blocked_reason() -> Option<String> {
    FAILURE.reason().or_else(route_blocked_reason)
}

fn route_blocked_reason() -> Option<String> {
    match route() {
        Route::Wlroots => wlroots::unavailable_reason(),
        Route::Portal => match portal::established() {
            Some(Err(e)) => Some(e.message()),
            // Not yet attempted, or attempted and still on screen: the warm-up
            // is waiting on the dialog.
            None => Some(portal::PortalError::Pending.message()),
            Some(Ok(())) => None,
        },
    }
}

/* ── the four operations `input` needs ──────────────────────── */

pub fn prepare_key(keysym: i32) -> Result<(), String> {
    FAILURE.record(match route() {
        Route::Wlroots => wlroots::prepare_key(keysym),
        Route::Portal => Ok(()),
    })
}

pub fn prepare_text(text: &str) -> Result<usize, String> {
    match route() {
        Route::Wlroots => wlroots::prepare_text(text),
        Route::Portal => Ok(text.len()),
    }.map_err(|why| {
        let _ = FAILURE.record(Err(why.clone()));
        why
    })
}

pub fn pointer_motion_absolute(x: f64, y: f64) -> Result<(), String> {
    FAILURE.record(match route() {
        Route::Wlroots => wlroots::pointer_motion_absolute(x, y),
        Route::Portal => portal::session()
            .map_err(|e| e.message())
            .and_then(|session| session.pointer_motion_absolute(x, y)),
    })
}

pub fn pointer_button(button: i32, pressed: bool) -> Result<(), String> {
    FAILURE.record(match route() {
        Route::Wlroots => wlroots::pointer_button(button, pressed),
        Route::Portal => portal::session()
            .map_err(|e| e.message())
            .and_then(|session| session.pointer_button(button, pressed)),
    })
}

pub fn pointer_axis_discrete(axis: u32, steps: i32) -> Result<(), String> {
    FAILURE.record(match route() {
        Route::Wlroots => wlroots::pointer_axis_discrete(axis, steps),
        Route::Portal => portal::session()
            .map_err(|e| e.message())
            .and_then(|session| session.pointer_axis_discrete(axis, steps)),
    })
}

pub fn keyboard_keysym(keysym: i32, pressed: bool) -> Result<(), String> {
    FAILURE.record(match route() {
        Route::Wlroots => wlroots::keyboard_keysym(keysym, pressed),
        Route::Portal => portal::session()
            .map_err(|e| e.message())
            .and_then(|session| session.keyboard_keysym(keysym, pressed)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_cannot_erase_an_input_failure() {
        let failure = ActionFailure::default();
        assert!(failure.record(Err("keymap failed".into())).is_err());
        assert!(failure.record(Ok(())).is_ok());
        assert!(failure.record(Err("release failed".into())).is_err());
        assert_eq!(failure.reason().as_deref(), Some("keymap failed"));
        failure.clear();
        assert_eq!(failure.reason(), None);
    }
}
