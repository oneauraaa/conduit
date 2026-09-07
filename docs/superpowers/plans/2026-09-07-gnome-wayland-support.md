# GNOME Wayland Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Add standard GNOME Wayland clipboard/accessibility behavior, use ScreenCast-only capture on wlroots sessions, and make unsupported window management explicit.

**Architecture:** Make the Linux portal session capability-aware. The input route chooses wlroots or RemoteDesktop; the capture route chooses ScreenCast-only on wlroots and the existing combined session on GNOME/KDE. The clipboard facade follows the same route, while KWin-only window operations fail before stale IDs or scripts are used.

**Tech Stack:** Rust, zbus blocking D-Bus, XDG Desktop Portal RemoteDesktop/ScreenCast/Clipboard, PipeWire, AT-SPI2, React/TypeScript, Tauri.

---

### Task 1: Make portal sessions route-aware

**Files:**
- Modify: `src-tauri/src/platform/linux/portal.rs`
- Modify: `src-tauri/src/platform/linux/sink.rs`
- Modify: `src-tauri/src/platform/linux/permissions.rs`
- Test: `src-tauri/src/platform/linux/portal.rs` tests

- [x] **Step 1: Add an explicit portal session mode.**

Define a private mode enum in `portal.rs` with `CaptureOnly` and `RemoteControl`, and store the selected mode on `Session`. Add a route helper that maps `super::sink::Route::Wlroots` to `CaptureOnly` and `Route::Portal` to `RemoteControl`. Keep the call one-way so portal initialization never probes itself through `sink::route` recursively.

- [x] **Step 2: Split the handshake at session creation.**

For `CaptureOnly`, call `ScreenCast.CreateSession`, `ScreenCast.SelectSources`, and `ScreenCast.Start`. For `RemoteControl`, preserve the existing `RemoteDesktop.CreateSession`, `RemoteDesktop.SelectDevices`, `ScreenCast.SelectSources`, and `RemoteDesktop.Start` flow. Keep `Session::streams` and `pipewire_fd` unchanged for capture callers.

- [x] **Step 3: Preserve route-specific readiness semantics.**

Make `PortalError::message` accept the selected mode or add a route-aware formatter. A missing RemoteDesktop interface must only be reported for `RemoteControl`; a wlroots `CaptureOnly` session must never issue that method. Add tests for both mode selections and for the missing-interface message.

- [x] **Step 4: Run focused tests.**

~~~bash
cargo test --lib platform::linux::portal
~~~

Expected: all portal tests pass, including the new mode and error-message cases.

### Task 2: Add the portal Clipboard capability

**Files:**
- Modify: `src-tauri/src/platform/linux/portal.rs`
- Modify: `src-tauri/src/platform/linux/clipboard.rs`
- Modify: `src-tauri/src/platform/linux/sink.rs`
- Test: `src-tauri/src/platform/linux/portal.rs` and `clipboard.rs` tests

- [x] **Step 1: Request Clipboard before RemoteDesktop.Start.**

In the `RemoteControl` handshake, call `org.freedesktop.portal.Clipboard.RequestClipboard` with the RemoteDesktop session path before `Start`. Treat an unknown interface or method as a non-fatal capability absence and keep the session usable for input/capture. Parse `clipboard_enabled` from the Start response and store it on `Session`.

- [x] **Step 2: Add portal clipboard read.**

Add `Session::clipboard_read_text() -> Result<String, String>`. Check the stored capability, call `SelectionRead` with `text/plain;charset=utf-8` and fall back to `text/plain`, deserialize the returned `OwnedFd`, read it to completion, and return lossy UTF-8 text. Return a focused error when the portal did not grant Clipboard.

- [x] **Step 3: Add portal clipboard write state and transfer servicing.**

Add process-lifetime clipboard state to `Session`, containing the current UTF-8 bytes and the session path. On write, store the bytes and call `SetSelection` with the two plain-text MIME types. Start one blocking signal worker for `SelectionTransfer`; filter the signal’s session path, call `SelectionWrite`, write the stored bytes to the returned descriptor, and call `SelectionWriteDone` with the write result. The worker must exit when the portal signal stream closes.

- [x] **Step 4: Route the Linux clipboard facade.**

Keep the current `wlr-data-control` implementation for `Route::Wlroots`. For `Route::Portal`, call the active portal session’s read/write methods. Return portal errors directly instead of claiming that GNOME lacks clipboard support.

- [x] **Step 5: Add protocol-shape tests.**

Test MIME preference, capability-disabled errors, session-path filtering, and transfer success/failure acknowledgement helpers without requiring a GNOME session. Keep the live D-Bus worker behind the existing session initialization.

