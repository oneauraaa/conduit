//! Browser-specific MCP argument contracts and boundary validation.
//!
//! Handlers remain in `tools.rs` because rmcp builds one router from that impl;
//! the pinned public schemas and browser-only filesystem/URL checks live here
//! so the desktop tool surface stays readable.

use rmcp::model::{CallToolResult, Tool};
use rmcp::{ErrorData as McpError, schemars};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserNavigateArgs {
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct BrowserSnapshotArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boxes: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserFindArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regex: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(super) enum BrowserMouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub(super) enum BrowserModifier {
    Alt,
    Control,
    ControlOrMeta,
    Meta,
    Shift,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct BrowserClickArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub double_click: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button: Option<BrowserMouseButton>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modifiers: Option<Vec<BrowserModifier>>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserTypeArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
    pub target: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submit: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slowly: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(super) enum BrowserFormFieldType {
    Textbox,
    Checkbox,
    Radio,
    Combobox,
    Slider,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserFormField {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: BrowserFormFieldType,
    pub target: String,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserFillFormArgs {
    pub fields: Vec<BrowserFormField>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserElementArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
    pub target: String,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct BrowserDragArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_element: Option<String>,
    pub start_target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_element: Option<String>,
    pub end_target: String,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserDropArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paths: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserSelectOptionArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
    pub target: String,
    pub values: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserPressKeyArgs {
    pub key: String,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct BrowserDialogArgs {
    pub accept: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_text: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserFileUploadArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paths: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(super) enum BrowserScreenshotType {
    Png,
    Jpeg,
    Webp,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(super) enum BrowserScreenshotScale {
    Css,
    Device,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct BrowserScreenshotArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub image_type: Option<BrowserScreenshotType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_page: Option<bool>,
    pub scale: BrowserScreenshotScale,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct BrowserWaitArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_gone: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(super) enum BrowserTabsAction {
    List,
    New,
    Close,
    Select,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserTabsArgs {
    pub action: BrowserTabsAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserResizeArgs {
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct BrowserHistoryArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

pub(super) fn validate_browser_url(raw: &str) -> Result<(), McpError> {
    if raw == "about:blank" {
        return Ok(());
    }
    let url = url::Url::parse(raw)
        .map_err(|_| McpError::invalid_params("browser URL is invalid", None))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(McpError::invalid_params(
            "conduit: browser navigation only allows http, https, and about:blank",
            None,
        ));
    }
    Ok(())
}

pub(super) fn validate_output_filename(filename: Option<&str>) -> Result<(), McpError> {
    let Some(filename) = filename else {
        return Ok(());
    };
    let path = std::path::Path::new(filename);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err(McpError::invalid_params(
            "browser output filenames must be relative and cannot contain traversal",
            None,
        ));
    }
    Ok(())
}

pub(super) fn validate_upload_paths(paths: &Option<Vec<String>>) -> Result<(), McpError> {
    let Some(paths) = paths else { return Ok(()) };
    if paths.len() > 32 {
        return Err(McpError::invalid_params(
            "a browser upload is limited to 32 files",
            None,
        ));
    }
    for path in paths {
        if !std::path::Path::new(path).is_absolute() {
            return Err(McpError::invalid_params(
                "browser upload paths must be absolute",
                None,
            ));
        }
    }
    Ok(())
}

/// Runs only inside the permission-gated operation. Until the user has
/// approved Upload Files, Conduit neither stats nor opens the named paths.
pub(super) fn canonicalize_upload_arguments(
    arguments: &mut serde_json::Value,
) -> Result<(), McpError> {
    let Some(paths) = arguments.get_mut("paths") else {
        return Ok(());
    };
    let Some(paths) = paths.as_array_mut() else {
        return Ok(());
    };
    for path in paths {
        let raw = path.as_str().ok_or_else(|| {
            McpError::invalid_params("browser upload paths must be strings", None)
        })?;
        let resolved = std::fs::canonicalize(raw)
            .map_err(|e| McpError::invalid_params(format!("cannot upload {raw}: {e}"), None))?;
        if !resolved.is_file() {
            return Err(McpError::invalid_params(
                format!(
                    "cannot upload {} because it is not a regular file",
                    resolved.display()
                ),
                None,
            ));
        }
        *path = serde_json::Value::String(resolved.to_string_lossy().into_owned());
    }
    Ok(())
}

pub(super) fn browser_result(value: serde_json::Value) -> Result<CallToolResult, McpError> {
    serde_json::from_value(value).map_err(|e| {
        McpError::internal_error(
            format!("the Chromium sidecar returned an invalid MCP result: {e}"),
            None,
        )
    })
}

pub(super) fn serialize_args<T: Serialize>(args: &T) -> Result<serde_json::Value, McpError> {
    serde_json::to_value(args).map_err(|e| McpError::internal_error(e.to_string(), None))
}

/// The compatible definitions are generated from the exact pinned upstream
/// package so Rust derivation cannot drift from Playwright's public contract.
pub(super) fn pinned_browser_tools() -> &'static [Tool] {
    static TOOLS: std::sync::OnceLock<Vec<Tool>> = std::sync::OnceLock::new();
    TOOLS.get_or_init(|| {
        serde_json::from_str(include_str!("../../../browser-runtime/pinned-tools.json"))
            .expect("the generated pinned browser schemas must be valid MCP tools")
    })
}

pub(super) fn pinned_browser_tool(name: &str) -> Option<Tool> {
    if name == "browser_history" {
        return Some(browser_history_tool());
    }
    pinned_browser_tools()
        .iter()
        .find(|tool| tool.name.as_ref() == name)
        .cloned()
}

pub(super) fn browser_history_tool() -> Tool {
    serde_json::from_value(serde_json::json!({
        "name": "browser_history",
        "description": "Search the selected Conduit browser profile's full navigation history, newest first.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Optional case-insensitive text to match against the URL or page title."
                },
                "before": {
                    "type": "string",
                    "description": "Opaque continuation cursor returned by an earlier browser_history call."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 200,
                    "default": 50,
                    "description": "Maximum entries to return."
                }
            },
            "additionalProperties": false
        },
        "annotations": {
            "title": "Browser history",
            "readOnlyHint": true,
            "destructiveHint": false,
            "openWorldHint": false
        }
    }))
    .expect("the browser history tool schema must be valid")
}
