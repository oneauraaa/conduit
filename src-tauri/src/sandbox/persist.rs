//! `sandbox/state.json`: the sandboxes the user has made.
//!
//! Kept out of `Settings` on purpose. `store::load` answers any parse failure
//! by falling back to `Settings::default()` — which is the right call for a
//! handful of toggles, and the wrong one here: a single malformed sandbox
//! entry would silently reset the user's port, sharing token and tool
//! switches. A file of its own fails alone.
//!
//! Written the same way as the browser's state: to `.next`, then swapped in
//! with the old file kept as `.previous` until the new one is in place.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::model::{SandboxSpec, validate_spec};

const FILE: &str = "state.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxDisk {
    /// Stamped on every container conduit creates, and required before it
    /// will stop or delete one. Random per install.
    pub install_id: String,
    /// Stop running sandboxes when conduit quits, so a forgotten desktop does
    /// not hold gigabytes of RAM until the next reboot.
    #[serde(default = "default_true")]
    pub stop_on_quit: bool,
    #[serde(default)]
    pub sandboxes: Vec<SandboxSpec>,
}

fn default_true() -> bool {
    true
}

impl SandboxDisk {
    pub fn fresh() -> Self {
        Self {
            install_id: crate::random::token_hex(),
            stop_on_quit: true,
            sandboxes: Vec::new(),
        }
    }
}

/// Reads the state, falling back to the previous copy if the current one is
/// missing or does not validate. `None` means neither is usable.
pub fn load(root: &Path) -> Option<SandboxDisk> {
    [FILE, "state.json.previous"]
        .iter()
        .find_map(|name| read_valid(&root.join(name)))
}

fn read_valid(path: &Path) -> Option<SandboxDisk> {
    let bytes = fs::read(path).ok()?;
    let disk: SandboxDisk = serde_json::from_slice(&bytes).ok()?;
    validate(&disk).ok()?;
    Some(disk)
}

pub fn validate(disk: &SandboxDisk) -> Result<(), String> {
    if disk.install_id.len() < 16 || !disk.install_id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("the install id is malformed".into());
    }
    let mut ids = std::collections::HashSet::new();
    for spec in &disk.sandboxes {
        validate_spec(spec)?;
        if !ids.insert(spec.id.as_str()) {
            return Err(format!("two sandboxes share the id {:?}", spec.id));
        }
    }
    Ok(())
}

/// Moves an unreadable state file aside rather than overwriting it, so a bug
/// in this version can never destroy what a later one could have read.
pub fn quarantine(root: &Path) {
    let current = root.join(FILE);
    if current.exists() {
        let aside = root.join(format!(
            "state.json.unreadable-{}",
            crate::state::now_millis()
        ));
        let _ = fs::rename(current, aside);
    }
}

pub fn save(root: &Path, disk: &SandboxDisk) -> Result<(), String> {
    validate(disk)?;
    fs::create_dir_all(root).map_err(|e| format!("could not create the sandbox folder: {e}"))?;
    let next = root.join("state.json.next");
    let current = root.join(FILE);
    let previous = root.join("state.json.previous");
    let bytes = serde_json::to_vec_pretty(disk)
        .map_err(|e| format!("could not encode the sandbox list: {e}"))?;
    fs::write(&next, bytes).map_err(|e| format!("could not save the sandbox list: {e}"))?;
    if previous.exists() {
        fs::remove_file(&previous)
            .map_err(|e| format!("could not remove the old sandbox list: {e}"))?;
    }
    if current.exists() {
        fs::rename(&current, &previous)
            .map_err(|e| format!("could not preserve the sandbox list: {e}"))?;
    }
    if let Err(error) = fs::rename(&next, &current) {
        if previous.exists() {
            let _ = fs::rename(&previous, &current);
        }
        return Err(format!("could not activate the sandbox list: {error}"));
    }
    let _ = fs::remove_file(previous);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::model::SandboxOs;

    fn temp_root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "conduit-sandbox-test-{name}-{}",
            crate::random::token_hex()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn spec(id: &str) -> SandboxSpec {
        SandboxSpec {
            id: id.into(),
            name: id.to_uppercase(),
            os: SandboxOs::Debian12,
            memory_mb: 2048,
            cpus: 1.0,
            width: 1280,
            height: 800,
            internet: false,
            auto_start: true,
            created_at: 5,
        }
    }

    #[test]
    fn a_saved_list_loads_back_identically() {
        let root = temp_root("roundtrip");
        let mut disk = SandboxDisk::fresh();
        disk.sandboxes = vec![spec("work"), spec("play")];
        save(&root, &disk).unwrap();
        assert_eq!(load(&root), Some(disk.clone()));
        // A second save goes through the .previous swap and leaves no debris.
        disk.stop_on_quit = false;
        save(&root, &disk).unwrap();
        assert_eq!(load(&root), Some(disk));
        assert!(!root.join("state.json.previous").exists());
        assert!(!root.join("state.json.next").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_entries_are_refused_on_both_sides() {
        let root = temp_root("invalid");
        let mut disk = SandboxDisk::fresh();
        disk.sandboxes = vec![spec("work"), spec("work")];
        assert!(
            save(&root, &disk).is_err(),
            "duplicate ids must not be written"
        );

        // Written by hand, as a bug or an editor might.
        let mut bad = spec("x");
        bad.id = "--privileged".into();
        let json = serde_json::json!({
            "installId": disk.install_id,
            "sandboxes": [bad],
        });
        fs::write(root.join("state.json"), json.to_string()).unwrap();
        assert_eq!(load(&root), None);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_corrupt_current_file_falls_back_to_the_previous_one() {
        let root = temp_root("fallback");
        let mut disk = SandboxDisk::fresh();
        disk.sandboxes = vec![spec("work")];
        fs::write(
            root.join("state.json.previous"),
            serde_json::to_vec(&disk).unwrap(),
        )
        .unwrap();
        fs::write(root.join("state.json"), b"{ not json").unwrap();
        assert_eq!(load(&root), Some(disk));
        quarantine(&root);
        assert!(!root.join("state.json").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn older_files_get_defaults() {
        let disk: SandboxDisk =
            serde_json::from_str(r#"{"installId":"0123456789abcdef0123456789abcdef"}"#).unwrap();
        assert!(disk.stop_on_quit);
        assert!(disk.sandboxes.is_empty());
    }
}
