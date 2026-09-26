//! A thin wrapper around the `docker` command-line tool.
//!
//! ## Why the CLI and not the Engine API
//!
//! The CLI already knows where this machine's daemon is. Docker Desktop, the
//! native Linux engine, OrbStack, Colima and Rancher Desktop all put their
//! socket somewhere different and tell the CLI through a *context*; talking to
//! the socket directly would mean re-implementing that resolution for each of
//! them. The CLI also brings BuildKit, credential helpers and proxy settings
//! for free.
//!
//! ## Finding it
//!
//! An app launched from Finder or Explorer does not inherit the user's shell
//! PATH, so `Command::new("docker")` finds nothing on a Mac where docker works
//! perfectly in Terminal. Discovery therefore tries absolute locations first,
//! exactly like `tailscale::cli()`, and every spawn puts the CLI's directory
//! (plus the usual bins) on PATH so credential helpers such as
//! `docker-credential-desktop` resolve too.
//!
//! Everything that builds an argument list is a pure function in this file, so
//! the security-relevant ones — what a sandbox container is created with — are
//! unit-tested rather than trusted.

use std::path::PathBuf;
use std::process::Stdio;

use serde::Deserialize;

use super::model::{DockerAvailability, DockerStatus, SandboxSpec};

/// The bridge network every sandbox with internet access joins. Created with
/// inter-container traffic off, so one sandbox cannot reach another.
pub const NETWORK: &str = "conduit-sandboxes";
/// On every container, volume and network conduit creates.
pub const LABEL_SANDBOX: &str = "ai.conduit.sandbox";
/// Ties a container to one conduit install, so two installs on one machine —
/// or a reinstall — never touch each other's sandboxes.
pub const LABEL_INSTALL: &str = "ai.conduit.install";
pub const LABEL_IMAGE: &str = "ai.conduit.image";
pub const LABEL_NETWORK: &str = "ai.conduit.network";
/// The unprivileged account every command in the guest runs as.
pub const GUEST_USER: &str = "conduit";
pub const GUEST_HOME: &str = "/home/conduit";

#[cfg(target_os = "macos")]
const CLI_CANDIDATES: &[&str] = &[
    "/usr/local/bin/docker",
    "/opt/homebrew/bin/docker",
    "~/.docker/bin/docker",
    "/Applications/Docker.app/Contents/Resources/bin/docker",
    "~/.orbstack/bin/docker",
    "~/.rd/bin/docker",
    "~/.colima/bin/docker",
];

#[cfg(target_os = "linux")]
const CLI_CANDIDATES: &[&str] = &[
    "/usr/bin/docker",
    "/usr/local/bin/docker",
    "/snap/bin/docker",
    "~/.docker/bin/docker",
    "~/.rd/bin/docker",
];

#[cfg(target_os = "windows")]
const CLI_CANDIDATES: &[&str] = &[
    r"C:\Program Files\Docker\Docker\resources\bin\docker.exe",
    r"C:\ProgramData\DockerDesktop\version-bin\docker.exe",
    r"~\.rd\bin\docker.exe",
];

/// Directories added to a spawned docker's PATH, for its helpers.
#[cfg(not(target_os = "windows"))]
const EXTRA_PATH: &[&str] = &[
    "/usr/local/bin",
    "/opt/homebrew/bin",
    "~/.docker/bin",
    "/Applications/Docker.app/Contents/Resources/bin",
    "/usr/bin",
    "/bin",
];

#[cfg(target_os = "windows")]
const EXTRA_PATH: &[&str] = &[r"C:\Program Files\Docker\Docker\resources\bin"];

fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~") {
        Some(rest) => dirs::home_dir()
            .unwrap_or_default()
            .join(rest.trim_start_matches(['/', '\\'])),
        None => PathBuf::from(path),
    }
}

#[derive(Debug, Clone)]
pub struct DockerCli {
    path: PathBuf,
}

