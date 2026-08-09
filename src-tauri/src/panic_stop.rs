//! The panic stop: hold Escape to take the machine back.
//!
//! The *policy* lives here — how long Escape must be held, what happens when it
//! trips — because it is a product decision, not a platform one. Each backend
//! contributes only the primitive: a listen-only keyboard hook that calls
//! [`escape_down`] and [`escape_up`].
//!
//! ## Why a passive hook and not a global shortcut
//!
//! Registering Escape as a global shortcut would swallow it system-wide — every
//! dialog, every vim session, every video player would stop seeing the key.
//! Both backends instead observe key events without consuming them and measure
//! how long Escape is held.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::state::Shared;

/// How long Escape must be held before control is dropped. Long enough that a
/// normal "dismiss this dialog" tap never triggers it.
const HOLD_MS: u64 = 800;

/// Millis when Escape went down, or 0 when it isn't held. A static because the
/// hook callbacks are bare `extern "C"` functions on both platforms, with no
/// room for captured state.
static ESC_DOWN_AT: AtomicU64 = AtomicU64::new(0);

/// Called by the platform hook when Escape goes down.
///
/// Auto-repeat fires repeatedly while the key is held; only the first matters,
/// so this is a compare-exchange rather than a store.
pub(crate) fn escape_down() {
    let _ = ESC_DOWN_AT.compare_exchange(
        0,
        crate::state::now_millis(),
        Ordering::Relaxed,
        Ordering::Relaxed,
    );
}

/// Called by the platform hook when Escape comes up.
pub(crate) fn escape_up() {
    ESC_DOWN_AT.store(0, Ordering::Relaxed);
}

/// Installs the platform hook and starts the watcher.
///
/// Returns whether the hook could be created. A `false` is not fatal — it just
/// means the OS withheld it (on macOS, Accessibility isn't granted yet), and
/// the Stop button in the pill remains the way out. `lib.rs` retries.
pub fn install(state: Shared) -> bool {
    if !crate::platform::panic_stop::install_hook() {
        return false;
    }
    spawn_watcher(state);
    true
}

/// Polls the held-duration and trips the stop once the threshold is crossed.
///
/// Polling rather than firing from the hook callback keeps the callback free of
/// locks and allocation — it runs on the system input path, where blocking
/// would stutter every keystroke on the machine.
fn spawn_watcher(state: Shared) {
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
        let mut fired = false;

        loop {
            tick.tick().await;
            let down_at = ESC_DOWN_AT.load(Ordering::Relaxed);

            if down_at == 0 {
                fired = false;
                continue;
            }
            if fired {
                continue;
            }

            if crate::state::now_millis().saturating_sub(down_at) >= HOLD_MS {
                fired = true;
                if state.control().phase == crate::state::ControlPhase::Active {
                    state.abort();
                    let _ = crate::platform::apps::notify(
                        "conduit stopped",
                        "you have control back.",
                    );
                }
            }
        }
    });
}
