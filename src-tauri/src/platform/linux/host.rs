//! What this machine is, in the words an agent should use for it.
//! See `platform/mac/host.rs` for why this module exists.

pub const OS: &str = "Linux";

pub const DEVICE: &str = "Linux machine";

/// What `run_shell` actually spawns — the user's own login shell, so naming a
/// specific one here would be a guess. See `shell::command`.
pub const SHELL: &str = "your login shell";

/// The real key behind the `"cmd"` modifier token. Linux has no Command key,
/// so the shortcut modifier every `cmd+…` combo lands on is Control.
pub const SHORTCUT_MODIFIER: &str = "Control";

/// "Panel" rather than "taskbar" or "Dock": it is the word every desktop
/// environment here uses for the thing, whichever one the user is running.
pub const CHROME_ANCHOR: &str = "your panel";

/// Where `read_screen_text` and `find_element` get their tree.
pub const AX_SOURCE: &str = "the AT-SPI accessibility tree";

/// Names the desktop as well as the OS.
///
/// This is the one platform where "Linux" alone is not enough for an agent to
/// reason with. Which desktop is running decides whether `list_windows`,
/// `focus_window` and `set_window_bounds` exist at all — they need KWin — so an
/// agent that knows it is on GNOME can stop trying to move windows instead of
/// retrying a tool that will never work. Session type matters for the same
/// reason: the input route differs entirely between Wayland and X11.
pub fn description() -> String {
    let desktop = super::permissions::desktop();
    let session = if super::permissions::wayland() { "Wayland" } else { "X11" };
    if desktop.is_empty() || desktop == "unknown" {
        format!("{OS} ({session})")
    } else {
        format!("{OS} ({desktop}, {session})")
    }
}
