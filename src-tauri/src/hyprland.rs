//! The user's Hyprland keyboard shortcuts.
//!
//! ## Why
//!
//! conduit hands an agent the machine "the way a person uses it", and on
//! Hyprland a good deal of how a person uses it is their own keybinds. An agent
//! that wants a launcher and does not know about `SUPER + R` will invent a
//! chord and hit nothing.
//!
//! ## Two sources, on purpose
//!
//! `hyprctl binds -j` is the truth for *what is bound*: the compositor's live
//! table, after variable substitution, after `source` includes, after whatever
//! a Lua config generated in a loop. What it does not carry is anything a human
//! wrote — no comments, no `$mainMod`, no section headings, no file or line.
//!
//! The config file has all of that and none of the authority. So both are read
//! and merged: hyprctl decides what exists, the config decorates it. Either can
//! be missing and the other still produces a usable list.
//!
//! ## Parsing hyprlang correctly
//!
//! The wiki has been Lua-only since 0.55, so the rules below come from
//! Hyprland's own `src/config/legacy/ConfigManager.cpp` at v0.56.2. They are
//! the ones a naive parser gets wrong:
//!
//! * **Flag letters** follow `bind` with no separator: `l` locked, `r` release,
//!   `e` repeat, `m` mouse, `n` non-consuming, `a` auto-consuming,
//!   `t` transparent, `i` ignore-mods, `s` multi-key, `o` long-press,
//!   `d` has-description, `p` don't-inhibit, `c` click, `g` drag,
//!   `u` submap-universal, `k` per-device, `x` allow-input-capture. Any other
//!   letter makes Hyprland reject the line, so an unknown one is skipped here
//!   too rather than guessed at.
//!
//! * **Fields are split with a maximum count**, not on every comma, so the last
//!   field keeps its own commas — `exec, foo --a, b` is one argument string.
//!   `max = (description ? 5 : 4) + (device ? 1 : 0)`, laid out as mods, key,
//!   \[device\], \[description\], dispatcher, args. A `bindd` therefore shifts
//!   the dispatcher one to the right, and a `bindm` has no args field at all
//!   because its dispatcher sits in that slot.
//!
//! * **Mod names are matched as substrings** of the uppercased field: SHIFT 1,
//!   CAPS 2, CTRL/CONTROL 4, ALT/MOD1 8, MOD2 16, MOD3 32,
//!   SUPER/WIN/LOGO/MOD4/META 64, MOD5 128. Empty mods is legal and common.
//!
//! * **`source =` globs**, with `~` expanded, resolved relative to the file
//!   being parsed rather than to the main config.
//!
//! Lua configs are deliberately *not* interpreted. Literal `hl.bind(...)` calls
//! are read for their descriptions; anything generated in a loop is left to
//! hyprctl, which is the whole reason this module reads both.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

/* ── what the UI and the agent see ──────────────────────────── */

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HyprlandState {
    /// This session is Hyprland. Everything else is meaningless when false.
    pub available: bool,
    pub version: Option<String>,
    /// The file the binds below were read from.
    pub config_path: Option<String>,
    /// `"conf"` or `"lua"`.
    pub config_kind: Option<String>,
    /// The file Hyprland itself loaded, which is not always the one above.
    pub active_config_path: Option<String>,
    /// Set when those two disagree — see [`choose_config`].
    pub mismatch: Option<String>,
    pub binds: Vec<Keybind>,
    /// Populated when a source failed, verbatim. Never fatal on its own: one
    /// source can fail and the other still fills the list.
    pub error: Option<String>,
}

