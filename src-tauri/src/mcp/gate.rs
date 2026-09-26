//! The permission gate.
//!
//! Every tool call goes through [`run`]. Nothing in `tools/` touches the
//! machine before this function has said yes, which keeps the whole security
//! story in one readable place:
//!
//!   1. panic stop in effect?      -> refuse
//!   2. Tools Access off?          -> refuse
//!   3. this tool switched off?    -> refuse
//!   4. mode says ask?             -> prompt the pill, await the answer
//!   5. run it, and log the outcome

use std::time::{Duration, Instant};

use rmcp::model::CallToolResult;
use rmcp::ErrorData as McpError;

use crate::mcp::catalog;
use crate::state::{
    AccessMode, BrowserPermissionCategory, BrowserPermissionMode, Decision, PendingApproval, Shared,
};

/// How long an approval sits on screen before it is treated as a refusal.
/// Erring toward denial matters more than erring toward convenience here.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(60);

pub struct CallCtx {
    pub tool: &'static str,
    /// Human-readable arguments, shown in the log and the approval card.
    pub detail: Option<String>,
    /// The connected agent's name, from the MCP `initialize` handshake.
    pub agent: Option<String>,
}

/// Runs `f` if policy allows. `f` is only invoked after every check passes.
pub async fn run<F, Fut>(state: &Shared, ctx: CallCtx, f: F) -> Result<CallToolResult, McpError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<CallToolResult, McpError>>,
{
    let risky = catalog::is_risky(ctx.tool);
    run_with_risk(state, ctx, risky, f).await
}

/// Runs a tool whose risk depends on its arguments. Browser tab listing and
/// selection are ordinary actions, while the same tool's `close` operation is
/// destructive and must prompt in Auto mode.
pub async fn run_with_risk<F, Fut>(
    state: &Shared,
    ctx: CallCtx,
    risky: bool,
    f: F,
) -> Result<CallToolResult, McpError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<CallToolResult, McpError>>,
{
    let started = Instant::now();
    let tool = ctx.tool;

    // 1. panic stop
    if state.is_aborted() {
        state.log_call(
            tool,
            ctx.agent.clone(),
            "blocked",
            Some("stopped by user".into()),
            None,
        );
        return Err(McpError::invalid_request(
            "conduit: the user stopped control. retrying will not help — they have to hand it \
             back from conduit's window before any tool works again. ask them to.",
            None,
        ));
    }

    // 2 & 3. the tools gate
    let settings = state.settings();
    if !settings.tool_enabled(tool) {
        let why = match settings.tools_access {
            crate::state::ToolsAccess::Off => {
                "conduit: all tools are switched off in the Tools tab."
            }
            _ => "conduit: this tool is switched off in the Tools tab.",
        };
        state.log_call(
            tool,
            ctx.agent.clone(),
            "blocked",
            Some("disabled".into()),
            None,
        );
        return Err(McpError::invalid_request(why, None));
    }

    // The session becomes "active" the moment an agent reaches for anything —
    // including reads. Watching the screen is exactly when the user most wants
    // to see the glow.
    state.begin_control(ctx.agent.clone(), catalog::action_label(tool), catalog::action_surface(tool));
    state.touch();
    let _active_call = state.track_active_call();

    // 4. approval
    let needs_approval = match state.control().mode {
        AccessMode::Full => false,
        AccessMode::Auto => risky,
        AccessMode::Manual => true,
    } && !state.session_allowed(tool);

    if needs_approval {
        let id = state.next_id("approval");
        let req = PendingApproval {
            id: id.clone(),
            tool: tool.to_string(),
            summary: catalog::approval_summary(tool),
            detail: ctx.detail.clone(),
            agent: ctx.agent.clone(),
            category: None,
        };
        let rx = state.open_approval(req);

        let decision = match tokio::time::timeout(APPROVAL_TIMEOUT, rx).await {
            Ok(Ok(d)) => d,
            // Timed out, or the sender was dropped (panic stop). Both are refusals.
            _ => {
                state.close_approval(&id);
                Decision::Deny
            }
        };

        match decision {
            Decision::Deny => {
                state.log_call(tool, ctx.agent.clone(), "denied", ctx.detail.clone(), None);
                return Err(McpError::invalid_request(
                    "conduit: the user denied this action.",
                    None,
                ));
            }
            Decision::Session => state.allow_for_session(tool),
            Decision::Allow => {}
        }
    }

    // A panic stop can land while an approval was on screen.
    if state.is_aborted() {
        state.log_call(
            tool,
            ctx.agent.clone(),
            "blocked",
            Some("stopped by user".into()),
            None,
        );
        return Err(McpError::invalid_request(
            "conduit: control was stopped by the user.",
            None,
        ));
    }

    // 5. go
    let result = f().await;
    let ms = started.elapsed().as_millis() as u64;
    // Touch again on the way out: a long call (a 10s wait, a slow shell
    // command) must not let the watchdog retire the session mid-flight.
    state.touch();

    match &result {
        Ok(_) => state.log_call(tool, ctx.agent, "ok", ctx.detail, Some(ms)),
        Err(e) => state.log_call(
            tool,
            ctx.agent,
            "error",
            Some(e.message.to_string()),
            Some(ms),
        ),
    }

    result
}

