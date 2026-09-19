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
    Browser,
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
    tool("web_search", System, "search the web with DuckDuckGo", false),
    tool("clipboard_read", System, "read the clipboard's text", false),
    tool("clipboard_write", System, "replace the clipboard's text", true),
    tool("run_shell", System, "run a shell command and capture its output", true),
    tool("wait", System, "pause, to let the ui settle", false),
    tool("notify", System, "post a notification", false),
    tool(
        "list_keybinds",
        System,
        "the keyboard shortcuts the user has bound",
        false,
    ),
    // browser — dynamically hidden until the required Chromium bundle is installed
    tool("browser_navigate", Browser, "navigate the active browser tab", false),
    tool("browser_navigate_back", Browser, "go back in the active tab", false),
    tool("browser_snapshot", Browser, "capture the page accessibility snapshot", false),
    tool("browser_find", Browser, "find text in the page snapshot", false),
    tool("browser_click", Browser, "click an element in the page", false),
    tool("browser_type", Browser, "type into an editable page element", false),
    tool("browser_fill_form", Browser, "fill several form controls", false),
    tool("browser_hover", Browser, "hover over a page element", false),
    tool("browser_drag", Browser, "drag between two page elements", false),
    tool("browser_drop", Browser, "drop files or data onto a page element", false),
    tool("browser_select_option", Browser, "select one or more dropdown options", false),
    tool("browser_press_key", Browser, "press a key in the page", false),
    tool("browser_handle_dialog", Browser, "accept or dismiss a page dialog", false),
    tool("browser_file_upload", Browser, "upload files through a page chooser", false),
    tool("browser_take_screenshot", Browser, "capture the current web page", false),
    tool("browser_wait_for", Browser, "wait for page text or time", false),
    tool("browser_tabs", Browser, "list, create, close, or select browser tabs", false),
    tool("browser_resize", Browser, "resize the browser viewport", false),
    tool("browser_close", Browser, "close the active browser page", true),
    tool("browser_history", Browser, "search the selected profile's navigation history", false),
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
        "web_search" => "searching DuckDuckGo",
        "clipboard_read" => "reading the clipboard",
        "clipboard_write" => "writing the clipboard",
        "run_shell" => "running a command",
        "wait" => "waiting",
        "notify" => "sending a notification",
        "list_keybinds" => "reading your shortcuts",
        "browser_navigate" | "browser_navigate_back" => "opening a website",
        "browser_snapshot" | "browser_find" => "reading a web page",
        "browser_click" => "clicking in the browser",
        "browser_type" | "browser_fill_form" => "typing in the browser",
        "browser_hover" => "hovering in the browser",
        "browser_drag" | "browser_drop" => "dragging in the browser",
        "browser_select_option" => "choosing an option",
        "browser_press_key" => "pressing a browser key",
        "browser_handle_dialog" => "handling a browser dialog",
        "browser_file_upload" => "uploading a file",
        "browser_take_screenshot" => "capturing a web page",
        "browser_wait_for" => "waiting for a web page",
        "browser_tabs" => "managing browser tabs",
        "browser_resize" => "resizing the browser",
        "browser_close" => "closing a browser page",
        "browser_history" => "reading browser history",
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
        "web_search" => "search the web",
        "clipboard_read" => "read your clipboard",
        "clipboard_write" => "overwrite your clipboard",
        "run_shell" => "run a shell command",
        "notify" => "send a notification",
        "list_keybinds" => "read your keyboard shortcuts",
        "browser_navigate" | "browser_navigate_back" => "open a website",
        "browser_snapshot" | "browser_find" => "read a web page",
        "browser_click" => "click in the browser",
        "browser_type" | "browser_fill_form" => "type in the browser",
        "browser_hover" => "hover in the browser",
        "browser_drag" | "browser_drop" => "drag in the browser",
        "browser_select_option" => "select a browser option",
        "browser_press_key" => "press a browser key",
        "browser_handle_dialog" => "handle a browser dialog",
        "browser_file_upload" => "upload a local file",
        "browser_take_screenshot" => "capture a web page",
        "browser_wait_for" => "wait for a web page",
        "browser_tabs" => "manage browser tabs",
        "browser_resize" => "resize the browser",
        "browser_close" => "close a browser page",
        "browser_history" => "read browser history",
        _ => name,
    };
    format!("wants to {verb}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_search_is_a_non_risky_system_tool() {
        let definition = find("web_search").expect("web_search should be catalogued");
        assert!(matches!(definition.group, ToolGroup::System));
        assert!(!definition.risky);
        assert_eq!(action_label("web_search"), "searching DuckDuckGo");
        assert_eq!(approval_summary("web_search"), "wants to search the web");
    }
}