impl HyprlandState {
    fn absent() -> Self {
        Self {
            available: false,
            version: None,
            config_path: None,
            config_kind: None,
            active_config_path: None,
            mismatch: None,
            binds: Vec::new(),
            error: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Keybind {
    /// Stable enough to key a list on.
    pub id: String,
    /// Resolved modifier names, in the order people write them.
    pub mods: Vec<String>,
    /// The variable the config used for the modifiers, e.g. `$mainMod`.
    pub mod_alias: Option<String>,
    /// `Q`, `left`, `mouse:272`, `code:28`, `XF86AudioMute`, `catchall`.
    pub key: String,
    pub dispatcher: String,
    pub args: String,
    /// A `bindd` description, or the trailing `# comment` on the line.
    pub description: Option<String>,
    pub flags: Vec<String>,
    pub submap: Option<String>,
    /// The nearest comment heading above it, used to group the list.
    pub section: Option<String>,
    pub source_file: Option<String>,
    pub source_line: Option<u32>,
    /// Hyprland has this bind loaded. False means the config declares it but
    /// the compositor does not have it — a rejected line, or a stale file.
    pub active: bool,
    /// The other binds on this same chord.
    ///
    /// Hyprland does not stop at the first match: `handleKeybinds` collects
    /// every hit and runs them top to bottom, so these all fire together. It is
    /// nearly always an accident, and invisible until you look for it.
    pub also_fires: Vec<String>,
    #[serde(skip)]
    modmask: u32,
}

impl Keybind {
    /// `SUPER + SHIFT + P`, the way it would be written down.
    pub fn chord(&self) -> String {
        let mut parts = self.mods.clone();
        parts.push(self.key.clone());
        parts.join(" + ")
    }

    /// `exec hyprshot -m region`, or just `killactive`.
    pub fn action(&self) -> String {
        if self.args.is_empty() {
            self.dispatcher.clone()
        } else {
            format!("{} {}", self.dispatcher, self.args)
        }
    }

    /// What two binds must share to collide, and to be merged across sources.
    fn ident(&self) -> (u32, String, String, String) {
        // A `bindm` reaches hyprctl as dispatcher `mouse` with the real
        // dispatcher moved into the argument, so normalise to that shape.
        let (disp, args) = if self.flags.iter().any(|f| f == "mouse") {
            ("mouse".to_string(), self.dispatcher.to_ascii_lowercase())
        } else {
            (
                self.dispatcher.to_ascii_lowercase(),
                self.args.trim().to_string(),
            )
        };
        (self.modmask, self.key.to_ascii_lowercase(), disp, args)
    }

    fn chord_ident(&self) -> (u32, String) {
        (self.modmask, self.key.to_ascii_lowercase())
    }
}

/* ── detection ──────────────────────────────────────────────── */

/// Whether this session is Hyprland.
///
/// `HYPRLAND_INSTANCE_SIGNATURE` is the reliable half — Hyprland sets it for
/// every client it starts, and nothing else does. `XDG_CURRENT_DESKTOP` is the
/// fallback for a session started some other way; it is a colon-separated list,
/// not a single name.
pub fn available() -> bool {
    #[cfg(not(target_os = "linux"))]
    return false;

    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
            return true;
        }
        std::env::var("XDG_CURRENT_DESKTOP")
            .map(|d| {
                d.split(':')
                    .any(|part| part.eq_ignore_ascii_case("hyprland"))
            })
            .unwrap_or(false)
    }
}

/* ── modifiers ──────────────────────────────────────────────── */

/// Substring matches against the uppercased field, exactly as
/// `CKeybindManager::stringToModMask` does it. `CTRL_L` and `SUPERSHIFT` both
/// resolve, which is why this is not a token split.
const MOD_TOKENS: &[(&str, u32)] = &[
    ("SHIFT", 1),
    ("CAPS", 2),
    ("CTRL", 4),
    ("CONTROL", 4),
    ("ALT", 8),
    ("MOD1", 8),
    ("MOD2", 16),
    ("MOD3", 32),
    ("SUPER", 64),
    ("WIN", 64),
    ("LOGO", 64),
    ("MOD4", 64),
    ("META", 64),
    ("MOD5", 128),
];

fn mod_mask(field: &str) -> u32 {
    let upper = field.to_ascii_uppercase();
    MOD_TOKENS
        .iter()
        .filter(|(name, _)| upper.contains(name))
        .fold(0, |mask, (_, bit)| mask | bit)
}

/// Display order, which is the order people say them rather than bit order.
fn mod_names(mask: u32) -> Vec<String> {
    const ORDER: &[(u32, &str)] = &[
        (64, "SUPER"),
        (4, "CTRL"),
        (8, "ALT"),
        (1, "SHIFT"),
        (2, "CAPS"),
        (16, "MOD2"),
        (32, "MOD3"),
        (128, "MOD5"),
    ];
    ORDER
        .iter()
        .filter(|(bit, _)| mask & bit != 0)
        .map(|(_, name)| (*name).to_string())
        .collect()
}

/* ── bind flags ─────────────────────────────────────────────── */

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Flags {
    locked: bool,
    release: bool,
    repeat: bool,
    mouse: bool,
    non_consuming: bool,
    auto_consuming: bool,
    transparent: bool,
    ignore_mods: bool,
    multi_key: bool,
    long_press: bool,
    has_description: bool,
    dont_inhibit: bool,
    click: bool,
    drag: bool,
    submap_universal: bool,
    per_device: bool,
    allow_input_capture: bool,
}

impl Flags {
    /// `None` for any letter Hyprland does not know, because Hyprland rejects
    /// the whole line in that case. It is also what keeps `binds:drag_threshold`
    /// from being read as a bind with flags `s:drag_threshold`.
    fn parse(letters: &str) -> Option<Self> {
        let mut f = Flags::default();
        for c in letters.chars() {
            match c {
                'l' => f.locked = true,
                'r' => f.release = true,
                'e' => f.repeat = true,
                'm' => f.mouse = true,
                'n' => f.non_consuming = true,
                'a' => f.auto_consuming = true,
                't' => f.transparent = true,
                'i' => f.ignore_mods = true,
                's' => f.multi_key = true,
                'o' => f.long_press = true,
                'd' => f.has_description = true,
                'p' => f.dont_inhibit = true,
                // Both imply release, per the flag table.
                'c' => {
                    f.click = true;
                    f.release = true;
                }
                'g' => {
                    f.drag = true;
                    f.release = true;
                }
                'u' => f.submap_universal = true,
                'k' => f.per_device = true,
                'x' => f.allow_input_capture = true,
                _ => return None,
            }
        }
        Some(f)
    }

    fn names(self) -> Vec<String> {
        let mut out = Vec::new();
        for (on, name) in [
            (self.locked, "locked"),
            (self.release, "release"),
            (self.repeat, "repeat"),
            (self.mouse, "mouse"),
            (self.non_consuming, "non_consuming"),
            (self.auto_consuming, "auto_consuming"),
            (self.transparent, "transparent"),
            (self.ignore_mods, "ignore_mods"),
            (self.multi_key, "multi_key"),
            (self.long_press, "long_press"),
            (self.dont_inhibit, "dont_inhibit"),
            (self.click, "click"),
            (self.drag, "drag"),
            (self.submap_universal, "submap_universal"),
            (self.per_device, "per_device"),
            (self.allow_input_capture, "allow_input_capture"),
        ] {
            if on {
                out.push(name.to_string());
            }
        }
        out
    }
}

/* ── line-level hyprlang helpers ────────────────────────────── */

/// Splits a line into code and comment. hyprlang cuts at the first `#` that is
/// not backslash-escaped, so a `#` inside a command truncates the line — here
/// as there.
fn split_comment(line: &str) -> (&str, Option<&str>) {
    let bytes = line.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'#' && (i == 0 || bytes[i - 1] != b'\\') {
            let comment = line[i..].trim_start_matches('#').trim();
            return (&line[..i], Some(comment));
        }
    }
    (line, None)
}

/// Hyprland's `CVarList(value, max)`: split on commas until `max` fields exist,
/// then stop, so the final field keeps every comma it contains.
fn split_fields(value: &str, max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = value;
    while out.len() + 1 < max {
        match rest.find(',') {
            Some(i) => {
                out.push(rest[..i].trim().to_string());
                rest = &rest[i + 1..];
            }
            None => break,
        }
    }
    out.push(rest.trim().to_string());
    out
}

/// A decorated or free-standing comment that reads as a heading for what
/// follows. Commented-out config is explicitly not one — `# bind = ...` is a
/// disabled bind, not a section title.
fn comment_heading(comment: &str) -> Option<String> {
    let text = comment.trim();
    if text.is_empty() {
        return None;
    }

    let lower = text.to_ascii_lowercase();
    const CONFIG_LIKE: &[&str] = &[
        "bind",
        "unbind",
        "source",
        "exec",
        "monitor",
        "env",
        "windowrule",
        "layerrule",
        "workspace",
        "submap",
        "gesture",
        "device",
        "general",
        "decoration",
        "animation",
        "input",
        "misc",
        "$",
    ];
    if CONFIG_LIKE.iter().any(|kw| lower.starts_with(kw)) {
        return None;
    }

    let body = text
        .trim_matches('#')
        .trim()
        .trim_matches(|c| c == '-' || c == '=' || c == '*')
        .trim();

    if body.is_empty() {
        return None;
    }
    Some(body.to_ascii_lowercase())
}

