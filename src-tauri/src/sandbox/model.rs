//! The shapes the Sandbox tab, the MCP layer and the on-disk state share.
//!
//! Everything here crosses into TypeScript, so every struct is camelCase and
//! the keys the UI reads are pinned by a test at the bottom — the same lesson
//! `state::Readiness` learned the hard way.

use serde::{Deserialize, Serialize};

/// The longest id a sandbox can have. MCP clients prefix every tool with the
/// server's config key — `mcp__conduit-<id>__get_cursor_position` — and several
/// of them refuse names past 64 characters, so this is the budget left over.
pub const MAX_ID_LEN: usize = 20;

pub const MIN_MEMORY_MB: u32 = 1024;
pub const MAX_MEMORY_MB: u32 = 64 * 1024;
pub const MIN_CPUS: f64 = 0.5;
pub const MAX_CPUS: f64 = 64.0;
pub const MIN_WIDTH: u32 = 800;
pub const MAX_WIDTH: u32 = 3840;
pub const MIN_HEIGHT: u32 = 600;
pub const MAX_HEIGHT: u32 = 2160;
pub const MAX_NAME_LEN: usize = 40;

/// The guest operating systems conduit knows how to build a desktop image for.
///
/// All three are apt-based on purpose: one Dockerfile covers them, and the
/// agent-facing behaviour (xdotool, wmctrl, AT-SPI, Firefox) is identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum SandboxOs {
    #[default]
    #[serde(rename = "ubuntu-24.04")]
    Ubuntu2404,
    #[serde(rename = "ubuntu-22.04")]
    Ubuntu2204,
    #[serde(rename = "debian-12")]
    Debian12,
}

impl SandboxOs {
    #[cfg(test)]
    pub const ALL: [SandboxOs; 3] = [Self::Ubuntu2404, Self::Ubuntu2204, Self::Debian12];

    /// The serialized name, which is also the image tag prefix.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Ubuntu2404 => "ubuntu-24.04",
            Self::Ubuntu2204 => "ubuntu-22.04",
            Self::Debian12 => "debian-12",
        }
    }

    /// What the agent is told it is driving.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ubuntu2404 => "Ubuntu 24.04 LTS",
            Self::Ubuntu2204 => "Ubuntu 22.04 LTS",
            Self::Debian12 => "Debian 12",
        }
    }

    /// The base image the Dockerfile builds `FROM`.
    ///
    /// Tags rather than digests: the image is built on the user's machine and
    /// runs `apt-get upgrade`, so a digest pin would only freeze the base's
    /// security fixes until the next conduit release without making the result
    /// any more reproducible.
    pub const fn base_image(self) -> &'static str {
        match self {
            Self::Ubuntu2404 => "ubuntu:24.04",
            Self::Ubuntu2204 => "ubuntu:22.04",
            Self::Debian12 => "debian:12",
        }
    }

    /// Where Firefox comes from. Ubuntu's own `firefox` package is a stub that
    /// installs the snap, and snapd cannot run in a container; Debian ships a
    /// real ESR build.
    pub const fn firefox_source(self) -> &'static str {
        match self {
            Self::Ubuntu2404 | Self::Ubuntu2204 => "mozilla",
            Self::Debian12 => "esr",
        }
    }
}

/// One sandbox, as the user configured it. This is what `state.json` holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxSpec {
    /// Immutable once created: it is in the endpoint URL and in every agent
    /// config that points here.
    pub id: String,
    pub name: String,
    pub os: SandboxOs,
    pub memory_mb: u32,
    pub cpus: f64,
    pub width: u32,
    pub height: u32,
    pub internet: bool,
    /// Start the sandbox when an agent calls a tool on a stopped one.
    pub auto_start: bool,
    pub created_at: u64,
}

/// What the Create dialog sends.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSandbox {
    pub name: String,
    #[serde(default)]
    pub os: SandboxOs,
    pub memory_mb: u32,
    pub cpus: f64,
    pub width: u32,
    pub height: u32,
    pub internet: bool,
    pub auto_start: bool,
}

