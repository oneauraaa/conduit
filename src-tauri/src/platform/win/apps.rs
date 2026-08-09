//! Window and application enumeration and control.

use std::collections::HashMap;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;

use parking_lot::RwLock;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, MAX_PATH, RECT, TRUE, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
};
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentProcessId, OpenProcess, PROCESS_NAME_FORMAT,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsHungAppWindow, IsIconic,
    IsWindow, IsWindowVisible, PostMessageW, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER,
    SW_RESTORE, SetForegroundWindow, SetWindowPos, ShowWindow, WM_CLOSE, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, PCWSTR, PWSTR};

use crate::platform::types::{AppInfo, WindowInfo};

/* ── enumeration ── */

/// On-screen windows, front to back.
///
/// `EnumWindows` walks in z-order, top first, so the iteration index *is* the
/// front-to-back ordering.
pub fn list_windows() -> Vec<WindowInfo> {
    let mut hwnds: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut hwnds as *mut _ as isize));
    }

    let own_pid = unsafe { GetCurrentProcessId() };
    let mut out = Vec::new();

    for hwnd in hwnds {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        // conduit's own overlay and pill must never be offered to an agent.
        if pid == own_pid || pid == 0 {
            continue;
        }

        let Some((x, y, w, h)) = window_bounds(hwnd) else {
            continue;
        };
        if w < 1.0 || h < 1.0 {
            continue;
        }

        out.push(WindowInfo {
            // Safe and reversible: Windows guarantees handle values are
            // 32-bit-significant even in 64-bit processes, precisely so they
            // can be passed between 32- and 64-bit code. It looks like a
            // truncation bug and is not — every use re-checks with `IsWindow`.
            id: hwnd.0 as usize as u32,
            title: window_title(hwnd),
            app: app_name_for_pid(pid),
            pid: pid as i32,
            x,
            y,
            width: w,
            height: h,
            layer_index: out.len(),
        });
    }

    out
}

unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> BOOL {
    let out = unsafe { &mut *(data.0 as *mut Vec<HWND>) };

    if is_listable(hwnd) {
        out.push(hwnd);
    }
    TRUE
}

/// Whether a window is something an agent could plausibly click.
fn is_listable(hwnd: HWND) -> bool {
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return false;
    }

    // **The most important Windows-specific filter.** Modern UWP apps keep
    // suspended, invisible shells around permanently — Settings, Calculator,
    // "Microsoft Text Input Application" — and every one of them is
    // `IsWindowVisible`. Without this check a listing is mostly ghosts, and an
    // agent will click coordinates where nothing is. It is the counterpart of
    // the macOS `kCGWindowLayer != 0` check.
    let mut cloaked = 0u32;
    let got = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut _ as *mut _,
            std::mem::size_of::<u32>() as u32,
        )
    };
    if got.is_ok() && cloaked != 0 {
        return false;
    }

    // Tool windows are palettes and helpers, not documents. This also excludes
    // conduit's own chrome, which sets WS_EX_TOOLWINDOW.
    let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    if ex & WS_EX_TOOLWINDOW.0 as isize != 0 {
        return false;
    }

    // An untitled top-level window is almost always an invisible message sink.
    (unsafe { GetWindowTextLengthW(hwnd) }) > 0
}

/// A window's visible bounds.
///
/// `DWMWA_EXTENDED_FRAME_BOUNDS`, not `GetWindowRect`. The latter includes the
/// invisible ~7px resize border Windows 10/11 draw around every window, so it
/// reports a window as noticeably larger than it looks — and a `list_windows`
/// → `set_window_bounds` round trip would drift outward every single time.
fn window_bounds(hwnd: HWND) -> Option<(f64, f64, f64, f64)> {
    let mut r = RECT::default();
    let dwm = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut _ as *mut _,
            std::mem::size_of::<RECT>() as u32,
        )
    };

    if dwm.is_err() {
        // Windows that predate DWM composition, and some full-screen exclusive
        // ones, have no extended frame. The raw rect is all there is.
        if unsafe { GetWindowRect(hwnd, &mut r) }.is_err() {
            return None;
        }
    }

    Some((
        r.left as f64,
        r.top as f64,
        (r.right - r.left) as f64,
        (r.bottom - r.top) as f64,
    ))
}

fn window_title(hwnd: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; len as usize + 1];
    let written = unsafe { GetWindowTextW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..written.max(0) as usize])
}

/* ── application names ── */

/// exe path -> display name. Reading a version resource costs a few
/// milliseconds and `list_windows` hits the same handful of executables over
/// and over, so this is memoised the same way `appicon` memoises icons.
fn name_cache() -> &'static RwLock<HashMap<PathBuf, String>> {
    static CACHE: std::sync::OnceLock<RwLock<HashMap<PathBuf, String>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn exe_path(pid: u32) -> Option<PathBuf> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = vec![0u16; MAX_PATH as usize * 2];
        let mut len = buf.len() as u32;

        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(handle);

        ok.then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
    }
}

