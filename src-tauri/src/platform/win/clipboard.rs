//! The clipboard.
//!
//! Raw Win32 rather than a crate: it is seventy lines, and the obvious
//! candidate (`arboard`) drags in X11 machinery that would have to be fought
//! the moment anyone tries a Linux build.
//!
//! The clipboard is a single machine-wide resource that must be opened,
//! touched and closed quickly — another process holding it open makes
//! `OpenClipboard` fail, which is normal rather than exceptional, so both
//! functions retry briefly instead of giving up on the first refusal.

use std::ffi::c_void;

use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;

/// Opens the clipboard, retrying while another process has it.
///
/// Returns a guard so no path can forget `CloseClipboard` — leaving it open
/// locks every other application out of copy and paste until conduit exits.
struct Clipboard;

impl Clipboard {
    fn open() -> Option<Self> {
        for attempt in 0..10 {
            if unsafe { OpenClipboard(None) }.is_ok() {
                return Some(Clipboard);
            }
            std::thread::sleep(std::time::Duration::from_millis(10 * (attempt + 1)));
        }
        None
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        let _ = unsafe { CloseClipboard() };
    }
}

pub fn read_text() -> Result<Option<String>, String> {
    let Some(_guard) = Clipboard::open() else {
        return Err("another application is holding the clipboard open".into());
    };

    let Ok(handle) = (unsafe { GetClipboardData(CF_UNICODETEXT.0 as u32) }) else {
        return Ok(None);
    };
    if handle.is_invalid() {
        return Ok(None);
    }

    let hglobal = HGLOBAL(handle.0);
    let ptr = unsafe { GlobalLock(hglobal) } as *const u16;
    if ptr.is_null() {
        return Ok(None);
    }

    // The buffer is NUL-terminated UTF-16; walk to the terminator.
    let mut len = 0usize;
    // A sane ceiling so a corrupt clipboard entry can't walk off the heap.
    const MAX_UNITS: usize = 64 * 1024 * 1024;
    while len < MAX_UNITS && unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }

    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    let text = String::from_utf16_lossy(slice);

    let _ = unsafe { GlobalUnlock(hglobal) };
    Ok(Some(text))
}

pub fn write_text(text: &str) -> Result<(), String> {
    let _guard = Clipboard::open().ok_or("another application is holding the clipboard open")?;

    let mut utf16: Vec<u16> = text.encode_utf16().collect();
    utf16.push(0);
    let bytes = utf16.len() * std::mem::size_of::<u16>();

    let hglobal = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) }
        .map_err(|e| format!("could not allocate for the clipboard: {e}"))?;

    let dst = unsafe { GlobalLock(hglobal) } as *mut u16;
    if dst.is_null() {
        return Err("could not lock the clipboard buffer".into());
    }
    unsafe {
        std::ptr::copy_nonoverlapping(utf16.as_ptr(), dst, utf16.len());
        let _ = GlobalUnlock(hglobal);
    }

    unsafe { EmptyClipboard() }.map_err(|e| format!("could not clear the clipboard: {e}"))?;

    // On success the clipboard owns the memory — it must not be freed here.
    // On failure it does not, and the allocation leaks; that is the documented
    // shape of this API and a few KB on a path that essentially never fails is
    // preferable to a double free.
    unsafe { SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(hglobal.0 as *mut c_void))) }
        .map_err(|e| format!("could not write the clipboard: {e}"))?;

    Ok(())
}
