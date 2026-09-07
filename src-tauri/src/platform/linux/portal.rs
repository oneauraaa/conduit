//! The one XDG portal session everything else on Linux hangs off.
//!
//! Wayland gives an ordinary client no way to move the pointer, press a key, or
//! read a pixel it does not own. That is the security model, not a gap, and the
//! sanctioned way through it is `xdg-desktop-portal`: the user consents once,
//! the compositor does the work, and the app never touches the hardware.
//!
//! ## Why RemoteDesktop and ScreenCast are one session
//!
//! They could be two. They must not be, for a reason that is easy to miss:
//! `NotifyPointerMotionAbsolute` takes a **stream id**, and streams only exist
//! on a session that has also been through `ScreenCast.SelectSources`. A
//! RemoteDesktop session on its own can move the pointer only *relatively*, and
//! relative motion goes through the compositor's pointer-acceleration curve —
//! so "move 400px left" lands somewhere that depends on how fast the previous
//! events went out. Absolute positioning is not a nicety here; it is the
//! difference between clicking the button and clicking near it.
//!
//! Binding them together also means one consent dialog instead of two.
//!
//! ## The dialog comes back every launch, and cannot not
//!
//! Every other portal grant can be made permanent with `persist_mode` and a
//! restore token. This one cannot: asking for it earns
//! `InvalidArgument: Remote desktop sessions cannot persist` from the portal.
//! That is deliberate upstream — a token that silently restores *full input
//! control* of a machine is precisely the thing an attacker would want to
//! steal, so the portal refuses to mint one.
//!
//! So conduit asks once per launch, at startup, next to the window explaining
//! what it is. Do not "fix" this by moving the prompt later; a consent dialog
//! that appears twenty minutes in, over whatever the agent was doing, is worse
//! in every way. It is also why conduit is a long-running tray app rather than
//! something you start per task.
//!
//! ## Why blocking zbus
//!
//! `platform::*` is a synchronous contract on all three platforms —
//! `mcp/tools.rs` calls `input::scroll(..)` and expects it to have happened.
//! zbus's async API would force either an executor hop per call or a
//! channel-and-oneshot dance behind every function. The blocking API keeps the
//! Linux backend shaped like its two siblings, and each call is a single local
//! IPC round trip well under a millisecond — the macOS backend blocks the same
//! threads on Quartz for longer.
//!
//! ## The handshake
//!
//! Every portal call is a two-step: the method returns a `Request` object path,
//! and the real answer arrives later as a `Response` signal on it. The
//! subscription must be opened *before* the method is issued — the portal is
//! free to emit `Response` before the method call returns, and a listener
//! attached afterwards waits forever for a signal that already went past.

use std::collections::HashMap;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};

use parking_lot::{Mutex, RwLock};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

const PORTAL_DEST: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const REMOTE_DESKTOP: &str = "org.freedesktop.portal.RemoteDesktop";
const SCREEN_CAST: &str = "org.freedesktop.portal.ScreenCast";

type Options = HashMap<String, Value<'static>>;
type Results = HashMap<String, OwnedValue>;

/* ── what a live session exposes ──────────────────────────────── */

/// One monitor as the portal describes it.
#[derive(Debug, Clone, Copy)]
pub struct Stream {
    /// PipeWire node id. Two things key off it: [`super::capture`] subscribes
    /// to it, and `NotifyPointerMotionAbsolute` names it to say *which screen*
    /// an absolute coordinate belongs to.
    pub node_id: u32,
    /// Position in the compositor's layout space, which is also conduit's.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Stream {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// Why the portal is not usable, phrased for the readiness card rather than for
/// a log. Every variant is something the user can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortalError {
    /// Nothing answering on the session bus.
    Missing(String),
    /// The user closed the dialog or pressed Deny.
    Denied,
    /// Reached the portal and it refused — usually no backend installed for
    /// this desktop, which is what a bare wlroots session looks like.
    Failed(String),
    /// The consent dialog is open and unanswered.
    ///
    /// Deliberately *not* cached: it is a "not yet", and the next call after
    /// the user clicks share must succeed.
    Pending,
}

