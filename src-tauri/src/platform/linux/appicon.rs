//! Reads an installed application's icon out of the active icon theme.
//!
//! The Agents tab shows each agent's real logo. Rather than bundling other
//! companies' artwork, conduit asks the system for the icon of the app already
//! on this machine — the same rule the macOS and Windows backends follow.
//! Nothing third-party ships inside conduit, the icons are always current, and
//! an agent that isn't installed simply has no icon to show.
//!
//! The handle is a desktop-entry name (`claude-desktop`, `code`), which is what
//! `agents.rs` supplies on Linux — the counterpart of a bundle id on macOS and
//! an executable name on Windows.
//!
//! ## SVG is not optional here
//!
//! Breeze — Plasma's default — ships almost everything as SVG, so a PNG-only
//! lookup finds nothing on exactly the desktop this port targets. Hence the
//! rasterizer. PNG is still preferred when the theme offers one at a usable
//! size: it is already the right pixels, and decoding it is far cheaper.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::Engine;
use parking_lot::RwLock;

/// Icon handle -> rendered data URL. Theme lookup walks a lot of directories
/// and rasterizing an SVG is not free; icons do not change while conduit runs.
static CACHE: RwLock<Option<HashMap<String, Option<String>>>> = RwLock::new(None);

/// Resolves an icon name to a PNG data URL at `size` logical pixels.
pub fn icon_data_url(name: &str, size: f64) -> Option<String> {
    if let Some(cache) = CACHE.read().as_ref() {
        if let Some(hit) = cache.get(name) {
            return hit.clone();
        }
    }

    let rendered = render(name, size);

    CACHE
        .write()
        .get_or_insert_with(HashMap::new)
        .insert(name.to_string(), rendered.clone());

    rendered
}

/// Populates the cache ahead of the user opening the Agents tab.
///
/// Unlike the macOS twin this has no main-thread requirement — nothing here
/// touches a UI toolkit — but it is called from the same place for the same
/// reason: so opening the tab is instant.
pub fn warm_cache(names: &[&str], size: f64) {
    for name in names {
        let _ = icon_data_url(name, size);
    }
}

fn render(name: &str, size: f64) -> Option<String> {
    let size = size.max(1.0) as u16;

    // The desktop entry is the indirection that makes this work: an agent is
    // known by its entry name, but the *icon* it points at often differs
    // (`code.desktop` carries `Icon=vscode`). Falling back to the name itself
    // covers entries whose icon happens to match, and apps with no entry.
    let icon_name = super::apps::find_desktop_entry(name)
        .and_then(|entry| entry.icon)
        .unwrap_or_else(|| name.to_string());

    let path = lookup(&icon_name, size)?;
    let png = rasterize(&path, size)?;

    let encoded = base64::engine::general_purpose::STANDARD.encode(png);
    Some(format!("data:image/png;base64,{encoded}"))
}

/// Finds an icon file in the active theme.
fn lookup(name: &str, size: u16) -> Option<PathBuf> {
    // An absolute path is a legal `Icon=` value and needs no theme lookup.
    let direct = Path::new(name);
    if direct.is_absolute() && direct.exists() {
        return Some(direct.to_path_buf());
    }

    let theme = current_theme();

    // The theme first, then hicolor, which the spec requires every theme to
    // fall back to and which is where third-party apps install.
    freedesktop_icons::lookup(name)
        .with_size(size)
        .with_theme(&theme)
        .find()
        .or_else(|| {
            freedesktop_icons::lookup(name)
                .with_size(size)
                .with_theme("hicolor")
                .find()
        })
        .or_else(|| freedesktop_icons::lookup(name).find())
}

/// The icon theme the user actually has set.
///
/// Read from kdeglobals first — this port targets Plasma, and asking gsettings
/// there returns GNOME's default rather than Breeze, which then fails to find
/// anything.
fn current_theme() -> String {
    if let Some(config) = dirs::config_dir() {
        if let Ok(text) = std::fs::read_to_string(config.join("kdeglobals")) {
            let mut in_icons = false;
            for line in text.lines() {
                let line = line.trim();
                if line.starts_with('[') {
                    in_icons = line == "[Icons]";
                } else if in_icons {
                    if let Some(theme) = line.strip_prefix("Theme=") {
                        return theme.trim().to_string();
                    }
                }
            }
        }
    }

    std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "icon-theme"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().trim_matches('\'').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "hicolor".into())
}

/// Loads an icon file and returns it as PNG bytes at roughly `size`.
fn rasterize(path: &Path, size: u16) -> Option<Vec<u8>> {
    let is_svg = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("svg") || e.eq_ignore_ascii_case("svgz"));

    if !is_svg {
        // Already pixels. Handed back untouched rather than re-encoded: the
        // theme picked the size, and a round trip through a decoder would only
        // lose sharpness.
        return std::fs::read(path).ok();
    }

    let data = std::fs::read(path).ok()?;
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(size as u32, size as u32)?;

    // Fit the drawing into the requested box, preserving aspect. Icons are
    // square in practice, but a non-square one should letterbox rather than
    // stretch.
    let tree_size = tree.size();
    let scale = (size as f32 / tree_size.width()).min(size as f32 / tree_size.height());
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);

    resvg::render(&tree, transform, &mut pixmap.as_mut());
    pixmap.encode_png().ok()
}
