//! Window management.
//!
//! Multi-window is a small registry owned by the event loop, since tao window
//! handles are not `Send`. Commands ask for a window by *label*; the request is
//! delivered to the UI thread through the `EventLoopProxy` and the window is
//! created there. Handles therefore never cross threads, and a command handler
//! running anywhere can still say "open a window".

use std::collections::HashMap;

use serde_json::Value;
use tao::{
    dpi::LogicalSize,
    event_loop::EventLoopWindowTarget,
    window::{Window, WindowId},
};
use wry::WebView;

use crate::error::Error;

/// Per-window overrides. Everything is optional; defaults come from the config.
#[derive(Debug, Clone, Default)]
pub struct WindowOptions {
    /// Stable name for the window, used by `__voussoir_close` and events. Defaults
    /// to the entry page path.
    pub label: Option<String>,
    pub title: Option<String>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    /// Page to load, relative to the served root.
    pub entry: Option<String>,
    /// Commands this window may call, intersected with the global allowlist.
    ///
    /// This is the runtime form of the `windows` map in `voussoir.json`: a window that
    /// renders untrusted content (a markdown preview, an embedded third-party
    /// page) can be given a strict subset without declaring its label up front.
    /// `None` means "the global surface", and a list can only ever remove
    /// commands, never add one the app withheld.
    pub allowlist: Option<Vec<String>>,
}

impl WindowOptions {
    /// Build options from a JSON payload, ignoring unknown fields.
    ///
    /// `allowlist` is accepted under its own name; `commands` is accepted too
    /// because that is what the config file calls the same idea, and a page
    /// forwarding its `voussoir.json` entry should not have to rename a field.
    pub fn from_json(payload: &Value) -> Self {
        let allowlist = payload
            .get("allowlist")
            .or_else(|| payload.get("commands"))
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            });
        Self {
            label: payload
                .get("label")
                .and_then(Value::as_str)
                .map(str::to_owned),
            title: payload
                .get("title")
                .and_then(Value::as_str)
                .map(str::to_owned),
            width: payload.get("width").and_then(Value::as_f64),
            height: payload.get("height").and_then(Value::as_f64),
            entry: payload
                .get("entry")
                .and_then(Value::as_str)
                .map(str::to_owned),
            allowlist,
        }
    }
}

/// The labels of every open window, shared with command handlers.
///
/// The event loop owns the windows themselves and tao handles are not `Send`, so
/// a handler cannot look at them. It can look at this: a plain list of labels,
/// kept in step with the loop's map, which is enough to answer `__voussoir_windows`
/// and to tell a page the label of the window it just opened. Ordering is
/// insertion order, so "the first window" means the one opened first rather than
/// whichever hash bucket came up.
#[derive(Debug, Clone, Default)]
pub struct OpenWindows {
    labels: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl OpenWindows {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a window as open. A repeated label is not inserted twice, because
    /// two windows sharing a label could not be addressed separately anyway.
    pub fn insert(&self, label: &str) {
        let mut labels = self.labels.lock().unwrap_or_else(|e| e.into_inner());
        if !labels.iter().any(|existing| existing == label) {
            labels.push(label.to_owned());
        }
    }

    /// Forget a window that closed. Unknown labels are ignored.
    pub fn remove(&self, label: &str) {
        let mut labels = self.labels.lock().unwrap_or_else(|e| e.into_inner());
        labels.retain(|existing| existing != label);
    }

    /// The open labels, in the order the windows were opened.
    pub fn labels(&self) -> Vec<String> {
        self.labels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Whether `label` names an open window.
    pub fn contains(&self, label: &str) -> bool {
        self.labels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|existing| existing == label)
    }

    /// A label that no open window is using, derived from `base`.
    ///
    /// A page that opens two windows without naming them must be able to address
    /// each one, which means the shell has to hand out distinct labels instead of
    /// defaulting both to the entry page. `base` comes back unchanged when it is
    /// free, so the common first window keeps its readable name.
    pub fn unique_label(&self, base: &str) -> String {
        let labels = self.labels.lock().unwrap_or_else(|e| e.into_inner());
        if !labels.iter().any(|existing| existing == base) {
            return base.to_owned();
        }
        for n in 2.. {
            let candidate = format!("{base}-{n}");
            if !labels.contains(&candidate) {
                return candidate;
            }
        }
        unreachable!("the loop above always finds a free label")
    }
}

/// One open window and the webview inside it.
pub struct WindowEntry {
    pub label: String,
    pub window: Window,
    pub webview: WebView,
}

/// Every open window, keyed by tao's window id.
pub type Windows = HashMap<WindowId, WindowEntry>;

/// Insert a window into the registry.
pub fn insert(windows: &mut Windows, id: WindowId, entry: WindowEntry) {
    windows.insert(id, entry);
}

/// Find a window by label.
pub fn find_by_label<'a>(windows: &'a Windows, label: &str) -> Option<&'a WindowEntry> {
    windows.values().find(|e| e.label == label)
}