impl PortalError {
    pub fn message(&self) -> String {
        match self {
            PortalError::Missing(e) => format!(
                "xdg-desktop-portal is not answering on the session bus ({e}). install \
                 xdg-desktop-portal and the backend for your desktop \
                 (xdg-desktop-portal-kde on plasma), then log back in."
            ),
            PortalError::Denied => "screen sharing was declined. conduit needs it to see \
                 the screen and move the pointer — press try again and choose share."
                .into(),
            PortalError::Failed(e) => format!(
                "the desktop portal refused the session ({e}). on wayland this usually \
                 means no portal backend is installed for this compositor."
            ),
            PortalError::Pending => "conduit is still waiting for the screen-sharing \
                 prompt to be answered. it is open on the user's screen — ask them to \
                 choose share, then try again."
                .into(),
        }
    }
}

/// The live session, once consent has been given.
pub struct Session {
    conn: Connection,
    path: OwnedObjectPath,
    streams: Vec<Stream>,
}

impl Session {
    pub fn streams(&self) -> &[Stream] {
        &self.streams
    }

    fn proxy<'a>(&self, interface: &'a str) -> Result<Proxy<'a>, String> {
        Proxy::new(&self.conn, PORTAL_DEST, PORTAL_PATH, interface).map_err(|e| e.to_string())
    }

    /// The stream a point falls on, and the point rebased to that stream's own
    /// origin.
    ///
    /// The portal wants coordinates relative to the stream, not to the desktop:
    /// (0,0) on the second monitor is *its* top-left, not the first monitor's.
    /// Getting this wrong puts every click on the wrong screen — and on a
    /// single-monitor machine it looks perfectly correct, which is exactly why
    /// it is computed here once rather than at four call sites.
    fn locate(&self, x: f64, y: f64) -> Option<(u32, f64, f64)> {
        let stream = self
            .streams
            .iter()
            .find(|s| s.contains(x, y))
            // A point just off an edge is far likelier to be rounding than a
            // real error, so fall back rather than refuse to move at all.
            .or_else(|| self.streams.first())?;
        Some((stream.node_id, x - stream.x, y - stream.y))
    }

    /* ── pointer ── */

    pub fn pointer_motion_absolute(&self, x: f64, y: f64) -> Result<(), String> {
        let (node, sx, sy) = self
            .locate(x, y)
            .ok_or("the portal session has no screens to position the pointer on")?;

        self.proxy(REMOTE_DESKTOP)?
            .call_method(
                "NotifyPointerMotionAbsolute",
                &(&self.path, Options::new(), node, sx, sy),
            )
            .map(|_| ())
            .map_err(|e| format!("the portal refused a pointer move: {e}"))
    }

    /// `button` is an **evdev** code (`BTN_LEFT` is 0x110), not a portal-local
    /// enum — see [`super::input`] for the constants.
    pub fn pointer_button(&self, button: i32, pressed: bool) -> Result<(), String> {
        self.proxy(REMOTE_DESKTOP)?
            .call_method(
                "NotifyPointerButton",
                &(&self.path, Options::new(), button, u32::from(pressed)),
            )
            .map(|_| ())
            .map_err(|e| format!("the portal refused a click: {e}"))
    }

    /// Wheel steps in detents. `axis` 0 is vertical, 1 horizontal; positive
    /// scrolls down and right.
    pub fn pointer_axis_discrete(&self, axis: u32, steps: i32) -> Result<(), String> {
        self.proxy(REMOTE_DESKTOP)?
            .call_method(
                "NotifyPointerAxisDiscrete",
                &(&self.path, Options::new(), axis, steps),
            )
            .map(|_| ())
            .map_err(|e| format!("the portal refused a scroll: {e}"))
    }

    /* ── keyboard ── */

    /// Presses or releases an X11 keysym.
    ///
    /// Keysyms rather than evdev keycodes on purpose: a keysym is a *character
    /// meaning*, and the compositor maps it through whatever layout is active,
    /// so `ü` and `@` arrive correctly on a Turkish or a US keyboard without
    /// conduit knowing which is loaded. Keycodes are positional and would type
    /// the wrong character on any non-US layout — the exact bug the macOS
    /// backend dodges by attaching unicode to the event instead.
    pub fn keyboard_keysym(&self, keysym: i32, pressed: bool) -> Result<(), String> {
        self.proxy(REMOTE_DESKTOP)?
            .call_method(
                "NotifyKeyboardKeysym",
                &(&self.path, Options::new(), keysym, u32::from(pressed)),
            )
            .map(|_| ())
            .map_err(|e| format!("the portal refused a keystroke: {e}"))
    }

    /* ── capture ── */

    /// A file descriptor for the PipeWire remote carrying this session's
    /// streams.
    ///
    /// The fd is deliberately leaked into the caller's ownership: PipeWire
    /// takes it over and closes it with its context, so closing it here as well
    /// would land a second `close()` on whatever descriptor number got reused —
    /// the kind of bug that surfaces an hour later as an unrelated socket
    /// dying.
    pub fn pipewire_fd(&self) -> Result<RawFd, String> {
        let reply = self
            .proxy(SCREEN_CAST)?
            .call_method("OpenPipeWireRemote", &(&self.path, Options::new()))
            .map_err(|e| format!("could not open the pipewire remote: {e}"))?;

        let fd: zbus::zvariant::OwnedFd = reply
            .body()
            .deserialize()
            .map_err(|e| format!("the portal returned no pipewire fd: {e}"))?;

        let fd = OwnedFd::from(fd);
        let raw = fd.as_raw_fd();
        std::mem::forget(fd);
        Ok(raw)
    }
}

