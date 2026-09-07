//! What this machine is, in the words an agent should use for it.
//! See `platform/mac/host.rs` for why this module exists.

pub const OS: &str = "Windows";

pub const DEVICE: &str = "PC";

/// What `run_shell` actually spawns. See `shell::command`.
pub const SHELL: &str = "PowerShell";

/// The real key behind the `"cmd"` modifier token. Windows has no Command key,
/// so the shortcut modifier every `cmd+…` combo lands on is Control.
pub const SHORTCUT_MODIFIER: &str = "Control";

pub const CHROME_ANCHOR: &str = "the taskbar";

/// Where `read_screen_text` and `find_element` get their tree.
pub const AX_SOURCE: &str = "the UI Automation tree";

pub fn description() -> String {
    OS.to_string()
}
