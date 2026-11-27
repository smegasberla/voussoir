//! Putting a verified artifact on disk.
//!
//! Everything that touches the filesystem, and every platform difference in
//! replacing a running binary, lives here. Deciding *whether* to install, and
//! what to tell the page, stays in the command layer above it.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::Error;

use super::fetch::fetch;
use super::manifest::{parse_manifest, require_trusted_url, verify_sha256, verify_signature};
use super::{is_newer, UpdateConfig};

// ---------------------------------------------------------------------------
// Applying an update (desktop only)
// ---------------------------------------------------------------------------

/// How long one artifact download may take, from connect to the last byte.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const DOWNLOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// The largest artifact accepted. The shell is one binary, so 256 MiB is far
/// past any real build and small enough that a wrong URL cannot fill a disk.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const ARTIFACT_CAP: u64 = 256 * 1024 * 1024;

/// Download `url` with a deadline and a cap.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(super) fn download(
    url: &str,
    on_progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<Vec<u8>, Error> {
    download_limited(url, DOWNLOAD_TIMEOUT, ARTIFACT_CAP, on_progress)
}

/// [`download`] with the bounds as parameters, so a test can use a short
/// deadline and a tiny cap against a local socket.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn download_limited(
    url: &str,
    timeout: std::time::Duration,
    cap: u64,
    on_progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<Vec<u8>, Error> {
    use std::io::Read;

    require_trusted_url(url)?;
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .timeout_connect(Some(timeout))
        .build()
        .new_agent();
    let mut response = agent
        .get(url)
        .call()
        .map_err(|error| Error::Handler(format!("update download failed: {error}")))?;
    let total = response.body().content_length();
    let mut reader = response.body_mut().as_reader();
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut chunk)
            .map_err(|error| Error::Handler(format!("update download failed: {error}")))?;
        if read == 0 {
            break;
        }
        if bytes.len() as u64 + read as u64 > cap {
            return Err(Error::Handler(format!(
                "update download failed: artifact exceeds {cap} bytes"
            )));
        }
        bytes.extend_from_slice(&chunk[..read]);
        on_progress(bytes.len() as u64, total);
    }
    // A final event with a known total, so a page that only listens for the end
    // still sees one even when the server sent no Content-Length.
    on_progress(bytes.len() as u64, Some(bytes.len() as u64));
    Ok(bytes)
}

/// Rate-limit download progress so a fast local server cannot flood the page.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(super) struct ProgressThrottle {
    last: Option<std::time::Instant>,
    interval: std::time::Duration,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
impl ProgressThrottle {
    pub(super) fn new(interval: std::time::Duration) -> Self {
        Self {
            last: None,
            interval,
        }
    }

    /// Whether this update should be sent: the first one, a finished one, or one
    /// that is at least `interval` after the previous send.
    pub(super) fn ready(&mut self, done: bool) -> bool {
        let now = std::time::Instant::now();
        let due = self
            .last
            .is_none_or(|last| now.duration_since(last) >= self.interval);
        if done || due {
            self.last = Some(now);
            return true;
        }
        false
    }
}

/// `target` with `suffix` appended to its file name.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn sibling(target: &Path, suffix: &str) -> PathBuf {
    let mut name = target
        .file_name()
        .map(|name| name.to_owned())
        .unwrap_or_else(|| "voussoir".into());
    name.push(suffix);
    target.with_file_name(name)
}

/// Write `bytes` to `path` and flush them to the device before returning.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    use std::io::Write;
    let mut file = std::fs::File::create(path)
        .map_err(|error| Error::Handler(format!("could not create the new build: {error}")))?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        let _ = std::fs::remove_file(path);
        return Err(Error::Handler(format!(
            "could not write the new build: {error}"
        )));
    }
    Ok(())
}