/* ── the global session ───────────────────────────────────────── */

/// Leaked on success so callers get a `&'static Session` without an Arc in
/// every signature. Exactly one is ever created per process.
static SESSION: RwLock<Option<&'static Session>> = RwLock::new(None);
/// The last failure, kept so a denial does not re-raise the dialog on every
/// subsequent tool call. [`retry`] clears it.
static LAST_ERROR: RwLock<Option<PortalError>> = RwLock::new(None);
/// Serializes the handshake, so two tools racing at startup raise one dialog.
static INIT_LOCK: Mutex<()> = Mutex::new(());

/// The live session, starting it on first use.
///
/// **Blocks on the consent dialog the first time.** Callers are tool handlers
/// on tokio workers, which is survivable, but [`warm_up`] exists so the dialog
/// appears at launch rather than in the middle of an agent's first click.
pub fn session() -> Result<&'static Session, PortalError> {
    if let Some(existing) = *SESSION.read() {
        return Ok(existing);
    }
    if let Some(error) = LAST_ERROR.read().clone() {
        return Err(error);
    }

    // `try_lock`, never `lock`. The warm-up thread holds this for as long as
    // the consent dialog is on screen, which is however long the user takes to
    // notice it. A tool call that blocked here would simply never return, and
    // an agent has no timeout to save it — an immediate "still waiting" is the
    // only honest answer, and the next call after they click share succeeds.
    let Some(_guard) = INIT_LOCK.try_lock() else {
        return Err(PortalError::Pending);
    };

    // The holder may have finished between the checks above and the lock.
    if let Some(existing) = *SESSION.read() {
        return Ok(existing);
    }
    if let Some(error) = LAST_ERROR.read().clone() {
        return Err(error);
    }

    match connect() {
        Ok(session) => {
            let leaked: &'static Session = Box::leak(Box::new(session));
            tracing::info!(streams = leaked.streams.len(), "portal session established");
            *SESSION.write() = Some(leaked);
            Ok(leaked)
        }
        Err(e) => {
            tracing::warn!("portal session unavailable: {}", e.message());
            *LAST_ERROR.write() = Some(e.clone());
            Err(e)
        }
    }
}

/// Whether the session is up, *without* starting one.
///
/// The readiness card polls this every two seconds; making that poll raise a
/// consent dialog would be indefensible.
pub fn established() -> Option<Result<(), PortalError>> {
    if SESSION.read().is_some() {
        return Some(Ok(()));
    }
    LAST_ERROR.read().clone().map(Err)
}

/// Forgets a previous failure so the next [`session`] call asks again.
///
/// This is what the readiness card's "try again" does after a denial. It cannot
/// tear down a *successful* session — the portal has no revoke, and pretending
/// otherwise would leave the UI claiming a capability conduit still holds.
pub fn retry() {
    *LAST_ERROR.write() = None;
}

