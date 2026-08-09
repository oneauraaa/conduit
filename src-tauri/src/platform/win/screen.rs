//! Display geometry, in physical pixels on the virtual screen.
//!
//! Unlike macOS there is no coordinate flip to do — Windows already measures
//! from the top-left, y down, which is the space conduit speaks. The primary
//! monitor's top-left is the origin; monitors placed to its left or above have
//! negative coordinates, and the virtual screen is their union.

use std::sync::OnceLock;

use parking_lot::RwLock;
use windows::Win32::Foundation::{LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    MONITORINFOF_PRIMARY, SPI_GETWORKAREA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    SystemParametersInfoW,
};
use windows::core::BOOL;

use crate::platform::types::Display;

/// One monitor as Windows describes it, before it becomes a [`Display`].
#[derive(Clone, Copy)]
struct Monitor {
    handle: isize,
    bounds: RECT,
    work: RECT,
    dpi: u32,
    primary: bool,
}

/// `HMONITOR` for each index in the order [`displays`] returns.
///
/// Capture needs the handle, but `Display` is a serialized wire type shared
/// with macOS and has no business carrying a Win32 handle. Keeping the mapping
/// beside the enumeration is the smaller compromise.
fn handles() -> &'static RwLock<Vec<isize>> {
    static HANDLES: OnceLock<RwLock<Vec<isize>>> = OnceLock::new();
    HANDLES.get_or_init(|| RwLock::new(Vec::new()))
}

unsafe extern "system" fn collect(
    monitor: HMONITOR,
    _hdc: HDC,
    _clip: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let out = unsafe { &mut *(data.0 as *mut Vec<Monitor>) };

    let mut info = MONITORINFOEXW {
        monitorInfo: MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFOEXW>() as u32,
            ..Default::default()
        },
        ..Default::default()
    };

    if unsafe { GetMonitorInfoW(monitor, &mut info as *mut _ as *mut MONITORINFO) }.as_bool() {
        // A monitor can refuse to report its DPI; 96 (100%) is the honest
        // fallback and only affects the informational `scale` field.
        let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
        let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };

        out.push(Monitor {
            handle: monitor.0 as isize,
            bounds: info.monitorInfo.rcMonitor,
            work: info.monitorInfo.rcWork,
            dpi: dpi_x,
            primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        });
    }

    TRUE
}

fn enumerate() -> Vec<Monitor> {
    let mut found: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut found as *mut _ as isize),
        );
    }

    // `EnumDisplayMonitors` makes no promise about order, and conduit's overlay
    // windows are labelled `overlay-0`, `overlay-1`… by index. Left unsorted,
    // those labels would shuffle between runs and cursor events would be
    // emitted to the overlay on the wrong screen. Primary first — matching
    // macOS, where index 0 is always primary — then left to right, top to
    // bottom.
    found.sort_by(|a, b| {
        b.primary
            .cmp(&a.primary)
            .then(a.bounds.left.cmp(&b.bounds.left))
            .then(a.bounds.top.cmp(&b.bounds.top))
    });
    found
}

/// Every attached display. Index 0 is always primary, matching macOS.
pub fn displays() -> Vec<Display> {
    let monitors = enumerate();

    *handles().write() = monitors.iter().map(|m| m.handle).collect();

    monitors
        .iter()
        .enumerate()
        .map(|(index, m)| Display {
            index,
            x: m.bounds.left as f64,
            y: m.bounds.top as f64,
            width: (m.bounds.right - m.bounds.left) as f64,
            height: (m.bounds.bottom - m.bounds.top) as f64,
            scale: m.dpi as f64 / 96.0,
            primary: m.primary,
        })
        .collect()
}

/// The `HMONITOR` at `index`, in the same order [`displays`] uses.
#[allow(dead_code)]
pub fn monitor_handle(index: usize) -> Result<HMONITOR, String> {
    // Enumerate if nothing has yet, so a capture that arrives before the first
    // refresh still works.
    if handles().read().is_empty() {
        let _ = displays();
    }

    handles()
        .read()
        .get(index)
        .map(|h| HMONITOR(*h as *mut _))
        .ok_or_else(|| format!("no display at index {index}"))
}

/// Where the control pill should sit: centred on the primary display's work
/// area, just above whatever the taskbar leaves free.
///
/// The work area is the exact counterpart of macOS's `visibleFrame` — it
/// already excludes the taskbar, so this lands correctly whether the taskbar is
/// on the bottom, the left, the right or the top, and it follows auto-hide.
///
/// `pill_w`/`pill_h` arrive as CSS pixels, so they are scaled to physical here
/// before being measured against a physical work area.
pub fn pill_anchor(pill_w: f64, pill_h: f64) -> (f64, f64) {
    // Prefer the primary monitor's own work area. `SPI_GETWORKAREA` reports
    // only the primary's, which is what we want, but going through the monitor
    // keeps the DPI beside it.
    let primary = enumerate().into_iter().find(|m| m.primary);

    let (work, scale) = match primary {
        Some(m) => (m.work, m.dpi as f64 / 96.0),
        None => {
            let mut r = RECT::default();
            let ok = unsafe {
                SystemParametersInfoW(
                    SPI_GETWORKAREA,
                    0,
                    Some(&mut r as *mut _ as *mut _),
                    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
                )
            }
            .is_ok();
            if !ok {
                return (0.0, 0.0);
            }
            (r, 1.0)
        }
    };

    let (pw, ph) = (pill_w * scale, pill_h * scale);
    let x = work.left as f64 + ((work.right - work.left) as f64 - pw) / 2.0;
    let y = work.bottom as f64 - ph - 8.0 * scale;

    (x, y)
}

/* ── bridging conduit's coordinate space to Tauri's ──────────── */

/// Windows coordinates are already physical, which is the unit Tauri positions
/// windows in. The macOS twin returns `Logical` instead; the pair is what keeps
/// `chrome.rs` free of `#[cfg]`.
pub fn tauri_position(x: f64, y: f64) -> tauri::Position {
    tauri::Position::Physical(tauri::PhysicalPosition::new(x as i32, y as i32))
}

pub fn tauri_size(w: f64, h: f64) -> tauri::Size {
    tauri::Size::Physical(tauri::PhysicalSize::new(w as u32, h as u32))
}

/// Tauri reports window geometry in physical units, and so does conduit on
/// Windows — so this is the identity. The macOS twin divides by the scale
/// factor. Getting this wrong is not cosmetic: `chrome::point_hits_conduit`
/// uses it to decide whether a click lands on conduit's own windows, and a
/// mis-scaled box would let an agent click the Tools tab.
pub fn physical_to_space(v: f64, _scale: f64) -> f64 {
    v
}

/// The screenshot scale to use when the caller didn't ask for one.
///
/// macOS bounds are logical points, so its default is 1.0. Windows bounds are
/// physical pixels, so dividing by the monitor's DPI scale lands in the same
/// place: a 150% 2560×1440 monitor captures at 1707×960, exactly as a Retina
/// panel captures at its point size. Models are billed per pixel and the
/// default should not depend on how the user configured their display.
pub fn default_capture_scale(display: &Display) -> f64 {
    if display.scale > 0.0 {
        1.0 / display.scale
    } else {
        1.0
    }
}
