//! The shell's unit tests.
//!
//! Split from `lib.rs` so the crate root is a map of the shell rather than a map
//! plus a thousand lines of assertions. Nothing here needs a window: the shim is a
//! string, the envelope is a string, and `AppHandle::detached_for_tests` supplies
//! the handle the rest needs.

use std::sync::Arc;

use serde_json::{json, Value};
use tao::keyboard::{KeyCode, ModifiersState};

use crate::handle::parse_envelope;
use crate::shell::{build_shim_with, invoke_base};
use crate::DEFAULT_COMMAND_TIMEOUT_MS;

/// The shim at the default deadline, which is what nearly every assertion here
/// is about; `build_shim_with` is used directly where the deadline is the point.
fn build_shim(dev_origin: Option<&str>) -> String {
    build_shim_with(dev_origin, DEFAULT_COMMAND_TIMEOUT_MS)
}
use crate::{
    content_type, lookup, register_builtins, transport, AppHandle, AssetTable, Config, Error,
    Fired, Registry, WindowOptions, APP_ORIGIN, PAGE_ORIGIN, SHIM_PROTOCOL,
};

use super::*;

const INDEX: &[u8] = b"<!doctype html>hi";
static ASSETS: AssetTable = &[("index.html", INDEX)];

#[test]
fn shim_carries_the_app_origin_and_any_dev_origin() {
    let shim = build_shim(None);
    assert!(shim.contains(APP_ORIGIN), "app origin must be allowed");
    assert!(
        !shim.contains("__ALLOWED_ORIGINS__"),
        "placeholder must be filled"
    );
    assert!(!shim.contains("__INVOKE_BASE__"));

    let shim = build_shim(Some("http://127.0.0.1:5177"));
    assert!(shim.contains("http://127.0.0.1:5177"));
    assert!(shim.contains(APP_ORIGIN));
}

#[test]
fn shim_names_the_origin_the_page_is_actually_served_from() {
    // On Windows and Android wry serves the page from the rewritten http
    // origin; a shim built for the custom scheme there would allowlist an
    // origin the page never reports and post to an endpoint that is not
    // intercepted, which is exactly how the WebView2 harness failed.
    let shim = build_shim(None);
    assert!(
        shim.contains(PAGE_ORIGIN),
        "page origin must be allowlisted"
    );
    assert!(shim.contains(&format!("\"{}\"", invoke_base(false))));
    let rewritten = cfg!(any(target_os = "android", target_os = "windows"));
    assert_eq!(shim.contains("http://voussoir.localhost"), rewritten);

    // In dev the page comes from the dev server, so a relative base would be
    // resolved against that server; on Windows the absolute rewritten URL is
    // the only spelling that still reaches the interceptor.
    if cfg!(target_os = "windows") {
        assert_eq!(invoke_base(true), "http://voussoir.localhost/__voussoir/");
        assert!(build_shim(Some("http://127.0.0.1:5177")).contains(invoke_base(true)));
    }
}

#[test]
fn shim_uses_the_bridge_wherever_a_protocol_post_cannot_answer() {
    // The reply route is a property of the platform, not a build detail: the
    // shim either posts to the custom protocol or hands the call to wry's IPC
    // handler, and picking wrong is a promise that never settles. This pins the
    // choice next to the reasons, so adding a platform has to decide out loud.
    let bridged = cfg!(any(target_os = "android", target_os = "ios"));
    let shim = build_shim(None);
    assert_eq!(
        shim.contains("const bridge = true;"),
        bridged,
        "the shim's transport must match where a protocol POST can deliver a body"
    );
}

