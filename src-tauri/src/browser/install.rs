use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use reqwest::header::RANGE;
use sha2::{Digest, Sha256};
use url::Url;
use zip::ZipArchive;

use super::model::{BrowserInstallStatus, RuntimeManifest, EXPECTED_REVISION};
use super::BrowserManager;

// Keep each Conduit build tied to the Chromium bundle released beside it.
// Using GitHub's mutable `latest` URL would strand older clients as soon as a
// later release publishes a different required browser revision.
const RELEASE_TAG_OVERRIDE: Option<&str> = option_env!("CONDUIT_RELEASE_TAG");
const OWNER_MARKER: &str = ".conduit-browser-runtime";
const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 3 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 100_000;
const CHECKSUM_ERROR: &str = "the Chromium download failed its SHA-256 integrity check";

pub(crate) async fn install(manager: Arc<BrowserManager>) -> Result<(), String> {
    let cancellation = Arc::new(AtomicBool::new(false));
    manager.begin_install(cancellation.clone())?;
    let worker = manager.clone();
    let result = tokio::task::spawn_blocking(move || install_blocking(&worker, &cancellation))
        .await
        .map_err(|e| format!("browser installer failed to run: {e}"))?;
    manager.finish_install(result.clone());
    result
}

pub(crate) async fn refresh_manifest(manager: Arc<BrowserManager>) -> Result<(), String> {
    let manifest = tokio::task::spawn_blocking(fetch_manifest)
        .await
        .map_err(|e| format!("browser manifest check failed to run: {e}"))??;
    manager.set_install_metadata(manifest.size);
    Ok(())
}

/// Remove only Conduit's managed runtime. Profiles live beside this folder and
/// retain their cookies and history for a later reinstall.
pub(crate) fn remove_runtime(root: &Path) -> Result<(), String> {
    match fs::symlink_metadata(root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("could not inspect Chromium runtime: {error}")),
        Ok(metadata) if !metadata.is_dir() || metadata.file_type().is_symlink() => {
            return Err("refusing to remove a redirected runtime folder".into());
        }
        Ok(_) => {}
    }
    let resolved_root = root
        .canonicalize()
        .map_err(|e| format!("could not resolve Chromium runtime: {e}"))?;
    let mut bundles = Vec::new();
    let mut cache = None;
    for entry in fs::read_dir(root).map_err(|e| format!("could not list Chromium runtime: {e}"))? {
        let entry = entry.map_err(|e| format!("could not inspect Chromium runtime: {e}"))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let path = entry.path();
        if name == "active" || name == "previous" || name.starts_with("staging-") {
            require_owned_directory(root, &path)?;
            bundles.push(path);
        } else if name == "downloads" {
            let metadata = fs::symlink_metadata(&path)
                .map_err(|e| format!("could not inspect Chromium cache: {e}"))?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err("refusing to remove a redirected Chromium cache".into());
            }
            let resolved_cache = path
                .canonicalize()
                .map_err(|e| format!("could not resolve Chromium cache: {e}"))?;
            if resolved_cache.parent() != Some(resolved_root.as_path()) {
                return Err("refusing to remove Chromium cache outside the runtime".into());
            }
            cache = Some(path);
        }
    }
    if bundles.is_empty() && cache.is_some() {
        return Err("refusing to remove an unowned Chromium cache".into());
    }
    for path in bundles {
        remove_owned_directory(root, &path)?;
    }
    if let Some(path) = cache {
        fs::remove_dir_all(&path)
            .map_err(|e| format!("could not remove Chromium cache: {e}"))?;
    }
    Ok(())
}