/// Runs a direct browser action whose data boundary is controlled by one of
/// the Browser tab's explicit policies. That policy intentionally replaces the
/// live Manual/Auto/Full choice for this call.
pub async fn run_browser<F, Fut>(
    state: &Shared,
    ctx: CallCtx,
    category: BrowserPermissionCategory,
    f: F,
) -> Result<CallToolResult, McpError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<CallToolResult, McpError>>,
{
    browser_hard_gate(state, ctx.tool)?;
    state.begin_control(ctx.agent.clone(), catalog::action_label(ctx.tool), catalog::ActionSurface::Browser);
    state.touch();
    let _active_call = state.track_active_call();
    if !approve_browser_category(state, category, ctx.detail.clone(), ctx.agent.clone()).await {
        state.log_call(ctx.tool, ctx.agent, "denied", ctx.detail, None);
        return Err(McpError::invalid_request(
            "conduit: the user denied this browser action.",
            None,
        ));
    }
    // Settings, runtime availability, or Panic Stop may have changed while an
    // approval card was open. Browser permissions never outrank those gates.
    browser_hard_gate(state, ctx.tool)?;
    let started = Instant::now();
    let result = f().await;
    state.touch();
    let duration = Some(started.elapsed().as_millis() as u64);
    match &result {
        Ok(_) => state.log_call(ctx.tool, ctx.agent, "ok", ctx.detail, duration),
        Err(error) => state.log_call(
            ctx.tool,
            ctx.agent,
            "error",
            Some(error.message.to_string()),
            duration,
        ),
    }
    result
}

/// Runs a browser call whose category boundary is discovered inside Chromium.
/// The sidecar routes the resulting URL/download through
/// [`approve_browser_category`], so this layer bypasses global mode without
/// pre-approving a destination it cannot yet describe.
pub async fn run_browser_discovered<F, Fut>(
    state: &Shared,
    ctx: CallCtx,
    f: F,
) -> Result<CallToolResult, McpError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<CallToolResult, McpError>>,
{
    browser_hard_gate(state, ctx.tool)?;
    state.begin_control(ctx.agent.clone(), catalog::action_label(ctx.tool), catalog::ActionSurface::Browser);
    state.touch();
    let _active_call = state.track_active_call();
    let started = Instant::now();
    let result = f().await;
    state.touch();
    let duration = Some(started.elapsed().as_millis() as u64);
    match &result {
        Ok(_) => state.log_call(ctx.tool, ctx.agent, "ok", ctx.detail, duration),
        Err(error) => state.log_call(
            ctx.tool,
            ctx.agent,
            "error",
            Some(error.message.to_string()),
            duration,
        ),
    }
    result
}

/* ── sandboxes ── */

pub struct SandboxCtx {
    pub sandbox: String,
    pub tool: &'static str,
    pub detail: Option<String>,
    pub agent: Option<String>,
    /// Where the pointer is headed, when the call says — drawn as a pulse on
    /// the Sandbox tab's live view.
    pub point: Option<(f64, f64)>,
}

