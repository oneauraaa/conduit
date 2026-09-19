mod history;
mod install;
pub mod model;
mod sidecar;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use rmcp::{service::Peer, RoleServer};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, Wry};
use tokio::sync::{Mutex as AsyncMutex, RwLock as AsyncRwLock};

use crate::state::{BrowserPermissionCategory, Shared};
use model::{
    BrowserDiskState, BrowserHistoryEntry, BrowserInstallState, BrowserInstallStatus, BrowserMode,
    BrowserProfile, BrowserRestartStrategy, BrowserRunStatus, BrowserState, RuntimeManifest,
    RuntimePaths, SidecarEvent, EXPECTED_REVISION,
};
use sidecar::SidecarProcess;

const PROFILE_MARKER: &str = ".conduit-browser-profile";
const INCOGNITO_ID: &str = "incognito";
const MAX_DOWNLOAD_ROWS: usize = 40;

pub const TOOL_NAMES: &[&str] = &[
    "browser_navigate",
    "browser_navigate_back",
    "browser_snapshot",
    "browser_find",
    "browser_click",
    "browser_type",
    "browser_fill_form",
    "browser_hover",
    "browser_drag",
    "browser_drop",
    "browser_select_option",
    "browser_press_key",
    "browser_handle_dialog",
    "browser_file_upload",
    "browser_take_screenshot",
    "browser_wait_for",
    "browser_tabs",
    "browser_resize",
    "browser_close",
    "browser_history",
];

pub struct BrowserManager {
    app: AppHandle<Wry>,
    root: PathBuf,
    runtime_root: PathBuf,
    state: RwLock<BrowserState>,
    owner_session: Mutex<Option<String>>,
    lifecycle: AsyncMutex<()>,
    process: AsyncMutex<Option<Arc<SidecarProcess>>>,
    install_cancel: Mutex<Option<Arc<AtomicBool>>>,
    tab_visible: AtomicBool,
    active_profile_dir: Mutex<Option<PathBuf>>,
    incognito_history: RwLock<Vec<BrowserHistoryEntry>>,
    peers: AsyncRwLock<Vec<Peer<RoleServer>>>,
    previous_window_size: Mutex<Option<(f64, f64)>>,
}

impl BrowserManager {
    pub fn new(app: AppHandle<Wry>) -> Arc<Self> {
        let root = app
            .path()
            .app_local_data_dir()
            .unwrap_or_else(|_| std::env::temp_dir().join("conduit"))
            .join("browser");
        let _ = fs::create_dir_all(root.join("profiles"));
        cleanup_orphan_incognito(&root);

        let disk = load_disk_state(&root).unwrap_or_else(|| {
            let profile = BrowserProfile {
                id: uuid::Uuid::new_v4().to_string(),
                name: "Default".into(),
                incognito: false,
            };
            BrowserDiskState {
                mode: BrowserMode::Headless,
                selected_profile_id: profile.id.clone(),
                profiles: vec![profile],
            }
        });
        for profile in &disk.profiles {
            let _ = ensure_profile_directory(&root, profile);
        }
        let _ = save_disk_state(&root, &disk);

        let (install, _) = installed_runtime(&root);
        let mut profiles = disk.profiles.clone();
        profiles.push(BrowserProfile {
            id: INCOGNITO_ID.into(),
            name: "Incognito".into(),
            incognito: true,
        });
        let selected = if profiles
            .iter()
            .any(|profile| profile.id == disk.selected_profile_id)
        {
            disk.selected_profile_id
        } else {
            profiles[0].id.clone()
        };
        Arc::new(Self {
            app,
            runtime_root: root.join("runtime"),
            root,
            state: RwLock::new(BrowserState {
                install,
                run_status: BrowserRunStatus::Stopped,
                mode: disk.mode,
                selected_profile_id: selected,
                profiles,
                tabs: Vec::new(),
                downloads: Vec::new(),
                preview: None,
                stop_latched: false,
                owner: None,
            }),
            lifecycle: AsyncMutex::new(()),
            process: AsyncMutex::new(None),
            owner_session: Mutex::new(None),
            install_cancel: Mutex::new(None),
            tab_visible: AtomicBool::new(false),
            active_profile_dir: Mutex::new(None),
            incognito_history: RwLock::new(Vec::new()),
            peers: AsyncRwLock::new(Vec::new()),
            previous_window_size: Mutex::new(None),
        })
    }

    pub fn snapshot(&self) -> BrowserState {
        self.state.read().clone()
    }

    pub fn ready(&self) -> bool {
        self.state.read().install.status == BrowserInstallStatus::Ready
    }

    pub(crate) fn runtime_root(&self) -> &Path {
        &self.runtime_root
    }

    fn emit(&self) {
        let _ = self.app.emit("browser:state", self.snapshot());
    }

    pub async fn register_peer(&self, peer: Peer<RoleServer>) {
        self.peers.write().await.push(peer);
    }