/// A partial edit. The OS is deliberately absent: changing it means a new
/// image and a new root filesystem, which is a new sandbox.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxPatch {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub memory_mb: Option<u32>,
    #[serde(default)]
    pub cpus: Option<f64>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub internet: Option<bool>,
    #[serde(default)]
    pub auto_start: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum DockerAvailability {
    /// Not probed yet.
    #[default]
    Checking,
    /// No `docker` CLI anywhere conduit looked.
    Missing,
    /// The CLI is there but the daemon is not answering — Docker Desktop is
    /// installed and not running, most often.
    NotRunning,
    /// The daemon is up but this user may not talk to it (Linux, not in the
    /// `docker` group).
    NoPermission,
    /// Something answered, but not something sandboxes can run on: Windows
    /// containers mode, or Podman's docker shim.
    Unsupported,
    Ready,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerStatus {
    pub availability: DockerAvailability,
    pub cli_path: Option<String>,
    pub client_version: Option<String>,
    pub server_version: Option<String>,
    /// e.g. "Docker Desktop", "Ubuntu 24.04 LTS", "OrbStack".
    pub engine: Option<String>,
    pub arch: Option<String>,
    /// What the daemon (on Docker Desktop, its VM) has to hand out. The UI
    /// clamps the RAM and CPU pickers to these.
    pub cpus: Option<u32>,
    pub memory_bytes: Option<u64>,
    /// Plain-language next step when not ready, or a warning when ready.
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SandboxStatus {
    /// The desktop image for this OS has not been built yet.
    NeedsImage,
    /// The desktop image is being built.
    Building,
    Stopped,
    Starting,
    Running,
    Stopping,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageBuild {
    pub os: SandboxOs,
    pub step: Option<u32>,
    pub total_steps: Option<u32>,
    pub last_line: Option<String>,
    pub started_at: u64,
}

/// A sandbox as the UI sees it: the spec plus everything live.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxView {
    pub spec: SandboxSpec,
    pub status: SandboxStatus,
    pub error: Option<String>,
    pub build: Option<ImageBuild>,
    /// The user pressed "stop agent": tool calls are refused until resumed.
    pub paused: bool,
    /// The full MCP URL an agent connects to.
    pub endpoint: String,
    /// Tool calls in flight right now.
    pub busy_calls: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxesState {
    pub docker: DockerStatus,
    pub sandboxes: Vec<SandboxView>,
    pub stop_on_quit: bool,
}

/// One step of what an agent is doing inside a sandbox, for the live view's
/// overlay and the sandbox list's activity line.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxActivity {
    pub sandbox_id: String,
    pub tool: String,
    pub action: String,
    pub agent: Option<String>,
    pub detail: Option<String>,
    /// "running" while the call is in flight, then "ok", "error" or "blocked".
    pub outcome: &'static str,
    /// Where the pointer ended up, for the click pulse.
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub at: u64,
}

/// Ids that mean something else where sandbox ids appear. `host` is how the
/// Agents tab names this computer.
const RESERVED_IDS: &[&str] = &["host"];

/// Whether `id` is a well-formed sandbox id: lowercase letters, digits and
/// single inner hyphens, at most [`MAX_ID_LEN`] long, and not reserved.
///
/// This is load-bearing well beyond cosmetics. The id is interpolated into a
/// container name, a volume name, a URL path segment and agent config keys,
/// and passed to `docker` as an argument — so it must never be able to look
/// like a flag, a path or anything a parser would treat specially.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-')
        && !id.contains("--")
        && !RESERVED_IDS.contains(&id)
}

/// Turns a display name into an id, leaving room for a `-N` suffix.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for c in name.trim().chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(c);
        } else {
            pending_dash = true;
        }
        if out.len() >= MAX_ID_LEN - 3 {
            break;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "sandbox".into()
    } else {
        out
    }
}

/// The first free id for `name`, given the ids already taken.
pub fn unique_id(name: &str, taken: &[&str]) -> String {
    let base = slugify(name);
    if valid_id(&base) && !taken.contains(&base.as_str()) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|candidate| !taken.contains(&candidate.as_str()))
        .expect("an unbounded range always has a free suffix")
}

pub fn clean_name(name: &str) -> Result<String, String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err("give the sandbox a name".into());
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(format!("keep the name under {MAX_NAME_LEN} characters"));
    }
    Ok(name)
}

/// Checks the numbers a spec carries, so a hand-edited state file or a
/// confused UI cannot hand `docker` a zero-byte memory limit.
pub fn validate_resources(
    memory_mb: u32,
    cpus: f64,
    width: u32,
    height: u32,
) -> Result<(), String> {
    if !(MIN_MEMORY_MB..=MAX_MEMORY_MB).contains(&memory_mb) {
        return Err(format!(
            "memory must be between {} and {} GB",
            MIN_MEMORY_MB / 1024,
            MAX_MEMORY_MB / 1024
        ));
    }
    if !cpus.is_finite() || !(MIN_CPUS..=MAX_CPUS).contains(&cpus) {
        return Err(format!("CPUs must be between {MIN_CPUS} and {MAX_CPUS}"));
    }
    if !(MIN_WIDTH..=MAX_WIDTH).contains(&width) || !(MIN_HEIGHT..=MAX_HEIGHT).contains(&height) {
        return Err(format!(
            "the screen must be between {MIN_WIDTH}x{MIN_HEIGHT} and {MAX_WIDTH}x{MAX_HEIGHT}"
        ));
    }
    Ok(())
}

