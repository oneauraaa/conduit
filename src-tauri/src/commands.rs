//! Tauri commands — the surface the webviews call.

use tauri::{AppHandle, Emitter, Manager, State, Wry};

use crate::agents::{self, AgentTarget};
use crate::mac::permissions;
use crate::mcp::{catalog::ToolDef, server};
use crate::state::{
    AccessMode, ControlState, Decision, PermissionState, ServerState, Settings, Shared, ToolsAccess,
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
pub fn get_tool_catalog() -> Vec<ToolDef> {
    crate::state::catalog_for_ui()
}

#[tauri::command]
pub fn set_default_access(state: State<'_, Shared>, mode: AccessMode) -> Settings {
    state.update_settings(|s| s.default_access = mode)
}

#[tauri::command]
pub fn set_tools_access(state: State<'_, Shared>, access: ToolsAccess) -> Settings {
    state.update_settings(|s| s.tools_access = access)
}

#[tauri::command]
pub fn set_tool_enabled(state: State<'_, Shared>, tool: String, enabled: bool) -> Settings {
    state.update_settings(|s| {
        s.tool_toggles.insert(tool, enabled);
    })
}

/* ── permissions ── */

#[tauri::command]
pub fn get_permissions() -> PermissionState {
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
#[tauri::command]
pub fn open_permission_settings(app: AppHandle<Wry>, which: String) {
    // Ask for the system prompt first — on a first run that's the nicer flow,
    // and it also registers conduit in the list so the pane isn't empty.
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
    let _ = app.emit("permissions:changed", permissions::snapshot());
}

/* ── control session ── */

#[tauri::command]
pub fn get_control_state(state: State<'_, Shared>) -> ControlState {
    state.control()
}

#[tauri::command]
pub fn set_session_mode(state: State<'_, Shared>, mode: AccessMode) -> ControlState {
    state.update_control(|c| c.mode = mode)
}

#[tauri::command]
pub fn stop_control(state: State<'_, Shared>) {
    state.abort();
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

#[tauri::command]
pub fn install_agent(
    app: AppHandle<Wry>,
    state: State<'_, Shared>,
    id: String,
) -> Result<AgentTarget, String> {
    let port = state.settings().port;
    let target = agents::install(&id, port)?;
    let _ = app.emit("agents:changed", agents::list(port));
    Ok(target)
}

#[tauri::command]
pub fn uninstall_agent(
    app: AppHandle<Wry>,
    state: State<'_, Shared>,
    id: String,
) -> Result<AgentTarget, String> {
    let port = state.settings().port;
    let target = agents::uninstall(&id, port)?;
    let _ = app.emit("agents:changed", agents::list(port));
    Ok(target)
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

/* ── window chrome ── */

/// The `x` button. Deliberately not a quit: conduit *is* the MCP server, so
/// quitting would silently revoke every agent's access. Quit lives in the tray.
#[tauri::command]
pub fn hide_to_tray(app: AppHandle<Wry>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}
