//! Window and application enumeration and control.
//!
//! Window *metadata* and geometry come from KWin's scripting engine — see
//! [`super::kwin`] for why that is the only door on Wayland, and what happens
//! on desktops that do not have it.
//!
//! Applications are derived from the window list rather than from `/proc`. That
//! is deliberate: the macOS backend lists `NSWorkspace::runningApplications`,
//! which is "processes with a UI presence", and a raw `/proc` walk is not that
//! — it is four hundred kernel threads and systemd units an agent can do
//! nothing with. A process that owns a window is the honest Linux equivalent.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use parking_lot::RwLock;
use serde::Deserialize;

use super::{hyprctl, kwin};
use crate::platform::types::{AppInfo, OwnWindows, WindowInfo};

/// Maps the `u32` window ids the tool surface uses onto KWin's UUIDs.
///
/// The contract wants a `u32` — it was shaped by `CGWindowID` and `HWND`, both
/// of which really are integers. KWin identifies windows by UUID string, so the
/// id conduit reports is a hash of that UUID, and this table is how it gets
/// back. Populated by every [`list_windows`], which is what an agent calls
/// before acting on a window anyway.
static WINDOW_IDS: RwLock<Option<HashMap<u32, String>>> = RwLock::new(None);

/// The pid owning the focused window, as of the last [`list_windows`]. KWin
/// reports this per window; `list_apps` needs it per application.
static ACTIVE_PID: RwLock<Option<i32>> = RwLock::new(None);

/// A window as the KWin script reports it.
#[derive(Debug, Deserialize)]
struct RawWindow {
    id: String,
    caption: String,
    #[serde(rename = "cls")]
    class: String,
    pid: i32,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    minimized: bool,
    active: bool,
}

/// A stable `u32` for a KWin UUID.
///
/// `DefaultHasher` is SipHash with fixed keys, so this is deterministic across
/// runs as well as within one — an id an agent noted a minute ago still
/// resolves. Collisions are theoretically possible; with a few dozen windows
/// the probability is far below that of the window closing mid-call, which the
/// code already handles.
fn window_id(uuid: &str) -> u32 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    uuid.hash(&mut hasher);
    // Fold to 32 bits and avoid 0, which reads as "no window" in a tool result.
    ((hasher.finish() as u32) | 1).max(1)
}

/// On-screen windows, front to back.
///
/// Excludes conduit's own windows so an agent never tries to click its own
/// chrome, and excludes anything that is not a normal application window —
/// panels, the desktop, notification popups. That is the counterpart of the
/// macOS `kCGWindowLayer != 0` check and of `DWMWA_CLOAKED` on Windows.
pub fn list_windows() -> Vec<WindowInfo> {
    let own_pid = std::process::id() as i32;

    // `stackingOrder` is bottom-to-top; reversed below so index 0 is frontmost,
    // matching `layer_index`'s contract on the other two platforms.
    let script = r#"
var out = [];
var wins = workspace.stackingOrder;
for (var i = 0; i < wins.length; i++) {
    var w = wins[i];
    if (!w.normalWindow) continue;
    if (w.skipTaskbar) continue;
    var g = w.frameGeometry;
    if (g.width < 1 || g.height < 1) continue;
    out.push({
        id: String(w.internalId),
        caption: String(w.caption || ""),
        cls: String(w.resourceClass || ""),
        pid: w.pid,
        x: g.x, y: g.y, width: g.width, height: g.height,
        minimized: !!w.minimized,
        active: !!w.active
    });
}
report(out);
"#;

    let payload = match kwin::run_script(script) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("could not list windows: {e}");
            return Vec::new();
        }
    };

    let raw: Vec<RawWindow> = match serde_json::from_str(&payload) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("could not parse the kwin window list: {e}");
            return Vec::new();
        }
    };

    let mut ids = HashMap::new();
    let mut out = Vec::new();
    let mut active_pid: Option<i32> = None;

    for window in raw.into_iter().rev() {
        if window.pid == own_pid {
            continue;
        }
        // A minimized window is not on screen and cannot be clicked. macOS
        // excludes it via `OptionOnScreenOnly` and Windows via the cloaked
        // check; this is the same rule. `focus_window` un-minimizes, so one is
        // still reachable by id from an earlier listing.
        if window.minimized {
            continue;
        }
        let id = window_id(&window.id);
        ids.insert(id, window.id.clone());
        if window.active {
            active_pid = Some(window.pid);
        }

        out.push(WindowInfo {
            id,
            title: window.caption,
            app: window.class,
            pid: window.pid,
            x: window.x,
            y: window.y,
            width: window.width,
            height: window.height,
            layer_index: out.len(),
        });
    }

    *WINDOW_IDS.write() = Some(ids);
    *ACTIVE_PID.write() = active_pid;
    out
}