    async fn notify_tool_list_changed(&self) {
        let mut peers = self.peers.write().await;
        let mut live = Vec::with_capacity(peers.len());
        for peer in peers.drain(..) {
            if peer.notify_tool_list_changed().await.is_ok() {
                live.push(peer);
            }
        }
        *peers = live;
    }

    pub async fn install(self: Arc<Self>) -> Result<(), String> {
        install::install(self).await
    }

    pub async fn refresh_install_metadata(self: Arc<Self>) -> Result<BrowserState, String> {
        let status = self.state.read().install.status;
        if matches!(
            status,
            BrowserInstallStatus::Ready
                | BrowserInstallStatus::Downloading
                | BrowserInstallStatus::Verifying
                | BrowserInstallStatus::Installing
        ) {
            return Ok(self.snapshot());
        }
        if let Err(error) = install::refresh_manifest(self.clone()).await {
            let mut state = self.state.write();
            if matches!(
                state.install.status,
                BrowserInstallStatus::Unavailable | BrowserInstallStatus::Error
            ) {
                state.install.error = Some(error);
                drop(state);
                self.emit();
            }
        }
        Ok(self.snapshot())
    }

    pub fn cancel_install(&self) {
        if let Some(cancel) = self.install_cancel.lock().as_ref() {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    pub(crate) fn begin_install(&self, cancel: Arc<AtomicBool>) -> Result<(), String> {
        let mut slot = self.install_cancel.lock();
        if slot.is_some() {
            return Err("a Chromium download is already in progress".into());
        }
        let mut state = self.state.write();
        if state.run_status != BrowserRunStatus::Stopped {
            return Err("stop Chromium before installing an update".into());
        }
        *slot = Some(cancel);
        state.install.status = BrowserInstallStatus::Downloading;
        state.install.downloaded_bytes = 0;
        state.install.error = None;
        drop(state);
        drop(slot);
        self.emit();
        if let Some(manager) = self
            .app
            .try_state::<Shared>()
            .map(|state| state.browser.clone())
        {
            tauri::async_runtime::spawn(async move { manager.notify_tool_list_changed().await });
        }
        Ok(())
    }

    pub(crate) fn set_install_progress(
        &self,
        status: BrowserInstallStatus,
        downloaded: u64,
        total: Option<u64>,
        error: Option<String>,
    ) {
        {
            let mut state = self.state.write();
            state.install.status = status;
            state.install.downloaded_bytes = downloaded;
            state.install.total_bytes = total;
            state.install.error = error;
        }
        self.emit();
    }

    pub(crate) fn set_install_metadata(&self, total: u64) {
        let mut state = self.state.write();
        // A download may have started while the network preflight was in
        // flight. In that case the installer owns all progress fields.
        if matches!(
            state.install.status,
            BrowserInstallStatus::Unavailable | BrowserInstallStatus::Error
        ) {
            state.install.total_bytes = Some(total);
            state.install.error = None;
            drop(state);
            self.emit();
        }
    }

    pub(crate) fn finish_install(&self, result: Result<(), String>) {
        self.install_cancel.lock().take();
        let was_ready = self.ready();
        let (mut install, _) = installed_runtime(&self.root);
        install = finish_install_state(install, result);
        self.state.write().install = install;
        self.emit();
        if was_ready != self.ready() {
            let manager = self
                .app
                .try_state::<Shared>()
                .map(|state| state.browser.clone());
            if let Some(manager) = manager {
                tauri::async_runtime::spawn(
                    async move { manager.notify_tool_list_changed().await },
                );
            }
        }
    }

    pub async fn start(
        self: &Arc<Self>,
        explicit: bool,
        agent: Option<String>,
        session: Option<String>,
    ) -> Result<BrowserState, String> {
        let _lifecycle = self.lifecycle.lock().await;
        self.start_inner(explicit, agent, session).await
    }

    async fn start_inner(
        self: &Arc<Self>,
        explicit: bool,
        agent: Option<String>,
        session: Option<String>,
    ) -> Result<BrowserState, String> {
        validate_start(self.ready(), self.state.read().stop_latched, explicit)?;
        if explicit {
            let mut state = self.state.write();
            state.stop_latched = false;
        }
        self.claim_owner(session.as_deref(), agent.as_deref())?;
        // Hold the process slot through launch so concurrent calls cannot each
        // spawn their own Chromium before either one publishes the process.
        let mut process_slot = self.process.lock().await;
        if process_slot.is_some() {
            return Ok(self.snapshot());
        }

        let runtime = runtime_paths(&self.root)?;
        let (profile, mode) = {
            let mut state = self.state.write();
            // Installation and launch both claim the stopped state under this
            // write lock. Recheck compatibility here so an installer that won
            // the race cannot be followed by a stale lazy launch.
            validate_start(
                state.install.status == BrowserInstallStatus::Ready,
                state.stop_latched,
                explicit,
            )?;
            let profile = state
                .profiles
                .iter()
                .find(|profile| profile.id == state.selected_profile_id)
                .cloned()
                .ok_or("the selected browser profile no longer exists")?;
            state.run_status = BrowserRunStatus::Starting;
            state.install.error = None;
            (profile, state.mode)
        };
        self.emit();

        let launch_directories = (|| -> Result<(PathBuf, PathBuf), String> {
            let profile_dir = if profile.incognito {
                let path = self
                    .root
                    .join(format!("incognito-{}", uuid::Uuid::new_v4()));
                fs::create_dir_all(&path)
                    .map_err(|e| format!("could not create Incognito profile: {e}"))?;
                if let Err(error) = fs::write(path.join(PROFILE_MARKER), INCOGNITO_ID) {
                    let _ = fs::remove_dir_all(&path);
                    return Err(format!("could not mark Incognito profile: {error}"));
                }
                path
            } else {
                ensure_profile_directory(&self.root, &profile)?
            };
            *self.active_profile_dir.lock() = Some(profile_dir.clone());
            let output_dir = dirs::download_dir()
                .unwrap_or_else(|| self.root.join("downloads"))
                .join("conduit")
                .join(&profile.id);
            fs::create_dir_all(&output_dir)
                .map_err(|e| format!("could not create browser download folder: {e}"))?;
            Ok((profile_dir, output_dir))
        })();
        let (profile_dir, output_dir) = match launch_directories {
            Ok(paths) => paths,
            Err(error) => {
                self.fail_start(&error);
                return Err(error);
            }
        };

        let process = match SidecarProcess::spawn(self.clone(), &runtime.sidecar) {
            Ok(process) => Arc::new(process),
            Err(error) => {
                self.fail_start(&error);
                return Err(error);
            }
        };
        // Publish the child before the potentially slow Chromium launch. Stop
        // and Panic Stop can now kill startup immediately instead of waiting
        // for a launch timeout.
        *process_slot = Some(process.clone());
        drop(process_slot);

        match process
            .initialize(runtime, profile, profile_dir, output_dir, mode)
            .await
        {
            Ok(()) => {
                self.state.write().run_status = BrowserRunStatus::Running;
                self.emit();
                if mode == BrowserMode::Headless && self.tab_visible.load(Ordering::Acquire) {
                    if let Err(error) = process.set_preview(true).await {
                        tracing::warn!("could not start the headless Browser tab preview: {error}");
                    }
                }
                Ok(self.snapshot())
            }
            Err(error) => {
                let mut process_slot = self.process.lock().await;
                if process_slot
                    .as_ref()
                    .is_some_and(|current| current.id() == process.id())
                {
                    process_slot.take();
                }
                drop(process_slot);
                process.kill().await;
                self.fail_start(&error);
                Err(error)
            }
        }
    }

    fn fail_start(&self, error: &str) {
        self.owner_session.lock().take();
        self.cleanup_incognito();
        let mut state = self.state.write();
        state.run_status = BrowserRunStatus::Crashed;
        state.install.error = Some(error.to_string());
        state.owner = None;
        drop(state);
        self.emit();
    }

    pub async fn stop(&self, latch: bool) -> BrowserState {
        self.prepare_stop(latch);
        if let Some(process) = self.process.lock().await.clone() {
            process.kill().await;
        }
        let _lifecycle = self.lifecycle.lock().await;
        self.stop_inner(latch).await
    }

    async fn stop_inner(&self, latch: bool) -> BrowserState {
        self.prepare_stop(latch);
        if let Some(process) = self.process.lock().await.take() {
            process.stop().await;
        }
        self.finish_stop()
    }

    fn prepare_stop(&self, latch: bool) {
        if let Some(state) = self.app.try_state::<Shared>() {
            state.cancel_browser_approvals();
            if latch {
                state.clear_session_grants();
            }
        }
        {
            let mut state = self.state.write();
            state.run_status = BrowserRunStatus::Stopping;
            state.stop_latched |= latch;
        }
        self.emit();
    }

    fn finish_stop(&self) -> BrowserState {
        self.cleanup_incognito();
        self.owner_session.lock().take();
        {
            let mut state = self.state.write();
            state.run_status = BrowserRunStatus::Stopped;
            state.tabs.clear();
            state.preview = None;
            state.owner = None;
        }
        self.emit();
        self.snapshot()
    }

    pub async fn panic_stop(&self) {
        self.prepare_stop(true);
        if let Some(process) = self.process.lock().await.clone() {
            process.kill().await;
        }
        let _lifecycle = self.lifecycle.lock().await;
        if let Some(process) = self.process.lock().await.take() {
            process.kill().await;
        }
        self.finish_stop();
    }

    pub fn release_owner(&self) {
        self.owner_session.lock().take();
        self.state.write().owner = None;
        self.emit();
    }

    /// Releases ownership only when the disconnecting MCP session owns it.
    pub fn release_owner_for(&self, session: &str) {
        let mut owner_session = self.owner_session.lock();
        if owner_session.as_deref() != Some(session) {
            return;
        }
        owner_session.take();
        self.state.write().owner = None;
        self.emit();
    }

    fn claim_owner(&self, session: Option<&str>, agent: Option<&str>) -> Result<(), String> {
        // Pressing Start in the Browser tab warms Chromium, but must not reserve
        // it for an invented client. Ownership starts with the first real MCP
        // caller and is released with Conduit's existing idle session.
        let Some(session) = session else {
            return Ok(());
        };
        let owner = agent.unwrap_or("MCP client");
        let mut owner_session = self.owner_session.lock();
        let mut state = self.state.write();
        claim_owner_slot(&mut owner_session, &mut state.owner, session, owner)
    }

    /// Reserves the browser control session before any approval UI is shown.
    /// The MCP entry point calls this only after runtime, Panic Stop, and tool
    /// switches have passed, so a non-owner cannot create a session grant for
    /// the client that actually owns Chromium.
    pub(crate) fn claim_owner_for_call(
        &self,
        session: &str,
        agent: Option<&str>,
    ) -> Result<(), String> {
        self.claim_owner(Some(session), agent)
    }

    pub async fn call_tool(
        self: &Arc<Self>,
        name: &str,
        arguments: Value,
        agent: Option<String>,
        session: String,
        direct_grant: Option<BrowserPermissionCategory>,
    ) -> Result<Value, String> {
        if !TOOL_NAMES.contains(&name) || name == "browser_history" {
            return Err(format!("unsupported browser tool: {name}"));
        }
        self.claim_owner(Some(&session), agent.as_deref())?;
        self.start(false, agent.clone(), Some(session)).await?;
        let process = self
            .process
            .lock()
            .await
            .clone()
            .ok_or("Chromium did not start")?;
        process
            .call_tool(
                name,
                arguments,
                agent.as_deref(),
                direct_grant.map(BrowserPermissionCategory::as_key),
            )
            .await
    }

    pub async fn set_preview(&self, visible: bool) -> Result<(), String> {
        if let Some(process) = self.process.lock().await.clone() {
            process.set_preview(visible).await?;
        } else if !visible {
            self.state.write().preview = None;
            self.emit();
        }
        Ok(())
    }

    pub fn download_directory(&self) -> PathBuf {
        let profile_id = self.state.read().selected_profile_id.clone();
        dirs::download_dir()
            .unwrap_or_else(|| self.root.join("downloads"))
            .join("conduit")
            .join(profile_id)
    }

    pub async fn set_tab_visible(&self, visible: bool) -> Result<(), String> {
        self.tab_visible.store(visible, Ordering::Release);
        self.set_preview(visible && self.state.read().mode == BrowserMode::Headless)
            .await?;
        let Some(window) = self.app.get_webview_window("main") else {
            return Ok(());
        };
        let scale = window.scale_factor().map_err(|e| e.to_string())?;
        let current: tauri::LogicalSize<f64> = window
            .inner_size()
            .map_err(|e| e.to_string())?
            .to_logical(scale);
        let target = if visible {
            let mut previous = self.previous_window_size.lock();
            if previous.is_none() {
                *previous = Some((current.width, current.height));
            }
            (1100.0_f64.max(current.width), 760.0_f64.max(current.height))
        } else {
            self.previous_window_size
                .lock()
                .take()
                .unwrap_or((current.width, current.height))
        };
        let steps = 8;
        for step in 1..=steps {
            let progress = step as f64 / steps as f64;
            let eased = 1.0 - (1.0 - progress).powi(3);
            let width = current.width + (target.0 - current.width) * eased;
            let height = current.height + (target.1 - current.height) * eased;
            window
                .set_size(tauri::LogicalSize::new(width, height))
                .map_err(|e| format!("could not resize the Conduit window: {e}"))?;
            tokio::time::sleep(std::time::Duration::from_millis(18)).await;
        }
        Ok(())
    }

    pub async fn set_mode(
        self: &Arc<Self>,
        mode: BrowserMode,
        restart: Option<BrowserRestartStrategy>,
    ) -> Result<BrowserState, String> {
        self.change_runtime_setting(Some(mode), None, restart).await
    }

    pub async fn select_profile(
        self: &Arc<Self>,
        profile_id: String,
        restart: Option<BrowserRestartStrategy>,
    ) -> Result<BrowserState, String> {
        if !self
            .state
            .read()
            .profiles
            .iter()
            .any(|profile| profile.id == profile_id)
        {
            return Err("browser profile not found".into());
        }
        self.change_runtime_setting(None, Some(profile_id), restart)
            .await
    }

    async fn change_runtime_setting(
        self: &Arc<Self>,
        mode: Option<BrowserMode>,
        profile_id: Option<String>,
        restart: Option<BrowserRestartStrategy>,
    ) -> Result<BrowserState, String> {
        let _lifecycle = self.lifecycle.lock().await;
        let running = self.process.lock().await.is_some();
        if running && restart.is_none() {
            return Err("changing a running browser requires a restart choice".into());
        }
        let owner_session = self.owner_session.lock().clone();
        let owner = self.state.read().owner.clone();
        let urls: Vec<String> = if running && restart == Some(BrowserRestartStrategy::ReopenUrls) {
            self.state
                .read()
                .tabs
                .iter()
                .filter(|tab| tab.url.starts_with("http://") || tab.url.starts_with("https://"))
                .map(|tab| tab.url.clone())
                .collect()
        } else {
            Vec::new()
        };
        if running {
            self.stop_inner(false).await;
        }
        {
            let mut state = self.state.write();
            if let Some(mode) = mode {
                state.mode = mode;
            }
            if let Some(profile_id) = profile_id {
                state.selected_profile_id = profile_id;
            }
        }
        self.save_disk()?;
        self.emit();
        if running {
            self.start_inner(true, owner, owner_session).await?;
            let process = self
                .process
                .lock()
                .await
                .clone()
                .ok_or("Chromium did not restart")?;
            for (index, url) in urls.into_iter().enumerate() {
                let (name, arguments) = if index == 0 {
                    ("browser_navigate", json!({ "url": url }))
                } else {
                    ("browser_tabs", json!({ "action": "new", "url": url }))
                };
                let _ = process
                    .call_tool(
                        name,
                        arguments,
                        None,
                        Some(BrowserPermissionCategory::OpenWebsites.as_key()),
                    )
                    .await;
            }
        }
        Ok(self.snapshot())
    }

    pub fn create_profile(&self, name: String) -> Result<BrowserState, String> {
        let name = clean_profile_name(&name)?;
        let profile = BrowserProfile {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            incognito: false,
        };
        {
            let state = self.state.read();
            ensure_unique_name(&state.profiles, &profile.name, None)?;
        }
        ensure_profile_directory(&self.root, &profile)?;
        let insert_at = self.state.read().profiles.len().saturating_sub(1);
        self.state.write().profiles.insert(insert_at, profile);
        self.save_disk()?;
        self.emit();
        Ok(self.snapshot())
    }

    pub fn rename_profile(&self, id: String, name: String) -> Result<BrowserState, String> {
        let name = clean_profile_name(&name)?;
        {
            let state = self.state.read();
            ensure_unique_name(&state.profiles, &name, Some(&id))?;
        }
        let mut state = self.state.write();
        let profile = state
            .profiles
            .iter_mut()
            .find(|profile| profile.id == id && !profile.incognito)
            .ok_or("persistent browser profile not found")?;
        profile.name = name;
        drop(state);
        self.save_disk()?;
        self.emit();
        Ok(self.snapshot())
    }

    pub fn delete_profile(&self, id: String) -> Result<BrowserState, String> {
        let mut state = self.state.write();
        if matches!(
            state.run_status,
            BrowserRunStatus::Running | BrowserRunStatus::Starting
        ) && state.selected_profile_id == id
        {
            return Err("stop Chromium before deleting the active profile".into());
        }
        let persistent = state
            .profiles
            .iter()
            .filter(|profile| !profile.incognito)
            .count();
        if persistent <= 1 {
            return Err("the last persistent browser profile cannot be deleted".into());
        }
        let index = state
            .profiles
            .iter()
            .position(|profile| profile.id == id && !profile.incognito)
            .ok_or("persistent browser profile not found")?;
        // Verify ownership and remove the on-disk profile before changing the
        // in-memory list. A refused or failed deletion must leave UI state
        // untouched rather than making the profile disappear until restart.
        remove_profile_directory(&self.root, &id)?;
        state.profiles.remove(index);
        if state.selected_profile_id == id {
            state.selected_profile_id = state
                .profiles
                .iter()
                .find(|profile| !profile.incognito)
                .expect("one persistent profile remains")
                .id
                .clone();
        }
        drop(state);
        self.save_disk()?;
        self.emit();
        Ok(self.snapshot())
    }

    pub fn history(
        &self,
        query: Option<&str>,
        before: Option<&str>,
        limit: usize,
        agent: Option<&str>,
        session: &str,
    ) -> Result<Value, String> {
        self.claim_owner(Some(session), agent)?;
        let state = self.state.read();
        let profile_id = state.selected_profile_id.clone();
        let incognito = profile_id == INCOGNITO_ID;
        drop(state);
        let (entries, cursor) = if incognito {
            let before_entry = history::parse_cursor(before)?;
            let needle = query.map(str::to_lowercase);
            let mut entries: Vec<_> = self
                .incognito_history
                .read()
                .iter()
                .filter(|entry| {
                    before_entry.as_ref().is_none_or(|(at, id)| {
                        (entry.visited_at, entry.id.as_str()) < (*at, id.as_str())
                    })
                })
                .filter(|entry| {
                    needle.as_ref().is_none_or(|q| {
                        entry.url.to_lowercase().contains(q)
                            || entry.title.to_lowercase().contains(q)
                    })
                })
                .cloned()
                .collect();
            entries.sort_by(|a, b| {
                b.visited_at
                    .cmp(&a.visited_at)
                    .then_with(|| b.id.cmp(&a.id))
            });
            let has_more = entries.len() > limit;
            entries.truncate(limit);
            let cursor = has_more
                .then(|| {
                    entries
                        .last()
                        .map(|last| format!("{}:{}", last.visited_at, last.id))
                })
                .flatten();
            (entries, cursor)
        } else {
            history::read(&self.root, &profile_id, query, before, limit)?
        };
        Ok(json!({
            "profileId": profile_id,
            "incognito": incognito,
            "entries": entries,
            "nextCursor": cursor,
        }))
    }

    pub(crate) fn handle_sidecar_event(&self, event: SidecarEvent) {
        match event.event.as_str() {
            "tabs" => self.state.write().tabs = event.tabs,
            "frame" => self.state.write().preview = event.frame,
            "history" => {
                if let Some(mut entry) = event.history {
                    let profile_id = self.state.read().selected_profile_id.clone();
                    entry.profile_id = profile_id.clone();
                    if profile_id == INCOGNITO_ID {
                        self.incognito_history.write().push(entry);
                    } else if let Err(error) = history::append(&self.root, &entry) {
                        tracing::warn!("{error}");
                    }
                }
            }
            "download" => {
                if let Some(download) = event.download {
                    let mut state = self.state.write();
                    if let Some(row) = state.downloads.iter_mut().find(|row| row.id == download.id)
                    {
                        *row = download;
                    } else {
                        state.downloads.insert(0, download);
                        state.downloads.truncate(MAX_DOWNLOAD_ROWS);
                    }
                }
            }
            "crash" => {
                let mut state = self.state.write();
                state.run_status = BrowserRunStatus::Crashed;
                state.install.error = event.error;
            }
            _ => return,
        }
        self.emit();
    }

    pub(crate) async fn approve_side_effect(
        &self,
        category: &str,
        detail: Option<String>,
        agent: Option<String>,
        tool: Option<&str>,
    ) -> bool {
        let Some(category) = BrowserPermissionCategory::from_key(category) else {
            return false;
        };
        let Some(state) = self.app.try_state::<Shared>() else {
            return false;
        };
        if !self.ready()
            || tool.is_some_and(|tool| {
                !TOOL_NAMES.contains(&tool) || !state.settings().tool_enabled(tool)
            })
        {
            return false;
        }
        let allowed =
            crate::mcp::gate::approve_browser_category(state.inner(), category, detail, agent)
                .await;
        allowed
            && !state.is_aborted()
            && self.ready()
            && tool.is_none_or(|tool| state.settings().tool_enabled(tool))
    }

    pub(crate) async fn sidecar_exited(&self, process_id: &str) {
        let mut process = self.process.lock().await;
        if process
            .as_ref()
            .is_none_or(|current| current.id() != process_id)
        {
            // Stop/restart may have already removed this sidecar and installed
            // a new one. A delayed EOF from the old reader must not tear down
            // the replacement process or overwrite its state as crashed.
            return;
        }
        process.take();
        drop(process);
        self.owner_session.lock().take();
        let mut state = self.state.write();
        state.owner = None;
        let crashed = !matches!(
            state.run_status,
            BrowserRunStatus::Stopping | BrowserRunStatus::Stopped
        );
        if crashed {
            state.run_status = BrowserRunStatus::Crashed;
            state.install.error = Some("the Chromium sidecar exited unexpectedly".into());
        }
        drop(state);
        if crashed {
            if let Some(state) = self.app.try_state::<Shared>() {
                state.cancel_browser_approvals();
                state.clear_session_grants();
            }
        }
        self.cleanup_incognito();
        self.emit();
    }

    fn cleanup_incognito(&self) {
        self.incognito_history.write().clear();
        if let Some(path) = self.active_profile_dir.lock().take() {
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("incognito-"))
            {
                let _ = remove_profile_path(&self.root, &path, INCOGNITO_ID);
            }
        }
    }

    fn save_disk(&self) -> Result<(), String> {
        let state = self.state.read();
        save_disk_state(
            &self.root,
            &BrowserDiskState {
                mode: state.mode,
                selected_profile_id: state.selected_profile_id.clone(),
                profiles: state
                    .profiles
                    .iter()
                    .filter(|profile| !profile.incognito)
                    .cloned()
                    .collect(),
            },
        )
    }
}

fn finish_install_state(
    mut installed: BrowserInstallState,
    result: Result<(), String>,
) -> BrowserInstallState {
    if let Err(error) = result {
        // Installation is staged and promoted atomically. If verification or
        // promotion fails while a compatible active runtime still exists,
        // restore its availability instead of hiding a bundle that is known
        // to work. A revision mismatch remains Unavailable/Error as intended.
        if installed.status == BrowserInstallStatus::Ready {
            installed.error = Some(format!(
                "Chromium update failed; continuing with the previous verified runtime: {error}"
            ));
        } else {
            installed.status = BrowserInstallStatus::Error;
            installed.error = Some(error);
        }
    }
    installed
}

fn validate_start(ready: bool, stop_latched: bool, explicit: bool) -> Result<(), String> {
    if !ready {
        return Err("download the required Chromium bundle from the Browser tab first".into());
    }
    if stop_latched && !explicit {
        return Err(
            "Chromium was stopped by the user; they must press Start in the Browser tab".into(),
        );
    }
    Ok(())
}

fn claim_owner_slot(
    owner_session: &mut Option<String>,
    owner_label: &mut Option<String>,
    session: &str,
    owner: &str,
) -> Result<(), String> {
    if let Some(current_session) = owner_session.as_deref() {
        if current_session != session {
            let current = owner_label.as_deref().unwrap_or("another MCP client");
            return Err(format!(
                "Chromium is currently controlled by {current}; retry after its session becomes idle"
            ));
        }
    } else {
        *owner_session = Some(session.into());
        *owner_label = Some(owner.into());
    }
    Ok(())
}

fn load_disk_state(root: &Path) -> Option<BrowserDiskState> {
    let bytes = fs::read(root.join("state.json")).ok()?;
    let state: BrowserDiskState = serde_json::from_slice(&bytes).ok()?;
    if state.profiles.is_empty() {
        return None;
    }
    let mut ids = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for profile in &state.profiles {
        let clean_name = clean_profile_name(&profile.name).ok()?;
        if profile.incognito
            || uuid::Uuid::parse_str(&profile.id).is_err()
            || clean_name != profile.name
            || !ids.insert(profile.id.clone())
            || !names.insert(profile.name.to_lowercase())
        {
            return None;
        }
    }
    Some(state)
}

fn save_disk_state(root: &Path, state: &BrowserDiskState) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|e| format!("could not create browser state folder: {e}"))?;
    let temporary = root.join("state.json.next");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(state)
            .map_err(|e| format!("could not encode browser state: {e}"))?,
    )
    .map_err(|e| format!("could not save browser state: {e}"))?;
    let current = root.join("state.json");
    let previous = root.join("state.json.previous");
    if previous.exists() {
        fs::remove_file(&previous)
            .map_err(|e| format!("could not remove old browser state: {e}"))?;
    }
    if current.exists() {
        fs::rename(&current, &previous)
            .map_err(|e| format!("could not preserve browser state: {e}"))?;
    }
    if let Err(error) = fs::rename(&temporary, &current) {
        if previous.exists() {
            let _ = fs::rename(&previous, &current);
        }
        return Err(format!("could not activate browser state: {error}"));
    }
    if previous.exists() {
        let _ = fs::remove_file(previous);
    }
    Ok(())
}