/// The label of the only window, when exactly one is open. Commands with no
/// window argument use it, which makes single-window apps ergonomic.
pub fn sole_label(windows: &Windows) -> Option<String> {
    if windows.len() == 1 {
        windows.values().next().map(|e| e.label.clone())
    } else {
        None
    }
}

/// Pick the target window label from a payload.
///
/// An explicit `label` always wins. Otherwise the sole open window is used, and
/// with several open this is an error rather than a guess, so a command never
/// silently acts on the wrong window.
pub fn pick_label(labels: &[String], payload: &Value) -> Result<String, Error> {
    if let Some(label) = payload.get("label").and_then(Value::as_str) {
        return Ok(label.to_owned());
    }
    if labels.len() == 1 {
        return Ok(labels[0].clone());
    }
    Err(Error::Handler(match labels.len() {
        0 => "no windows are open".to_owned(),
        _ => "no window label given and more than one window is open".to_owned(),
    }))
}

/// Resolve a payload's `label` field against the open windows.
pub fn resolve_label(windows: &Windows, payload: &Value) -> Result<String, Error> {
    let labels: Vec<String> = windows.values().map(|e| e.label.clone()).collect();
    pick_label(&labels, payload)
}

/// Window size for a window, honoring per-window overrides.
pub fn size_for(options: &WindowOptions, default: (f64, f64)) -> LogicalSize<f64> {
    LogicalSize::new(
        options.width.unwrap_or(default.0),
        options.height.unwrap_or(default.1),
    )
}

/// Where pages are served from, shared by every window.
#[derive(Debug, Clone)]
pub struct Spec {
    pub entry: String,
    pub default_title: String,
    pub default_size: (f64, f64),
    /// Dev-server origin in debug builds, `None` in release.
    pub dev_origin: Option<String>,
}

impl Spec {
    /// The URL for a page path. Absolute URLs pass through, which is what makes
    /// "open a window pointed at the dev server" and "open a window pointed at a
    /// local file" both work.
    pub fn url_for(&self, entry: &str) -> String {
        if entry.starts_with("voussoir://")
            || entry.starts_with("http://")
            || entry.starts_with("https:")
        {
            return entry.to_owned();
        }
        match &self.dev_origin {
            Some(dev) => format!("{}/{entry}", dev.trim_end_matches('/')),
            None => format!("{}/{entry}", crate::APP_ORIGIN),
        }
    }

    /// The entry page for a set of options.
    pub fn entry_for(&self, options: &WindowOptions) -> String {
        options.entry.clone().unwrap_or_else(|| self.entry.clone())
    }

    /// The label for a set of options, defaulting to the entry page.
    pub fn label_for(&self, options: &WindowOptions) -> String {
        options
            .label
            .clone()
            .unwrap_or_else(|| self.entry_for(options))
    }

    pub fn title_for(&self, options: &WindowOptions) -> String {
        options
            .title
            .clone()
            .unwrap_or_else(|| self.default_title.clone())
    }
}

