# GNOME Wayland Support Design

**Date:** 2026-09-07  
**Status:** Approved design, implementation pending

## Goal

Make Conduit behave correctly on GNOME Wayland without requiring a GNOME Shell extension: use the standard portal for clipboard access, keep AT-SPI as the screen-text path, use ScreenCast-only capture on wlroots sessions, and report window-management limits honestly.

## Scope

### In scope

- Route portal initialization by the input compositor:
  - GNOME/KDE-style sessions keep the combined RemoteDesktop + ScreenCast session.
  - wlroots sessions request ScreenCast only; input remains on the wlroots virtual pointer and virtual keyboard backend.
- Integrate the XDG Clipboard portal into the combined portal session, including reads and writes.
- Preserve `wlr-data-control` clipboard handling for wlroots sessions.
- Improve AT-SPI readiness and failure messages for GNOME and keep screenshot capture available when no accessibility tree is published.
- Make Linux window listing, focusing, moving, and resizing return explicit unsupported messages where KWin is absent, including GNOME Wayland.
- Update the Server tab so a missing RemoteDesktop interface on a wlroots compositor is not presented as the input route failing.

### Out of scope

- GNOME Shell extensions.
- Mutter private `Eval` or other undocumented GNOME Shell interfaces.
- A generic Wayland window-management backend; no standard protocol provides cross-client geometry or focus control.
- OCR implementation. Screenshots remain the fallback when an application publishes no AT-SPI tree.

## Architecture

### Portal sessions

`platform/linux/portal.rs` will represent the session capabilities explicitly. The handshake will select one of two modes:

1. **Capture-only mode** for `Route::Wlroots`: create a ScreenCast session, select monitors, and start it. The resulting streams feed the existing PipeWire capture path. No RemoteDesktop call is attempted.
2. **Remote-control mode** for `Route::Portal`: create a RemoteDesktop session, select keyboard and pointer, attach ScreenCast sources, request Clipboard before `Start`, and retain the resulting combined session.

The portal session will expose whether clipboard access was granted. A backend that supports RemoteDesktop but does not grant or implement Clipboard will keep input and capture working and return a focused clipboard error.

### Portal clipboard

The Linux clipboard facade will select by input route:

- `Route::Wlroots`: current `wlr-data-control` implementation.
- `Route::Portal`: `org.freedesktop.portal.Clipboard` attached to the active RemoteDesktop session.

Reads call `SelectionRead` and consume the returned file descriptor. Writes publish text with `SetSelection`; a session-owned transfer worker handles `SelectionTransfer`, writes the requested bytes through `SelectionWrite`, and acknowledges with `SelectionWriteDone`. The worker is bounded to the leaked process-lifetime portal session and filters transfers to Conduit’s session path.

### Accessibility

AT-SPI remains the standard GNOME Wayland screen-text mechanism. Readiness will distinguish “the accessibility bus is unavailable” from “the bus is up but no application publishes a tree.” Tool errors will retain the existing actionable GTK/Qt guidance. No global accessibility setting will be silently changed and no extension will be installed. Screenshot capture remains available independently.

### Window-management errors

Linux window operations will check KWin availability before resolving a window id. Without KWin they return a stable message stating that window listing/focus/move/resize is unsupported on this Wayland desktop, with GNOME Wayland named when detected. This prevents generic “no window with id” or portal errors from implying a transient failure.

### Readiness UI

The Server tab will treat capture and input as independent capabilities:

- On Hyprland/wlroots, the input row can be green while the capture row waits for or reports a ScreenCast failure.
- A missing `org.freedesktop.portal.RemoteDesktop` on wlroots is no longer shown, because that interface is not needed by the selected route.
- On GNOME/KDE, RemoteDesktop remains required for input and capture; failures continue to identify the portal/backend problem.
- Window control explicitly says it is unsupported on GNOME Wayland when KWin is absent.

## Error handling

- Missing RemoteDesktop is fatal only to the portal input route, never to wlroots input or to a wlroots capture-only session.
- Missing ScreenCast remains a capture error and is shown with the backend installation/session guidance.
- Missing Clipboard is reported only by clipboard operations; it does not invalidate a working RemoteDesktop session.
- Clipboard transfer failures acknowledge the portal request as failed and preserve the first actionable error for the tool response.
- Unsupported window operations fail before any KWin script or stale window-id lookup.

## Testing

- Unit tests for portal mode selection and route-specific error messages.
- Unit tests for Clipboard portal response parsing, text MIME selection, and transfer filtering/acknowledgement helpers.
- Unit tests for explicit unsupported window-operation errors without KWin.
- Existing Rust test suite, build, frontend typecheck/build, and diff checks.
- Live Hyprland verification: input still works through wlroots, ScreenCast capture no longer produces a missing-RemoteDesktop readiness error, and the portal fallback remains intact.
- GNOME-specific D-Bus behavior will be compile-tested and covered by protocol-shape tests here; a real GNOME session is required for end-to-end consent and Clipboard portal validation.

## User-visible result

On GNOME Wayland, opening Conduit gives working portal-backed input/capture/clipboard where the installed portal exposes those capabilities, AT-SPI screen text when applications publish it, and clear unsupported responses for window management. On Hyprland, opening Conduit uses wlroots for input and ScreenCast-only portal capture, without claiming that a missing RemoteDesktop interface is the input failure.
