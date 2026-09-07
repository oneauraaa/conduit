//! Readiness checks.
//!
//! Linux has no TCC and no UAC. What it has is a set of capabilities that are
//! each granted by a different mechanism, any of which can be absent while the
//! app otherwise runs perfectly — and every one of them fails *silently* at the
//! point of use if it isn't reported here first.
//!
//! Five things, in the order they matter:
//!
//!   1. **The portal session.** Input and capture both. Without it conduit can
//!      see and touch nothing. Granted by answering one dialog, and — unlike
//!      every other portal grant — it cannot be made permanent, so it is asked
//!      again on every launch. See [`super::portal`].
//!   2. **Frames arriving.** A session can be live while capture is not, if the
//!      compositor negotiated a buffer type conduit cannot map.
//!   3. **KWin.** Window listing, moving and focusing. Absent on GNOME and
//!      wlroots, where those three tools report as unsupported and the rest
//!      still works.
//!   4. **AT-SPI.** Off by default on Plasma; see [`super::ax`].
//!   5. **evdev.** The hold-Escape panic stop; see [`super::panic_stop`].

use crate::state::Readiness;

/// Whether this is a Wayland session at all.
///
/// conduit works under XWayland-less Wayland only. On an X11 session the portal
/// route still functions on most desktops, so this is reported rather than
/// refused — but it explains a great deal when something behaves oddly.
pub fn wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|s| s.eq_ignore_ascii_case("wayland"))
        .unwrap_or_else(|_| std::env::var_os("WAYLAND_DISPLAY").is_some())
}

/// The desktop's own name, for the readiness card.
pub fn desktop() -> String {
    std::env::var("XDG_CURRENT_DESKTOP")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

/// Whether the portal has granted a session.
///
/// Never *starts* one — the card polls this every two seconds and a poll that
/// raised a consent dialog would be indefensible.
pub fn screen_recording_granted() -> bool {
    matches!(super::portal::established(), Some(Ok(())))
}

/// The same grant. On Linux input and capture come from one session, so these
/// two cannot disagree — they stay separate because the contract has both and
/// the UI asks them independently.
pub fn accessibility_granted() -> bool {
    screen_recording_granted()
}

/// Asks for the portal session, raising the dialog if it has not been answered.
///
/// Both `prompt_*` functions do the same thing for the same reason as above.
/// The blocking wait runs on a background thread so the UI thread that invoked
/// the command is not held for however long the user takes to answer.
pub fn prompt_screen_recording() -> bool {
    if screen_recording_granted() {
        return true;
    }
    super::portal::retry();
    super::portal::warm_up();
    false
}

pub fn prompt_accessibility() -> bool {
    prompt_screen_recording()
}

pub fn snapshot() -> Readiness {
    let session = super::portal::established();

    Readiness::Linux {
        desktop: desktop(),
        wayland: wayland(),
        portal_ready: matches!(session, Some(Ok(()))),
        portal_error: match session {
            Some(Err(e)) => Some(e.message()),
            // Not attempted yet: the warm-up is still waiting on the dialog.
            None => None,
            Some(Ok(())) => None,
        },
        input_route: match super::sink::route() {
            super::sink::Route::Wlroots => "wlroots".into(),
            super::sink::Route::Portal => "portal".into(),
        },
        input_ready: super::sink::blocked_reason().is_none(),
        input_error: super::sink::blocked_reason(),
        capture_ready: super::capture::has_frames(),
        window_management: super::kwin::available(),
        accessibility_tree: super::ax::enabled(),
        accessibility_hint: super::ax::how_to_enable().to_string(),
        panic_stop: super::panic_stop::readable(),
        panic_stop_hint: super::panic_stop::how_to_enable(),
    }
}

/// No-op on Linux: there is no elevation to gain.
///
/// Running conduit as root would not help and would hurt — the portal session,
/// the accessibility bus and the clipboard all live in the *user's* session,
/// and a root process is not in it.
pub fn relaunch_elevated(_app: &tauri::AppHandle<tauri::Wry>) -> Result<(), String> {
    Err("elevation is a windows concept; on linux conduit needs the screen-sharing \
         prompt answered, and running it as root would break more than it fixes."
        .into())
}