/* ── path helpers ───────────────────────────────────────────── */

fn expand_tilde(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(raw)
}

/// `*` and `?` matching, enough for the `source = ~/.config/hypr/*.conf` form
/// that people actually write. Backtracking two-pointer, so no recursion.
fn glob_match(pattern: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (pattern.chars().collect(), name.chars().collect());
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut mark) = (usize::MAX, 0usize);

    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = pi;
            mark = ni;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Expands a `source =` pattern. Wildcards are only honoured in the last
/// component, which is where every real config puts them; a directory that
/// matches is skipped, as Hyprland skips it.
fn expand_glob(path: &Path) -> Vec<PathBuf> {
    let name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return Vec::new(),
    };

    if !name.contains('*') && !name.contains('?') {
        return if path.is_file() {
            vec![path.to_path_buf()]
        } else {
            Vec::new()
        };
    }

    let dir = path.parent().unwrap_or(Path::new("."));
    let mut out: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .flatten()
            .filter(|e| e.path().is_file())
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|f| glob_match(name, f))
            })
            .map(|e| e.path())
            .collect(),
        Err(_) => Vec::new(),
    };
    out.sort();
    out
}

/* ── the hyprlang parser ────────────────────────────────────── */

#[derive(Default)]
struct ConfigParser {
    /// `$name` -> value, longest name first so `$mod` cannot clobber `$mainMod`.
    vars: Vec<(String, String)>,
    binds: Vec<Keybind>,
    section: Option<String>,
    submap: Option<String>,
    visited: Vec<PathBuf>,
}

impl ConfigParser {
    fn substitute(&self, value: &str) -> String {
        if !value.contains('$') {
            return value.to_string();
        }
        let mut out = value.to_string();
        for (name, replacement) in &self.vars {
            if out.contains(name.as_str()) {
                out = out.replace(name.as_str(), replacement);
            }
        }
        out
    }

    fn set_var(&mut self, name: String, value: String) {
        let resolved = self.substitute(&value);
        self.vars.retain(|(n, _)| n != &name);
        self.vars.push((name, resolved));
        // Longest first, so a prefix name never eats a longer one.
        self.vars
            .sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
    }

    fn parse_file(&mut self, path: &Path, depth: usize) {
        if depth > 8 || self.visited.iter().any(|p| p == path) {
            return;
        }
        self.visited.push(path.to_path_buf());

        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };

        // Only category blocks are tracked, and only in the `name {` / `}`
        // shape hyprlang uses — counting braces anywhere would trip over a
        // command like `exec, foo {bar}`.
        let mut block_depth = 0usize;

        for (index, raw_line) in text.lines().enumerate() {
            let line_no = index as u32 + 1;
            let (code, comment) = split_comment(raw_line);
            let code = code.trim();

            if code.is_empty() {
                if let Some(heading) = comment.and_then(comment_heading) {
                    self.section = Some(heading);
                }
                continue;
            }

            if code == "}" {
                block_depth = block_depth.saturating_sub(1);
                continue;
            }
            if code.ends_with('{') {
                block_depth += 1;
                self.section = None;
                continue;
            }
            if block_depth > 0 {
                continue;
            }

            let Some(eq) = code.find('=') else { continue };
            let keyword = code[..eq].trim();
            let value = code[eq + 1..].trim();

            if let Some(name) = keyword.strip_prefix('$') {
                if !name.is_empty() {
                    self.set_var(format!("${name}"), value.to_string());
                }
                continue;
            }

            match keyword {
                "source" => {
                    let raw = self.substitute(value);
                    let expanded = expand_tilde(&raw);
                    let joined = if expanded.is_absolute() {
                        expanded
                    } else {
                        path.parent().unwrap_or(Path::new(".")).join(expanded)
                    };
                    for target in expand_glob(&joined) {
                        self.parse_file(&target, depth + 1);
                    }
                }
                "submap" => {
                    let name = self.substitute(value);
                    let name = name.trim();
                    self.submap = if name.is_empty() || name == "reset" {
                        None
                    } else {
                        Some(name.to_string())
                    };
                }
                "unbind" => {
                    let fields = split_fields(&self.substitute(value), 2);
                    if fields.len() == 2 {
                        let mask = mod_mask(&fields[0]);
                        let key = fields[1].to_ascii_lowercase();
                        self.binds
                            .retain(|b| b.modmask != mask || b.key.to_ascii_lowercase() != key);
                    }
                }
                k if k.starts_with("bind") => {
                    if let Some(bind) =
                        self.parse_bind(&k[4..], value, path, line_no, comment)
                    {
                        self.binds.push(bind);
                    }
                }
                // A heading carries across blank lines, comments and binds, but
                // not across unrelated config. Without this a `# hyprland-run
                // helper` comment sitting above two window rules ends up
                // labelling the next bind that happens to follow them.
                _ => self.section = None,
            }
        }
    }

    fn parse_bind(
        &self,
        letters: &str,
        value: &str,
        path: &Path,
        line_no: u32,
        comment: Option<&str>,
    ) -> Option<Keybind> {
        let flags = Flags::parse(letters)?;

        let max = if flags.has_description { 5 } else { 4 } + usize::from(flags.per_device);

        // Split first, substitute per field: a variable whose value contains a
        // comma would otherwise change how many fields there are.
        let raw_fields = split_fields(value, max);
        if raw_fields.len() < 3 {
            return None; // hyprland: "bind: too few args"
        }
        let fields: Vec<String> = raw_fields.iter().map(|f| self.substitute(f)).collect();

        let mut idx = 2;
        if flags.per_device {
            idx += 1;
        }
        let described = if flags.has_description {
            let d = fields.get(idx).cloned();
            idx += 1;
            d.filter(|d| !d.is_empty())
        } else {
            None
        };

        let dispatcher = fields.get(idx).cloned().unwrap_or_default();
        // A mouse bind has no argument field: its dispatcher occupies that slot.
        let args = if flags.mouse {
            String::new()
        } else {
            fields.get(idx + 1).cloned().unwrap_or_default()
        };

        if dispatcher.is_empty() {
            return None;
        }

        let mod_alias = raw_fields[0]
            .split_whitespace()
            .find(|token| token.starts_with('$'))
            .map(|token| token.to_string());

        Some(Keybind {
            id: String::new(), // assigned once the list is final
            mods: mod_names(mod_mask(&fields[0])),
            mod_alias,
            key: fields[1].clone(),
            dispatcher: dispatcher.to_ascii_lowercase(),
            args,
            description: described.or_else(|| {
                comment
                    .map(str::trim)
                    .filter(|c| !c.is_empty())
                    .map(str::to_string)
            }),
            flags: flags.names(),
            submap: self.submap.clone(),
            section: self.section.clone(),
            source_file: Some(path.display().to_string()),
            source_line: Some(line_no),
            active: false,
            also_fires: Vec::new(),
            modmask: mod_mask(&fields[0]),
        })
    }
}

