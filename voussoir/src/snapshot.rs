//! Headless-ish snapshots: render a window to a PNG and describe it as JSON.
//!
//! This exists for agents. A coding agent that cannot see a window can still
//! answer "did the right page load, on the right platform, with the bridge up"
//! from the two files this module writes, without a human clicking through
//! anything.
//!
//! # What "headless" is allowed to mean here
//!
//! There is no display-free path to pixels: the engine draws into the widget it
//! was given. Measured on `webkit2gtk` 2.0.2, three attempts and two failures:
//!
//! | Attempt | Result |
//! | --- | --- |
//! | `gtk::init` with no `DISPLAY` | fails; there is no display-free GTK |
//! | window built but not `show_all()`ed, then `snapshot()` | **fails**; a zero-size webview has nothing to capture |
//! | window `show_all()`ed, then `snapshot()` | works |
//!
//! So a capture needs a window manager or an X server, and CI uses `xvfb-run`.
//! That is documented rather than hidden: a feature advertised as headless that
//! silently needs a display is exactly the bug this project writes comments
//! about. The *dump* has no such requirement, and answers most questions on its
//! own.
//!
//! # Enabling it
//!
//! `VOUSSOIR_SNAPSHOT=<path>` (PNG) and/or `VOUSSOIR_DUMP=<path>` (JSON). Both
//! are unset in every normal run, so a release binary carries the code and never
//! takes the branch. The capture is taken once, from the first window whose page
//! finished loading, and the process exits when the files are written.
//!
//! # Why this is a three-step dance
//!
//! Nothing here may block the UI thread: `snapshot` completes on the GTK main
//! context, so a blocking wait inside an event-loop callback would deadlock
//! against the very loop that has to deliver the result. So the work is two
//! hops through `UserEvent` — ask the page about itself, then capture — and the
//! third hop writes the files and exits.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::{platform_name, PAGE_ORIGIN, SHIM_PROTOCOL};

/// What the environment asked for, if anything.
///
/// Read once at startup rather than per window: the two variables describe the
/// run, not a window, and a second window finishing its load must not overwrite
/// the first capture.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Request {
    /// Where to write the PNG.
    pub(crate) png: Option<PathBuf>,
    /// Where to write the JSON description.
    pub(crate) dump: Option<PathBuf>,
}

impl Request {
    /// Parse the two variables.
    ///
    /// An empty value counts as absent: `VOUSSOIR_SNAPSHOT= app` is a shell
    /// mistake, not a request to write a file called `""`.
    pub(crate) fn from_env() -> Self {
        Self {
            png: non_empty(std::env::var_os("VOUSSOIR_SNAPSHOT")),
            dump: non_empty(std::env::var_os("VOUSSOIR_DUMP")),
        }
    }

    /// Whether anything was asked for. `false` means every hook below is a no-op.
    pub(crate) fn is_wanted(&self) -> bool {
        self.png.is_some() || self.dump.is_some()
    }
}

fn non_empty(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    value
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// What the page says about itself, which is the half a dump cannot invent.
///
/// Every field comes from inside the page, so a `false` bridge is a real result:
/// the shim refused this origin, or it never installed. Reading that from Rust
/// would answer a different question and could disagree with what a frontend
/// actually sees.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct PageFacts {
    pub(crate) title: String,
    pub(crate) bridge: bool,
    pub(crate) protocol: Option<u32>,
    pub(crate) platform: Option<String>,
    pub(crate) page_origin: Option<String>,
    /// Set when the probe script itself threw.
    pub(crate) error: Option<String>,
}

/// The script that asks the page about itself.
///
/// A JSON expression (not an IIFE), wrapped in parentheses: wry takes the value of
/// the last statement, and an expression is the form every engine evaluates the
/// same way. The answer is a JSON *string* containing JSON, which is what the one
/// callback argument holds.
pub(crate) const PROBE: &str = r#"JSON.stringify({
  title: String(document.title || ''),
  bridge: !!window.voussoir,
  protocol: window.voussoir ? window.voussoir.protocol : null,
  platform: window.voussoir ? window.voussoir.platform : null,
  pageOrigin: window.voussoir ? window.voussoir.pageOrigin : null
})"#;

