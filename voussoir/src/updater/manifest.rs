//! The feed: what a manifest may say, and what has to be true before its bytes
//! are trusted.
//!
//! Split out so the parsing and the crypto are testable without a socket or a
//! disk, and so the command layer depends on a module that cannot open a
//! connection.

use serde_json::Value;

use crate::Error;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
/// One platform's artifact inside a manifest.
#[derive(Debug, Clone)]
pub(super) struct Release {
    pub(super) version: String,
    pub(super) url: String,
    pub(super) sha256: String,
    /// Base64 Ed25519 signature over the artifact bytes.
    pub(super) signature: Option<String>,
    /// Patches from older versions to this one, oldest first.
    ///
    /// A publisher may offer several: the updater picks the one whose `from`
    /// matches the running version, so a user who skipped releases still only
    /// downloads the delta. Patch bytes are themselves checked before use, and
    /// the reconstructed binary is checked against `sha256`/`signature` exactly
    /// as a full download would be.
    pub(super) patches: Vec<Patch>,
    pub(super) notes: Value,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[derive(Debug, Clone)]
pub(super) struct Patch {
    pub(super) from: String,
    pub(super) url: String,
    pub(super) sha256: String,
    pub(super) signature: Option<String>,
}

/// The key a manifest uses for this build's artifact.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(super) fn platform_key() -> &'static str {
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
pub(super) fn parse_manifest(body: &str) -> Result<Release, Error> {
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

    let patches = match value.get("patches") {
        Some(Value::Object(map)) => match map.get(platform_key()) {
            // A list is the current shape. A single object is what an older
            // publisher wrote, and reading it keeps those feeds updating instead
            // of failing with "`patches` must be an object keyed by platform".
            Some(Value::Array(entries)) => entries
                .iter()
                .map(parse_patch)
                .collect::<Result<Vec<_>, _>>()?,
            Some(entry @ Value::Object(_)) => vec![parse_patch(entry)?],
            Some(_) => {
                return Err(Error::Handler(format!(
                    "update `patches` for `{}` must be a list of patches",
                    platform_key()
                )))
            }
            None => Vec::new(),
        },
        Some(_) => {
            return Err(Error::Handler(
                "update endpoint `patches` must be an object keyed by platform".to_owned(),
            ))
        }
        None => Vec::new(),
    };

    Ok(Release {
        version,
        url,
        sha256,
        signature,
        patches,
        notes,
    })
}

/// A manifest as `parse_manifest` understood it, for the contract test.
///
/// The CLI generates feeds and this crate consumes them, and neither can see the
/// other's tests: a format change on one side passed both suites while the
/// shipped pair was broken. `tests/updater_contract.rs` parses a fixture written
/// by `scripts/make-updater-fixture.ts` and applies its patches, so a drift in
/// the shape fails here instead of in a user's update.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct ParsedManifest {
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub signature: Option<String>,
    /// Oldest first, as the feed listed them.
    pub patches: Vec<ParsedPatch>,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct ParsedPatch {
    pub from: String,
    pub url: String,
    pub sha256: String,
    pub signature: Option<String>,
}

/// Parse a feed exactly as `update.apply` does. See [`ParsedManifest`].
#[cfg(not(any(target_os = "android", target_os = "ios")))]
#[doc(hidden)]
pub fn parse_manifest_for_contract(body: &str) -> Result<ParsedManifest, Error> {
    let release = parse_manifest(body)?;
    Ok(ParsedManifest {
        version: release.version,
        url: release.url,
        sha256: release.sha256,
        signature: release.signature,
        patches: release
            .patches
            .into_iter()
            .map(|patch| ParsedPatch {
                from: patch.from,
                url: patch.url,
                sha256: patch.sha256,
                signature: patch.signature,
            })
            .collect(),
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
pub(super) fn require_trusted_url(url: &str) -> Result<(), Error> {
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
pub(super) fn sha256_hex(bytes: &[u8]) -> String {
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
pub(super) fn verify_sha256(bytes: &[u8], expected: &str) -> Result<(), Error> {
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
            Error::Handler(format!("`update.public_key` is not valid base64: {error}"))
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
pub(super) fn verify_signature(
    bytes: &[u8],
    signature: &str,
    public_key: &str,
) -> Result<(), Error> {
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
    use serde_json::json;

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
