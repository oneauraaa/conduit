use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex as AsyncMutex};

use super::model::{BrowserMode, BrowserProfile, RuntimePaths, SidecarEvent};
use super::BrowserManager;

// A click can spend up to 60 seconds awaiting a discovered navigation or
// download approval and then up to 60 seconds navigating.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(130);
type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value, String>>>>>;

pub(crate) struct SidecarProcess {
    id: String,
    writer: Arc<AsyncMutex<ChildStdin>>,
    child: Arc<AsyncMutex<Child>>,
    pending: Pending,
    sequence: AtomicU64,
}

impl std::fmt::Debug for SidecarProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SidecarProcess").finish_non_exhaustive()
    }
}

impl SidecarProcess {
    /// Starts the private sidecar process without launching Chromium yet.
    ///
    /// The manager registers this handle before [`Self::initialize`] so Stop
    /// and Panic Stop can kill a browser that is still in its launch sequence.
    pub(crate) fn spawn(
        manager: Arc<BrowserManager>,
        executable: &std::path::Path,
    ) -> Result<Self, String> {
        let process_id = crate::random::uuid_v4();
        let mut child = Command::new(executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("could not start the Chromium sidecar: {e}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or("the Chromium sidecar has no input pipe")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("the Chromium sidecar has no output pipe")?;
        let stderr = child
            .stderr
            .take()
            .ok_or("the Chromium sidecar has no error pipe")?;
        let writer = Arc::new(AsyncMutex::new(stdin));
        let child = Arc::new(AsyncMutex::new(child));
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));

        let reader_manager = manager.clone();
        let reader_writer = writer.clone();
        let reader_pending = pending.clone();
        let reader_process_id = process_id.clone();
        tauri::async_runtime::spawn(async move {
            read_stdout(
                stdout,
                reader_manager,
                reader_writer,
                reader_pending,
                reader_process_id,
            )
            .await;
        });
        tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::warn!("Chromium sidecar: {line}");
            }
        });

        Ok(Self {
            id: process_id,
            writer,
            child,
            pending,
            sequence: AtomicU64::new(1),
        })
    }

    pub(crate) async fn initialize(
        &self,
        runtime: RuntimePaths,
        profile: BrowserProfile,
        profile_dir: std::path::PathBuf,
        output_dir: std::path::PathBuf,
        mode: BrowserMode,
    ) -> Result<(), String> {
        self.request(json!({
            "type": "handshake",
            "protocol": 1,
            "revision": runtime.revision,
        }))
        .await?;
        self.request(json!({
            "type": "launch",
            "chromium": runtime.chromium,
            "profileId": profile.id,
            "userDataDir": profile_dir,
            "outputDir": output_dir,
            "incognito": profile.incognito,
            "headless": mode == BrowserMode::Headless,
            "viewport": { "width": 1280, "height": 720 },
        }))
        .await?;
        Ok(())
    }

    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
        agent: Option<&str>,
        direct_grant: Option<&str>,
    ) -> Result<Value, String> {
        let grants: Vec<&str> = direct_grant.into_iter().collect();
        self.request(json!({
            "type": "callTool",
            "name": name,
            "arguments": arguments,
            "agent": agent,
            "grants": grants,
        }))
        .await
    }

    pub(crate) async fn set_preview(&self, visible: bool) -> Result<(), String> {
        self.request(json!({ "type": "setPreview", "visible": visible }))
            .await
            .map(|_| ())
    }

    pub(crate) async fn stop(&self) {
        let _ = tokio::time::timeout(
            Duration::from_secs(3),
            self.request(json!({ "type": "stop" })),
        )
        .await;
        let _ = self.child.lock().await.kill().await;
    }

    pub(crate) async fn kill(&self) {
        let _ = self.child.lock().await.kill().await;
    }

    async fn request(&self, mut body: Value) -> Result<Value, String> {
        let id = format!("sidecar-{}", self.sequence.fetch_add(1, Ordering::Relaxed));
        body.as_object_mut()
            .ok_or("internal sidecar request was not an object")?
            .insert("id".into(), Value::String(id.clone()));
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(id.clone(), tx);
        if let Err(error) = write_message(&self.writer, &body).await {
            self.pending.lock().remove(&id);
            return Err(error);
        }
        match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("the Chromium sidecar stopped before replying".into()),
            Err(_) => {
                self.pending.lock().remove(&id);
                Err("the Chromium sidecar timed out".into())
            }
        }
    }
}

async fn read_stdout(
    stdout: tokio::process::ChildStdout,
    manager: Arc<BrowserManager>,
    writer: Arc<AsyncMutex<ChildStdin>>,
    pending: Pending,
    process_id: String,
) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!("ignored invalid Chromium sidecar message: {error}");
                continue;
            }
        };
        if let Some(id) = value.get("id").and_then(Value::as_str) {
            if let Some(sender) = pending.lock().remove(id) {
                let result = if value.get("ok").and_then(Value::as_bool) == Some(true) {
                    Ok(value.get("result").cloned().unwrap_or(Value::Null))
                } else {
                    Err(value
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("the Chromium sidecar rejected the request")
                        .to_string())
                };
                let _ = sender.send(result);
            }
            continue;
        }
        let event: SidecarEvent = match serde_json::from_value(value) {
            Ok(event) => event,
            Err(error) => {
                tracing::warn!("ignored malformed Chromium sidecar event: {error}");
                continue;
            }
        };
        if event.event == "permission" {
            let permission_manager = manager.clone();
            let permission_writer = writer.clone();
            tauri::async_runtime::spawn(async move {
                let request_id = event.request_id.unwrap_or_default();
                let allowed = permission_manager
                    .approve_side_effect(
                        event.category.as_deref().unwrap_or_default(),
                        event.detail,
                        event.agent,
                        event.tool.as_deref(),
                    )
                    .await;
                let response = json!({
                    "type": "permissionResult",
                    "requestId": request_id,
                    "allowed": allowed,
                });
                let _ = write_message(&permission_writer, &response).await;
            });
        } else {
            manager.handle_sidecar_event(event);
        }
    }
    for (_, sender) in pending.lock().drain() {
        let _ = sender.send(Err("the Chromium sidecar exited".into()));
    }
    manager.sidecar_exited(&process_id).await;
}

async fn write_message(writer: &Arc<AsyncMutex<ChildStdin>>, value: &Value) -> Result<(), String> {
    let mut line =
        serde_json::to_vec(value).map_err(|e| format!("could not encode sidecar request: {e}"))?;
    line.push(b'\n');
    let mut writer = writer.lock().await;
    writer
        .write_all(&line)
        .await
        .map_err(|e| format!("could not write to Chromium sidecar: {e}"))?;
    writer
        .flush()
        .await
        .map_err(|e| format!("could not flush Chromium sidecar request: {e}"))
}
