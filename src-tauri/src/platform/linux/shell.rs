//! The shell `run_shell` runs commands through.

/// A shell command, not yet spawned.
///
/// The user's own login shell, falling back to `/bin/sh`. macOS hardcodes zsh
/// because it is the login shell on every supported version; Linux has no such
/// guarantee — fish, zsh and bash are all common defaults — and a command the
/// user could paste into their terminal should behave the same here.
///
/// `-c` is the one flag every POSIX-ish shell agrees on, fish included.
pub fn command(script: &str) -> tokio::process::Command {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut cmd = tokio::process::Command::new(shell);
    cmd.arg("-c").arg(script);
    cmd
}
