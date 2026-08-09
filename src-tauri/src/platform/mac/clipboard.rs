//! Clipboard access via NSPasteboard.

use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::NSString;

pub fn read_text() -> Option<String> {
    let pb = NSPasteboard::generalPasteboard();
    unsafe { pb.stringForType(NSPasteboardTypeString) }.map(|s| s.to_string())
}

pub fn write_text(text: &str) -> Result<(), String> {
    let pb = NSPasteboard::generalPasteboard();
    // Every write must clear first; NSPasteboard rejects writes to a board
    // whose change count hasn't been bumped.
    pb.clearContents();

    let ns = NSString::from_str(text);
    let ok = unsafe { pb.setString_forType(&ns, NSPasteboardTypeString) };
    if ok {
        Ok(())
    } else {
        Err("the system rejected the clipboard write".into())
    }
}
