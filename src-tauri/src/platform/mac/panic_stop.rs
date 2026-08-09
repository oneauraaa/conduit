//! The macOS half of the panic stop: a listen-only CGEventTap watching Escape.
//!
//! The hold threshold and what happens when it trips live in
//! `crate::panic_stop`; this file only reports the key's up/down edges.
//!
//! A tap needs a CFRunLoop, so it lives on its own thread. It requires the same
//! Accessibility grant the input tools already need; if that's missing the tap
//! simply fails to install and the Stop button in the pill remains the way out.

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_core_foundation::{CFMachPort, CFRetained, CFRunLoop, kCFRunLoopCommonModes};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventMask, CGEventTapLocation, CGEventTapOptions,
    CGEventTapPlacement, CGEventTapProxy, CGEventType,
};

/// ANSI virtual keycode for Escape.
const ESC: i64 = 53;

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
            CGEventType::KeyDown => crate::panic_stop::escape_down(),
            CGEventType::KeyUp => crate::panic_stop::escape_up(),
            _ => {}
        }
    }

    // Listen-only: hand every event straight back untouched.
    event.as_ptr()
}

/// Installs the tap on a dedicated run-loop thread.
///
/// Returns whether the tap could be created. A `false` here is not fatal — it
/// just means Accessibility hasn't been granted yet.
pub fn install_hook() -> bool {
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

    rx.recv_timeout(std::time::Duration::from_secs(3))
        .unwrap_or(false)
}