/// Ask the page what it thinks it is, and hand the answer to `done`.
///
/// `done` runs synchronously on the UI thread when the engine answers, which is
/// the only thread that may start the capture.
pub(crate) fn probe(
    webview: &wry::WebView,
    done: impl FnOnce(PageFacts) + Send + 'static,
) -> Result<(), crate::Error> {
    // `evaluate_script_with_callback` takes an `Fn`, but a probe only ever answers
    // once: two answers would mean two captures under one set of file names. The
    // `Option` is what lets a one-shot callback live in an `Fn` closure.
    let done = std::sync::Mutex::new(Some(done));
    webview
        .evaluate_script_with_callback(PROBE, move |reply| {
            // One argument, already JSON text. Parsing in Rust rather than trusting
            // the shape keeps a platform that quotes it differently from turning
            // into a wrong dump.
            let value = serde_json::from_str::<Value>(&reply).unwrap_or(Value::Null);
            let Some(done) = done.lock().unwrap_or_else(|e| e.into_inner()).take() else {
                return;
            };
            done(parse_page(&[value]));
        })
        .map_err(|error| crate::Error::Runtime(error.to_string()))
}

/// Parse the probe's answer. Separate from the capture so it is testable with no
/// window at all.
///
/// wry hands the reply back as one argument, already a JSON *string* containing
/// JSON. A platform that adds quoting or an `undefined` string is handled by
/// reporting it rather than by panicking: this runs where a panic would abort the
/// process instead of failing a test.
pub(crate) fn parse_page(arguments: &[Value]) -> PageFacts {
    let Some(Value::String(text)) = arguments.first() else {
        return PageFacts {
            error: Some("the page returned no readable reply".to_owned()),
            ..PageFacts::default()
        };
    };
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return PageFacts {
            error: Some("the page's reply was not JSON".to_owned()),
            ..PageFacts::default()
        };
    };
    PageFacts {
        title: value["title"].as_str().unwrap_or_default().to_owned(),
        bridge: value["bridge"].as_bool().unwrap_or(false),
        protocol: value["protocol"].as_u64().map(|n| n as u32),
        platform: value["platform"].as_str().map(str::to_owned),
        page_origin: value["pageOrigin"].as_str().map(str::to_owned),
        error: value["error"].as_str().map(str::to_owned),
    }
}

/// One window plus what its page reported.
pub(crate) struct Captured {
    pub(crate) label: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) scale: f64,
    pub(crate) page: PageFacts,
}

impl Captured {
    /// The JSON written to `VOUSSOIR_DUMP`.
    ///
    /// Deliberately not an element tree: this shell does not own one, and a dump
    /// that invented a tree would be a second source of truth beside the DOM.
    /// What the page *cannot* say about itself is here — the platform, the real
    /// origin, the shim protocol — and everything else is the page's own report.
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "window": {
                "label": self.label,
                "title": self.page.title,
                "width": self.width,
                "height": self.height,
                "scale": self.scale,
            },
            "voussoir": {
                "platform": platform_name(),
                "pageOrigin": PAGE_ORIGIN,
                "protocol": SHIM_PROTOCOL,
                "version": env!("CARGO_PKG_VERSION"),
                // From the page, not from Rust: the shim installs per frame, and
                // an origin it refused looks exactly like a working shell from
                // out here.
                "bridge": self.page.bridge,
                "reported": {
                    "protocol": self.page.protocol,
                    "platform": self.page.platform,
                    "pageOrigin": self.page.page_origin,
                },
            },
            "error": self.page.error,
        })
    }

    /// The one line printed to stdout, so a driver waits for it instead of
    /// sleeping.
    pub(crate) fn line(&self, request: &Request) -> String {
        let mut fields = vec![format!("window={}", self.label)];
        if let Some(png) = &request.png {
            fields.push(format!("png={}", png.display()));
        }
        if let Some(dump) = &request.dump {
            fields.push(format!("dump={}", dump.display()));
        }
        fields.push(format!("bridge={}", self.page.bridge));
        format!("VOUSSOIR_SNAPSHOT {}", fields.join(" "))
    }
}