/// Why a sandbox call may not run, or `None` if policy allows it. The global
/// panic latch and the Tools tab both reach into sandboxes; the Manual/Auto
/// approval mode does not, because a sandbox is the place an agent is meant to
/// act without asking (the Sandbox tab says so).
pub(crate) fn sandbox_refusal(
    aborted: bool,
    tool_enabled: bool,
    tools_access: crate::state::ToolsAccess,
) -> Option<&'static str> {
    if aborted {
        return Some(
            "conduit: the user stopped control. retrying will not help — they have to hand it \
             back from conduit's window before any tool works again. ask them to.",
        );
    }
    if !tool_enabled {
        return Some(match tools_access {
            crate::state::ToolsAccess::Off => {
                "conduit: all tools are switched off in the Tools tab."
            }
            _ => "conduit: this tool is switched off in the Tools tab.",
        });
    }
    None
}

/// The gate for a tool call into a sandbox.
///
/// Deliberately never touches the host's control session: no glow, no pill,
/// no AI cursor, no approval card on the user's screen — the reason sandboxes
/// exist is to leave that screen alone. Calls are still logged, still stopped
/// by panic stop, and cancelled by the Sandbox tab's "stop agent".
pub async fn run_sandbox<F, Fut>(
    state: &Shared,
    ctx: SandboxCtx,
    f: F,
) -> Result<CallToolResult, McpError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<CallToolResult, McpError>>,
{
    let started = Instant::now();
    let tool = ctx.tool;
    let target = Some(ctx.sandbox.clone());
    let activity = |outcome: &'static str| crate::sandbox::model::SandboxActivity {
        sandbox_id: ctx.sandbox.clone(),
        tool: tool.to_string(),
        action: catalog::action_label(tool).to_string(),
        agent: ctx.agent.clone(),
        detail: ctx.detail.clone(),
        outcome,
        x: ctx.point.map(|p| p.0),
        y: ctx.point.map(|p| p.1),
        at: crate::state::now_millis(),
    };
    let refuse = |why: String| {
        state.log_call_for(target.clone(), tool, ctx.agent.clone(), "blocked", Some(why.clone()), None);
        state.sandboxes.emit_activity(activity("blocked"));
        Err(McpError::invalid_request(why, None))
    };

    let settings = state.settings();
    if let Some(why) = sandbox_refusal(
        state.is_aborted(),
        settings.tool_enabled(tool),
        settings.tools_access,
    ) {
        return refuse(why.to_string());
    }

    let guard = match state.sandboxes.begin_call(&ctx.sandbox).await {
        Ok(guard) => guard,
        Err(why) => return refuse(format!("conduit: {why}")),
    };
    // A panic stop can land while the sandbox was starting on demand.
    if state.is_aborted() {
        return refuse("conduit: control was stopped by the user.".into());
    }
    state.sandboxes.emit_activity(activity("running"));

    let result = tokio::select! {
        result = f() => result,
        _ = guard.cancel.cancelled() => Err(McpError::invalid_request(
            "conduit: the user stopped the agent in this sandbox.",
            None,
        )),
    };
    drop(guard);

    let ms = Some(started.elapsed().as_millis() as u64);
    match &result {
        Ok(_) => {
            state.log_call_for(target, tool, ctx.agent.clone(), "ok", ctx.detail.clone(), ms);
            state.sandboxes.emit_activity(activity("ok"));
        }
        Err(e) => {
            state.log_call_for(
                target,
                tool,
                ctx.agent.clone(),
                "error",
                Some(e.message.to_string()),
                ms,
            );
            state.sandboxes.emit_activity(activity("error"));
        }
    }
    result
}

pub(crate) fn browser_hard_gate(state: &Shared, tool: &str) -> Result<(), McpError> {
    if state.is_aborted() {
        return Err(McpError::invalid_request(
            "conduit: control was stopped by the user.",
            None,
        ));
    }
    if !state.settings().tool_enabled(tool) {
        return Err(McpError::invalid_request(
            "conduit: this browser tool is disabled in the Tools tab.",
            None,
        ));
    }
    if !state.browser.ready() {
        return Err(McpError::invalid_request(
            "conduit: Chromium is not installed. ask the user to download it from the Browser tab.",
            None,
        ));
    }
    Ok(())
}

