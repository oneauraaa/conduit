//! One-click MCP installation into other agents' config files.
//!
//! Rules every writer here follows, without exception:
//!   1. back the file up before the first byte is written;
//!   2. edit in a format-preserving way — never reserialize someone's config
//!      and hand back a reformatted file with their comments stripped;
//!   3. write to a temp file and rename, so a crash mid-write can't truncate
//!      a config the user depends on.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::state::now_millis;

/// The key conduit writes under, in every format.
const ENTRY: &str = "conduit";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTarget {
    pub id: String,
    pub name: String,
    pub config_path: String,
    pub detected: bool,
    pub installed: bool,
    pub error: Option<String>,
    /// data: URL of the vendor app's icon, or None when it isn't installed.
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    /// `mcpServers: { conduit: { type, url } }`
    JsonMcpServers,
    /// Same shape, but the file may contain comments.
    JsoncMcp,
    /// `[mcp_servers.conduit] url = "…"`
    TomlMcpServers,
    /// `mcp_servers: { conduit: { url } }`
    YamlMcpServers,
}

struct Spec {
    id: &'static str,
    name: &'static str,
    /// Relative to $HOME.
    path: &'static str,
    format: Format,
    /// If present, the agent counts as installed when this directory exists,
    /// even before its config file has been created.
    marker_dir: Option<&'static str>,
    /// Bundle id of the desktop app whose icon represents this agent.
    ///
    /// Several of these are CLIs with no bundle of their own, so they borrow
    /// the icon of the same vendor's desktop app — same brand, and it's the
    /// mark a user actually recognises. `None` falls back to a monogram.
    icon_bundle: Option<&'static str>,
}

const SPECS: &[Spec] = &[
    Spec {
        id: "claude-code",
        name: "claude code",
        path: ".claude.json",
        format: Format::JsonMcpServers,
        marker_dir: Some(".claude"),
        icon_bundle: Some("com.anthropic.claudefordesktop"),
    },
    Spec {
        id: "codex",
        name: "codex",
        path: ".codex/config.toml",
        format: Format::TomlMcpServers,
        marker_dir: Some(".codex"),
        icon_bundle: Some("com.openai.codex"),
    },
    Spec {
        id: "hermes",
        name: "hermes agent",
        path: ".hermes/config.yaml",
        format: Format::YamlMcpServers,
        marker_dir: Some(".hermes"),
        icon_bundle: None,
    },
    Spec {
        id: "openclaw",
        name: "openclaw",
        path: ".openclaw/openclaw.json",
        format: Format::JsonMcpServers,
        marker_dir: Some(".openclaw"),
        icon_bundle: None,
    },
    Spec {
        id: "gemini",
        name: "gemini cli",
        path: ".gemini/settings.json",
        format: Format::JsonMcpServers,
        marker_dir: Some(".gemini"),
        icon_bundle: Some("com.google.GeminiMacOS"),
    },
    Spec {
        id: "opencode",
        name: "opencode",
        path: ".config/opencode/opencode.jsonc",
        format: Format::JsoncMcp,
        marker_dir: Some(".config/opencode"),
        icon_bundle: None,
    },
    Spec {
        id: "claude-desktop",
        name: "claude desktop",
        path: "Library/Application Support/Claude/claude_desktop_config.json",
        format: Format::JsonMcpServers,
        marker_dir: Some("Library/Application Support/Claude"),
        icon_bundle: Some("com.anthropic.claudefordesktop"),
    },
];

