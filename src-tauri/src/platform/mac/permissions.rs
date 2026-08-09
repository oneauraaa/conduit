//! TCC permission checks.
//!
//! conduit needs two grants that macOS will not give silently:
//!   - **Accessibility**, for synthesizing input and reading the AX tree;
//!   - **Screen Recording**, for screenshots.
//!
//! Both are per-bundle, which is why the app has to be run from a real `.app`
//! — a bare binary won't even appear in System Settings on Tahoe.

use accessibility_sys::{AXIsProcessTrusted, AXIsProcessTrustedWithOptions};
use objc2_core_foundation::{CFBoolean, CFDictionary, CFRetained, CFString};
use objc2_core_graphics::{CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess};

use crate::state::Readiness;

pub fn accessibility_granted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Checks accessibility and, if it isn't granted, asks macOS to show its
/// "open System Settings" prompt. macOS only shows that prompt once per bundle,
/// so the Server tab keeps a deep link around for every time after the first.
pub fn prompt_accessibility() -> bool {
    // Literal rather than the `kAXTrustedCheckOptionPrompt` extern: that symbol
    // is typed against core-foundation-sys, and threading its CFStringRef into
    // an objc2-core-foundation dictionary means casting between two binding
    // styles. The key's value is stable public API.
    let key = CFString::from_static_str("AXTrustedCheckOptionPrompt");
    let value = CFBoolean::new(true);

    let options: CFRetained<CFDictionary<CFString, CFBoolean>> =
        CFDictionary::from_slices(&[&*key], &[&*value]);

    unsafe { AXIsProcessTrustedWithOptions(CFRetained::as_ptr(&options).as_ptr().cast()) }
}

pub fn screen_recording_granted() -> bool {
    CGPreflightScreenCaptureAccess()
}

/// Triggers the system's screen-recording consent dialog. Returns the grant
/// state; macOS often requires an app relaunch before it takes effect.
pub fn prompt_screen_recording() -> bool {
    CGRequestScreenCaptureAccess()
}

pub fn snapshot() -> Readiness {
    Readiness::MacOS {
        accessibility: accessibility_granted(),
        screen_recording: screen_recording_granted(),
    }
}

/// No-op on macOS: there is no elevation to gain, and the two things conduit
/// needs are TCC grants the user makes in System Settings instead.
pub fn relaunch_elevated(_app: &tauri::AppHandle<tauri::Wry>) -> Result<(), String> {
    Err("elevation is a Windows concept; macOS uses the Accessibility and Screen Recording grants".into())
}
