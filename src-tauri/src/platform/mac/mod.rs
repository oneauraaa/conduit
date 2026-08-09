//! The macOS backend. Everything that touches AppKit, Quartz, ScreenCaptureKit
//! or the accessibility API lives behind this module.
//!
//! Module names and signatures are mirrored exactly by `platform/win`; see
//! `platform/contract.rs`.

pub mod appicon;
pub mod apps;
pub mod ax;
pub mod capture;
pub mod cf;
pub mod clipboard;
pub mod input;
pub mod keycodes;
pub mod panic_stop;
pub mod permissions;
pub mod screen;
pub mod shell;