/// The name a user would recognise, e.g. "Google Chrome" rather than
/// "chrome.exe".
///
/// This matters more than it looks: `list_apps`, `open_app`, `quit_app` and
/// `read_screen_text(app:)` all match on this string, so an agent told to
/// "close Chrome" has to be able to find it. Falls back to the file stem when
/// an executable carries no version resource.
fn app_name_for_pid(pid: u32) -> String {
    let Some(path) = exe_path(pid) else {
        return String::new();
    };

    if let Some(hit) = name_cache().read().get(&path) {
        return hit.clone();
    }

    let name = file_description(&path).unwrap_or_else(|| {
        path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    });

    name_cache().write().insert(path, name.clone());
    name
}

/// The `FileDescription` string from an executable's version resource.
fn file_description(path: &PathBuf) -> Option<String> {
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), None);
        if size == 0 {
            return None;
        }

        let mut block = vec![0u8; size as usize];
        GetFileVersionInfoW(
            PCWSTR(wide.as_ptr()),
            None,
            size,
            block.as_mut_ptr() as *mut _,
        )
        .ok()?;

        // The description is filed under a language/codepage pair, which has to
        // be read first — hardcoding 040904b9 works for US-English builds and
        // silently returns nothing for everyone else.
        let mut ptr = std::ptr::null_mut();
        let mut len = 0u32;
        let key: Vec<u16> = "\\VarFileInfo\\Translation\0".encode_utf16().collect();
        if !VerQueryValueW(
            block.as_ptr() as *const _,
            PCWSTR(key.as_ptr()),
            &mut ptr,
            &mut len,
        )
        .as_bool()
            || len < 4
        {
            return None;
        }

        let lang = *(ptr as *const u16);
        let codepage = *(ptr as *const u16).add(1);

        let sub = format!("\\StringFileInfo\\{lang:04x}{codepage:04x}\\FileDescription\0");
        let sub: Vec<u16> = sub.encode_utf16().collect();

        let mut val = std::ptr::null_mut();
        let mut val_len = 0u32;
        if !VerQueryValueW(
            block.as_ptr() as *const _,
            PCWSTR(sub.as_ptr()),
            &mut val,
            &mut val_len,
        )
        .as_bool()
            || val_len == 0
        {
            return None;
        }

        let slice = std::slice::from_raw_parts(val as *const u16, val_len as usize);
        let text = String::from_utf16_lossy(slice)
            .trim_end_matches('\0')
            .trim()
            .to_string();

        (!text.is_empty()).then_some(text)
    }
}

/// Running applications that have at least one real window.
///
/// Windows has no `NSWorkspace.runningApplications`, and enumerating every
/// process would bury the agent in services it can never interact with. The
/// windowed processes *are* the applications, so this derives them from the
/// window list.
pub fn list_apps() -> Vec<AppInfo> {
    let foreground_pid = {
        let hwnd = unsafe { GetForegroundWindow() };
        let mut pid = 0u32;
        if !hwnd.is_invalid() {
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        }
        pid as i32
    };

    let mut seen: Vec<AppInfo> = Vec::new();
    for w in list_windows() {
        if w.app.is_empty() || seen.iter().any(|a| a.pid == w.pid) {
            continue;
        }
        seen.push(AppInfo {
            name: w.app,
            pid: w.pid,
            // Windows has no bundle identifiers; the field stays so the tool
            // result keeps one shape across platforms.
            bundle_id: None,
            active: w.pid == foreground_pid,
        });
    }
    seen
}

/// What a window listing is quietly not telling the agent.
pub fn list_windows_hint(_windows: &[WindowInfo]) -> Option<String> {
    // Titles are readable without any grant on Windows, so the macOS
    // Screen-Recording caveat has no counterpart. UIPI restricts *driving*
    // elevated windows, not seeing them, and `input::blocked_reason` reports
    // that at the moment it actually bites.
    None
}

/* ── control ── */

fn hwnd_for(window_id: u32) -> Result<HWND, String> {
    let hwnd = HWND(window_id as usize as *mut _);
    if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        Ok(hwnd)
    } else {
        Err(format!("no window with id {window_id}"))
    }
}

