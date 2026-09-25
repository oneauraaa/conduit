//! Sandboxes: Linux desktops in Docker that an agent drives instead of the
//! user's own computer.
//!
//! Each sandbox is one container (`conduit-sbx-<id>`) with its home directory
//! on a named volume, reached by agents at `/sandbox/<id>/mcp`. The manager
//! here owns everything about their lifecycle: finding Docker, building the
//! desktop image, creating, starting, isolating, stopping and deleting
//! containers, and keeping its idea of their state in step with Docker's.
//!
//! ## What keeps a sandbox from reaching the user's machine
//!
//! 1. The container is created without privileges, published ports, bind
//!    mounts, extra capabilities or the Docker socket — see
//!    [`docker::create_args`], whose test pins exactly that.
//! 2. It joins a dedicated bridge network with inter-container traffic off, or
//!    no network at all when internet access is off.
//! 3. After every start, a short-lived helper sharing the sandbox's network
//!    namespace rejects traffic to the host's gateway addresses
//!    ([`ISOLATE_SCRIPT`]). Conduit's own endpoint is unauthenticated and
//!    Docker Desktop forwards `host.docker.internal` to the host's loopback, so
//!    without this an agent in the sandbox could drive the real PC. The
//!    sandbox has no `NET_ADMIN` capability and cannot undo the rules.
//! 4. conduit only stops, starts or deletes containers and volumes carrying its
//!    own install label ([`owned_by`]).

pub mod docker;
pub mod guest;
pub mod image;
pub mod keys;
pub mod model;
mod persist;
pub mod viewer;
pub mod xdo;

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock};
use tauri::{AppHandle, Emitter, Manager, Wry};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use crate::state::now_millis;
use docker::{DockerCli, args, container_name, volume_name};
use guest::Guest;
use model::{
    DockerAvailability, DockerStatus, ImageBuild, NewSandbox, SandboxActivity, SandboxOs,
    SandboxPatch, SandboxSpec, SandboxStatus, SandboxView, SandboxesState,
};
use persist::SandboxDisk;

/// How long a started sandbox has to bring its desktop up before the start
/// is reported as failed.
const READY_TIMEOUT: Duration = Duration::from_secs(60);
const BUILD_EMIT_INTERVAL: Duration = Duration::from_millis(250);

/// Rejects traffic from the sandbox to the machine running it.
///
/// Run as root in a throwaway container that shares the sandbox's network
/// namespace and holds `NET_ADMIN` — the sandbox itself never does. Blocks the
/// default gateway (the host's side of the bridge on a native engine) and
/// whatever `host.docker.internal` / `gateway.docker.internal` resolve to
/// (Docker Desktop's forwarders to the host's loopback, where conduit
/// listens). Only traffic *addressed to* those IPs is refused; traffic routed
/// through the gateway to the internet is untouched.
///
/// Idempotent (`-C` before `-I`), so it is safe to re-run after a network
/// change. Fails closed: an IPv4 rule that cannot be added fails the start.
pub const ISOLATE_SCRIPT: &str = r#"set -u
ipt=iptables
$ipt -L OUTPUT -n >/dev/null 2>&1 || ipt=iptables-legacy
v4="$( { ip -4 route show default | awk '{print $3}'; getent ahostsv4 host.docker.internal gateway.docker.internal 2>/dev/null | awk '{print $1}'; } | sort -u)"
if [ -z "$v4" ]; then echo "no gateway address to isolate" >&2; exit 1; fi
for addr in $v4; do
  $ipt -C OUTPUT -d "$addr" -j REJECT 2>/dev/null || $ipt -I OUTPUT -d "$addr" -j REJECT || exit 1
done
v6="$( { ip -6 route show default 2>/dev/null | awk '{print $3}'; getent ahostsv6 host.docker.internal gateway.docker.internal 2>/dev/null | awk '{print $1}'; } | sort -u)"
for addr in $v6; do
  ip6tables -C OUTPUT -d "$addr" -j REJECT 2>/dev/null || ip6tables -I OUTPUT -d "$addr" -j REJECT 2>/dev/null || true
done
echo $v4 $v6
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Stopped,
    Starting,
    Running,
    Stopping,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageState {
    Unknown,
    Missing,
    Ready,
}

struct Runtime {
    phase: Phase,
    error: Option<String>,
    /// The user pressed "stop agent". Calls are refused until resumed.
    paused: bool,
    /// Cancelled to abort every call in flight; replaced after each use.
    cancel: CancellationToken,
    calls: u32,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            phase: Phase::Stopped,
            error: None,
            paused: false,
            cancel: CancellationToken::new(),
            calls: 0,
        }
    }
}

struct BuildJob {
    cancel: CancellationToken,
    progress: ImageBuild,
}

pub struct SandboxManager {
    app: AppHandle<Wry>,
    root: PathBuf,
    cli: RwLock<Option<Arc<DockerCli>>>,
    docker: RwLock<DockerStatus>,
    disk: RwLock<SandboxDisk>,
    runtime: RwLock<HashMap<String, Runtime>>,
    images: RwLock<HashMap<SandboxOs, ImageState>>,
    builds: Mutex<HashMap<SandboxOs, BuildJob>>,
    build_locks: Mutex<HashMap<SandboxOs, Arc<AsyncMutex<()>>>>,
    /// Serialises lifecycle changes per sandbox, so a start and a stop of the
    /// same one never interleave while two different ones can start at once.
    locks: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    /// Held across an input tool's xdotool chain, so two agents on one
    /// sandbox cannot interleave their pointer movements.
    input_locks: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    probing: AsyncMutex<()>,
    port: AtomicU16,
    watcher_started: AtomicBool,
    last_build_emit: Mutex<Instant>,
    pub(crate) viewers: viewer::Viewers,
}