fn parse_conf(path: &Path) -> Vec<Keybind> {
    let mut parser = ConfigParser::default();
    parser.parse_file(path, 0);
    parser.binds
}

/* ── the lua reader ─────────────────────────────────────────── */

/// Reads literal `hl.bind("KEYS", hl.dsp.<what>(...), { flags })` calls.
///
/// Deliberately shallow: a Lua config is a program, and the one this was built
/// against generates its workspace binds in a `for` loop. Running it is out of
/// the question and emulating it is a losing game, so anything not written
/// literally is left to hyprctl — which has it all anyway. What this recovers
/// that hyprctl cannot is the description, the section and the line number.
fn parse_lua(path: &Path) -> Vec<Keybind> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };

    let mut locals: Vec<(String, String)> = Vec::new();
    let mut section: Option<String> = None;
    let mut binds = Vec::new();

    for (index, raw_line) in text.lines().enumerate() {
        let line_no = index as u32 + 1;
        let line = raw_line.trim();

        if let Some(comment) = line.strip_prefix("--") {
            if !line.contains("hl.bind") {
                if let Some(heading) = comment_heading(comment.trim_start_matches('-')) {
                    section = Some(heading);
                }
            }
            continue;
        }

        // `local mainMod = "SUPER"` — the Lua spelling of `$mainMod`.
        if let Some(rest) = line.strip_prefix("local ") {
            if let Some((name, value)) = rest.split_once('=') {
                if let Some(literal) = lua_string(value) {
                    locals.push((name.trim().to_string(), literal));
                }
            }
        }

        let Some(call) = line.find("hl.bind(") else {
            continue;
        };
        let body = &line[call + "hl.bind(".len()..];

        // First argument: the key expression, up to the top-level comma.
        let Some(comma) = top_level_comma(body) else {
            continue;
        };
        let keys_expr = &body[..comma];
        let rest = &body[comma + 1..];

        let keys = lua_concat(keys_expr, &locals);
        if keys.is_empty() {
            continue;
        }

        let mods_part: Vec<&str> = keys.split('+').map(str::trim).collect();
        let Some((key, mod_tokens)) = mods_part.split_last() else {
            continue;
        };
        let mods_field = mod_tokens.join(" ");
        let mask = mod_mask(&mods_field);

        let dispatcher = rest
            .find("hl.dsp.")
            .map(|i| {
                rest[i + "hl.dsp.".len()..]
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
                    .collect::<String>()
            })
            .unwrap_or_else(|| "function".to_string());

        let args = lua_string(rest).unwrap_or_default();

        binds.push(Keybind {
            id: String::new(),
            mods: mod_names(mask),
            mod_alias: mod_tokens
                .iter()
                .find(|t| locals.iter().any(|(n, _)| n == *t))
                .map(|t| (*t).to_string()),
            key: (*key).to_string(),
            dispatcher,
            args,
            description: lua_description(rest),
            flags: lua_flags(rest),
            submap: None,
            section: section.clone(),
            source_file: Some(path.display().to_string()),
            source_line: Some(line_no),
            active: false,
            also_fires: Vec::new(),
            modmask: mask,
        });
    }

    binds
}

