//! Driving KWin through its scripting engine.
//!
//! ## Why this exists at all
//!
//! On X11 any client could enumerate, move and raise any window. Wayland
//! removed that wholesale: there is no protocol by which one client learns
//! another's geometry, let alone changes it. `ext-foreign-toplevel-list-v1`
//! gets as far as titles and app ids, and deliberately stops short of bounds.
//!
//! Window management on Wayland is therefore a *compositor* feature, reached
//! through whatever door that compositor opens. KWin's door is its JavaScript
//! engine, which runs inside KWin with full access to the window list and is
//! addressable over D-Bus. That is the entire reason `list_windows` and
//! `set_window_bounds` work on Plasma and report "not supported here"
//! elsewhere — it is not an oversight in the port, it is the shape of the
//! platform.
//!
//! ## How a call works
//!
//! KWin's scripting API can load a script and run it, but has no way to return
//! a value. The script talks back out over D-Bus instead: conduit owns a bus
//! name, the generated script ends in a `callDBus(...)` to it, and
//! [`run_script`] blocks on that callback.
//!
//! Two traps, both of which cost an afternoon:
//!
//!   - **The method name is PascalCase on the wire.** `#[zbus::interface]`
//!     renames `fn report` to `Report`, and KWin's `callDBus` passes the name
//!     through verbatim. Calling `"report"` fails completely silently — the
//!     script runs, the call goes nowhere, and the only symptom is a timeout.
//!   - **A script name stays registered until unloaded.** Loading the same name
//!     twice is refused, so every call unloads first and unloads after, even on
//!     the error path.
//!
//! Parameters go *into* the script by being baked into its source text before
//! it is written to disk — there is no argument channel either. Everything
//! interpolated goes through [`js_string`].

use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use parking_lot::Mutex;
use zbus::blocking::{Connection, Proxy};

/// How long to wait for a script's callback. KWin runs these on its own event
/// loop, which is also the compositor's — a script that has not answered in
/// this long is not going to, and blocking a tool call further is worse than
/// reporting failure.
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(5);

/// The bus name conduit serves the callback on.
///
/// Suffixed with the pid so a second instance — which the single-instance
/// plugin should prevent, but which a developer running `cargo tauri dev`
/// alongside a release build will absolutely produce — does not fail to claim
/// the name and lose window management entirely.
fn bus_name() -> String {
    format!("ai.conduit.kwin.p{}", std::process::id())
}

/* ── the callback service ─────────────────────────────────────── */

struct Bridge {
    /// Held only to keep the bus name claimed for the life of the process.
    _conn: Connection,
    rx: Mutex<Receiver<String>>,
    /// Serializes calls, so one script's callback cannot be read by another.
    turn: Mutex<()>,
    name: String,
}

struct Callback {
    tx: std::sync::Mutex<Sender<String>>,
}

#[zbus::interface(name = "ai.conduit.KWin")]
impl Callback {
    /// Called by the generated script. Named to serialize as `Report` — see the
    /// module header.
    fn report(&self, payload: String) {
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.send(payload);
        }
    }
}

static BRIDGE: OnceLock<Result<Bridge, String>> = OnceLock::new();