#[test]
fn shim_survives_a_document_with_no_root_element() {
    // The shim is injected at document start, where WKWebView and WebView2 have
    // neither `document.head` nor `document.documentElement`. Touching one threw
    // and the whole script died before `window.voussoir` was assigned, so the
    // page's own call failed with a ReferenceError and no request was ever made.
    // A viewport hint must never be able to take the API down with it.
    let shim = build_shim(None);
    assert!(
        shim.contains("if (document.documentElement) addViewport();"),
        "the viewport hint must wait for the root element instead of assuming it"
    );
    assert!(
        shim.contains(r#"document.addEventListener("DOMContentLoaded", addViewport"#),
        "a start-time shim must defer to DOMContentLoaded"
    );
}

#[test]
fn the_shim_announces_its_protocol_version() {
    let shim = build_shim(None);
    assert!(
        !shim.contains("__SHIM_PROTOCOL__"),
        "the placeholder must be substituted"
    );
    assert!(
        shim.contains(&format!("protocol: {}", SHIM_PROTOCOL)),
        "the page must be able to read the shim protocol"
    );
    // A version is a number the page can compare, never a string.
    const { assert!(SHIM_PROTOCOL >= 2, "version 2 added windows()") };
}

#[test]
fn the_shim_exposes_windows_and_chrome_helpers() {
    let shim = build_shim(None);
    // A page opens a window, then needs the list of what is open and the
    // ability to tick a tray or menu item without re-declaring anything.
    for member in ["windows", "tray", "menu", "pageOrigin"] {
        assert!(shim.contains(member), "the shim must expose {member}");
    }
    assert!(shim.contains("__voussoir_windows"));
    assert!(shim.contains("__voussoir_tray_checked"));
    assert!(shim.contains("__voussoir_tray_tooltip"));
    assert!(shim.contains("__voussoir_menu_checked"));
}

#[test]
fn envelopes_round_trip_through_invoke_command_parsing() {
    assert_eq!(
        parse_envelope(r#"{"ok":{"a":1}}"#).unwrap(),
        json!({ "a": 1 })
    );
    assert!(parse_envelope(r#"{"error":"nope"}"#).is_err());
    assert!(parse_envelope("not json").is_err());
}

#[test]
fn assets_are_looked_up_by_full_extension_not_prefix() {
    assert_eq!(lookup(ASSETS, "/").unwrap().0, "index.html");
    assert!(lookup(ASSETS, "/nope.css").is_none());
    assert_eq!(content_type("a.js"), "text/javascript; charset=utf-8");
    assert_eq!(content_type("a.css"), "text/css; charset=utf-8");
}

#[test]
fn devtools_shortcut_is_platform_appropriate() {
    #[cfg(not(target_os = "macos"))]
    {
        assert!(is_devtools_shortcut(KeyCode::F12, ModifiersState::empty()));
        assert!(!is_devtools_shortcut(KeyCode::F11, ModifiersState::empty()));
    }
    #[cfg(target_os = "macos")]
    {
        let mods = ModifiersState::SUPER | ModifiersState::ALT;
        assert!(is_devtools_shortcut(KeyCode::KeyI, mods));
        assert!(!is_devtools_shortcut(KeyCode::KeyI, ModifiersState::SUPER));
    }
}

#[test]
fn both_transports_share_the_envelope() {
    // The mobile bridge and the desktop protocol must not be able to answer
    // differently for the same command.
    let mut registry = Registry::new();
    registry.add("echo", |_app, payload| Ok(payload));

    let via_protocol = transport::respond(
        &registry,
        &AppHandle::detached_for_tests(),
        "echo",
        transport::parse_payload(br#"{"a":1}"#).unwrap(),
    );

    let (method, payload) =
        transport::parse_message(r#"{"method":"echo","payload":{"a":1}}"#).unwrap();
    let via_bridge = transport::respond(
        &registry,
        &AppHandle::detached_for_tests(),
        &method,
        payload,
    );

    assert_eq!(via_protocol, via_bridge);
    assert_eq!(via_protocol, r#"{"ok":{"a":1}}"#);
}

#[test]
fn an_empty_body_is_null_and_junk_is_rejected() {
    assert_eq!(transport::parse_payload(b"").unwrap(), Value::Null);
    assert!(transport::parse_payload(b"not json").is_err());
    assert_eq!(
        transport::parse_payload(br#"{"x":1}"#).unwrap(),
        json!({ "x": 1 })
    );
}

#[test]
fn bridge_messages_need_a_method() {
    assert!(transport::parse_message(r#"{"payload":{}}"#).is_err());
    assert!(transport::parse_message("nope").is_err());
    let (method, payload) = transport::parse_message(r#"{"method":"m"}"#).unwrap();
    assert_eq!(method, "m");
    assert_eq!(payload, Value::Null);
}

#[derive(serde::Deserialize)]
struct ConformanceArgs {
    n: u32,
}

/// The same requests through both transports, side by side.
///
/// The transports already share `transport::respond`, so this guards the
/// seam rather than reimplementing it: every case goes through the desktop
/// protocol path and the mobile bridge path and must produce an identical
/// body. Before this, a per-platform CI job going red was the first signal
/// that the two had drifted.
#[test]
fn the_two_transports_answer_identically() {
    let handle = AppHandle::detached_for_tests();

    let mut registry = Registry::new();
    registry.add("echo", |_app, payload| Ok(payload));
    registry.add("fail", |_app, _payload| {
        Err(Error::Handler("nope".to_owned()))
    });
    registry.add("boom", |_app, _payload| panic!("conformance boom"));
    registry.add_typed("typed", |_app, args: ConformanceArgs| Ok(json!(args.n)));
    registry.add("withheld", |_app, _payload| Ok(json!("secret")));
    registry.set_allowlist(Some(
        ["echo", "fail", "boom", "typed"]
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
    ));

    let cases: &[(&str, &str, &[u8], &str)] = &[
        (
            "a payload round-trips",
            "echo",
            br#"{"a":1}"#,
            r#"{"ok":{"a":1}}"#,
        ),
        ("an absent body is null", "echo", b"", r#"{"ok":null}"#),
        (
            "a typed reply keeps its shape",
            "typed",
            br#"{"n":7}"#,
            r#"{"ok":7}"#,
        ),
        (
            "a malformed typed payload is a payload error",
            "typed",
            br#"{"n":"x"}"#,
            r#""kind":"payload""#,
        ),
        (
            "a handler error is an envelope",
            "fail",
            b"",
            r#""kind":"handler""#,
        ),
        ("a panic is contained", "boom", b"", r#""kind":"panic""#),
        (
            "an unknown command is unknown",
            "nope",
            b"",
            r#""kind":"unknown""#,
        ),
        (
            "a withheld command is unknown too, not forbidden",
            "withheld",
            b"",
            r#""kind":"unknown""#,
        ),
    ];

    for (label, method, body, expected) in cases {
        let payload = transport::parse_payload(body).expect("the case body must parse");
        let via_protocol = transport::respond(&registry, &handle, method, payload.clone());

        let message = json!({ "method": method, "payload": payload }).to_string();
        let (bridged_method, bridged_payload) =
            transport::parse_message(&message).expect("the bridge message must parse");
        let via_bridge = transport::respond(&registry, &handle, &bridged_method, bridged_payload);

        assert_eq!(via_protocol, via_bridge, "{label}: transports disagree");
        assert!(
            via_protocol.contains(expected),
            "{label}: expected {expected} in {via_protocol}"
        );
    }
}

/// A malformed body is an HTTP 400 on the desktop transport and a dropped
/// message on the bridge (there is no status code over `postMessage`). Both
/// must refuse rather than dispatch something.
#[test]
fn a_malformed_body_is_refused_on_both_transports() {
    assert!(transport::parse_payload(b"not json").is_err());
    assert!(transport::parse_message("not json").is_err());
    assert!(transport::parse_message(r#"{"payload":{}}"#).is_err());
}

/// The origin the shim reports must be one the navigation policy accepts.
///
/// `pageOrigin` exists so a page need not guess the spelling, which matters
/// because Windows rewrites it. If the shim reported an origin the policy
/// refuses, every absolute URL a page built would be blocked.
#[test]
fn the_reported_page_origin_is_navigable() {
    assert!(
        security::navigation_allowed(&format!("{PAGE_ORIGIN}/"), None),
        "the shim reports {PAGE_ORIGIN}, which the policy must accept"
    );
    assert!(build_shim(None).contains(&format!("\"{PAGE_ORIGIN}\"")));
}

#[test]
fn builtin_commands_are_registered() {
    let mut registry = Registry::new();
    register_builtins(&mut registry, &Config::default());
    for name in [
        "__voussoir_version",
        "__voussoir_reports",
        "__voussoir_diagnostics",
        "__voussoir_open_window",
        "__voussoir_windows",
        "__voussoir_close_window",
        "__voussoir_devtools",
    ] {
        assert!(registry.has(name), "{name} should exist");
    }
    // `full` carries `storage`, so the store is reachable from the page
    // whenever the shell advertises that feature set.
    // The runtime tray/menu surface is part of `full`, so a page can rely on
    // these names when the shell advertises the feature.
    #[cfg(feature = "tray")]
    for name in [
        "__voussoir_tray_tooltip",
        "__voussoir_tray_checked",
        "__voussoir_tray_popup",
    ] {
        assert!(registry.has(name), "{name} should exist under tray");
    }
    #[cfg(feature = "menu")]
    assert!(
        registry.has("__voussoir_menu_checked"),
        "__voussoir_menu_checked should exist under menu"
    );
    #[cfg(feature = "storage")]
    for name in [
        "storage.get",
        "storage.set",
        "storage.delete",
        "storage.keys",
        "storage.path",
    ] {
        assert!(registry.has(name), "{name} should exist under storage");
    }
}

#[test]
fn lifecycle_hooks_fire_exactly_once_per_window() {
    let fired = Fired::default();
    assert!(fired.first_ready("main"));
    assert!(
        !fired.first_ready("main"),
        "a reload must not fire ready twice"
    );
    assert!(fired.first_ready("settings"));
    assert!(fired.first_close("main"));
    assert!(
        !fired.first_close("main"),
        "a duplicate close must be ignored"
    );
    assert!(fired.first_close("settings"));
    // The two hooks track labels independently: a window can become ready
    // and close, and neither set may consume the other's entry.
    let independent = Fired::default();
    assert!(independent.first_ready("main"));
    assert!(independent.first_close("main"));
}

#[test]
fn the_ready_hook_receives_the_window_label() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let app = App::new().on_ready({
        let seen = Arc::clone(&seen);
        move |_app, label| seen.lock().unwrap().push(label.to_owned())
    });
    let ready = app.ready.expect("hook stored");
    let handle = AppHandle::detached_for_tests();
    ready(&handle, "main");
    ready(&handle, "settings");
    assert_eq!(*seen.lock().unwrap(), vec!["main", "settings"]);
}

#[test]
fn the_close_hook_is_optional_and_labelled() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let app = App::new().on_window_close({
        let seen = Arc::clone(&seen);
        move |_app, label| seen.lock().unwrap().push(label.to_owned())
    });
    let closed = app.closed.expect("hook stored");
    closed(&AppHandle::detached_for_tests(), "preview");
    assert_eq!(*seen.lock().unwrap(), vec!["preview"]);

    // Registering neither hook is the default and must stay valid.
    let bare = App::new();
    assert!(bare.ready.is_none());
    assert!(bare.closed.is_none());
}

#[test]
fn builtins_respect_an_explicit_allowlist() {
    let mut registry = Registry::new();
    let config = Config {
        commands: Some(vec!["__voussoir_version".to_owned()]),
        ..Config::default()
    };
    register_builtins(&mut registry, &config);
    registry.set_allowlist(config.commands.clone());

    assert!(registry.has("__voussoir_version"));
    assert!(!registry.has("__voussoir_reports"));
}

#[test]
fn window_commands_are_reachable_from_the_page() {
    let handle = AppHandle::detached_for_tests();
    // Detached: opening a window reports the missing loop instead of lying.
    assert!(handle.open_window(WindowOptions::default()).is_err());
    // Closing something that cannot exist is a silent no-op.
    handle.close_window("nope");
}

#[test]
fn empty_responses_are_never_served_as_success() {
    // A 404 must not carry an empty body that the page could parse as JSON.
    assert!(parse_envelope(r#"{"ok":null}"#).is_ok());
    assert!(parse_envelope("").is_err());
}

#[test]
fn the_shim_carries_the_configured_deadline() {
    // A fetch with no deadline waits forever, so a hung handler was
    // indistinguishable from a slow one. The number comes from `voussoir.json`,
    // which is why it must be injected rather than hardcoded in the shim.
    let shim = build_shim_with(None, 1234);
    assert!(
        shim.contains("const defaultTimeout = 1234;"),
        "the configured deadline must reach the page"
    );
    assert!(!shim.contains("__TIMEOUT__"), "the token must be replaced");

    // 0 is the documented escape hatch for commands that legitimately run long.
    assert!(build_shim_with(None, 0).contains("const defaultTimeout = 0;"));
}

#[test]
fn the_shim_frees_the_request_when_the_deadline_passes() {
    // Rejecting alone would leave the socket open for as long as the handler
    // blocked, which is the leak the deadline is meant to stop.
    let shim = build_shim(None);
    assert!(
        shim.contains("AbortController"),
        "the fetch must be abortable"
    );
    assert!(
        shim.contains("controller.abort()"),
        "and aborted on timeout"
    );
    assert!(
        shim.contains(r#"kind: "timeout""#),
        "a page must be able to branch on the deadline rather than on message text"
    );
    // The bridge has no abort: it has to drop the pending entry instead, or a
    // late reply would settle a promise that already rejected.
    assert!(shim.contains("pending.delete(id)"));
}

#[test]
fn the_page_can_read_the_deadline_it_is_subject_to() {
    let shim = build_shim_with(None, 5000);
    assert!(
        shim.contains("timeout: defaultTimeout"),
        "exposing it lets a page show or adapt to the value instead of guessing"
    );
}