/// Install `bytes` and leave behind what a failed launch would need to undo it.
///
/// The old binary is copied aside *before* the rename, so the copy is the build
/// that was running. The marker then names the version being installed, which is
/// what `rollback::guard` counts launches against. A failure to keep the copy is
/// not fatal: the update installs, it simply cannot be undone. The marker is
/// written only when the copy exists, so no launch can loop on a restore that
/// could never succeed.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn install_with_rollback(bytes: &[u8], target: &Path, version: &str) -> Result<(), Error> {
    let kept = crate::rollback::keep_previous(target).is_ok();
    install_bytes(bytes, target)?;
    if kept {
        if let Err(error) = crate::rollback::record_pending(target, version) {
            eprintln!("voussoir: could not record the pending update: {error}");
        }
    }
    Ok(())
}

/// Replace `target` with `bytes` without ever leaving it half-written.
///
/// The new file is written beside the old one and renamed into place, because a
/// rename is atomic within a filesystem: a crash or a full disk leaves the old
/// binary runnable. A running executable can be replaced on Unix. Windows
/// refuses that but allows *renaming* it, so the old file steps aside as `.old`
/// and is restored if the second rename fails; the `.old` file is removed on the
/// next install, once the process that ran it has exited.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn install_bytes(bytes: &[u8], target: &Path) -> Result<(), Error> {
    let parent = target.parent().ok_or_else(|| {
        Error::Handler(format!(
            "cannot install next to `{}`: it has no parent directory",
            target.display()
        ))
    })?;
    let temp = sibling(target, &format!(".{}.tmp", std::process::id()));
    let backup = sibling(target, ".old");

    write_new_file(&temp, bytes)?;
    // A leftover backup is a previous update whose process has since exited.
    let _ = std::fs::remove_file(&backup);

    let replaced = install_platform(&temp, target, &backup, parent);
    if replaced.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    replaced
}

#[cfg(all(unix, not(any(target_os = "android", target_os = "ios"))))]
fn install_platform(
    temp: &Path,
    target: &Path,
    _backup: &Path,
    _parent: &Path,
) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt;
    // Keep the mode the running binary had, so an installed copy stays
    // executable even when the umask would not have made it so.
    let mode = std::fs::metadata(target)
        .map(|metadata| metadata.permissions().mode())
        .unwrap_or(0o755);
    std::fs::set_permissions(temp, std::fs::Permissions::from_mode(mode))
        .map_err(|error| Error::Handler(format!("could not set the build executable: {error}")))?;
    std::fs::rename(temp, target)
        .map_err(|error| Error::Handler(format!("could not install the update: {error}")))
}

/// Install by renaming the old binary aside first, then the new one into place,
/// and putting the old one back if that second rename fails.
///
/// Windows needs this because it refuses to replace a running executable while
/// allowing one to be renamed. The logic is not Windows-specific, so it lives
/// here instead of inside a `cfg(windows)` function: the branch that only runs on
/// Windows could never be exercised on the machines the tests run on, and an
/// untested rollback path is the one that loses a user's working binary. The
/// Windows `install_platform` is now a call into this.
// Called by the Windows installer and by tests that run on every platform, so on a
// non-Windows build without tests it has no caller. That is a fact about cfg.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn install_by_renaming_aside(temp: &Path, target: &Path, backup: &Path) -> Result<(), Error> {
    let stepped_aside = std::fs::rename(target, backup).is_ok();
    if let Err(error) = std::fs::rename(temp, target) {
        if stepped_aside {
            let _ = std::fs::rename(backup, target);
        }
        return Err(Error::Handler(format!(
            "could not install the update: {error}"
        )));
    }
    // The replaced binary is this process's own executable on Windows, so it
    // may stay locked until exit; a leftover is swept on the next install.
    let _ = std::fs::remove_file(backup);
    Ok(())
}

#[cfg(all(windows, not(any(target_os = "android", target_os = "ios"))))]
fn install_platform(
    temp: &Path,
    target: &Path,
    backup: &Path,
    _parent: &Path,
) -> Result<(), Error> {
    install_by_renaming_aside(temp, target, backup)
}