/// Admits one tool call into a running sandbox. Holds the sandbox's cancel
/// token for the call's lifetime, and counts the call as in flight.
pub struct CallGuard {
    manager: Arc<SandboxManager>,
    id: String,
    pub cancel: CancellationToken,
}

impl Drop for CallGuard {
    fn drop(&mut self) {
        if let Some(rt) = self.manager.runtime.write().get_mut(&self.id) {
            rt.calls = rt.calls.saturating_sub(1);
        }
    }
}

impl SandboxManager {
    pub fn new(app: AppHandle<Wry>, port: u16) -> Arc<Self> {
        let root = app
            .path()
            .app_local_data_dir()
            .unwrap_or_else(|_| std::env::temp_dir().join("conduit"))
            .join("sandbox");
        let disk = match persist::load(&root) {
            Some(disk) => disk,
            None => {
                if root.join("state.json").exists() {
                    tracing::warn!("the sandbox list could not be read; starting a new one");
                    persist::quarantine(&root);
                }
                let fresh = SandboxDisk::fresh();
                if let Err(error) = persist::save(&root, &fresh) {
                    tracing::warn!(%error, "could not save the sandbox list");
                }
                fresh
            }
        };
        let runtime = disk
            .sandboxes
            .iter()
            .map(|s| (s.id.clone(), Runtime::default()))
            .collect();
        Arc::new(Self {
            app,
            root,
            cli: RwLock::new(None),
            docker: RwLock::new(DockerStatus::default()),
            disk: RwLock::new(disk),
            runtime: RwLock::new(runtime),
            images: RwLock::new(HashMap::new()),
            builds: Mutex::new(HashMap::new()),
            build_locks: Mutex::new(HashMap::new()),
            locks: Mutex::new(HashMap::new()),
            input_locks: Mutex::new(HashMap::new()),
            probing: AsyncMutex::new(()),
            port: AtomicU16::new(port),
            watcher_started: AtomicBool::new(false),
            last_build_emit: Mutex::new(Instant::now()),
            viewers: viewer::Viewers::default(),
        })
    }

