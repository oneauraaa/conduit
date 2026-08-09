//! Shared application state: settings, server lifecycle, and the live control
//! session. Everything the MCP tools and the UI both need lives here.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::mcp::catalog;

pub const DEFAULT_PORT: u16 = 6767;

/// The sharing listener sits on the next port up. It is a separate server from
/// the local one on purpose — see `crate::tailscale` for why.
pub fn remote_port(local: u16) -> u16 {
    local.saturating_add(1)
}

/* ── settings ───────────────────────────────────────────────── */

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccessMode {
    /// Every action waits for an explicit approval.
    Manual,
    /// Reads run freely; anything flagged risky asks first.
    Auto,
    /// Nothing asks.
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolsAccess {
    /// Every tool in the catalog is available.
    All,
    /// Per-tool switches decide.
    Custom,
    /// Nothing is available, regardless of the per-tool switches.
    Off,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub default_access: AccessMode,
    pub tools_access: ToolsAccess,
    /// Only consulted when `tools_access == Custom`. Missing means enabled.
    pub tool_toggles: HashMap<String, bool>,
    pub port: u16,
    /// Whether the endpoint is published to the internet via Tailscale Funnel.
    pub remote_enabled: bool,
    /// Secret path segment guarding the public endpoint. Generated on first
    /// run, never empty — the public URL is only as private as this string.
    pub remote_token: String,
    /// Whether browser-based clients may reach the endpoint at all.
    ///
    /// Off by default, and deliberately so: this endpoint can drive the whole
    /// machine and has no authentication, so allowing arbitrary web pages to
    /// call it would be a drive-by RCE. See `mcp/cors.rs`.
    #[serde(default)]
    pub cors_enabled: bool,
    /// Exact browser origins allowed when [`Settings::cors_enabled`] is on,
    /// e.g. `http://127.0.0.1:8080`. Nothing is implied — an empty list allows
    /// nothing even when the toggle is on.
    #[serde(default)]
    pub cors_origins: Vec<String>,
    /// Launch conduit when the user logs in.
    ///
    /// The server comes up with the app either way, so this is really "have the
    /// endpoint available without remembering to start it".
    #[serde(default)]
    pub start_on_login: bool,
    /// When launched at login, go straight to the tray instead of showing the
    /// window. Only meaningful alongside [`Settings::start_on_login`] — a
    /// manual launch always shows the window, because a double-click that
    /// appears to do nothing is worse than a window you have to dismiss.
    #[serde(default)]
    pub start_hidden: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            // Enabling a tool in the Tools tab is the grant. Asking again at
            // call time was friction on top of a decision the user already
            // made, so sessions start unrestricted; the pill can tighten a live
            // session to auto or manual at any point.
            default_access: AccessMode::Full,
            tools_access: ToolsAccess::All,
            tool_toggles: HashMap::new(),
            port: DEFAULT_PORT,
            remote_enabled: false,
            remote_token: crate::tailscale::generate_token(),
            cors_enabled: false,
            cors_origins: Vec::new(),
            start_on_login: false,
            start_hidden: false,
        }
    }
}

impl Settings {
    /// Whether `tool` may run right now, ignoring the approval question.
    pub fn tool_enabled(&self, tool: &str) -> bool {
        match self.tools_access {
            ToolsAccess::All => true,
            ToolsAccess::Off => false,
            ToolsAccess::Custom => *self.tool_toggles.get(tool).unwrap_or(&true),
        }
    }
}

