//! The types that cross the platform boundary.
//!
//! Every one of these is serialized into an MCP tool result, so they are
//! defined once here rather than twice in the two backends. That is the whole
//! point: a Windows backend physically cannot drift from the macOS one on JSON
//! shape, because there is only one shape.
//!
//! ## Coordinate space
//!
//! Both backends speak a single space, and every geometric value in this file
//! is in it — display bounds, window bounds, element bounds, cursor position,
//! click targets, screenshot regions.
//!
//!   - **macOS**: Quartz points. Origin top-left of the primary display, y down.
//!     Logical, so a Retina panel reports its point size.
//!   - **Windows**: physical pixels on the virtual screen. Origin top-left of
//!     the primary monitor, y down; monitors to the left or above have negative
//!     coordinates. Physical, because every Win32 and WinRT API in the port
//!     already speaks pixels — converting would just add a place to apply the
//!     wrong monitor's scale.
//!
//! What matters to an agent is not which of the two it gets, but that all of
//! them agree. They do.

use serde::Serialize;

/* ── displays ── */

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Display {
    pub index: usize,
    /// Top-left origin, in the coordinate space described above.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Backing scale. On macOS this is the Retina factor and the bounds above
    /// are already divided by it. On Windows it is `dpi / 96.0` and the bounds
    /// are *not* — there it is informational, telling the agent "this monitor
    /// runs at 150%" and nothing more.
    pub scale: f64,
    pub primary: bool,
}

impl Display {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// Which display a point falls on. Falls back to the primary rather than
/// failing: a point just off the edge of a screen is far more likely to be a
/// rounding artefact than a real error, and refusing it would strand the agent.
pub fn display_at(displays: &[Display], x: f64, y: f64) -> usize {
    displays
        .iter()
        .find(|d| d.contains(x, y))
        .map(|d| d.index)
        .unwrap_or(0)
}

/// Where conduit's own windows are, for [`crate::chrome::point_hits_conduit`].
///
/// Three answers rather than a `Vec`, because "I do not know" and "there are
/// none" have to be told apart: the first must not be read as *"nothing here is
/// conduit, click freely"*, and the second is a real state — every window
/// hidden to the tray.
// Each build constructs only its own platform's variant, so the other two
// always look dead. That is the point of a shared enum.
#[allow(dead_code)]
pub enum OwnWindows {
    /// The window manager's own answer, in conduit's coordinate space.
    Rects(Vec<(f64, f64, f64, f64)>),
    /// This platform reports its own window geometry correctly, so Tauri's
    /// numbers are authoritative and the caller should use them.
    AskTauri,
    /// Nothing on this system will say. Wayland withholds a client's own
    /// position by design — `outer_position()` answers `(0, 0)` rather than
    /// failing — so on a compositor conduit cannot interrogate there is no
    /// honest rectangle to guard.
    Unknown,
}

/* ── capture ── */

pub struct Shot {
    pub png: Vec<u8>,
    /// Pixel dimensions of the encoded image (after any downscale).
    pub width: u32,
    pub height: u32,
}

/* ── accessibility ── */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Element {
    /// Whatever label the control actually presents: title, value or description.
    pub role: String,
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Centre point, ready to hand straight to `click`.
    pub center_x: f64,
    pub center_y: f64,
}

/// Ranks elements against a query, best match first, capped at 20.
///
/// Ranking rather than raw filtering matters: searching "Save" in a save dialog
/// hits both "Save" and "Save As…", and the agent wants the exact one. Shared
/// so both backends rank identically — an agent's prompt should not have to
/// know which OS it is driving.
pub fn rank_matches(query: &str, elements: Vec<Element>) -> Vec<Element> {
    let needle = query.trim().to_lowercase();

    let mut matches: Vec<(u8, Element)> = elements
        .into_iter()
        .filter_map(|e| {
            let hay = e.text.to_lowercase();
            let rank = if hay == needle {
                0
            } else if hay.starts_with(&needle) {
                1
            } else if hay.contains(&needle) {
                2
            } else {
                return None;
            };
            Some((rank, e))
        })
        .collect();

    matches.sort_by_key(|(rank, _)| *rank);
    matches.into_iter().map(|(_, e)| e).take(20).collect()
}

/* ── windows & apps ── */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub app: String,
    pub pid: i32,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Front-to-back ordering; 0 is frontmost.
    pub layer_index: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub pid: i32,
    /// macOS bundle identifier. Always `None` on Windows, which has no
    /// equivalent — the field stays so the tool result keeps one shape.
    pub bundle_id: Option<String>,
    pub active: bool,
}

/* ── input ── */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

/// Modifier keys, parsed here rather than in either backend so both accept
/// exactly the same names and reject unknown ones with exactly the same
/// message.
///
/// [`Modifiers::cmd`] is *the shortcut modifier*, not a physical key: Command
/// on macOS, Control on Windows. An agent that learned `cmd+c` means copy
/// should not have to relearn anything to drive a PC, and an agent that asks
/// for `ctrl+c` on Windows gets the same keystroke by the other name.
///
/// [`Modifiers::win`] is the separate "logo" key — the one Windows opens the
/// Start menu with. It collapses onto Command on macOS, where that *is* the
/// logo key. Keeping it apart from `cmd` is what lets an agent ask for `win+e`
/// without also asking for copy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub cmd: bool,
    pub win: bool,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub func: bool,
}

impl Modifiers {
    pub fn parse(modifiers: &[String]) -> Result<Self, String> {
        let mut out = Self::default();
        for m in modifiers {
            match m.trim().to_ascii_lowercase().as_str() {
                "cmd" | "command" => out.cmd = true,
                "meta" | "super" | "win" | "windows" => out.win = true,
                "shift" => out.shift = true,
                "alt" | "option" | "opt" => out.alt = true,
                "ctrl" | "control" => out.ctrl = true,
                "fn" | "function" => out.func = true,
                other => return Err(format!("unknown modifier: {other}")),
            }
        }
        Ok(out)
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(text: &str) -> Element {
        Element {
            role: "button".into(),
            text: text.into(),
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            center_x: 5.0,
            center_y: 5.0,
        }
    }

    #[test]
    fn exact_matches_outrank_prefixes_and_substrings() {
        let got = rank_matches(
            "save",
            vec![el("Save As…"), el("Autosave"), el("Save"), el("Cancel")],
        );
        let texts: Vec<_> = got.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, ["Save", "Save As…", "Autosave"]);
    }

    #[test]
    fn modifier_aliases_agree() {
        let cmd = Modifiers::parse(&["cmd".into()]).unwrap();
        assert_eq!(cmd, Modifiers::parse(&["Command".into()]).unwrap());
        assert!(Modifiers::parse(&["hyper".into()]).is_err());
    }

    #[test]
    fn the_logo_key_is_not_the_shortcut_key() {
        // They collapse onto Command on macOS but are Ctrl and Win on Windows,
        // so they must stay distinct through parsing.
        let cmd = Modifiers::parse(&["cmd".into()]).unwrap();
        let win = Modifiers::parse(&["super".into()]).unwrap();
        assert_ne!(cmd, win);
        assert_eq!(win, Modifiers::parse(&["windows".into()]).unwrap());
        assert_eq!(win, Modifiers::parse(&["meta".into()]).unwrap());
    }

    #[test]
    fn a_point_off_every_display_falls_back_to_primary() {
        let displays = [Display {
            index: 0,
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
            scale: 1.0,
            primary: true,
        }];
        assert_eq!(display_at(&displays, 50.0, 50.0), 0);
        assert_eq!(display_at(&displays, 500.0, 50.0), 0);
    }
}
