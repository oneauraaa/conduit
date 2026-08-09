//! Reads an installed application's icon.
//!
//! The Agents tab shows each agent's real logo. Rather than bundling other
//! companies' artwork, conduit asks Windows for the icon of the app already on
//! this machine — the same thing Explorer draws. Nothing third-party ships
//! inside conduit, the icons are always the current ones, and an agent that
//! isn't installed simply has no icon to show.
//!
//! The macOS twin keys on a bundle identifier. Windows has none, so the key is
//! an executable name resolved through `startmenu` — the registry's App Paths
//! first, then Start Menu shortcuts.

use std::collections::HashMap;

use base64::Engine;
use parking_lot::RwLock;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits,
    GetObjectW, HBITMAP, ReleaseDC,
};
use windows::Win32::UI::Shell::{
    SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGetFileInfoW,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};
use windows::core::HSTRING;

use super::com::Com;

/// App key -> rendered icon. Extraction walks the registry, rasterizes and
/// re-encodes, which made the Agents tab hang every time it opened. Icons don't
/// change while conduit is running, so once is enough.
static CACHE: RwLock<Option<HashMap<String, Option<String>>>> = RwLock::new(None);

/// Resolves an executable name to its app, then renders that app's icon as a
/// PNG data URL at `size` points. Memoized — see [`CACHE`].
pub fn icon_data_url(app_key: &str, size: f64) -> Option<String> {
    if let Some(cache) = CACHE.read().as_ref() {
        if let Some(hit) = cache.get(app_key) {
            return hit.clone();
        }
    }

    let rendered = render_icon(app_key, size);

    CACHE
        .write()
        .get_or_insert_with(HashMap::new)
        .insert(app_key.to_string(), rendered.clone());

    rendered
}

/// Populates the cache ahead of the user opening the Agents tab.
pub fn warm_cache(app_keys: &[&str], size: f64) {
    for key in app_keys {
        let _ = icon_data_url(key, size);
    }
}

fn render_icon(app_key: &str, size: f64) -> Option<String> {
    let _com = Com::init();

    let path = super::startmenu::resolve_path(app_key)?;

    let mut info = SHFILEINFOW::default();
    let got = unsafe {
        SHGetFileInfoW(
            &HSTRING::from(path.as_os_str()),
            windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        )
    };
    if got == 0 || info.hIcon.is_invalid() {
        return None;
    }

    let rgba = icon_to_rgba(info.hIcon);
    unsafe { DestroyIcon(info.hIcon).ok() };

    let (pixels, w, h) = rgba?;
    let img = image::RgbaImage::from_raw(w, h, pixels)?;

    let target_px = size.round().max(1.0) as u32;
    let scaled = image::imageops::resize(
        &img,
        target_px,
        target_px,
        image::imageops::FilterType::Lanczos3,
    );

    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            scaled.as_raw(),
            target_px,
            target_px,
            image::ExtendedColorType::Rgba8,
        )
        .ok()?;

    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&png)
    ))
}

use image::ImageEncoder;

/// Pulls RGBA pixels out of an `HICON`.
fn icon_to_rgba(icon: HICON) -> Option<(Vec<u8>, u32, u32)> {
    unsafe {
        let mut ii = ICONINFO::default();
        GetIconInfo(icon, &mut ii).ok()?;

        // Both bitmaps are owned by us now and must be released on every path.
        let result = (|| {
            let mut bm = BITMAP::default();
            if GetObjectW(
                ii.hbmColor.into(),
                std::mem::size_of::<BITMAP>() as i32,
                Some(&mut bm as *mut _ as *mut _),
            ) == 0
            {
                return None;
            }

            let (w, h) = (bm.bmWidth.max(0) as u32, bm.bmHeight.max(0) as u32);
            if w == 0 || h == 0 {
                return None;
            }

            let mut colour = read_bitmap(ii.hbmColor, w, h)?;

            // Modern icons carry a real alpha channel. Legacy ones (and a
            // surprising number of installers' icons) leave it entirely zero
            // and express transparency through the 1bpp mask instead. Trusting
            // the alpha blindly renders those as fully transparent rectangles —
            // half the Agents tab silently blank.
            let opaque = colour.chunks_exact(4).any(|px| px[3] != 0);
            if !opaque {
                if let Some(mask) = read_bitmap(ii.hbmMask, w, h) {
                    for (px, m) in colour.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
                        // In an icon mask, white (non-zero) means transparent.
                        px[3] = if m[0] == 0 { 255 } else { 0 };
                    }
                } else {
                    // No usable mask either; assume fully opaque, which at
                    // least shows the artwork.
                    for px in colour.chunks_exact_mut(4) {
                        px[3] = 255;
                    }
                }
            }

            Some((colour, w, h))
        })();

        if !ii.hbmColor.is_invalid() {
            let _ = DeleteObject(ii.hbmColor.into());
        }
        if !ii.hbmMask.is_invalid() {
            let _ = DeleteObject(ii.hbmMask.into());
        }

        result
    }
}

/// Reads a GDI bitmap as top-down 32bpp RGBA.
fn read_bitmap(bitmap: HBITMAP, w: u32, h: u32) -> Option<Vec<u8>> {
    if bitmap.is_invalid() {
        return None;
    }

    unsafe {
        let dc = GetDC(None);
        if dc.is_invalid() {
            return None;
        }

        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                // Negative height requests top-down rows. Without it the image
                // comes back vertically mirrored.
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut buf = vec![0u8; (w as usize) * (h as usize) * 4];
        let read = GetDIBits(
            dc,
            bitmap,
            0,
            h,
            Some(buf.as_mut_ptr() as *mut _),
            &mut info,
            DIB_RGB_COLORS,
        );

        ReleaseDC(Some(HWND::default()), dc);

        if read == 0 {
            return None;
        }

        // GDI hands back BGRA; the PNG encoder wants RGBA.
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        Some(buf)
    }
}