/// A finished docker invocation.
pub struct Output {
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Output {
    pub fn success(&self) -> bool {
        self.status == Some(0)
    }

    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).trim().to_string()
    }
}

impl DockerCli {
    pub fn discover() -> Option<Self> {
        for candidate in CLI_CANDIDATES {
            let path = expand_home(candidate);
            if path.is_file() {
                return Some(Self { path });
            }
        }
        which_docker().map(|path| Self { path })
    }

    /// A `docker` invocation, not yet spawned. Stdin is null and the child is
    /// killed if its handle is dropped — a cancelled tool call must not leave
    /// a `docker exec` client behind.
    pub fn command(&self) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(&self.path);
        cmd.env("PATH", self.search_path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(target_os = "windows")]
        cmd.creation_flags(crate::platform::shell::CREATE_NO_WINDOW);
        cmd
    }

    /// The same, as a blocking std command — for the quit path, which fires a
    /// stop and exits without waiting for it.
    pub fn std_command(&self) -> std::process::Command {
        let mut cmd = std::process::Command::new(&self.path);
        cmd.env("PATH", self.search_path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(target_os = "windows")]
        std::os::windows::process::CommandExt::creation_flags(
            &mut cmd,
            crate::platform::shell::CREATE_NO_WINDOW,
        );
        cmd
    }

    fn search_path(&self) -> std::ffi::OsString {
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(parent) = self.path.parent() {
            dirs.push(parent.to_path_buf());
        }
        dirs.extend(EXTRA_PATH.iter().map(|d| expand_home(d)));
        if let Some(existing) = std::env::var_os("PATH") {
            dirs.extend(std::env::split_paths(&existing));
        }
        std::env::join_paths(dirs).unwrap_or_default()
    }

    /// Runs docker to completion and captures everything.
    pub async fn run(&self, args: &[String]) -> Result<Output, String> {
        let out = self
            .command()
            .args(args)
            .output()
            .await
            .map_err(|e| format!("could not run docker: {e}"))?;
        Ok(Output {
            status: out.status.code(),
            stdout: out.stdout,
            stderr: out.stderr,
        })
    }

    /// Runs docker and returns stdout, or stderr as the error.
    pub async fn run_ok(&self, args: &[String]) -> Result<String, String> {
        let out = self.run(args).await?;
        if out.success() {
            Ok(out.stdout_text())
        } else {
            Err(docker_error(&out.stderr_text()))
        }
    }

    /// What the daemon behind this CLI is, and whether sandboxes can use it.
    pub async fn probe(&self) -> DockerStatus {
        let mut status = DockerStatus {
            cli_path: Some(self.path.to_string_lossy().into_owned()),
            ..Default::default()
        };

        let version = match self
            .run(&args(&["version", "--format", "{{json .}}"]))
            .await
        {
            Ok(out) => out,
            Err(e) => {
                status.availability = DockerAvailability::Missing;
                status.hint = Some(e);
                return status;
            }
        };
        let parsed = parse_version(&version.stdout_text());
        status.client_version = parsed.client;
        status.server_version = parsed.server.clone();
        if parsed.podman {
            status.availability = DockerAvailability::Unsupported;
            status.hint = Some(
                "this `docker` is Podman's compatibility shim. sandboxes need Docker Desktop \
                 or Docker Engine for now."
                    .into(),
            );
            return status;
        }
        if parsed.server.is_none() {
            let (availability, hint) = classify_unreachable(&version.stderr_text());
            status.availability = availability;
            status.hint = Some(hint);
            return status;
        }

        match self.run(&args(&["info", "--format", "{{json .}}"])).await {
            Ok(out) if out.success() => {
                let info = parse_info(&out.stdout_text());
                status.engine = info.operating_system.clone();
                status.arch = info.architecture.clone();
                status.cpus = info.ncpu;
                status.memory_bytes = info.mem_total;
                if info.os_type.as_deref().is_some_and(|os| os != "linux") {
                    status.availability = DockerAvailability::Unsupported;
                    status.hint = Some(
                        "Docker is in Windows-containers mode. switch it to Linux containers \
                         from the Docker Desktop tray menu."
                            .into(),
                    );
                    return status;
                }
                if info.rootless {
                    status.hint = Some(
                        "rootless Docker may ignore the memory and CPU limits, depending on \
                         how its cgroups are delegated."
                            .into(),
                    );
                }
                status.availability = DockerAvailability::Ready;
            }
            Ok(out) => {
                let (availability, hint) = classify_unreachable(&out.stderr_text());
                status.availability = availability;
                status.hint = Some(hint);
            }
            Err(e) => {
                status.availability = DockerAvailability::NotRunning;
                status.hint = Some(e);
            }
        }
        status
    }
}