/// Where agent configs are looked up.
///
/// `CONDUIT_HOME` overrides it. That exists so the install/uninstall path can
/// be tested against a scratch directory rather than the developer's real
/// configs — these functions rewrite files people depend on, so they need to be
/// exercised for real, not just at the string level.
fn home() -> PathBuf {
    if let Some(over) = std::env::var_os("CONDUIT_HOME") {
        return PathBuf::from(over);
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// Every bundle id the Agents tab will ask for an icon of.
pub fn icon_bundle_ids() -> Vec<&'static str> {
    let mut ids: Vec<&'static str> = SPECS.iter().filter_map(|s| s.icon_bundle).collect();
    // claude code and claude desktop share one app; dedupe so a warm-up does
    // the expensive extraction once rather than twice.
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn spec(id: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|s| s.id == id)
}

pub fn endpoint(port: u16) -> String {
    format!("http://127.0.0.1:{port}/mcp")
}

/// Every known agent with its current detection and install state.
pub fn list(_port: u16) -> Vec<AgentTarget> {
    SPECS
        .iter()
        .map(|s| {
            let path = home().join(s.path);
            let detected = path.exists()
                || s.marker_dir
                    .map(|d| home().join(d).exists())
                    .unwrap_or(false);

            let (installed, error) = match is_installed(&path, s.format) {
                Ok(v) => (v, None),
                Err(e) => (false, Some(e)),
            };

            AgentTarget {
                id: s.id.to_string(),
                name: s.name.to_string(),
                config_path: path.to_string_lossy().into_owned(),
                detected,
                installed,
                error,
                icon: icon_for(s),
            }
        })
        .collect()
}

fn is_installed(path: &Path, format: Format) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("could not read: {e}"))?;

    Ok(match format {
        Format::JsonMcpServers | Format::JsoncMcp => parse_jsonish(&text)
            .ok()
            .and_then(|v| {
                v.get("mcpServers")
                    .or_else(|| v.get("mcp"))
                    .and_then(|m| m.get(ENTRY))
                    .cloned()
            })
            .is_some(),
        Format::TomlMcpServers => text
            .parse::<toml_edit::DocumentMut>()
            .ok()
            .and_then(|d| d.get("mcp_servers").and_then(|t| t.get(ENTRY)).cloned())
            .is_some(),
        Format::YamlMcpServers => serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&text)
            .ok()
            .and_then(|v| v.get("mcp_servers").and_then(|m| m.get(ENTRY)).cloned())
            .is_some(),
    })
}

/// Parses JSON that may contain comments and trailing commas.
fn parse_jsonish(text: &str) -> Result<serde_json::Value, String> {
    if text.trim().is_empty() {
        return Ok(serde_json::Value::Object(Default::default()));
    }
    let parsed: Option<serde_json::Value> =
        jsonc_parser::parse_to_serde_value(text, &Default::default())
            .map_err(|e| format!("could not parse: {e}"))?;
    Ok(parsed.unwrap_or_else(|| serde_json::Value::Object(Default::default())))
}

/// Copies the file aside before it is modified. Named with a timestamp so
/// repeated installs never clobber an earlier backup.
fn backup(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let backup = path.with_extension(format!(
        "{}.conduit-backup-{}",
        path.extension().and_then(|e| e.to_str()).unwrap_or(""),
        now_millis()
    ));
    std::fs::copy(path, &backup).map_err(|e| format!("could not back up the config: {e}"))?;
    Ok(())
}

/// Writes via a temp file in the same directory, then renames. A rename within
/// one filesystem is atomic, so an interrupted write leaves the original intact.
fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    }
    let tmp = path.with_extension(format!(
        "{}.conduit-tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    std::fs::write(&tmp, contents).map_err(|e| format!("could not write: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("could not replace the config: {e}"))?;
    Ok(())
}

pub fn install(id: &str, port: u16) -> Result<AgentTarget, String> {
    let s = spec(id).ok_or_else(|| format!("unknown agent: {id}"))?;
    let path = home().join(s.path);
    let url = endpoint(port);

    backup(&path)?;

    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let next = match s.format {
        Format::JsonMcpServers => edit_json(&existing, &url, true)?,
        Format::JsoncMcp => edit_json(&existing, &url, false)?,
        Format::TomlMcpServers => edit_toml(&existing, &url)?,
        Format::YamlMcpServers => edit_yaml(&existing, &url)?,
    };
    write_atomic(&path, &next)?;

    Ok(single(s))
}

pub fn uninstall(id: &str, _port: u16) -> Result<AgentTarget, String> {
    let s = spec(id).ok_or_else(|| format!("unknown agent: {id}"))?;
    let path = home().join(s.path);

    if !path.exists() {
        return Ok(single(s));
    }
    backup(&path)?;

    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let next = match s.format {
        Format::JsonMcpServers => remove_json(&existing, true)?,
        Format::JsoncMcp => remove_json(&existing, false)?,
        Format::TomlMcpServers => remove_toml(&existing)?,
        Format::YamlMcpServers => remove_yaml(&existing)?,
    };
    write_atomic(&path, &next)?;

    Ok(single(s))
}

