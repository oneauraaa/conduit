# Notes for the next person

Things that cost someone a day, written down so they cost the next person
nothing. Split by platform; the first list applies to the shared code and to
macOS.

## macOS and shared

- **ScreenCaptureKit is mandatory.** `CGDisplayCreateImage` still compiles on
  Tahoe but returns desktop wallpaper with no app windows — a silent wrong
  answer, which is worse than an error.
- **The Swift rpath in `build.rs` is load-bearing.** screencapturekit bridges
  through Swift; without `-rpath /usr/lib/swift` the app builds and then dies at
  launch on `@rpath/libswift_Concurrency.dylib`.
- **Capabilities must list every window.** `capabilities/default.json` names
  `main`, `pill` and `overlay-*`. Miss one and its `event.listen` is silently
  rejected — the window renders but never updates.
- **Don't hold a `CFRetained` across an `await`.** It isn't `Send`, and it makes
  the whole enclosing tool future non-`Send`. See the scoped blocks in
  `mac/input.rs`.
- **`macOSPrivateApi` and the `macos-private-api` cargo feature must agree**, or
  transparent windows render solid black.
- **Agent logos are never bundled.** `platform/*/appicon.rs` asks the OS for the
  icon of the vendor app already installed, so conduit redistributes no
  third-party artwork and the marks are always current. No app installed →
  tinted monogram.

## Windows

- **Coordinates are physical pixels**, not logical. Every API in the port —
  `GetMonitorInfoW`, `SendInput`, WGC frames, UIA bounds, `SetWindowPos` —
  already speaks them, so converting would only add places to apply the wrong
  monitor's scale. `Display.scale` is informational there.
- **`AutomationElementMode_None` breaks the accessibility walk.** It looks like
  a free optimisation. Elements returned under it carry no live reference, so
  recursing into one fails and `read_screen_text` returns five elements for a
  screen with two hundred — silently.
- **`SendInput` returning short is UIPI, not a blip.** Unchecked, conduit would
  report clicks that Windows threw away. `input::blocked_reason` is what turns
  that into an error the agent can act on.
- **`DWMWA_CLOAKED` is not optional.** Without it `list_windows` is half
  invisible UWP shells, and the agent clicks where nothing is. It is the
  counterpart of the macOS `kCGWindowLayer != 0` check.
- **Read bounds with `DWMWA_EXTENDED_FRAME_BOUNDS`, write with `SetWindowPos`.**
  The latter includes an invisible ~7px resize border, so a list → set round
  trip drifts outward unless the delta is applied.
- **`Direct3D11CaptureFramePool::Create` needs a DispatcherQueue** — use
  `CreateFreeThreaded`. And `RowPitch` is almost never `width * 4`; copy row by
  row or the screenshot comes out sheared.
- **WinRT calls need a COM apartment.** `GraphicsCaptureSession::IsSupported()`
  fails with `CO_E_NOTINITIALIZED` on a bare thread pool thread, which reads
  exactly like "this Windows is too old". The readiness card said screenshots
  were unsupported on a machine where they worked, for precisely this reason.
- **`AreDpiAwarenessContextsEqual` does not match a manifest-declared context**
  against the predefined PMv2 constant. Check
  `GetAwarenessFromDpiAwarenessContext` as well, or a correctly configured
  process reports itself broken.
- **`macOSPrivateApi` needs no `cfg`.** It only asserts a cargo feature that
  resolves to two empty wry feature lists. The obvious assumption is the
  opposite and someone will "fix" it.
- **`HWND` truncated to `u32` is safe** — Windows guarantees handles are
  32-bit-significant even in 64-bit processes. It looks like a bug; it isn't.

## Linux (Wayland)

The Linux backend is shaped by one fact: **Wayland gives an ordinary client
nothing.** It cannot move the pointer, press a key, read a pixel it does not
own, or learn another window's geometry. That is the security model working as
designed, and it is why this backend is five different mechanisms rather than
one API — see `platform/linux/mod.rs` for the table.

- **The screen-sharing prompt cannot be made permanent.** Every other portal
  grant takes `persist_mode` and a restore token. Asking for one on a
  RemoteDesktop session earns `InvalidArgument: Remote desktop sessions cannot
  persist`. That is deliberate upstream — a token that silently restores full
  input control is exactly what an attacker would steal — so conduit asks on
  every launch, at startup, and there is no fix to find. Do not go looking for
  one; that hour is already spent.
- **Absolute pointer motion needs a ScreenCast session.**
  `NotifyPointerMotionAbsolute` takes a *stream id*, and streams only exist once
  `ScreenCast.SelectSources` has run. RemoteDesktop alone gives you relative
  motion, which goes through the compositor's acceleration curve — so "move to
  x,y" lands somewhere that depends on how fast the previous events went out.
  The two interfaces share one session for this reason, not for tidiness.
- **Subscribe to the portal's `Response` signal *before* calling the method.**
  The portal may emit it before the method call returns. A listener attached
  afterwards waits forever for a signal that already went past.