/// Handles a browser effect discovered after a tool has started, such as a
/// link opening a new page or a click producing a download.
pub async fn approve_browser_category(
    state: &Shared,
    category: BrowserPermissionCategory,
    detail: Option<String>,
    agent: Option<String>,
) -> bool {
    if state.is_aborted() {
        return false;
    }
    let _active_call = state.track_active_call();
    let key = format!("browser:{}", category.as_key());
    if !browser_permission_requires_prompt(
        state.settings().browser_permissions.get(category),
        state.control().mode,
        state.session_allowed(&key),
    ) {
        return true;
    }
    state.begin_control(agent.clone(), browser_action_label(category), catalog::ActionSurface::Browser);
    state.touch();
    let id = state.next_id("approval");
    let request = PendingApproval {
        id: id.clone(),
        tool: format!("browser:{}", category.as_key()),
        summary: browser_approval_summary(category).into(),
        detail,
        agent,
        category: Some(category),
    };
    let receiver = state.open_approval(request);
    let decision = match tokio::time::timeout(APPROVAL_TIMEOUT, receiver).await {
        Ok(Ok(decision)) => decision,
        _ => {
            state.close_approval(&id);
            Decision::Deny
        }
    };
    state.touch();
    match decision {
        Decision::Allow => true,
        Decision::Session => {
            state.allow_for_session(&key);
            true
        }
        Decision::Deny => false,
    }
}

fn browser_permission_requires_prompt(
    permission: BrowserPermissionMode,
    _global_mode: AccessMode,
    session_granted: bool,
) -> bool {
    !session_granted && permission == BrowserPermissionMode::AlwaysAsk
}

fn browser_action_label(category: BrowserPermissionCategory) -> &'static str {
    match category {
        BrowserPermissionCategory::OpenWebsites => "opening a website",
        BrowserPermissionCategory::ReadHistory => "reading browser history",
        BrowserPermissionCategory::DownloadFiles => "downloading a file",
        BrowserPermissionCategory::UploadFiles => "uploading a file",
    }
}

fn browser_approval_summary(category: BrowserPermissionCategory) -> &'static str {
    match category {
        BrowserPermissionCategory::OpenWebsites => "wants to open a website",
        BrowserPermissionCategory::ReadHistory => "wants to read your browser history",
        BrowserPermissionCategory::DownloadFiles => "wants to download a file",
        BrowserPermissionCategory::UploadFiles => "wants to upload a local file",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_permission_matrix_overrides_every_global_mode() {
        let categories = [
            BrowserPermissionCategory::OpenWebsites,
            BrowserPermissionCategory::ReadHistory,
            BrowserPermissionCategory::DownloadFiles,
            BrowserPermissionCategory::UploadFiles,
        ];
        let global_modes = [AccessMode::Manual, AccessMode::Auto, AccessMode::Full];

        for category in categories {
            for global in global_modes {
                assert!(
                    !browser_permission_requires_prompt(
                        BrowserPermissionMode::AlwaysAllow,
                        global,
                        false,
                    ),
                    "{category:?} Always allow must bypass {global:?}",
                );
                assert!(
                    browser_permission_requires_prompt(
                        BrowserPermissionMode::AlwaysAsk,
                        global,
                        false,
                    ),
                    "{category:?} Always ask must override {global:?}",
                );
            }
        }
    }

    /// The panic latch and the Tools tab reach into sandboxes; nothing else in
    /// this function may refuse.
    #[test]
    fn sandbox_calls_answer_to_panic_stop_and_the_tools_tab_only() {
        use crate::state::ToolsAccess;
        for access in [ToolsAccess::All, ToolsAccess::Custom, ToolsAccess::Off] {
            assert!(sandbox_refusal(true, true, access).is_some(), "panic stop must win");
            assert!(sandbox_refusal(true, false, access).is_some());
        }
        assert!(sandbox_refusal(false, true, ToolsAccess::All).is_none());
        assert!(sandbox_refusal(false, true, ToolsAccess::Custom).is_none());
        let off = sandbox_refusal(false, false, ToolsAccess::Off).unwrap();
        assert!(off.contains("all tools"), "{off}");
        let one = sandbox_refusal(false, false, ToolsAccess::Custom).unwrap();
        assert!(one.contains("this tool"), "{one}");
    }

    #[test]
    fn category_session_grant_suppresses_only_the_category_prompt() {
        for global in [AccessMode::Manual, AccessMode::Auto, AccessMode::Full] {
            assert!(!browser_permission_requires_prompt(
                BrowserPermissionMode::AlwaysAsk,
                global,
                true,
            ));
        }
    }
}
