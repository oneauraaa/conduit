//! Publishing the MCP endpoint to the internet through Tailscale Funnel.
//!
//! ## Why a second listener
//!
//! Funnel proxies `https://<host>.ts.net/` to a local port, and the proxied
//! request arrives from 127.0.0.1 — indistinguishable from a genuinely local
//! one. So there is no reliable way to keep `/mcp` on the main port open to
//! local agents while denying the public.
//!
//! Instead conduit runs a *separate* listener for sharing, on its own port,
//! whose only route sits behind a secret path. Funnel points at that port. The
//! main loopback port is never exposed, and the public surface is exactly one
//! unguessable URL. Turning sharing off drops the listener entirely.

use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;

/// Where the Tailscale CLI tends to live. The Mac App Store build hides it
/// inside the bundle; the standalone installer and Homebrew put it on PATH.
const CLI_CANDIDATES: &[&str] = &[
    "/usr/local/bin/tailscale",
    "/opt/homebrew/bin/tailscale",
    "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TailscaleState {
    /// The CLI was found on this machine.
    pub installed: bool,
    /// tailscaled is up and logged in.
    pub connected: bool,
    /// This machine's MagicDNS name, e.g. `mac-studio.tail1234.ts.net`.
    pub hostname: Option<String>,
    /// Whether conduit currently has a funnel running.
    pub sharing: bool,
    /// The full public MCP URL while sharing.
    pub public_url: Option<String>,
    /// Populated when a Tailscale command failed, verbatim.
    pub error: Option<String>,
}

impl TailscaleState {
    fn absent() -> Self {
        Self {
            installed: false,
            connected: false,
            hostname: None,
            sharing: false,
            public_url: None,
            error: None,
        }
    }
}

pub fn cli() -> Option<PathBuf> {
    for c in CLI_CANDIDATES {
        let p = PathBuf::from(c);
        if p.exists() {
            return Some(p);
        }
    }
    // Fall back to PATH, for unusual installs.
    let out = Command::new("/usr/bin/which").arg("tailscale").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

fn run(cli: &PathBuf, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cli)
        .args(args)
        .output()
        .map_err(|e| format!("could not run tailscale: {e}"))?;

    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        // Tailscale's own message is far more useful than anything we'd invent
        // (not logged in, funnel disabled for the tailnet, HTTPS not enabled…).
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let msg = if err.is_empty() {
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        } else {
            err
        };
        Err(msg)
    }
}

/// Current Tailscale state, plus whether conduit's funnel is up.
pub fn state(remote_port: u16, token: &str) -> TailscaleState {
    let Some(cli) = cli() else {
        return TailscaleState::absent();
    };

    let mut st = TailscaleState {
        installed: true,
        ..TailscaleState::absent()
    };

    match run(&cli, &["status", "--json"]) {
        Ok(json) => {
            let parsed: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
            let backend = parsed
                .get("BackendState")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            st.connected = backend == "Running";

            // DNSName arrives with a trailing dot, which would produce a URL
            // like https://host.ts.net./mcp — valid but alarming to look at.
            st.hostname = parsed
                .get("Self")
                .and_then(|s| s.get("DNSName"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim_end_matches('.').to_string());
        }
        Err(e) => st.error = Some(e),
    }

    if let Ok(status) = run(&cli, &["funnel", "status"]) {
        st.sharing = status.contains(&format!("127.0.0.1:{remote_port}"))
            || status.contains(&format!("localhost:{remote_port}"));
    }

    if st.sharing {
        st.public_url = st
            .hostname
            .as_ref()
            .map(|h| public_url(h, token));
    }

    st
}

pub fn public_url(hostname: &str, token: &str) -> String {
    format!("https://{hostname}/{token}/mcp")
}

/// Starts a background funnel pointing at conduit's sharing listener.
pub fn start_funnel(remote_port: u16) -> Result<(), String> {
    let cli = cli().ok_or("tailscale is not installed")?;
    run(&cli, &["funnel", "--bg", &remote_port.to_string()]).map(|_| ())
}

/// Tears the funnel down. Targets conduit's port specifically so any other
/// funnel the user has running is left alone.
pub fn stop_funnel(remote_port: u16) -> Result<(), String> {
    let cli = cli().ok_or("tailscale is not installed")?;
    let port = remote_port.to_string();

    // Newer CLIs take `funnel --bg <port> off`; older ones want `funnel off`.
    // Try the specific form first so we don't clear unrelated funnels.
    match run(&cli, &["funnel", "--bg", &port, "off"]) {
        Ok(_) => Ok(()),
        Err(specific) => run(&cli, &["funnel", "off"])
            .map(|_| ())
            .map_err(|general| format!("{specific}; then: {general}")),
    }
}

/// A fresh 32-character hex secret.
///
/// Read straight from the system CSPRNG rather than pulling in an RNG crate —
/// this is the only random value conduit needs, and `/dev/urandom` on macOS is
/// exactly what a crate would end up calling.
pub fn generate_token() -> String {
    use std::io::Read;

    let mut bytes = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        if f.read_exact(&mut bytes).is_ok() {
            return bytes.iter().map(|b| format!("{b:02x}")).collect();
        }
    }

    // Should never happen on macOS. Better to fail loudly than to hand out a
    // predictable key that looks legitimate.
    panic!("could not read /dev/urandom to generate a sharing key");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn tokens_are_long_hex_and_unique() {
        let mut seen = HashSet::new();
        for _ in 0..64 {
            let t = generate_token();
            assert_eq!(t.len(), 32, "token should be 128 bits of hex");
            assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
            // A URL path segment is the whole guard, so collisions would be a
            // security bug, not just a nuisance.
            assert!(seen.insert(t), "generate_token repeated a value");
        }
    }

    #[test]
    fn public_url_puts_the_token_in_the_path() {
        let url = public_url("mac.tail1.ts.net", "abc123");
        assert_eq!(url, "https://mac.tail1.ts.net/abc123/mcp");
    }

    /// The sharing listener must never collide with the local one.
    #[test]
    fn remote_port_is_distinct_from_local() {
        let local = crate::state::DEFAULT_PORT;
        assert_ne!(crate::state::remote_port(local), local);
    }
}
