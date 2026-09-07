//! Reading the screen through the AT-SPI2 accessibility tree.
//!
//! This is the tool agents should reach for first. Pixels force a model to
//! guess at text and button edges; the tree hands over the actual strings and
//! the actual bounds, so a click lands on the real centre of the real control.
//! It is also dramatically cheaper than a screenshot.
//!
//! ## AT-SPI is opt-in, and usually off
//!
//! This is the one place where the Linux backend has a caveat with no macOS or
//! Windows equivalent. The AX tree on macOS and UI Automation on Windows are
//! always there; AT-SPI is a *bus* that toolkits populate only when they think
//! assistive technology is listening.
//!
//!   - **Qt/KDE apps** check `QT_ACCESSIBILITY=1` or the presence of a screen
//!     reader.
//!   - **GTK apps** check the `toolkit-accessibility` gsetting.
//!
//! On a default Plasma install both are off, so the tree is nearly empty and
//! `read_screen` returns two elements for a screen with two hundred. That looks
//! exactly like a bug and is not one, which is why [`enabled`] exists and why
//! the readiness card reports it as its own row with the command to fix it.
//!
//! ## Coordinates
//!
//! `GetExtents(0)` asks for screen coordinates, which is the space conduit
//! already speaks. On Wayland a client does not always know where its own
//! window is, and a toolkit that guesses returns extents anchored at the origin.
//! Elements whose bounds are degenerate are dropped rather than reported at the
//! wrong place — a wrong coordinate is worse than a missing one, because the
//! agent will click it.

use std::collections::HashMap;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedValue;

use crate::platform::types::{Element, rank_matches};

const ATSPI_ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const ATSPI_COMPONENT: &str = "org.a11y.atspi.Component";
const REGISTRY_NAME: &str = "org.a11y.atspi.Registry";
const ROOT_PATH: &str = "/org/a11y/atspi/accessible/root";

/// Depth cap. Real UIs nest deeply; beyond this the payload grows faster than
/// its usefulness.
const MAX_DEPTH: usize = 18;
/// Hard cap on returned elements, so one pathological app can't blow the
/// model's context window.
const MAX_ELEMENTS: usize = 400;
/// Hard cap on D-Bus round trips for one call.
///
/// Every node costs at least one. Unlike the in-process AX and UIA walks this
/// mirrors, a runaway tree here would hold a tokio worker for many seconds, so
/// the walk is bounded by work as well as by depth.
const MAX_VISITS: usize = 4000;

/// A node: the bus name that owns it, and its object path.
type Node = (String, zbus::zvariant::OwnedObjectPath);

/* ── the accessibility bus ────────────────────────────────────── */

/// Connects to the accessibility bus.
///
/// It is a *separate* bus from the session bus, whose address is published on
/// the session bus by `org.a11y.Bus`. Talking AT-SPI on the session bus is the
/// classic first mistake and fails with "no such name".
fn a11y_bus() -> Result<Connection, String> {
    let session = Connection::session().map_err(|e| format!("no session bus: {e}"))?;

    let bus = Proxy::new(&session, "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus")
        .map_err(|e| format!("could not reach the accessibility bus launcher: {e}"))?;

    let address: String = bus
        .call("GetAddress", &())
        .map_err(|e| format!("the accessibility bus is not running: {e}"))?;

    zbus::blocking::connection::Builder::address(address.as_str())
        .and_then(|b| b.build())
        .map_err(|e| format!("could not connect to the accessibility bus: {e}"))
}

/// Whether any application is actually publishing an accessibility tree.
///
/// The bus being up says nothing — it is launched on demand and is up on every
/// desktop. What matters is whether toolkits have opted in, which is only
/// visible by looking for applications on it.
pub fn enabled() -> bool {
    let Ok(conn) = a11y_bus() else {
        return false;
    };
    children(&conn, &(REGISTRY_NAME.to_string(), root_path()))
        .map(|apps| !apps.is_empty())
        .unwrap_or(false)
}

fn root_path() -> zbus::zvariant::OwnedObjectPath {
    zbus::zvariant::ObjectPath::try_from(ROOT_PATH)
        .expect("the atspi root path is a literal")
        .into()
}

