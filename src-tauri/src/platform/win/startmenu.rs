//! Resolving an application name to something launchable.
//!
//! macOS has Launch Services, so `open -a "Google Chrome"` resolves a fuzzy
//! display name across every Applications directory. Windows has no such
//! index, so this reconstructs one from the two places an installed app
//! reliably registers itself:
//!
//!   1. **App Paths** in the registry — how `Win+R` finds `chrome` without a
//!      full path. Keyed by executable name.
//!   2. **Start Menu shortcuts** — how the user finds it, keyed by the display
//!      name they actually see. This is the one that answers "open Chrome".
//!
//! Both are searched because neither is complete: CLIs and store apps are often
//! missing from one or the other.

use std::path::{Path, PathBuf};

use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_SZ, RegCloseKey,
    RegGetValueW, RegOpenKeyExW,
};
use windows::core::{HSTRING, w};

/// A concrete file `open_app` can hand to the shell.
pub struct Target(PathBuf);

/// Finds the best launch target for a name the user or agent typed.
///
/// The order matters, and one step of it is the whole reason this module is not
/// three lines long:
///
///   1. **App execution alias.** Windows 11's Notepad, Terminal and Calculator
///      are packaged Store apps, and their App Paths entry points *inside the
///      package* — `C:\Program Files\WindowsApps\Microsoft.WindowsNotepad_…\
///      Notepad.exe`. Executing that path directly skips package activation, so
///      the app dies on a missing dependency DLL
///      ("Microsoft.UI.Windowing.Core.dll was not found") before it ever draws
///      a window. The zero-byte alias in `WindowsApps` is a reparse point that
///      activates the package properly, and it must be tried *before* App
///      Paths, not after.
///   2. **Start Menu shortcut**, keyed on the display name a human recognises
///      ("Visual Studio Code", not "code.exe"). Launched as-is so its
///      arguments, working directory and icon survive.
///   3. **App Paths**, for classic installers with no shortcut.
pub fn resolve(name: &str) -> Option<Target> {
    let needle = name.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }

    let exe_names = if needle.ends_with(".exe") {
        vec![needle.clone()]
    } else {
        vec![format!("{needle}.exe"), needle.clone()]
    };

    for candidate in &exe_names {
        if let Some(alias) = execution_alias(candidate) {
            return Some(Target(alias));
        }
    }

    if let Some(lnk) = best_shortcut(&needle) {
        return Some(Target(lnk));
    }

    exe_names
        .iter()
        .find_map(|c| app_paths_lookup(c).or_else(|| path_lookup(c)))
        .map(Target)
}

/// Launches a resolved target.
pub fn launch(target: &Target) -> Result<(), String> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let path = &target.0;
    let file = HSTRING::from(path.as_os_str());

    // ShellExecuteW reports success as an HINSTANCE-shaped value: over 32 means
    // it worked, at or below is an error code.
    let result = unsafe { ShellExecuteW(None, w!("open"), &file, None, None, SW_SHOWNORMAL) };

    let code = result.0 as usize as u32;
    if code > 32 {
        Ok(())
    } else {
        Err(format!(
            "windows refused to launch {} (error {code})",
            path.display()
        ))
    }
}

/// A real file on disk representing `name`, for reading an icon out of.
///
/// Separate from [`resolve`] because the two want different things: launching a
/// packaged app must go through the shell by name, but *reading its icon* is
/// happiest with a concrete path, and the execution-alias stub carries the
/// right icon even though running it directly would fail.
pub fn resolve_path(name: &str) -> Option<PathBuf> {
    let needle = name.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }

    // Shortcut first here, unlike [`resolve`]: a `.lnk` carries the app's real
    // icon, while an execution alias is a zero-byte stub whose icon is
    // whatever the shell substitutes. App Paths comes before the alias for the
    // same reason — the packaged exe has artwork, the stub does not.
    if let Some(lnk) = best_shortcut(&needle) {
        return Some(lnk);
    }

    let candidates = if needle.ends_with(".exe") {
        vec![needle.clone()]
    } else {
        vec![format!("{needle}.exe"), needle.clone()]
    };

    candidates.into_iter().find_map(|c| {
        app_paths_lookup(&c)
            .or_else(|| path_lookup(&c))
            .or_else(|| execution_alias(&c))
    })
}

/// Searches `PATH` for an executable.
///
/// The registry and the Start Menu between them cover installed *applications*,
/// and miss the entire class of agent CLIs — npm, scoop, cargo and pipx all
/// drop an exe in a directory on `PATH` and register nothing. Claude Code is
/// exactly this: `~/.local/bin/claude.exe`, invisible to both other lookups,
/// carrying a perfectly good icon.
fn path_lookup(exe: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(exe))
        .find(|candidate| candidate.is_file())
}

