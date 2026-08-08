//! Reads an installed application's icon.
//!
//! The Agents tab shows each agent's real logo. Rather than bundling other
//! companies' artwork, conduit asks macOS for the icon of the app already on
//! this machine — the same thing Finder draws. Nothing third-party ships inside
//! conduit, the icons are always the current ones, and an agent that isn't
//! installed simply has no icon to show.

use std::collections::HashMap;

use base64::Engine;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSWorkspace};
use objc2_foundation::{NSDictionary, NSSize, NSString};
use parking_lot::RwLock;

/// Bundle id -> rendered icon. Extraction costs 0.5–1.1s per app (it walks
/// Launch Services, then rasterizes and re-encodes the .icns), which made the
/// Agents tab hang for several seconds every time it opened. Icons don't change
/// while conduit is running, so once is enough.
static CACHE: RwLock<Option<HashMap<String, Option<String>>>> = RwLock::new(None);

/// Resolves a bundle id to its app, then renders that app's icon as a PNG data
/// URL at `size` points. Memoized — see [`CACHE`].
pub fn icon_data_url(bundle_id: &str, size: f64) -> Option<String> {
    if let Some(cache) = CACHE.read().as_ref() {
        if let Some(hit) = cache.get(bundle_id) {
            return hit.clone();
        }
    }

    let rendered = render_icon(bundle_id, size);

    CACHE
        .write()
        .get_or_insert_with(HashMap::new)
        .insert(bundle_id.to_string(), rendered.clone());

    rendered
}

/// Populates the cache ahead of the user opening the Agents tab.
///
/// Must run on the main thread: this is AppKit, and NSWorkspace/NSImage are not
/// safe to touch from a background thread.
pub fn warm_cache(bundle_ids: &[&str], size: f64) {
    for id in bundle_ids {
        let _ = icon_data_url(id, size);
    }
}

fn render_icon(bundle_id: &str, size: f64) -> Option<String> {
    let workspace = NSWorkspace::sharedWorkspace();
    let identifier = NSString::from_str(bundle_id);

    let url = workspace.URLForApplicationWithBundleIdentifier(&identifier)?;
    let path = url.path()?;

    let icon = workspace.iconForFile(&path);

    // NSImage holds several representations; pinning the size tells the draw
    // below which one to rasterize, otherwise we get whatever came first.
    icon.setSize(NSSize {
        width: size,
        height: size,
    });

    let tiff = icon.TIFFRepresentation()?;
    let rep = NSBitmapImageRep::imageRepWithData(&tiff)?;
    let empty = NSDictionary::new();
    let png = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &empty) }?;

    let encoded = base64::engine::general_purpose::STANDARD.encode(png.to_vec());
    Some(format!("data:image/png;base64,{encoded}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Extraction has to work against a real installed app — the failure mode
    /// this guards is silently returning None and quietly falling back to a
    /// monogram for every agent.
    #[test]
    fn reads_an_installed_apps_icon() {
        // Finder is on every Mac, so this test isn't machine-specific.
        let Some(url) = icon_data_url("com.apple.finder", 64.0) else {
            panic!("could not read Finder's icon");
        };
        assert!(url.starts_with("data:image/png;base64,"));

        let b64 = url.trim_start_matches("data:image/png;base64,");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("payload should be valid base64");
        assert_eq!(&bytes[..4], b"\x89PNG", "payload should be a real PNG");
        assert!(bytes.len() > 1000, "an app icon should not be a stub");
    }

    #[test]
    fn unknown_bundle_id_yields_none() {
        assert!(icon_data_url("com.example.definitely-not-installed", 64.0).is_none());
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    /// The Agents tab used to block for seconds because it re-extracted every
    /// icon on each open. This pins the memoization that fixed it.
    #[test]
    fn second_lookup_is_effectively_free() {
        let id = "com.apple.finder";

        let cold = std::time::Instant::now();
        let first = icon_data_url(id, 64.0);
        let cold = cold.elapsed();

        let warm = std::time::Instant::now();
        let second = icon_data_url(id, 64.0);
        let warm = warm.elapsed();

        assert!(first.is_some());
        assert_eq!(first, second, "cache must return the same bytes");
        assert!(
            warm.as_micros() * 20 < cold.as_micros().max(1),
            "cached lookup ({warm:?}) should be orders of magnitude under the cold one ({cold:?})"
        );
    }
}
