//! Hiding the system pointer while an agent is driving.
//!
//! **Currently a deliberate no-op.** The system arrow stays visible inside the
//! overlay glow, alongside conduit's drawn cursor.
//!
//! ## Why not just hide it
//!
//! `ShowCursor(FALSE)` is not an option: it decrements a refcount on the
//! *calling thread's* input queue, so it works in a toy test against your own
//! window and does exactly nothing to another application's cursor. It would
//! look implemented and not be.
//!
//! `SetSystemCursor` does work, and it is a machine-wide, persistent change to
//! the user's pointer scheme. If conduit exits without putting the cursors
//! back — killed from Task Manager, a hard crash, a power cut — **the user has
//! no mouse pointer anywhere in Windows** until they change the pointer scheme
//! by hand or log out. Recovering from that without a visible cursor is genuinely
//! hard.
//!
//! macOS's own path here is explicitly best-effort — `CGDisplayHideCursor` is
//! ignored when conduit isn't frontmost, which is most of the time, and
//! `mac/input.rs` notes that the arrow sitting inside the glow "still reads as
//! intentional". So the visible-arrow case is already the common one on the
//! platform this was designed for, and matching it costs nothing.
//!
//! Phase 6 adds the real implementation behind an off-by-default setting, with
//! restores wired into all four exit paths (`end_control`, the tray quit,
//! `RunEvent::Exit`, and a panic hook). Until every one of those is in place,
//! not touching the user's cursor scheme is the right trade.

pub(super) fn hide() {}

pub(super) fn restore() {}