fn ensure_profile_directory(root: &Path, profile: &BrowserProfile) -> Result<PathBuf, String> {
    let directory = root.join("profiles").join(&profile.id);
    fs::create_dir_all(directory.join("user-data"))
        .map_err(|e| format!("could not create browser profile: {e}"))?;
    fs::write(directory.join(PROFILE_MARKER), &profile.id)
        .map_err(|e| format!("could not mark browser profile: {e}"))?;
    Ok(directory.join("user-data"))
}

fn remove_profile_directory(root: &Path, id: &str) -> Result<(), String> {
    remove_profile_path(root, &root.join("profiles").join(id), id)
}

fn remove_profile_path(root: &Path, target: &Path, expected_id: &str) -> Result<(), String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("could not resolve browser root: {e}"))?;
    let target = target
        .canonicalize()
        .map_err(|e| format!("could not resolve browser profile: {e}"))?;
    if target.parent() != Some(root.join("profiles").as_path())
        && target.parent() != Some(root.as_path())
    {
        return Err("refusing to delete a browser profile outside Conduit's browser folder".into());
    }
    let marker = fs::read_to_string(target.join(PROFILE_MARKER))
        .map_err(|_| "refusing to delete an unowned browser profile")?;
    if marker != expected_id {
        return Err("refusing to delete a browser profile with the wrong ownership marker".into());
    }
    fs::remove_dir_all(target).map_err(|e| format!("could not delete browser profile: {e}"))
}

