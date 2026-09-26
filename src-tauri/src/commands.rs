//! Tauri commands — the surface the webviews call.

use tauri::ipc::{Channel, InvokeBody, InvokeResponseBody};
use tauri::{AppHandle, Emitter, Manager, State, Wry};

use crate::agents::{self, AgentTarget};
use crate::browser::model::{BrowserMode, BrowserRestartStrategy, BrowserState};
use crate::mcp::{catalog::ToolDef, server};
use crate::platform::permissions;
use crate::sandbox::model::{NewSandbox, SandboxPatch, SandboxesState};
use crate::state::{
    AccessMode, BrowserPermissionCategory, BrowserPermissionMode, ControlState, Decision,
    PendingApproval, Readiness, ServerState, Settings, Shared, ToolsAccess,
};

/* ── server ── */

#[tauri::command]
pub fn get_server_state(state: State<'_, Shared>) -> ServerState {
    state.server()
}

#[tauri::command]
pub async fn start_server(state: State<'_, Shared>) -> Result<ServerState, ()> {
    let shared = state.inner().clone();
    server::start(shared.clone()).await;
    Ok(shared.server())
}

#[tauri::command]
pub async fn stop_server(state: State<'_, Shared>) -> Result<ServerState, ()> {
    let shared = state.inner().clone();
    server::stop(shared.clone()).await;
    Ok(shared.server())
}

#[tauri::command]
pub async fn restart_server(state: State<'_, Shared>) -> Result<ServerState, ()> {
    let shared = state.inner().clone();
    server::restart(shared.clone()).await;
    Ok(shared.server())
}

#[tauri::command]
pub async fn set_port(state: State<'_, Shared>, port: u16) -> Result<ServerState, String> {
    if port < 1024 {
        return Err("ports below 1024 need root; pick something higher".into());
    }
    let shared = state.inner().clone();
    shared.update_settings(|s| s.port = port);

    // A port change only means anything after a rebind.
    if matches!(shared.server().status, crate::state::ServerStatus::Running) {
        server::restart(shared.clone()).await;
    } else {
        shared.update_server(|s| s.port = port);
    }
    Ok(shared.server())
}

/* ── settings & tools ── */

#[tauri::command]
pub fn get_settings(state: State<'_, Shared>) -> Settings {
    state.settings()
}

#[tauri::command]
pub fn get_tool_catalog(state: State<'_, Shared>) -> Vec<ToolDef> {
    crate::state::catalog_for_ui(state.browser.ready())
}

#[tauri::command]
pub fn set_default_access(state: State<'_, Shared>, mode: AccessMode) -> Settings {
    state.update_settings(|s| s.default_access = mode)
}

#[tauri::command]
pub fn set_tools_access(state: State<'_, Shared>, access: ToolsAccess) -> Settings {
    state.update_settings(|s| s.set_tools_access(access))
}

#[tauri::command]
pub fn set_tool_enabled(state: State<'_, Shared>, tool: String, enabled: bool) -> Settings {
    state.update_settings(|s| s.set_tool_enabled(tool, enabled))
}

/* ── built-in browser ── */

#[tauri::command]
pub fn get_browser_state(state: State<'_, Shared>) -> BrowserState {
    state.browser.snapshot()
}

#[tauri::command]
pub async fn refresh_browser_install(state: State<'_, Shared>) -> Result<BrowserState, String> {
    state.browser.clone().refresh_install_metadata().await
}

#[tauri::command]
pub async fn install_browser(state: State<'_, Shared>) -> Result<BrowserState, String> {
    let browser = state.browser.clone();
    browser.clone().install().await?;
    if state.settings().browser_auto_start {
        return browser.start(true, None, None).await;
    }
    Ok(browser.snapshot())
}

#[tauri::command]
pub async fn uninstall_browser(state: State<'_, Shared>) -> Result<BrowserState, String> {
    state.browser.uninstall().await
}

#[tauri::command]
pub fn cancel_browser_install(state: State<'_, Shared>) {
    state.browser.cancel_install();
}

#[tauri::command]
pub async fn start_browser(state: State<'_, Shared>) -> Result<BrowserState, String> {
    let browser = state.browser.clone();
    browser.start(true, None, None).await
}

#[tauri::command]
pub async fn stop_browser(state: State<'_, Shared>) -> Result<BrowserState, String> {
    let browser = state.browser.clone();
    Ok(browser.stop(true).await)
}

