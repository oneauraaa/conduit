//! Tauri commands — the surface the webviews call.

use tauri::{AppHandle, Emitter, Manager, State, Wry};

use crate::agents::{self, AgentTarget};
use crate::mcp::{catalog::ToolDef, server};
use crate::platform::permissions;
use crate::state::{
    AccessMode, ControlState, Decision, Readiness, ServerState, Settings, Shared, ToolsAccess,
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
    result.map_err(|e| format!("could not change the launch-at-login entry: {e}"))
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
