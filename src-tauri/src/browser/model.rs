use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const EXPECTED_REVISION: &str = "playwright-mcp-0.0.80-chromium-1243";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BrowserMode {
    #[default]
    Headless,
    Visible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum BrowserInstallStatus {
    #[default]
    Unavailable,
    Downloading,
    Verifying,
    Installing,
    Ready,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum BrowserRunStatus {
    #[default]
    Stopped,
    Starting,
    Running,
    Stopping,
    Paused,
    Crashed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BrowserRestartStrategy {
    Fresh,
    ReopenUrls,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserInstallState {
    pub status: BrowserInstallStatus,
    pub expected_revision: String,
    pub installed_revision: Option<String>,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub error: Option<String>,
}

impl Default for BrowserInstallState {
    fn default() -> Self {
        Self {
            status: BrowserInstallStatus::Unavailable,
            expected_revision: EXPECTED_REVISION.into(),
            installed_revision: None,
            downloaded_bytes: 0,
            total_bytes: None,
            error: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserProfile {
    pub id: String,
    pub name: String,
    pub incognito: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTab {
    pub index: usize,
    pub title: String,
    pub url: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserDownload {
    pub id: String,
    pub filename: String,
    pub source_url: String,
    pub path: Option<String>,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserPreviewFrame {
    pub data: String,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserHistoryEntry {
    pub id: String,
    pub profile_id: String,
    pub url: String,
    pub title: String,
    pub visited_at: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserState {
    pub install: BrowserInstallState,
    pub run_status: BrowserRunStatus,
    pub mode: BrowserMode,
    pub selected_profile_id: String,
    pub profiles: Vec<BrowserProfile>,
    pub tabs: Vec<BrowserTab>,
    pub downloads: Vec<BrowserDownload>,
    pub preview: Option<BrowserPreviewFrame>,
    pub stop_latched: bool,
    pub owner: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeManifest {
    pub revision: String,
    pub target: String,
    pub archive_url: String,
    pub size: u64,
    pub sha256: String,
    pub sidecar_path: String,
    pub chromium_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowserDiskState {
    pub mode: BrowserMode,
    pub selected_profile_id: String,
    pub profiles: Vec<BrowserProfile>,
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimePaths {
    pub sidecar: PathBuf,
    pub chromium: PathBuf,
    pub revision: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SidecarEvent {
    pub event: String,
    #[serde(default)]
    pub tabs: Vec<BrowserTab>,
    #[serde(default)]
    pub frame: Option<BrowserPreviewFrame>,
    #[serde(default)]
    pub history: Option<BrowserHistoryEntry>,
    #[serde(default)]
    pub download: Option<BrowserDownload>,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}