/// Brings a window to the front.
///
/// `SetForegroundWindow` alone is unreliable by design: Windows only lets the
/// process that *owns* the foreground window hand it over, and conduit is
/// always in the background — which is the whole point. Briefly attaching to
/// the foreground thread's input queue makes conduit a legitimate caller for
/// the duration.
///
/// The other common workaround — synthesizing an Alt keypress — is deliberately
/// not used: that keystroke lands in whatever app currently has focus and pops
/// its menu bar, which is a visible, confusing side effect on someone's screen.
pub fn focus_window(window_id: u32) -> Result<(), String> {
    let hwnd = hwnd_for(window_id)?;

    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }

        let foreground = GetForegroundWindow();
        // Attaching to a hung window's thread hangs *this* thread too, with no
        // timeout and no way out.
        let safe_to_attach = !foreground.is_invalid() && !IsHungAppWindow(foreground).as_bool();

        let (mut from, mut to) = (0u32, 0u32);
        if safe_to_attach {
            from = GetWindowThreadProcessId(foreground, None);
            to = GetWindowThreadProcessId(hwnd, None);
            if from != to && from != 0 && to != 0 {
                let _ = AttachThreadInput(from, to, true);
            }
        }

        let raised = SetForegroundWindow(hwnd).as_bool();

        if safe_to_attach && from != to && from != 0 && to != 0 {
            let _ = AttachThreadInput(from, to, false);
        }

        if raised {
            Ok(())
        } else {
            Err("windows refused to raise that window".into())
        }
    }
}

/// Moves and resizes a window.
///
/// `SetWindowPos` positions the *window rect*, which includes the invisible
/// resize border, while [`window_bounds`] reports the visible frame. Applying
/// the difference is what makes a `list_windows` → `set_window_bounds` round
/// trip land exactly where it started instead of creeping outward each time.
pub fn set_window_bounds(
    window_id: u32,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let hwnd = hwnd_for(window_id)?;

    let mut raw = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut raw) }.is_err() {
        return Err("could not read that window's current bounds".into());
    }
    let visible = window_bounds(hwnd).ok_or("could not read that window's visible bounds")?;

    // Positive when the raw rect is larger than the visible frame, which is the
    // normal case on Windows 10/11.
    let pad_left = visible.0 - raw.left as f64;
    let pad_top = visible.1 - raw.top as f64;
    let pad_w = (raw.right - raw.left) as f64 - visible.2;
    let pad_h = (raw.bottom - raw.top) as f64 - visible.3;

    unsafe {
        SetWindowPos(
            hwnd,
            None,
            (x - pad_left).round() as i32,
            (y - pad_top).round() as i32,
            (width + pad_w).round() as i32,
            (height + pad_h).round() as i32,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        )
    }
    .map_err(|e| format!("that window refused to move or resize ({e})"))
}

/// Launches an app by name, or brings it forward if it's already running.
pub fn open_app(name: &str) -> Result<(), String> {
    // Already running? Raising beats launching a second copy.
    if let Some(app) = list_apps()
        .into_iter()
        .find(|a| a.name.eq_ignore_ascii_case(name))
    {
        if let Some(w) = list_windows().into_iter().find(|w| w.pid == app.pid) {
            return focus_window(w.id);
        }
    }

    let target = super::startmenu::resolve(name)
        .ok_or_else(|| format!("no application named {name} was found"))?;

    super::startmenu::launch(&target)
}

/// Asks an app to quit.
///
/// Deliberately a request, not a kill: `WM_CLOSE` is what clicking the X does,
/// so unsaved work still gets its chance to prompt — matching the macOS
/// contract, which uses `terminate` rather than `kill`.
///
/// `PostMessageW`, never `SendMessageW`. `SendMessage` blocks until the target
/// finishes handling it, so an app that answers by opening a "save changes?"
/// dialog would hang conduit until a human dismissed it.
pub fn quit_app(name: &str) -> Result<(), String> {
    let app = list_apps()
        .into_iter()
        .find(|a| a.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("{name} is not running"))?;

    let windows: Vec<WindowInfo> = list_windows()
        .into_iter()
        .filter(|w| w.pid == app.pid)
        .collect();

    if windows.is_empty() {
        return Err(format!("{name} has no window to close"));
    }

    for w in windows {
        if let Ok(hwnd) = hwnd_for(w.id) {
            let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
        }
    }
    Ok(())
}

/// Posts a notification.
pub fn notify(title: &str, body: &str) -> Result<(), String> {
    use tauri_winrt_notification::Toast;

    // A toast is addressed to an Application User Model ID, and only an
    // installed app has one — the installer's Start Menu shortcut is what
    // registers it. From `tauri tauri dev` there is no shortcut and no AUMID,
    // so borrowing PowerShell's is the difference between a working
    // notification and a silent no-op during development.
    Toast::new(Toast::POWERSHELL_APP_ID)
        .title(title)
        .text1(body)
        .show()
        .map_err(|e| format!("could not post notification: {e}"))
}