/// Create the window itself. The caller supplies a prepared webview builder so
/// protocol registration and the ready hook stay in one place.
pub(crate) fn create_window(
    event_loop: &EventLoopWindowTarget<crate::UserEvent>,
    spec: &Spec,
    options: &WindowOptions,
    build: &dyn Fn(String, Option<String>) -> wry::WebViewBuilder<'static>,
) -> Result<(WindowId, WindowEntry), Error> {
    use tao::window::WindowBuilder;

    let window = WindowBuilder::new()
        .with_title(spec.title_for(options))
        .with_inner_size(size_for(options, spec.default_size))
        .build(event_loop)
        .map_err(|e| Error::Runtime(e.to_string()))?;

    let target = spec.url_for(&spec.entry_for(options));
    let webview = crate::build_webview(build(target, Some(spec.label_for(options))), &window)?;

    Ok((
        window.id(),
        WindowEntry {
            label: spec.label_for(options),
            window,
            webview,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec() -> Spec {
        Spec {
            entry: "index.html".to_owned(),
            default_title: "App".to_owned(),
            default_size: (800.0, 600.0),
            dev_origin: None,
        }
    }

    #[test]
    fn urls_are_built_against_the_app_origin() {
        assert_eq!(
            spec().url_for("index.html"),
            "voussoir://localhost/index.html"
        );
        assert_eq!(
            spec().url_for("/app/x.html"),
            "voussoir://localhost//app/x.html"
        );
        // Pass-through for absolute URLs.
        assert_eq!(
            spec().url_for("http://127.0.0.1:5177/"),
            "http://127.0.0.1:5177/"
        );
    }

    #[test]
    fn dev_origin_replaces_the_app_origin() {
        let dev = Spec {
            dev_origin: Some("http://127.0.0.1:5177".to_owned()),
            ..spec()
        };
        assert_eq!(
            dev.url_for("index.html"),
            "http://127.0.0.1:5177/index.html"
        );
    }

    #[test]
    fn options_parse_from_json_and_ignore_junk() {
        let opts = WindowOptions::from_json(&json!({
            "label": "settings",
            "title": "Settings",
            "width": 400.0,
            "height": 300,
            "unknown": true,
        }));
        assert_eq!(opts.label.as_deref(), Some("settings"));
        assert_eq!(opts.title.as_deref(), Some("Settings"));
        assert_eq!(opts.width, Some(400.0));
        assert_eq!(opts.height, Some(300.0));

        let empty = WindowOptions::from_json(&json!({}));
        assert!(empty.label.is_none() && empty.entry.is_none());
    }

    #[test]
    fn sizes_fall_back_to_the_defaults_per_axis() {
        let size = size_for(
            &WindowOptions {
                width: Some(320.0),
                ..WindowOptions::default()
            },
            (800.0, 600.0),
        );
        assert_eq!(size.width, 320.0);
        assert_eq!(size.height, 600.0);
    }

    #[test]
    fn open_windows_track_labels_in_open_order() {
        let open = OpenWindows::new();
        assert!(open.labels().is_empty());

        open.insert("main");
        open.insert("settings");
        assert_eq!(open.labels(), vec!["main", "settings"]);
        assert!(open.contains("settings"));
        assert!(!open.contains("preview"));

        // A closed window is gone; closing it twice is harmless.
        open.remove("main");
        open.remove("main");
        assert_eq!(open.labels(), vec!["settings"]);
    }

    #[test]
    fn duplicate_labels_are_not_tracked_twice() {
        let open = OpenWindows::new();
        open.insert("main");
        open.insert("main");
        assert_eq!(open.labels(), vec!["main"]);
    }

    #[test]
    fn unnamed_windows_get_distinct_labels() {
        let open = OpenWindows::new();
        // The first request keeps its readable base name.
        let first = open.unique_label("index.html");
        assert_eq!(first, "index.html");
        open.insert(&first);

        // The second cannot reuse it, or the page could not address either.
        let second = open.unique_label("index.html");
        assert_eq!(second, "index.html-2");
        open.insert(&second);

        assert_eq!(open.unique_label("index.html"), "index.html-3");
        // A different base is unaffected.
        assert_eq!(open.unique_label("settings"), "settings");

        // Freed labels are reused rather than skipped forever.
        open.remove("index.html-2");
        assert_eq!(open.unique_label("index.html"), "index.html-2");
    }

    #[test]
    fn a_window_can_be_built_with_its_own_allowlist() {
        let options = WindowOptions::from_json(&json!({
            "label": "preview",
            "allowlist": ["greet", "stats"],
        }));
        assert_eq!(options.label.as_deref(), Some("preview"));
        assert_eq!(
            options.allowlist,
            Some(vec!["greet".to_owned(), "stats".to_owned()])
        );

        // The config file's spelling of the same idea is accepted too.
        let via_commands = WindowOptions::from_json(&json!({ "commands": ["greet"] }));
        assert_eq!(via_commands.allowlist, Some(vec!["greet".to_owned()]));

        // Absent means "the global surface", not "nothing allowed".
        assert!(WindowOptions::from_json(&json!({ "label": "main" }))
            .allowlist
            .is_none());
        // Malformed entries are ignored rather than producing an empty lane,
        // which would silently lock a window out of everything.
        assert!(WindowOptions::from_json(&json!({ "allowlist": "greet" }))
            .allowlist
            .is_none());
        assert_eq!(
            WindowOptions::from_json(&json!({ "allowlist": [1, "greet"] })).allowlist,
            Some(vec!["greet".to_owned()])
        );
    }

    #[test]
    fn labels_default_to_the_entry_page() {
        let s = spec();
        let options = WindowOptions::default();
        assert_eq!(s.label_for(&options), "index.html");
        let named = WindowOptions {
            label: Some("settings".to_owned()),
            entry: Some("settings.html".to_owned()),
            ..WindowOptions::default()
        };
        assert_eq!(s.label_for(&named), "settings");
        assert_eq!(s.entry_for(&named), "settings.html");
    }

    #[test]
    fn target_labels_are_never_guessed() {
        // No windows: an error, not a wrong window.
        let err = pick_label(&[], &json!({})).unwrap_err();
        assert!(err.to_string().contains("no windows are open"), "{err}");

        // One window: that window.
        let one = vec!["main".to_owned()];
        assert_eq!(pick_label(&one, &json!({})).unwrap(), "main");

        // Two windows with no label: refuse rather than pick one.
        let two = vec!["main".to_owned(), "settings".to_owned()];
        let err = pick_label(&two, &json!({})).unwrap_err();
        assert!(err.to_string().contains("more than one window"), "{err}");

        // An explicit label always wins, even when several are open.
        assert_eq!(
            pick_label(&two, &json!({ "label": "settings" })).unwrap(),
            "settings"
        );
    }
}
