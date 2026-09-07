//! The Hyprland half of "where are conduit's own windows".
//!
//! `kwin.rs` is the same idea for Plasma, and the two are the only routes there
//! are: no Wayland protocol reports another client's geometry, or even a
//! client's own, so a compositor's private interface is the whole answer.
//!
//! Deliberately separate from the crate-level `hyprland` module, which reads
//! keybinds out of the user's config. This one asks the *running* compositor a
//! single question and is used by the platform layer, so it lives with the rest
//! of the platform layer.

use std::process::Command;

use serde::Deserialize;

/// Whether this session is Hyprland with a reachable control socket.
///
/// The environment variable rather than `XDG_CURRENT_DESKTOP`: it is set by
/// Hyprland itself for the instance that is actually running, so it cannot be
/// inherited from a stale login or spoofed by a desktop file.
pub fn available() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
}

#[derive(Deserialize)]
struct Client {
    at: (f64, f64),
    size: (f64, f64),
    pid: i32,
    mapped: bool,
    hidden: bool,
}

/// The on-screen rectangles belonging to a process.
///
/// `mapped && !hidden` is the same rule `list_windows` applies through KWin's
/// `minimized`: a window on another workspace or rolled up to the tray is not
/// on screen, so it is not something a click can land on.
pub fn rects_for_pid(pid: i32) -> Result<Vec<(f64, f64, f64, f64)>, String> {
    let output = Command::new("hyprctl")
        .args(["clients", "-j"])
        .output()
        .map_err(|e| format!("could not run hyprctl: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "hyprctl exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let clients: Vec<Client> = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))
        .map_err(|e| format!("could not parse hyprctl's reply: {e}"))?;

    Ok(clients
        .into_iter()
        .filter(|c| c.pid == pid && c.mapped && !c.hidden)
        .filter(|c| c.size.0 >= 1.0 && c.size.1 >= 1.0)
        .map(|c| (c.at.0, c.at.1, c.size.0, c.size.1))
        .collect())
}