#[tauri::command]
pub async fn set_browser_mode(
    state: State<'_, Shared>,
    mode: BrowserMode,
    restart: Option<BrowserRestartStrategy>,
) -> Result<BrowserState, String> {
    let browser = state.browser.clone();
    browser.set_mode(mode, restart).await
}

#[tauri::command]
pub async fn select_browser_profile(
    state: State<'_, Shared>,
    profile_id: String,
    restart: Option<BrowserRestartStrategy>,
) -> Result<BrowserState, String> {
    let browser = state.browser.clone();
    browser.select_profile(profile_id, restart).await
}

#[tauri::command]
pub fn create_browser_profile(
    state: State<'_, Shared>,
    name: String,
) -> Result<BrowserState, String> {
    state.browser.create_profile(name)
}

#[tauri::command]
pub fn rename_browser_profile(
    state: State<'_, Shared>,
    id: String,
    name: String,
) -> Result<BrowserState, String> {
    state.browser.rename_profile(id, name)
}

#[tauri::command]
pub fn delete_browser_profile(
    state: State<'_, Shared>,
    id: String,
) -> Result<BrowserState, String> {
    state.browser.delete_profile(id)
}

#[tauri::command]
pub fn set_browser_permission(
    state: State<'_, Shared>,
    category: BrowserPermissionCategory,
    mode: BrowserPermissionMode,
) -> Settings {
    state.update_settings(|settings| match category {
        BrowserPermissionCategory::OpenWebsites => {
            settings.browser_permissions.open_websites = mode
        }
        BrowserPermissionCategory::ReadHistory => settings.browser_permissions.read_history = mode,
        BrowserPermissionCategory::DownloadFiles => {
            settings.browser_permissions.download_files = mode
        }
        BrowserPermissionCategory::UploadFiles => settings.browser_permissions.upload_files = mode,
    })
}

#[tauri::command]
pub async fn set_browser_tab_visible(
    state: State<'_, Shared>,
    visible: bool,
) -> Result<(), String> {
    let browser = state.browser.clone();
    browser.set_tab_visible(visible).await
}

#[tauri::command]
pub fn open_browser_downloads(state: State<'_, Shared>) -> Result<(), String> {
    let path = state.browser.download_directory();
    std::fs::create_dir_all(&path)
        .map_err(|e| format!("could not create browser downloads folder: {e}"))?;
    tauri_plugin_opener::open_path(path, None::<&str>)
        .map_err(|e| format!("could not open browser downloads: {e}"))
}

/// Turns browser access on or off. Takes effect immediately — the origin
/// guard reads settings per request, so there is no server restart.
#[tauri::command]
pub fn set_cors_enabled(state: State<'_, Shared>, enabled: bool) -> Settings {
    state.update_settings(|s| s.cors_enabled = enabled)
}

/// Replaces the allowlist. Origins are stored exactly as the user typed them,
/// minus surrounding whitespace and any trailing slash; comparison is
/// normalized in `mcp::cors`.
#[tauri::command]
pub fn set_cors_origins(state: State<'_, Shared>, origins: Vec<String>) -> Settings {
    let cleaned: Vec<String> = origins
        .into_iter()
        .map(|o| o.trim().trim_end_matches('/').to_string())
        .filter(|o| !o.is_empty())
        .collect();
    state.update_settings(|s| s.cors_origins = cleaned)
}

/* ── startup ── */

/// Registers or removes conduit's launch-at-login entry.
///
/// The setting and the OS entry are kept in step here rather than only at
/// startup, so a failure to write the registry key or the LaunchAgent surfaces
/// as an error the user sees instead of a switch that lies.
#[tauri::command]
pub fn set_start_on_login(
    app: AppHandle<Wry>,
    state: State<'_, Shared>,
    enabled: bool,
) -> Result<Settings, String> {
    apply_autostart(&app, enabled)?;
    Ok(state.update_settings(|s| s.start_on_login = enabled))
}

#[tauri::command]
pub fn set_start_hidden(state: State<'_, Shared>, hidden: bool) -> Settings {
    state.update_settings(|s| s.start_hidden = hidden)
}

#[tauri::command]
pub fn set_browser_auto_start(state: State<'_, Shared>, enabled: bool) -> Settings {
    state.update_settings(|s| s.browser_auto_start = enabled)
}

#[tauri::command]
pub fn set_outline_desktop(state: State<'_, Shared>, enabled: bool) -> Settings {
    state.update_settings(|s| s.outline_desktop = enabled)
}