#[cfg(not(any(unix, windows, target_os = "android", target_os = "ios")))]
fn install_platform(
    _temp: &Path,
    _target: &Path,
    _backup: &Path,
    _parent: &Path,
) -> Result<(), Error> {
    Err(Error::Handler(
        "self-update is not supported on this platform; install it through the OS".to_owned(),
    ))
}

/// Download, install and config-version rules that need a disk and a socket.
#[cfg(all(test, not(any(target_os = "android", target_os = "ios"))))]
mod install_tests {
    use super::super::manifest::{platform_key, sha256_hex};
    use super::*;
    use crate::updater::{relaunch_command, spawn_and_exit};
    use base64::Engine;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener};
    use std::time::Duration;

    /// A deterministic keypair, so the tests do not depend on a random source.
    fn test_key() -> (SigningKey, String) {
        let signing = SigningKey::from_bytes(&[42u8; 32]);
        let public =
            base64::engine::general_purpose::STANDARD.encode(signing.verifying_key().as_bytes());
        (signing, public)
    }

    fn signature(signing: &SigningKey, bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(signing.sign(bytes).to_bytes())
    }

    /// Serve `routes` in order, one connection each, checking the request path.
    fn serve_with(build: impl FnOnce(SocketAddr) -> Vec<(String, Vec<u8>)>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let routes = build(addr);
        std::thread::spawn(move || {
            for (path, body) in routes {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut request = [0u8; 2048];
                let read = stream.read(&mut request).unwrap_or(0);
                let request = String::from_utf8_lossy(&request[..read]).to_string();
                assert!(
                    request.starts_with(&format!("GET {path}")),
                    "unexpected request: {request}"
                );
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
        addr
    }

    fn serve(routes: Vec<(String, Vec<u8>)>) -> SocketAddr {
        serve_with(|_| routes)
    }

    fn temp_target(name: &str, contents: &[u8]) -> (PathBuf, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("voussoir-update-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("app");
        let mut file = std::fs::File::create(&target).unwrap();
        file.write_all(contents).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        (dir, target)
    }

    fn feed(signing: &SigningKey, addr: SocketAddr, version: &str, artifact: &[u8]) -> Vec<u8> {
        json!({
            "version": version,
            "notes": "notes",
            "url": format!("http://{addr}/app"),
            "sha256": sha256_hex(artifact),
            "signature": signature(signing, artifact),
        })
        .to_string()
        .into_bytes()
    }

    fn update_for(addr: SocketAddr, public: &str) -> UpdateConfig {
        UpdateConfig {
            url: Some(format!("http://{addr}/feed")),
            public_key: Some(public.to_owned()),
            current: "1.0.0".to_owned(),
        }
    }

    #[test]
    fn a_download_is_bounded() {
        let addr = serve(vec![("/big".to_owned(), vec![b'x'; 4096])]);
        let error = download_limited(
            &format!("http://{addr}/big"),
            Duration::from_secs(5),
            64,
            &mut |_, _| {},
        )
        .unwrap_err();
        assert!(error.to_string().contains("download failed"), "{error}");
    }

    #[test]
    fn a_download_returns_exactly_the_bytes_served() {
        let addr = serve(vec![("/app".to_owned(), b"binary-ish".to_vec())]);
        let bytes = download_limited(
            &format!("http://{addr}/app"),
            Duration::from_secs(5),
            1024,
            &mut |_, _| {},
        )
        .unwrap();
        assert_eq!(bytes, b"binary-ish");
    }

    #[test]
    fn installing_replaces_the_binary_and_keeps_it_executable() {
        let (dir, target) = temp_target("replace", b"old");
        install_bytes(b"new", &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(
                std::fs::metadata(&target).unwrap().permissions().mode() & 0o111,
                0,
                "the replacement must stay executable"
            );
        }
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_second_install_clears_the_previous_backup() {
        let (dir, target) = temp_target("twice", b"old");
        install_bytes(b"one", &target).unwrap();
        let backup = sibling(&target, ".old");
        std::fs::write(&backup, b"stale").unwrap();
        install_bytes(b"two", &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"two");
        assert!(!backup.exists(), "the stale backup is swept up");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn applying_installs_what_the_feed_describes() {
        let (signing, public) = test_key();
        let artifact = b"new-binary-contents".to_vec();
        let addr = serve_with(|addr| {
            vec![
                ("/feed".to_owned(), feed(&signing, addr, "9.9.9", &artifact)),
                ("/app".to_owned(), artifact.clone()),
            ]
        });
        let (dir, target) = temp_target("apply", b"old");
        let result = apply_to(&update_for(addr, &public), &target, false, &mut |_, _| {}).unwrap();
        assert_eq!(result["installed"], true, "{result}");
        assert_eq!(result["version"], "9.9.9");
        assert_eq!(std::fs::read(&target).unwrap(), artifact);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_windows_install_strategy_replaces_the_binary_and_sweeps_the_backup() {
        // Windows renames the running binary aside rather than replacing it. This
        // runs that strategy on whatever platform the tests run on, because the
        // branch guarded by `cfg(windows)` is never executed here and a rollback
        // path that only exists in an untested branch is how a user loses a
        // working binary.
        let (dir, target) = temp_target("win-replace", b"old-binary");
        let temp = sibling(&target, ".incoming");
        let backup = sibling(&target, ".old");
        std::fs::write(&temp, b"new-binary").unwrap();

        install_by_renaming_aside(&temp, &target, &backup).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"new-binary");
        assert!(!temp.exists(), "the incoming file is renamed into place");
        assert!(!backup.exists(), "the swept backup is gone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_windows_install_strategy_restores_the_old_binary_when_the_rename_fails() {
        // The failure this exists for: the old file stepped aside and the new one
        // could not be moved in. Without the restore the target is simply gone.
        let (dir, target) = temp_target("win-restore", b"old-binary");
        let backup = sibling(&target, ".old");
        // Nothing to rename into place, so the second rename fails.
        let missing = sibling(&target, ".never-written");

        let error = install_by_renaming_aside(&missing, &target, &backup).unwrap_err();

        assert!(error.to_string().contains("could not install"), "{error}");
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"old-binary",
            "the old binary must be back in place"
        );
        assert!(
            !backup.exists(),
            "the backup was renamed back, not left behind"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn applying_leaves_what_a_failed_launch_would_need() {
        // The whole install, through `apply_to`, must leave the state rollback
        // reads: the build that was running, and a marker naming the new version.
        // Testing the pieces separately proves composition, not the sequence.
        let (signing, public) = test_key();
        let artifact = b"new-binary-contents".to_vec();
        let addr = serve_with(|addr| {
            vec![
                ("/feed".to_owned(), feed(&signing, addr, "9.9.9", &artifact)),
                ("/app".to_owned(), artifact.clone()),
            ]
        });
        let (dir, target) = temp_target("apply-rollback", b"old-binary");
        apply_to(&update_for(addr, &public), &target, false, &mut |_, _| {}).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), artifact);
        // The copy is the build that was running, taken before the rename.
        assert_eq!(
            std::fs::read(sibling(&target, ".voussoir-previous")).unwrap(),
            b"old-binary"
        );
        let marker = std::fs::read_to_string(sibling(&target, ".voussoir-pending"))
            .expect("the pending marker should exist after an install");
        assert!(
            marker.contains("9.9.9"),
            "the marker names the new version: {marker:?}"
        );

        // First launch counts and proceeds; a second without confirmation, as a
        // build that died would leave it, reverts to the copy.
        assert_eq!(
            crate::rollback::guard(&target),
            crate::rollback::Startup::Continue
        );
        assert_eq!(std::fs::read(&target).unwrap(), artifact);
        assert!(matches!(
            crate::rollback::guard(&target),
            crate::rollback::Startup::RolledBack { .. }
        ));
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"old-binary",
            "the build that was running is back"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tampered_artifact_is_refused_and_the_old_binary_survives() {
        let (signing, public) = test_key();
        let artifact = b"served-by-the-mirror".to_vec();
        let addr = serve_with(|addr| {
            let mut feed_body: Value =
                serde_json::from_slice(&feed(&signing, addr, "9.9.9", &artifact))
                    .expect("the test feed is valid JSON");
            feed_body["sha256"] = json!(sha256_hex(b"expected-by-the-feed"));
            vec![
                ("/feed".to_owned(), feed_body.to_string().into_bytes()),
                ("/app".to_owned(), artifact.clone()),
            ]
        });
        let (dir, target) = temp_target("tampered", b"old");
        let error =
            apply_to(&update_for(addr, &public), &target, false, &mut |_, _| {}).unwrap_err();
        assert!(error.to_string().contains("failed verification"), "{error}");
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"old",
            "nothing may be installed after a failed check"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bad_signature_is_refused_even_when_the_digest_matches() {
        let (signing, public) = test_key();
        let artifact = b"a-mirror-can-keep-the-hash".to_vec();
        let addr = serve_with(|addr| {
            let mut feed_body: Value =
                serde_json::from_slice(&feed(&signing, addr, "9.9.9", &artifact))
                    .expect("the test feed is valid JSON");
            feed_body["signature"] =
                json!(base64::engine::general_purpose::STANDARD.encode([0u8; 64]));
            vec![
                ("/feed".to_owned(), feed_body.to_string().into_bytes()),
                ("/app".to_owned(), artifact.clone()),
            ]
        });
        let (dir, target) = temp_target("signature", b"old");
        let error =
            apply_to(&update_for(addr, &public), &target, false, &mut |_, _| {}).unwrap_err();
        assert!(
            error.to_string().contains("signature does not match"),
            "{error}"
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unconfigured_key_or_unsigned_feed_is_refused() {
        let (signing, public) = test_key();
        let artifact = b"artifact".to_vec();

        // A key is required before anything is fetched.
        let no_key = UpdateConfig {
            url: Some("http://127.0.0.1:9/feed".to_owned()),
            public_key: None,
            current: "1.0.0".to_owned(),
        };
        let (dir, target) = temp_target("nokey", b"old");
        let error = apply_to(&no_key, &target, false, &mut |_, _| {}).unwrap_err();
        assert!(error.to_string().contains("update.public_key"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);

        // A configured key requires a signature in the feed.
        let addr = serve_with(|addr| {
            let mut feed_body: Value =
                serde_json::from_slice(&feed(&signing, addr, "9.9.9", &artifact))
                    .expect("valid JSON");
            feed_body.as_object_mut().unwrap().remove("signature");
            vec![("/feed".to_owned(), feed_body.to_string().into_bytes())]
        });
        let (dir, target) = temp_target("nosig", b"old");
        let error =
            apply_to(&update_for(addr, &public), &target, false, &mut |_, _| {}).unwrap_err();
        assert!(error.to_string().contains("no `signature`"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_up_to_date_feed_installs_nothing() {
        let (signing, public) = test_key();
        let artifact = b"same".to_vec();
        let addr =
            serve_with(|addr| vec![("/feed".to_owned(), feed(&signing, addr, "1.0.0", &artifact))]);
        let (dir, target) = temp_target("uptodate", b"old");
        let result = apply_to(&update_for(addr, &public), &target, false, &mut |_, _| {}).unwrap();
        assert_eq!(result["installed"], false, "{result}");
        assert_eq!(result["reason"], "up to date");
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn force_reinstalls_the_same_version() {
        let (signing, public) = test_key();
        let artifact = b"same-version-new-build".to_vec();
        let addr = serve_with(|addr| {
            vec![
                ("/feed".to_owned(), feed(&signing, addr, "1.0.0", &artifact)),
                ("/app".to_owned(), artifact.clone()),
            ]
        });
        let (dir, target) = temp_target("force", b"old");
        let result = apply_to(&update_for(addr, &public), &target, true, &mut |_, _| {}).unwrap();
        assert_eq!(result["installed"], true, "{result}");
        assert_eq!(std::fs::read(&target).unwrap(), artifact);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_app_version_wins_over_the_framework_constant() {
        let config = crate::Config {
            version: Some("9.9.9".to_owned()),
            ..Default::default()
        };
        assert_eq!(UpdateConfig::from_config(&config, "0.2.0").current, "9.9.9");
        assert_eq!(
            UpdateConfig::from_config(&crate::Config::default(), "0.2.0").current,
            "0.2.0"
        );
    }

    #[test]
    fn signatures_hold_only_for_the_bytes_that_were_signed() {
        let (signing, public) = test_key();
        let signed = signature(&signing, b"release-bytes");
        assert!(verify_signature(b"release-bytes", &signed, &public).is_ok());
        let error = verify_signature(b"other-bytes", &signed, &public).unwrap_err();
        assert!(
            error.to_string().contains("signature does not match"),
            "{error}"
        );
        assert!(verify_signature(b"x", &signed, "not-base64").is_err());
        assert!(verify_signature(b"x", &signed, &public[..8]).is_err());
    }

    #[test]
    fn progress_is_rate_limited_but_never_loses_the_end() {
        let mut throttle = ProgressThrottle::new(Duration::from_secs(60));
        assert!(throttle.ready(false), "the first update is sent");
        assert!(!throttle.ready(false), "a burst is coalesced");
        assert!(throttle.ready(true), "the final update is always sent");
    }

    #[test]
    fn a_download_reports_monotonic_progress() {
        let body = vec![b'x'; 200_000];
        let addr = serve(vec![("/app".to_owned(), body.clone())]);
        let mut seen = Vec::new();
        let bytes = download_limited(
            &format!("http://{addr}/app"),
            Duration::from_secs(5),
            1 << 20,
            &mut |received, _total| seen.push(received),
        )
        .unwrap();
        assert_eq!(bytes.len(), body.len());
        assert!(!seen.is_empty());
        assert!(seen.windows(2).all(|pair| pair[0] <= pair[1]), "{seen:?}");
        assert_eq!(seen.last().copied(), Some(body.len() as u64));
    }

    #[test]
    fn a_patch_rebuilds_the_binary_the_feed_signed() {
        let (signing, public) = test_key();
        let old = b"AAAA-old".to_vec();
        let new = b"AAAA-new".to_vec();
        // copy "AAAA-" from the old file, then insert "new".
        let mut patch = b"VSD1".to_vec();
        patch.push(1);
        patch.extend_from_slice(&[0, 0, 0, 0, 5, 0, 0, 0]);
        patch.push(2);
        patch.extend_from_slice(&[3, 0, 0, 0]);
        patch.extend_from_slice(b"new");
        assert_eq!(crate::delta::apply(&old, &patch).unwrap(), new);

        let addr = serve_with(|addr| {
            let feed_body = json!({
                "version": "2.0.0",
                "url": format!("http://{addr}/full"),
                "sha256": sha256_hex(&new),
                "signature": signature(&signing, &new),
                "patches": {
                    platform_key(): {
                        "from": "1.0.0",
                        "url": format!("http://{addr}/patch"),
                        "sha256": sha256_hex(&patch),
                        "signature": signature(&signing, &patch),
                    }
                },
            })
            .to_string()
            .into_bytes();
            vec![
                ("/feed".to_owned(), feed_body),
                ("/patch".to_owned(), patch.clone()),
            ]
        });
        let (dir, target) = temp_target("patch", &old);
        let result = apply_to(&update_for(addr, &public), &target, false, &mut |_, _| {})
            .expect("a valid patch installs");
        assert_eq!(result["patched"], true, "{result}");
        assert_eq!(std::fs::read(&target).unwrap(), new);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_patch_that_rebuilds_the_wrong_bytes_is_refused() {
        let (signing, public) = test_key();
        let old = b"AAAA-old".to_vec();
        let expected = b"AAAA-new".to_vec();
        let mut patch = b"VSD1".to_vec();
        patch.push(2);
        patch.extend_from_slice(&[4, 0, 0, 0]);
        patch.extend_from_slice(b"evil");
        let addr = serve_with(|addr| {
            let feed_body = json!({
                "version": "2.0.0",
                "url": format!("http://{addr}/full"),
                "sha256": sha256_hex(&expected),
                "signature": signature(&signing, &expected),
                "patches": {
                    platform_key(): {
                        "from": "1.0.0",
                        "url": format!("http://{addr}/patch"),
                        "sha256": sha256_hex(&patch),
                        "signature": signature(&signing, &patch),
                    }
                },
            })
            .to_string()
            .into_bytes();
            vec![
                ("/feed".to_owned(), feed_body),
                ("/patch".to_owned(), patch.clone()),
            ]
        });
        let (dir, target) = temp_target("badpatch", &old);
        let error =
            apply_to(&update_for(addr, &public), &target, false, &mut |_, _| {}).unwrap_err();
        assert!(error.to_string().contains("failed verification"), "{error}");
        assert_eq!(std::fs::read(&target).unwrap(), old);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn relaunch_starts_the_installed_path() {
        let path = PathBuf::from("/tmp/voussoir-relaunch-test");
        let command = relaunch_command(&path);
        assert_eq!(command.get_program(), path.as_os_str());
    }

    #[test]
    fn a_refused_restart_is_reported() {
        let error = spawn_and_exit(PathBuf::from("/nonexistent/voussoir")).unwrap_err();
        assert!(error.to_string().contains("could not restart"), "{error}");
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(super) fn apply_to(
    update: &UpdateConfig,
    target: &Path,
    force: bool,
    on_progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<Value, Error> {
    let url = update.url.as_deref().ok_or_else(|| {
        Error::Handler("no update endpoint is configured in `update.url`".to_owned())
    })?;
    let public_key = update.public_key.as_deref().ok_or_else(|| {
        Error::Handler(
            "`update.public_key` is not configured, so an update cannot be verified; add the publisher's base64 Ed25519 key to voussoir.json"
                .to_owned(),
        )
    })?;
    let body = fetch(url)?;
    let release = parse_manifest(&body)?;
    let signature = release.signature.as_deref().ok_or_else(|| {
        Error::Handler(
            "update endpoint has no `signature`; refusing an artifact the configured key cannot authenticate"
                .to_owned(),
        )
    })?;
    if !force && !is_newer(&update.current, &release.version) {
        return Ok(json!({
            "installed": false,
            "reason": "up to date",
            "current": update.current,
            "latest": release.version,
        }));
    }
    // A patch is only useful from the exact version it was built against, and
    // only when the old binary is readable. Either way the reconstructed file
    // must equal what a full download would have produced.
    // Oldest-first order means the first matching hop is the furthest back the
    // publisher supports, which is the most a user can save.
    if let Some(patch) = release
        .patches
        .iter()
        .find(|hop| hop.from == update.current)
    {
        if let Ok(old) = std::fs::read(target) {
            let bytes = download(&patch.url, on_progress)?;
            verify_sha256(&bytes, &patch.sha256)?;
            if let Some(patch_signature) = &patch.signature {
                verify_signature(&bytes, patch_signature, public_key)?;
            }
            let rebuilt = crate::delta::apply(&old, &bytes)?;
            verify_sha256(&rebuilt, &release.sha256)?;
            verify_signature(&rebuilt, signature, public_key)?;
            install_with_rollback(&rebuilt, target, &release.version)?;
            return Ok(json!({
                "installed": true,
                "patched": true,
                "version": release.version,
                "path": target.display().to_string(),
                "notes": release.notes,
            }));
        }
    }

    let bytes = download(&release.url, on_progress)?;
    verify_sha256(&bytes, &release.sha256)?;
    verify_signature(&bytes, signature, public_key)?;
    install_with_rollback(&bytes, target, &release.version)?;
    Ok(json!({
        "installed": true,
        "patched": false,
        "version": release.version,
        "path": target.display().to_string(),
        "notes": release.notes,
    }))
}
