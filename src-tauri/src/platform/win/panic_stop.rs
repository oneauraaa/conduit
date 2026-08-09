//! The Windows half of the panic stop: a low-level keyboard hook watching
//! Escape.
//!
//! The hold threshold and what happens when it trips live in
//! `crate::panic_stop`; this file only reports the key's up/down edges.
//!
//! ## Why a hook and not a hotkey
//!
//! `RegisterHotKey(VK_ESCAPE)` would swallow Escape system-wide — every dialog,
//! every vim session, every video player would stop seeing it.
//! `WH_KEYBOARD_LL` observes without consuming, as long as the callback always
//! hands the event on through `CallNextHookEx`.
//!
//! Unlike macOS this needs no permission at all, so it installs on the first
//! attempt and `lib.rs`'s retry loop exits immediately.

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, HHOOK, KBDLLHOOKSTRUCT, MSG, SetWindowsHookExW,
    TranslateMessage, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;

unsafe extern "system" fn callback(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // A negative code means "not ours to inspect"; the documented contract is
    // to pass it straight on without looking at the payload.
    if code >= 0 {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };

        if info.vkCode == VK_ESCAPE.0 as u32 {
            match wparam.0 as u32 {
                WM_KEYDOWN | WM_SYSKEYDOWN => crate::panic_stop::escape_down(),
                WM_KEYUP | WM_SYSKEYUP => crate::panic_stop::escape_up(),
                _ => {}
            }
        }
    }

    // Listen-only: hand every event straight on, untouched. Returning anything
    // else here would eat Escape for the whole machine.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Installs the hook on a dedicated thread with its own message pump.
///
/// The pump is not optional. Windows delivers low-level hook callbacks by
/// posting to the installing thread's message queue, so a thread that never
/// pumps simply never sees a key — and after `LowLevelHooksTimeout` (300ms by
/// default) Windows quietly unhooks it for being unresponsive. That failure is
/// silent: the hook handle stays valid and the callback just stops firing.
pub fn install_hook() -> bool {
    let (tx, rx) = std::sync::mpsc::channel::<bool>();

    let spawned = std::thread::Builder::new()
        .name("conduit-panic-stop".into())
        .spawn(move || {
            let hook: Option<HHOOK> = unsafe {
                SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(callback),
                    // NULL rather than this module's handle: for a low-level
                    // hook the procedure lives in the calling process, which is
                    // the documented form.
                    None,
                    0,
                )
            }
            .ok();

            if hook.is_none() {
                tracing::warn!("could not install the low-level keyboard hook");
                let _ = tx.send(false);
                return;
            }

            tracing::info!("panic-stop keyboard hook installed");
            let _ = tx.send(true);

            // Pump until the process exits. `GetMessageW` blocks, so this
            // thread costs nothing while idle.
            let mut msg = MSG::default();
            while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        })
        .is_ok();

    if !spawned {
        return false;
    }

    rx.recv_timeout(std::time::Duration::from_secs(3))
        .unwrap_or(false)
}
