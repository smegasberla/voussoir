//! Native file dialogs, exposed as commands.
//!
//! Registered names, all async because a dialog blocks until the user answers:
//!
//! | Command | Returns |
//! | --- | --- |
//! | `dialog.open` | `{"path": string} \| null` |
//! | `dialog.open_many` | `{"paths": string[]} \| null` |
//! | `dialog.save` | `{"path": string} \| null` |
//! | `dialog.folder` | `{"path": string} \| null` |
//!
//! Payload accepts `title`, `file_name`, `directory`, and `filters` as
//! `[{"name": "Images", "extensions": ["png"]}]`. Returning `null` on cancel is
//! deliberate: it is the one shape a caller can branch on without catching.
//!
//! Desktop goes through `rfd`, which blocks the calling thread. Android has no
//! `rfd`: the picker is an Android intent started by the activity, so the request
//! is described as JSON, handed to the activity, and the answer comes back as the
//! same JSON shape. Both entry points are async, so neither blocks the window.

use serde_json::Value;

#[cfg(any(not(any(target_os = "android", target_os = "ios")), test))]
use serde_json::json;

use crate::Registry;

/// Register every dialog command.
///
/// These are registered async (`add_async`) even though nothing here blocks:
/// the desktop backend blocks the worker thread it runs on, and the Android
/// backend parks that thread on a channel until the activity answers. Registering
/// them synchronously would hold the UI thread for the same interval.
pub fn register(registry: &mut Registry) {
    registry.add_async("dialog.open", |_app, payload| open(&payload, "open"));
    registry.add_async("dialog.open_many", |_app, payload| {
        open(&payload, "open_many")
    });
    registry.add_async("dialog.save", |_app, payload| open(&payload, "save"));
    registry.add_async("dialog.folder", |_app, payload| open(&payload, "folder"));
}

/// One dialog command, from payload to the reply the page sees.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn open(payload: &Value, kind: &str) -> Result<Value, crate::Error> {
    let dialog = build(payload);
    // A path that cannot cross the boundary is an error, not an empty path: the
    // page would otherwise see `{"path": ""}` as a successful pick of "".
    let picked: Option<Value> = match kind {
        "open" => match dialog.pick_file() {
            Some(path) => Some(json!({ "path": path_string(&path)? })),
            None => None,
        },
        "open_many" => match dialog.pick_files() {
            Some(paths) => Some(json!({
                "paths": paths
                    .iter()
                    .map(|path| path_string(path))
                    .collect::<Result<Vec<_>, _>>()?
            })),
            None => None,
        },
        "save" => match dialog.save_file() {
            Some(path) => Some(json!({ "path": path_string(&path)? })),
            None => None,
        },
        "folder" => match dialog.pick_folder() {
            Some(path) => Some(json!({ "path": path_string(&path)? })),
            None => None,
        },
        other => {
            return Err(crate::Error::Handler(format!(
                "unknown dialog kind {other}"
            )));
        }
    };
    Ok(picked.unwrap_or(Value::Null))
}

/// One dialog command, from payload to the reply the page sees.
///
/// The request is the payload plus the `kind`, which is what the activity needs
/// to know in order to pick the intent. The answer is already the reply shape,
/// because the activity builds it from the same JSON, so it is passed through
/// unread rather than parsed and rebuilt.
#[cfg(target_os = "android")]
fn open(payload: &Value, kind: &str) -> Result<Value, crate::Error> {
    let mut request = payload.clone();
    if let Some(object) = request.as_object_mut() {
        object.insert("kind".to_owned(), Value::from(kind));
    }
    let request = request.to_string();

    match crate::platform::pick_dialog(&request) {
        Ok(Some(response)) => bridge_reply(&response, kind),
        Ok(None) => Ok(Value::Null),
        Err(message) => Err(crate::Error::Handler(format!(
            "dialog {kind} failed: {message}"
        ))),
    }
}

/// iOS has no system file picker a webview app may drive: `UIDocumentPicker`
/// needs the Swift side, which the IOS-PLAN Phase I-3 template owns. The
/// honest answer until then is an error, not a silent `null` a page would
/// read as “nothing selected”.
#[cfg(target_os = "ios")]
fn open(_payload: &Value, kind: &str) -> Result<Value, crate::Error> {
    Err(crate::Error::Handler(format!(
        "dialog {kind} is not available on iOS yet"
    )))
}

/// Parse the reply the Android activity sends back.
///
/// The activity builds the same JSON shape the desktop backend produces, so a
/// reply that does not parse is a broken contract, not a cancel. Reporting it
/// keeps a stale or buggy Java side from looking like "the user picked nothing",
/// which is the one answer a page cannot diagnose.
#[cfg(any(test, target_os = "android"))]
fn bridge_reply(response: &str, kind: &str) -> Result<Value, crate::Error> {
    serde_json::from_str(response).map_err(|error| {
        crate::Error::Handler(format!(
            "dialog {kind} returned an unreadable answer: {error}"
        ))
    })
}

