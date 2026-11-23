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

/// Fetch a URL as text. Kept separate so the comparison logic stays testable
/// without a network.
///
/// On Android this goes through the activity's `HttpURLConnection`, because a
/// bundled TLS stack has to carry its own root certificates while the platform
/// one already trusts the system store.
///
/// The timeouts and the cap are not optional. ureq sets no default timeout, and
/// this command runs on a worker thread: a hung endpoint parked that worker
/// forever, which from the page looks like a command that never answers. The
/// endpoint answers with a version and a note, so a body past the cap is not
/// that document and reading it would let one bad endpoint cost an unbounded
/// allocation.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn fetch(url: &str) -> Result<String, Error> {
    fetch_limited(url, FETCH_TIMEOUT, BODY_CAP)
}

/// How long one update check may take, from connect to the last body byte.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The largest body an update check will read.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const BODY_CAP: u64 = 64 * 1024;

/// [`fetch`] with the bounds as parameters, so a test can use a short deadline
/// and a tiny cap against a local socket instead of waiting out production
/// values.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn fetch_limited(
    url: &str,
    timeout: std::time::Duration,
    cap: u64,
) -> Result<String, Error> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .timeout_connect(Some(timeout))
        .build()
        .new_agent();
    let mut response = agent
        .get(url)
        .call()
        .map_err(|e| Error::Handler(format!("update check failed: {e}")))?;
    response
        .body_mut()
        .with_config()
        .limit(cap)
        .read_to_string()
        .map_err(|e| Error::Handler(format!("update check failed: {e}")))
}

/// Fetch a URL as text through the activity's HTTP stack.
///
/// Android ships a curated certificate store the app cannot see, and a bundled
/// one has to be updated with the app, so the platform's `HttpURLConnection` is
/// the honest choice. It blocks, which is fine: `update.check` is async.
#[cfg(target_os = "android")]
fn fetch(url: &str) -> Result<String, Error> {
    crate::platform::http_get(url).ok_or_else(|| {
        Error::Handler("update check failed: no activity can make the request".to_owned())
    })
}

