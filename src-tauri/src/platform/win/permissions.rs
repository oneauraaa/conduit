//! Readiness checks.
//!
//! Windows has no TCC: nothing here is a permission the user grants to conduit,
//! and every capability is available the moment the app starts. What it does
//! have are conditions that silently change what works, and the Server tab
//! exists to surface exactly those.
//!
//! The one that matters is **UIPI**. A process cannot send synthetic input to,
//! or read the accessibility tree of, a window owned by a process at a higher
//! integrity level. An unelevated conduit driving Task Manager or an installer
//! will have its clicks discarded by the OS with no error at the API it called.
//! That is the same class of silent-wrong-answer the capture path is designed
//! to avoid, so it is reported here rather than discovered mid-session.

use windows::Foundation::Metadata::ApiInformation;
use windows::Graphics::Capture::GraphicsCaptureSession;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::{
    AreDpiAwarenessContextsEqual, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    DPI_AWARENESS_PER_MONITOR_AWARE, GetAwarenessFromDpiAwarenessContext,
    GetThreadDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use windows::core::HSTRING;

use crate::state::Readiness;

/// Whether conduit itself is running elevated.
pub fn elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }

        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
        .is_ok();

        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

/// Whether the window the user is actually looking at belongs to a process
/// conduit cannot touch.
///
/// Only the foreground window is checked. That keeps this cheap enough for the
/// Server tab's 2s poll, and the foreground window is precisely where the
/// restriction bites — an agent drives what's in front of it.
pub fn elevated_foreground() -> bool {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return false;
        }

        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return false;
        }

        // ERROR_ACCESS_DENIED here means the target sits at a higher integrity
        // level than conduit — which is exactly the condition being reported.
        match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(handle) => {
                let _ = CloseHandle(handle);
                false
            }
            Err(_) => true,
        }
    }
}

/// Whether the process is per-monitor-DPI-aware v2.
///
/// Everything conduit reports on Windows is in physical pixels, which is only
/// coherent under PMv2 — anything less and Windows virtualises coordinates
/// behind our back on a scaled monitor. The app manifest declares it, and tao
/// sets it again at `EventLoop::new`; this verifies the result rather than
/// trusting either.
pub fn dpi_aware() -> bool {
    unsafe {
        let ctx = GetThreadDpiAwarenessContext();

        // Compare both ways. `AreDpiAwarenessContextsEqual` is the documented
        // check, but the handle a *manifest*-declared awareness returns does
        // not always compare equal to the predefined V2 constant — whereas
        // `GetAwarenessFromDpiAwarenessContext` collapses it to a plain enum
        // that does. Trusting only the first reports a correctly-configured
        // process as broken.
        let exact = AreDpiAwarenessContextsEqual(ctx, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
            .as_bool();
        let awareness = GetAwarenessFromDpiAwarenessContext(ctx);

        let aware = exact || awareness == DPI_AWARENESS_PER_MONITOR_AWARE;
        if !aware {
            tracing::warn!(?awareness, "process is not per-monitor dpi aware");
        }
        aware
    }
}

/// Whether Windows.Graphics.Capture exists at all (Windows 10 1903+).
///
/// The COM guard is load-bearing. This is a WinRT activation, so on a thread
/// with no apartment it fails with `CO_E_NOTINITIALIZED` — which is
/// indistinguishable from "this Windows is too old" unless you look. Tauri
/// commands run on a thread pool where that is the normal case, so without
/// this the Server tab confidently tells the user screenshots are unsupported
/// on a machine where they work perfectly.
pub fn capture_supported() -> bool {
    let _com = super::com::Com::init();
    match GraphicsCaptureSession::IsSupported() {
        Ok(supported) => supported,
        Err(e) => {
            tracing::warn!("could not query capture support: {e}");
            false
        }
    }
}

/// Whether the capture session can suppress the yellow recording border
/// (Windows 11 build 22000+).
///
/// Queried through `ApiInformation` rather than by sniffing the build number,
/// which is the documented way and does not lie under compatibility shims.
pub fn borderless_capture() -> bool {
    let _com = super::com::Com::init();
    ApiInformation::IsPropertyPresent(
        &HSTRING::from("Windows.Graphics.Capture.GraphicsCaptureSession"),
        &HSTRING::from("IsBorderRequired"),
    )
    .unwrap_or(false)
}

pub fn snapshot() -> Readiness {
    Readiness::Windows {
        elevated: elevated(),
        elevated_foreground: elevated_foreground(),
        dpi_aware: dpi_aware(),
        capture_supported: capture_supported(),
        borderless_capture: borderless_capture(),
    }
}

/// Relaunches conduit as administrator.
///
/// Windows has no in-process way to gain elevation — a new process started with
/// the `runas` verb is the only route — so the old one must exit, or two
/// conduits fight over port 6767 and the second loses silently.
///
/// Cancelling the UAC prompt is a decision, not a failure: it returns
/// `ERROR_CANCELLED` and this reports success having done nothing.
pub fn relaunch_elevated(app: &tauri::AppHandle<tauri::Wry>) -> Result<(), String> {
    use windows::Win32::Foundation::ERROR_CANCELLED;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::w;

    if elevated() {
        return Ok(());
    }

    let exe = std::env::current_exe()
        .map_err(|e| format!("could not find conduit's own path: {e}"))?;
    let exe: HSTRING = exe.as_os_str().into();

    // ShellExecuteW's "success" is an HINSTANCE-shaped lie: values over 32 mean
    // it worked, everything at or below is an error code.
    let result = unsafe { ShellExecuteW(None, w!("runas"), &exe, None, None, SW_SHOWNORMAL) };
    let code = result.0 as usize as u32;

    if code > 32 {
        // The new process is coming up and will bind the port. Stand down.
        app.exit(0);
        Ok(())
    } else if code == ERROR_CANCELLED.0 {
        Ok(())
    } else {
        Err(format!("could not restart as administrator (error {code})"))
    }
}

/* ── the macOS grant prompts, which have no Windows counterpart ── */

pub fn accessibility_granted() -> bool {
    true
}

pub fn prompt_accessibility() -> bool {
    true
}

pub fn screen_recording_granted() -> bool {
    true
}

pub fn prompt_screen_recording() -> bool {
    true
}
