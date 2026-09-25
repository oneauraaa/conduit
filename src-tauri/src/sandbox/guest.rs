//! Running things inside a sandbox.
//!
//! Every tool call is one `docker exec` with an argv — never a string handed
//! to a host shell, so nothing an agent sends is ever interpreted on the
//! user's machine. This is the seam a persistent in-guest agent would replace
//! if per-exec latency ever matters more than simplicity.

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::AsyncWriteExt;

use super::docker::{self, DockerCli};
use super::image::GUEST_PATH;

pub struct ExecOutput {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl ExecOutput {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }

    /// The last meaningful stderr line, for an error message.
    pub fn error_line(&self) -> String {
        if is_daemon_error(&self.stderr) {
            return daemon_message(&self.stderr);
        }
        let line = self
            .stderr
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .last()
            .map(str::to_string);
        line.unwrap_or_else(|| format!("exited with status {}", self.code.unwrap_or(-1)))
    }
}

/// Whether stderr came from docker itself rather than the program it ran.
/// `docker exec` reports "container not running" with the same exit code a
/// failing command might use, so the text is the only way to tell.
pub fn is_daemon_error(stderr: &str) -> bool {
    stderr.contains("Error response from daemon")
        || stderr.contains("failed to connect to the docker API")
        || stderr.contains("Cannot connect to the Docker daemon")
        || stderr.contains("error during connect")
}

fn daemon_message(stderr: &str) -> String {
    let detail = docker::docker_error(stderr);
    if detail.contains("is not running") || detail.contains("No such container") {
        "the sandbox is not running".into()
    } else {
        format!("docker: {detail}")
    }
}

#[derive(Clone)]
pub struct Guest {
    cli: Arc<DockerCli>,
    container: String,
}

impl Guest {
    pub fn new(cli: Arc<DockerCli>, id: &str) -> Self {
        Self {
            cli,
            container: docker::container_name(id),
        }
    }

    /// Runs `argv` in the sandbox as the guest user. The docker client is
    /// killed if the timeout passes or the future is dropped; the process
    /// inside is not, which is why long-running callers bound it on the guest
    /// side too (`timeout -k`).
    pub async fn exec(
        &self,
        argv: &[String],
        stdin: Option<Vec<u8>>,
        timeout: Duration,
    ) -> Result<ExecOutput, String> {
        let args = docker::exec_args(&self.container, stdin.is_some(), argv);
        let mut cmd = self.cli.command();
        cmd.args(&args);
        if stdin.is_some() {
            cmd.stdin(Stdio::piped());
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("could not run docker: {e}"))?;
        // Fed from its own task so a large input and a large output can never
        // wait on each other.
        if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
            tokio::spawn(async move {
                let _ = pipe.write_all(&bytes).await;
                let _ = pipe.shutdown().await;
            });
        }
        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(out)) => Ok(ExecOutput {
                code: out.status.code(),
                stdout: out.stdout,
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            }),
            Ok(Err(e)) => Err(format!("docker exec failed: {e}")),
            Err(_) => Err(format!("timed out after {}s", timeout.as_secs())),
        }
    }

    /// `exec`, succeeding only on exit status 0.
    pub async fn run(
        &self,
        argv: &[String],
        stdin: Option<Vec<u8>>,
        timeout: Duration,
    ) -> Result<Vec<u8>, String> {
        let out = self.exec(argv, stdin, timeout).await?;
        if out.success() {
            Ok(out.stdout)
        } else {
            Err(out.error_line())
        }
    }

    /// A `guest.py` subcommand.
    pub async fn helper(
        &self,
        args: &[String],
        stdin: Option<Vec<u8>>,
        timeout: Duration,
    ) -> Result<Vec<u8>, String> {
        let mut argv = vec!["python3".to_string(), GUEST_PATH.to_string()];
        argv.extend(args.iter().cloned());
        self.run(&argv, stdin, timeout).await
    }

    /// A `guest.py` subcommand whose stdout is JSON.
    pub async fn helper_json<T: serde::de::DeserializeOwned>(
        &self,
        args: &[String],
        timeout: Duration,
    ) -> Result<T, String> {
        let bytes = self.helper(args, None, timeout).await?;
        serde_json::from_slice(&bytes)
            .map_err(|e| format!("the sandbox helper answered something unreadable: {e}"))
    }
}

pub fn argv(list: &[&str]) -> Vec<String> {
    docker::args(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_errors_are_not_mistaken_for_command_failures() {
        let not_running = ExecOutput {
            code: Some(1),
            stdout: vec![],
            stderr: "Error response from daemon: container abc is not running\n".into(),
        };
        assert_eq!(not_running.error_line(), "the sandbox is not running");

        let command = ExecOutput {
            code: Some(1),
            stdout: vec![],
            stderr: "warning: something\nxdotool: unknown command\n".into(),
        };
        assert_eq!(command.error_line(), "xdotool: unknown command");

        let silent = ExecOutput {
            code: Some(3),
            stdout: vec![],
            stderr: String::new(),
        };
        assert_eq!(silent.error_line(), "exited with status 3");
    }
}