/// Write the files, print the line, and leave.
///
/// Exits rather than unwinding: this runs inside the event loop, and returning
/// would carry on a run whose only purpose was one capture. Exit code 1 means
/// something that was asked for could not be produced, which is what a driver
/// should treat as a failed run rather than as a file to retry.
pub(crate) fn finish(request: &Request, captured: &Captured, png: Option<&[u8]>) {
    let mut ok = true;
    if let Some(path) = &request.png {
        match png {
            Some(bytes) => ok &= write_file(path, bytes),
            // Asked for pixels and got none. Say which, rather than writing a
            // zero-byte file that looks like a capture.
            None => {
                eprintln!(
                    "voussoir: snapshot requested but no capture could be taken on this platform; \
                     {}",
                    match request.dump.is_some() {
                        true => "the dump still describes the window",
                        false => "request VOUSSOIR_DUMP as well for the window description",
                    }
                );
                ok = false;
            }
        }
    }
    if let Some(path) = &request.dump {
        let body =
            serde_json::to_vec_pretty(&captured.to_json()).unwrap_or_else(|_| b"{}".to_vec());
        ok &= write_file(path, &body);
    }
    println!("{}", captured.line(request));
    // Flushed explicitly: `println!` to a pipe is buffered, and a driver waiting
    // on this line would otherwise see nothing before the exit.
    let _ = std::io::stdout().flush();
    std::process::exit(if ok { 0 } else { 1 });
}

fn write_file(path: &Path, bytes: &[u8]) -> bool {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        if let Err(error) = std::fs::create_dir_all(parent) {
            eprintln!("voussoir: could not create {}: {error}", parent.display());
            return false;
        }
    }
    match std::fs::File::create(path).and_then(|mut file| file.write_all(bytes)) {
        Ok(()) => true,
        Err(error) => {
            eprintln!("voussoir: could not write {}: {error}", path.display());
            false
        }
    }
}

/// Start a capture, delivering the encoded PNG to `done`.
///
/// Asynchronous on purpose: `snapshot` completes on the GTK main context, so a
/// blocking wait here would stall the loop that has to deliver it. `done` runs
/// whenever the platform finishes, which is never before this returns.
///
/// Only GTK is wired: `WebViewExtUnix::webview()` hands back the
/// `webkit2gtk::WebView`, whose `snapshot` draws the visible region through
/// cairo. The other three platforms each need their own region capture
/// (`WKWebView.takeSnapshotWithConfiguration`, `ICoreWebView2CapturePreview`,
/// `View::draw` into a bitmap) and this shell carries no dependency for them, so
/// they report "no capture" and the dump still works.
#[cfg(voussoir_gtk_webview)]
pub(crate) fn capture(webview: &wry::WebView, done: impl FnOnce(Option<Vec<u8>>) + Send + 'static) {
    use webkit2gtk::WebViewExt as _;
    use wry::WebViewExtUnix;
    // `snapshot` wants an `Fn`, and the surface callback can only fire once, so
    // the handler is moved behind an `Option` the closure takes on its single run.
    let done = std::sync::Mutex::new(Some(done));

    webview.webview().snapshot(
        webkit2gtk::SnapshotRegion::Visible,
        webkit2gtk::SnapshotOptions::NONE,
        None::<&gtk::gio::Cancellable>,
        move |result| {
            let png = result.ok().and_then(|surface| {
                // `png` is a cairo-rs feature; the surface is a `gtk::cairo::Surface`,
                // which is the same crate gtk re-exports. Encoding goes through
                // `write_to_png` on that type, which needs the feature enabled on the
                // copy gtk already depends on.
                let mut bytes = Vec::new();
                surface.write_to_png(&mut bytes).ok()?;
                Some(bytes)
            });
            let Some(done) = done.lock().unwrap_or_else(|e| e.into_inner()).take() else {
                return;
            };
            done(png);
        },
    );
}

