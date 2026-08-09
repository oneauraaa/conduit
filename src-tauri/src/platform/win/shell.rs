//! The shell `run_shell` runs commands through.


/// Suppresses the console window a child process would otherwise flash on
/// screen. Without it every `run_shell` call pops a black box — on top of the
/// overlay glow, in the middle of a session the user is watching.
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A shell command, not yet spawned.
///
/// PowerShell rather than `cmd`: it is what a Windows user would paste into a
/// terminal, and `cmd`'s quoting rules would mangle most commands an agent
/// writes.
///
/// The encoding prefix is not optional. Windows PowerShell 5.1 — still the
/// default on a stock Windows 11 — writes stdout in the console's OEM codepage,
/// so any non-ASCII output would reach `from_utf8_lossy` as mojibake. Forcing
/// UTF-8 on both the input and output encodings makes it round-trip.
pub fn command(script: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("powershell.exe");
    cmd.arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(format!(
            "$OutputEncoding=[Console]::OutputEncoding=[Text.Encoding]::UTF8; {script}"
        ))
        .creation_flags(CREATE_NO_WINDOW);
    cmd
}
