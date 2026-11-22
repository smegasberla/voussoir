//! The single config file: `voussoir.json`.
//!
//! Code-first defaults mean a minimal app needs none of it. Every field is
//! optional; unknown fields are ignored so a newer config does not break an
//! older shell.

use serde_json::Value;
use std::collections::HashMap;

const DEFAULT_NAME: &str = "Voussoir App";

/// Window and identity settings for one app.
#[derive(Debug, Clone)]
pub struct Config {
    pub name: String,
    pub title: String,
    /// The app's own version, used by the `updater` feature.
    ///
    /// `None` means the updater falls back to the framework's crate version,
    /// which is almost never what an app's feed tracks; set this to the same
    /// string that is in the app's own `Cargo.toml`.
    pub version: Option<String>,
    pub width: f64,
    pub height: f64,
    /// Page loaded at startup, relative to the served root.
    pub entry: String,
    /// Optional allowlist. When present, only these commands may be dispatched,
    /// even if more were registered. `None` means "everything registered".
    pub commands: Option<Vec<String>>,
    /// Per-window allowlists keyed by window label. A window named here can
    /// only call commands from the intersection of its list and the global
    /// allowlist, so a `voussoir.json` entry can only ever narrow a window's
    /// surface, never widen it.
    pub window_commands: HashMap<String, Vec<String>>,
    /// Update endpoint, read by the `updater` feature. `None` disables checks.
    pub update_url: Option<String>,
    /// Base64 Ed25519 public key an update artifact must be signed with.
    ///
    /// Installing requires this: without it the shell could only prove the bytes
    /// match the feed, not that the publisher produced them. `None` leaves
    /// `update.check` working and makes `update.apply` refuse.
    pub update_public_key: Option<String>,
    /// How long the page waits for a command before rejecting the promise.
    ///
    /// A fetch with no deadline waits forever, so a handler that deadlocks made
    /// a hang indistinguishable from slowness: the page kept spinning and the
    /// developer had no signal. `0` disables the deadline for apps whose commands
    /// legitimately run for minutes; [`crate::Config::command_timeout_ms`] is
    /// the whole-file default, and a page can override it per command.
    pub command_timeout_ms: u64,
}

/// The default command deadline: long enough for real I/O, short enough that a
/// deadlock is reported while someone is still watching.
pub const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 30_000;

impl Default for Config {
    fn default() -> Self {
        Self {
            name: DEFAULT_NAME.to_owned(),
            title: DEFAULT_NAME.to_owned(),
            version: None,
            width: 800.0,
            height: 600.0,
            entry: "index.html".to_owned(),
            commands: None,
            window_commands: HashMap::new(),
            update_url: None,
            update_public_key: None,
            command_timeout_ms: DEFAULT_COMMAND_TIMEOUT_MS,
        }
    }
}

impl Config {
    /// Parse a `voussoir.json` string on top of the defaults.
    ///
    /// Returns the JSON error message rather than a crate error type so it can
    /// be surfaced with the file name that failed.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let value: Value = serde_json::from_str(json)
            .map_err(|e| format!("voussoir.json is not valid JSON: {e}"))?;
        let Value::Object(map) = value else {
            return Err("voussoir.json must be a JSON object".to_owned());
        };

        let mut config = Self::default();

        if let Some(name) = map.get("name").and_then(Value::as_str) {
            config.name = name.to_owned();
            config.title = name.to_owned();
        }
        if let Some(version) = map.get("version").and_then(Value::as_str) {
            config.version = Some(version.to_owned());
        }
        if let Some(title) = map.get("title").and_then(Value::as_str) {
            config.title = title.to_owned();
        }
        if let Some(w) = map.get("width").and_then(Value::as_f64) {
            config.width = w;
        }
        if let Some(h) = map.get("height").and_then(Value::as_f64) {
            config.height = h;
        }
        if let Some(entry) = map.get("entry").and_then(Value::as_str) {
            config.entry = entry.trim_start_matches('/').to_owned();
        }
        if let Some(url) = map
            .get("update")
            .and_then(|u| u.get("url"))
            .and_then(Value::as_str)
        {
            config.update_url = Some(url.to_owned());
        }
        if let Some(key) = map
            .get("update")
            .and_then(|u| u.get("public_key"))
            .and_then(Value::as_str)
        {
            config.update_public_key = Some(key.trim().to_owned());
        }
        if let Some(ms) = map.get("command_timeout_ms").and_then(Value::as_u64) {
            config.command_timeout_ms = ms;
        }
        if let Some(list) = map.get("commands").and_then(Value::as_array) {
            let names = list
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            config.commands = Some(names);
        }
        if let Some(windows) = map.get("windows").and_then(Value::as_object) {
            for (label, spec) in windows {
                if let Some(list) = spec.get("commands").and_then(Value::as_array) {
                    let names = list
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect();
                    config.window_commands.insert(label.clone(), names);
                }
            }
        }