/* ── server ─────────────────────────────────────────────────── */

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerStatus {
    Stopped,
    Starting,
    Running,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerState {
    pub status: ServerStatus,
    pub port: u16,
    /// Unix millis, or None while stopped.
    pub started_at: Option<u64>,
    pub last_error: Option<String>,
}

/* ── control session ────────────────────────────────────────── */

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ControlPhase {
    Idle,
    Active,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlState {
    pub phase: ControlPhase,
    pub agent: Option<String>,
    pub mode: AccessMode,
    pub action: Option<String>,
    /// A panic stop is latched: agents are refused until the user hands control
    /// back. Surfaced so the UI can offer that, because otherwise the only way
    /// out is restarting the app — which is exactly the bug this field exists
    /// to fix.
    pub stopped: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingApproval {
    pub id: String,
    pub tool: String,
    pub summary: String,
    pub detail: Option<String>,
    pub agent: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    /// Allow just this call.
    Allow,
    /// Allow this tool for the rest of the session.
    Session,
    Deny,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallEvent {
    pub id: String,
    pub tool: String,
    pub client: Option<String>,
    pub at: u64,
    pub outcome: &'static str,
    pub detail: Option<String>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorEvent {
    pub x: f64,
    pub y: f64,
    pub display: usize,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PulseEvent {
    pub kind: &'static str,
    pub display: usize,
}

/* ── readiness ──────────────────────────────────────────────── */

/// What the machine needs from the user before conduit can do its job.
///
/// A tagged union rather than a lowest-common-denominator struct, because the
/// two platforms genuinely differ. macOS withholds two capabilities behind TCC
/// grants. Windows grants everything up front but has conditions that silently
/// change what works — reporting those as two always-`true` booleans named
/// after macOS concepts would be a lie in a place the user goes *specifically*
/// to find out why something isn't working.
/// `rename_all_fields`, not `rename_all`. On an enum, `rename_all` renames the
/// *variants* — it does nothing to the fields of a struct variant, which then
/// serialize as `capture_supported` while the TypeScript reads
/// `captureSupported`. The mismatch is invisible in Rust and reads as `false`
/// in the UI, so the Server tab confidently reported broken capture and broken
/// DPI awareness on a machine where both were fine. `elevated` hid it for a
/// while by being a single word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "platform", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Readiness {
    #[serde(rename = "macos")]
    MacOS {
        accessibility: bool,
        screen_recording: bool,
    },
    #[serde(rename = "windows")]
    Windows {
        /// conduit is running as administrator. Without it, UIPI silently
        /// blocks synthetic input into — and accessibility reads of — any
        /// window belonging to an elevated process.
        elevated: bool,
        /// An elevated window is in the foreground right now: the moment the
        /// above actually bites.
        elevated_foreground: bool,
        /// Per-monitor DPI aware v2. Everything conduit reports on Windows is
        /// in physical pixels, which is only coherent if this holds.
        dpi_aware: bool,
        /// Windows.Graphics.Capture is present (Windows 10 1903+).
        capture_supported: bool,
        /// The capture session can suppress the yellow border (Win 11 22000+).
        /// When false, screenshots flash a yellow frame around the display.
        borderless_capture: bool,
    },
}

/* ── the app state ──────────────────────────────────────────── */

/// Everything mutable, behind one handle that is cheap to clone into the MCP
/// task, the Tauri commands and the tray.
pub struct AppState {
    pub app: AppHandle,
    settings: RwLock<Settings>,
    server: RwLock<ServerState>,
    control: RwLock<ControlState>,
    /// Cancels the running axum task. None while stopped.
    pub server_cancel: RwLock<Option<CancellationToken>>,
    /// Cancels the public sharing listener. None while sharing is off.
    pub remote_cancel: RwLock<Option<CancellationToken>>,
    /// Tools the user allowed for the remainder of this control session.
    session_allows: RwLock<Vec<String>>,
    /// In-flight approval requests, keyed by id.
    approvals: RwLock<HashMap<String, oneshot::Sender<Decision>>>,
    /// Set while a panic-stop is in effect; every tool short-circuits.
    pub aborted: RwLock<bool>,
    /// Unix millis of the last tool call, used to retire an idle session.
    last_activity: AtomicU64,
    seq: AtomicU64,
}

/// How long an agent can go without calling a tool before conduit decides the
/// session is over and takes the glow down. Long enough to survive a model
/// thinking between steps, short enough that a forgotten session doesn't leave
/// the screen glowing.
pub const IDLE_TIMEOUT_MS: u64 = 12_000;

pub type Shared = Arc<AppState>;

impl AppState {
    pub fn new(app: AppHandle, settings: Settings) -> Self {
        let port = settings.port;
        let mode = settings.default_access;
        Self {
            app,
            settings: RwLock::new(settings),
            server: RwLock::new(ServerState {
                status: ServerStatus::Stopped,
                port,
                started_at: None,
                last_error: None,
            }),
            control: RwLock::new(ControlState {
                phase: ControlPhase::Idle,
                agent: None,
                mode,
                action: None,
                stopped: false,
            }),
            server_cancel: RwLock::new(None),
            remote_cancel: RwLock::new(None),
            session_allows: RwLock::new(Vec::new()),
            approvals: RwLock::new(HashMap::new()),
            aborted: RwLock::new(false),
            last_activity: AtomicU64::new(0),
            seq: AtomicU64::new(0),
        }
    }

    /// Records that an agent just did something. Drives the idle watchdog.
    pub fn touch(&self) {
        self.last_activity.store(now_millis(), Ordering::Relaxed);
    }

    /// True when a session is active but has gone quiet past the timeout.
    pub fn is_idle(&self) -> bool {
        if self.control.read().phase != ControlPhase::Active {
            return false;
        }
        let last = self.last_activity.load(Ordering::Relaxed);
        last > 0 && now_millis().saturating_sub(last) > IDLE_TIMEOUT_MS
    }

    pub fn next_id(&self, prefix: &str) -> String {
        format!("{prefix}-{}", self.seq.fetch_add(1, Ordering::Relaxed))
    }

    /* ── settings ── */

    pub fn settings(&self) -> Settings {
        self.settings.read().clone()
    }

    /// Mutates settings, persists them, and notifies every window.
    pub fn update_settings(&self, f: impl FnOnce(&mut Settings)) -> Settings {
        let next = {
            let mut s = self.settings.write();
            f(&mut s);
            s.clone()
        };
        crate::store::save(&self.app, &next);
        let _ = self.app.emit("settings:changed", &next);
        next
    }

    /* ── server ── */

    pub fn server(&self) -> ServerState {
        self.server.read().clone()
    }

    pub fn update_server(&self, f: impl FnOnce(&mut ServerState)) -> ServerState {
        let next = {
            let mut s = self.server.write();
            f(&mut s);
            s.clone()
        };
        crate::tray::sync_label(matches!(next.status, ServerStatus::Running));
        let _ = self.app.emit("server:state", &next);
        next
    }

    /* ── control ── */

    pub fn control(&self) -> ControlState {
        self.control.read().clone()
    }

    pub fn update_control(&self, f: impl FnOnce(&mut ControlState)) -> ControlState {
        let next = {
            let mut c = self.control.write();
            f(&mut c);
            c.clone()
        };
        let _ = self.app.emit("control:state", &next);
        next
    }

    /// Marks an agent as driving and shows the overlay + pill. Idempotent, so
    /// every tool call can call it without thrashing the windows.
    pub fn begin_control(&self, agent: Option<String>, action: &str) {
        let was_idle = self.control.read().phase == ControlPhase::Idle;
        if was_idle {
            // Note: this does *not* clear `aborted`. The gate refuses a
            // latched stop before it ever gets here, and clearing it silently
            // would let an agent shrug off the panic button by simply
            // retrying. Only `resume` unlatches.
            self.session_allows.write().clear();
            let mode = self.settings().default_access;
            let next = self.update_control(|c| {
                c.phase = ControlPhase::Active;
                c.agent = agent.clone();
                c.mode = mode;
                c.action = Some(action.to_string());
            });
            crate::chrome::show_control_chrome(&self.app, next);
        } else {
            self.update_control(|c| {
                if c.agent.is_none() {
                    c.agent = agent.clone();
                }
                c.action = Some(action.to_string());
            });
        }
    }

    /// Hides the overlay + pill and forgets session grants.
    pub fn end_control(&self) {
        if self.control.read().phase == ControlPhase::Idle {
            return;
        }
        self.session_allows.write().clear();
        self.update_control(|c| {
            c.phase = ControlPhase::Idle;
            c.agent = None;
            c.action = None;
        });
        let _ = self.app.emit("control:approval", Option::<PendingApproval>::None);
        crate::chrome::hide_control_chrome(&self.app);
    }

    /// Panic stop: deny every in-flight approval and drop control immediately.
    /// Drops control and latches the stop.
    ///
    /// The latch is deliberate: a panic stop should not be undone by whatever
    /// the agent sends a hundred milliseconds later. [`AppState::resume`] is
    /// the way back, and the UI offers it — before that existed, the only way
    /// to clear this was restarting the app.
    pub fn abort(&self) {
        *self.aborted.write() = true;
        let pending: Vec<_> = self.approvals.write().drain().collect();
        for (_, tx) in pending {
            let _ = tx.send(Decision::Deny);
        }
        self.end_control();
        // After `end_control`, so it survives that update and reaches the UI.
        self.update_control(|c| c.stopped = true);
    }

    /// Hands control back, so agents may start a new session.
    pub fn resume(&self) {
        *self.aborted.write() = false;
        self.session_allows.write().clear();
        self.update_control(|c| c.stopped = false);
    }

    pub fn is_aborted(&self) -> bool {
        *self.aborted.read()
    }

    /* ── approvals ── */

    pub fn session_allowed(&self, tool: &str) -> bool {
        self.session_allows.read().iter().any(|t| t == tool)
    }

    pub fn allow_for_session(&self, tool: &str) {
        let mut a = self.session_allows.write();
        if !a.iter().any(|t| t == tool) {
            a.push(tool.to_string());
        }
    }

    /// Registers an approval request and hands back the receiver to await.
    pub fn open_approval(&self, req: PendingApproval) -> oneshot::Receiver<Decision> {
        let (tx, rx) = oneshot::channel();
        self.approvals.write().insert(req.id.clone(), tx);
        let _ = self.app.emit("control:approval", Some(&req));
        rx
    }

    pub fn resolve_approval(&self, id: &str, decision: Decision) {
        if let Some(tx) = self.approvals.write().remove(id) {
            let _ = tx.send(decision);
        }
        let _ = self.app.emit("control:approval", Option::<PendingApproval>::None);
    }

    /// Drops a request that timed out, so the map doesn't grow unbounded.
    pub fn close_approval(&self, id: &str) {
        self.approvals.write().remove(id);
        let _ = self.app.emit("control:approval", Option::<PendingApproval>::None);
    }

    /* ── logging ── */

    pub fn log_call(
        &self,
        tool: &str,
        client: Option<String>,
        outcome: &'static str,
        detail: Option<String>,
        duration_ms: Option<u64>,
    ) {
        let event = ToolCallEvent {
            id: self.next_id("call"),
            tool: tool.to_string(),
            client,
            at: now_millis(),
            outcome,
            detail,
            duration_ms,
        };
        let _ = self.app.emit("server:tool-call", &event);
    }
}

pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The tool catalog, shaped for the UI.
pub fn catalog_for_ui() -> Vec<catalog::ToolDef> {
    catalog::CATALOG.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The UI reads these keys by name. A mismatch is invisible on the Rust
    /// side and surfaces as `undefined` in TypeScript, which is falsy — so the
    /// Server tab silently renders every boolean's failure state and tells the
    /// user their machine is broken.
    ///
    /// This is not hypothetical: `rename_all` on an enum renames *variants*,
    /// not the fields of struct variants, and that shipped a card claiming
    /// Windows.Graphics.Capture was missing on a machine where capture worked.
    #[test]
    fn readiness_serializes_the_keys_the_ui_reads() {
        let win = Readiness::Windows {
            elevated: false,
            elevated_foreground: true,
            dpi_aware: true,
            capture_supported: true,
            borderless_capture: false,
        };
        let json = serde_json::to_value(win).unwrap();

        assert_eq!(json["platform"], "windows");
        for key in [
            "elevated",
            "elevatedForeground",
            "dpiAware",
            "captureSupported",
            "borderlessCapture",
        ] {
            assert!(
                json.get(key).is_some(),
                "missing `{key}` — the UI reads that name; got {json}"
            );
        }
        // Values must survive too, not just the keys.
        assert_eq!(json["dpiAware"], true);
        assert_eq!(json["captureSupported"], true);
        assert_eq!(json["borderlessCapture"], false);

        let mac = Readiness::MacOS {
            accessibility: true,
            screen_recording: false,
        };
        let json = serde_json::to_value(mac).unwrap();
        assert_eq!(json["platform"], "macos");
        assert!(json.get("screenRecording").is_some(), "got {json}");
    }
}