fn install_blocking(manager: &BrowserManager, cancelled: &AtomicBool) -> Result<(), String> {
    fs::create_dir_all(manager.runtime_root())
        .map_err(|e| format!("could not create runtime folder: {e}"))?;
    let client = download_client()?;
    let manifest = fetch_manifest_with(&client)?;
    manager.set_install_progress(
        BrowserInstallStatus::Downloading,
        0,
        Some(manifest.size),
        None,
    );

    let download_dir = manager.runtime_root().join("downloads");
    fs::create_dir_all(&download_dir)
        .map_err(|e| format!("could not create download folder: {e}"))?;
    let part = download_dir.join(format!("{}.zip.part", manifest.revision));
    download_archive(&client, &manifest, &part, cancelled, |downloaded| {
        manager.set_install_progress(
            BrowserInstallStatus::Downloading,
            downloaded,
            Some(manifest.size),
            None,
        );
    })?;
    ensure_not_cancelled(cancelled)?;

    manager.set_install_progress(
        BrowserInstallStatus::Verifying,
        manifest.size,
        Some(manifest.size),
        None,
    );
    verify_download(&part, &manifest.sha256, cancelled)?;
    ensure_not_cancelled(cancelled)?;

    manager.set_install_progress(
        BrowserInstallStatus::Installing,
        manifest.size,
        Some(manifest.size),
        None,
    );
    let staging = manager
        .runtime_root()
        .join(format!("staging-{}", crate::random::uuid_v4()));
    fs::create_dir_all(&staging).map_err(|e| format!("could not create staging folder: {e}"))?;
    fs::write(staging.join(OWNER_MARKER), EXPECTED_REVISION)
        .map_err(|e| format!("could not mark the staging folder: {e}"))?;
    let result: Result<(), String> = (|| {
        extract_zip(&part, &staging, cancelled)?;
        validate_runtime_paths(&staging, &manifest)?;
        fs::write(
            staging.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("could not save the installed manifest: {e}"))?;
        set_executable(&staging.join(&manifest.sidecar_path))?;
        set_executable(&staging.join(&manifest.chromium_path))?;
        run_self_test(&staging, &manifest, cancelled)?;
        ensure_not_cancelled(cancelled)?;
        promote(manager.runtime_root(), &staging)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = remove_owned_directory(manager.runtime_root(), &staging);
    }
    result?;
    let _ = fs::remove_file(part);
    Ok(())
}

fn download_client() -> Result<Client, String> {
    Client::builder()
        .user_agent(concat!("conduit/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("could not prepare the browser download: {e}"))
}

fn fetch_manifest() -> Result<RuntimeManifest, String> {
    fetch_manifest_with(&download_client()?)
}

fn fetch_manifest_with(client: &Client) -> Result<RuntimeManifest, String> {
    let target = release_target()?;
    let manifest_url = format!("{}/conduit-browser-{target}.json", release_root());
    let manifest: RuntimeManifest = client
        .get(&manifest_url)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|e| format!("could not fetch the Chromium manifest: {e}"))?
        .json()
        .map_err(|e| format!("the Chromium manifest is invalid: {e}"))?;
    validate_manifest(&manifest, target)?;
    Ok(manifest)
}

fn validate_manifest(manifest: &RuntimeManifest, target: &str) -> Result<(), String> {
    if manifest.revision != EXPECTED_REVISION {
        return Err(format!(
            "the release provides revision {}, but this Conduit build requires {EXPECTED_REVISION}",
            manifest.revision
        ));
    }
    if manifest.target != target {
        return Err(format!(
            "the Chromium bundle is for {}, not {target}",
            manifest.target
        ));
    }
    if manifest.size == 0
        || manifest.size > MAX_ARCHIVE_BYTES
        || manifest.sha256.len() != 64
        || !manifest.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("the Chromium manifest has an invalid size or checksum".into());
    }
    for relative in [&manifest.sidecar_path, &manifest.chromium_path] {
        if !safe_bundle_path(relative) {
            return Err("the Chromium manifest contains an unsafe executable path".into());
        }
    }
    let url =
        Url::parse(&manifest.archive_url).map_err(|_| "the Chromium archive URL is invalid")?;
    let expected_archive = format!("{}/conduit-browser-{target}.zip", release_root());
    if url.as_str() != expected_archive {
        return Err("the Chromium archive URL does not match this Conduit release".into());
    }
    Ok(())
}

fn release_tag() -> &'static str {
    RELEASE_TAG_OVERRIDE
        .filter(|tag| !tag.is_empty())
        .unwrap_or(concat!("v", env!("CARGO_PKG_VERSION")))
}

fn release_root() -> String {
    format!(
        "https://github.com/oneauraaa/conduit/releases/download/{}",
        release_tag()
    )
}

pub(crate) fn safe_bundle_path(relative: &str) -> bool {
    let path = Path::new(relative);
    !path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
}

