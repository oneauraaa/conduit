//! The MCP tool surface.
//!
//! Every handler is thin on purpose: it parses arguments, calls through
//! [`gate::run`], and delegates to `mac/`. Policy lives in the gate; platform
//! detail lives in `mac/`. Nothing in between.

use base64::Engine;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;
use tauri::Emitter;

use crate::mac::{ax, capture, clipboard, input, screen, windows as macwin};
use crate::mcp::gate::{self, CallCtx};
use crate::state::{CursorEvent, PulseEvent, Shared};

/* ── argument shapes ────────────────────────────────────────── */

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ScreenshotArgs {
    /// Which display to capture. Defaults to the primary display.
    #[serde(default)]
    pub display: Option<usize>,
    /// Optional crop within the display: [x, y, width, height] in points,
    /// relative to that display's top-left corner.
    #[serde(default)]
    pub region: Option<[f64; 4]>,
    /// Downscale factor, 0.1–1.0. Lower costs fewer tokens; 0.5 is usually
    /// still readable for large text.
    #[serde(default)]
    pub scale: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PointArgs {
    /// Global screen x, in points, top-left origin.
    pub x: f64,
    /// Global screen y, in points, top-left origin.
    pub y: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ClickArgs {
    /// Where to click. Omit to click wherever the cursor already is.
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    /// "left" (default), "right" or "middle".
    #[serde(default)]
    pub button: Option<String>,
    /// 1 for a single click, 2 for a double-click.
    #[serde(default)]
    pub count: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DragArgs {
    /// Start point. Omit to start from the current cursor position.
    #[serde(default)]
    pub from_x: Option<f64>,
    #[serde(default)]
    pub from_y: Option<f64>,
    pub to_x: f64,
    pub to_y: f64,
    #[serde(default)]
    pub button: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ScrollArgs {
    /// Horizontal scroll in points. Positive scrolls right.
    #[serde(default)]
    pub dx: Option<i32>,
    /// Vertical scroll in points. Positive scrolls up, matching macOS.
    #[serde(default)]
    pub dy: Option<i32>,
    /// Move here before scrolling, so the right surface receives it.
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TypeArgs {
    /// Literal text. Layout-independent — unicode is fine.
    pub text: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct KeyArgs {
    /// Key name, e.g. "return", "tab", "escape", "a", "f5", "left".
    pub key: String,
    /// Any of "cmd", "shift", "alt", "ctrl", "fn".
    #[serde(default)]
    pub modifiers: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WindowIdArgs {
    pub window_id: u32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WindowBoundsArgs {
    pub window_id: u32,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AppNameArgs {
    /// The application's display name, e.g. "Safari".
    pub name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadScreenArgs {
    /// Which app to read. Defaults to the frontmost one.
    #[serde(default)]
    pub app: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FindElementArgs {
    /// Text to look for, matched case-insensitively.
    pub query: String,
    #[serde(default)]
    pub app: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TextArgs {
    pub text: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ShellArgs {
    /// The command line, run through `/bin/zsh -c`.
    pub command: String,
    /// Give up after this many seconds. Defaults to 30.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WaitArgs {
    pub milliseconds: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct NotifyArgs {
    pub title: String,
    pub body: String,
}

/* ── helpers ────────────────────────────────────────────────── */

fn ok(text: impl Into<String>) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

fn json_ok<T: serde::Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
    ok(text)
}

fn fail(msg: impl Into<String>) -> McpError {
    McpError::internal_error(msg.into(), None)
}

/// conduit's own UI is off limits to agents — see
/// [`crate::windows::point_hits_conduit`] for why.
fn guard_self_target(app: &tauri::AppHandle, x: f64, y: f64) -> Result<(), McpError> {
    if crate::windows::point_hits_conduit(app, x, y) {
        return Err(McpError::invalid_request(
            "conduit: that point is inside conduit's own window. its controls are for the user \
             only — you cannot change your own permissions. work around it or ask the user.",
            None,
        ));
    }
    Ok(())
}

fn parse_button(s: Option<&str>) -> input::Button {
    match s.map(|b| b.to_ascii_lowercase()).as_deref() {
        Some("right") => input::Button::Right,
        Some("middle") => input::Button::Middle,
        _ => input::Button::Left,
    }
}

#[derive(Clone)]
pub struct Conduit {
    state: Shared,
    /// Read by the `#[tool_handler]` macro's generated `ServerHandler` impl,
    /// which dead-code analysis can't see through.
    #[allow(dead_code)]
    tool_router: ToolRouter<Conduit>,
}

impl Conduit {
    pub fn new(state: Shared) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    /// The connected agent's name, taken from the MCP handshake. This is what
    /// the pill shows, so it needs to read like a product name.
    fn agent(&self, ctx: &RequestContext<RoleServer>) -> Option<String> {
        ctx.peer
            .peer_info()
            .map(|info| pretty_agent(&info.client_info.name))
    }

    /// Pushes the cursor position to whichever overlay owns that point.
    ///
    /// Coordinates are made local to that display here, so the overlay never
    /// needs to know where its screen sits in the global arrangement. Display
    /// geometry comes from the cache because NSScreen is main-thread-only and
    /// this runs on a tokio worker.
    fn emit_cursor(&self, x: f64, y: f64) {
        let displays = crate::windows::cached_displays();
        let idx = screen::display_at(&displays, x, y);
        let Some(d) = displays.get(idx) else { return };

        let _ = self.state.app.emit_to(
            crate::windows::overlay_label(idx),
            "control:cursor",
            CursorEvent {
                x: x - d.x,
                y: y - d.y,
                display: idx,
            },
        );
    }

    fn emit_pulse(&self, kind: &'static str) {
        let (x, y) = input::cursor_position();
        let displays = crate::windows::cached_displays();
        let idx = screen::display_at(&displays, x, y);
        let _ = self.state.app.emit_to(
            crate::windows::overlay_label(idx),
            "control:pulse",
            PulseEvent { kind, display: idx },
        );
    }
}

/// Turns an MCP client id into something worth showing a human.
fn pretty_agent(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("claude-code") || lower.contains("claude code") {
        "claude code".into()
    } else if lower.contains("codex") {
        "codex".into()
    } else if lower.contains("hermes") {
        "hermes".into()
    } else if lower.contains("openclaw") {
        "openclaw".into()
    } else if lower.contains("opencode") {
        "opencode".into()
    } else if lower.contains("gemini") || lower.contains("antigravity") {
        "gemini".into()
    } else if lower.contains("claude") {
        "claude".into()
    } else {
        lower.replace(['_', '-'], " ")
    }
}

#[tool_router]
impl Conduit {
    /* ── vision ── */

    #[tool(
        description = "Capture the screen as a PNG. Prefer read_screen_text when you only need \
                       text or control positions — it is far cheaper and more accurate. Use this \
                       when you need to see layout, images or anything visual."
    )]
    async fn screenshot(
        &self,
        Parameters(args): Parameters<ScreenshotArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let agent = self.agent(&ctx);
        let detail = args.display.map(|d| format!("display {d}"));

        gate::run(
            &self.state,
            CallCtx { tool: "screenshot", detail, agent },
            || async {
                let displays = crate::windows::cached_displays();
                let index = args.display.unwrap_or(0);
                let display = *displays
                    .get(index)
                    .ok_or_else(|| fail(format!("no display at index {index}")))?;

                let region = args.region.map(|r| (r[0], r[1], r[2], r[3]));
                let scale = args.scale.unwrap_or(1.0).clamp(0.1, 1.0);

                // ScreenCaptureKit blocks; keep it off the async runtime.
                let shot = tokio::task::spawn_blocking(move || {
                    capture::capture(&display, region, scale)
                })
                .await
                .map_err(|e| fail(format!("capture task failed: {e}")))?
                .map_err(fail)?;

                let b64 = base64::engine::general_purpose::STANDARD.encode(&shot.png);
                Ok(CallToolResult::success(vec![
                    ContentBlock::text(format!(
                        "display {index}, {}x{} points captured at {}x{} pixels",
                        display.width as u32, display.height as u32, shot.width, shot.height
                    )),
                    ContentBlock::image(b64, "image/png"),
                ]))
            },
        )
        .await
    }

    #[tool(description = "List the attached displays with their bounds, in the same top-left \
                          coordinate space every other tool uses.")]
    async fn list_displays(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let agent = self.agent(&ctx);
        gate::run(
            &self.state,
            CallCtx { tool: "list_displays", detail: None, agent },
            || async { json_ok(&crate::windows::cached_displays()) },
        )
        .await
    }

    /* ── input ── */

    #[tool(description = "Move the cursor to a point, with a natural glide rather than a jump.")]
    async fn move_cursor(
        &self,
        Parameters(args): Parameters<PointArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let agent = self.agent(&ctx);
        let detail = Some(format!("{:.0}, {:.0}", args.x, args.y));

        gate::run(
            &self.state,
            CallCtx { tool: "move_cursor", detail, agent },
            || async {
                guard_self_target(&self.state.app, args.x, args.y)?;
                input::glide(args.x, args.y, |x, y| self.emit_cursor(x, y)).await;
                ok(format!("cursor at {:.0}, {:.0}", args.x, args.y))
            },
        )
        .await
    }

    #[tool(description = "Click the mouse. Pass x and y to move there first, or omit them to \
                          click where the cursor already is. count: 2 gives a real double-click.")]
    async fn click(
        &self,
        Parameters(args): Parameters<ClickArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let agent = self.agent(&ctx);
        let button = parse_button(args.button.as_deref());
        let count = args.count.unwrap_or(1).clamp(1, 3);
        let detail = match (args.x, args.y) {
            (Some(x), Some(y)) => Some(format!("{:.0}, {:.0}", x, y)),
            _ => Some("at the cursor".into()),
        };

        gate::run(
            &self.state,
            CallCtx { tool: "click", detail, agent },
            || async {
                if let (Some(x), Some(y)) = (args.x, args.y) {
                    guard_self_target(&self.state.app, x, y)?;
                    input::glide(x, y, |px, py| self.emit_cursor(px, py)).await;
                } else {
                    // No explicit target: the cursor may already be parked over
                    // conduit from an earlier move, so check where it actually is.
                    let (cx, cy) = input::cursor_position();
                    guard_self_target(&self.state.app, cx, cy)?;
                }
                input::click(button, count).await;
                self.emit_pulse("click");
                let (x, y) = input::cursor_position();
                ok(format!("clicked at {x:.0}, {y:.0}"))
            },
        )
        .await
    }

    #[tool(description = "Press, move and release — for sliders, resize handles and \
                          drag-and-drop.")]
    async fn drag(
        &self,
        Parameters(args): Parameters<DragArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let agent = self.agent(&ctx);
        let button = parse_button(args.button.as_deref());
        let detail = Some(format!("to {:.0}, {:.0}", args.to_x, args.to_y));

        gate::run(
            &self.state,
            CallCtx { tool: "drag", detail, agent },
            || async {
                guard_self_target(&self.state.app, args.to_x, args.to_y)?;
                if let (Some(x), Some(y)) = (args.from_x, args.from_y) {
                    guard_self_target(&self.state.app, x, y)?;
                    input::glide(x, y, |px, py| self.emit_cursor(px, py)).await;
                }
                input::drag(args.to_x, args.to_y, button, |px, py| self.emit_cursor(px, py)).await;
                ok(format!("dragged to {:.0}, {:.0}", args.to_x, args.to_y))
            },
        )
        .await
    }

    #[tool(description = "Scroll the surface under the cursor. Positive dy scrolls up.")]
    async fn scroll(
        &self,
        Parameters(args): Parameters<ScrollArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let agent = self.agent(&ctx);
        let (dx, dy) = (args.dx.unwrap_or(0), args.dy.unwrap_or(0));
        let detail = Some(format!("dx {dx}, dy {dy}"));

        gate::run(
            &self.state,
            CallCtx { tool: "scroll", detail, agent },
            || async {
                if let (Some(x), Some(y)) = (args.x, args.y) {
                    guard_self_target(&self.state.app, x, y)?;
                    input::glide(x, y, |px, py| self.emit_cursor(px, py)).await;
                }
                input::scroll(dx, dy);
                ok(format!("scrolled dx {dx}, dy {dy}"))
            },
        )
        .await
    }

    #[tool(description = "Type literal text into whatever has keyboard focus. Layout-independent, \
                          so accented and non-Latin characters work. Use key_press for shortcuts.")]
    async fn type_text(
        &self,
        Parameters(args): Parameters<TypeArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let agent = self.agent(&ctx);
        let preview: String = args.text.chars().take(42).collect();
        let detail = Some(if args.text.chars().count() > 42 {
            format!("{preview}…")
        } else {
            preview
        });

        gate::run(
            &self.state,
            CallCtx { tool: "type_text", detail, agent },
            || async {
                input::type_text(&args.text).await;
                self.emit_pulse("key");
                ok(format!("typed {} characters", args.text.chars().count()))
            },
        )
        .await
    }

    #[tool(description = "Press a key with optional modifiers, e.g. key 's' with modifiers \
                          ['cmd'] to save.")]
    async fn key_press(
        &self,
        Parameters(args): Parameters<KeyArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let agent = self.agent(&ctx);
        let combo = if args.modifiers.is_empty() {
            args.key.clone()
        } else {
            format!("{}+{}", args.modifiers.join("+"), args.key)
        };
        let detail = Some(combo.clone());

        gate::run(
            &self.state,
            CallCtx { tool: "key_press", detail, agent },
            || async {
                input::key_press(&args.key, &args.modifiers).map_err(fail)?;
                self.emit_pulse("key");
                ok(format!("pressed {combo}"))
            },
        )
        .await
    }

    #[tool(description = "Where the cursor is right now.")]
    async fn get_cursor_position(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let agent = self.agent(&ctx);
        gate::run(
            &self.state,
            CallCtx { tool: "get_cursor_position", detail: None, agent },
            || async {
                let (x, y) = input::cursor_position();
                json_ok(&serde_json::json!({ "x": x, "y": y }))
            },
        )
        .await
    }

    /* ── windows & apps ── */

    #[tool(description = "Every on-screen window with its title, owning app and bounds, \
                          frontmost first.")]
    async fn list_windows(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let agent = self.agent(&ctx);
        gate::run(
            &self.state,
            CallCtx { tool: "list_windows", detail: None, agent },
            || async {
                let windows = macwin::list_windows();

                // macOS gates kCGWindowName behind Screen Recording, so without
                // that grant every title comes back empty and the list looks
                // broken rather than restricted. Say which it is.
                let titles_hidden = !windows.is_empty()
                    && windows.iter().all(|w| w.title.is_empty())
                    && !crate::mac::permissions::screen_recording_granted();

                let json = serde_json::to_string_pretty(&windows)
                    .map_err(|e| fail(e.to_string()))?;

                if titles_hidden {
                    return ok(format!(
                        "note: window titles are empty because conduit does not have \
                         Screen Recording permission — macOS hides them without it. \
                         Bounds and app names are still accurate. Ask the user to grant \
                         it in conduit's Server tab.\n\n{json}"
                    ));
                }
                ok(json)
            },
        )
        .await
    }

    #[tool(description = "Bring a window to the front, by the window_id from list_windows.")]
    async fn focus_window(
        &self,
        Parameters(args): Parameters<WindowIdArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let agent = self.agent(&ctx);
        gate::run(
            &self.state,
            CallCtx { tool: "focus_window", detail: Some(format!("#{}", args.window_id)), agent },
            || async {
                macwin::focus_window(args.window_id).map_err(fail)?;
                ok(format!("focused window {}", args.window_id))
            },
        )
        .await
    }

    #[tool(description = "Move and resize a window.")]
    async fn set_window_bounds(
        &self,
        Parameters(args): Parameters<WindowBoundsArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let agent = self.agent(&ctx);
        gate::run(
            &self.state,
            CallCtx {
                tool: "set_window_bounds",
                detail: Some(format!("#{}", args.window_id)),
                agent,
            },
            || async {
                macwin::set_window_bounds(args.window_id, args.x, args.y, args.width, args.height)
                    .map_err(fail)?;
                ok("window moved")
            },
        )
        .await
    }

    #[tool(description = "Running applications.")]
    async fn list_apps(&self, ctx: RequestContext<RoleServer>) -> Result<CallToolResult, McpError> {
        let agent = self.agent(&ctx);
        gate::run(
            &self.state,
            CallCtx { tool: "list_apps", detail: None, agent },
            || async { json_ok(&macwin::list_apps()) },
        )
        .await
    }

    #[tool(description = "Launch an application, or bring it forward if it is already running.")]
    async fn open_app(
        &self,
        Parameters(args): Parameters<AppNameArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let agent = self.agent(&ctx);
        gate::run(
            &self.state,
            CallCtx { tool: "open_app", detail: Some(args.name.clone()), agent },
            || async {
                macwin::open_app(&args.name).map_err(fail)?;
                ok(format!("opened {}", args.name))
            },
        )
        .await
    }

    #[tool(description = "Ask an application to quit. It may prompt the user about unsaved work.")]
    async fn quit_app(
        &self,
        Parameters(args): Parameters<AppNameArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let agent = self.agent(&ctx);
        gate::run(
            &self.state,
            CallCtx { tool: "quit_app", detail: Some(args.name.clone()), agent },
            || async {
                macwin::quit_app(&args.name).map_err(fail)?;
                ok(format!("asked {} to quit", args.name))
            },
        )
        .await
    }

    /* ── accessibility ── */

    #[tool(
        description = "Read on-screen text and control bounds from the macOS accessibility tree. \
                       Strongly prefer this over screenshot for finding buttons, fields and \
                       labels: it returns exact strings and exact coordinates instead of making \
                       you estimate from pixels. Each element includes centerX/centerY you can \
                       pass straight to click."
    )]
    async fn read_screen_text(
        &self,
        Parameters(args): Parameters<ReadScreenArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let agent = self.agent(&ctx);
        gate::run(
            &self.state,
            CallCtx { tool: "read_screen_text", detail: args.app.clone(), agent },
            || async {
                let elements = ax::read_screen(args.app.as_deref()).map_err(fail)?;
                json_ok(&elements)
            },
        )
        .await
    }

    #[tool(description = "Find a control by its text and get a point to click. Results are \
                          ranked, best match first.")]
    async fn find_element(
        &self,
        Parameters(args): Parameters<FindElementArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let agent = self.agent(&ctx);
        gate::run(
            &self.state,
            CallCtx { tool: "find_element", detail: Some(args.query.clone()), agent },
            || async {
                let found = ax::find_element(&args.query, args.app.as_deref()).map_err(fail)?;
                if found.is_empty() {
                    return ok(format!("nothing matching {:?} is on screen", args.query));
                }
                json_ok(&found)
            },
        )
        .await
    }

    /* ── system ── */

    #[tool(description = "Read the clipboard's text.")]
    async fn clipboard_read(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let agent = self.agent(&ctx);
        gate::run(
            &self.state,
            CallCtx { tool: "clipboard_read", detail: None, agent },
            || async {
                match clipboard::read_text() {
                    Some(text) => ok(text),
                    None => ok("the clipboard holds no text"),
                }
            },
        )
        .await
    }

    #[tool(description = "Replace the clipboard's contents. This destroys whatever the user had \
                          copied, so it asks first unless the session is on full access.")]
    async fn clipboard_write(
        &self,
        Parameters(args): Parameters<TextArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let agent = self.agent(&ctx);
        let preview: String = args.text.chars().take(42).collect();
        gate::run(
            &self.state,
            CallCtx { tool: "clipboard_write", detail: Some(preview), agent },
            || async {
                clipboard::write_text(&args.text).map_err(fail)?;
                ok("clipboard updated")
            },
        )
        .await
    }

    #[tool(description = "Run a shell command and capture its output.")]
    async fn run_shell(
        &self,
        Parameters(args): Parameters<ShellArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let agent = self.agent(&ctx);
        let detail = Some(args.command.chars().take(64).collect::<String>());

        gate::run(
            &self.state,
            CallCtx { tool: "run_shell", detail, agent },
            || async {
                let timeout = std::time::Duration::from_secs(
                    args.timeout_seconds.unwrap_or(30).clamp(1, 300),
                );

                let child = tokio::process::Command::new("/bin/zsh")
                    .arg("-c")
                    .arg(&args.command)
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .map_err(|e| fail(format!("could not start the command: {e}")))?;

                let out = match tokio::time::timeout(timeout, child.wait_with_output()).await {
                    Ok(result) => result.map_err(|e| fail(format!("command failed: {e}")))?,
                    Err(_) => {
                        return Err(fail(format!(
                            "command timed out after {}s",
                            timeout.as_secs()
                        )));
                    }
                };

                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let code = out.status.code().unwrap_or(-1);

                let mut body = String::new();
                if !stdout.trim().is_empty() {
                    body.push_str(stdout.trim_end());
                }
                if !stderr.trim().is_empty() {
                    if !body.is_empty() {
                        body.push_str("\n\n");
                    }
                    body.push_str("stderr:\n");
                    body.push_str(stderr.trim_end());
                }
                if body.is_empty() {
                    body.push_str("(no output)");
                }
                ok(format!("exit {code}\n\n{body}"))
            },
        )
        .await
    }

    #[tool(description = "Pause, to let an animation finish or a window appear.")]
    async fn wait(
        &self,
        Parameters(args): Parameters<WaitArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let agent = self.agent(&ctx);
        let ms = args.milliseconds.clamp(1, 30_000);
        gate::run(
            &self.state,
            CallCtx { tool: "wait", detail: Some(format!("{ms}ms")), agent },
            || async {
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                ok(format!("waited {ms}ms"))
            },
        )
        .await
    }

    #[tool(description = "Post a macOS notification.")]
    async fn notify(
        &self,
        Parameters(args): Parameters<NotifyArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let agent = self.agent(&ctx);
        gate::run(
            &self.state,
            CallCtx { tool: "notify", detail: Some(args.title.clone()), agent },
            || async {
                macwin::notify(&args.title, &args.body).map_err(fail)?;
                ok("notification posted")
            },
        )
        .await
    }
}

#[tool_handler]
impl ServerHandler for Conduit {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_protocol_version(ProtocolVersion::V_2025_06_18)
            .with_instructions(
                "conduit gives you this Mac, the way a person uses it.\n\n\
                 Coordinates are logical points with the origin at the top-left of the primary \
                 display; list_displays tells you how the screens are arranged.\n\n\
                 A good loop is: read_screen_text or find_element to locate a control, click its \
                 centerX/centerY, then screenshot only if you need to confirm something visual. \
                 Reaching for screenshot first is slower, costlier and less accurate.\n\n\
                 The user can see everything you do — the screen glows and a pill above the Dock \
                 shows your current action. They can revoke a tool or stop you at any moment, so \
                 a refusal is a real answer from a real person, not a bug to route around."
                    .to_string(),
            )
    }
}
