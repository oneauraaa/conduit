//! Settings persistence, on top of tauri-plugin-store.
//!
//! The frontend uses the same store file for its theme, so both halves of the
//! app keep their preferences in one place the user can inspect or delete.

use tauri::{AppHandle, Wry};
use tauri_plugin_store::StoreExt;

use crate::state::Settings;

const FILE: &str = "conduit.json";
const KEY: &str = "settings";

pub fn load(app: &AppHandle<Wry>) -> Settings {
    let Ok(store) = app.store(FILE) else {
        return Settings::default();
    };
    store
        .get(KEY)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

pub fn save(app: &AppHandle<Wry>, settings: &Settings) {
    let Ok(store) = app.store(FILE) else {
        return;
    };
    if let Ok(value) = serde_json::to_value(settings) {
        store.set(KEY, value);
        // Persist immediately: the app is expected to be force-quit from the
        // tray, so waiting for an autosave tick would lose the change.
        let _ = store.save();
    }
}
