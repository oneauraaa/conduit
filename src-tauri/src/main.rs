// Prevents an extra console window on Windows in release. conduit is macOS-only,
// but the attribute is harmless and keeps the crate portable if that changes.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    conduit_lib::run()
}