fn which_docker() -> Option<PathBuf> {
    #[cfg(not(target_os = "windows"))]
    let mut lookup = std::process::Command::new("/usr/bin/which");
    #[cfg(target_os = "windows")]
    let mut lookup = {
        let mut c = std::process::Command::new("where.exe");
        std::os::windows::process::CommandExt::creation_flags(
            &mut c,
            crate::platform::shell::CREATE_NO_WINDOW,
        );
        c
    };
    let out = lookup.arg("docker").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let path = stdout.lines().next().unwrap_or("").trim().to_string();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

pub fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// The container that backs sandbox `id`.
pub fn container_name(id: &str) -> String {
    format!("conduit-sbx-{id}")
}

/// The volume that holds sandbox `id`'s home directory.
pub fn volume_name(id: &str) -> String {
    format!("conduit-sbx-{id}-home")
}

/// Trims docker's error prose down to the part a person can act on.
pub fn docker_error(stderr: &str) -> String {
    let line = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .last()
        .unwrap_or("docker failed without saying why");
    line.strip_prefix("Error response from daemon: ")
        .or_else(|| line.strip_prefix("Error: "))
        .unwrap_or(line)
        .to_string()
}

/// Why the daemon did not answer, from the CLI's stderr.
pub fn classify_unreachable(stderr: &str) -> (DockerAvailability, String) {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("permission denied") {
        return (
            DockerAvailability::NoPermission,
            "your user may not talk to the Docker daemon. add it to the `docker` group \
             (`sudo usermod -aG docker $USER`), then log out and back in."
                .into(),
        );
    }
    #[cfg(target_os = "macos")]
    let start = "open Docker Desktop (or OrbStack) and wait for it to finish starting.";
    #[cfg(target_os = "windows")]
    let start = "open Docker Desktop and wait for it to finish starting.";
    #[cfg(target_os = "linux")]
    let start = "start the Docker daemon (`sudo systemctl start docker`), or open Docker Desktop.";
    let detail = docker_error(stderr);
    (
        DockerAvailability::NotRunning,
        format!("Docker is not running. {start} ({detail})"),
    )
}

/* ── parsers ─────────────────────────────────────────────────── */

#[derive(Debug, Default, PartialEq)]
pub struct VersionSummary {
    pub client: Option<String>,
    pub server: Option<String>,
    pub podman: bool,
}

pub fn parse_version(json: &str) -> VersionSummary {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json.trim()) else {
        return VersionSummary::default();
    };
    let text = |v: &serde_json::Value| v.as_str().map(str::to_string);
    let client = value.pointer("/Client/Version").and_then(text);
    let server = value.pointer("/Server/Version").and_then(text);
    let names = [
        value.pointer("/Client/Platform/Name"),
        value.pointer("/Server/Platform/Name"),
    ];
    let podman = names
        .iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .any(|n| n.to_ascii_lowercase().contains("podman"))
        || value.pointer("/Client/Podman").is_some();
    VersionSummary {
        client,
        server,
        podman,
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct InfoSummary {
    pub os_type: Option<String>,
    pub architecture: Option<String>,
    pub operating_system: Option<String>,
    pub ncpu: Option<u32>,
    pub mem_total: Option<u64>,
    pub rootless: bool,
}

pub fn parse_info(json: &str) -> InfoSummary {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json.trim()) else {
        return InfoSummary::default();
    };
    let text = |key: &str| {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let rootless = value
        .get("SecurityOptions")
        .and_then(|v| v.as_array())
        .is_some_and(|opts| {
            opts.iter()
                .filter_map(|o| o.as_str())
                .any(|o| o.contains("rootless"))
        });
    InfoSummary {
        os_type: text("OSType"),
        architecture: text("Architecture"),
        operating_system: text("OperatingSystem"),
        ncpu: value
            .get("NCPU")
            .and_then(|v| v.as_u64())
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0),
        mem_total: value
            .get("MemTotal")
            .and_then(|v| v.as_u64())
            .filter(|n| *n > 0),
        rootless,
    }
}

/// One row of `docker ps -a --format '{{json .}}'`.
#[derive(Debug, Clone, PartialEq)]
pub struct PsRow {
    pub name: String,
    pub state: String,
    pub sandbox: Option<String>,
    pub install: Option<String>,
}

#[derive(Deserialize)]
struct RawPs {
    #[serde(rename = "Names", default)]
    names: String,
    #[serde(rename = "State", default)]
    state: String,
    #[serde(rename = "Labels", default)]
    labels: String,
}

/// `docker ps` flattens labels into `k=v,k=v`. Conduit's own label values are
/// ids and hex, which never contain commas, so splitting is exact for the two
/// labels read here.
pub fn parse_ps_line(line: &str) -> Option<PsRow> {
    let raw: RawPs = serde_json::from_str(line.trim()).ok()?;
    let label = |key: &str| {
        raw.labels.split(',').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            (k == key).then(|| v.to_string())
        })
    };
    Some(PsRow {
        name: raw.names.split(',').next().unwrap_or("").to_string(),
        state: raw.state.to_ascii_lowercase(),
        sandbox: label(LABEL_SANDBOX),
        install: label(LABEL_INSTALL),
    })
}

