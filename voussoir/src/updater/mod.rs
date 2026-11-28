//! Update checks, and on desktop applying them.
//!
//! `update.check` fetches a JSON document from the URL in `update.url`, compares
//! its version against the app's, and hands the result to the page.
//!
//! `update.apply` goes further: it re-fetches the same feed (never a URL the page
//! supplies), downloads the artifact the feed names, checks its SHA-256, and
//! renames it over the running binary. `update.relaunch` starts the installed
//! binary and exits, and refuses unless this process installed something.
//!
//! Nothing is silent: the digest is mandatory, plain HTTP is accepted only on
//! loopback, and the install only happens when the page asks for it. The digest
//! proves the bytes match the feed; authenticity rests on TLS to the configured
//! endpoint, which is why `https` is required.
//!
//! Expected endpoint shape (extra fields ignored):
//!
//! ```json
//! { "version": "1.2.0", "notes": "Fixes a crash on save.",
//!   "url": "https://example.com/app", "sha256": "…64 hex…" }
//! ```
//!
//! or, for one feed serving several platforms:
//!
//! ```json
//! { "version": "1.2.0", "downloads": { "linux-x86_64": { "url": "…", "sha256": "…" } } }
//! ```
//!
//! Commands: `update.check` (async), `update.apply` (async, desktop only) and
//! `update.relaunch` (desktop only). All need `update.url`.

use serde_json::{json, Value};

use crate::{Config, Error, Registry};

// `fetch` compiles everywhere: a desktop TLS client, Android's
// `HttpURLConnection`, and an honest "the App Store owns this" on iOS.
mod fetch;

// The artifact half of the updater is desktop-only work, and the store owns
// installation on mobile. Gating the *module* rather than each item inside it is
// what makes that one fact instead of thirty: every `pub fn` in these files used
// to carry its own `#[cfg]`, and the ones at the top of the file — the `use`
// statements — did not, which is how `--features full` stopped compiling for
// Android while every desktop check stayed green.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod install;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod manifest;

// Public API path, unchanged by the split: the contract test and a publisher's own
// checks call `voussoir::updater::parse_manifest_for_contract`.
//
// Gated with the manifest items it re-exports, which are gated with the functions
// that build them. An ungated `pub use` of a gated item compiles on desktop and
// fails on Android, which is how `--features full` stopped building for mobile.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub use manifest::{parse_manifest_for_contract, ParsedManifest, ParsedPatch};

// `fetch` exists on every target — a desktop TLS client, Android's
// `HttpURLConnection`, and an honest "the App Store owns this" on iOS — so the
// import is not gated. The artifact half is, because installing is desktop work.
use fetch::fetch;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use install::{apply_to, ProgressThrottle};

#[cfg(not(any(target_os = "android", target_os = "ios")))]
use std::path::{Path, PathBuf};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use std::sync::{Arc, Mutex};

/// The configured endpoint, if any.
#[derive(Debug, Clone)]
pub struct UpdateConfig {
    pub url: Option<String>,
    /// Base64 Ed25519 public key; `update.apply` refuses without it.
    pub public_key: Option<String>,
    /// The running version, compared against the endpoint's `version`.
    pub current: String,
}

impl UpdateConfig {
    /// Read `update.url` and the app's own version out of a parsed config.
    ///
    /// The app's `version` wins over the `current` passed in, which is the
    /// framework's crate version: a feed tracks the app, and comparing it against
    /// the framework's version made `available` almost never true.
    pub fn from_config(config: &Config, current: &str) -> Self {
        Self {
            url: config.update_url.clone(),
            public_key: config.update_public_key.clone(),
            current: config.version.clone().unwrap_or_else(|| current.to_owned()),
        }
    }
}

/// Register the update command. A no-op endpoint answers "not configured" rather
/// than failing, so an app can ship before its update feed exists.
pub fn register(registry: &mut Registry, config: &Config) {
    let update = UpdateConfig::from_config(config, env!("CARGO_PKG_VERSION"));

    // Downloading and replacing the running binary is desktop work: on Android
    // and iOS the store owns installation.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    install_commands(registry, update.clone());

    registry.add_async("update.check", move |_app, _payload| {
        let Some(url) = &update.url else {
            return Ok(json!({ "configured": false, "available": false }));
        };
        let body = fetch(url)?;
        let remote: Value = serde_json::from_str(&body)
            .map_err(|e| Error::Handler(format!("update endpoint returned invalid JSON: {e}")))?;
        let latest = remote
            .get("version")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Handler("update endpoint has no version".to_owned()))?;

        Ok(json!({
            "configured": true,
            "current": update.current,
            "latest": latest,
            "available": is_newer(&update.current, latest),
            "notes": remote.get("notes").cloned().unwrap_or(Value::Null),
            "url": remote.get("url").cloned().unwrap_or(Value::Null),
        }))
    });
}

/// Whether `latest` is newer than `current`, comparing dotted numeric segments.
///
/// Missing segments count as zero, so `1.2` and `1.2.0` are equal. A segment that
/// is not a number sorts below a number, which is safe: it never claims an update
/// exists when it cannot tell.
pub fn is_newer(current: &str, latest: &str) -> bool {
    let a: Vec<Option<u64>> = segments(current);
    let b: Vec<Option<u64>> = segments(latest);
    let len = a.len().max(b.len());
    for i in 0..len {
        let left = a.get(i).copied().flatten().unwrap_or(0);
        let right = b.get(i).copied().flatten().unwrap_or(0);
        if right != left {
            return right > left;
        }
    }
    false
}