/// On iOS the honest answer is that there is nothing to fetch: apps update
/// through the App Store, and a bundled TLS stack would be the kind of
/// dynamic-policy surface Apple reviews line by line. `update.check` stays
/// registered so an app declaring the feature on every platform compiles,
/// and the version comparison stays testable; the endpoint fetch waits for
/// the Swift glue (IOS-PLAN Phase I-3) if a diagnostic fetch is ever wanted.
#[cfg(target_os = "ios")]
fn fetch(_url: &str) -> Result<String, Error> {
    Err(Error::Handler(
        "update checks are diagnostic on iOS; the App Store owns updates".to_owned(),
    ))
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

/// Tests for the fetch bounds. They use a local socket and real HTTP framing,
/// because the failure being fixed is a network call that never returns and a
/// body that never ends.
#[cfg(all(test, not(any(target_os = "android", target_os = "ios"))))]
mod fetch_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::time::{Duration, Instant};

    fn serve_once(response: String) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let addr = listener.local_addr().expect("local addr");
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut scratch = [0u8; 1024];
                let _ = stream.read(&mut scratch);
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        addr
    }

    fn ok_response(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    #[test]
    fn a_body_under_the_cap_is_read() {
        let addr = serve_once(ok_response(r#"{"version":"1.2.0"}"#));
        let body = fetch_limited(
            &format!("http://{addr}/"),
            Duration::from_secs(5),
            BODY_CAP,
        )
        .expect("a well-framed response must be read");
        assert_eq!(body, r#"{"version":"1.2.0"}"#);
    }

    #[test]
    fn a_body_over_the_cap_is_refused() {
        let body = "x".repeat(4096);
        let addr = serve_once(ok_response(&body));
        let error = fetch_limited(&format!("http://{addr}/"), Duration::from_secs(5), 64)
            .unwrap_err();
        assert!(
            error.to_string().contains("update check failed"),
            "the page must see a failed check, not a truncated document: {error}"
        );
    }

    #[test]
    fn a_hung_endpoint_times_out_instead_of_parking_the_worker() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let addr = listener.local_addr().expect("local addr");
        std::thread::spawn(move || {
            // Accept, then say nothing: this is what a hung endpoint does.
            if let Ok((stream, _)) = listener.accept() {
                std::thread::sleep(Duration::from_secs(30));
                drop::<TcpStream>(stream);
            }
        });
        let started = Instant::now();
        let error = fetch_limited(
            &format!("http://{addr}/"),
            Duration::from_millis(150),
            BODY_CAP,
        )
        .unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the check must give up at its deadline, took {:?}",
            started.elapsed()
        );
        assert!(error.to_string().contains("update check failed"), "{error}");
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
/// One platform's artifact inside a manifest.
#[derive(Debug, Clone)]
struct Release {
    version: String,
    url: String,
    sha256: String,
    /// Base64 Ed25519 signature over the artifact bytes.
    signature: Option<String>,
    /// A patch from one older version to this one, when the feed offers one for
    /// this platform. Patch bytes are themselves checked before use, and the
    /// reconstructed binary is checked against `sha256`/`signature` exactly as a
    /// full download would be.
    patch: Option<Patch>,
    notes: Value,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[derive(Debug, Clone)]
struct Patch {
    from: String,
    url: String,
    sha256: String,
    signature: Option<String>,
}

/// The key a manifest uses for this build's artifact.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn platform_key() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return "linux-x86_64";
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        return "linux-aarch64";
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return "macos-x86_64";
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return "macos-aarch64";
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return "windows-x86_64";
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        return "windows-aarch64";
    }
    #[allow(unreachable_code)]
    "unknown"
}

/// Parse and validate a manifest before anything is downloaded.
///
/// Two shapes are accepted: a single artifact at the top level (`url` and
/// `sha256`), or a `downloads` object keyed by platform so one feed can serve
/// every release. `sha256` is mandatory: an update that cannot be checked
/// against the feed must not be installed, because a truncated download of a
/// binary is not a smaller bug than a malicious one.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn parse_manifest(body: &str) -> Result<Release, Error> {
    let value: Value = serde_json::from_str(body)
        .map_err(|e| Error::Handler(format!("update endpoint returned invalid JSON: {e}")))?;
    let version = value
        .get("version")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| Error::Handler("update endpoint has no `version`".to_owned()))?
        .to_owned();
    let notes = value.get("notes").cloned().unwrap_or(Value::Null);

    let artifact = match value.get("downloads") {
        Some(Value::Object(map)) => {
            let key = platform_key();
            map.get(key).cloned().ok_or_else(|| {
                let mut offered: Vec<&str> = map.keys().map(String::as_str).collect();
                offered.sort_unstable();
                Error::Handler(format!(
                    "update endpoint has no download for `{key}`; it offers [{}]",
                    offered.join(", ")
                ))
            })?
        }
        Some(_) => {
            return Err(Error::Handler(
                "update endpoint `downloads` must be an object keyed by platform".to_owned(),
            ))
        }
        None => value.clone(),
    };

    let url = artifact
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| {
            Error::Handler(format!(
                "update endpoint has no download `url` for `{}`",
                platform_key()
            ))
        })?
        .to_owned();
    require_trusted_url(&url)?;

    let sha256 = artifact
        .get("sha256")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            Error::Handler(
                "update endpoint has no `sha256`; refusing to install an unverified artifact"
                    .to_owned(),
            )
        })?
        .to_owned();
    check_sha256_text(&sha256)?;

    let signature = artifact
        .get("signature")
        .and_then(Value::as_str)
        .map(|signature| signature.trim().to_owned());

    let patch = match value.get("patches") {
        Some(Value::Object(map)) => match map.get(platform_key()) {
            Some(entry) => Some(parse_patch(entry)?),
            None => None,
        },
        Some(_) => {
            return Err(Error::Handler(
                "update endpoint `patches` must be an object keyed by platform".to_owned(),
            ))
        }
        None => None,
    };

    Ok(Release {
        version,
        url,
        sha256,
        signature,
        patch,
        notes,
    })
}