    /// Finds Docker, reconciles with what is already running, and starts
    /// following Docker's events. Called once from setup; never blocks it.
    pub fn initialize(self: &Arc<Self>) {
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            manager.refresh_docker().await;
            manager.spawn_watcher();
        });
    }

    pub fn set_port(&self, port: u16) {
        self.port.store(port, Ordering::Relaxed);
    }

    /* ── reading ── */

    pub fn exists(&self, id: &str) -> bool {
        self.disk.read().sandboxes.iter().any(|s| s.id == id)
    }

    pub fn spec(&self, id: &str) -> Option<SandboxSpec> {
        self.disk
            .read()
            .sandboxes
            .iter()
            .find(|s| s.id == id)
            .cloned()
    }

    pub fn ids(&self) -> Vec<String> {
        self.disk
            .read()
            .sandboxes
            .iter()
            .map(|s| s.id.clone())
            .collect()
    }

    pub fn endpoint(&self, id: &str) -> String {
        format!(
            "http://127.0.0.1:{}/sandbox/{id}/mcp",
            self.port.load(Ordering::Relaxed)
        )
    }

    pub fn snapshot(&self) -> SandboxesState {
        let disk = self.disk.read();
        let runtime = self.runtime.read();
        let images = self.images.read();
        let builds = self.builds.lock();
        let sandboxes = disk
            .sandboxes
            .iter()
            .map(|spec| {
                let rt = runtime.get(&spec.id);
                let phase = rt.map(|r| r.phase).unwrap_or(Phase::Stopped);
                let build = builds.get(&spec.os).map(|job| job.progress.clone());
                let image = images.get(&spec.os).copied().unwrap_or(ImageState::Unknown);
                SandboxView {
                    status: derive_status(phase, build.is_some(), image),
                    error: rt.and_then(|r| r.error.clone()),
                    build,
                    paused: rt.is_some_and(|r| r.paused),
                    endpoint: self.endpoint(&spec.id),
                    busy_calls: rt.map(|r| r.calls).unwrap_or(0),
                    spec: spec.clone(),
                }
            })
            .collect();
        SandboxesState {
            docker: self.docker.read().clone(),
            sandboxes,
            stop_on_quit: disk.stop_on_quit,
        }
    }

    fn emit(&self) {
        let _ = self.app.emit("sandbox:state", self.snapshot());
    }

    pub fn emit_activity(&self, activity: SandboxActivity) {
        let _ = self.app.emit("sandbox:activity", activity);
    }

    fn ready_cli(&self) -> Result<Arc<DockerCli>, String> {
        let status = self.docker.read().clone();
        if status.availability != DockerAvailability::Ready {
            return Err(status
                .hint
                .unwrap_or_else(|| "Docker is not available".into()));
        }
        self.cli
            .read()
            .clone()
            .ok_or_else(|| "Docker is not available".into())
    }

    pub fn guest(&self, id: &str) -> Result<Guest, String> {
        Ok(Guest::new(self.ready_cli()?, id))
    }

    pub(crate) fn cli_for_viewer(&self) -> Result<Arc<DockerCli>, String> {
        self.ready_cli()
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.phase(id) == Phase::Running
    }

    fn phase(&self, id: &str) -> Phase {
        self.runtime
            .read()
            .get(id)
            .map(|r| r.phase)
            .unwrap_or(Phase::Stopped)
    }

    fn set_phase(&self, id: &str, phase: Phase, error: Option<String>) {
        {
            let mut runtime = self.runtime.write();
            let rt = runtime.entry(id.to_string()).or_default();
            rt.phase = phase;
            rt.error = error;
        }
        self.emit();
    }

    fn lock_for(&self, id: &str) -> Arc<AsyncMutex<()>> {
        self.locks.lock().entry(id.to_string()).or_default().clone()
    }

    pub fn input_lock(&self, id: &str) -> Arc<AsyncMutex<()>> {
        self.input_locks
            .lock()
            .entry(id.to_string())
            .or_default()
            .clone()
    }

    fn save(&self) -> Result<(), String> {
        let disk = self.disk.read().clone();
        persist::save(&self.root, &disk)
    }

    /* ── docker ── */

    /// Looks for Docker again and re-reads the daemon's state. Cheap enough to
    /// call from the Sandbox tab's "check again" button and on a timer.
    pub async fn refresh_docker(self: &Arc<Self>) -> SandboxesState {
        let _probing = self.probing.lock().await;
        let known = self.cli.read().clone();
        let cli = match known {
            Some(cli) => Some(cli),
            None => tokio::task::spawn_blocking(DockerCli::discover)
                .await
                .ok()
                .flatten()
                .map(Arc::new),
        };
        let status = match &cli {
            Some(cli) => cli.probe().await,
            None => DockerStatus {
                availability: DockerAvailability::Missing,
                hint: Some(missing_hint().into()),
                ..Default::default()
            },
        };
        // Forget a CLI that has vanished, so the next refresh looks again.
        *self.cli.write() = if status.availability == DockerAvailability::Missing {
            None
        } else {
            cli
        };
        let became_ready = status.availability == DockerAvailability::Ready
            && self.docker.read().availability != DockerAvailability::Ready;
        *self.docker.write() = status;
        if became_ready {
            self.reconcile().await;
        }
        self.emit();
        self.snapshot()
    }

    /// Brings conduit's idea of each sandbox in line with Docker's: which are
    /// running, and which desktop images already exist.
    async fn reconcile(self: &Arc<Self>) {
        let Ok(cli) = self.ready_cli() else { return };
        let install = self.disk.read().install_id.clone();
        let filter = format!("label={}={install}", docker::LABEL_INSTALL);
        if let Ok(text) = cli
            .run_ok(&args(&[
                "ps",
                "-a",
                "--filter",
                &filter,
                "--format",
                "{{json .}}",
            ]))
            .await
        {
            let rows: Vec<_> = text.lines().filter_map(docker::parse_ps_line).collect();
            let mut adopt = Vec::new();
            {
                let mut runtime = self.runtime.write();
                for id in self.ids() {
                    let running = rows
                        .iter()
                        .any(|r| r.sandbox.as_deref() == Some(&id) && r.state == "running");
                    let rt = runtime.entry(id.clone()).or_default();
                    match (rt.phase, running) {
                        // Running without conduit having started it this run —
                        // after a relaunch, or started by hand. Its network
                        // rules cannot be vouched for, so it is re-isolated
                        // (idempotently) before being called running.
                        (Phase::Stopped | Phase::Error, true) => adopt.push(id),
                        (Phase::Running, false) => rt.phase = Phase::Stopped,
                        _ => {}
                    }
                }
            }
            for id in adopt {
                let manager = self.clone();
                tauri::async_runtime::spawn(async move {
                    manager.adopt_external_start(&id).await;
                });
            }
        }
        let wanted: Vec<SandboxOs> = {
            let mut os: Vec<_> = self.disk.read().sandboxes.iter().map(|s| s.os).collect();
            os.sort_by_key(|o| o.key());
            os.dedup();
            os
        };
        for os in wanted {
            self.check_image(&cli, os).await;
        }
    }

    async fn check_image(&self, cli: &DockerCli, os: SandboxOs) -> ImageState {
        let present = cli
            .run(&args(&[
                "image",
                "inspect",
                "--format",
                "{{.Id}}",
                &image::tag(os),
            ]))
            .await
            .map(|out| out.success())
            .unwrap_or(false);
        let state = if present {
            ImageState::Ready
        } else {
            ImageState::Missing
        };
        self.images.write().insert(os, state);
        state
    }

    /// Follows `docker events` for conduit's containers, reconnecting with
    /// backoff when the daemon restarts.
    fn spawn_watcher(self: &Arc<Self>) {
        if self.watcher_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut backoff = Duration::from_secs(2);
            loop {
                if let Ok(cli) = manager.ready_cli() {
                    let started = Instant::now();
                    manager.watch_events(&cli).await;
                    if started.elapsed() > Duration::from_secs(30) {
                        backoff = Duration::from_secs(2);
                    }
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(30));
                manager.refresh_docker().await;
                // Events were missed while the stream was down; catch up.
                manager.reconcile().await;
            }
        });
    }

    async fn watch_events(self: &Arc<Self>, cli: &DockerCli) {
        let install = self.disk.read().install_id.clone();
        let label = format!("label={}={install}", docker::LABEL_INSTALL);
        let mut cmd = cli.command();
        cmd.args([
            "events",
            "--filter",
            "type=container",
            "--filter",
            &label,
            "--filter",
            "event=start",
            "--filter",
            "event=die",
            "--filter",
            "event=oom",
            "--filter",
            "event=destroy",
            "--format",
            "{{json .}}",
        ]);
        let Ok(mut child) = cmd.spawn() else { return };
        let Some(stdout) = child.stdout.take() else {
            return;
        };
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(event) = docker::parse_event_line(&line) {
                if event.install.as_deref() == Some(install.as_str()) {
                    self.handle_event(&event.sandbox, &event.action);
                }
            }
        }
        let _ = child.kill().await;
    }

    fn handle_event(self: &Arc<Self>, id: &str, action: &str) {
        if !self.exists(id) {
            return;
        }
        match action {
            "start" => {
                // A start conduit did not ask for — the Docker Desktop UI, a
                // `docker start` in a terminal. Its network rules died with the
                // last run, so isolate it before calling it running.
                if matches!(self.phase(id), Phase::Stopped | Phase::Error) {
                    let manager = self.clone();
                    let id = id.to_string();
                    tauri::async_runtime::spawn(async move {
                        manager.adopt_external_start(&id).await;
                    });
                }
            }
            "oom" => {
                if let Some(rt) = self.runtime.write().get_mut(id) {
                    rt.error = Some(
                        "the sandbox ran out of memory. give it more RAM in its settings.".into(),
                    );
                }
            }
            "die" | "destroy" => {
                let was = self.phase(id);
                if matches!(was, Phase::Running | Phase::Error) {
                    let error = self.runtime.read().get(id).and_then(|r| r.error.clone());
                    let error = error.or_else(|| {
                        (was == Phase::Running).then(|| "the sandbox stopped unexpectedly".into())
                    });
                    self.interrupt_calls(id);
                    self.set_phase(id, Phase::Stopped, error);
                }
            }
            _ => {}
        }
    }

    async fn adopt_external_start(self: &Arc<Self>, id: &str) {
        let lock = self.lock_for(id);
        let _held = lock.lock().await;
        if !matches!(self.phase(id), Phase::Stopped | Phase::Error) {
            return;
        }
        let (Some(spec), Ok(cli)) = (self.spec(id), self.ready_cli()) else {
            return;
        };
        let result = async {
            if spec.internet {
                self.isolate(&cli, &spec).await?;
            }
            Ok::<_, String>(())
        }
        .await;
        match result {
            Ok(()) => self.set_phase(id, Phase::Running, None),
            Err(error) => {
                let _ = cli
                    .run(&args(&["stop", "-t", "3", &container_name(id)]))
                    .await;
                self.set_phase(
                    id,
                    Phase::Error,
                    Some(format!("stopped: it was started outside conduit and could not be isolated ({error})")),
                );
            }
        }
    }

    /* ── editing ── */

    pub fn create(self: &Arc<Self>, new: NewSandbox) -> Result<SandboxSpec, String> {
        let name = model::clean_name(&new.name)?;
        model::validate_resources(new.memory_mb, new.cpus, new.width, new.height)?;
        let spec = {
            let mut disk = self.disk.write();
            if disk
                .sandboxes
                .iter()
                .any(|s| s.name.eq_ignore_ascii_case(&name))
            {
                return Err(format!("a sandbox called {name:?} already exists"));
            }
            let taken: Vec<&str> = disk.sandboxes.iter().map(|s| s.id.as_str()).collect();
            let spec = SandboxSpec {
                id: model::unique_id(&name, &taken),
                name,
                os: new.os,
                memory_mb: new.memory_mb,
                cpus: new.cpus,
                width: new.width,
                height: new.height,
                internet: new.internet,
                auto_start: new.auto_start,
                created_at: now_millis(),
            };
            disk.sandboxes.push(spec.clone());
            spec
        };
        if let Err(error) = self.save() {
            self.disk.write().sandboxes.retain(|s| s.id != spec.id);
            return Err(error);
        }
        self.runtime
            .write()
            .insert(spec.id.clone(), Runtime::default());
        self.emit();
        Ok(spec)
    }

    pub async fn update(self: &Arc<Self>, id: &str, patch: SandboxPatch) -> Result<(), String> {
        let lock = self.lock_for(id);
        let _held = lock.lock().await;
        let before = self
            .spec(id)
            .ok_or_else(|| format!("no sandbox called {id}"))?;
        let mut next = before.clone();
        if let Some(name) = patch.name {
            let name = model::clean_name(&name)?;
            if self
                .disk
                .read()
                .sandboxes
                .iter()
                .any(|s| s.id != id && s.name.eq_ignore_ascii_case(&name))
            {
                return Err(format!("a sandbox called {name:?} already exists"));
            }
            next.name = name;
        }
        if let Some(v) = patch.memory_mb {
            next.memory_mb = v;
        }
        if let Some(v) = patch.cpus {
            next.cpus = v;
        }
        if let Some(v) = patch.width {
            next.width = v;
        }
        if let Some(v) = patch.height {
            next.height = v;
        }
        if let Some(v) = patch.internet {
            next.internet = v;
        }
        if let Some(v) = patch.auto_start {
            next.auto_start = v;
        }
        model::validate_resources(next.memory_mb, next.cpus, next.width, next.height)?;
        {
            let mut disk = self.disk.write();
            if let Some(slot) = disk.sandboxes.iter_mut().find(|s| s.id == id) {
                *slot = next.clone();
            }
        }
        if let Err(error) = self.save() {
            let mut disk = self.disk.write();
            if let Some(slot) = disk.sandboxes.iter_mut().find(|s| s.id == id) {
                *slot = before;
            }
            return Err(error);
        }

        // A running sandbox takes most changes live. Everything is re-applied
        // on the next start anyway, so a failure here is reported, not fatal
        // to the saved setting.
        let mut problem = None;
        if self.phase(id) == Phase::Running {
            if let Ok(cli) = self.ready_cli() {
                if (next.memory_mb, next.cpus) != (before.memory_mb, before.cpus) {
                    if let Err(e) = cli
                        .run_ok(&docker::update_args(id, next.memory_mb, next.cpus))
                        .await
                    {
                        problem = Some(format!("the new limits apply next start ({e})"));
                    }
                }
                if next.internet != before.internet {
                    let result = async {
                        self.sync_network(&cli, &next).await?;
                        if next.internet {
                            self.isolate(&cli, &next).await?;
                        }
                        Ok::<_, String>(())
                    }
                    .await;
                    if let Err(e) = result {
                        // Fail closed: a sandbox that may now reach the host
                        // does not stay up.
                        let _ = cli
                            .run(&args(&["stop", "-t", "3", &container_name(id)]))
                            .await;
                        self.set_phase(id, Phase::Error, Some(format!("stopped: {e}")));
                        return Ok(());
                    }
                }
                if (next.width, next.height) != (before.width, before.height) {
                    let guest = Guest::new(cli.clone(), id);
                    if let Err(e) = guest
                        .helper(
                            &guest::argv(&[
                                "geometry",
                                &next.width.to_string(),
                                &next.height.to_string(),
                            ]),
                            None,
                            Duration::from_secs(20),
                        )
                        .await
                    {
                        problem = Some(e);
                    }
                }
            }
        }
        if let Some(rt) = self.runtime.write().get_mut(id) {
            rt.error = problem;
        }
        self.emit();
        Ok(())
    }

    pub fn set_stop_on_quit(&self, stop: bool) -> Result<(), String> {
        self.disk.write().stop_on_quit = stop;
        self.save()?;
        self.emit();
        Ok(())
    }

    /// Removes the sandbox, its container and its home volume. Only touches
    /// Docker objects that carry this install's label.
    pub async fn delete(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let lock = self.lock_for(id);
        let _held = lock.lock().await;
        if !self.exists(id) {
            return Ok(());
        }
        match self.docker.read().availability {
            DockerAvailability::Ready | DockerAvailability::Missing => {}
            _ => {
                return Err(
                    "start Docker first, so the sandbox's container and disk are removed too"
                        .into(),
                );
            }
        }
        self.interrupt_calls(id);
        self.viewers.close_sandbox(id);
        if let Ok(cli) = self.ready_cli() {
            let install = self.disk.read().install_id.clone();
            let name = container_name(id);
            if let Some(labels) = self.labels(&cli, "container", &name).await? {
                owned_by(&labels, id, &install)?;
                cli.run_ok(&args(&["rm", "-f", &name])).await?;
            }
            let volume = volume_name(id);
            if let Some(labels) = self.labels(&cli, "volume", &volume).await? {
                owned_by(&labels, id, &install)?;
                cli.run_ok(&args(&["volume", "rm", &volume])).await?;
            }
        }
        self.disk.write().sandboxes.retain(|s| s.id != id);
        self.save()?;
        self.runtime.write().remove(id);
        self.locks.lock().remove(id);
        self.input_locks.lock().remove(id);
        self.emit();
        Ok(())
    }

    /* ── lifecycle ── */

    pub async fn start(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let lock = self.lock_for(id);
        let _held = lock.lock().await;
        let spec = self
            .spec(id)
            .ok_or_else(|| format!("no sandbox called {id}"))?;
        let cli = self.ready_cli()?;
        if self.phase(id) == Phase::Running {
            return Ok(());
        }
        self.set_phase(id, Phase::Starting, None);
        match self.start_inner(&cli, &spec).await {
            Ok(()) => {
                self.set_phase(id, Phase::Running, None);
                Ok(())
            }
            Err(error) => {
                self.set_phase(id, Phase::Error, Some(error.clone()));
                Err(error)
            }
        }
    }

    async fn start_inner(
        self: &Arc<Self>,
        cli: &Arc<DockerCli>,
        spec: &SandboxSpec,
    ) -> Result<(), String> {
        let tag = image::tag(spec.os);
        if self.check_image(cli, spec.os).await != ImageState::Ready {
            self.build_image(cli, spec.os).await?;
        }
        self.ensure_network(cli).await?;

        let install = self.disk.read().install_id.clone();
        let name = container_name(&spec.id);
        match self.labels(cli, "container", &name).await? {
            Some(labels) => owned_by(&labels, &spec.id, &install)?,
            None => {
                let volume = volume_name(&spec.id);
                match self.labels(cli, "volume", &volume).await? {
                    Some(labels) => owned_by(&labels, &spec.id, &install)?,
                    None => {
                        cli.run_ok(&args(&[
                            "volume",
                            "create",
                            "--label",
                            &format!("{}={}", docker::LABEL_SANDBOX, spec.id),
                            "--label",
                            &format!("{}={install}", docker::LABEL_INSTALL),
                            &volume,
                        ]))
                        .await?;
                    }
                }
                cli.run_ok(&docker::create_args(spec, &tag, &install))
                    .await?;
            }
        }

        cli.run_ok(&docker::update_args(&spec.id, spec.memory_mb, spec.cpus))
            .await?;
        self.sync_network(cli, spec).await?;
        self.copy_runtime(cli, spec).await?;
        cli.run_ok(&args(&["start", &name])).await?;

        if spec.internet {
            if let Err(error) = self.isolate(cli, spec).await {
                let _ = cli.run(&args(&["stop", "-t", "3", &name])).await;
                return Err(format!(
                    "could not wall the sandbox off from this computer, so it was stopped: {error}"
                ));
            }
        }

        let guest = Guest::new(cli.clone(), &spec.id);
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            let ready = guest
                .helper(&guest::argv(&["ready"]), None, Duration::from_secs(10))
                .await;
            match ready {
                Ok(_) => break,
                Err(error) if Instant::now() >= deadline => {
                    return Err(format!("the desktop did not come up: {error}"));
                }
                Err(error) if error == "the sandbox is not running" => {
                    let logs = cli
                        .run(&args(&["logs", "--tail", "5", &name]))
                        .await
                        .map(|o| o.stderr_text())
                        .unwrap_or_default();
                    return Err(format!("the sandbox exited while starting. {logs}"));
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(500)).await,
            }
        }
        Ok(())
    }

    /// Writes `guest.py` and the screen size into `/opt/conduit`. Works on a
    /// stopped container, which is when the entrypoint's geometry must be set.
    async fn copy_runtime(&self, cli: &DockerCli, spec: &SandboxSpec) -> Result<(), String> {
        let bundle = image::runtime_bundle(spec.width, spec.height);
        let target = format!("{}:{}", container_name(&spec.id), image::RUNTIME_DIR);
        let mut child = cli
            .command()
            .args(["cp", "-", &target])
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not run docker: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(&bundle)
                .await
                .map_err(|e| format!("could not copy conduit's helper in: {e}"))?;
            drop(stdin);
        }
        let out = child
            .wait_with_output()
            .await
            .map_err(|e| format!("could not copy conduit's helper in: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(docker::docker_error(&String::from_utf8_lossy(&out.stderr)))
        }
    }

    async fn ensure_network(&self, cli: &DockerCli) -> Result<(), String> {
        let exists = cli
            .run(&args(&[
                "network",
                "inspect",
                "--format",
                "{{.Id}}",
                docker::NETWORK,
            ]))
            .await?
            .success();
        if exists {
            return Ok(());
        }
        let created = cli
            .run(&args(&[
                "network",
                "create",
                "--driver",
                "bridge",
                "--label",
                &format!("{}=1", docker::LABEL_NETWORK),
                "--opt",
                "com.docker.network.bridge.enable_icc=false",
                docker::NETWORK,
            ]))
            .await?;
        // Two sandboxes starting together race to create it; losing is fine.
        if created.success() || created.stderr_text().contains("already exists") {
            Ok(())
        } else {
            Err(docker::docker_error(&created.stderr_text()))
        }
    }

    /// Attaches the container to conduit's network, or detaches it from every
    /// network, to match the sandbox's internet setting.
    async fn sync_network(&self, cli: &DockerCli, spec: &SandboxSpec) -> Result<(), String> {
        let name = container_name(&spec.id);
        let networks = cli
            .run_ok(&args(&[
                "inspect",
                "--type",
                "container",
                "--format",
                "{{json .NetworkSettings.Networks}}",
                &name,
            ]))
            .await?;
        let attached: Vec<String> = serde_json::from_str::<serde_json::Value>(networks.trim())
            .ok()
            .and_then(|v| v.as_object().map(|m| m.keys().cloned().collect()))
            .unwrap_or_default();
        if spec.internet {
            if !attached.iter().any(|n| n == docker::NETWORK) {
                self.ensure_network(cli).await?;
                cli.run_ok(&args(&["network", "connect", docker::NETWORK, &name]))
                    .await?;
            }
        } else {
            for network in attached {
                if network == "none" {
                    continue;
                }
                cli.run_ok(&args(&[
                    "network",
                    "disconnect",
                    "--force",
                    &network,
                    &name,
                ]))
                .await?;
            }
        }
        Ok(())
    }

    /// Applies [`ISOLATE_SCRIPT`] inside the running sandbox's network
    /// namespace. See the module docs for why this is not optional.
    async fn isolate(&self, cli: &DockerCli, spec: &SandboxSpec) -> Result<(), String> {
        let out = tokio::time::timeout(
            Duration::from_secs(60),
            cli.run(&args(&[
                "run",
                "--rm",
                "--network",
                &format!("container:{}", container_name(&spec.id)),
                "--cap-add",
                "NET_ADMIN",
                "--user",
                "root",
                "--label",
                &format!("ai.conduit.isolator={}", spec.id),
                "--entrypoint",
                "/bin/sh",
                &image::tag(spec.os),
                "-c",
                ISOLATE_SCRIPT,
            ])),
        )
        .await
        .map_err(|_| "the isolation helper timed out".to_string())??;
        if out.success() {
            tracing::info!(sandbox = %spec.id, blocked = %out.stdout_text().trim(), "sandbox isolated from the host");
            Ok(())
        } else {
            Err(docker::docker_error(&out.stderr_text()))
        }
    }

    async fn labels(
        &self,
        cli: &DockerCli,
        kind: &str,
        name: &str,
    ) -> Result<Option<HashMap<String, String>>, String> {
        let format = if kind == "volume" {
            "{{json .Labels}}"
        } else {
            "{{json .Config.Labels}}"
        };
        let list = if kind == "volume" {
            args(&["volume", "inspect", "--format", format, name])
        } else {
            args(&["inspect", "--type", kind, "--format", format, name])
        };
        let out = cli.run(&list).await?;
        if !out.success() {
            let err = out.stderr_text();
            if err.to_ascii_lowercase().contains("no such") {
                return Ok(None);
            }
            return Err(docker::docker_error(&err));
        }
        let labels: Option<HashMap<String, String>> =
            serde_json::from_str(out.stdout_text().trim()).unwrap_or_default();
        Ok(Some(labels.unwrap_or_default()))
    }

    pub async fn stop(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let lock = self.lock_for(id);
        let _held = lock.lock().await;
        if !self.exists(id) {
            return Err(format!("no sandbox called {id}"));
        }
        let cli = self.ready_cli()?;
        self.set_phase(id, Phase::Stopping, None);
        self.interrupt_calls(id);
        self.viewers.close_sandbox(id);
        let install = self.disk.read().install_id.clone();
        let name = container_name(id);
        let result = async {
            if let Some(labels) = self.labels(&cli, "container", &name).await? {
                owned_by(&labels, id, &install)?;
                cli.run_ok(&args(&["stop", "-t", "5", &name])).await?;
            }
            Ok::<_, String>(())
        }
        .await;
        match result {
            Ok(()) => {
                self.set_phase(id, Phase::Stopped, None);
                Ok(())
            }
            Err(error) => {
                self.set_phase(id, Phase::Error, Some(error.clone()));
                Err(error)
            }
        }
    }

    /// Fires `docker stop` for every running sandbox and returns at once. For
    /// quitting: the stops finish after conduit has gone.
    pub fn stop_all_detached(&self) {
        if !self.disk.read().stop_on_quit {
            return;
        }
        let Some(cli) = self.cli.read().clone() else {
            return;
        };
        let running: Vec<String> = self
            .runtime
            .read()
            .iter()
            .filter(|(_, rt)| matches!(rt.phase, Phase::Running | Phase::Starting))
            .map(|(id, _)| container_name(id))
            .collect();
        if running.is_empty() {
            return;
        }
        let mut cmd = cli.std_command();
        cmd.args(["stop", "-t", "3"]).args(&running);
        if let Err(error) = cmd.spawn() {
            tracing::warn!(%error, "could not stop the sandboxes on quit");
        }
    }

    /* ── image ── */

    fn build_lock(&self, os: SandboxOs) -> Arc<AsyncMutex<()>> {
        self.build_locks.lock().entry(os).or_default().clone()
    }

    /// Builds the desktop image for `os`, streaming progress to the UI. A
    /// second caller for the same OS waits for the first build instead of
    /// starting another.
    async fn build_image(&self, cli: &DockerCli, os: SandboxOs) -> Result<(), String> {
        let lock = self.build_lock(os);
        let _held = lock.lock().await;
        if self.check_image(cli, os).await == ImageState::Ready {
            return Ok(());
        }
        let cancel = CancellationToken::new();
        self.builds.lock().insert(
            os,
            BuildJob {
                cancel: cancel.clone(),
                progress: ImageBuild {
                    os,
                    step: None,
                    total_steps: None,
                    last_line: Some("preparing".into()),
                    started_at: now_millis(),
                },
            },
        );
        self.emit();
        let result = self.run_build(cli, os, &cancel).await;
        self.builds.lock().remove(&os);
        self.images.write().insert(
            os,
            if result.is_ok() {
                ImageState::Ready
            } else {
                ImageState::Missing
            },
        );
        self.emit();
        result
    }

    async fn run_build(
        &self,
        cli: &DockerCli,
        os: SandboxOs,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        let mut child = cli
            .command()
            .args(image::build_args(os))
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not run docker build: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            let context = image::build_context();
            tokio::spawn(async move {
                let _ = stdin.write_all(&context).await;
                let _ = stdin.shutdown().await;
            });
        }
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        for stream in [
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
        ]
        .into_iter()
        .flatten()
        {
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stream).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);

        let mut tail: std::collections::VecDeque<String> = std::collections::VecDeque::new();
        loop {
            tokio::select! {
                line = rx.recv() => {
                    let Some(line) = line else { break };
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if tail.len() == 12 {
                        tail.pop_front();
                    }
                    tail.push_back(trimmed.to_string());
                    self.note_build_line(os, trimmed);
                }
                _ = cancel.cancelled() => {
                    let _ = child.kill().await;
                    return Err("the image build was cancelled".into());
                }
            }
        }
        let status = tokio::select! {
            status = child.wait() => status.map_err(|e| format!("docker build failed: {e}"))?,
            _ = cancel.cancelled() => {
                let _ = child.kill().await;
                return Err("the image build was cancelled".into());
            }
        };
        if status.success() {
            Ok(())
        } else {
            let why = tail
                .iter()
                .rev()
                .find(|l| l.contains("ERROR") || l.contains("error") || l.contains("failed"))
                .or_else(|| tail.back())
                .cloned()
                .unwrap_or_else(|| "docker build failed".into());
            Err(format!("the desktop image did not build: {why}"))
        }
    }

    fn note_build_line(&self, os: SandboxOs, line: &str) {
        {
            let mut builds = self.builds.lock();
            let Some(job) = builds.get_mut(&os) else {
                return;
            };
            if let Some((step, total)) = docker::parse_build_progress(line) {
                job.progress.step = Some(step);
                job.progress.total_steps = Some(total);
            }
            let short: String = line.chars().take(160).collect();
            job.progress.last_line = Some(short);
        }
        let mut last = self.last_build_emit.lock();
        if last.elapsed() >= BUILD_EMIT_INTERVAL {
            *last = Instant::now();
            drop(last);
            self.emit();
        }
    }

    pub fn cancel_build(&self, id: &str) {
        let Some(spec) = self.spec(id) else { return };
        if let Some(job) = self.builds.lock().get(&spec.os) {
            job.cancel.cancel();
        }
    }

    /* ── agent calls ── */

    /// Lets one tool call into sandbox `id`, starting the sandbox first if it
    /// is stopped and set to start on demand. Never builds an image: that
    /// takes minutes, and the agent's call would time out long before.
    pub async fn begin_call(self: &Arc<Self>, id: &str) -> Result<CallGuard, String> {
        let spec = self
            .spec(id)
            .ok_or_else(|| format!("sandbox {id} no longer exists."))?;
        if self.runtime.read().get(id).is_some_and(|r| r.paused) {
            return Err(format!(
                "the user stopped the agent in sandbox \"{}\". retrying will not help — ask them to \
                 resume it from conduit's Sandbox tab.",
                spec.name
            ));
        }
        if self.phase(id) == Phase::Starting {
            // Wait for the start already under way, then look again.
            let lock = self.lock_for(id);
            drop(lock.lock().await);
        }
        match self.phase(id) {
            Phase::Running => {}
            Phase::Stopping => {
                return Err(format!("sandbox \"{}\" is shutting down.", spec.name));
            }
            Phase::Starting | Phase::Stopped | Phase::Error => {
                if !spec.auto_start {
                    return Err(format!(
                        "sandbox \"{}\" is stopped. ask the user to start it from conduit's \
                         Sandbox tab.",
                        spec.name
                    ));
                }
                let cli = self
                    .ready_cli()
                    .map_err(|e| format!("sandbox \"{}\" cannot start: {e}", spec.name))?;
                if self.builds.lock().contains_key(&spec.os) {
                    return Err(format!(
                        "sandbox \"{}\" is still building its desktop image. try again in a few \
                         minutes.",
                        spec.name
                    ));
                }
                if self.check_image(&cli, spec.os).await != ImageState::Ready {
                    return Err(format!(
                        "sandbox \"{}\" has never been started, so its desktop image is not built \
                         yet. ask the user to start it once from conduit's Sandbox tab.",
                        spec.name
                    ));
                }
                self.start(id)
                    .await
                    .map_err(|e| format!("sandbox \"{}\" could not start: {e}", spec.name))?;
            }
        }
        let mut runtime = self.runtime.write();
        let rt = runtime.entry(id.to_string()).or_default();
        rt.calls += 1;
        Ok(CallGuard {
            manager: self.clone(),
            id: id.to_string(),
            cancel: rt.cancel.clone(),
        })
    }

    /// Cancels every call in flight in `id`, without refusing new ones.
    fn interrupt_calls(&self, id: &str) {
        if let Some(rt) = self.runtime.write().get_mut(id) {
            rt.cancel.cancel();
            rt.cancel = CancellationToken::new();
        }
    }

    /// "Stop agent": cancels what is running and refuses what comes next,
    /// until [`SandboxManager::resume`].
    pub fn interrupt(&self, id: &str) {
        self.interrupt_calls(id);
        if let Some(rt) = self.runtime.write().get_mut(id) {
            rt.paused = true;
        }
        self.emit();
    }

    pub fn resume(&self, id: &str) {
        if let Some(rt) = self.runtime.write().get_mut(id) {
            rt.paused = false;
        }
        self.emit();
    }

    /// Panic stop: cancels every call in every sandbox. The global latch in
    /// the gate is what refuses the next ones.
    pub fn interrupt_all(&self) {
        let ids: Vec<String> = self.runtime.read().keys().cloned().collect();
        for id in ids {
            self.interrupt_calls(&id);
        }
    }
}