/// The first comma that is not inside a string or nested parentheses.
fn top_level_comma(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"') | (None, '\'') => quote = Some(c),
            (None, '(') | (None, '{') => depth += 1,
            (None, ')') | (None, '}') => depth -= 1,
            (None, ',') if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// The first double-quoted literal in an expression.
fn lua_string(expr: &str) -> Option<String> {
    let start = expr.find('"')?;
    let rest = &expr[start + 1..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Resolves `mainMod .. " + Q"` into `SUPER + Q`.
fn lua_concat(expr: &str, locals: &[(String, String)]) -> String {
    expr.split("..")
        .map(|piece| {
            let piece = piece.trim();
            match lua_string(piece) {
                Some(literal) => literal,
                None => locals
                    .iter()
                    .find(|(name, _)| name == piece)
                    .map(|(_, value)| value.clone())
                    .unwrap_or_default(),
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn lua_description(rest: &str) -> Option<String> {
    let at = rest.find("description")?;
    lua_string(&rest[at..])
}

fn lua_flags(rest: &str) -> Vec<String> {
    const KNOWN: &[&str] = &[
        "locked",
        "release",
        "repeating",
        "mouse",
        "non_consuming",
        "auto_consuming",
        "transparent",
        "ignore_mods",
        "long_press",
        "dont_inhibit",
        "click",
        "drag",
        "submap_universal",
        "allow_input_capture",
    ];
    KNOWN
        .iter()
        .filter(|flag| rest.contains(&format!("{flag} = true")))
        // `repeating` is the Lua name for what hyprlang calls `e`/repeat.
        .map(|flag| if *flag == "repeating" { "repeat" } else { flag }.to_string())
        .collect()
}

/* ── hyprctl ────────────────────────────────────────────────── */

#[derive(Debug, Deserialize)]
struct RawBind {
    #[serde(default)]
    locked: bool,
    #[serde(default)]
    mouse: bool,
    #[serde(default)]
    release: bool,
    #[serde(default)]
    repeat: bool,
    #[serde(default, rename = "longPress")]
    long_press: bool,
    #[serde(default)]
    non_consuming: bool,
    #[serde(default)]
    auto_consuming: bool,
    #[serde(default)]
    allow_input_capture: bool,
    #[serde(default)]
    modmask: u32,
    #[serde(default)]
    submap: String,
    #[serde(default)]
    key: String,
    #[serde(default)]
    keycode: i64,
    #[serde(default)]
    catch_all: bool,
    #[serde(default)]
    description: String,
    #[serde(default)]
    dispatcher: String,
    #[serde(default)]
    arg: String,
}

fn hyprctl(args: &[&str]) -> Result<String, String> {
    let out = Command::new("hyprctl")
        .args(args)
        .output()
        .map_err(|e| format!("could not run hyprctl: {e}"))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "hyprctl {} failed: {}",
            args.join(" "),
            stderr.trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn live_binds() -> Result<Vec<Keybind>, String> {
    let json = hyprctl(&["binds", "-j"])?;
    let raw: Vec<RawBind> =
        serde_json::from_str(&json).map_err(|e| format!("could not read hyprctl binds: {e}"))?;

    Ok(raw
        .into_iter()
        .map(|b| {
            let key = if b.catch_all {
                "catchall".to_string()
            } else if !b.key.is_empty() {
                b.key.clone()
            } else if b.keycode != 0 {
                format!("code:{}", b.keycode)
            } else {
                String::new()
            };

            // Undo hyprctl's mouse-bind shape, so the list reads the way the
            // config was written.
            let (dispatcher, args) = if b.mouse && b.dispatcher == "mouse" {
                (b.arg.clone(), String::new())
            } else {
                (b.dispatcher.clone(), b.arg.clone())
            };

            let mut flags = Vec::new();
            for (on, name) in [
                (b.locked, "locked"),
                (b.release, "release"),
                (b.repeat, "repeat"),
                (b.mouse, "mouse"),
                (b.non_consuming, "non_consuming"),
                (b.auto_consuming, "auto_consuming"),
                (b.long_press, "long_press"),
                (b.allow_input_capture, "allow_input_capture"),
            ] {
                if on {
                    flags.push(name.to_string());
                }
            }

            Keybind {
                id: String::new(),
                mods: mod_names(b.modmask),
                mod_alias: None,
                key,
                dispatcher: dispatcher.to_ascii_lowercase(),
                args,
                description: Some(b.description).filter(|d| !d.is_empty()),
                flags,
                submap: Some(b.submap).filter(|s| !s.is_empty()),
                section: None,
                source_file: None,
                source_line: None,
                active: true,
                also_fires: Vec::new(),
                modmask: b.modmask,
            }
        })
        .collect())
}

fn version() -> Option<String> {
    let json = hyprctl(&["version", "-j"]).ok()?;
    let value: serde_json::Value = serde_json::from_str(&json).ok()?;
    value
        .get("tag")
        .and_then(|t| t.as_str())
        .map(|t| t.to_string())
}

/* ── choosing a config file ─────────────────────────────────── */

/// Every directory Hyprland searches, in its order.
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        dirs.push(PathBuf::from(xdg).join("hypr"));
    }
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".config").join("hypr"));
        dirs.push(home.join(".hypr"));
    }
    if let Ok(xdg_dirs) = std::env::var("XDG_CONFIG_DIRS") {
        for dir in xdg_dirs.split(':').filter(|d| !d.is_empty()) {
            dirs.push(PathBuf::from(dir).join("hypr"));
        }
    }
    dirs.push(PathBuf::from("/etc/xdg/hypr"));

    dirs.dedup();
    dirs
}

/// The file Hyprland itself loaded.
///
/// Its own order, from `Config::Supplementary::Jeremy::getMainConfigPath` —
/// `HYPRLAND_CONFIG` first, then **lua before conf**. That is the opposite of
/// what conduit shows, which is why [`snapshot`] compares the two.
fn hyprland_choice() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("HYPRLAND_CONFIG").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(explicit));
    }
    for dir in search_dirs() {
        for name in ["hyprland.lua", "hyprland.conf"] {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

struct Chosen {
    path: PathBuf,
    kind: &'static str,
    binds: Vec<Keybind>,
}

fn read_config(path: &Path) -> (&'static str, Vec<Keybind>) {
    if path.extension().is_some_and(|e| e == "lua") {
        ("lua", parse_lua(path))
    } else {
        ("conf", parse_conf(path))
    }
}

/// The file conduit reads binds from.
///
/// `hyprland.conf` is preferred over `hyprland.lua` — the user's rule, and the
/// reverse of Hyprland's. The preference is conditional on the `.conf`
/// *having* binds: a `.conf` left behind empty should not hide a `.lua` that is
/// doing the work.
fn choose_config() -> Option<Chosen> {
    if let Some(explicit) = std::env::var_os("HYPRLAND_CONFIG").filter(|v| !v.is_empty()) {
        let path = PathBuf::from(explicit);
        let (kind, binds) = read_config(&path);
        return Some(Chosen { path, kind, binds });
    }

    let mut fallback: Option<Chosen> = None;

    for dir in search_dirs() {
        for name in ["hyprland.conf", "hyprland.lua"] {
            let path = dir.join(name);
            if !path.is_file() {
                continue;
            }
            let (kind, binds) = read_config(&path);
            if !binds.is_empty() {
                return Some(Chosen { path, kind, binds });
            }
            fallback.get_or_insert(Chosen { path, kind, binds });
        }
    }

    fallback
}

/* ── merging ────────────────────────────────────────────────── */

/// Config order first, so sections read the way they were written; anything
/// hyprctl knows about that the config did not yield goes on the end.
fn merge(config: Vec<Keybind>, live: Vec<Keybind>) -> Vec<Keybind> {
    let mut buckets: HashMap<(u32, String, String, String), Vec<usize>> = HashMap::new();
    for (i, bind) in live.iter().enumerate() {
        buckets.entry(bind.ident()).or_default().push(i);
    }

    let mut claimed = vec![false; live.len()];
    let mut out = Vec::with_capacity(config.len().max(live.len()));

    for mut bind in config {
        let matched = buckets
            .get(&bind.ident())
            .and_then(|idxs| idxs.iter().copied().find(|i| !claimed[*i]));

        if let Some(i) = matched {
            claimed[i] = true;
            // The compositor is authoritative about what a bind actually is;
            // the config is authoritative about how to describe it.
            bind.active = true;
            bind.flags = live[i].flags.clone();
            bind.submap = live[i].submap.clone();
            if bind.description.is_none() {
                bind.description = live[i].description.clone();
            }
        }
        out.push(bind);
    }

    for (i, bind) in live.into_iter().enumerate() {
        if !claimed[i] {
            out.push(bind);
        }
    }

    out
}

/// Fills in `also_fires` and the ids, once the list is final.
fn finish(mut binds: Vec<Keybind>) -> Vec<Keybind> {
    // Active binds only, on both sides. A bind the compositor never loaded does
    // not fire, so it neither joins anyone else's list nor gets one of its own —
    // it carries `active: false` instead, which is the accurate thing to say.
    let mut chords: HashMap<(u32, String), Vec<String>> = HashMap::new();
    for bind in binds.iter().filter(|b| b.active) {
        chords.entry(bind.chord_ident()).or_default().push(bind.action());
    }

    for (i, bind) in binds.iter_mut().enumerate() {
        if let Some(actions) = chords.get(&bind.chord_ident()).filter(|_| bind.active) {
            if actions.len() > 1 {
                let mine = bind.action();
                let mut seen_self = false;
                bind.also_fires = actions
                    .iter()
                    .filter(|a| {
                        if **a == mine && !seen_self {
                            seen_self = true;
                            return false;
                        }
                        true
                    })
                    .cloned()
                    .collect();
            }
        }
        bind.id = format!("{i}-{}-{}", bind.modmask, bind.key);
    }

    binds
}

/* ── the snapshot ───────────────────────────────────────────── */

pub fn snapshot() -> HyprlandState {
    if !available() {
        return HyprlandState::absent();
    }

    let mut errors: Vec<String> = Vec::new();

    let live = match live_binds() {
        Ok(binds) => binds,
        Err(e) => {
            errors.push(e);
            Vec::new()
        }
    };

    let chosen = choose_config();
    if chosen.is_none() {
        errors.push("no hyprland.conf or hyprland.lua found in any of the usual places".into());
    }

    let active_path = hyprland_choice();
    let mismatch = match (&chosen, &active_path) {
        (Some(c), Some(a)) if c.path != *a && !c.binds.is_empty() => Some(format!(
            "showing {}, but hyprland is running {}",
            c.path.display(),
            a.display()
        )),
        _ => None,
    };

    let (config_path, config_kind, config_binds) = match chosen {
        Some(c) => (
            Some(c.path.display().to_string()),
            Some(c.kind.to_string()),
            c.binds,
        ),
        None => (None, None, Vec::new()),
    };

    HyprlandState {
        available: true,
        version: version(),
        config_path,
        config_kind,
        active_config_path: active_path.map(|p| p.display().to_string()),
        mismatch,
        binds: finish(merge(config_binds, live)),
        error: if errors.is_empty() {
            None
        } else {
            Some(errors.join("; "))
        },
    }
}

/// The compact form the MCP tool hands an agent.
///
/// The full [`Keybind`] carries file paths, line numbers and flags that matter
/// to the tab and to nobody else; sending all of it would be a few thousand
/// tokens of noise per call.
pub fn agent_view(state: &HyprlandState) -> serde_json::Value {
    let binds: Vec<serde_json::Value> = state
        .binds
        .iter()
        .filter(|b| b.active || state.binds.iter().all(|o| !o.active))
        .map(|b| {
            let mut entry = serde_json::json!({
                "chord": b.chord(),
                "does": b.action(),
            });
            let map = entry.as_object_mut().expect("json object");
            if let Some(d) = &b.description {
                map.insert("description".into(), serde_json::json!(d));
            }
            if let Some(s) = &b.submap {
                map.insert("submap".into(), serde_json::json!(s));
            }
            if let Some(s) = &b.section {
                map.insert("group".into(), serde_json::json!(s));
            }
            entry
        })
        .collect();

    serde_json::json!({
        "compositor": "hyprland",
        "version": state.version,
        "config": state.config_path,
        "note": "these are the user's own shortcuts. prefer one of these over \
                 synthesizing a key combination — a chord that is not listed here \
                 most likely does nothing.",
        "count": binds.len(),
        "keybinds": binds,
    })
}

/* ── tests ──────────────────────────────────────────────────── */

#[cfg(test)]
mod tests {
    use super::*;

    /// Parses a config body from a string, the way `parse_conf` does a file.
    fn binds_from(text: &str) -> Vec<Keybind> {
        let dir = std::env::temp_dir().join(format!(
            "conduit-hypr-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hyprland.conf");
        std::fs::write(&path, text).unwrap();
        let binds = parse_conf(&path);
        std::fs::remove_dir_all(&dir).ok();
        binds
    }

    #[test]
    fn mod_mask_matches_hyprlands_substring_rule() {
        assert_eq!(mod_mask("SUPER"), 64);
        assert_eq!(mod_mask("SUPER SHIFT"), 65);
        assert_eq!(mod_mask("SUPER CTRL"), 68);
        assert_eq!(mod_mask("SUPER ALT"), 72);
        assert_eq!(mod_mask(""), 0);
        // Variables are resolved before this runs. An undefined one falls
        // through as literal text, which is also what Hyprland does with it.
        assert_eq!(mod_mask("$mainMod"), 0);
        // Aliases, and the substring behaviour that makes them work.
        assert_eq!(mod_mask("WIN"), 64);
        assert_eq!(mod_mask("CONTROL"), 4);
        assert_eq!(mod_mask("MOD1"), 8);
    }

    #[test]
    fn variables_are_substituted_and_the_alias_is_kept() {
        let binds = binds_from("$mainMod = SUPER\nbind = $mainMod, Q, exec, kitty\n");
        assert_eq!(binds.len(), 1);
        assert_eq!(binds[0].mods, vec!["SUPER"]);
        assert_eq!(binds[0].mod_alias.as_deref(), Some("$mainMod"));
        assert_eq!(binds[0].dispatcher, "exec");
        assert_eq!(binds[0].args, "kitty");
    }

    #[test]
    fn a_longer_variable_name_is_not_eaten_by_a_shorter_one() {
        let binds = binds_from(
            "$mod = ALT\n$mainMod = SUPER\nbind = $mainMod, Q, exec, kitty\n",
        );
        assert_eq!(binds[0].mods, vec!["SUPER"]);
    }

    /// The real trap: the argument field keeps every comma and operator it has.
    #[test]
    fn arguments_keep_their_commas() {
        let binds = binds_from(
            "bind = SUPER, M, exec, command -v hyprshutdown >/dev/null 2>&1 && hyprshutdown || hyprctl dispatch exit\n",
        );
        assert_eq!(
            binds[0].args,
            "command -v hyprshutdown >/dev/null 2>&1 && hyprshutdown || hyprctl dispatch exit"
        );

        let commas = binds_from("bind = SUPER, X, exec, foo --a, b, c\n");
        assert_eq!(commas[0].args, "foo --a, b, c");
    }

    #[test]
    fn empty_mods_and_stacked_flags() {
        let binds = binds_from(
            "bindel = , XF86AudioRaiseVolume, exec, wpctl set-volume -l 1 @DEFAULT_AUDIO_SINK@ 1%+\n",
        );
        assert_eq!(binds.len(), 1);
        assert!(binds[0].mods.is_empty());
        assert_eq!(binds[0].key, "XF86AudioRaiseVolume");
        assert_eq!(binds[0].args, "wpctl set-volume -l 1 @DEFAULT_AUDIO_SINK@ 1%+");
        assert!(binds[0].flags.contains(&"locked".to_string()));
        assert!(binds[0].flags.contains(&"repeat".to_string()));
    }

    /// A mouse bind has three fields: its dispatcher sits where args normally do.
    #[test]
    fn mouse_binds_have_no_argument_field() {
        let binds = binds_from("$mainMod = SUPER\nbindm = $mainMod, mouse:272, movewindow\n");
        assert_eq!(binds.len(), 1);
        assert_eq!(binds[0].key, "mouse:272");
        assert_eq!(binds[0].dispatcher, "movewindow");
        assert_eq!(binds[0].args, "");
        assert!(binds[0].flags.contains(&"mouse".to_string()));
    }

    /// `bindd` inserts a description, shifting dispatcher and args right by one.
    #[test]
    fn a_description_shifts_the_remaining_fields() {
        let binds = binds_from("bindd = SUPER, X, open my editor, exec, zed --new, -w\n");
        assert_eq!(binds.len(), 1);
        assert_eq!(binds[0].description.as_deref(), Some("open my editor"));
        assert_eq!(binds[0].dispatcher, "exec");
        assert_eq!(binds[0].args, "zed --new, -w");
    }

    #[test]
    fn a_trailing_comment_becomes_the_description() {
        let binds = binds_from(
            "bind = SUPER SHIFT, P, exec, hyprshot -m region   # select a region\n",
        );
        assert_eq!(binds[0].args, "hyprshot -m region");
        assert_eq!(binds[0].description.as_deref(), Some("select a region"));
    }

    #[test]
    fn headings_group_binds_and_commented_out_config_is_not_a_heading() {
        let binds = binds_from(
            "###################\n\
             ### KEYBINDINGS ###\n\
             ###################\n\
             bind = SUPER, Q, exec, kitty\n\
             # Move focus with mainMod + arrow keys\n\
             bind = SUPER, left, movefocus, l\n\
             # bind = SUPER, R, exec, rofi\n\
             bind = SUPER, right, movefocus, r\n",
        );
        assert_eq!(binds.len(), 3);
        assert_eq!(binds[0].section.as_deref(), Some("keybindings"));
        assert_eq!(
            binds[1].section.as_deref(),
            Some("move focus with mainmod + arrow keys")
        );
        // The commented-out bind must not become the next heading.
        assert_eq!(binds[2].section, binds[1].section);
    }

    #[test]
    fn a_heading_does_not_carry_across_unrelated_config() {
        let binds = binds_from(
            "# hyprland-run helper\n\
             windowrule = match:class ^(hyprland-run)$, float on\n\
             env = QS_ICON_THEME, Papirus-Dark\n\
             bind = SUPER, L, global, caelestia:lock\n",
        );
        assert_eq!(binds.len(), 1);
        assert_eq!(binds[0].section, None);
    }

    /// The heading must survive the `$mainMod = SUPER` that every generated
    /// config puts between `### KEYBINDINGS ###` and the binds themselves.
    #[test]
    fn a_heading_survives_a_variable_definition() {
        let binds = binds_from(
            "### KEYBINDINGS ###\n$mainMod = SUPER\nbind = $mainMod, Q, exec, kitty\n",
        );
        assert_eq!(binds[0].section.as_deref(), Some("keybindings"));
    }

    #[test]
    fn invalid_flag_letters_are_skipped_not_guessed() {
        // `binds:drag_threshold` starts with "bind" but is a config value.
        let binds = binds_from("binds:drag_threshold = 10\nbind = SUPER, Q, exec, kitty\n");
        assert_eq!(binds.len(), 1);
        assert_eq!(binds[0].key, "Q");

        assert!(Flags::parse("zz").is_none());
        assert!(Flags::parse("el").unwrap().locked);
        assert!(Flags::parse("el").unwrap().repeat);
        // `c` and `g` both imply release.
        assert!(Flags::parse("c").unwrap().release);
        assert!(Flags::parse("g").unwrap().release);
    }

    #[test]
    fn submaps_scope_the_binds_inside_them() {
        let binds = binds_from(
            "bind = SUPER, R, submap, resize\n\
             submap = resize\n\
             binde = , right, resizeactive, 10 0\n\
             submap = reset\n\
             bind = SUPER, Q, exec, kitty\n",
        );
        assert_eq!(binds.len(), 3);
        assert_eq!(binds[0].submap, None);
        assert_eq!(binds[1].submap.as_deref(), Some("resize"));
        assert_eq!(binds[1].args, "10 0");
        assert_eq!(binds[2].submap, None);
    }

    #[test]
    fn unbind_removes_an_earlier_bind() {
        let binds = binds_from(
            "bind = SUPER, Q, exec, kitty\nbind = SUPER, W, killactive,\nunbind = SUPER, Q\n",
        );
        assert_eq!(binds.len(), 1);
        assert_eq!(binds[0].key, "W");
        assert_eq!(binds[0].dispatcher, "killactive");
    }

    #[test]
    fn source_pulls_in_another_file_relative_to_the_one_including_it() {
        let dir = std::env::temp_dir().join(format!("conduit-hypr-src-{}", std::process::id()));
        let nested = dir.join("conf.d");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("extra.conf"), "bind = SUPER, E, exec, dolphin\n").unwrap();

        let main = dir.join("hyprland.conf");
        std::fs::write(
            &main,
            "bind = SUPER, Q, exec, kitty\nsource = conf.d/*.conf\n",
        )
        .unwrap();

        let binds = parse_conf(&main);
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(binds.len(), 2);
        assert_eq!(binds[1].key, "E");
        assert!(binds[1].source_file.as_ref().unwrap().ends_with("extra.conf"));
        assert_eq!(binds[1].source_line, Some(1));
    }

    #[test]
    fn a_source_cycle_terminates() {
        let dir = std::env::temp_dir().join(format!("conduit-hypr-cyc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.conf"), "bind = SUPER, Q, exec, kitty\nsource = b.conf\n")
            .unwrap();
        std::fs::write(dir.join("b.conf"), "source = a.conf\n").unwrap();

        let binds = parse_conf(&dir.join("a.conf"));
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(binds.len(), 1);
    }

    #[test]
    fn category_blocks_are_not_mistaken_for_binds() {
        let binds = binds_from(
            "general {\n    gaps_in = 5\n    layout = dwindle\n}\n\
             windowrule {\n    name = vesktop\n    float = on\n}\n\
             bind = SUPER, Q, exec, kitty\n",
        );
        assert_eq!(binds.len(), 1);
    }

    #[test]
    fn split_fields_stops_at_the_maximum() {
        assert_eq!(split_fields("a, b, c, d, e", 4), vec!["a", "b", "c", "d, e"]);
        assert_eq!(split_fields("a, b, c", 4), vec!["a", "b", "c"]);
        assert_eq!(split_fields(", b, c, d", 4), vec!["", "b", "c", "d"]);
    }

    #[test]
    fn a_hash_inside_a_command_truncates_the_line_as_hyprland_does() {
        let (code, comment) = split_comment("bind = SUPER, X, exec, echo # hi");
        assert_eq!(code.trim(), "bind = SUPER, X, exec, echo");
        assert_eq!(comment, Some("hi"));
    }

    #[test]
    fn duplicate_chords_are_reported_as_firing_together() {
        // Hyprland's handleKeybinds collects every match and runs them in
        // order; it does not stop at the first.
        let mut parsed = binds_from(
            "bind = SUPER, L, exec, hyprlock\nbind = SUPER, L, global, caelestia:lock\n",
        );
        // `finish` runs on merged output, where a loaded bind is already marked.
        for b in &mut parsed {
            b.active = true;
        }
        let binds = finish(parsed);
        assert_eq!(binds.len(), 2);
        assert_eq!(binds[0].also_fires, vec!["global caelestia:lock"]);
        assert_eq!(binds[1].also_fires, vec!["exec hyprlock"]);
    }

    /// A bind the compositor rejected does not run, so it must not be reported
    /// as running alongside the one that does — nor drag that one into a
    /// collision warning for a conflict that does not exist.
    #[test]
    fn a_bind_that_never_loaded_fires_with_nothing() {
        let mut binds = binds_from(
            "bind = SUPER, L, exec, hyprlock\nbind = SUPER, L, exec, ghost\n",
        );
        binds[0].active = true;
        let done = finish(binds);
        assert!(done[0].also_fires.is_empty(), "{:?}", done[0].also_fires);
        assert!(done[1].also_fires.is_empty(), "{:?}", done[1].also_fires);
    }

    #[test]
    fn a_lone_bind_fires_alone() {
        let mut parsed = binds_from("bind = SUPER, Q, exec, kitty\n");
        parsed[0].active = true;
        let binds = finish(parsed);
        assert!(binds[0].also_fires.is_empty());
    }

    /// Not a test so much as a diagnostic: dumps this machine's real binds.
    /// `cargo test --lib hyprland::tests::dump_this_machine -- --ignored --nocapture`
    #[test]
    #[ignore = "reads the developer's own hyprland config"]
    fn dump_this_machine() {
        let state = snapshot();
        println!("available: {}", state.available);
        println!("version:   {:?}", state.version);
        println!("config:    {:?} ({:?})", state.config_path, state.config_kind);
        println!("hyprland:  {:?}", state.active_config_path);
        println!("mismatch:  {:?}", state.mismatch);
        println!("error:     {:?}", state.error);
        println!("binds:     {}", state.binds.len());
        for b in &state.binds {
            println!(
                "  {:<28} {:<50} active={} section={:?} desc={:?} also={:?}",
                b.chord(),
                b.action(),
                b.active,
                b.section,
                b.description,
                b.also_fires,
            );
        }
    }

    #[test]
    fn glob_matching() {
        assert!(glob_match("*.conf", "extra.conf"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("a?c.conf", "abc.conf"));
        assert!(!glob_match("*.conf", "extra.lua"));
        assert!(!glob_match("a?c.conf", "ac.conf"));
    }

    #[test]
    fn lua_binds_are_read_literally() {
        let dir = std::env::temp_dir().join(format!("conduit-hypr-lua-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hyprland.lua");
        std::fs::write(
            &path,
            "local mainMod = \"SUPER\"\n\
             hl.bind(mainMod .. \" + Q\", hl.dsp.exec_cmd(\"kitty\"), { description = \"terminal\" })\n\
             hl.bind(\"XF86AudioMute\", hl.dsp.exec_cmd(\"wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle\"), { locked = true })\n",
        )
        .unwrap();

        let binds = parse_lua(&path);
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(binds.len(), 2);
        assert_eq!(binds[0].mods, vec!["SUPER"]);
        assert_eq!(binds[0].key, "Q");
        assert_eq!(binds[0].dispatcher, "exec_cmd");
        assert_eq!(binds[0].args, "kitty");
        assert_eq!(binds[0].description.as_deref(), Some("terminal"));
        assert!(binds[1].mods.is_empty());
        assert_eq!(binds[1].key, "XF86AudioMute");
        assert!(binds[1].flags.contains(&"locked".to_string()));
    }

    /// The compositor decides what a bind *is*; the config decides how it reads.
    #[test]
    fn merging_keeps_the_config_prose_and_the_live_truth() {
        let config = binds_from(
            "bind = SUPER, Q, exec, kitty   # my terminal\nbind = SUPER, Z, exec, ghost\n",
        );
        let live = vec![Keybind {
            id: String::new(),
            mods: vec!["SUPER".into()],
            mod_alias: None,
            key: "Q".into(),
            dispatcher: "exec".into(),
            args: "kitty".into(),
            description: None,
            flags: vec!["locked".into()],
            submap: None,
            section: None,
            source_file: None,
            source_line: None,
            active: true,
            also_fires: Vec::new(),
            modmask: 64,
        }];

        let merged = finish(merge(config, live));
        assert_eq!(merged.len(), 2);
        assert!(merged[0].active);
        assert_eq!(merged[0].description.as_deref(), Some("my terminal"));
        assert_eq!(merged[0].flags, vec!["locked"]);
        assert!(merged[0].source_line.is_some());
        // Declared in the config but not loaded by the compositor.
        assert!(!merged[1].active);
    }

    #[test]
    fn live_binds_the_config_did_not_yield_are_still_listed() {
        let live = vec![Keybind {
            id: String::new(),
            mods: vec!["SUPER".into()],
            mod_alias: None,
            key: "5".into(),
            dispatcher: "workspace".into(),
            args: "5".into(),
            description: None,
            flags: Vec::new(),
            submap: None,
            section: None,
            source_file: None,
            source_line: None,
            active: true,
            also_fires: Vec::new(),
            modmask: 64,
        }];
        let merged = finish(merge(Vec::new(), live));
        assert_eq!(merged.len(), 1);
        assert!(merged[0].active);
        assert_eq!(merged[0].chord(), "SUPER + 5");
    }
}