pub fn validate_spec(spec: &SandboxSpec) -> Result<(), String> {
    if !valid_id(&spec.id) {
        return Err(format!("{:?} is not a valid sandbox id", spec.id));
    }
    if clean_name(&spec.name)? != spec.name {
        return Err(format!("{:?} is not a clean sandbox name", spec.name));
    }
    validate_resources(spec.memory_mb, spec.cpus, spec.width, spec.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_strict_slugs() {
        for good in ["work", "a", "ubuntu-2", "x1-y2-z3", "abcdefghijklmnopqrst"] {
            assert!(valid_id(good), "{good} should be valid");
        }
        for bad in [
            "",
            "-work",
            "work-",
            "wo--rk",
            "Work",
            "wörk",
            "../etc",
            "a b",
            "a_b",
            "--privileged",
            "abcdefghijklmnopqrstu",
            "host",
        ] {
            assert!(!valid_id(bad), "{bad:?} should be invalid");
        }
    }

    #[test]
    fn slugs_are_always_valid_ids() {
        for name in [
            "Work",
            "  My Ubuntu Box!  ",
            "日本語",
            "---",
            "a really long sandbox name that goes on",
            "ubuntu 24.04",
        ] {
            let slug = slugify(name);
            assert!(valid_id(&slug), "{name:?} slugged to invalid {slug:?}");
            assert!(
                slug.len() <= MAX_ID_LEN - 3,
                "{slug} leaves no room for a suffix"
            );
        }
        assert_eq!(slugify("My Ubuntu Box!"), "my-ubuntu-box");
        assert_eq!(slugify("日本語"), "sandbox");
    }

    #[test]
    fn unique_ids_take_the_first_free_suffix() {
        assert_eq!(unique_id("Work", &[]), "work");
        assert_eq!(unique_id("Work", &["work"]), "work-2");
        assert_eq!(unique_id("Work", &["work", "work-2"]), "work-3");
        assert_eq!(unique_id("Host", &[]), "host-2", "host means this computer");
        let long = unique_id("abcdefghijklmnopqrstuvwxyz", &["abcdefghijklmnopq"]);
        assert!(valid_id(&long), "{long}");
    }

    #[test]
    fn resources_are_bounded() {
        assert!(validate_resources(4096, 2.0, 1280, 800).is_ok());
        assert!(validate_resources(0, 2.0, 1280, 800).is_err());
        assert!(validate_resources(4096, 0.0, 1280, 800).is_err());
        assert!(validate_resources(4096, f64::NAN, 1280, 800).is_err());
        assert!(validate_resources(4096, 2.0, 100, 800).is_err());
        assert!(validate_resources(4096, 2.0, 1280, 5000).is_err());
    }

    /// The UI reads these names; a rename here is invisible in Rust and reads
    /// as `undefined` in TypeScript.
    #[test]
    fn views_serialize_the_keys_the_ui_reads() {
        let spec = SandboxSpec {
            id: "work".into(),
            name: "Work".into(),
            os: SandboxOs::Ubuntu2404,
            memory_mb: 4096,
            cpus: 2.0,
            width: 1280,
            height: 800,
            internet: true,
            auto_start: true,
            created_at: 1,
        };
        let view = SandboxView {
            spec,
            status: SandboxStatus::NeedsImage,
            error: None,
            build: None,
            paused: false,
            endpoint: "http://127.0.0.1:6767/sandbox/work/mcp".into(),
            busy_calls: 0,
        };
        let json = serde_json::to_value(&view).unwrap();
        for key in [
            "spec",
            "status",
            "error",
            "build",
            "paused",
            "endpoint",
            "busyCalls",
        ] {
            assert!(json.get(key).is_some(), "missing {key}: {json}");
        }
        for key in [
            "id",
            "name",
            "os",
            "memoryMb",
            "cpus",
            "width",
            "height",
            "internet",
            "autoStart",
            "createdAt",
        ] {
            assert!(
                json["spec"].get(key).is_some(),
                "missing spec.{key}: {json}"
            );
        }
        assert_eq!(json["status"], "needsImage");
        assert_eq!(json["spec"]["os"], "ubuntu-24.04");
        assert!(json["error"].is_null());

        let docker = serde_json::to_value(DockerStatus {
            availability: DockerAvailability::NotRunning,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(docker["availability"], "notRunning");
        for key in [
            "cliPath",
            "clientVersion",
            "serverVersion",
            "memoryBytes",
            "hint",
        ] {
            assert!(docker.get(key).is_some(), "missing docker.{key}: {docker}");
        }
    }

    #[test]
    fn os_keys_round_trip() {
        for os in SandboxOs::ALL {
            let json = serde_json::to_value(os).unwrap();
            assert_eq!(json, os.key());
            let back: SandboxOs = serde_json::from_value(json).unwrap();
            assert_eq!(back, os);
        }
    }
}