/// macOS and iOS: `takeSnapshotWithConfiguration` is reached through Objective-C
/// message sends, which belongs in a platform module rather than a line here.
#[cfg(not(voussoir_gtk_webview))]
pub(crate) fn capture(_webview: &wry::WebView, done: impl FnOnce(Option<Vec<u8>>) + 'static) {
    done(None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_variable_means_no_request() {
        let request = Request {
            png: non_empty(Some(std::ffi::OsString::from(""))),
            dump: non_empty(Some(std::ffi::OsString::from(""))),
        };
        assert_eq!(request, Request::default());
        assert!(!request.is_wanted());
    }

    #[test]
    fn a_set_variable_is_a_request() {
        let request = Request {
            png: non_empty(Some(std::ffi::OsString::from("/tmp/a.png"))),
            dump: None,
        };
        assert!(request.is_wanted());
    }

    #[test]
    fn the_page_report_survives_the_round_trip() {
        let reply = serde_json::json!(
            "{\"title\":\"Note\",\"bridge\":true,\"protocol\":3,\
             \"platform\":\"linux\",\"pageOrigin\":\"voussoir://localhost\"}"
        );
        let facts = parse_page(&[reply]);
        assert_eq!(facts.title, "Note");
        assert!(facts.bridge);
        assert_eq!(facts.protocol, Some(3));
        assert_eq!(facts.platform.as_deref(), Some("linux"));
        assert_eq!(facts.page_origin.as_deref(), Some("voussoir://localhost"));
        assert_eq!(facts.error, None);
    }

    #[test]
    fn a_page_with_no_shim_is_reported_not_guessed() {
        // The shim refused this origin, or never installed. Rust cannot tell the
        // difference, so the page's own answer is what the dump carries.
        let facts = parse_page(&[serde_json::json!("{\"title\":\"x\",\"bridge\":false}")]);
        assert!(!facts.bridge);
        assert_eq!(facts.protocol, None);
    }

    #[test]
    fn a_probe_that_threw_is_reported() {
        let facts = parse_page(&[serde_json::json!("{\"error\":\"boom\"}")]);
        assert_eq!(facts.error.as_deref(), Some("boom"));
        assert!(!facts.bridge);
    }

    #[test]
    fn an_unreadable_reply_becomes_an_error_rather_than_a_panic() {
        assert!(parse_page(&[]).error.is_some());
        assert!(parse_page(&[serde_json::json!(7)]).error.is_some());
        assert!(parse_page(&[serde_json::json!("not json")]).error.is_some());
    }

    #[test]
    fn the_dump_names_the_platform_and_the_page() {
        let captured = Captured {
            label: "index.html".to_owned(),
            width: 800,
            height: 600,
            scale: 1.0,
            page: PageFacts {
                title: "Notes".to_owned(),
                bridge: true,
                protocol: Some(3),
                platform: Some("linux".to_owned()),
                page_origin: Some("voussoir://localhost".to_owned()),
                error: None,
            },
        };
        let json = captured.to_json();
        assert_eq!(json["window"]["title"], "Notes");
        assert_eq!(json["voussoir"]["bridge"], true);
        assert_eq!(json["voussoir"]["protocol"], SHIM_PROTOCOL);
        assert_eq!(json["voussoir"]["reported"]["protocol"], 3);
        assert_eq!(json["error"], Value::Null);
    }

    #[test]
    fn the_line_names_both_paths_so_a_driver_can_find_them() {
        let request = Request {
            png: Some(PathBuf::from("/tmp/a.png")),
            dump: Some(PathBuf::from("/tmp/a.json")),
        };
        let captured = Captured {
            label: "main".to_owned(),
            width: 1,
            height: 1,
            scale: 1.0,
            page: PageFacts::default(),
        };
        let line = captured.line(&request);
        assert!(line.starts_with("VOUSSOIR_SNAPSHOT "), "{line}");
        assert!(line.contains("window=main"));
        assert!(line.contains("png=/tmp/a.png"));
        assert!(line.contains("dump=/tmp/a.json"));
        assert!(line.contains("bridge=false"));
    }

    #[test]
    fn the_probe_asks_for_everything_the_dump_reports() {
        for key in ["title", "bridge", "protocol", "platform", "pageOrigin"] {
            assert!(PROBE.contains(key), "the probe does not report {key}");
        }
    }
}
