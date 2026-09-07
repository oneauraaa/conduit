# Finish the wlroots input backend

**Goal:** Complete the existing virtual-pointer and virtual-keyboard implementation and verify it on the current Hyprland session.

**Architecture:** Keep the existing Linux sink: prefer advertised virtual-input protocols and retain the portal fallback. Preserve the compositor-based guard for conduit's own windows. Validate actual delivered events in a disposable input receiver.

**Tech stack:** Rust, wayland-client, wlroots virtual pointer, virtual keyboard, GTK, Tauri.

- [x] Restore compilation in `src-tauri/src/platform/linux/wlroots.rs`: read `globals.pointer_version` before moving `globals` into `pump`, and use `ProtocolError.object_interface`.
- [x] Run `cargo build` in `src-tauri` and restart the existing development app with its Vite server available.
- [x] Use a disposable GTK receiver to check actual pointer coordinates, button press/release, scrolling, ASCII, Unicode, modifiers, and a drag. Exercise the MCP tools so the sink and guard participate.
- [x] For each observed failure, add a focused regression test, fix its cause, and rerun that test. Check connection failures reach input readiness and MCP error reporting.
- [x] Run `cargo test --lib`, `cargo build`, and `pnpm build`; inspect the diff and remove test processes. The live verification used Hyprland 0.56.2 with `GDK_BACKEND=wayland`; an X11-forced GTK launch intentionally falls back to ordinary windows and is not the wlroots path.
