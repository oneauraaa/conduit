//! The tool catalog — the single source of truth for what conduit exposes.
//!
//! The Tools tab renders straight from this list, and the gate reads `risky`
//! from it, so adding a tool means adding one row here plus the handler.

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolGroup {
    Vision,
    Input,
    Windows,
    Accessibility,
    System,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDef {
    pub name: &'static str,
    pub group: ToolGroup,
    pub summary: &'static str,
    /// Prompts for confirmation in `auto` mode; always prompts in `manual`.
    pub risky: bool,
}

const fn tool(
    name: &'static str,
    group: ToolGroup,
    summary: &'static str,
    risky: bool,
) -> ToolDef {
    ToolDef {
        name,
        group,
        summary,
        risky,
    }
}

use ToolGroup::*;

pub static CATALOG: &[ToolDef] = &[
    // vision
    tool("screenshot", Vision, "capture a display, or a region of one", false),
    tool("list_displays", Vision, "enumerate displays with their bounds", false),
    // input
    tool("move_cursor", Input, "glide the cursor to a point", false),
    tool("click", Input, "click, double-click or right-click", false),
    tool("drag", Input, "press, move, release — for sliders and drag-and-drop", false),
    tool("scroll", Input, "scroll the surface under the cursor", false),
    tool("type_text", Input, "type a string, unicode included", false),
    tool("key_press", Input, "press a key with optional modifiers", false),
    tool("get_cursor_position", Input, "read where the cursor is now", false),
    // windows & apps
    tool("list_windows", Windows, "every on-screen window with its bounds", false),
    tool("focus_window", Windows, "bring a window to the front", false),
    tool("set_window_bounds", Windows, "move or resize a window", false),
    tool("list_apps", Windows, "running applications", false),
    tool("open_app", Windows, "launch or activate an application", false),
    tool("quit_app", Windows, "ask an application to quit", true),
    // accessibility
    tool(
        "read_screen_text",
        Accessibility,
        "read on-screen text and control bounds from the accessibility tree",
        false,
    ),
    tool(
        "find_element",
        Accessibility,
        "locate a control by its label and get a point to click",
        false,
    ),
    // system
    tool("clipboard_read", System, "read the clipboard's text", false),
    tool("clipboard_write", System, "replace the clipboard's text", true),
    tool("run_shell", System, "run a shell command and capture its output", true),
    tool("wait", System, "pause, to let the ui settle", false),
    tool("notify", System, "post a notification", false),
];

pub fn find(name: &str) -> Option<&'static ToolDef> {
    CATALOG.iter().find(|t| t.name == name)
}

pub fn is_risky(name: &str) -> bool {
    find(name).is_some_and(|t| t.risky)
}

/// Present-tense label shown in the pill while the tool runs.
pub fn action_label(name: &str) -> &'static str {
    match name {
        "screenshot" => "looking at the screen",
        "list_displays" => "checking displays",
        "move_cursor" => "moving",
        "click" => "clicking",
        "drag" => "dragging",
        "scroll" => "scrolling",
        "type_text" => "typing",
        "key_press" => "pressing keys",
        "get_cursor_position" => "checking the cursor",
        "list_windows" | "list_apps" => "looking around",
        "focus_window" => "switching windows",
        "set_window_bounds" => "arranging a window",
        "open_app" => "opening an app",
        "quit_app" => "quitting an app",
        "read_screen_text" | "find_element" => "reading the screen",
        "clipboard_read" => "reading the clipboard",
        "clipboard_write" => "writing the clipboard",
        "run_shell" => "running a command",
        "wait" => "waiting",
        "notify" => "sending a notification",
        _ => "working",
    }
}

/// Second-person phrasing for the approval card: "<agent> wants to …".
pub fn approval_summary(name: &str) -> String {
    let verb = match name {
        "screenshot" => "take a screenshot",
        "move_cursor" => "move the cursor",
        "click" => "click",
        "drag" => "drag",
        "scroll" => "scroll",
        "type_text" => "type",
        "key_press" => "press a key combination",
        "focus_window" => "switch windows",
        "set_window_bounds" => "move a window",
        "open_app" => "open an app",
        "quit_app" => "quit an app",
        "read_screen_text" | "find_element" => "read the screen",
        "clipboard_read" => "read your clipboard",
        "clipboard_write" => "overwrite your clipboard",
        "run_shell" => "run a shell command",
        "notify" => "send a notification",
        _ => name,
    };
    format!("wants to {verb}")
}
