//! The shell `run_shell` runs commands through.

/// A shell command, not yet spawned.
///
/// zsh rather than sh because it is the macOS login shell, so a command the
/// user could paste into Terminal behaves the same here.
pub fn command(script: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("/bin/zsh");
    cmd.arg("-c").arg(script);
    cmd
}