/// Paths cross the IPC boundary as UTF-8 strings. A path that is not valid UTF-8
/// cannot be represented in JSON at all, so it is refused: mangling it into a
/// replacement-character path would name a file the user did not pick.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn path_string(path: &std::path::Path) -> Result<String, crate::Error> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        crate::reporting::record(
            &format!("non-UTF-8 dialog path: {}", path.display()),
            "dialogs",
        );
        crate::Error::Handler(format!(
            "the selected path is not valid UTF-8 and cannot be sent to the page: {}",
            path.display()
        ))
    })
}

/// Apply the payload's optional settings to a fresh dialog.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn build(payload: &Value) -> rfd::FileDialog {
    let mut dialog = rfd::FileDialog::new();

    if let Some(title) = payload.get("title").and_then(Value::as_str) {
        dialog = dialog.set_title(title);
    }
    if let Some(name) = payload.get("file_name").and_then(Value::as_str) {
        dialog = dialog.set_file_name(name);
    }
    if let Some(dir) = payload.get("directory").and_then(Value::as_str) {
        dialog = dialog.set_directory(dir);
    }
    for (name, extensions) in parse_filters(payload) {
        if !extensions.is_empty() {
            dialog = dialog.add_filter(name, &extensions);
        }
    }

    dialog
}

/// Parse the payload's filter list without a dialog. Used by `build` and kept
/// separate so the parsing is testable without opening a window.
///
/// Android receives these too, as part of the JSON request: the activity maps an
/// extension to a MIME type, so the filter has to be described the same way.
pub fn parse_filters(payload: &Value) -> Vec<(String, Vec<String>)> {
    payload
        .get("filters")
        .and_then(Value::as_array)
        .map(|filters| {
            filters
                .iter()
                .map(|filter| {
                    let name = filter
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("Files")
                        .to_owned();
                    let exts = filter
                        .get("extensions")
                        .and_then(Value::as_array)
                        .map(|list| {
                            list.iter()
                                .filter_map(Value::as_str)
                                .map(|e| e.trim_start_matches('.').to_owned())
                                .collect()
                        })
                        .unwrap_or_default();
                    (name, exts)
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_are_parsed_and_dots_stripped() {
        let payload = json!({
            "filters": [
                { "name": "Images", "extensions": ["png", ".jpg"] },
                { "name": "All", "extensions": ["*"] },
                { "name": "Empty", "extensions": [] },
            ]
        });
        let filters = parse_filters(&payload);
        assert_eq!(filters.len(), 3);
        assert_eq!(filters[0].0, "Images");
        assert_eq!(filters[0].1, vec!["png", "jpg"]);
        assert_eq!(filters[1].1, vec!["*"]);
        assert!(filters[2].1.is_empty());
    }

    #[test]
    fn a_payload_without_filters_yields_none() {
        assert!(parse_filters(&json!({})).is_empty());
        assert!(parse_filters(&json!({ "filters": "nope" })).is_empty());
    }

    #[test]
    fn commands_are_registered_as_async() {
        let mut registry = Registry::new();
        register(&mut registry);
        for name in [
            "dialog.open",
            "dialog.open_many",
            "dialog.save",
            "dialog.folder",
        ] {
            assert!(registry.has(name), "{name} should be registered");
        }
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    #[test]
    fn paths_convert_to_utf8_strings() {
        let path = std::path::Path::new("/tmp/file.txt");
        assert_eq!(path_string(path).unwrap(), "/tmp/file.txt");
    }

    #[cfg(unix)]
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    #[test]
    fn a_non_utf8_path_is_an_error_not_an_empty_path() {
        use std::os::unix::ffi::OsStrExt;
        let path = std::path::Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xff"));
        assert!(path.to_str().is_none());
        let error = path_string(path).unwrap_err();
        assert!(
            error.to_string().contains("UTF-8"),
            "the page must learn why the pick failed: {error}"
        );
    }

    #[cfg(windows)]
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    #[test]
    fn a_non_utf8_path_is_an_error_not_an_empty_path() {
        use std::os::windows::ffi::OsStringExt;
        // A lone surrogate is not valid UTF-16 either, so `to_str` is None.
        let wide = vec![0x61u16, 0xD800, 0x62];
        let path = std::path::PathBuf::from(std::ffi::OsString::from_wide(&wide));
        let error = path_string(&path).unwrap_err();
        assert!(error.to_string().contains("UTF-8"), "{error}");
    }

    #[test]
    fn a_malformed_activity_reply_is_an_error_not_a_cancel() {
        let error = bridge_reply("not json", "open").unwrap_err();
        assert_eq!(error.kind(), "handler", "{error}");
        assert!(error.to_string().contains("unreadable"), "{error}");
        // A well-formed reply is passed through unchanged.
        let reply = bridge_reply(r#"{"path":"/tmp/a"}"#, "open").unwrap();
        assert_eq!(reply["path"], "/tmp/a");
    }

    #[cfg(target_os = "android")]
    #[test]
    fn the_request_carries_the_kind_for_the_activity() {
        // Android's picker is chosen from this field, so a command that lost it
        // would open the wrong intent.
        let request = open(&json!({ "title": "Pick" }), "folder")
            .err()
            .expect("no activity in a test process, so this must report rather than hang");
        assert!(request.to_string().contains("folder"));
    }
}