#[tauri::command]
pub fn set_outline_browser(state: State<'_, Shared>, enabled: bool) -> Settings {
    state.update_settings(|s| s.outline_browser = enabled)
}

#[tauri::command]
pub fn set_outline_background(state: State<'_, Shared>, enabled: bool) -> Settings {
    state.update_settings(|s| s.outline_background = enabled)
}

#[tauri::command]
pub fn set_pill_desktop(state: State<'_, Shared>, enabled: bool) -> Settings {
    state.update_settings(|s| s.pill_desktop = enabled)
}

#[tauri::command]
pub fn set_pill_browser(state: State<'_, Shared>, enabled: bool) -> Settings {
    state.update_settings(|s| s.pill_browser = enabled)
}

#[tauri::command]
pub fn set_pill_background(state: State<'_, Shared>, enabled: bool) -> Settings {
    state.update_settings(|s| s.pill_background = enabled)
}

/// Points the OS's login entry at the current state of the setting.
///
/// ## Why a development build refuses
///
/// The login entry is `current_exe()`. In a `cargo run`/`cargo build` binary
/// that is `target/debug/conduit`, which is a perfectly good executable — and
/// a completely broken app to start at login, because a development build does
/// not carry the frontend. It loads it from the Vite dev server, which is not
/// running at login and will not be. What the user gets is conduit's real
/// window frame filled with WebKit's *"Could not connect to localhost:
/// Connection refused"*, with no hint that the cause is which binary got
/// registered — and because closing hides to the tray, it comes back.
///
/// `is_dev()` is exactly the right question to ask: it is
/// `!cfg!(feature = "custom-protocol")`, the same condition
/// `generate_context!` uses to decide between the embedded frontend and the dev
/// URL. True here means, precisely, "these webviews need a dev server".
///
/// Any entry an earlier development run already wrote is removed on the way
/// out. It can only point at a dev binary, so it can only produce that dead
/// window; leaving it in place to be polite would be leaving the bug. The
/// *setting* is untouched — a release build re-asserts it at startup and
/// registers the real binary.
pub fn apply_autostart(app: &AppHandle<Wry>, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;

    let manager = app.autolaunch();

    if enabled && tauri::is_dev() {
        let _ = manager.disable();
        return Err(
            "this is a development build, so it can't start at login — the entry would point at \
             target/debug/conduit, whose window is served by the dev server and would come up \
             empty. build a release binary (pnpm run package) and turn this on there."
                .into(),
        );
    }

    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    result.map_err(|e| format!("could not change the launch-at-login entry: {e}"))?;

    // Hyprland never reads the XDG entry above; see `hyprland::set_autostart`.
    // `available` is false off Linux, so this is a no-op everywhere else.
    if crate::hyprland::available() {
        crate::hyprland::set_autostart(enabled)?;
    }
    Ok(())
}

/* ── readiness ── */

/// Every command below stays registered on both platforms, with the bodies
/// cfg'd rather than the `generate_handler!` entries. Gating individual entries
/// in that macro is possible and the errors when you get it wrong are terrible.

#[tauri::command]
pub fn get_readiness() -> Readiness {
    permissions::snapshot()
}

#[tauri::command]
pub fn request_accessibility() {
    permissions::prompt_accessibility();
}

#[tauri::command]
pub fn request_screen_recording() {
    permissions::prompt_screen_recording();
}

