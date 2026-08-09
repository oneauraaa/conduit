//! The platform layer. Everything that touches an OS API lives behind this
//! module, so the MCP tools above it stay readable.
//!
//! Both backends expose the same module names with the same signatures, so
//! `mcp/tools.rs` contains no `#[cfg]` at all — it just calls
//! `platform::input::click` and lets the linker decide which one that is.
//! [`contract`] pins that promise at compile time.
//!
//! Plain cfg-gated modules rather than a trait, deliberately: `glide` and
//! `drag` are `async fn`s taking `impl FnMut(f64, f64)`, which is a fight in a
//! trait for no benefit, and the compiler already catches a missing function at
//! the call site.

pub mod types;

#[allow(dead_code)]
pub mod tween;

mod contract;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::*;

#[cfg(target_os = "windows")]
mod win;
#[cfg(target_os = "windows")]
pub use win::*;
