//! The Windows backend. Everything that touches Win32 or WinRT lives behind
//! this module.
//!
//! Module names and signatures mirror `platform/mac` exactly; see
//! `platform/contract.rs`, which fails to compile if they ever drift.
//!
//! ## Coordinate space
//!
//! Physical pixels on the virtual screen, throughout. Every API here already
//! speaks them — `GetMonitorInfoW`, `GetCursorPos`, `SendInput`'s
//! `VIRTUALDESK` normalisation, WGC frames, UIA bounding rectangles,
//! `DWMWA_EXTENDED_FRAME_BOUNDS` — so converting to logical units would only
//! add a place to apply the wrong monitor's scale. `Display::scale` is
//! reported for the agent's information and is never divided by in here.

pub mod appicon;
pub mod apps;
pub mod ax;
pub mod capture;
pub mod clipboard;
pub mod com;
pub mod cursor;
pub mod d3d;
pub mod input;
pub mod keycodes;
pub mod panic_stop;
pub mod permissions;
pub mod screen;
pub mod shell;
pub mod startmenu;

/// The error a not-yet-ported tool returns.
///
/// An explicit refusal, never a plausible-looking empty result: the whole point
/// of the gate is that an agent can trust what comes back, and "no windows
/// found" would be a lie that costs it a retry loop.
#[allow(dead_code)]
pub(crate) fn unported(what: &str) -> String {
    format!("{what} is not implemented on Windows yet")
}