/// Read one platform's patch entry.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn parse_patch(entry: &Value) -> Result<Patch, Error> {
    let text = |field: &str| {
        entry
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                Error::Handler(format!("update patch has no `{field}` for this platform"))
            })
    };
    let patch = Patch {
        from: text("from")?,
        url: text("url")?,
        sha256: text("sha256")?,
        signature: entry
            .get("signature")
            .and_then(Value::as_str)
            .map(|signature| signature.trim().to_owned()),
    };
    require_trusted_url(&patch.url)?;
    check_sha256_text(&patch.sha256)?;
    Ok(patch)
}

/// Refuse to download a binary over a channel that can rewrite it in transit.
///
/// Plain HTTP is accepted only for loopback, which is what a test server and a
/// local release directory use; a real feed has to be TLS.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn require_trusted_url(url: &str) -> Result<(), Error> {
    if url.starts_with("https://") {
        return Ok(());
    }
    if let Some(rest) = url.strip_prefix("http://") {
        let host = rest.split(['/', ':', '?']).next().unwrap_or("");
        if matches!(host, "127.0.0.1" | "localhost" | "[::1]") {
            return Ok(());
        }
    }
    Err(Error::Handler(format!(
        "update download URL must use https (http is allowed on loopback for tests): {url}"
    )))
}

/// The lowercase hex SHA-256 of `bytes`.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Check that an expected digest is well formed before downloading anything.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn check_sha256_text(expected: &str) -> Result<(), Error> {
    let normalized = expected.trim().to_ascii_lowercase();
    let looks_right = normalized.len() == 64 && normalized.bytes().all(|b| b.is_ascii_hexdigit());
    if looks_right {
        return Ok(());
    }
    Err(Error::Handler(format!(
        "update manifest has an invalid `sha256` (`{expected}`); expected 64 hex characters"
    )))
}

/// The digest check, which is what makes a download safe to install at all.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn verify_sha256(bytes: &[u8], expected: &str) -> Result<(), Error> {
    let actual = sha256_hex(bytes);
    if actual == expected.trim().to_ascii_lowercase() {
        return Ok(());
    }
    Err(Error::Handler(format!(
        "update artifact failed verification: expected sha256 {}, got {actual}",
        expected.trim()
    )))
}

/// The exact length of an Ed25519 public key and signature.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const PUBLIC_KEY_LEN: usize = 32;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const SIGNATURE_LEN: usize = 64;

/// Decode the base64 public key from `update.public_key`.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn decode_public_key(text: &str) -> Result<[u8; PUBLIC_KEY_LEN], Error> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .map_err(|error| {
            Error::Handler(format!(
                "`update.public_key` is not valid base64: {error}"
            ))
        })?;
    bytes.as_slice().try_into().map_err(|_| {
        Error::Handler(format!(
            "`update.public_key` must be {PUBLIC_KEY_LEN} bytes (a raw Ed25519 key), got {}",
            bytes.len()
        ))
    })
}

/// Verify an artifact's detached Ed25519 signature.
///
/// This is what makes a feed trustworthy, which a digest alone cannot do: the
/// digest proves the bytes match the feed, and the signature proves the feed's
/// publisher produced them.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn verify_signature(bytes: &[u8], signature: &str, public_key: &str) -> Result<(), Error> {
    use base64::Engine;
    use ed25519_dalek::{Signature, VerifyingKey};

    let key = decode_public_key(public_key)?;
    let verifying = VerifyingKey::from_bytes(&key)
        .map_err(|error| Error::Handler(format!("`update.public_key` is not a key: {error}")))?;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(signature.trim())
        .map_err(|error| {
            Error::Handler(format!("update `signature` is not valid base64: {error}"))
        })?;
    let raw: [u8; SIGNATURE_LEN] = raw.as_slice().try_into().map_err(|_| {
        Error::Handler(format!(
            "update `signature` must be {SIGNATURE_LEN} bytes, got {}",
            raw.len()
        ))
    })?;
    let signature = Signature::from_bytes(&raw);
    verifying.verify_strict(bytes, &signature).map_err(|error| {
        Error::Handler(format!(
            "update artifact signature does not match `update.public_key`: {error}"
        ))
    })
}

/// Manifest, trust and digest rules, independent of the network and the disk.
#[cfg(all(test, not(any(target_os = "android", target_os = "ios"))))]
mod manifest_tests {
    use super::*;

