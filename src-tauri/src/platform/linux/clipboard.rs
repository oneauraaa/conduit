//! Clipboard access over `wlr-data-control`.
//!
//! ## Why not the ordinary Wayland clipboard
//!
//! Reading or writing the normal `wl_data_device` clipboard requires a
//! *focused surface* — the compositor hands clipboard access to whoever the
//! user is currently typing into. conduit is never that: its windows are
//! click-through overlays and a tray app, and the whole point is that some
//! other application has focus.
//!
//! `wlr-data-control` is the protocol built for exactly this case — clipboard
//! managers, which have the same problem. KDE, wlroots and most others
//! implement it. GNOME does not, which is why the errors below name the
//! protocol rather than saying "clipboard failed".

use wl_clipboard_rs::copy::{self, MimeSource, Options, Source};
use wl_clipboard_rs::paste::{self, ClipboardType, Seat};

fn read_wlr_text() -> Option<String> {
    // `text/plain;charset=utf-8` first, falling back to whatever text the owner
    // offers — some applications advertise only `TEXT` or `STRING`.
    let result = paste::get_contents(
        ClipboardType::Regular,
        Seat::Unspecified,
        paste::MimeType::Text,
    );

    match result {
        Ok((mut pipe, _mime)) => {
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut pipe, &mut buf).ok()?;
            // Lossy rather than strict: a clipboard holding invalid UTF-8 is
            // still worth handing over as text, and an agent gets more from
            // slightly mangled content than from nothing.
            Some(String::from_utf8_lossy(&buf).into_owned())
        }
        // An empty clipboard is not an error, and neither is one holding only
        // an image — both are simply "no text here".
        Err(paste::Error::NoSeats)
        | Err(paste::Error::ClipboardEmpty)
        | Err(paste::Error::NoMimeType) => None,
        Err(e) => {
            tracing::warn!("could not read the clipboard: {e}");
            None
        }
    }
}

fn write_wlr_text(text: &str) -> Result<(), String> {
    // Deliberately *not* `foreground(true)`. Wayland has no clipboard store:
    // the copying client owns the selection and must stay alive to serve it,
    // so `copy` leaves something running either way. In the default mode that
    // something is a thread inside conduit, which is exactly right for a
    // long-running app. `foreground(true)` instead blocks the calling thread
    // until another application copies something else — which, from a tool
    // handler, means `clipboard_write` never returns at all.
    let options = Options::new();

    options
        .copy(
            Source::Bytes(text.as_bytes().into()),
            copy::MimeType::Text,
        )
        .map_err(|e| {
            format!(
                "could not write to the clipboard ({e}). this needs the \
                 wlr-data-control protocol, which kde and wlroots compositors \
                 have but gnome does not."
            )
        })
}

/// Reads the clipboard through the mechanism supported by this compositor.
/// wlroots uses data-control; GNOME and KDE use the portal session's clipboard
/// extension, so clipboard access follows the same consent boundary as input.
pub fn read_text() -> Result<Option<String>, String> {
    match super::sink::route() {
        super::sink::Route::Wlroots => Ok(read_wlr_text()),
        super::sink::Route::Portal => super::portal::session()
            .map_err(|error| error.message())
            .and_then(|session| session.clipboard_read_text()),
    }
}

/// Writes text through the mechanism supported by this compositor.
pub fn write_text(text: &str) -> Result<(), String> {
    match super::sink::route() {
        super::sink::Route::Wlroots => write_wlr_text(text),
        super::sink::Route::Portal => super::portal::session()
            .map_err(|error| error.message())?
            .clipboard_write_text(text),
    }
}

#[allow(dead_code)]
fn _unused(_: MimeSource) {}
