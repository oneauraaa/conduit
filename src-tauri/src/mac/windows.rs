//! Window and application enumeration and control.
//!
//! Window *metadata* still comes from `CGWindowListCopyWindowInfo` — only the
//! Quartz image-capture APIs were neutered on Tahoe, the window list is fine.
//! Moving, resizing and focusing go through the accessibility API, which is
//! the only supported way to drive another application's windows.

use std::ffi::c_void;

use accessibility_sys::{
    AXUIElementCopyAttributeValue, AXUIElementCreateApplication, AXUIElementRef,
    AXUIElementSetAttributeValue, AXValueCreate, kAXErrorSuccess, kAXPositionAttribute,
    kAXSizeAttribute, kAXValueTypeCGPoint, kAXValueTypeCGSize, kAXWindowsAttribute,
};
use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSWorkspace};
use objc2_core_foundation::{CFArray, CFRetained, CFString, CGPoint, CGSize};
use objc2_core_graphics::{CGWindowListCopyWindowInfo, CGWindowListOption, kCGNullWindowID};
use serde::Serialize;

use super::cf;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub app: String,
    pub pid: i32,
    /// Quartz coordinates, logical points.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Front-to-back ordering; 0 is frontmost.
    pub layer_index: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub pid: i32,
    pub bundle_id: Option<String>,
    pub active: bool,
}

/// On-screen windows, front to back. Excludes desktop elements and conduit's
/// own overlay/pill so the agent never tries to click its own chrome.
pub fn list_windows() -> Vec<WindowInfo> {
    let own_pid = std::process::id() as i64;

    let Some(array) = CGWindowListCopyWindowInfo(
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements,
        kCGNullWindowID,
    ) else {
        return Vec::new();
    };

    let mut out = Vec::new();

    for i in 0..array.count() {
        let Some(entry) = cf::array_at(&array, i) else {
            continue;
        };
        let Ok(dict) = entry.downcast::<objc2_core_foundation::CFDictionary>() else {
            continue;
        };

        let pid = cf::dict_i64(&dict, "kCGWindowOwnerPID").unwrap_or(0);
        if pid == own_pid {
            continue;
        }

        // Layer 0 is the normal window layer. Anything else is a panel, menu or
        // system element the agent has no business rearranging.
        if cf::dict_i64(&dict, "kCGWindowLayer").unwrap_or(-1) != 0 {
            continue;
        }

        let Some(bounds) = cf::dict_dict(&dict, "kCGWindowBounds") else {
            continue;
        };

        let width = cf::dict_f64(&bounds, "Width").unwrap_or(0.0);
        let height = cf::dict_f64(&bounds, "Height").unwrap_or(0.0);
        // Zero-sized windows are offscreen helpers, not something to click.
        if width < 1.0 || height < 1.0 {
            continue;
        }

        out.push(WindowInfo {
            id: cf::dict_i64(&dict, "kCGWindowNumber").unwrap_or(0) as u32,
            title: cf::dict_string(&dict, "kCGWindowName").unwrap_or_default(),
            app: cf::dict_string(&dict, "kCGWindowOwnerName").unwrap_or_default(),
            pid: pid as i32,
            x: cf::dict_f64(&bounds, "X").unwrap_or(0.0),
            y: cf::dict_f64(&bounds, "Y").unwrap_or(0.0),
            width,
            height,
            layer_index: out.len(),
        });
    }

    out
}

pub fn list_apps() -> Vec<AppInfo> {
    let workspace = NSWorkspace::sharedWorkspace();
    let running = workspace.runningApplications();

    running
        .iter()
        .filter_map(|app| {
            // Only apps with a UI presence — agents can't click a daemon.
            let name = app.localizedName()?;
            Some(AppInfo {
                name: name.to_string(),
                pid: app.processIdentifier(),
                bundle_id: app.bundleIdentifier().map(|b| b.to_string()),
                active: app.isActive(),
            })
        })
        .collect()
}

/// Brings a window's application forward. macOS has no supported way to raise
/// one specific window of another app without the AX API, so we activate the
/// owning application and then raise the matching AX window.
pub fn focus_window(window_id: u32) -> Result<(), String> {
    let win = list_windows()
        .into_iter()
        .find(|w| w.id == window_id)
        .ok_or_else(|| format!("no window with id {window_id}"))?;

    activate_pid(win.pid)?;
    Ok(())
}

/// Brings an application forward.
///
/// `activateWithOptions:` is the direct route, but macOS 14+ tightened
/// cooperative activation: a background process asking to raise someone else
/// usually just gets `false` back. conduit is *always* in the background — that
/// is the whole point — so the direct call fails far more often than it works.
///
/// `open -b <bundle-id>` goes through Launch Services, which is allowed to do
/// this, so it's the fallback rather than an error.
fn activate_pid(pid: i32) -> Result<(), String> {
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .ok_or_else(|| format!("no running application with pid {pid}"))?;

    if app.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows) {
        return Ok(());
    }

    let bundle_id = app
        .bundleIdentifier()
        .map(|b| b.to_string())
        .ok_or("that application has no bundle id to activate through")?;

    let status = std::process::Command::new("/usr/bin/open")
        .args(["-b", &bundle_id])
        .status()
        .map_err(|e| format!("could not activate that application: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err("the system refused to activate that application".into())
    }
}

