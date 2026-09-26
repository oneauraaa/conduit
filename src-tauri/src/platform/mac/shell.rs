//! The shell `run_shell` runs commands through.

/// A shell command, not yet spawned.
///
/// zsh rather than sh because it is the macOS login shell, so a command the
/// user could paste into Terminal behaves the same here.
pub fn command(
    script: &str,
    requested_shell: Option<&str>,
) -> Result<tokio::process::Command, String> {
    if requested_shell.is_some() {
        return Err("shell selection is available only on Linux".into());
    }
    let mut cmd = tokio::process::Command::new("/bin/zsh");
    cmd.arg("-c").arg(script);
    Ok(cmd)
}
