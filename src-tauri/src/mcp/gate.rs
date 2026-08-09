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

use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;

use crate::mcp::catalog;
use crate::state::{AccessMode, Decision, PendingApproval, Shared};

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
    let started = Instant::now();
    let tool = ctx.tool;

    // 1. panic stop
    if state.is_aborted() {
        state.log_call(tool, ctx.agent.clone(), "blocked", Some("stopped by user".into()), None);
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
        state.log_call(tool, ctx.agent.clone(), "blocked", Some("disabled".into()), None);
        return Err(McpError::invalid_request(why, None));
    }

    // The session becomes "active" the moment an agent reaches for anything —
    // including reads. Watching the screen is exactly when the user most wants
    // to see the glow.
    state.begin_control(ctx.agent.clone(), catalog::action_label(tool));
    state.touch();

    // 4. approval
    let needs_approval = match state.control().mode {
        AccessMode::Full => false,
        AccessMode::Auto => catalog::is_risky(tool),
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
        state.log_call(tool, ctx.agent.clone(), "blocked", Some("stopped by user".into()), None);
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
        Err(e) => state.log_call(tool, ctx.agent, "error", Some(e.message.to_string()), Some(ms)),
    }

    result
}