/// Brings the session up on a background thread at launch.
///
/// The point is *when* the dialog appears: at startup, beside the window that
/// explains what conduit is, rather than several minutes later under whatever
/// the agent was doing.
pub fn warm_up() {
    std::thread::Builder::new()
        .name("conduit-portal".into())
        .spawn(|| {
            if session().is_ok() {
                // Capture only has streams to attach to once consent is in, so
                // it starts from here rather than racing the dialog.
                super::capture::start();
            }
        })
        .ok();
}

/* ── the handshake ────────────────────────────────────────────── */

fn connect() -> Result<Session, PortalError> {
    let conn = Connection::session().map_err(|e| PortalError::Missing(e.to_string()))?;

    // The portal derives Request object paths from the caller's unique bus
    // name with dots turned into underscores, so this is what makes the path
    // predictable enough to subscribe to before the call goes out.
    let unique = conn
        .unique_name()
        .map(|n| n.as_str().trim_start_matches(':').replace('.', "_"))
        .ok_or_else(|| PortalError::Missing("the session bus gave us no unique name".into()))?;

    let remote = Proxy::new(&conn, PORTAL_DEST, PORTAL_PATH, REMOTE_DESKTOP)
        .map_err(|e| PortalError::Missing(e.to_string()))?;
    let cast = Proxy::new(&conn, PORTAL_DEST, PORTAL_PATH, SCREEN_CAST)
        .map_err(|e| PortalError::Missing(e.to_string()))?;

    /* 1. CreateSession */
    let created = request(&conn, &unique, |token| {
        remote
            .call_method(
                "CreateSession",
                &(options(
                    token,
                    vec![("session_handle_token", Value::from("conduit"))],
                ),),
            )
            .map(|_| ())
    })?;

    let handle: String = created
        .get("session_handle")
        .and_then(|v| v.downcast_ref::<String>().ok())
        .ok_or_else(|| PortalError::Failed("the portal returned no session handle".into()))?;
    let session_path: OwnedObjectPath = ObjectPath::try_from(handle)
        .map_err(|e| PortalError::Failed(format!("bad session handle: {e}")))?
        .into();

    /* 2. SelectDevices — what conduit is allowed to drive.
       1 = keyboard, 2 = pointer. Touchscreen (4) is left out deliberately:
       conduit has no touch tools, and asking for a capability it never uses
       makes the consent dialog claim more than it should. */
    request(&conn, &unique, |token| {
        remote
            .call_method(
                "SelectDevices",
                &(
                    &session_path,
                    options(token, vec![("types", Value::from(1u32 | 2u32))]),
                ),
            )
            .map(|_| ())
    })?;

    /* 3. SelectSources — the screens. Also what makes absolute pointer
       coordinates possible at all; see the module header. */
    request(&conn, &unique, |token| {
        cast.call_method(
            "SelectSources",
            &(
                &session_path,
                options(
                    token,
                    vec![
                        // 1 = monitors. Offering windows (2) as well reads as
                        // friendlier and is in fact a trap: the agent would be
                        // blind to everything outside the chosen window and
                        // would click into a coordinate space that does not
                        // exist.
                        ("types", Value::from(1u32)),
                        ("multiple", Value::from(true)),
                        // 1 = hidden. conduit draws its own cursor in the
                        // overlay, so a system arrow baked into the frames
                        // would double it — and would show the agent a cursor
                        // it did not put there.
                        ("cursor_mode", Value::from(1u32)),
                    ],
                ),
            ),
        )
        .map(|_| ())
    })?;

    /* 4. Start — this is what raises the dialog. No persist_mode: see header. */
    let started = request(&conn, &unique, |token| {
        remote
            .call_method("Start", &(&session_path, "", options(token, vec![])))
            .map(|_| ())
    })?;

    let streams = parse_streams(&started);
    if streams.is_empty() {
        return Err(PortalError::Failed(
            "the session started but shared no screens".into(),
        ));
    }

    Ok(Session {
        conn,
        path: session_path,
        streams,
    })
}