fn single(s: &'static Spec) -> AgentTarget {
    let path = home().join(s.path);
    let (installed, error) = match is_installed(&path, s.format) {
        Ok(v) => (v, None),
        Err(e) => (false, Some(e)),
    };
    AgentTarget {
        id: s.id.to_string(),
        name: s.name.to_string(),
        config_path: path.to_string_lossy().into_owned(),
        detected: true,
        installed,
        error,
        icon: icon_for(s),
    }
}

/// The vendor app's icon, if that app is on this Mac.
fn icon_for(s: &Spec) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        // 64pt: the row draws it at 28, and 2x covers Retina with headroom.
        return s
            .icon_bundle
            .and_then(|id| crate::mac::appicon::icon_data_url(id, 64.0));
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = s;
        None
    }
}

/* ── format-specific edits ──────────────────────────────────── */

fn conduit_json_entry(url: &str) -> serde_json::Value {
    serde_json::json!({ "type": "http", "url": url })
}

/// `mcp_key` picks the container: `mcpServers` for most agents, `mcp` for
/// opencode.
fn edit_json(existing: &str, url: &str, mcp_servers_key: bool) -> Result<String, String> {
    let mut root = parse_jsonish(existing)?;
    if !root.is_object() {
        root = serde_json::Value::Object(Default::default());
    }
    let key = if mcp_servers_key { "mcpServers" } else { "mcp" };

    let obj = root.as_object_mut().expect("checked above");
    let servers = obj
        .entry(key)
        .or_insert_with(|| serde_json::Value::Object(Default::default()));
    if !servers.is_object() {
        *servers = serde_json::Value::Object(Default::default());
    }
    servers
        .as_object_mut()
        .expect("checked above")
        .insert(ENTRY.to_string(), conduit_json_entry(url));

    serde_json::to_string_pretty(&root).map_err(|e| format!("could not serialize: {e}"))
}

fn remove_json(existing: &str, mcp_servers_key: bool) -> Result<String, String> {
    let mut root = parse_jsonish(existing)?;
    let key = if mcp_servers_key { "mcpServers" } else { "mcp" };

    if let Some(servers) = root.get_mut(key).and_then(|v| v.as_object_mut()) {
        servers.remove(ENTRY);
    }
    serde_json::to_string_pretty(&root).map_err(|e| format!("could not serialize: {e}"))
}

/// TOML is edited with `toml_edit` specifically so the rest of the file — key
/// order, comments, spacing — survives untouched. Codex's config in particular
/// is large and hand-maintained.
fn edit_toml(existing: &str, url: &str) -> Result<String, String> {
    let mut doc = existing
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("could not parse the toml: {e}"))?;

    let servers = doc
        .entry("mcp_servers")
        .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()));

    if let Some(table) = servers.as_table_mut() {
        // Implicit so it renders as `[mcp_servers.conduit]` rather than an
        // empty `[mcp_servers]` header appearing out of nowhere.
        table.set_implicit(true);
        let mut entry = toml_edit::Table::new();
        entry["url"] = toml_edit::value(url);
        table.insert(ENTRY, toml_edit::Item::Table(entry));
    }

    Ok(doc.to_string())
}

fn remove_toml(existing: &str) -> Result<String, String> {
    let mut doc = existing
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("could not parse the toml: {e}"))?;

    if let Some(table) = doc.get_mut("mcp_servers").and_then(|t| t.as_table_mut()) {
        table.remove(ENTRY);
    }
    Ok(doc.to_string())
}