fn download_archive(
    client: &Client,
    manifest: &RuntimeManifest,
    path: &Path,
    cancelled: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<(), String> {
    let existing_length = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if existing_length == manifest.size {
        progress(manifest.size);
        return Ok(());
    }
    let existing = if existing_length < manifest.size {
        existing_length
    } else {
        0
    };
    let mut request = client.get(&manifest.archive_url);
    if existing > 0 {
        request = request.header(RANGE, format!("bytes={existing}-"));
    }
    let mut response = request
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|e| format!("could not download Chromium: {e}"))?;
    let resumed = existing > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .read(true)
        .open(path)
        .map_err(|e| format!("could not open the Chromium download: {e}"))?;
    let mut downloaded = if resumed { existing } else { 0 };
    if resumed {
        file.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    } else {
        file.set_len(0).map_err(|e| e.to_string())?;
    }
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        ensure_not_cancelled(cancelled)?;
        let read = response
            .read(&mut buffer)
            .map_err(|e| format!("Chromium download failed: {e}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|e| format!("could not save Chromium: {e}"))?;
        downloaded = downloaded.saturating_add(read as u64);
        progress(downloaded);
    }
    if downloaded != manifest.size {
        return Err(format!(
            "Chromium download ended at {downloaded} bytes; expected {}",
            manifest.size
        ));
    }
    file.sync_all()
        .map_err(|e| format!("could not flush the Chromium download: {e}"))
}

fn verify_sha256(path: &Path, expected: &str, cancelled: &AtomicBool) -> Result<(), String> {
    let mut file = File::open(path).map_err(|e| format!("could not verify Chromium: {e}"))?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        ensure_not_cancelled(cancelled)?;
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("could not verify Chromium: {e}"))?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hash.finalize());
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(CHECKSUM_ERROR.into());
    }
    Ok(())
}

fn verify_download(path: &Path, expected: &str, cancelled: &AtomicBool) -> Result<(), String> {
    let result = verify_sha256(path, expected, cancelled);
    if result.as_ref().is_err_and(|error| error == CHECKSUM_ERROR) {
        // A complete-but-corrupt .part would otherwise be treated as already
        // downloaded on every retry, permanently trapping the user on the
        // checksum error. Interrupted/cancelled transfers remain resumable.
        let _ = fs::remove_file(path);
    }
    result
}

fn extract_zip(archive: &Path, destination: &Path, cancelled: &AtomicBool) -> Result<(), String> {
    let file = File::open(archive).map_err(|e| format!("could not open Chromium archive: {e}"))?;
    let mut zip = ZipArchive::new(file).map_err(|e| format!("Chromium archive is invalid: {e}"))?;
    if zip.len() > MAX_ARCHIVE_ENTRIES {
        return Err("Chromium archive contains too many files".into());
    }
    let mut extracted_bytes = 0_u64;
    for index in 0..zip.len() {
        ensure_not_cancelled(cancelled)?;
        let mut entry = zip
            .by_index(index)
            .map_err(|e| format!("could not read Chromium archive: {e}"))?;
        let relative = entry
            .enclosed_name()
            .ok_or_else(|| "Chromium archive contains a path outside its destination".to_string())?
            .to_path_buf();
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("Chromium archive contains a symbolic link".into());
        }
        extracted_bytes = extracted_bytes
            .checked_add(entry.size())
            .ok_or("Chromium archive expanded size overflowed")?;
        if extracted_bytes > MAX_EXTRACTED_BYTES {
            return Err("Chromium archive expands beyond the allowed size".into());
        }
        let output = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&output).map_err(|e| format!("could not extract Chromium: {e}"))?;
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("could not extract Chromium: {e}"))?;
        }
        let mut target =
            File::create(&output).map_err(|e| format!("could not extract Chromium: {e}"))?;
        std::io::copy(&mut entry, &mut target)
            .map_err(|e| format!("could not extract Chromium: {e}"))?;
        apply_archive_permissions(&output, entry.unix_mode())?;
    }
    Ok(())
}