- **KWin caches a loaded script by name and path.** Reuse either and the next
  `loadScript` silently re-runs the *previous* script. This is invisible from
  the calling side: the stale script reports something, the D-Bus call succeeds,
  and the caller concludes it worked. It cost an afternoon —
  `set_window_bounds` reported "window moved" while actually re-running the
  preceding `list_windows`, so the window never moved and nothing said so.
  `kwin::run_script` now uses a fresh name *and* a fresh path per call.
- **`callDBus`'s method name is PascalCase on the wire.** `#[zbus::interface]`
  renames `fn report` to `Report`, and KWin passes the string through verbatim.
  Calling `"report"` fails completely silently — the script runs, the call goes
  nowhere, and the only symptom is a timeout.
- **A window's geometry changes asynchronously.** KWin sends the client a
  configure and the geometry only moves once the client acks and commits. Read
  `frameGeometry` immediately after assigning it and you get the old value —
  which is not a bug, it is "the window has not agreed yet". `set_window_bounds`
  settles and re-reads before reporting, so a window that genuinely refuses is
  an error rather than a false success.
- **`WEBKIT_DISABLE_DMABUF_RENDERER=1` is load-bearing.** Without it conduit
  dies at startup on KWin with `wp_linux_drm_syncobj_surface_v1: explicit sync
  is used, but no acquire point is set`, which surfaces as
  `Gdk-Message: Error 71 (Protocol error)` and no window at all. WebKitGTK
  commits through the explicit-sync protocol without attaching an acquire point;
  KWin is right to refuse it. Set in `lib.rs` before anything touches GTK.
- **`set_ignore_cursor_events` aborts the process on an unshown window.** tao
  implements it by combining an empty input region onto the window's
  `GdkWindow`, which does not exist until the widget is realized — and it
  `unwrap()`s. Since all of conduit's chrome is built hidden, this has to happen
  *after* `show()`. The panic surfaces from inside the GTK main loop as "panic
  in a function that cannot unwind", which names nothing useful.
- **gtk-layer-shell is `dlopen`ed, not linked.** conduit ships as a zip with no
  dependency resolution, so linking it would turn a missing optional library
  into a binary that does not start. Without it the glow and pill still render,
  just wherever the compositor puts them. `CONDUIT_NO_LAYER_SHELL=1` forces that
  path for debugging.
- **AT-SPI is opt-in and usually off.** Toolkits publish an accessibility tree
  only when they think assistive technology is listening — Qt wants
  `QT_ACCESSIBILITY=1`, GTK wants the `toolkit-accessibility` gsetting. On a
  default Plasma install `read_screen_text` therefore returns almost nothing,
  which looks exactly like a bug. The readiness card reports it as its own row
  with the commands to fix it.
- **Filter conduit out of the accessibility tree.** conduit is a GTK app and
  publishes its own tree like any other, so `read_screen_text` will happily hand
  an agent conduit's own Tools tab and mode dropdown — the controls
  `chrome::point_hits_conduit` exists to keep out of reach. `list_windows`
  filters by pid; `ax.rs` filters by application name.
- **AT-SPI reports `i32::MIN` for "not laid out".** A size check alone waves
  those through as 1x1 rectangles two billion pixels off-screen, and the agent
  clicks them. Reject the sentinel and anything landing on no display.
- **`wl-clipboard-rs`'s `foreground(true)` never returns.** Wayland has no
  clipboard store — the copying client owns the selection and must stay alive to
  serve it — so `copy` leaves something running either way. In the default mode
  that something is a thread; in foreground mode it is the calling thread, until
  another application copies something else. From a tool handler that means
  `clipboard_write` simply hangs.
- **Never block a tool call on the consent dialog.** `portal::session` uses
  `try_lock`, not `lock`: the warm-up thread holds that lock for as long as the
  prompt is on screen, and an agent has no timeout to save it. An immediate
  "still waiting, ask them to click share" is the only honest answer.
- **conduit's own overlay is in its own screenshots.** macOS excludes it with an
  `SCContentFilter` and Windows with `WDA_EXCLUDEFROMCAPTURE`; Wayland has no
  per-window capture exclusion, so the glow, the AI cursor and the pill appear
  in every frame an agent captures. Hiding the chrome around each capture was
  considered and rejected: at 10fps it would cost a settle wait per screenshot
  and make the glow flicker constantly, which is worse than an agent seeing a
  border effect it can neither click (`point_hits_conduit` refuses) nor be
  confused by for long.
- **Ask PipeWire for packed formats only.** Advertise DMA-BUF support and the
  compositor will hand over GPU buffers that `MAP_BUFFERS` cannot map and that
  need EGL to read. Offering only BGRx/RGBx/BGRA/RGBA keeps the server on memfd.
  And `stride` is almost never `width * 4` — copy row by row, exactly as on
  Windows with `RowPitch`.
- **Hold-Escape needs the `input` group, and that is not a bug either.** Wayland
  has no passive key listener by design; a client that could watch keys it does
  not own is a keylogger. A GlobalShortcuts registration would *consume* Escape
  system-wide — breaking every dialog and vim session, swallowing the Escape
  conduit itself injects, and tripping the panic stop each time. evdev is the
  only route that observes without taking.