/// What the UI shows for a sandbox, from its container phase and its image.
fn derive_status(phase: Phase, building: bool, image: ImageState) -> SandboxStatus {
    match phase {
        Phase::Starting if building => SandboxStatus::Building,
        Phase::Starting => SandboxStatus::Starting,
        Phase::Running => SandboxStatus::Running,
        Phase::Stopping => SandboxStatus::Stopping,
        Phase::Stopped | Phase::Error if building => SandboxStatus::Building,
        Phase::Stopped | Phase::Error if image == ImageState::Missing => SandboxStatus::NeedsImage,
        Phase::Error => SandboxStatus::Error,
        Phase::Stopped => SandboxStatus::Stopped,
    }
}

/// Refuses to act on a Docker object that is not this install's sandbox `id`.
/// A container that merely has the right *name* could be anyone's.
fn owned_by(labels: &HashMap<String, String>, id: &str, install: &str) -> Result<(), String> {
    let sandbox = labels.get(docker::LABEL_SANDBOX).map(String::as_str);
    let owner = labels.get(docker::LABEL_INSTALL).map(String::as_str);
    if sandbox == Some(id) && owner == Some(install) {
        Ok(())
    } else {
        Err(format!(
            "a Docker object named after sandbox {id} exists but conduit did not create it \
             for this sandbox, so it was left alone. remove or rename it and try again."
        ))
    }
}