- [x] **Step 6: Run focused tests.**

~~~bash
cargo test --lib platform::linux::portal platform::linux::clipboard platform::linux::sink
~~~

Expected: all focused tests pass.

### Task 3: Make window-management limitations explicit

**Files:**
- Modify: `src-tauri/src/platform/linux/apps.rs`
- Modify: `src/tabs/ServerTab.tsx`
- Modify: `src-tauri/src/platform/linux/host.rs` if the desktop label needs normalization
- Test: `src-tauri/src/platform/linux/apps.rs` tests

- [x] **Step 1: Add a stable unsupported-operation helper.**

Return messages such as `window control is unsupported on GNOME Wayland: this desktop exposes no standard API for another app's window geometry or focus` when KWin is absent. Use the same helper for listing, focusing, and bounds changes, before resolving window IDs or invoking KWin scripts.

- [x] **Step 2: Update the Server tab copy.**

When `windowManagement` is false and the reported desktop is GNOME, explicitly name GNOME Wayland. For other non-KWin Wayland desktops, retain a generic compositor limitation message. Do not show an amber “permission” action for an unavailable protocol.

- [x] **Step 3: Add tests for unsupported operations.**

Test that the helper names GNOME Wayland and that the operation paths return it before attempting a stale window lookup. Keep KWin behavior unchanged.

- [x] **Step 4: Run focused tests.**

~~~bash
cargo test --lib platform::linux::apps
~~~

Expected: existing Linux application tests and the new unsupported-operation tests pass.

### Task 4: Improve AT-SPI readiness guidance

**Files:**
- Modify: `src-tauri/src/platform/linux/ax.rs`
- Modify: `src-tauri/src/platform/linux/permissions.rs`
- Modify: `src/tabs/ServerTab.tsx`
- Test: `src-tauri/src/platform/linux/ax.rs` tests

- [x] **Step 1: Separate AT-SPI failure causes.**

Give readiness and tool errors distinct messages for an unavailable accessibility bus, an empty registry, and a target application that is not publishing a tree. Keep the existing GTK setting and Qt environment guidance, and state that screenshots still work.

- [x] **Step 2: Preserve no-extension behavior.**

Do not mutate global GSettings, install packages, set environment variables for unrelated processes, or add a GNOME Shell extension. The app may report exact remediation and continue using screenshots/input/capture.

- [x] **Step 3: Add parser/guidance tests.**

Test the message selection and keep existing AT-SPI tree traversal tests intact.

- [x] **Step 4: Run focused tests.**

~~~bash
cargo test --lib platform::linux::ax platform::linux::permissions
~~~

Expected: all accessibility and readiness tests pass.

### Task 5: Update readiness state and validate Hyprland capture

**Files:**
- Modify: `src-tauri/src/state.rs` only if readiness text needs a new capability field
- Modify: `src/lib/types.ts` and `src/lib/standalone.ts` if the state shape changes
- Modify: `src/tabs/ServerTab.tsx`
- Test: existing state serialization tests and live Hyprland session

- [x] **Step 1: Make capture and input text reflect the selected route.**

Ensure a wlroots session reports `inputRoute: "wlroots"` and shows input as ready independently of the ScreenCast prompt. Ensure its portal error describes only ScreenCast failure. Ensure GNOME/KDE still report combined RemoteDesktop failures.

- [x] **Step 2: Update standalone preview state.**

If copy changes require new fields, update `src/lib/standalone.ts` so browser preview remains type-correct.

- [x] **Step 3: Run the full automated checks.**

~~~bash
cd src-tauri && cargo test --lib && cargo build
cd .. && pnpm build
git diff --check
~~~

Expected: Rust tests pass, the Tauri crate builds, TypeScript/Vite production build passes, and the diff is clean.

- [x] **Step 4: Run live Hyprland verification.**

With the existing Wayland development server and `GDK_BACKEND=wayland`, restart Conduit and verify:

1. readiness reports `input_route: "wlroots"` and `input_ready: true`;
2. the portal handshake no longer logs a missing `RemoteDesktop` error and uses ScreenCast-only capture;
3. an MCP screenshot reaches the PipeWire frame path;
4. pointer, keyboard, Unicode, and pill-guard checks remain green.

Record any missing ScreenCast backend as a capture-only limitation; do not fall back to a misleading RemoteDesktop error.

- [x] **Step 5: Review final changes.**

~~~bash
git status --short
git diff --stat
git diff --check
~~~

Confirm no GNOME extension, private Mutter interface, or unrelated generated artifact was added.