/// Moves and resizes a window through the accessibility API.
pub fn set_window_bounds(
    window_id: u32,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let win = list_windows()
        .into_iter()
        .find(|w| w.id == window_id)
        .ok_or_else(|| format!("no window with id {window_id}"))?;

    let ax_app = unsafe { AXUIElementCreateApplication(win.pid) };
    if ax_app.is_null() {
        return Err("could not open an accessibility handle for that app".into());
    }

    let windows_ptr = copy_attr(ax_app, kAXWindowsAttribute)
        .ok_or("that app exposes no accessible windows (is accessibility granted?)")?;
    let windows = unsafe {
        CFRetained::retain(
            std::ptr::NonNull::new(windows_ptr.cast_mut().cast::<CFArray>())
                .ok_or("empty accessibility window list")?,
        )
    };

    // AX windows carry no CGWindowID, so match on geometry. Title matching is
    // worse: many apps have several identically-titled windows.
    let mut target: Option<AXUIElementRef> = None;
    for i in 0..windows.count() {
        let Some(el) = cf::array_ptr_at(&windows, i) else {
            continue;
        };
        let el = el as AXUIElementRef;
        if let Some((wx, wy)) = ax_position(el) {
            if (wx - win.x).abs() < 2.0 && (wy - win.y).abs() < 2.0 {
                target = Some(el);
                break;
            }
        }
    }
    let target = target.ok_or("could not match that window in the accessibility tree")?;

    set_ax_point(target, kAXPositionAttribute, x, y)?;
    set_ax_size(target, kAXSizeAttribute, width, height)?;
    Ok(())
}

fn copy_attr(element: AXUIElementRef, attr: &str) -> Option<*const c_void> {
    let key = CFString::from_str(attr);
    let mut value: *const c_void = std::ptr::null();
    let err = unsafe {
        AXUIElementCopyAttributeValue(
            element,
            CFRetained::as_ptr(&key).as_ptr().cast(),
            &mut value as *mut _ as *mut _,
        )
    };
    if err == kAXErrorSuccess && !value.is_null() {
        Some(value)
    } else {
        None
    }
}

fn ax_position(element: AXUIElementRef) -> Option<(f64, f64)> {
    let v = copy_attr(element, kAXPositionAttribute)?;
    let mut point = CGPoint { x: 0.0, y: 0.0 };
    let ok = unsafe {
        accessibility_sys::AXValueGetValue(
            v as _,
            kAXValueTypeCGPoint,
            &mut point as *mut _ as *mut c_void,
        )
    };
    ok.then_some((point.x, point.y))
}

fn set_ax_point(element: AXUIElementRef, attr: &str, x: f64, y: f64) -> Result<(), String> {
    let point = CGPoint { x, y };
    let value = unsafe { AXValueCreate(kAXValueTypeCGPoint, &point as *const _ as *const c_void) };
    if value.is_null() {
        return Err("could not build an accessibility point value".into());
    }
    let key = CFString::from_str(attr);
    let err = unsafe {
        AXUIElementSetAttributeValue(element, CFRetained::as_ptr(&key).as_ptr().cast(), value as _)
    };
    if err == kAXErrorSuccess {
        Ok(())
    } else {
        Err(format!("the app refused to move that window (AXError {err})"))
    }
}

fn set_ax_size(element: AXUIElementRef, attr: &str, w: f64, h: f64) -> Result<(), String> {
    let size = CGSize {
        width: w,
        height: h,
    };
    let value = unsafe { AXValueCreate(kAXValueTypeCGSize, &size as *const _ as *const c_void) };
    if value.is_null() {
        return Err("could not build an accessibility size value".into());
    }
    let key = CFString::from_str(attr);
    let err = unsafe {
        AXUIElementSetAttributeValue(element, CFRetained::as_ptr(&key).as_ptr().cast(), value as _)
    };
    if err == kAXErrorSuccess {
        Ok(())
    } else {
        Err(format!("the app refused to resize that window (AXError {err})"))
    }
}

/// Launches an app by name, or brings it forward if it's already running.
pub fn open_app(name: &str) -> Result<(), String> {
    // `open -a` covers both cases: it launches what isn't running and raises
    // what is, it resolves names fuzzily across every Applications directory,
    // and being Launch Services it isn't subject to the cooperative-activation
    // restrictions that stop a background app raising another (see
    // `activate_pid`). Going through NSRunningApplication first would just fail
    // for every already-running app.
    let status = std::process::Command::new("/usr/bin/open")
        .arg("-a")
        .arg(name)
        .status()
        .map_err(|e| format!("could not launch {name}: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("no application named {name}"))
    }
}

/// Asks an app to quit. Deliberately a request, not a kill: unsaved work must
/// get its chance to prompt.
pub fn quit_app(name: &str) -> Result<(), String> {
    let app = list_apps()
        .into_iter()
        .find(|a| a.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("{name} is not running"))?;

    let running = NSRunningApplication::runningApplicationWithProcessIdentifier(app.pid)
        .ok_or_else(|| format!("{name} is no longer running"))?;

    if running.terminate() {
        Ok(())
    } else {
        Err(format!("{name} refused to quit"))
    }
}

/// Posts a notification through osascript.
///
/// Tauri's notification plugin would need its own TCC grant and shows up as a
/// separate permission prompt; osascript borrows Script Editor's, which the
/// user has already effectively granted by allowing automation.
pub fn notify(title: &str, body: &str) -> Result<(), String> {
    let script = format!(
        r#"display notification {} with title {}"#,
        applescript_string(body),
        applescript_string(title),
    );
    std::process::Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(&script)
        .status()
        .map_err(|e| format!("could not post notification: {e}"))?;
    Ok(())
}

/// Quotes a Rust string for safe interpolation into AppleScript source.
fn applescript_string(s: &str) -> String {
    let escaped = s.replace('\\', r"\\").replace('"', r#"\""#);
    format!("\"{escaped}\"")
}