#[cfg(unix)]
fn apply_archive_permissions(path: &Path, mode: Option<u32>) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let Some(mode) = mode else { return Ok(()) };
    // Preserve ordinary rwx bits for Chromium's helper executables, but never
    // accept setuid/setgid/sticky bits from a downloaded archive.
    fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o777)).map_err(|e| {
        format!(
            "could not restore archive permissions for {}: {e}",
            path.display()
        )
    })
}

#[cfg(not(unix))]
fn apply_archive_permissions(_path: &Path, _mode: Option<u32>) -> Result<(), String> {
    Ok(())
}

fn validate_runtime_paths(root: &Path, manifest: &RuntimeManifest) -> Result<(), String> {
    for relative in [&manifest.sidecar_path, &manifest.chromium_path] {
        let path = root.join(relative);
        if !path.is_file() {
            return Err(format!("Chromium bundle is missing {relative}"));
        }
    }
    Ok(())
}

fn run_self_test(
    root: &Path,
    manifest: &RuntimeManifest,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    let mut child = Command::new(root.join(&manifest.sidecar_path))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start the Chromium self-test: {e}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or("the Chromium self-test has no input pipe")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("the Chromium self-test has no output pipe")?;
    let handshake = serde_json::json!({
        "id": "install-handshake",
        "type": "handshake",
        "protocol": 1,
        "revision": manifest.revision,
    });
    let test = serde_json::json!({
        "id": "install-self-test",
        "type": "selfTest",
        "chromium": root.join(&manifest.chromium_path),
    });
    writeln!(stdin, "{handshake}")
        .and_then(|_| writeln!(stdin, "{test}"))
        .map_err(|e| format!("could not request the Chromium self-test: {e}"))?;
    stdin
        .flush()
        .map_err(|e| format!("could not send the Chromium self-test: {e}"))?;

    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
        {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if cancelled.load(Ordering::Relaxed) {
            terminate_child(&mut child);
            return Err("Chromium download cancelled".into());
        }
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => {
                let Ok(response) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                if response.get("id").and_then(serde_json::Value::as_str)
                    == Some("install-self-test")
                {
                    terminate_child(&mut child);
                    if response.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
                        return Ok(());
                    }
                    let error = response
                        .get("error")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("the Chromium sidecar rejected its self-test");
                    return Err(format!("Chromium self-test failed: {error}"));
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                let status = child
                    .wait()
                    .map_err(|e| format!("Chromium self-test failed: {e}"))?;
                return Err(format!("Chromium self-test exited with {status}"));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        if Instant::now() >= deadline {
            terminate_child(&mut child);
            return Err("Chromium self-test timed out".into());
        }
    }
}

fn terminate_child(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn promote(runtime_root: &Path, staging: &Path) -> Result<(), String> {
    let active = runtime_root.join("active");
    let previous = runtime_root.join("previous");
    if previous.exists() {
        remove_owned_directory(runtime_root, &previous)?;
    }
    if active.exists() {
        require_owned_directory(runtime_root, &active)?;
        fs::rename(&active, &previous)
            .map_err(|e| format!("could not preserve the previous Chromium runtime: {e}"))?;
    }
    if let Err(error) = fs::rename(staging, &active) {
        if previous.exists() {
            let _ = fs::rename(&previous, &active);
        }
        return Err(format!("could not activate the Chromium runtime: {error}"));
    }
    Ok(())
}

fn require_owned_directory(root: &Path, target: &Path) -> Result<(), String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("could not resolve runtime root: {e}"))?;
    let target = target
        .canonicalize()
        .map_err(|e| format!("could not resolve runtime folder: {e}"))?;
    if target.parent() != Some(root.as_path()) || !target.join(OWNER_MARKER).is_file() {
        return Err("refusing to modify an unowned runtime directory".into());
    }
    Ok(())
}

pub(crate) fn remove_owned_directory(root: &Path, target: &Path) -> Result<(), String> {
    require_owned_directory(root, target)?;
    fs::remove_dir_all(target)
        .map_err(|e| format!("could not remove the old Chromium runtime: {e}"))
}

fn ensure_not_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Relaxed) {
        Err("Chromium download cancelled".into())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path).map_err(|e| e.to_string())?.permissions();
    permissions.set_mode(permissions.mode() | 0o700);
    fs::set_permissions(path, permissions)
        .map_err(|e| format!("could not make {} executable: {e}", path.display()))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