    fn single(sha: &str) -> String {
        format!(
            r#"{{"version":"1.2.0","notes":"fixes","url":"https://example.com/app","sha256":"{sha}"}}"#
        )
    }

    const GOOD: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn a_single_artifact_manifest_parses() {
        let release = parse_manifest(&single(GOOD)).unwrap();
        assert_eq!(release.version, "1.2.0");
        assert_eq!(release.url, "https://example.com/app");
        assert_eq!(release.notes, json!("fixes"));
    }

    #[test]
    fn a_per_platform_manifest_picks_this_build() {
        let key = platform_key();
        let body = json!({
            "version": "2.0.0",
            "downloads": {
                key: { "url": format!("https://example.com/{key}"), "sha256": GOOD },
                "other": { "url": "https://example.com/other", "sha256": GOOD }
            }
        })
        .to_string();
        let release = parse_manifest(&body).unwrap();
        assert!(release.url.ends_with(platform_key()), "{}", release.url);
    }

    #[test]
    fn a_missing_platform_names_the_ones_offered() {
        let body = json!({
            "version": "2.0.0",
            "downloads": {
                "bermuda-riscv": { "url": "https://example.com/x", "sha256": GOOD }
            }
        })
        .to_string();
        let error = parse_manifest(&body).unwrap_err().to_string();
        assert!(error.contains(platform_key()), "{error}");
        assert!(error.contains("bermuda-riscv"), "{error}");
    }

    #[test]
    fn an_unverified_artifact_is_never_accepted() {
        let body = r#"{"version":"1.2.0","url":"https://example.com/app"}"#;
        let error = parse_manifest(body).unwrap_err().to_string();
        assert!(error.contains("sha256"), "{error}");
        for bad in ["", "abc", "zz"] {
            let error = parse_manifest(&single(bad)).unwrap_err().to_string();
            assert!(error.contains("invalid"), "{bad}: {error}");
        }
    }

    #[test]
    fn plain_http_is_refused_outside_loopback() {
        let body = json!({
            "version": "1.0.0",
            "url": "http://example.com/app",
            "sha256": GOOD
        })
        .to_string();
        assert!(parse_manifest(&body).is_err());
        let localhost = json!({
            "version": "1.0.0",
            "url": "http://127.0.0.1:9/app",
            "sha256": GOOD
        })
        .to_string();
        assert!(parse_manifest(&localhost).is_ok());
    }

    #[test]
    fn the_digest_matches_the_known_sha256_of_ab_c() {
        assert_eq!(sha256_hex(b"abc"), GOOD);
        assert!(check_sha256_text(GOOD).is_ok());
    }

    #[test]
    fn a_mismatched_digest_is_refused() {
        let error = verify_sha256(b"tampered", GOOD).unwrap_err().to_string();
        assert!(error.contains("failed verification"), "{error}");
        assert!(verify_sha256(b"abc", &GOOD.to_ascii_uppercase()).is_ok());
    }
}

// ---------------------------------------------------------------------------
// Applying an update (desktop only)
// ---------------------------------------------------------------------------

#[cfg(not(any(target_os = "android", target_os = "ios")))]
use std::path::{Path, PathBuf};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use std::sync::{Arc, Mutex};

/// How long one artifact download may take, from connect to the last byte.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const DOWNLOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// The largest artifact accepted. The shell is one binary, so 256 MiB is far
/// past any real build and small enough that a wrong URL cannot fill a disk.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const ARTIFACT_CAP: u64 = 256 * 1024 * 1024;

/// Download `url` with a deadline and a cap.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn download(
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
struct ProgressThrottle {
    last: Option<std::time::Instant>,
    interval: std::time::Duration,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
impl ProgressThrottle {
    fn new(interval: std::time::Duration) -> Self {
        Self {
            last: None,
            interval,
        }
    }