fn missing_hint() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Docker is not installed. install Docker Desktop (or OrbStack) and open it once."
    }
    #[cfg(target_os = "windows")]
    {
        "Docker is not installed. install Docker Desktop and open it once."
    }
    #[cfg(target_os = "linux")]
    {
        "Docker is not installed. install Docker Engine (or Docker Desktop) from docker.com."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_prefers_live_phases_then_image_state() {
        use ImageState::*;
        use Phase::*;
        assert_eq!(derive_status(Running, false, Ready), SandboxStatus::Running);
        assert_eq!(
            derive_status(Starting, true, Missing),
            SandboxStatus::Building
        );
        assert_eq!(
            derive_status(Starting, false, Ready),
            SandboxStatus::Starting
        );
        assert_eq!(
            derive_status(Stopped, true, Missing),
            SandboxStatus::Building
        );
        assert_eq!(
            derive_status(Stopped, false, Missing),
            SandboxStatus::NeedsImage
        );
        assert_eq!(
            derive_status(Stopped, false, Unknown),
            SandboxStatus::Stopped
        );
        assert_eq!(derive_status(Stopped, false, Ready), SandboxStatus::Stopped);
        assert_eq!(derive_status(Error, false, Ready), SandboxStatus::Error);
        assert_eq!(
            derive_status(Error, false, Missing),
            SandboxStatus::NeedsImage
        );
        assert_eq!(
            derive_status(Stopping, false, Ready),
            SandboxStatus::Stopping
        );
    }

    #[test]
    fn only_this_installs_sandbox_is_touched() {
        let labels = |pairs: &[(&str, &str)]| -> HashMap<String, String> {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        let ours = labels(&[
            (docker::LABEL_SANDBOX, "work"),
            (docker::LABEL_INSTALL, "i1"),
        ]);
        assert!(owned_by(&ours, "work", "i1").is_ok());
        // Another install's sandbox with the same id.
        assert!(owned_by(&ours, "work", "i2").is_err());
        // Our install, another sandbox's container under this name.
        assert!(owned_by(&ours, "play", "i1").is_err());
        // Someone's own container that happens to be called conduit-sbx-work.
        assert!(owned_by(&labels(&[("com.example", "x")]), "work", "i1").is_err());
        assert!(owned_by(&HashMap::new(), "work", "i1").is_err());
    }

    /// The isolation script is the only thing standing between an agent in a
    /// sandbox and conduit's unauthenticated endpoint on the host. Pin its
    /// essentials so an edit cannot quietly drop one.
    #[test]
    fn isolation_blocks_the_gateway_and_docker_desktops_host_names() {
        for needle in [
            "ip -4 route show default",
            "host.docker.internal",
            "gateway.docker.internal",
            "-j REJECT",
            "|| exit 1",
            "-C OUTPUT",
            "no gateway address to isolate",
        ] {
            assert!(
                ISOLATE_SCRIPT.contains(needle),
                "isolation script lost `{needle}`"
            );
        }
    }
}