/// Opens the relevant System Settings pane. macOS only shows its own consent
/// prompt once per bundle, so this is the reliable path on every later attempt.
///
/// A no-op on Windows, which withholds nothing behind a settings pane.
#[tauri::command]
pub fn open_permission_settings(app: AppHandle<Wry>, which: String) {
    #[cfg(target_os = "macos")]
    {
        // Ask for the system prompt first — on a first run that's the nicer
        // flow, and it also registers conduit in the list so the pane isn't
        // empty.
        match which.as_str() {
            "accessibility" => {
                permissions::prompt_accessibility();
            }
            _ => {
                permissions::prompt_screen_recording();
            }
        }

        let pane = match which.as_str() {
            "accessibility" => "Privacy_Accessibility",
            _ => "Privacy_ScreenCapture",
        };
        let url = format!("x-apple.systempreferences:com.apple.preference.security?{pane}");
        let _ = tauri_plugin_opener::open_url(url, None::<&str>);
    }

    // On Linux there is no settings pane to open: the one grant conduit needs
    // is the portal's, and the only way to ask again is to ask again. `which`
    // is ignored because input and capture come from the same session — see
    // `platform/linux/permissions.rs`.
    #[cfg(target_os = "linux")]
    {
        let _ = which;
        permissions::prompt_screen_recording();
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = which;

    let _ = app.emit("readiness:changed", permissions::snapshot());
}

/// Restarts conduit as administrator so it can drive elevated windows.
///
/// Windows only. This tears the current process down on success, so it is the
/// one command here that does not return to its caller.
#[tauri::command]
pub fn relaunch_elevated(app: AppHandle<Wry>) -> Result<(), String> {
    permissions::relaunch_elevated(&app)
}

/* ── control session ── */

#[tauri::command]
pub fn get_control_state(state: State<'_, Shared>) -> ControlState {
    state.control()
}

#[tauri::command]
pub fn get_pending_approval(state: State<'_, Shared>) -> Option<PendingApproval> {
    state.pending_approval()
}

#[tauri::command]
pub fn set_session_mode(state: State<'_, Shared>, mode: AccessMode) -> ControlState {
    state.update_control(|c| c.mode = mode)
}

#[tauri::command]
pub fn stop_control(state: State<'_, Shared>) {
    state.abort();
}

/// Hands control back after a panic stop.
///
/// The stop is latched on purpose, so without this the endpoint stays closed
/// for the life of the process and the only remedy is restarting conduit.
#[tauri::command]
pub fn resume_control(state: State<'_, Shared>) -> ControlState {
    state.resume();
    state.control()
}

#[tauri::command]
pub fn resolve_approval(state: State<'_, Shared>, id: String, decision: Decision) {
    state.resolve_approval(&id, decision);
}

/* ── agents ── */

#[tauri::command]
pub fn list_agents(state: State<'_, Shared>) -> Vec<AgentTarget> {
    agents::list(state.settings().port)
}

/// `target` is `"host"` (or absent) for this computer, else a sandbox id.
#[tauri::command]
pub fn install_agent(
    app: AppHandle<Wry>,
    state: State<'_, Shared>,
    id: String,
    target: Option<String>,
) -> Result<AgentTarget, String> {
    let port = state.settings().port;
    let target = agents::Target::parse(target.as_deref())?;
    if let agents::Target::Sandbox(sandbox) = &target {
        if !state.sandboxes.exists(sandbox) {
            return Err(format!("there is no sandbox called {sandbox}"));
        }
    }
    let row = agents::install(&id, port, &target)?;
    let _ = app.emit("agents:changed", agents::list(port));
    Ok(row)
}

#[tauri::command]
pub fn uninstall_agent(
    app: AppHandle<Wry>,
    state: State<'_, Shared>,
    id: String,
    target: Option<String>,
) -> Result<AgentTarget, String> {
    let port = state.settings().port;
    let target = agents::Target::parse(target.as_deref())?;
    let row = agents::uninstall(&id, port, &target)?;
    let _ = app.emit("agents:changed", agents::list(port));
    Ok(row)
}

/* ── sandboxes ── */

#[tauri::command]
pub fn get_sandbox_state(state: State<'_, Shared>) -> SandboxesState {
    state.sandboxes.snapshot()
}

#[tauri::command]
pub async fn refresh_docker(state: State<'_, Shared>) -> Result<SandboxesState, ()> {
    Ok(state.sandboxes.refresh_docker().await)
}

/// Creating a sandbox is asking for one, so it also starts — building the
/// desktop image first if this OS has never been used. Progress arrives on
/// `sandbox:state`; the command itself returns as soon as the entry exists.
#[tauri::command]
pub async fn create_sandbox(
    state: State<'_, Shared>,
    sandbox: NewSandbox,
) -> Result<SandboxesState, String> {
    let manager = state.sandboxes.clone();
    let spec = manager.create(sandbox)?;
    let starter = manager.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = starter.start(&spec.id).await {
            tracing::warn!(sandbox = %spec.id, %error, "the new sandbox did not start");
        }
    });
    Ok(manager.snapshot())
}

#[tauri::command]
pub async fn update_sandbox(
    state: State<'_, Shared>,
    id: String,
    patch: SandboxPatch,
) -> Result<SandboxesState, String> {
    let manager = state.sandboxes.clone();
    manager.update(&id, patch).await?;
    Ok(manager.snapshot())
}