fn bridge() -> Result<&'static Bridge, String> {
    BRIDGE
        .get_or_init(|| {
            let (tx, rx) = channel();
            let name = bus_name();

            let conn = zbus::blocking::connection::Builder::session()
                .map_err(|e| format!("no session bus: {e}"))?
                .name(name.as_str())
                .map_err(|e| format!("could not claim {name}: {e}"))?
                .serve_at(
                    "/",
                    Callback {
                        tx: std::sync::Mutex::new(tx),
                    },
                )
                .map_err(|e| format!("could not serve the kwin bridge: {e}"))?
                .build()
                .map_err(|e| format!("could not start the kwin bridge: {e}"))?;

            Ok(Bridge {
                _conn: conn,
                rx: Mutex::new(rx),
                turn: Mutex::new(()),
                name,
            })
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// Whether this desktop is one whose windows conduit can manage.
///
/// Checked by name rather than by trying, because the readiness card asks on a
/// poll and loading a script to find out would be absurd.
pub fn available() -> bool {
    let Ok(conn) = Connection::session() else {
        return false;
    };
    let Ok(dbus) = Proxy::new(
        &conn,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    ) else {
        return false;
    };
    dbus.call::<_, _, bool>("NameHasOwner", &("org.kde.KWin",))
        .unwrap_or(false)
}

/// What to tell an agent on a desktop this cannot drive.
pub fn unsupported(what: &str) -> String {
    format!(
        "{what} needs the compositor's cooperation, and only kwin (kde plasma) \
         exposes it. wayland has no protocol that lets one application read or \
         change another's window geometry, so this tool is unavailable on this \
         desktop. screenshots, clicking, typing and reading the screen all still work."
    )
}

/* ── running a script ─────────────────────────────────────────── */

/// Runs `body` inside KWin and returns whatever it reported back.
///
/// `body` must end by calling `report(...)` — the helper the wrapper defines —
/// exactly once. A script that never reports times out.
pub fn run_script(body: &str) -> Result<String, String> {
    if !available() {
        return Err(unsupported("this"));
    }
    let bridge = bridge()?;
    let _turn = bridge.turn.lock();

    // A previous call that timed out may have landed its answer late. Anything
    // sitting in the channel now belongs to that call, not this one.
    if let Some(rx) = bridge.rx.try_lock() {
        while rx.try_recv().is_ok() {}
    }

    let source = format!(
        r#"
function report(value) {{
    callDBus({name}, "/", "ai.conduit.KWin", "Report", JSON.stringify(value));
}}
try {{
{body}
}} catch (e) {{
    report({{ error: String(e) }});
}}
"#,
        name = js_string(&bridge.name),
        body = body,
    );

    // A fresh name *and* a fresh path for every call. This is not hygiene, it
    // is correctness: KWin caches a loaded script against its name and path, so
    // reusing either makes the second call silently re-run the *first* script.
    //
    // That failure is invisible from here — the stale script reports something,
    // `run_script` returns Ok, and the caller concludes it worked. It cost an
    // afternoon: `set_window_bounds` cheerfully reported "window moved" while
    // actually re-running the preceding `list_windows`, so the window never
    // moved and nothing anywhere said so.
    let serial = {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NEXT.fetch_add(1, Ordering::Relaxed)
    };
    let script_name = format!("conduit{serial}");

    let dir = std::env::temp_dir().join(format!("conduit-kwin-{}", std::process::id()));
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create a scratch directory for kwin: {e}"))?;
    let path = dir.join(format!("{script_name}.js"));
    std::fs::write(&path, source)
        .map_err(|e| format!("could not write the kwin script: {e}"))?;

    let conn = Connection::session().map_err(|e| e.to_string())?;
    let scripting = Proxy::new(&conn, "org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting")
        .map_err(|e| e.to_string())?;

    let script_name = script_name.as_str();
    let path_str = path.to_string_lossy().to_string();
    scripting
        .call::<_, _, i32>("loadScript", &(path_str.as_str(), script_name))
        .map_err(|e| format!("kwin refused the script: {e}"))?;

    let start = scripting.call::<_, _, ()>("start", &());

    let received = match start {
        Ok(()) => bridge
            .rx
            .lock()
            .recv_timeout(SCRIPT_TIMEOUT)
            .map_err(|_| "kwin did not answer in time".to_string()),
        Err(e) => Err(format!("kwin refused to run the script: {e}")),
    };

    // Unloaded even when the call failed, so KWin does not accumulate one
    // registered script per tool call for the life of the session.
    let _: bool = scripting
        .call("unloadScript", &(script_name,))
        .unwrap_or(false);
    let _ = std::fs::remove_file(&path);

    let payload = received?;

    // The wrapper reports `{"error": ...}` for anything the script threw.
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&payload) {
        if let Some(error) = value.get("error").and_then(|e| e.as_str()) {
            return Err(format!("kwin script failed: {error}"));
        }
    }
    Ok(payload)
}

/// Quotes a Rust string as a JavaScript literal.
///
/// Everything conduit interpolates into a script — a window's UUID, an app
/// name a user typed — goes through this. The values are not attacker-supplied
/// in any meaningful sense, but a window title with an apostrophe in it would
/// otherwise produce a syntax error and a mystifying failure.
pub fn js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // JSON/JS string literals may not carry raw control characters.
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property that actually matters: whatever goes in comes back out when
    /// the result is parsed as a string literal.
    ///
    /// Asserting on the exact spelling of each escape would pin an
    /// implementation detail — a control character may legally be written
    /// `` or `\x01` — and the first version of this test did exactly
    /// that, then failed on its own expectation rather than on the code.
    /// JavaScript string literals are a superset of JSON's, so a value that
    /// round-trips through a JSON parser is one KWin's engine reads back
    /// identically.
    fn round_trips(input: &str) {
        let quoted = js_string(input);
        let parsed: String = serde_json::from_str(&quoted)
            .unwrap_or_else(|e| panic!("{quoted:?} is not a valid string literal: {e}"));
        assert_eq!(parsed, input);
    }

    #[test]
    fn js_strings_survive_quotes_backslashes_and_control_characters() {
        round_trips(r#"a"b"#);
        round_trips(r"a\b");
        round_trips("a\nb");
        round_trips("a\tb");
        // A raw control character is not legal inside a JS string literal.
        round_trips(&format!("a{}b", char::from(1u8)));
        // Real window titles carry em dashes and non-latin text.
        round_trips("WhatsApp — Zen Browser");
        round_trips("ソフトウェア");
        round_trips("");
    }

    /// A window title is interpolated straight into script source that KWin
    /// executes, so a title carrying a quote must not be able to close the
    /// literal and start running statements of its own.
    #[test]
    fn a_hostile_title_cannot_break_out_of_its_literal() {
        let hostile = r#""); workspace.activeWindow.closeWindow(); ("#;
        // Parsing back to exactly the input proves it stayed one string the
        // whole way through, and that nothing in it was ever code.
        round_trips(hostile);

        let quoted = js_string(hostile);
        assert!(quoted.starts_with('"') && quoted.ends_with('"'));
    }
}
