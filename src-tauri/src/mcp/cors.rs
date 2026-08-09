//! Origin policy for the MCP endpoint.
//!
//! ## Why this is off by default
//!
//! conduit's endpoint has no authentication. The security model is that it is
//! reachable only from this machine, and that quitting the app revokes it.
//!
//! A browser breaks that assumption. Any page you visit can issue requests to
//! `127.0.0.1` — it just can't *read* the replies without CORS. For most APIs
//! that is enough protection, because reading the response is the point. It is
//! not enough here: `run_shell`, `click` and `type_text` do their damage on the
//! way in. A wildcard `Access-Control-Allow-Origin` on this server would be a
//! drive-by remote-code-execution hole, not a convenience.
//!
//! What actually protects conduit today is that rmcp requires
//! `Content-Type: application/json`, which is not a CORS "simple" content type,
//! so every cross-origin attempt must first pass a preflight — and an
//! unconfigured conduit answers none.
//!
//! So: opt in, per origin, from the Settings tab. Nothing is allowed until the
//! user names it.
//!
//! ## What this adds beyond CORS
//!
//! A request carrying a disallowed `Origin` is **rejected**, not merely denied
//! the response headers. The MCP specification requires servers on an HTTP
//! transport to validate `Origin` to defend against DNS rebinding, and refusing
//! outright is the only thing that stops a request whose author never intended
//! to read the answer.
//!
//! Requests with no `Origin` header at all — every native client, Claude Code,
//! Codex, curl — are untouched by any of this.

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::state::{Settings, Shared};

/// Headers a browser MCP client needs to send.
const ALLOW_HEADERS: &str =
    "content-type, accept, authorization, last-event-id, mcp-session-id, mcp-protocol-version";

/// Headers it needs to *read back*.
///
/// `Mcp-Session-Id` is the load-bearing one: the streamable-HTTP transport
/// hands the session out in that header, and a browser cannot see it unless it
/// is explicitly exposed. Without this line a client connects, gets a 200, and
/// then fails on every following request with no obvious cause.
const EXPOSE_HEADERS: &str = "mcp-session-id";

const ALLOW_METHODS: &str = "GET, POST, DELETE, OPTIONS";

/// Normalizes an origin for comparison.
///
/// Browsers send `http://127.0.0.1:8080` with no trailing slash, but people
/// type the address bar's version, which has one. Treating those as different
/// origins would be a maddening five minutes for the user.
fn normalize(origin: &str) -> String {
    origin.trim().trim_end_matches('/').to_ascii_lowercase()
}

/// Whether `origin` is on the user's allowlist.
///
/// Takes `Settings` rather than `Shared` so the decision — the part with
/// security consequences — is a pure function that can be tested exhaustively
/// without standing up a Tauri app handle.
pub(crate) fn origin_allowed(settings: &Settings, origin: &str) -> bool {
    if !settings.cors_enabled {
        return false;
    }
    let wanted = normalize(origin);
    settings
        .cors_origins
        .iter()
        .any(|allowed| normalize(allowed) == wanted)
}

fn apply_headers(response: &mut Response, origin: &HeaderValue) {
    let headers = response.headers_mut();

    // Echo the specific origin rather than `*`. A wildcard would also apply to
    // origins the user never named.
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static(ALLOW_METHODS),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static(ALLOW_HEADERS),
    );
    headers.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static(EXPOSE_HEADERS),
    );
    headers.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("600"),
    );
    // The response differs by origin, so anything caching it must key on that.
    headers.insert(header::VARY, HeaderValue::from_static("Origin"));

    // Deliberately no `Access-Control-Allow-Credentials`. conduit has no
    // cookies or auth to send, and enabling it would let a page attach the
    // user's ambient credentials to these requests.
}