/// Removes the sandbox, its container and disk, its MCP endpoint, and every
/// agent config entry that pointed at it — a dangling entry would only give
/// the agent a 404 to puzzle over.
#[tauri::command]
pub async fn delete_sandbox(
    app: AppHandle<Wry>,
    state: State<'_, Shared>,
    id: String,
) -> Result<SandboxesState, String> {
    let shared = state.inner().clone();
    shared.sandboxes.delete(&id).await?;
    if let Some(services) = shared.sandbox_services.read().clone() {
        services.remove(&id);
    }
    let port = shared.settings().port;
    if let Err(error) = agents::remove_everywhere(&agents::Target::Sandbox(id.clone())) {
        tracing::warn!(sandbox = %id, %error, "could not remove the sandbox from every agent");
    }
    let _ = app.emit("agents:changed", agents::list(port));
    Ok(shared.sandboxes.snapshot())
}

#[tauri::command]
pub async fn start_sandbox(state: State<'_, Shared>, id: String) -> Result<SandboxesState, String> {
    let manager = state.sandboxes.clone();
    manager.start(&id).await?;
    Ok(manager.snapshot())
}

#[tauri::command]
pub async fn stop_sandbox(state: State<'_, Shared>, id: String) -> Result<SandboxesState, String> {
    let manager = state.sandboxes.clone();
    manager.stop(&id).await?;
    Ok(manager.snapshot())
}

#[tauri::command]
pub fn cancel_sandbox_build(state: State<'_, Shared>, id: String) {
    state.sandboxes.cancel_build(&id);
}

#[tauri::command]
pub fn set_sandbox_stop_on_quit(
    state: State<'_, Shared>,
    stop: bool,
) -> Result<SandboxesState, String> {
    state.sandboxes.set_stop_on_quit(stop)?;
    Ok(state.sandboxes.snapshot())
}

/// The Sandbox tab's "stop agent": cancels whatever an agent is doing in this
/// sandbox and refuses its next calls until resumed.
#[tauri::command]
pub fn interrupt_sandbox_agent(state: State<'_, Shared>, id: String) -> SandboxesState {
    state.sandboxes.interrupt(&id);
    state.sandboxes.snapshot()
}

#[tauri::command]
pub fn resume_sandbox_agent(state: State<'_, Shared>, id: String) -> SandboxesState {
    state.sandboxes.resume(&id);
    state.sandboxes.snapshot()
}

/// Opens a live view of a running sandbox. VNC bytes stream to `on_data`; an
/// empty message means the stream ended. Returns the viewer's handle.
///
/// Async not for any await, but for where it runs: a sync command runs on the
/// main thread, which has no Tokio reactor to spawn the `docker exec` on.
#[tauri::command]
pub async fn open_sandbox_viewer(
    state: State<'_, Shared>,
    id: String,
    on_data: Channel<InvokeResponseBody>,
) -> Result<String, String> {
    let manager = &state.sandboxes;
    if !manager.is_running(&id) {
        return Err("the sandbox is not running".into());
    }
    let cli = manager.cli_for_viewer()?;
    manager.viewers.open(cli, &id, on_data)
}

/// Bytes from the viewer to the sandbox, as a raw body with the viewer's
/// handle in the `x-viewer` header — raw so a key press is not a JSON array.
#[tauri::command]
pub async fn sandbox_viewer_send(
    state: State<'_, Shared>,
    request: tauri::ipc::Request<'_>,
) -> Result<(), String> {
    let handle = request
        .headers()
        .get("x-viewer")
        .and_then(|v| v.to_str().ok())
        .ok_or("missing viewer handle")?
        .to_string();
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected raw bytes".into());
    };
    state.sandboxes.viewers.send(&handle, bytes).await
}

#[tauri::command]
pub fn close_sandbox_viewer(state: State<'_, Shared>, viewer: String) {
    state.sandboxes.viewers.close(&viewer);
}

/// Docker's own download page for this platform. A fixed URL — the webview
/// never gets to hand the opener an arbitrary one.
#[tauri::command]
pub fn open_docker_download() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let url = "https://docs.docker.com/engine/install/";
    #[cfg(not(target_os = "linux"))]
    let url = "https://www.docker.com/products/docker-desktop/";
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_sandbox_tab_visible(app: AppHandle<Wry>, visible: bool) -> Result<(), String> {
    crate::chrome::set_wide(&app, "sandbox", visible).await
}