/// How to turn accessibility on, for the readiness card and for errors.
pub fn how_to_enable() -> &'static str {
    "no application is publishing an accessibility tree. turn it on with:\n  \
     gsettings set org.gnome.desktop.interface toolkit-accessibility true\n\
     and add QT_ACCESSIBILITY=1 to ~/.config/environment.d/ for qt and kde apps, \
     then log back in. screenshots and clicking work without it."
}

/* ── walking ──────────────────────────────────────────────────── */

/// `interface` is `&'static str` because the owned bus name and path force the
/// proxy's lifetime to `'static`; every caller passes a const, so this costs
/// nothing and removes an unsatisfiable bound.
fn proxy(conn: &Connection, node: &Node, interface: &'static str) -> Option<Proxy<'static>> {
    Proxy::new(conn, node.0.clone(), node.1.clone(), interface).ok()
}

fn children(conn: &Connection, node: &Node) -> Option<Vec<Node>> {
    let raw: Vec<(String, zbus::zvariant::OwnedObjectPath)> = proxy(conn, node, ATSPI_ACCESSIBLE)?
        .call("GetChildren", &())
        .ok()?;
    Some(raw)
}

/// Name, description and child count in one round trip.
///
/// `GetAll` rather than three `Get` calls: this is the hot path of the walk,
/// and over D-Bus the round trips, not the bytes, are the cost.
fn attributes(conn: &Connection, node: &Node) -> Option<HashMap<String, OwnedValue>> {
    Proxy::new(
        conn,
        node.0.clone(),
        node.1.clone(),
        "org.freedesktop.DBus.Properties",
    )
    .ok()?
    .call("GetAll", &(ATSPI_ACCESSIBLE,))
    .ok()
}

fn string_attr(attrs: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    let text: String = attrs.get(key)?.downcast_ref::<String>().ok()?;
    (!text.trim().is_empty()).then_some(text)
}

/// Screen-space bounds, or `None` if the toolkit does not really know them.
///
/// Three rejections, and the last two matter more than they look. A zero-sized
/// element cannot be clicked. An element at `i32::MIN` is AT-SPI's way of
/// saying "not laid out" — Qt returns exactly that for controls in a window
/// that has never been shown, and a naive size check waves them through as 1x1
/// rectangles two billion pixels off-screen. And an element landing on no
/// display at all is not something an agent can act on.
///
/// Reporting any of them is worse than omitting them, because the agent will
/// click the coordinate it was handed.
fn extents(conn: &Connection, node: &Node) -> Option<(f64, f64, f64, f64)> {
    // 0 = screen coordinates, which is conduit's space.
    let (x, y, w, h): (i32, i32, i32, i32) = proxy(conn, node, ATSPI_COMPONENT)?
        .call("GetExtents", &(0u32,))
        .ok()?;

    if w < 1 || h < 1 {
        return None;
    }
    // The sentinel, and anything else absurd enough to be one.
    if x <= i32::MIN / 2 || y <= i32::MIN / 2 {
        return None;
    }

    let (x, y, w, h) = (x as f64, y as f64, w as f64, h as f64);

    // Must overlap a real screen. An empty display list means geometry has not
    // been read yet, in which case this check is skipped rather than rejecting
    // every element.
    let displays = crate::chrome::cached_displays();
    let on_screen = displays.is_empty()
        || displays
            .iter()
            .any(|d| x < d.x + d.width && x + w > d.x && y < d.y + d.height && y + h > d.y);

    on_screen.then_some((x, y, w, h))
}

fn role_name(conn: &Connection, node: &Node) -> String {
    proxy(conn, node, ATSPI_ACCESSIBLE)
        .and_then(|p| p.call::<_, _, String>("GetRoleName", &()).ok())
        .unwrap_or_default()
}

fn walk(conn: &Connection, node: &Node, depth: usize, visits: &mut usize, out: &mut Vec<Element>) {
    if depth > MAX_DEPTH || out.len() >= MAX_ELEMENTS || *visits >= MAX_VISITS {
        return;
    }
    *visits += 1;

    if let Some(attrs) = attributes(conn, node) {
        // A control is worth reporting when it says something. Name first, then
        // description — the same preference order the macOS walk uses, for the
        // same reason: an icon-only button's only label is its description.
        let text = string_attr(&attrs, "Name").or_else(|| string_attr(&attrs, "Description"));

        if let Some(text) = text {
            if let Some((x, y, w, h)) = extents(conn, node) {
                out.push(Element {
                    role: role_name(conn, node),
                    text,
                    x,
                    y,
                    width: w,
                    height: h,
                    center_x: x + w / 2.0,
                    center_y: y + h / 2.0,
                });
            }
        }
    }

    let Some(kids) = children(conn, node) else {
        return;
    };
    for child in kids {
        if out.len() >= MAX_ELEMENTS || *visits >= MAX_VISITS {
            return;
        }
        walk(conn, &child, depth + 1, visits, out);
    }
}

/// The application node to read, given an optional name.
fn target(conn: &Connection, app_name: Option<&str>) -> Result<Vec<Node>, String> {
    let root = (REGISTRY_NAME.to_string(), root_path());
    let apps = children(conn, &root)
        .ok_or("the accessibility registry returned no applications")?;

    if apps.is_empty() {
        return Err(how_to_enable().into());
    }

    let named: Vec<(String, Node)> = apps
        .into_iter()
        .map(|node| {
            let name = attributes(conn, &node)
                .and_then(|a| string_attr(&a, "Name"))
                .unwrap_or_default();
            (name, node)
        })
        // conduit is a GTK app and publishes its own tree like any other. Left
        // in, `read_screen_text` hands an agent conduit's own Tools tab and
        // mode dropdown — the exact controls `chrome::point_hits_conduit`
        // exists to keep out of reach. `list_windows` already filters conduit
        // out by pid; this is the same rule for the accessibility tree.
        .filter(|(name, _)| !name.eq_ignore_ascii_case("conduit"))
        .collect();

    match app_name {
        Some(wanted) => {
            let needle = wanted.trim().to_lowercase();
            named
                .into_iter()
                .find(|(name, _)| {
                    let name = name.to_lowercase();
                    name == needle || name.contains(&needle)
                })
                .map(|(_, node)| vec![node])
                .ok_or_else(|| {
                    format!("{wanted} is not running, or is not publishing an accessibility tree")
                })
        }
        None => {
            // No name given means "whatever the user is looking at". AT-SPI has
            // no "frontmost application" call, so the active window's class from
            // KWin is matched against the application names on the bus. When
            // that fails — a non-KDE desktop, or a name that simply differs —
            // every application is walked, which is slower but never wrong.
            let active = super::apps::list_windows().into_iter().next();
            if let Some(window) = active {
                let class = window.app.to_lowercase();
                let title = window.title.to_lowercase();
                if let Some((_, node)) = named.iter().find(|(name, _)| {
                    let name = name.to_lowercase();
                    !name.is_empty()
                        && (class.contains(&name)
                            || name.contains(&class)
                            || title.contains(&name))
                }) {
                    return Ok(vec![node.clone()]);
                }
            }
            Ok(named.into_iter().map(|(_, node)| node).collect())
        }
    }
}

/// Every labelled, clickable element in `app` (or the active application).
pub fn read_screen(app_name: Option<&str>) -> Result<Vec<Element>, String> {
    let conn = a11y_bus()?;
    let roots = target(&conn, app_name)?;

    let mut out = Vec::new();
    let mut visits = 0usize;
    for root in &roots {
        walk(&conn, root, 0, &mut visits, &mut out);
        if out.len() >= MAX_ELEMENTS || visits >= MAX_VISITS {
            break;
        }
    }

    if out.is_empty() {
        return Err(how_to_enable().into());
    }
    Ok(out)
}

/// Elements whose text matches `query`, best match first.
pub fn find_element(query: &str, app_name: Option<&str>) -> Result<Vec<Element>, String> {
    Ok(rank_matches(query, read_screen(app_name)?))
}