/// One line of `docker events --format '{{json .}}'`, reduced to what the
/// manager acts on.
#[derive(Debug, Clone, PartialEq)]
pub struct EventRow {
    pub action: String,
    pub sandbox: String,
    pub install: Option<String>,
}

pub fn parse_event_line(line: &str) -> Option<EventRow> {
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let action = value
        .get("Action")
        .or_else(|| value.get("status"))
        .and_then(|v| v.as_str())?
        // `exec_start: bash -lc …` style actions carry their command after a
        // colon; only the verb matters.
        .split(':')
        .next()?
        .trim()
        .to_string();
    let attrs = value.pointer("/Actor/Attributes")?;
    let sandbox = attrs.get(LABEL_SANDBOX)?.as_str()?.to_string();
    let install = attrs
        .get(LABEL_INSTALL)
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Some(EventRow {
        action,
        sandbox,
        install,
    })
}

/// Pulls `(step, total)` out of one line of build output, in either BuildKit's
/// plain format (`#7 [3/9] RUN …`) or the legacy builder's (`Step 3/9 : RUN …`).
pub fn parse_build_progress(line: &str) -> Option<(u32, u32)> {
    let line = line.trim();
    let fraction = if let Some(rest) = line.strip_prefix("Step ") {
        rest.split_whitespace().next()?
    } else if line.starts_with('#') {
        let open = line.find('[')?;
        let close = line[open..].find(']')? + open;
        let inner = &line[open + 1..close];
        // BuildKit prefixes multi-stage steps with the stage name:
        // `[stage-1 3/9]`. The fraction is always the last word.
        inner.split_whitespace().last()?
    } else {
        return None;
    };
    let (step, total) = fraction.split_once('/')?;
    let step = step.parse().ok()?;
    let total = total.parse().ok()?;
    (step <= total && total > 0).then_some((step, total))
}

/* ── argument builders ───────────────────────────────────────── */

