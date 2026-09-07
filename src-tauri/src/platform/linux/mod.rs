//! The Linux backend, targeting Wayland.
//!
//! Module names and signatures are mirrored exactly by `platform/mac` and
//! `platform/win`; see `platform/contract.rs`.
//!
//! ## What is different here, and why
//!
//! On macOS and Windows the OS hands a trusted process the machine: one grant,
//! and every API works. Wayland does not have that shape. Each capability comes
//! from a different place, with a different consent model:
//!
//! | capability | route | needs |
//! |---|---|---|
//! | input, capture | `xdg-desktop-portal` RemoteDesktop + ScreenCast | a dialog, every launch |
//! | window geometry | KWin scripting over D-Bus | Plasma |
//! | screen text | AT-SPI2 over D-Bus | toolkits opted in |
//! | clipboard | `wlr-data-control` | KDE or wlroots |
//! | hold-Escape | evdev | the `input` group |
//!
//! Only the first is required. Everything else degrades to a specific,
//! actionable message rather than a silent wrong answer — which is the same
//! standard the other two backends hold themselves to, applied to a platform
//! that fragments the problem further.
//!
//! [`permissions::snapshot`] reports all five, and the Server tab shows them.

pub mod appicon;
pub mod apps;
pub mod ax;
pub mod capture;
pub mod clipboard;
pub mod host;
pub mod hyprctl;
pub mod input;
pub mod keycodes;
pub mod kwin;
pub mod panic_stop;
pub mod permissions;
pub mod portal;
pub mod screen;
pub mod shell;
pub mod sink;
pub mod wlroots;