pub async fn guard(State(state): State<Shared>, request: Request<Body>, next: Next) -> Response {
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    // No Origin means a native client. This is the overwhelmingly common path
    // and it must behave exactly as if this middleware did not exist.
    let Some(origin) = origin else {
        return next.run(request).await;
    };

    if !origin_allowed(&state.settings(), &origin) {
        tracing::warn!(%origin, "refused a request from a browser origin that is not allowed");
        return (
            StatusCode::FORBIDDEN,
            format!(
                "conduit refused a request from {origin}. browser clients are blocked by default \
                 because this endpoint can drive the whole machine. to allow this one, open \
                 conduit → settings, turn on browser access and add {origin} to the allowlist."
            ),
        )
            .into_response();
    }

    // Safe to unwrap into a header value: it came out of one.
    let header_origin = HeaderValue::from_str(&origin)
        .unwrap_or_else(|_| HeaderValue::from_static("null"));

    // Preflight is answered here rather than passed down: rmcp's router only
    // knows GET, POST and DELETE, so an OPTIONS would 405 and the browser would
    // never send the real request.
    if request.method() == Method::OPTIONS {
        let mut response = StatusCode::NO_CONTENT.into_response();
        apply_headers(&mut response, &header_origin);
        return response;
    }

    let mut response = next.run(request).await;
    apply_headers(&mut response, &header_origin);
    response
}

#[cfg(test)]
mod tests {
    use super::{normalize, origin_allowed};
    use crate::state::Settings;

    fn settings(enabled: bool, origins: &[&str]) -> Settings {
        Settings {
            cors_enabled: enabled,
            cors_origins: origins.iter().map(|s| s.to_string()).collect(),
            ..Settings::default()
        }
    }

    /// The default must refuse everything. This is the line between "a local
    /// tool" and "any website can run shell commands on this machine", so it
    /// gets a test of its own rather than relying on the field's initialiser.
    #[test]
    fn nothing_is_allowed_by_default() {
        let s = Settings::default();
        assert!(!s.cors_enabled);
        assert!(s.cors_origins.is_empty());
        assert!(!origin_allowed(&s, "http://127.0.0.1:8080"));
        assert!(!origin_allowed(&s, "https://example.com"));
    }

    #[test]
    fn the_switch_and_the_list_must_both_agree() {
        // Listed but switched off.
        assert!(!origin_allowed(
            &settings(false, &["http://127.0.0.1:8080"]),
            "http://127.0.0.1:8080"
        ));
        // Switched on but nothing listed — an empty allowlist is not a wildcard.
        assert!(!origin_allowed(&settings(true, &[]), "http://127.0.0.1:8080"));
        // Both.
        assert!(origin_allowed(
            &settings(true, &["http://127.0.0.1:8080"]),
            "http://127.0.0.1:8080"
        ));
    }

    #[test]
    fn allowing_one_origin_does_not_allow_a_neighbour() {
        let s = settings(true, &["http://127.0.0.1:8080"]);
        for other in [
            "http://127.0.0.1:8081",
            "https://127.0.0.1:8080",
            "http://localhost:8080",
            "http://evil.example",
            "null",
        ] {
            assert!(!origin_allowed(&s, other), "{other} should not be allowed");
        }
    }

    #[test]
    fn a_trailing_slash_or_different_case_still_matches() {
        let s = settings(true, &["http://127.0.0.1:8080/"]);
        assert!(origin_allowed(&s, "http://127.0.0.1:8080"));
        assert!(origin_allowed(&s, "HTTP://127.0.0.1:8080"));
    }

    #[test]
    fn origins_compare_regardless_of_trailing_slash_or_case() {
        assert_eq!(normalize("http://127.0.0.1:8080/"), normalize("http://127.0.0.1:8080"));
        assert_eq!(normalize("HTTP://LocalHost:6274"), normalize("http://localhost:6274"));
        assert_eq!(normalize("  http://localhost:8080  "), "http://localhost:8080");
    }

    #[test]
    fn different_ports_and_schemes_stay_different() {
        assert_ne!(normalize("http://127.0.0.1:8080"), normalize("http://127.0.0.1:8081"));
        assert_ne!(normalize("http://127.0.0.1:8080"), normalize("https://127.0.0.1:8080"));
        // localhost and 127.0.0.1 are genuinely different origins to a browser,
        // so conduit must not quietly treat them as one.
        assert_ne!(normalize("http://localhost:8080"), normalize("http://127.0.0.1:8080"));
    }
}
