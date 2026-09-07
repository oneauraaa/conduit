//! What this machine is, in the words an agent should use for it.
//!
//! Every one of these facts used to be a hardcoded "macOS" somewhere in
//! `mcp/tools.rs` — including the very first sentence the agent reads, which
//! told it flatly that it was driving a Mac. On the other two platforms that is
//! not a cosmetic wrong word: an agent that believes it is on macOS reaches for
//! `cmd`, looks for the Dock, and writes zsh into `run_shell`.
//!
//! Tool *descriptions* are compile-time literals in an attribute macro, so they
//! cannot carry any of this — which is exactly why they are now written
//! OS-neutrally and the specifics live here, in the `instructions` the server
//! hands over during the handshake.

/// The OS, named the way its own users name it. Fine detail belongs in
/// [`description`]; this is the short label.
pub const OS: &str = "macOS";

/// The machine, as a noun an instruction can be written around: "this Mac".
pub const DEVICE: &str = "Mac";

/// What `run_shell` actually spawns. See `shell::command`.
pub const SHELL: &str = "zsh";

/// The real key behind the `"cmd"` modifier token.
pub const SHORTCUT_MODIFIER: &str = "Command";

/// Where the control pill parks, so the sentence describing it is true.
pub const CHROME_ANCHOR: &str = "the Dock";

/// Where `read_screen_text` and `find_element` get their tree.
pub const AX_SOURCE: &str = "the macOS accessibility tree";

/// The full name, including anything only knowable at runtime.
///
/// A `String` rather than a const because Linux has a desktop environment to
/// name here and the other two do not — see that backend's copy.
pub fn description() -> String {
    OS.to_string()
}