/// Everything a sandbox container is created with.
///
/// This is the security boundary between an agent and the user's machine, so
/// it is spelled out rather than assembled piecemeal, and a test pins what may
/// never appear in it: no `--privileged`, no published ports, no bind mounts,
/// no extra capabilities, no host namespaces.
pub fn create_args(spec: &SandboxSpec, image: &str, install_id: &str) -> Vec<String> {
    let id = &spec.id;
    let mut out = args(&["create"]);
    out.extend([
        "--name".into(),
        container_name(id),
        "--hostname".into(),
        id.clone(),
        "--label".into(),
        format!("{LABEL_SANDBOX}={id}"),
        "--label".into(),
        format!("{LABEL_INSTALL}={install_id}"),
        "--label".into(),
        format!("{LABEL_IMAGE}={image}"),
        // tini as PID 1: reaps the zombies a desktop session produces and
        // forwards `docker stop`'s SIGTERM to the entrypoint.
        "--init".into(),
    ]);
    out.extend(resource_args(spec.memory_mb, spec.cpus));
    out.extend([
        // Browsers need far more than Docker's 64 MB default /dev/shm.
        "--shm-size".into(),
        "1g".into(),
        // A fork bomb in the guest should hit this, not the host.
        "--pids-limit".into(),
        "4096".into(),
        "--network".into(),
        NETWORK.into(),
        "--mount".into(),
        format!("type=volume,source={},target={GUEST_HOME}", volume_name(id)),
        image.to_string(),
    ]);
    out
}

/// Memory and CPU limits. `--memory-swap` always travels with `--memory`:
/// `docker update` refuses a memory limit above an existing swap limit, and
/// equal values mean "no swap", which keeps the limit honest.
pub fn resource_args(memory_mb: u32, cpus: f64) -> Vec<String> {
    vec![
        "--memory".into(),
        format!("{memory_mb}m"),
        "--memory-swap".into(),
        format!("{memory_mb}m"),
        "--cpus".into(),
        format_cpus(cpus),
    ]
}

pub fn update_args(id: &str, memory_mb: u32, cpus: f64) -> Vec<String> {
    let mut out = args(&["update"]);
    out.extend(resource_args(memory_mb, cpus));
    out.push(container_name(id));
    out
}

fn format_cpus(cpus: f64) -> String {
    let rounded = (cpus * 100.0).round() / 100.0;
    if rounded.fract() == 0.0 {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded}")
    }
}

