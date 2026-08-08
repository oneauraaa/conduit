//! The macOS platform layer. Everything that touches AppKit, Quartz,
//! ScreenCaptureKit or the accessibility API lives behind this module, so the
//! MCP tools above it stay readable.

pub mod appicon;
pub mod ax;
pub mod capture;
pub mod cf;
pub mod clipboard;
pub mod input;
pub mod keycodes;
pub mod panic_stop;
pub mod permissions;
pub mod screen;
pub mod windows;