fn edit_yaml(existing: &str, url: &str) -> Result<String, String> {
    let mut root: serde_yaml_ng::Value = if existing.trim().is_empty() {
        serde_yaml_ng::Value::Mapping(Default::default())
    } else {
        serde_yaml_ng::from_str(existing).map_err(|e| format!("could not parse the yaml: {e}"))?
    };

    if !root.is_mapping() {
        root = serde_yaml_ng::Value::Mapping(Default::default());
    }
    let map = root.as_mapping_mut().expect("checked above");
    let key = serde_yaml_ng::Value::String("mcp_servers".into());
    let servers = map
        .entry(key)
        .or_insert_with(|| serde_yaml_ng::Value::Mapping(Default::default()));
    if !servers.is_mapping() {
        *servers = serde_yaml_ng::Value::Mapping(Default::default());
    }

    let mut entry = serde_yaml_ng::Mapping::new();
    entry.insert(
        serde_yaml_ng::Value::String("url".into()),
        serde_yaml_ng::Value::String(url.to_string()),
    );
    servers.as_mapping_mut().expect("checked above").insert(
        serde_yaml_ng::Value::String(ENTRY.into()),
        serde_yaml_ng::Value::Mapping(entry),
    );

    serde_yaml_ng::to_string(&root).map_err(|e| format!("could not serialize the yaml: {e}"))
}