/// What a window listing is quietly not telling the agent.
pub fn list_windows_hint(windows: &[WindowInfo]) -> Option<String> {
    if !kwin::available() {
        return Some(kwin::unsupported("listing windows"));
    }
    // An empty list on a desktop that plainly has windows means the script
    // failed rather than that nothing is open. Saying so beats an agent
    // concluding the session is over.
    if windows.is_empty() {
        return Some(
            "kwin returned no windows. if the desktop clearly has some, conduit's \
             scripting bridge failed — check conduit's log."
                .into(),
        );
    }
    None
}

/// Running applications, one per process that owns a window.
pub fn list_apps() -> Vec<AppInfo> {
    let mut seen: HashMap<i32, AppInfo> = HashMap::new();

    for window in list_windows() {
        let entry = seen.entry(window.pid).or_insert_with(|| AppInfo {
            name: window.app.clone(),
            pid: window.pid,
            // The resource class *is* Linux's reverse-DNS app identifier —
            // `org.kde.dolphin`, `com.anthropic.Claude` — which is the same
            // concept a bundle id names on macOS.
            bundle_id: (!window.app.is_empty()).then(|| window.app.clone()),
            active: false,
        });
        // An app is active if it owns the focused window.
        if Some(window.pid) == *ACTIVE_PID.read() {
            entry.active = true;
        }
    }

    let mut out: Vec<AppInfo> = seen.into_values().collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

fn uuid_for(window_id: u32) -> Result<String, String> {
    if let Some(uuid) = WINDOW_IDS
        .read()
        .as_ref()
        .and_then(|ids| ids.get(&window_id))
        .cloned()
    {
        return Ok(uuid);
    }

    // The table is filled by `list_windows`; an agent that acts on an id it got
    // from a previous session's listing lands here. Refresh once rather than
    // failing on what is a perfectly reasonable call order.
    let _ = list_windows();
    WINDOW_IDS
        .read()
        .as_ref()
        .and_then(|ids| ids.get(&window_id))
        .cloned()
        .ok_or_else(|| format!("no window with id {window_id}"))
}

/// Brings a window to the front and gives it focus.
pub fn focus_window(window_id: u32) -> Result<(), String> {
    let uuid = uuid_for(window_id)?;
    let script = format!(
        r#"
var target = {uuid};
var wins = workspace.windowList();
var found = null;
for (var i = 0; i < wins.length; i++) {{
    if (String(wins[i].internalId) === target) {{ found = wins[i]; break; }}
}}
if (!found) {{
    report({{ error: "that window is gone" }});
}} else {{
    // A minimized window cannot take focus, and raising it without this is a
    // silent no-op that looks exactly like a permission problem.
    found.minimized = false;
    // Following the window to its own desktop beats yanking it to this one,
    // which would rearrange the user's workspace behind their back.
    if (found.desktops && found.desktops.length > 0) {{
        workspace.currentDesktop = found.desktops[0];
    }}
    workspace.activeWindow = found;
    report("ok");
}}
"#,
        uuid = kwin::js_string(&uuid),
    );

    kwin::run_script(&script).map(|_| ())
}

/// How long to give a window to acknowledge a geometry change.
///
/// Unlike macOS and Windows, this write is asynchronous *by protocol*: KWin
/// sends the client a configure, and the geometry only changes once the client
/// acks and commits it. Reading `frameGeometry` immediately after assigning it
/// returns the old value — which is not a KWin bug, it is what "the window has
/// not agreed yet" looks like.
///
/// Long enough for a browser or an Electron app under load; short enough not to
/// be felt in a tool call.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(180);

/// Reads one window's current geometry, by UUID.
fn geometry_of(uuid: &str) -> Option<(f64, f64, f64, f64)> {
    let script = format!(
        r#"
var target = {uuid};
var wins = workspace.windowList();
var found = null;
for (var i = 0; i < wins.length; i++) {{
    if (String(wins[i].internalId) === target) {{ found = wins[i]; break; }}
}}
// Exactly one report per script, always. Two would leave an extra reply in the
// bridge's channel for the *next* call to pick up as its own answer.
if (found) {{
    var g = found.frameGeometry;
    report({{ x: g.x, y: g.y, width: g.width, height: g.height }});
}} else {{
    report(null);
}}
"#,
        uuid = kwin::js_string(uuid),
    );

    let payload = kwin::run_script(&script).ok()?;
    let value: serde_json::Value = serde_json::from_str(&payload).ok()?;
    Some((
        value.get("x")?.as_f64()?,
        value.get("y")?.as_f64()?,
        value.get("width")?.as_f64()?,
        value.get("height")?.as_f64()?,
    ))
}

/// Moves and resizes a window.
pub fn set_window_bounds(
    window_id: u32,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let uuid = uuid_for(window_id)?;
    let script = format!(
        r#"
var target = {uuid};
var wins = workspace.windowList();
var found = null;
for (var i = 0; i < wins.length; i++) {{
    if (String(wins[i].internalId) === target) {{ found = wins[i]; break; }}
}}
if (!found) {{
    report({{ error: "that window is gone" }});
}} else if (!found.moveable && !found.resizeable) {{
    report({{ error: "that window refuses to be moved or resized" }});
}} else {{
    // Full-screen and maximized windows ignore geometry until the mode is
    // dropped, which otherwise reads as the call silently doing nothing.
    found.fullScreen = false;
    if (found.setMaximize) found.setMaximize(false, false);
    found.frameGeometry = {{ x: {x}, y: {y}, width: {w}, height: {h} }};
    report("ok");
}}
"#,
        uuid = kwin::js_string(&uuid),
        x = x.round() as i64,
        y = y.round() as i64,
        // A zero or negative size is rejected by KWin with an opaque failure;
        // clamping turns a bad request into a small window instead.
        w = (width.round() as i64).max(1),
        h = (height.round() as i64).max(1),
    );

    let before = geometry_of(&uuid);
    kwin::run_script(&script)?;

    // Verify rather than assume. KWin accepting the assignment only means the
    // *request* went out; whether the window took it is up to the window. A
    // fixed-size dialog simply ignores it, and reporting "window moved" for
    // that is exactly the silent wrong answer this codebase refuses to ship.
    std::thread::sleep(SETTLE);
    let after = geometry_of(&uuid);

    if let (Some(before), Some(after)) = (before, after) {
        // Only a complete no-op counts as refusal. Windows legitimately land
        // near rather than on a request — terminals snap to character cells,
        // and toolkits clamp to minimum sizes — and calling that a failure
        // would be worse than useless.
        let unmoved = (after.0 - before.0).abs() < 1.0
            && (after.1 - before.1).abs() < 1.0
            && (after.2 - before.2).abs() < 1.0
            && (after.3 - before.3).abs() < 1.0;
        let asked_to_change = (before.0 - x).abs() >= 1.0
            || (before.1 - y).abs() >= 1.0
            || (before.2 - width).abs() >= 1.0
            || (before.3 - height).abs() >= 1.0;

        if unmoved && asked_to_change {
            return Err(format!(
                "the window did not move. it is still {}x{} at {},{} — it most \
                 likely has fixed size constraints, or is tiled by a kwin rule \
                 that overrides scripted geometry.",
                after.2 as i64, after.3 as i64, after.0 as i64, after.1 as i64
            ));
        }
    }

    Ok(())
}

/* ── launching and quitting ───────────────────────────────────── */

/// Launches an app by name, or brings it forward if it's already running.
pub fn open_app(name: &str) -> Result<(), String> {
    // Already running? Raise it, the way `open -a` does on macOS. Matching on
    // both the window class and the title covers "dolphin" and "Dolphin" as
    // well as an app whose class is a reverse-DNS id the user would never type.
    let needle = name.trim().to_lowercase();
    if let Some(window) = list_windows().into_iter().find(|w| {
        w.app.to_lowercase() == needle
            || w.app.to_lowercase().ends_with(&format!(".{needle}"))
            || w.title.to_lowercase().contains(&needle)
    }) {
        return focus_window(window.id);
    }

    let entry = find_desktop_entry(&needle)
        .ok_or_else(|| format!("no application named {name}"))?;

    // `gio launch` runs the entry the way the desktop would — correct working
    // directory, the right environment, and the process reparented away from
    // conduit so quitting conduit does not take the app with it. Parsing `Exec`
    // by hand is the fallback for systems without glib's CLI.
    if std::process::Command::new("gio")
        .arg("launch")
        .arg(&entry.path)
        .spawn()
        .map(|mut c| {
            // Reaped so a launched app does not linger as a zombie.
            std::thread::spawn(move || {
                let _ = c.wait();
            });
        })
        .is_ok()
    {
        return Ok(());
    }

    let exec = entry
        .exec
        .ok_or_else(|| format!("{name} has no launch command"))?;
    let mut parts = exec.split_whitespace();
    let program = parts.next().ok_or("empty launch command")?;

    std::process::Command::new(program)
        .args(parts)
        .spawn()
        .map(|mut c| {
            std::thread::spawn(move || {
                let _ = c.wait();
            });
        })
        .map_err(|e| format!("could not launch {name}: {e}"))
}

/// Asks an app to quit.
///
/// Deliberately a request, not a kill: `SIGTERM` is what a desktop's "close"
/// sends, and an application with unsaved work gets its chance to prompt.
/// `SIGKILL` would take that away, which is why it is never sent here — the
/// macOS backend makes the same choice with `terminate` over `forceTerminate`.
pub fn quit_app(name: &str) -> Result<(), String> {
    let needle = name.trim().to_lowercase();
    let app = list_apps()
        .into_iter()
        .find(|a| {
            a.name.to_lowercase() == needle || a.name.to_lowercase().ends_with(&format!(".{needle}"))
        })
        .ok_or_else(|| format!("{name} is not running"))?;

    // SAFETY: `kill` with a pid this process just read from the window list.
    // A pid that has exited in the meantime returns ESRCH, handled below.
    let result = unsafe { libc::kill(app.pid, libc::SIGTERM) };
    if result == 0 {
        Ok(())
    } else {
        Err(format!(
            "could not ask {name} to quit: {}",
            std::io::Error::last_os_error()
        ))
    }
}

/* ── desktop entries ──────────────────────────────────────────── */

pub struct DesktopEntry {
    pub path: std::path::PathBuf,
    pub name: String,
    pub exec: Option<String>,
    pub icon: Option<String>,
}

/// Every `.desktop` file on the system, in XDG precedence order.
pub fn desktop_entries() -> Vec<DesktopEntry> {
    let mut out = Vec::new();
    for dir in application_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            if let Some(parsed) = parse_desktop_entry(&path) {
                out.push(parsed);
            }
        }
    }
    out
}

