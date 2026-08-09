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