/// Issues one portal call and blocks until its `Response` signal arrives.
///
/// `call` receives the handle token it must embed in the method's trailing
/// options dictionary. Threading that token through by hand at each call site
/// invites exactly one of them to forget it — and a forgotten token means the
/// Request path is unguessable and the call hangs forever.
fn request(
    conn: &Connection,
    unique: &str,
    call: impl FnOnce(&str) -> zbus::Result<()>,
) -> Result<Results, PortalError> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);

    let token = format!("conduit{}", NEXT.fetch_add(1, Ordering::Relaxed));
    let path = format!("/org/freedesktop/portal/desktop/request/{unique}/{token}");

    let request = Proxy::new(
        conn,
        PORTAL_DEST,
        path.as_str(),
        "org.freedesktop.portal.Request",
    )
    .map_err(|e| PortalError::Failed(e.to_string()))?;

    // Subscribed before the call goes out — see the module header for why the
    // other order silently hangs.
    let mut responses = request
        .receive_signal("Response")
        .map_err(|e| PortalError::Failed(e.to_string()))?;

    call(&token).map_err(|e| PortalError::Failed(e.to_string()))?;

    let Some(message) = responses.next() else {
        return Err(PortalError::Failed(
            "the portal closed the connection before answering".into(),
        ));
    };

    let (code, results): (u32, Results) = message
        .body()
        .deserialize()
        .map_err(|e| PortalError::Failed(e.to_string()))?;

    match code {
        0 => Ok(results),
        // 1 is "the user cancelled", 2 is "ended some other way". Both mean the
        // same thing to conduit, and both deserve the actionable message rather
        // than a bare number.
        1 | 2 => Err(PortalError::Denied),
        other => Err(PortalError::Failed(format!(
            "the portal returned an unknown response code {other}"
        ))),
    }
}

/// Builds a portal options dictionary with `handle_token` filled in.
fn options(token: &str, entries: Vec<(&str, Value<'static>)>) -> Options {
    let mut map = Options::new();
    for (key, value) in entries {
        map.insert(key.to_string(), value);
    }
    map.insert("handle_token".into(), Value::from(token.to_string()));
    map
}

/// Pulls the stream list out of `Start`'s response.
///
/// Shape is `a(ua{sv})`: a node id paired with a property bag. `position` and
/// `size` are both optional in the spec, and a stream missing either can be
/// captured but cannot be pointed at — so those are dropped rather than given
/// invented geometry that would silently misplace every click on that monitor.
fn parse_streams(response: &Results) -> Vec<Stream> {
    let Some(raw) = response.get("streams") else {
        return Vec::new();
    };
    let Ok(list) = raw.downcast_ref::<zbus::zvariant::Array>() else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in list.iter() {
        let Ok(structure) = entry.downcast_ref::<zbus::zvariant::Structure>() else {
            continue;
        };
        let fields = structure.fields();
        let (Some(node), Some(props)) = (fields.first(), fields.get(1)) else {
            continue;
        };
        let Ok(node_id) = node.downcast_ref::<u32>() else {
            continue;
        };
        let Ok(props) = props.downcast_ref::<zbus::zvariant::Dict>() else {
            continue;
        };

        let (Some((x, y)), Some((w, h))) = (pair(&props, "position"), pair(&props, "size")) else {
            tracing::warn!(node_id, "portal stream has no geometry; skipping it");
            continue;
        };

        out.push(Stream {
            node_id,
            x: x as f64,
            y: y as f64,
            width: w as f64,
            height: h as f64,
        });
    }

    // Top-to-bottom, left-to-right, so stream order matches the order
    // `screen::displays` reports monitors in.
    out.sort_by(|a, b| {
        (a.y, a.x)
            .partial_cmp(&(b.y, b.x))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// Reads an `(ii)` tuple out of a stream's property bag.
fn pair(dict: &zbus::zvariant::Dict, key: &str) -> Option<(i32, i32)> {
    let key = key.to_string();
    let value: Value = dict.get::<String, Value>(&key).ok().flatten()?;
    let structure = value.downcast_ref::<zbus::zvariant::Structure>().ok()?;
    let fields = structure.fields();
    let a = fields.first()?.downcast_ref::<i32>().ok()?;
    let b = fields.get(1)?.downcast_ref::<i32>().ok()?;
    Some((a, b))
}