fn application_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = dirs::data_local_dir() {
        dirs.push(home.join("applications"));
    }
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    for dir in data_dirs.split(':').filter(|d| !d.is_empty()) {
        dirs.push(std::path::PathBuf::from(dir).join("applications"));
    }
    dirs
}

/// Parses the `[Desktop Entry]` group of a `.desktop` file.
///
/// Only that group: a file's `[Desktop Action ...]` groups carry their own
/// `Name` and `Exec`, and reading straight through would pick up "Open a New
/// Window" as the application's name.
fn parse_desktop_entry(path: &std::path::Path) -> Option<DesktopEntry> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_main = false;
    let (mut name, mut exec, mut icon) = (None, None, None);
    let mut hidden = false;

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            // Localized keys look like `Name[tr]`; the unlocalized one is what
            // an agent and a user will both type.
            "Name" => name = Some(value.trim().to_string()),
            "Exec" => exec = Some(value.trim().to_string()),
            "Icon" => icon = Some(value.trim().to_string()),
            "NoDisplay" | "Hidden" => hidden |= value.trim() == "true",
            _ => {}
        }
    }

    if hidden {
        return None;
    }

    Some(DesktopEntry {
        path: path.to_path_buf(),
        name: name?,
        // Field codes (%U, %f, %i…) are launcher placeholders, not arguments,
        // and passing them through makes the app open a file called "%U".
        exec: exec.map(|e| strip_field_codes(&e)),
        icon,
    })
}

