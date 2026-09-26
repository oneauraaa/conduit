//! The shell `run_shell` runs commands through.

/// A shell command, not yet spawned.
///
/// The requested shell, or the user's login shell when omitted. Linux users may
/// have fish, zsh, bash, or another shell installed; a bare name is resolved on
/// PATH, while an executable path is used directly. Falls back to `/bin/sh` if
/// SHELL is unset.
///
/// `-c` is the one flag every POSIX-ish shell agrees on, fish included.
pub fn command(
    script: &str,
    requested_shell: Option<&str>,
) -> Result<tokio::process::Command, String> {
    if requested_shell.is_some_and(|shell| shell.trim().is_empty()) {
        return Err("shell must be an executable name or path".into());
    }
    let shell = requested_shell
        .map(str::to_owned)
        .unwrap_or_else(|| std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()));
    let mut cmd = tokio::process::Command::new(shell);
    cmd.arg("-c").arg(script);
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_shell_and_script_are_separate_process_arguments() {
        let cmd = command("printf hello", Some("fish")).unwrap();
        let cmd = cmd.as_std();
        assert_eq!(cmd.get_program(), "fish");
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), ["-c", "printf hello"]);
    }

    #[test]
    fn omitted_shell_uses_the_existing_default() {
        let cmd = command("true", None).unwrap();
        let expected = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        assert_eq!(cmd.as_std().get_program(), expected.as_str());
    }

    #[test]
    fn empty_shell_is_rejected() {
        assert!(command("true", Some("")).is_err());
        assert!(command("true", Some("  ")).is_err());
    }

    #[tokio::test]
    async fn selected_shell_executes_the_script() {
        let output = command("printf selected", Some("/bin/sh"))
            .unwrap()
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"selected");
    }
}