/// Whether an app execution alias exists for `exe`.
///
/// These live in `%LOCALAPPDATA%\Microsoft\WindowsApps` as zero-byte reparse
/// points and are how every Store app is reachable by name. Checked for
/// existence only — launching goes through the shell by name.
fn execution_alias(exe: &str) -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    let path = PathBuf::from(local)
        .join(r"Microsoft\WindowsApps")
        .join(exe);
    path.exists().then_some(path)
}

/* ── registry App Paths ── */

fn app_paths_lookup(exe: &str) -> Option<PathBuf> {
    const SUBKEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\";

    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let path = HSTRING::from(format!("{SUBKEY}{exe}"));
        let mut key = HKEY::default();

        let opened =
            unsafe { RegOpenKeyExW(root, &path, Some(0), KEY_READ, &mut key) }.is_ok();
        if !opened {
            continue;
        }

        // The default (unnamed) value holds the full path to the executable.
        let mut size = 0u32;
        let probe = unsafe {
            RegGetValueW(key, None, None, RRF_RT_REG_SZ, None, None, Some(&mut size))
        };

        let value = if probe.is_ok() && size > 0 {
            let mut buf = vec![0u16; (size as usize / 2) + 1];
            let mut len = size;
            let got = unsafe {
                RegGetValueW(
                    key,
                    None,
                    None,
                    RRF_RT_REG_SZ,
                    None,
                    Some(buf.as_mut_ptr() as *mut _),
                    Some(&mut len),
                )
            };
            got.is_ok().then(|| {
                let s = String::from_utf16_lossy(&buf);
                s.trim_end_matches('\0').trim_matches('"').to_string()
            })
        } else {
            None
        };

        unsafe { RegCloseKey(key).ok() };

        if let Some(v) = value {
            // These are routinely REG_EXPAND_SZ — `%SystemRoot%\system32\...`.
            // `RegGetValueW` does not expand them, so without this the path
            // never exists on disk and every App Paths hit is silently missed.
            let p = PathBuf::from(expand_env(&v));
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

fn expand_env(value: &str) -> String {
    use windows::Win32::System::Environment::ExpandEnvironmentStringsW;

    if !value.contains('%') {
        return value.to_string();
    }

    let wide = HSTRING::from(value);
    let needed = unsafe { ExpandEnvironmentStringsW(&wide, None) };
    if needed == 0 {
        return value.to_string();
    }

    let mut buf = vec![0u16; needed as usize];
    let written = unsafe { ExpandEnvironmentStringsW(&wide, Some(&mut buf)) };
    if written == 0 {
        return value.to_string();
    }

    String::from_utf16_lossy(&buf[..written.saturating_sub(1) as usize])
}

/* ── Start Menu shortcuts ── */

fn start_menu_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        roots.push(PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    if let Some(program_data) = std::env::var_os("ProgramData") {
        roots.push(PathBuf::from(program_data).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    roots
}

/// The closest-matching Start Menu shortcut.
///
/// Ranked rather than first-match, on the same reasoning as `find_element`:
/// searching "code" hits both "Visual Studio Code" and "Visual Studio Code
/// (Insiders)", and the agent means the plain one. Exact beats prefix beats
/// substring, and shorter names break ties — "Chrome" should not resolve to
/// "Chrome Remote Desktop".
fn best_shortcut(needle: &str) -> Option<PathBuf> {
    let mut best: Option<(u8, usize, PathBuf)> = None;

    for root in start_menu_roots() {
        walk(&root, 0, &mut |path| {
            let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().to_lowercase()) else {
                return;
            };

            let rank = if stem == needle {
                0
            } else if stem.starts_with(needle) {
                1
            } else if stem.contains(needle) {
                2
            } else {
                return;
            };

            let candidate = (rank, stem.len(), path.to_path_buf());
            if best
                .as_ref()
                .is_none_or(|(r, l, _)| (rank, stem.len()) < (*r, *l))
            {
                best = Some(candidate);
            }
        });
    }

    best.map(|(_, _, p)| p)
}

/// Walks a Start Menu tree looking for `.lnk` files.
///
/// Depth-capped: these trees are shallow by convention, and a symlink loop in
/// someone's Programs folder should not hang a tool call.
fn walk(dir: &Path, depth: usize, f: &mut impl FnMut(&Path)) {
    const MAX_DEPTH: usize = 4;
    if depth > MAX_DEPTH {
        return;
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() => walk(&path, depth + 1, f),
            Ok(t) if t.is_file() => {
                if path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
                {
                    f(&path);
                }
            }
            _ => {}
        }
    }
}