/// `docker exec` as the guest user, in their home directory.
pub fn exec_args(container: &str, interactive: bool, argv: &[String]) -> Vec<String> {
    let mut out = args(&["exec"]);
    if interactive {
        out.push("-i".into());
    }
    out.extend([
        "-u".into(),
        GUEST_USER.into(),
        "-w".into(),
        GUEST_HOME.into(),
        container.to_string(),
    ]);
    out.extend(argv.iter().cloned());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::model::SandboxOs;

    fn spec() -> SandboxSpec {
        SandboxSpec {
            id: "work".into(),
            name: "Work".into(),
            os: SandboxOs::Ubuntu2404,
            memory_mb: 4096,
            cpus: 2.0,
            width: 1280,
            height: 800,
            internet: true,
            auto_start: true,
            created_at: 0,
        }
    }

    /// The whole point of a sandbox is that it cannot reach back into the
    /// machine it runs on. Every one of these would hand it a way.
    #[test]
    fn create_never_grants_a_way_out() {
        let args = create_args(&spec(), "conduit-sandbox:ubuntu-24.04-abc", "install-1");
        let joined = args.join(" ");
        for forbidden in [
            "--privileged",
            "-p",
            "--publish",
            "-P",
            "--publish-all",
            "-v",
            "--volume",
            "--cap-add",
            "--device",
            "--pid",
            "--ipc",
            "--userns",
            "--uts",
            "docker.sock",
            "type=bind",
            "--network host",
            "--net",
            "seccomp=unconfined",
            "apparmor=unconfined",
        ] {
            assert!(
                !args.iter().any(|a| a == forbidden) && !joined.contains(&format!("{forbidden} ")),
                "`{forbidden}` must never be part of a sandbox: {joined}"
            );
        }
        assert!(!joined.contains("docker.sock"));
        assert!(!joined.contains("type=bind"));
    }

    #[test]
    fn create_labels_limits_and_isolates() {
        let args = create_args(&spec(), "img:tag", "install-1");
        let has_pair =
            |flag: &str, value: &str| args.windows(2).any(|w| w[0] == flag && w[1] == value);
        assert_eq!(args[0], "create");
        assert!(has_pair("--name", "conduit-sbx-work"));
        assert!(has_pair("--label", "ai.conduit.sandbox=work"));
        assert!(has_pair("--label", "ai.conduit.install=install-1"));
        assert!(has_pair("--label", "ai.conduit.image=img:tag"));
        assert!(has_pair("--memory", "4096m"));
        assert!(has_pair("--memory-swap", "4096m"));
        assert!(has_pair("--cpus", "2"));
        assert!(has_pair("--network", NETWORK));
        assert!(has_pair("--pids-limit", "4096"));
        assert!(has_pair(
            "--mount",
            "type=volume,source=conduit-sbx-work-home,target=/home/conduit"
        ));
        assert!(args.contains(&"--init".to_string()));
        // The image is the last argument: nothing after it may be read as a
        // command to run in place of the entrypoint.
        assert_eq!(args.last().unwrap(), "img:tag");
    }

    #[test]
    fn memory_always_travels_with_memory_swap() {
        let args = update_args("work", 8192, 1.5);
        assert_eq!(
            args,
            [
                "update",
                "--memory",
                "8192m",
                "--memory-swap",
                "8192m",
                "--cpus",
                "1.5",
                "conduit-sbx-work"
            ]
        );
    }

    #[test]
    fn exec_runs_as_the_guest_user() {
        let args = exec_args("c", true, &["xdotool".into(), "getmouselocation".into()]);
        assert_eq!(
            args,
            [
                "exec",
                "-i",
                "-u",
                "conduit",
                "-w",
                "/home/conduit",
                "c",
                "xdotool",
                "getmouselocation"
            ]
        );
    }

    /// Captured from a real CLI with no daemon: `Server` is null and the exit
    /// code is 1, while `docker info` would have exited 0 with empty fields.
    #[test]
    fn version_without_a_daemon_has_no_server() {
        let json = r#"{"Client":{"Platform":{"Name":"Docker Engine - Community"},"Version":"29.3.1","ApiVersion":"1.54","Os":"linux","Arch":"amd64","Context":"default"},"Server":null}"#;
        assert_eq!(
            parse_version(json),
            VersionSummary {
                client: Some("29.3.1".into()),
                server: None,
                podman: false
            }
        );
        let desktop = r#"{"Client":{"Version":"27.3.1"},"Server":{"Platform":{"Name":"Docker Desktop 4.36.0 (175267)"},"Version":"27.3.1"}}"#;
        assert_eq!(parse_version(desktop).server.as_deref(), Some("27.3.1"));
        let podman = r#"{"Client":{"Version":"5.0.0","Platform":{"Name":"Podman Engine"}},"Server":{"Version":"5.0.0"}}"#;
        assert!(parse_version(podman).podman);
        assert_eq!(parse_version("not json"), VersionSummary::default());
    }

    #[test]
    fn unreachable_daemons_are_told_apart() {
        let (a, _) = classify_unreachable(
            "failed to connect to the docker API at unix:///var/run/docker.sock; check if the path is correct and if the daemon is running: dial unix /var/run/docker.sock: connect: no such file or directory",
        );
        assert_eq!(a, DockerAvailability::NotRunning);
        let (a, hint) = classify_unreachable(
            "permission denied while trying to connect to the Docker daemon socket at unix:///var/run/docker.sock",
        );
        assert_eq!(a, DockerAvailability::NoPermission);
        assert!(hint.contains("docker` group"));
    }

    #[test]
    fn info_reads_limits_and_mode() {
        let json = r#"{"OSType":"linux","Architecture":"aarch64","OperatingSystem":"Docker Desktop","NCPU":8,"MemTotal":8219504640,"SecurityOptions":["name=seccomp,profile=unconfined","name=cgroupns"]}"#;
        let info = parse_info(json);
        assert_eq!(info.os_type.as_deref(), Some("linux"));
        assert_eq!(info.architecture.as_deref(), Some("aarch64"));
        assert_eq!(info.operating_system.as_deref(), Some("Docker Desktop"));
        assert_eq!(info.ncpu, Some(8));
        assert_eq!(info.mem_total, Some(8219504640));
        assert!(!info.rootless);
        let rootless = parse_info(
            r#"{"OSType":"linux","SecurityOptions":["name=seccomp,profile=builtin","name=rootless"]}"#,
        );
        assert!(rootless.rootless);
        let empty = parse_info(r#"{"OSType":"","NCPU":0,"MemTotal":0}"#);
        assert_eq!(empty, InfoSummary::default());
    }

    #[test]
    fn ps_rows_carry_the_ownership_labels() {
        let line = r#"{"Command":"\"/init\"","ID":"abc","Labels":"ai.conduit.image=conduit-sandbox:ubuntu-24.04-1a2b,ai.conduit.install=inst-1,ai.conduit.sandbox=work","Names":"conduit-sbx-work","State":"running","Status":"Up 2 minutes"}"#;
        assert_eq!(
            parse_ps_line(line),
            Some(PsRow {
                name: "conduit-sbx-work".into(),
                state: "running".into(),
                sandbox: Some("work".into()),
                install: Some("inst-1".into()),
            })
        );
        assert!(parse_ps_line("garbage").is_none());
    }

    #[test]
    fn events_reduce_to_verb_and_sandbox() {
        let line = r#"{"status":"die","id":"abc","from":"img","Type":"container","Action":"die","Actor":{"ID":"abc","Attributes":{"ai.conduit.install":"inst-1","ai.conduit.sandbox":"work","exitCode":"137","name":"conduit-sbx-work"}},"scope":"local","time":1}"#;
        assert_eq!(
            parse_event_line(line),
            Some(EventRow {
                action: "die".into(),
                sandbox: "work".into(),
                install: Some("inst-1".into())
            })
        );
        let exec = r#"{"Action":"exec_start: bash -lc ls","Actor":{"Attributes":{"ai.conduit.sandbox":"work"}}}"#;
        assert_eq!(parse_event_line(exec).unwrap().action, "exec_start");
        let foreign = r#"{"Action":"start","Actor":{"Attributes":{"name":"postgres"}}}"#;
        assert!(parse_event_line(foreign).is_none());
    }

    #[test]
    fn build_progress_reads_both_builders() {
        assert_eq!(
            parse_build_progress("#7 [3/9] RUN apt-get update"),
            Some((3, 9))
        );
        assert_eq!(
            parse_build_progress("#12 [stage-1 5/8] COPY x y"),
            Some((5, 8))
        );
        assert_eq!(
            parse_build_progress("Step 4/11 : RUN useradd"),
            Some((4, 11))
        );
        assert_eq!(parse_build_progress("#7 0.412 Get:1 http://archive"), None);
        assert_eq!(
            parse_build_progress("#1 [internal] load build definition"),
            None
        );
        assert_eq!(parse_build_progress("#7 [9/3] nonsense"), None);
    }

    #[test]
    fn daemon_errors_lose_their_boilerplate() {
        assert_eq!(
            docker_error("Error response from daemon: No such container: conduit-sbx-x\n"),
            "No such container: conduit-sbx-x"
        );
        assert_eq!(docker_error(""), "docker failed without saying why");
    }

    #[test]
    fn fractional_cpus_are_formatted_plainly() {
        assert_eq!(format_cpus(2.0), "2");
        assert_eq!(format_cpus(0.5), "0.5");
        assert_eq!(format_cpus(1.333333), "1.33");
    }
}
