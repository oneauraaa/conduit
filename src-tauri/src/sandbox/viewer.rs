//! The Sandbox tab's live view: a VNC stream carried over Tauri IPC.
//!
//! The desktop's VNC server listens only on the container's own loopback, and
//! nothing publishes it. Each viewer is instead one `docker exec -i … socat -
//! TCP:127.0.0.1:5901` whose stdin and stdout *are* the VNC connection. Bytes
//! from the sandbox go to the webview on an IPC [`Channel`] (which preserves
//! order); bytes from noVNC come back through `sandbox_viewer_send`.
//!
//! Compared with publishing a port for a WebSocket bridge, this opens nothing
//! on the host, needs no VNC password, needs no CSP change, and keeps working
//! with the sandbox's network turned off entirely.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use parking_lot::Mutex;
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::ChildStdin;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use super::docker::{self, DockerCli};

/// Largest chunk forwarded to the webview at once. Channel payloads over a
/// kilobyte already take the fetch path, so batching up to here costs nothing
/// per byte and saves a round trip per framebuffer tile.
const CHUNK: usize = 64 * 1024;
/// A viewer that sends more than this in one call is not noVNC.
const MAX_SEND: usize = 1024 * 1024;

struct Viewer {
    sandbox: String,
    stdin: AsyncMutex<ChildStdin>,
    cancel: CancellationToken,
}

#[derive(Default)]
pub struct Viewers {
    open: Mutex<HashMap<String, Arc<Viewer>>>,
}

impl Viewers {
    /// Connects a new viewer to sandbox `id`'s display and returns its handle.
    /// The stream ends — and an empty message is sent — when the sandbox
    /// stops, the viewer is closed, or socat exits.
    pub fn open(
        &self,
        cli: Arc<DockerCli>,
        id: &str,
        on_data: Channel<InvokeResponseBody>,
    ) -> Result<String, String> {
        let argv = docker::args(&["socat", "-", "TCP:127.0.0.1:5901"]);
        let mut child = cli
            .command()
            .args(docker::exec_args(&docker::container_name(id), true, &argv))
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not open the live view: {e}"))?;
        let stdin = child.stdin.take().ok_or("the live view has no input")?;
        let mut stdout = child.stdout.take().ok_or("the live view has no output")?;

        let handle = crate::random::token_hex();
        let cancel = CancellationToken::new();
        self.open.lock().insert(
            handle.clone(),
            Arc::new(Viewer {
                sandbox: id.to_string(),
                stdin: AsyncMutex::new(stdin),
                cancel: cancel.clone(),
            }),
        );

        tauri::async_runtime::spawn(async move {
            let mut buf = vec![0u8; CHUNK];
            loop {
                tokio::select! {
                    read = stdout.read(&mut buf) => match read {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if on_data.send(InvokeResponseBody::Raw(buf[..n].to_vec())).is_err() {
                                break;
                            }
                        }
                    },
                    _ = cancel.cancelled() => break,
                }
            }
            let _ = child.kill().await;
            // Zero bytes is the end-of-stream signal the webview listens for.
            let _ = on_data.send(InvokeResponseBody::Raw(Vec::new()));
        });
        Ok(handle)
    }

    pub async fn send(&self, handle: &str, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > MAX_SEND {
            return Err("that is too much input for the live view".into());
        }
        let viewer = self
            .open
            .lock()
            .get(handle)
            .cloned()
            .ok_or("the live view is closed")?;
        let mut stdin = viewer.stdin.lock().await;
        stdin
            .write_all(bytes)
            .await
            .map_err(|e| format!("the live view dropped: {e}"))?;
        stdin
            .flush()
            .await
            .map_err(|e| format!("the live view dropped: {e}"))
    }

    pub fn close(&self, handle: &str) {
        if let Some(viewer) = self.open.lock().remove(handle) {
            viewer.cancel.cancel();
        }
    }

    /// Closes every viewer of sandbox `id` — on stop and delete.
    pub fn close_sandbox(&self, id: &str) {
        self.open.lock().retain(|_, viewer| {
            let keep = viewer.sandbox != id;
            if !keep {
                viewer.cancel.cancel();
            }
            keep
        });
    }
}
