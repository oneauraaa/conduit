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

use super::{portal, wlroots};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// wlroots virtual-pointer and virtual-keyboard.
    Wlroots,
    /// The desktop portal's RemoteDesktop interface.
    Portal,
}

static ROUTE: OnceLock<Route> = OnceLock::new();

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

pub fn pointer_motion_absolute(x: f64, y: f64) -> Result<(), String> {
    match route() {
        Route::Wlroots => wlroots::pointer_motion_absolute(x, y),
        Route::Portal => portal::session()
            .map_err(|e| e.message())?
            .pointer_motion_absolute(x, y),
    }
}

pub fn pointer_button(button: i32, pressed: bool) -> Result<(), String> {
    match route() {
        Route::Wlroots => wlroots::pointer_button(button, pressed),
        Route::Portal => portal::session()
            .map_err(|e| e.message())?
            .pointer_button(button, pressed),
    }
}

pub fn pointer_axis_discrete(axis: u32, steps: i32) -> Result<(), String> {
    match route() {
        Route::Wlroots => wlroots::pointer_axis_discrete(axis, steps),
        Route::Portal => portal::session()
            .map_err(|e| e.message())?
            .pointer_axis_discrete(axis, steps),
    }
}

pub fn keyboard_keysym(keysym: i32, pressed: bool) -> Result<(), String> {
    match route() {
        Route::Wlroots => wlroots::keyboard_keysym(keysym, pressed),
        Route::Portal => portal::session()
            .map_err(|e| e.message())?
            .keyboard_keysym(keysym, pressed),
    }
}
