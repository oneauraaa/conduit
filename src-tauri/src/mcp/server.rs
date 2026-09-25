//! MCP server lifecycle.
//!
//! rmcp's `StreamableHttpService` is a tower service, so it mounts as an axum
//! route. The whole thing runs as one tokio task owned by [`crate::state`] and
//! is torn down by cancelling its token — which is what makes "quit conduit"
//! genuinely revoke every agent's access rather than just hiding a window.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use tokio_util::sync::CancellationToken;

use crate::mcp::cors;
use crate::mcp::sandbox_tools::SandboxConduit;
use crate::mcp::tools::Conduit;
use crate::state::{ServerStatus, Shared, now_millis};

type SandboxService = StreamableHttpService<SandboxConduit, LocalSessionManager>;

/// One MCP service per sandbox, made on first use and cached.
///
/// Separate services rather than one that reads the id from the path: rmcp
/// keys sessions by the `Mcp-Session-Id` header alone, so with a shared
/// service a session opened on `/sandbox/a/mcp` could be replayed against
/// `/sandbox/b/mcp`. Each service's token is a child of the server's, so
/// stopping the server ends every sandbox session with it, and deleting a
/// sandbox ends just its own.
pub struct SandboxServices {
    state: Shared,
    parent: CancellationToken,
    services: parking_lot::Mutex<HashMap<String, (CancellationToken, SandboxService)>>,
}

impl SandboxServices {
    fn new(state: Shared, parent: CancellationToken) -> Self {
        Self {
            state,
            parent,
            services: parking_lot::Mutex::new(HashMap::new()),
        }
    }

    fn get_or_create(&self, id: &str) -> Option<SandboxService> {
        if !self.state.sandboxes.exists(id) {
            self.remove(id);
            return None;
        }
        let mut services = self.services.lock();
        if let Some((_, service)) = services.get(id) {
            return Some(service.clone());
        }
        let token = self.parent.child_token();
        let (state, sandbox) = (self.state.clone(), id.to_string());
        let service = StreamableHttpService::new(
            move || Ok(SandboxConduit::new(state.clone(), sandbox.clone())),
            LocalSessionManager::default().into(),
            StreamableHttpServerConfig::default().with_cancellation_token(token.clone()),
        );
        services.insert(id.to_string(), (token, service.clone()));
        Some(service)
    }

    /// Ends every session on sandbox `id` and forgets its service.
    pub fn remove(&self, id: &str) {
        if let Some((token, _)) = self.services.lock().remove(id) {
            token.cancel();
        }
    }
}

async fn sandbox_mcp(
    State(services): State<Arc<SandboxServices>>,
    Path(id): Path<String>,
    request: axum::extract::Request,
) -> axum::response::Response {
    let service = crate::sandbox::model::valid_id(&id)
        .then(|| services.get_or_create(&id))
        .flatten();
    match service {
        Some(service) => service.handle(request).await.into_response(),
        None => (
            axum::http::StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({
                "error": format!("conduit has no sandbox called {id:?}. the Sandbox tab lists them."),
            })),
        )
            .into_response(),
    }
}

pub async fn start(state: Shared) {
    if matches!(
        state.server().status,
        ServerStatus::Running | ServerStatus::Starting
    ) {
        return;
    }

    let port = state.settings().port;
    state.update_server(|s| {
        s.status = ServerStatus::Starting;
        s.port = port;
        s.last_error = None;
    });

    // Loopback only. conduit can drive the whole machine, so the endpoint must
    // never be reachable from the network.
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            let message = if e.kind() == std::io::ErrorKind::AddrInUse {
                format!("port {port} is already in use — pick another in the field above")
            } else {
                format!("could not bind 127.0.0.1:{port}: {e}")
            };
            state.update_server(|s| {
                s.status = ServerStatus::Error;
                s.last_error = Some(message);
            });
            return;
        }
    };

    let cancel = CancellationToken::new();
    *state.server_cancel.write() = Some(cancel.clone());

    let handler_state = state.clone();
    let service = StreamableHttpService::new(
        move || Ok(Conduit::new(handler_state.clone())),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default().with_cancellation_token(cancel.child_token()),
    );

    let sandboxes = Arc::new(SandboxServices::new(state.clone(), cancel.child_token()));
    *state.sandbox_services.write() = Some(sandboxes.clone());
    state.sandboxes.set_port(port);

    // Every request passes the origin guard first. Native clients send no
    // Origin and are unaffected; browser clients are refused unless the user
    // has allowed them in Settings. See `mcp/cors.rs` for why this is not a
    // permissive CORS layer. The layer goes last so it wraps both routes —
    // axum only layers the routes added before it.
    let router = axum::Router::new()
        .nest_service("/mcp", service)
        .route("/sandbox/{id}/mcp", axum::routing::any(sandbox_mcp))
        .with_state(sandboxes)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            cors::guard,
        ));

    state.update_server(|s| {
        s.status = ServerStatus::Running;
        s.started_at = Some(now_millis());
    });

    let serve_state = state.clone();
    tauri::async_runtime::spawn(async move {
        let shutdown = cancel.clone();
        let result = axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown.cancelled().await })
            .await;

        if let Err(e) = result {
            serve_state.update_server(|s| {
                s.status = ServerStatus::Error;
                s.last_error = Some(e.to_string());
                s.started_at = None;
            });
        } else {
            serve_state.update_server(|s| {
                s.status = ServerStatus::Stopped;
                s.started_at = None;
            });
        }
    });
}

pub async fn stop(state: Shared) {
    if let Some(cancel) = state.server_cancel.write().take() {
        cancel.cancel();
    }
    *state.sandbox_services.write() = None;
    // Any session in flight loses its tools; drop the control chrome with it.
    state.end_control();
    state.update_server(|s| {
        s.status = ServerStatus::Stopped;
        s.started_at = None;
        s.last_error = None;
    });
}

pub async fn restart(state: Shared) {
    stop(state.clone()).await;
    // Give the listener a moment to release the port before rebinding.
    tokio::time::sleep(std::time::Duration::from_millis(180)).await;
    start(state).await;
}

/* ── the sharing listener ───────────────────────────────────── */

/// Brings up the second listener that Tailscale Funnel points at.
///
/// Its only route is `/{token}/mcp`. Everything else 404s, so the public
/// surface is one unguessable URL and the main loopback port is never exposed.
pub async fn start_remote(state: Shared) -> Result<(), String> {
    if state.remote_cancel.read().is_some() {
        return Ok(());
    }

    let settings = state.settings();
    let port = crate::state::remote_port(settings.port);
    let token = settings.remote_token.clone();

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("could not bind the sharing port {port}: {e}"))?;

    let cancel = CancellationToken::new();
    *state.remote_cancel.write() = Some(cancel.clone());

    let handler_state = state.clone();
    let service = StreamableHttpService::new(
        move || Ok(Conduit::new(handler_state.clone())),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default().with_cancellation_token(cancel.child_token()),
    );

    // The same policy guards the shared listener. The secret path is what
    // makes this endpoint private; the origin allowlist is what keeps a web
    // page from using it on the user's behalf.
    let router = axum::Router::new()
        .nest_service(&format!("/{token}/mcp"), service)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            cors::guard,
        ));

    tauri::async_runtime::spawn(async move {
        let shutdown = cancel.clone();
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown.cancelled().await })
            .await;
    });

    Ok(())
}

pub fn stop_remote(state: &Shared) {
    if let Some(cancel) = state.remote_cancel.write().take() {
        cancel.cancel();
    }
}
