//! Compile-time proof that both backends expose the same surface.
//!
//! Every function below is coerced to a `fn` pointer of the expected type. If a
//! Windows implementation drifts from its macOS twin — a reordered argument, a
//! `String` where the other returns `Option<String>` — this file fails to
//! compile, naming the function. That is a far better error than one surfacing
//! three layers up inside a tool handler, and it is the entire reason the
//! backends can be plain modules instead of a trait.
//!
//! This file is compiled on every platform and referenced by nothing. The
//! `const _` block is evaluated for type-checking and emits no code.
//!
//! Not listed: `input::glide`, `click`, `drag` and `type_text`. They are `async`
//! and/or take `impl FnMut`, so they have no nameable `fn` type. Their call
//! sites in `mcp/tools.rs` catch any drift instead.

#![allow(dead_code)]

use super::types::{AppInfo, Display, Element, OwnWindows, Shot, WindowInfo};
use super::*;

const _: () = {
    let _: fn(&str, f64) -> Option<String> = appicon::icon_data_url;
    let _: fn(&[&str], f64) = appicon::warm_cache;

    let _: fn(Option<&str>) -> Result<Vec<Element>, String> = ax::read_screen;
    let _: fn(&str, Option<&str>) -> Result<Vec<Element>, String> = ax::find_element;

    let _: fn(&Display, Option<(f64, f64, f64, f64)>, f64) -> Result<Shot, String> = capture::capture;

    let _: fn() -> Option<String> = clipboard::read_text;
    let _: fn(&str) -> Result<(), String> = clipboard::write_text;

    let _: fn() -> String = host::description;
    let _: &str = host::OS;
    let _: &str = host::DEVICE;
    let _: &str = host::SHELL;
    let _: &str = host::SHORTCUT_MODIFIER;
    let _: &str = host::CHROME_ANCHOR;
    let _: &str = host::AX_SOURCE;

    let _: fn() -> (f64, f64) = input::cursor_position;
    let _: fn(i32, i32) = input::scroll;
    let _: fn(&str, &[String]) -> Result<(), String> = input::key_press;
    let _: fn() -> Option<String> = input::blocked_reason;
    let _: fn() = input::hide_system_cursor;
    let _: fn() = input::show_system_cursor;

    let _: fn(&str) -> Option<u16> = keycodes::lookup;

    let _: fn() -> bool = panic_stop::install_hook;

    let _: fn() -> crate::state::Readiness = permissions::snapshot;
    let _: fn(&tauri::AppHandle<tauri::Wry>) -> Result<(), String> = permissions::relaunch_elevated;
    let _: fn() -> bool = permissions::accessibility_granted;
    let _: fn() -> bool = permissions::prompt_accessibility;
    let _: fn() -> bool = permissions::screen_recording_granted;
    let _: fn() -> bool = permissions::prompt_screen_recording;

    let _: fn() -> Vec<Display> = screen::displays;
    let _: fn(f64, f64) -> (f64, f64) = screen::pill_anchor;
    let _: fn(f64, f64) -> tauri::Position = screen::tauri_position;
    let _: fn(f64, f64) -> tauri::Size = screen::tauri_size;
    let _: fn(f64, f64) -> f64 = screen::physical_to_space;
    let _: fn(&Display) -> f64 = screen::default_capture_scale;

    let _: fn() -> Vec<WindowInfo> = apps::list_windows;
    let _: fn(&[WindowInfo]) -> Option<String> = apps::list_windows_hint;
    let _: fn() -> Vec<AppInfo> = apps::list_apps;
    let _: fn(u32) -> Result<(), String> = apps::focus_window;
    let _: fn(u32, f64, f64, f64, f64) -> Result<(), String> = apps::set_window_bounds;
    let _: fn(&str) -> Result<(), String> = apps::open_app;
    let _: fn(&str) -> Result<(), String> = apps::quit_app;
    let _: fn(&str, &str) -> Result<(), String> = apps::notify;
    let _: fn() -> OwnWindows = apps::own_windows;

    let _: fn(&str) -> tokio::process::Command = shell::command;
};