    /// Whether this update should be sent: the first one, a finished one, or one
    /// that is at least `interval` after the previous send.
    fn ready(&mut self, done: bool) -> bool {
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

#[cfg(all(windows, not(any(target_os = "android", target_os = "ios"))))]
fn install_platform(
    temp: &Path,
    target: &Path,
    backup: &Path,
    _parent: &Path,
) -> Result<(), Error> {
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

/// The whole install: fetch the feed, download, verify, replace.
///
/// `target` is a parameter so a test can install into a temporary file instead
/// of replacing the test binary.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn apply_to(
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
    if let Some(patch) = &release.patch {
        if patch.from == update.current {
            if let Ok(old) = std::fs::read(target) {
                let bytes = download(&patch.url, on_progress)?;
                verify_sha256(&bytes, &patch.sha256)?;
                if let Some(patch_signature) = &patch.signature {
                    verify_signature(&bytes, patch_signature, public_key)?;
                }
                let rebuilt = crate::delta::apply(&old, &bytes)?;
                verify_sha256(&rebuilt, &release.sha256)?;
                verify_signature(&rebuilt, signature, public_key)?;
                install_bytes(&rebuilt, target)?;
                return Ok(json!({
                    "installed": true,
                    "patched": true,
                    "version": release.version,
                    "path": target.display().to_string(),
                    "notes": release.notes,
                }));
            }
        }
    }

    let bytes = download(&release.url, on_progress)?;
    verify_sha256(&bytes, &release.sha256)?;
    verify_signature(&bytes, signature, public_key)?;
    install_bytes(&bytes, target)?;
    Ok(json!({
        "installed": true,
        "patched": false,
        "version": release.version,
        "path": target.display().to_string(),
        "notes": release.notes,
    }))
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

/// Download, install and config-version rules that need a disk and a socket.
#[cfg(all(test, not(any(target_os = "android", target_os = "ios"))))]
mod install_tests {
    use super::*;
    use base64::Engine;
    use ed25519_dalek::{Signer, SigningKey};
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener};
    use std::time::Duration;

    /// A deterministic keypair, so the tests do not depend on a random source.
    fn test_key() -> (SigningKey, String) {
        let signing = SigningKey::from_bytes(&[42u8; 32]);
        let public = base64::engine::general_purpose::STANDARD
            .encode(signing.verifying_key().as_bytes());
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
        let dir = std::env::temp_dir().join(format!(
            "voussoir-update-{}-{}",
            std::process::id(),
            name
        ));
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
        let error =
            download_limited(&format!("http://{addr}/big"), Duration::from_secs(5), 64, &mut |_, _| {})
                .unwrap_err();
        assert!(error.to_string().contains("download failed"), "{error}");
    }

    #[test]
    fn a_download_returns_exactly_the_bytes_served() {
        let addr = serve(vec![("/app".to_owned(), b"binary-ish".to_vec())]);
        let bytes =
            download_limited(&format!("http://{addr}/app"), Duration::from_secs(5), 1024, &mut |_, _| {}).unwrap();
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
        let error = apply_to(&update_for(addr, &public), &target, false, &mut |_, _| {}).unwrap_err();
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
            feed_body["signature"] = json!(base64::engine::general_purpose::STANDARD.encode([0u8; 64]));
            vec![
                ("/feed".to_owned(), feed_body.to_string().into_bytes()),
                ("/app".to_owned(), artifact.clone()),
            ]
        });
        let (dir, target) = temp_target("signature", b"old");
        let error = apply_to(&update_for(addr, &public), &target, false, &mut |_, _| {}).unwrap_err();
        assert!(error.to_string().contains("signature does not match"), "{error}");
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
        let error = apply_to(&update_for(addr, &public), &target, false, &mut |_, _| {}).unwrap_err();
        assert!(error.to_string().contains("no `signature`"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_up_to_date_feed_installs_nothing() {
        let (signing, public) = test_key();
        let artifact = b"same".to_vec();
        let addr = serve_with(|addr| {
            vec![("/feed".to_owned(), feed(&signing, addr, "1.0.0", &artifact))]
        });
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
        assert_eq!(
            UpdateConfig::from_config(&config, "0.2.0").current,
            "9.9.9"
        );
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
        assert!(error.to_string().contains("signature does not match"), "{error}");
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
        let error = apply_to(&update_for(addr, &public), &target, false, &mut |_, _| {})
            .unwrap_err();
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