fn strip_field_codes(exec: &str) -> String {
    exec.split_whitespace()
        .filter(|token| !(token.len() == 2 && token.starts_with('%')))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The best `.desktop` match for a name a human typed.
pub fn find_desktop_entry(needle: &str) -> Option<DesktopEntry> {
    let needle = needle.trim().to_lowercase();
    let entries = desktop_entries();

    // Exact name, then the file's own id, then a prefix, then a substring —
    // the same ranking `types::rank_matches` applies to elements, for the same
    // reason: "code" should find Code, not "Qt Code Editor Settings".
    let rank = |entry: &DesktopEntry| -> Option<u8> {
        let name = entry.name.to_lowercase();
        let stem = entry
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        if name == needle {
            Some(0)
        } else if stem == needle {
            Some(1)
        } else if stem.rsplit('.').next() == Some(needle.as_str()) {
            Some(2)
        } else if name.starts_with(&needle) {
            Some(3)
        } else if name.contains(&needle) {
            Some(4)
        } else {
            None
        }
    };

    entries
        .into_iter()
        .filter_map(|e| rank(&e).map(|r| (r, e)))
        .min_by_key(|(r, _)| *r)
        .map(|(_, e)| e)
}

/* ── notifications ────────────────────────────────────────────── */

/// Posts a desktop notification.
///
/// Straight to `org.freedesktop.Notifications` rather than through Tauri's
/// notification plugin, which would need an `AppHandle` and so would change
/// `notify`'s signature on all three platforms for no benefit to any of them.
pub fn notify(title: &str, body: &str) -> Result<(), String> {
    let conn = zbus::blocking::Connection::session()
        .map_err(|e| format!("no session bus for notifications: {e}"))?;

    let proxy = zbus::blocking::Proxy::new(
        &conn,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )
    .map_err(|e| format!("could not reach the notification service: {e}"))?;

    let hints: HashMap<&str, zbus::zvariant::Value> = HashMap::new();

    proxy
        .call::<_, _, u32>(
            "Notify",
            &(
                "conduit",
                // 0 means "new notification" rather than replacing one.
                0u32,
                "ai.conduit.app",
                title,
                body,
                Vec::<&str>::new(),
                hints,
                // -1 lets the desktop pick its own timeout, which respects
                // whatever the user configured.
                -1i32,
            ),
        )
        .map(|_| ())
        .map_err(|e| format!("could not post notification: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_ids_are_stable_and_never_zero() {
        let a = window_id("{1457d1b6-8ac9-4584-95bb-a85a668549b1}");
        let b = window_id("{1457d1b6-8ac9-4584-95bb-a85a668549b1}");
        assert_eq!(a, b, "the same uuid must map to the same id every time");
        assert_ne!(a, window_id("{27322c74-2d1b-45ed-868b-2f2882bdd12f}"));
        assert_ne!(a, 0, "0 reads as \"no window\" in a tool result");
    }

    /// Field codes are launcher placeholders. Left in, they make the app open a
    /// file literally named `%U`.
    #[test]
    fn field_codes_are_stripped_from_exec() {
        assert_eq!(strip_field_codes("firefox %u"), "firefox");
        assert_eq!(strip_field_codes("code --new-window %F"), "code --new-window");
        assert_eq!(strip_field_codes("kitty"), "kitty");
        // A bare `%` argument is not a field code and must survive.
        assert_eq!(strip_field_codes("calc %"), "calc %");
    }
}

/// Where conduit's own windows are, according to the compositor.
///
/// Wayland deliberately does not tell a client where its own toplevel sits —
/// there is no `xdg_toplevel` request for it, because a client that knew could
/// place itself over another's. tao answers `outer_position()` with `(0, 0)`
/// rather than failing, which is the worst of both: [`crate::chrome::point_hits_conduit`]
/// used to guard a phantom rectangle at the screen's origin while conduit's real
/// window sat somewhere else entirely, refusing legitimate clicks across most of
/// the screen and protecting nothing.
///
/// So the question goes to the only thing that knows. Both routes filter by pid,
/// which catches every window conduit owns — the main window, the pill and the
/// overlays — without this needing to know their labels.
pub fn own_windows() -> OwnWindows {
    let own_pid = std::process::id() as i32;

    if hyprctl::available() {
        return match hyprctl::rects_for_pid(own_pid) {
            Ok(rects) => OwnWindows::Rects(rects),
            Err(e) => {
                tracing::warn!("could not ask hyprland where conduit's windows are: {e}");
                OwnWindows::Unknown
            }
        };
    }

    if !kwin::available() {
        return OwnWindows::Unknown;
    }

    let script = format!(
        r#"
var out = [];
var wins = workspace.stackingOrder;
for (var i = 0; i < wins.length; i++) {{
    var w = wins[i];
    if (w.pid !== {own_pid}) continue;
    if (w.minimized) continue;
    var g = w.frameGeometry;
    if (g.width < 1 || g.height < 1) continue;
    out.push({{ x: g.x, y: g.y, width: g.width, height: g.height }});
}}
report(out);
"#
    );

    #[derive(serde::Deserialize)]
    struct Rect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    }

    match kwin::run_script(&script).and_then(|p| {
        serde_json::from_str::<Vec<Rect>>(&p).map_err(|e| format!("could not parse the reply: {e}"))
    }) {
        Ok(rects) => OwnWindows::Rects(
            rects
                .into_iter()
                .map(|r| (r.x, r.y, r.width, r.height))
                .collect(),
        ),
        Err(e) => {
            tracing::warn!("could not ask kwin where conduit's windows are: {e}");
            OwnWindows::Unknown
        }
    }
}