/* ── tailscale sharing ── */

#[tauri::command]
pub fn get_tailscale_state(state: State<'_, Shared>) -> crate::tailscale::TailscaleState {
    let s = state.settings();
    crate::tailscale::state(crate::state::remote_port(s.port), &s.remote_token)
}

/// Publishes the endpoint: brings up the sharing listener, then points a
/// Tailscale funnel at it.
///
/// The listener comes up first on purpose — a funnel aimed at a dead port
/// would answer public requests with a connection error rather than 404, and
/// the failure would look like Tailscale's fault.
#[tauri::command]
pub async fn enable_remote(
    app: AppHandle<Wry>,
    state: State<'_, Shared>,
) -> Result<crate::tailscale::TailscaleState, String> {
    let shared = state.inner().clone();
    let settings = shared.settings();
    let port = crate::state::remote_port(settings.port);

    crate::mcp::server::start_remote(shared.clone()).await?;

    if let Err(e) = crate::tailscale::start_funnel(port) {
        // Don't leave a listener up that nothing can reach.
        crate::mcp::server::stop_remote(&shared);
        return Err(e);
    }

    shared.update_settings(|s| s.remote_enabled = true);
    let next = crate::tailscale::state(port, &settings.remote_token);
    let _ = app.emit("tailscale:state", &next);
    Ok(next)
}

/// After a Linux sudo prompt, grant this login user Tailscale operator access
/// once, then retry the same sharing flow. The password is never persisted.
#[tauri::command]
pub async fn enable_remote_with_password(
    app: AppHandle<Wry>,
    state: State<'_, Shared>,
    password: String,
) -> Result<crate::tailscale::TailscaleState, String> {
    #[cfg(target_os = "linux")]
    {
        tokio::task::spawn_blocking(move || crate::tailscale::grant_operator(password))
            .await
            .map_err(|e| format!("sudo worker failed: {e}"))??;
        enable_remote(app, state).await
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, state, password);
        Err("Tailscale's sudo setup is only available on Linux".into())
    }
}

#[tauri::command]
pub async fn disable_remote(
    app: AppHandle<Wry>,
    state: State<'_, Shared>,
) -> Result<crate::tailscale::TailscaleState, String> {
    let shared = state.inner().clone();
    let settings = shared.settings();
    let port = crate::state::remote_port(settings.port);

    let funnel = crate::tailscale::stop_funnel(port);
    crate::mcp::server::stop_remote(&shared);
    shared.update_settings(|s| s.remote_enabled = false);

    let mut next = crate::tailscale::state(port, &settings.remote_token);
    if let Err(e) = funnel {
        // The listener is down either way, so the endpoint is unreachable —
        // but say so rather than pretending the funnel cleaned up.
        next.error = Some(e);
    }
    let _ = app.emit("tailscale:state", &next);
    Ok(next)
}

/// Issues a new secret and restarts sharing on it, which instantly invalidates
/// the old public URL.
#[tauri::command]
pub async fn regenerate_remote_token(
    app: AppHandle<Wry>,
    state: State<'_, Shared>,
) -> Result<crate::tailscale::TailscaleState, String> {
    let shared = state.inner().clone();
    let was_sharing = shared.settings().remote_enabled;

    crate::mcp::server::stop_remote(&shared);
    let settings = shared.update_settings(|s| s.remote_token = crate::tailscale::generate_token());
    let port = crate::state::remote_port(settings.port);

    if was_sharing {
        crate::mcp::server::start_remote(shared.clone()).await?;
    }

    let next = crate::tailscale::state(port, &settings.remote_token);
    let _ = app.emit("tailscale:state", &next);
    Ok(next)
}

/* ── hyprland ── */

/// The user's own keyboard shortcuts, when this is a Hyprland session.
///
/// Off Hyprland this reports `available: false` rather than failing, so the
/// tab has something to render on every platform. The work is a `hyprctl` call
/// plus one config read, cheap enough to do per invocation — like
/// `get_readiness`, there is no cached copy to go stale.
#[tauri::command]
pub fn get_hyprland_state() -> crate::hyprland::HyprlandState {
    crate::hyprland::snapshot()
}

/* ── window chrome ── */

/// The `x` button. Deliberately not a quit: conduit *is* the MCP server, so
/// quitting would silently revoke every agent's access. Quit lives in the tray.
#[tauri::command]
pub fn hide_to_tray(app: AppHandle<Wry>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}
