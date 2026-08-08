//! The panic stop: hold Escape to take the machine back.
//!
//! ## Why a passive tap and not a global shortcut
//!
//! Registering Escape as a global shortcut would swallow it system-wide —
//! every dialog, every vim session, every video player would stop seeing the
//! key. Instead this installs a **listen-only** CGEventTap, which observes key
//! events without consuming them, and measures how long Escape is held.
//!
//! A tap needs a CFRunLoop, so it lives on its own thread. Requires the same
//! Accessibility grant the input tools already need; if that's missing the tap
//! simply fails to install and the Stop button in the pill remains the way out.

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};

use objc2_core_foundation::{CFMachPort, CFRetained, CFRunLoop, kCFRunLoopCommonModes};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventMask, CGEventTapLocation, CGEventTapOptions,
    CGEventTapPlacement, CGEventTapProxy, CGEventType,
};

use crate::state::Shared;

/// ANSI virtual keycode for Escape.
const ESC: i64 = 53;

/// How long Escape must be held before control is dropped. Long enough that a
/// normal "dismiss this dialog" tap never triggers it.
const HOLD_MS: u64 = 800;

/// Millis when Escape went down, or 0 when it isn't held. A static because the
/// tap callback is a bare `extern "C" fn` with no room for captured state.
static ESC_DOWN_AT: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    crate::state::now_millis()
}

unsafe extern "C-unwind" fn callback(
    _proxy: CGEventTapProxy,
    event_type: CGEventType,
    event: NonNull<CGEvent>,
    _user: *mut c_void,
) -> *mut CGEvent {
    let keycode = CGEvent::integer_value_field(
        Some(unsafe { event.as_ref() }),
        CGEventField::KeyboardEventKeycode,
    );

    if keycode == ESC {
        match event_type {
            CGEventType::KeyDown => {
                // Auto-repeat fires KeyDown repeatedly; only the first matters.
                let _ = ESC_DOWN_AT.compare_exchange(
                    0,
                    now_ms(),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                );
            }
            CGEventType::KeyUp => ESC_DOWN_AT.store(0, Ordering::Relaxed),
            _ => {}
        }
    }

    // Listen-only: hand every event straight back untouched.
    event.as_ptr()
}

/// Installs the tap on a dedicated run-loop thread and starts the watcher.
///
/// Returns whether the tap could be created. A `false` here is not fatal — it
/// just means Accessibility hasn't been granted yet.
pub fn install(state: Shared) -> bool {
    if !super::permissions::accessibility_granted() {
        return false;
    }

    let (tx, rx) = std::sync::mpsc::channel::<bool>();

    std::thread::Builder::new()
        .name("conduit-panic-stop".into())
        .spawn(move || {
            let mask: CGEventMask =
                (1 << CGEventType::KeyDown.0 as u64) | (1 << CGEventType::KeyUp.0 as u64);

            let tap: Option<CFRetained<CFMachPort>> = unsafe {
                CGEvent::tap_create(
                    CGEventTapLocation::HIDEventTap,
                    CGEventTapPlacement::HeadInsertEventTap,
                    // ListenOnly is the whole point: observe, never consume.
                    CGEventTapOptions::ListenOnly,
                    mask,
                    Some(callback),
                    std::ptr::null_mut(),
                )
            };

            let Some(tap) = tap else {
                let _ = tx.send(false);
                return;
            };

            let source = CFMachPort::new_run_loop_source(None, Some(&tap), 0);
            let Some(source) = source else {
                let _ = tx.send(false);
                return;
            };

            let run_loop = CFRunLoop::current();
            let Some(run_loop) = run_loop else {
                let _ = tx.send(false);
                return;
            };

            unsafe {
                run_loop.add_source(Some(&source), kCFRunLoopCommonModes);
                CGEvent::tap_enable(&tap, true);
            }

            let _ = tx.send(true);
            CFRunLoop::run();
        })
        .ok();

    let installed = rx
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap_or(false);

    if installed {
        spawn_watcher(state);
    }
    installed
}

/// Polls the held-duration and trips the stop once the threshold is crossed.
///
/// Polling rather than firing from the tap callback keeps the callback free of
/// locks and allocation — it runs on the HID event path, where blocking would
/// stutter the whole system's input.
fn spawn_watcher(state: Shared) {
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
        let mut fired = false;

        loop {
            tick.tick().await;
            let down_at = ESC_DOWN_AT.load(Ordering::Relaxed);

            if down_at == 0 {
                fired = false;
                continue;
            }
            if fired {
                continue;
            }

            if now_ms().saturating_sub(down_at) >= HOLD_MS {
                fired = true;
                if state.control().phase == crate::state::ControlPhase::Active {
                    state.abort();
                    let _ = crate::mac::windows::notify(
                        "conduit stopped",
                        "you have control back.",
                    );
                }
            }
        }
    });
}