        if !config.width.is_finite()
            || config.width <= 0.0
            || !config.height.is_finite()
            || config.height <= 0.0
        {
            return Err(format!(
                "width and height must be positive finite numbers, got {}x{}",
                config.width, config.height
            ));
        }
        if config.entry.split('/').any(|segment| segment == "..") || config.entry.contains('\\') {
            return Err(format!(
                "entry {:?} must stay inside the served root",
                config.entry
            ));
        }
        if matches!(&config.version, Some(version) if version.trim().is_empty()) {
            return Err("version must not be empty when set".to_owned());
        }

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_need_no_config() {
        let c = Config::default();
        assert_eq!(c.title, "Voussoir App");
        assert_eq!((c.width, c.height), (800.0, 600.0));
        assert!(c.commands.is_none());
    }

    #[test]
    fn name_becomes_title_unless_overridden() {
        let c = Config::from_json(r#"{"name":"Notes"}"#).unwrap();
        assert_eq!(c.title, "Notes");

        let c = Config::from_json(r#"{"name":"Notes","title":"Notes - main"}"#).unwrap();
        assert_eq!(c.title, "Notes - main");
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let c =
            Config::from_json(r#"{"name":"X","plugin_registry":{"a":1},"future":true}"#).unwrap();
        assert_eq!(c.name, "X");
    }

    #[test]
    fn command_allowlist_parses() {
        let c = Config::from_json(r#"{"commands":["greet","save"]}"#).unwrap();
        assert_eq!(c.commands.unwrap(), vec!["greet", "save"]);
    }

    #[test]
    fn per_window_allowlists_parse_by_label() {
        let c = Config::from_json(
            r#"{"commands":["greet","save"],"windows":{"preview":{"commands":["greet"]}}}"#,
        )
        .unwrap();
        assert_eq!(c.window_commands["preview"], vec!["greet"]);
        // A window without an entry falls back to the global list.
        assert!(!c.window_commands.contains_key("main"));
        // A window entry without a commands array is ignored.
        let c = Config::from_json(r#"{"windows":{"preview":{"width":300}}}"#).unwrap();
        assert!(c.window_commands.is_empty());
    }

    #[test]
    fn entry_defaults_and_normalises() {
        assert_eq!(Config::default().entry, "index.html");
        let c = Config::from_json(r#"{"entry":"/app/index.html"}"#).unwrap();
        assert_eq!(c.entry, "app/index.html");
    }

    #[test]
    fn a_version_is_read_for_the_updater() {
        let config = Config::from_json(r#"{"name":"Notes","version":"2.1.0"}"#).unwrap();
        assert_eq!(config.version.as_deref(), Some("2.1.0"));
        assert!(Config::default().version.is_none());
    }

    #[test]
    fn the_update_endpoint_parses_and_defaults_to_none() {
        assert!(Config::default().update_url.is_none());

        let c = Config::from_json(r#"{"update":{"url":"https://example.com/v.json"}}"#).unwrap();
        assert_eq!(c.update_url.as_deref(), Some("https://example.com/v.json"));

        // A malformed update block is ignored rather than fatal.
        let c = Config::from_json(r#"{"update":{"url":42}}"#).unwrap();
        assert!(c.update_url.is_none());
    }

    #[test]
    fn impossible_sizes_and_escaping_entries_are_refused() {
        assert!(Config::from_json(r#"{"width":0}"#).is_err());
        assert!(Config::from_json(r#"{"height":-1}"#).is_err());
        let error = Config::from_json(r#"{"entry":"../secret.html"}"#).unwrap_err();
        assert!(error.contains("served root"), "{error}");
        assert!(Config::from_json(r#"{"entry":"sub\\page.html"}"#).is_err());
        assert!(Config::from_json(r#"{"version":"  "}"#).is_err());
    }

    #[test]
    fn invalid_json_reports_clearly() {
        let err = Config::from_json("{,}").unwrap_err();
        assert!(err.contains("voussoir.json"), "{err}");

        let err = Config::from_json("[1,2]").unwrap_err();
        assert!(err.contains("JSON object"), "{err}");
    }
}