fn cleanup_orphan_incognito(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with("incognito-")
        {
            let _ = remove_profile_path(root, &path, INCOGNITO_ID);
        }
    }
}

fn clean_profile_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err("profile names must contain 1–64 printable characters".into());
    }
    Ok(name.into())
}

fn ensure_unique_name(
    profiles: &[BrowserProfile],
    name: &str,
    except: Option<&str>,
) -> Result<(), String> {
    let folded = name.to_lowercase();
    if profiles
        .iter()
        .any(|profile| Some(profile.id.as_str()) != except && profile.name.to_lowercase() == folded)
    {
        return Err("browser profile names must be unique".into());
    }
    Ok(())
}

fn installed_runtime(root: &Path) -> (BrowserInstallState, Option<RuntimePaths>) {
    let manifest_path = root.join("runtime").join("active").join("manifest.json");
    let Ok(bytes) = fs::read(manifest_path) else {
        return (BrowserInstallState::default(), None);
    };
    let Ok(manifest) = serde_json::from_slice::<RuntimeManifest>(&bytes) else {
        return (BrowserInstallState::default(), None);
    };
    let compatible = manifest.revision == EXPECTED_REVISION
        && install::release_target().is_ok_and(|target| manifest.target == target)
        && install::safe_bundle_path(&manifest.sidecar_path)
        && install::safe_bundle_path(&manifest.chromium_path);
    let active = root.join("runtime").join("active");
    let paths = RuntimePaths {
        sidecar: active.join(&manifest.sidecar_path),
        chromium: active.join(&manifest.chromium_path),
        revision: manifest.revision.clone(),
    };
    let ready = compatible && paths.sidecar.is_file() && paths.chromium.is_file();
    (
        BrowserInstallState {
            status: if ready {
                BrowserInstallStatus::Ready
            } else {
                BrowserInstallStatus::Unavailable
            },
            expected_revision: EXPECTED_REVISION.into(),
            installed_revision: Some(manifest.revision),
            downloaded_bytes: 0,
            total_bytes: Some(manifest.size),
            error: None,
        },
        ready.then_some(paths),
    )
}