pub(crate) fn release_target() -> Result<&'static str, String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Ok("windows-x64"),
        ("linux", "x86_64") => Ok("linux-x64"),
        ("macos", "x86_64") => Ok("macos-x64"),
        ("macos", "aarch64") => Ok("macos-arm64"),
        (os, arch) => Err(format!("Chromium is not published for {os}/{arch}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    fn manifest() -> RuntimeManifest {
        RuntimeManifest {
            revision: EXPECTED_REVISION.into(),
            target: release_target().unwrap().into(),
            archive_url: format!(
                "{}/conduit-browser-{}.zip",
                release_root(),
                release_target().unwrap()
            ),
            size: 42,
            sha256: "0".repeat(64),
            sidecar_path: "bin/conduit-browser".into(),
            chromium_path: "chromium/chrome".into(),
        }
    }

    #[test]
    fn browser_manifest_is_pinned_to_this_conduit_release() {
        assert!(release_root().ends_with(release_tag()));
        assert!(!release_root().contains("/latest/"));
        assert!(release_tag().starts_with('v'));
    }

    #[test]
    fn manifest_rejects_paths_and_non_github_downloads() {
        assert!(validate_manifest(&manifest(), release_target().unwrap()).is_ok());
        let mut bad = manifest();
        bad.sidecar_path = "../escape".into();
        assert!(validate_manifest(&bad, release_target().unwrap()).is_err());
        let mut bad = manifest();
        bad.target = "wrong-platform".into();
        assert!(validate_manifest(&bad, release_target().unwrap()).is_err());
        let mut bad = manifest();
        bad.archive_url = "https://example.com/browser.zip".into();
        assert!(validate_manifest(&bad, release_target().unwrap()).is_err());
        let mut bad = manifest();
        bad.size = MAX_ARCHIVE_BYTES + 1;
        assert!(validate_manifest(&bad, release_target().unwrap()).is_err());
        let mut bad = manifest();
        bad.sha256 = "z".repeat(64);
        assert!(validate_manifest(&bad, release_target().unwrap()).is_err());
    }

    #[test]
    fn safe_extraction_rejects_traversal() {
        let root = std::env::temp_dir().join(format!("conduit-extract-{}", crate::random::uuid_v4()));
        fs::create_dir_all(&root).unwrap();
        let archive = root.join("runtime.zip");
        let file = File::create(&archive).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file("../escape", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"nope").unwrap();
        writer.finish().unwrap();
        let destination = root.join("destination");
        fs::create_dir_all(&destination).unwrap();
        assert!(extract_zip(&archive, &destination, &AtomicBool::new(false)).is_err());
        assert!(!root.join("escape").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn extraction_preserves_helper_executable_bits() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!("conduit-mode-{}", crate::random::uuid_v4()));
        fs::create_dir_all(&root).unwrap();
        let archive = root.join("runtime.zip");
        let file = File::create(&archive).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file(
                "chromium/helper",
                SimpleFileOptions::default().unix_permissions(0o755),
            )
            .unwrap();
        writer.write_all(b"helper").unwrap();
        writer.finish().unwrap();
        let destination = root.join("destination");
        fs::create_dir_all(&destination).unwrap();
        extract_zip(&archive, &destination, &AtomicBool::new(false)).unwrap();
        let mode = fs::metadata(destination.join("chromium/helper"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn promotion_retains_the_previous_working_runtime() {
        let root = std::env::temp_dir().join(format!("conduit-promote-{}", crate::random::uuid_v4()));
        let active = root.join("active");
        let staging = root.join("staging-new");
        for (directory, value) in [(&active, "old"), (&staging, "new")] {
            fs::create_dir_all(directory).unwrap();
            fs::write(directory.join(OWNER_MARKER), "owned").unwrap();
            fs::write(directory.join("version"), value).unwrap();
        }
        promote(&root, &staging).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("active/version")).unwrap(),
            "new"
        );
        assert_eq!(
            fs::read_to_string(root.join("previous/version")).unwrap(),
            "old"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cancellation_is_fail_closed() {
        assert!(ensure_not_cancelled(&AtomicBool::new(true)).is_err());
        assert!(ensure_not_cancelled(&AtomicBool::new(false)).is_ok());
    }

    #[test]
    fn checksum_failure_discards_the_corrupt_partial_for_retry() {
        let root = std::env::temp_dir().join(format!("conduit-hash-{}", crate::random::uuid_v4()));
        fs::create_dir_all(&root).unwrap();
        let part = root.join("runtime.zip.part");
        fs::write(&part, b"corrupt archive").unwrap();
        let error = verify_download(&part, &"0".repeat(64), &AtomicBool::new(false)).unwrap_err();
        assert_eq!(error, CHECKSUM_ERROR);
        assert!(!part.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn interrupted_download_resumes_from_the_existing_byte() {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0_u8; 2048];
            let read = socket.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(
                request.to_ascii_lowercase().contains("range: bytes=6-"),
                "{request}"
            );
            socket
                .write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 5\r\nConnection: close\r\n\r\nworld")
                .unwrap();
        });

        let root = std::env::temp_dir().join(format!("conduit-resume-{}", crate::random::uuid_v4()));
        fs::create_dir_all(&root).unwrap();
        let part = root.join("bundle.part");
        fs::write(&part, b"hello ").unwrap();
        let mut manifest = manifest();
        manifest.archive_url = format!("http://{address}/bundle.zip");
        manifest.size = 11;
        let mut progress = Vec::new();
        download_archive(
            &Client::new(),
            &manifest,
            &part,
            &AtomicBool::new(false),
            |bytes| progress.push(bytes),
        )
        .unwrap();
        server.join().unwrap();
        assert_eq!(fs::read(&part).unwrap(), b"hello world");
        assert_eq!(progress.last(), Some(&11));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn deletion_requires_a_direct_child_with_our_marker() {
        let root = std::env::temp_dir().join(format!("conduit-runtime-{}", crate::random::uuid_v4()));
        let owned = root.join("previous");
        fs::create_dir_all(&owned).unwrap();
        assert!(remove_owned_directory(&root, &owned).is_err());
        fs::write(owned.join(OWNER_MARKER), "owned").unwrap();
        assert!(remove_owned_directory(&root, &owned).is_ok());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn uninstall_removes_runtime_but_keeps_sibling_profiles() {
        let root = std::env::temp_dir().join(format!("conduit-uninstall-{}", crate::random::uuid_v4()));
        let runtime = root.join("runtime");
        let active = runtime.join("active");
        let profiles = root.join("profiles");
        fs::create_dir_all(&active).unwrap();
        fs::create_dir_all(runtime.join("downloads")).unwrap();
        fs::create_dir_all(&profiles).unwrap();
        fs::write(active.join(OWNER_MARKER), "owned").unwrap();
        fs::write(active.join("chrome"), "binary").unwrap();
        fs::write(runtime.join("downloads/archive.zip.part"), "cached").unwrap();
        fs::write(profiles.join("cookies"), "saved").unwrap();

        remove_runtime(&runtime).unwrap();
        assert!(!active.exists());
        assert!(!runtime.join("downloads").exists());
        assert_eq!(fs::read_to_string(profiles.join("cookies")).unwrap(), "saved");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn uninstall_refuses_unowned_runtime_directory() {
        let root = std::env::temp_dir().join(format!("conduit-unowned-{}", crate::random::uuid_v4()));
        fs::create_dir_all(root.join("active")).unwrap();
        fs::write(root.join("active/chrome"), "unknown").unwrap();
        assert!(remove_runtime(&root).is_err());
        assert!(root.join("active/chrome").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn uninstall_refuses_redirected_cache_before_removing_chromium() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!("conduit-cache-{}", crate::random::uuid_v4()));
        let outside = std::env::temp_dir().join(format!("conduit-outside-{}", crate::random::uuid_v4()));
        fs::create_dir_all(root.join("active")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(root.join("active").join(OWNER_MARKER), "owned").unwrap();
        fs::write(outside.join("keep"), "private").unwrap();
        symlink(&outside, root.join("downloads")).unwrap();

        assert!(remove_runtime(&root).is_err());
        assert!(root.join("active").exists());
        assert_eq!(fs::read_to_string(outside.join("keep")).unwrap(), "private");
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }
}