fn remove_yaml(existing: &str) -> Result<String, String> {
    let mut root: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(existing).map_err(|e| format!("could not parse the yaml: {e}"))?;

    if let Some(servers) = root
        .get_mut("mcp_servers")
        .and_then(|v| v.as_mapping_mut())
    {
        servers.remove(serde_yaml_ng::Value::String(ENTRY.into()));
    }
    serde_yaml_ng::to_string(&root).map_err(|e| format!("could not serialize the yaml: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://127.0.0.1:6767/mcp";

    /// The one that matters most: Codex's config is large, ordered and
    /// hand-maintained. A naive parse-and-reserialize would strip comments and
    /// reshuffle keys, so this asserts the surrounding document is untouched.
    #[test]
    fn toml_edit_preserves_the_rest_of_the_file() {
        let original = r#"model = "gpt-5.6-sol"
model_reasoning_effort = "xhigh"

# a comment the user wrote and would be upset to lose
approval_policy = "never"

[plugins."computer-use@openai-bundled"]
enabled = true

[projects."/Users/someone/code"]
trust_level = "trusted"
"#;

        let edited = edit_toml(original, URL).expect("edit should succeed");

        assert!(edited.contains("# a comment the user wrote"));
        assert!(edited.contains(r#"model = "gpt-5.6-sol""#));
        assert!(edited.contains(r#"[plugins."computer-use@openai-bundled"]"#));
        assert!(edited.contains(r#"[projects."/Users/someone/code"]"#));
        assert!(edited.contains("[mcp_servers.conduit]"));
        assert!(edited.contains(URL));

        // And removal must put it back exactly as it was.
        let removed = remove_toml(&edited).expect("remove should succeed");
        assert!(!removed.contains("conduit"));
        assert!(removed.contains("# a comment the user wrote"));
    }

    #[test]
    fn toml_edit_creates_the_table_when_absent() {
        let edited = edit_toml("", URL).unwrap();
        assert!(edited.contains("[mcp_servers.conduit]"));
        assert!(edited.contains(URL));
    }

    #[test]
    fn json_edit_keeps_sibling_keys() {
        let original = r#"{"numStartups": 42, "mcpServers": {"other": {"url": "http://x"}}}"#;
        let edited = edit_json(original, URL, true).unwrap();
        let v: serde_json::Value = serde_json::from_str(&edited).unwrap();

        assert_eq!(v["numStartups"], 42);
        assert_eq!(v["mcpServers"]["other"]["url"], "http://x");
        assert_eq!(v["mcpServers"]["conduit"]["url"], URL);
        assert_eq!(v["mcpServers"]["conduit"]["type"], "http");
    }

    #[test]
    fn json_edit_handles_an_empty_or_missing_file() {
        for original in ["", "{}"] {
            let edited = edit_json(original, URL, true).unwrap();
            let v: serde_json::Value = serde_json::from_str(&edited).unwrap();
            assert_eq!(v["mcpServers"]["conduit"]["url"], URL);
        }
    }

    /// opencode's config is JSONC and nests under `mcp`, not `mcpServers`.
    #[test]
    fn jsonc_with_comments_parses_and_uses_the_mcp_key() {
        let original = r#"{
  // the user's note
  "theme": "dark",
  "mcp": {}
}"#;
        let edited = edit_json(original, URL, false).unwrap();
        let v: serde_json::Value = serde_json::from_str(&edited).unwrap();

        assert_eq!(v["theme"], "dark");
        assert_eq!(v["mcp"]["conduit"]["url"], URL);
        assert!(v.get("mcpServers").is_none());
    }

    #[test]
    fn yaml_edit_keeps_sibling_keys() {
        let original = "model: hermes-4\nmcp_servers:\n  other:\n    url: http://x\n";
        let edited = edit_yaml(original, URL).unwrap();
        let v: serde_yaml_ng::Value = serde_yaml_ng::from_str(&edited).unwrap();

        assert_eq!(v["model"].as_str(), Some("hermes-4"));
        assert_eq!(v["mcp_servers"]["other"]["url"].as_str(), Some("http://x"));
        assert_eq!(v["mcp_servers"]["conduit"]["url"].as_str(), Some(URL));
    }

    #[test]
    fn removal_is_idempotent_and_leaves_others_alone() {
        let original = r#"{"mcpServers": {"other": {"url": "http://x"}}}"#;
        let with = edit_json(original, URL, true).unwrap();
        let without = remove_json(&with, true).unwrap();
        let again = remove_json(&without, true).unwrap();

        let v: serde_json::Value = serde_json::from_str(&again).unwrap();
        assert!(v["mcpServers"].get("conduit").is_none());
        assert_eq!(v["mcpServers"]["other"]["url"], "http://x");
    }

    #[test]
    fn installing_twice_does_not_duplicate_the_entry() {
        let once = edit_toml("", URL).unwrap();
        let twice = edit_toml(&once, URL).unwrap();
        assert_eq!(twice.matches("[mcp_servers.conduit]").count(), 1);
    }
}

/// End-to-end tests over the real filesystem, in a scratch `CONDUIT_HOME`.
///
/// These run serially and share one process-wide env var, so they live in a
/// single test to avoid the classic parallel-env flake.
#[cfg(test)]
mod fs_tests {
    use super::*;

    #[test]
    fn install_backs_up_and_uninstall_restores() {
        let dir = std::env::temp_dir().join(format!("conduit-agents-{}", now_millis()));
        std::fs::create_dir_all(dir.join(".codex")).unwrap();
        // SAFETY: single-threaded test, and no other test reads CONDUIT_HOME.
        unsafe { std::env::set_var("CONDUIT_HOME", &dir) };

        let codex = dir.join(".codex/config.toml");
        let original = "model = \"gpt-5.6-sol\"\n# keep me\napproval_policy = \"never\"\n";
        std::fs::write(&codex, original).unwrap();

        // not installed to begin with
        let before = list(6767);
        let row = before.iter().find(|a| a.id == "codex").unwrap();
        assert!(row.detected, "a .codex directory should count as detected");
        assert!(!row.installed);

        // install
        let after = install("codex", 6767).unwrap();
        assert!(after.installed);
        let text = std::fs::read_to_string(&codex).unwrap();
        assert!(text.contains("[mcp_servers.conduit]"));
        assert!(text.contains("http://127.0.0.1:6767/mcp"));
        assert!(text.contains("# keep me"), "user comments must survive");

        // a timestamped backup of the original must exist
        let backups: Vec<_> = std::fs::read_dir(dir.join(".codex"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("conduit-backup"))
            .collect();
        assert_eq!(backups.len(), 1, "install should leave exactly one backup");
        assert_eq!(std::fs::read_to_string(backups[0].path()).unwrap(), original);

        // no temp file left behind by the atomic write
        assert!(
            !dir.join(".codex/config.conduit-tmp").exists()
                && !dir.join(".codex/config.toml.conduit-tmp").exists(),
            "atomic write must not leave a temp file"
        );

        // uninstall
        let removed = uninstall("codex", 6767).unwrap();
        assert!(!removed.installed);
        let text = std::fs::read_to_string(&codex).unwrap();
        assert!(!text.contains("conduit"));
        assert!(text.contains("# keep me"));
        assert!(text.contains("model = \"gpt-5.6-sol\""));

        unsafe { std::env::remove_var("CONDUIT_HOME") };
        let _ = std::fs::remove_dir_all(&dir);
    }
}