fn runtime_paths(root: &Path) -> Result<RuntimePaths, String> {
    installed_runtime(root)
        .1
        .ok_or_else(|| "the required Chromium runtime is not installed".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_names_are_case_insensitively_unique() {
        let profiles = vec![BrowserProfile {
            id: "one".into(),
            name: "Work".into(),
            incognito: false,
        }];
        assert!(ensure_unique_name(&profiles, "work", None).is_err());
        assert!(ensure_unique_name(&profiles, "work", Some("one")).is_ok());
        let profiles = vec![BrowserProfile {
            id: "two".into(),
            name: "Ärea".into(),
            incognito: false,
        }];
        assert!(ensure_unique_name(&profiles, "äREA", None).is_err());
    }

    #[test]
    fn malformed_profile_state_is_rejected_before_paths_are_used() {
        let root =
            std::env::temp_dir().join(format!("conduit-profile-state-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let state = BrowserDiskState {
            mode: BrowserMode::Headless,
            selected_profile_id: "../../escape".into(),
            profiles: vec![BrowserProfile {
                id: "../../escape".into(),
                name: "Default".into(),
                incognito: false,
            }],
        };
        fs::write(root.join("state.json"), serde_json::to_vec(&state).unwrap()).unwrap();
        assert!(load_disk_state(&root).is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn only_the_published_targets_are_accepted() {
        let target = install::release_target();
        if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            assert_eq!(target.unwrap(), "linux-x64");
        }
    }

    #[test]
    fn manual_stop_blocks_lazy_start_until_an_explicit_start() {
        assert!(validate_start(true, true, false).is_err());
        assert!(validate_start(true, true, true).is_ok());
        assert!(validate_start(true, false, false).is_ok());
        assert!(validate_start(false, false, true).is_err());
    }

    #[test]
    fn one_session_exclusively_owns_all_browser_tools() {
        let mut session = None;
        let mut owner = None;
        claim_owner_slot(&mut session, &mut owner, "session-a", "Agent A").unwrap();
        claim_owner_slot(&mut session, &mut owner, "session-a", "Agent A").unwrap();
        let error = claim_owner_slot(&mut session, &mut owner, "session-b", "Agent B").unwrap_err();
        assert!(error.contains("Agent A"));
        session = None;
        owner = None;
        claim_owner_slot(&mut session, &mut owner, "session-b", "Agent B").unwrap();
        assert_eq!(owner.as_deref(), Some("Agent B"));
    }

    #[test]
    fn a_failed_update_keeps_a_compatible_active_runtime_ready() {
        let ready = BrowserInstallState {
            status: BrowserInstallStatus::Ready,
            ..BrowserInstallState::default()
        };
        let retained = finish_install_state(ready, Err("verification failed".into()));
        assert_eq!(retained.status, BrowserInstallStatus::Ready);
        assert!(retained
            .error
            .unwrap()
            .contains("previous verified runtime"));

        let unavailable = finish_install_state(
            BrowserInstallState::default(),
            Err("verification failed".into()),
        );
        assert_eq!(unavailable.status, BrowserInstallStatus::Error);
    }
}