fn segments(version: &str) -> Vec<Option<u64>> {
    version
        .trim()
        .trim_start_matches('v')
        .split(['.', '-', '+'])
        .map(|part| {
            let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        })
        .collect()
}

/// Register `update.apply` and `update.relaunch`.
///
/// The two are separate on purpose: installing can be interrupted (a crash, a
/// revoked permission) without the app disappearing, and restarting is a
/// decision the page can announce before it happens.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn install_commands(registry: &mut Registry, update: UpdateConfig) {
    let installed: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));

    let record = Arc::clone(&installed);
    registry.add_async("update.apply", move |app, payload| {
        let force = payload
            .get("force")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let target = std::env::current_exe().map_err(|error| {
            Error::Handler(format!(
                "cannot find the running binary to replace: {error}"
            ))
        })?;
        let mut throttle = ProgressThrottle::new(std::time::Duration::from_millis(150));
        let mut on_progress = |received: u64, total: Option<u64>| {
            let done = total.is_some_and(|total| received >= total);
            if throttle.ready(done) {
                app.emit(
                    "update-progress",
                    json!({ "received": received, "total": total }),
                );
            }
        };
        let result = apply_to(&update, &target, force, &mut on_progress)?;
        if result.get("installed").and_then(Value::as_bool) == Some(true) {
            *record
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(target);
        }
        Ok(result)
    });

    registry.add("update.relaunch", move |_app, _payload| {
        let path = installed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .ok_or_else(|| {
                Error::Handler("no update is installed; call `update.apply` first".to_owned())
            })?;
        spawn_and_exit(path)?;
        Ok(json!({ "restarting": true }))
    });
}

/// Start `path` and leave, for the startup guard in `app.rs`.
///
/// Separate from `update.relaunch` because nothing was installed in this
/// process: `path` is the build the installer kept, and this process is running
/// the one that never reached a window.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) fn spawn_and_exit_for_rollback(path: PathBuf) {
    if let Err(error) = spawn_and_exit(path) {
        // Nothing else can be done here, and exiting would leave no app at all.
        // The marker is already cleared, so the next launch starts normally.
        eprintln!("voussoir: could not start the previous build: {error}");
    }
}

/// The command that starts the freshly installed binary.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn relaunch_command(path: &Path) -> std::process::Command {
    let mut command = std::process::Command::new(path);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command
}

/// Start the new build and leave this process.
///
/// The exit is delayed briefly because the command's reply leaves when the
/// handler returns; exiting inside it would drop the reply the page is waiting
/// for. On Windows the exit is also what releases the old binary.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn spawn_and_exit(path: PathBuf) -> Result<(), Error> {
    relaunch_command(&path).spawn().map_err(|error| {
        Error::Handler(format!("could not restart into the new build: {error}"))
    })?;
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(300));
        std::process::exit(0);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_versions_are_detected() {
        assert!(is_newer("0.1.0", "0.2.0"));
        assert!(is_newer("0.1.0", "0.1.1"));
        assert!(is_newer("0.1.0", "1.0.0"));
        assert!(is_newer("1.9.9", "1.10.0"));
        assert!(is_newer("1.2", "1.2.1"));
    }

    #[test]
    fn same_and_older_versions_are_not() {
        assert!(!is_newer("1.2.0", "1.2.0"));
        assert!(!is_newer("1.2", "1.2.0"));
        assert!(!is_newer("2.0.0", "1.9.9"));
    }

    #[test]
    fn prefixes_and_suffixes_are_tolerated() {
        assert!(is_newer("v1.0.0", "1.0.1"));
        assert!(is_newer("1.0.0", "v1.0.1"));
        assert!(!is_newer("1.0.0-beta.1", "1.0.0"));
        assert!(is_newer("1.0.0-beta.1", "1.0.1"));
    }

    #[test]
    fn non_numeric_segments_never_claim_an_update() {
        // Cannot tell: must not say "update available".
        assert!(!is_newer("1.0.0", "nightly"));
        assert!(!is_newer("1.0.0", ""));
    }

    #[test]
    fn the_command_is_registered_and_reports_unconfigured_without_a_url() {
        let mut registry = Registry::new();
        let config = Config::default();
        register(&mut registry, &config);
        assert!(registry.has("update.check"));

        let body = registry.dispatch(
            &crate::AppHandle::detached_for_tests(),
            "update.check",
            Value::Null,
        );
        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["ok"]["configured"], false);
        assert_eq!(parsed["ok"]["available"], false);
    }

    #[test]
    fn config_supplies_the_endpoint() {
        let config =
            Config::from_json(r#"{"update":{"url":"https://example.invalid/u.json"}}"#).unwrap();
        let update = UpdateConfig::from_config(&config, "1.0.0");
        assert_eq!(
            update.url.as_deref(),
            Some("https://example.invalid/u.json")
        );
        assert_eq!(update.current, "1.0.0");
    }
}
