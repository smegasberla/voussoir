//! Unit tests for the registry, the envelope and the decoders.

use super::*;

use std::collections::HashMap;
use std::sync::mpsc;

use serde_json::{json, Value};

use crate::{AppHandle, Error};

fn registry_with(names: &[&str]) -> Registry {
    let mut reg = Registry::new();
    for name in names {
        let owned = (*name).to_owned();
        reg.add(name, move |_h, payload| {
            Ok(json!({ "echo": payload, "name": owned }))
        });
    }
    reg
}

fn empty_handle() -> AppHandle {
    AppHandle::detached_for_tests()
}

#[test]
fn happy_path_wraps_in_ok() {
    let reg = registry_with(&["greet"]);
    let out = reg.dispatch(&empty_handle(), "greet", json!({ "a": 1 }));
    assert_eq!(out, r#"{"ok":{"echo":{"a":1},"name":"greet"}}"#);
}

#[test]
fn unknown_method_is_an_error_envelope_not_a_panic() {
    let reg = registry_with(&["greet"]);
    let out = reg.dispatch(&empty_handle(), "nope", Value::Null);
    assert!(
        out.contains("unknown command: nope") && out.contains(r#""kind":"unknown""#),
        "{out}"
    );
}

#[test]
fn allowlist_restricts_registered_commands() {
    let mut reg = registry_with(&["a", "b"]);
    reg.set_allowlist(Some(vec!["a".to_owned()]));
    assert_eq!(reg.callable(), vec!["a"]);
    assert!(reg
        .dispatch(&empty_handle(), "b", Value::Null)
        .contains("error"));
    assert!(reg
        .dispatch(&empty_handle(), "a", Value::Null)
        .contains(r#""ok""#));
}

fn dispatch_for_payload(
    reg: &Registry,
    label: Option<&str>,
    method: &str,
    payload: Value,
) -> String {
    let (tx, rx) = mpsc::channel();
    reg.dispatch_for_window(&empty_handle(), label, method, payload, move |body| {
        let _ = tx.send(body);
    });
    rx.recv().unwrap()
}

fn dispatch_for(reg: &Registry, label: Option<&str>, method: &str) -> String {
    let (tx, rx) = mpsc::channel();
    reg.dispatch_for_window(&empty_handle(), label, method, Value::Null, move |body| {
        let _ = tx.send(body);
    });
    rx.recv().unwrap()
}

#[test]
fn a_window_entry_narrows_to_a_strict_subset() {
    let mut reg = registry_with(&["a", "b"]);
    reg.set_window_allowlist(HashMap::from([(
        "preview".to_owned(),
        vec!["a".to_owned()],
    )]));

    // The preview window can only call "a"; the denial looks like an
    // unknown command, so the page cannot tell which commands exist.
    assert!(dispatch_for(&reg, Some("preview"), "a").contains(r#""ok""#));
    assert!(dispatch_for(&reg, Some("preview"), "b").contains(r#""kind":"unknown""#));

    // A window without a config entry keeps the global surface.
    assert!(dispatch_for(&reg, Some("main"), "b").contains(r#""ok""#));

    // App-level dispatch (menus, the Android fallback) is untouched.
    assert!(reg
        .dispatch(&empty_handle(), "b", Value::Null)
        .contains(r#""ok""#));
}

#[test]
fn a_window_entry_cannot_widen_the_global_allowlist() {
    let mut reg = registry_with(&["a", "b"]);
    reg.set_allowlist(Some(vec!["a".to_owned()]));
    reg.set_window_allowlist(HashMap::from([(
        "preview".to_owned(),
        vec!["a".to_owned(), "b".to_owned()],
    )]));
    assert!(dispatch_for(&reg, Some("preview"), "a").contains(r#""ok""#));
    assert!(dispatch_for(&reg, Some("preview"), "b").contains(r#""kind":"unknown""#));
}

#[test]
fn dispatch_latency_is_recorded_only_while_diagnostics_are_on() {
    let _guard = crate::diagnostics::test_guard();
    // One test, both states: a release build must not pay for the map, and a
    // verbose dev run must actually get numbers out of it.
    let reg = registry_with(&["greet"]);
    let handle = crate::AppHandle::detached_for_tests();

    crate::diagnostics::set_enabled(false);
    assert_eq!(reg.clear_latencies(), 0);
    let _ = reg.dispatch(&handle, "greet", Value::Null);
    assert!(
        reg.latencies().is_empty(),
        "a timed dispatch must cost nothing when diagnostics are off"
    );

    crate::diagnostics::set_enabled(true);
    let _ = reg.dispatch(&handle, "greet", Value::Null);
    let _ = reg.dispatch(&handle, "greet", Value::Null);
    let latencies = reg.latencies();
    crate::diagnostics::set_enabled(false);

    let (method, summary) = latencies.first().expect("a timed command is reported");
    assert_eq!(method, "greet");
    assert_eq!(summary.count, 2, "both calls are counted");
    assert_eq!(reg.clear_latencies(), 2);
    assert!(reg.latencies().is_empty());
}

#[test]
fn a_latency_report_is_sorted_slowest_first_and_names_each_command() {
    let _guard = crate::diagnostics::test_guard();
    let reg = registry_with(&["fast", "slow"]);
    let handle = crate::AppHandle::detached_for_tests();
    crate::diagnostics::set_enabled(true);
    let _ = reg.dispatch(&handle, "fast", Value::Null);
    let _ = reg.dispatch(&handle, "slow", Value::Null);
    let report = reg.latency_report();
    crate::diagnostics::set_enabled(false);

    let commands = report.as_array().expect("a report is an array");
    assert_eq!(commands.len(), 2);
    for command in commands {
        assert!(command["method"].is_string());
        assert!(command["p50_ms"].is_number());
        assert!(command["count"].as_u64().unwrap_or(0) >= 1);
    }
    // Sorted by mean, so a reader takes the first line as the one to fix.
    let means: Vec<u64> = commands
        .iter()
        .map(|command| command["mean_ms"].as_u64().unwrap_or(0))
        .collect();
    assert!(means.windows(2).all(|pair| pair[0] >= pair[1]), "{means:?}");
}

#[test]
fn a_runtime_narrowing_applies_to_the_window_it_names() {
    let reg = registry_with(&["greet", "save"]);
    // Registered after construction, which is what a window opened from a
    // command does: the registry is already shared by its handlers.
    reg.allow_window("preview", vec!["greet".to_owned()]);

    assert!(dispatch_for(&reg, Some("preview"), "greet").contains(r#""ok""#));
    assert!(
        dispatch_for(&reg, Some("preview"), "save").contains(r#""kind":"unknown""#),
        "a runtime lane must withhold what it does not name"
    );
    // Every other window, and app-level requests, keep the global surface.
    assert!(dispatch_for(&reg, None, "save").contains(r#""ok""#));
    assert!(dispatch_for(&reg, Some("main"), "save").contains(r#""ok""#));
}

#[test]
fn a_runtime_narrowing_cannot_widen_the_global_allowlist() {
    let mut reg = registry_with(&["a", "b"]);
    reg.set_allowlist(Some(vec!["a".to_owned()]));
    reg.allow_window("preview", vec!["a".to_owned(), "b".to_owned()]);

    assert!(dispatch_for(&reg, Some("preview"), "a").contains(r#""ok""#));
    assert!(dispatch_for(&reg, Some("preview"), "b").contains(r#""kind":"unknown""#));
}

#[test]
fn a_second_runtime_narrowing_cannot_widen_an_existing_one() {
    let reg = registry_with(&["greet", "save"]);
    reg.allow_window("preview", vec!["greet".to_owned()]);
    // Reusing a live label must intersect with the existing entry, not replace
    // it: this is the widening the label-reuse bug allowed.
    reg.allow_window("preview", vec!["greet".to_owned(), "save".to_owned()]);

    assert!(dispatch_for(&reg, Some("preview"), "greet").contains(r#""ok""#));
    assert!(
        dispatch_for(&reg, Some("preview"), "save").contains(r#""kind":"unknown""#),
        "a second call for the same label must not add commands"
    );
}

#[test]
fn a_second_runtime_narrowing_can_still_remove_more() {
    let reg = registry_with(&["greet", "save"]);
    reg.allow_window("preview", vec!["greet".to_owned(), "save".to_owned()]);
    reg.allow_window("preview", vec!["greet".to_owned()]);

    assert!(dispatch_for(&reg, Some("preview"), "greet").contains(r#""ok""#));
    assert!(
        dispatch_for(&reg, Some("preview"), "save").contains(r#""kind":"unknown""#),
        "narrowing further is always allowed"
    );
}

#[test]
fn config_and_runtime_narrowings_accumulate_rather_than_replace() {
    let mut reg = registry_with(&["a", "b"]);
    reg.set_window_allowlist(HashMap::from([(
        "from-config".to_owned(),
        vec!["a".to_owned()],
    )]));
    reg.allow_window("from-runtime", vec!["b".to_owned()]);

    assert_eq!(
        reg.narrowed_windows(),
        vec!["from-config".to_owned(), "from-runtime".to_owned()]
    );
    assert!(dispatch_for(&reg, Some("from-config"), "a").contains(r#""ok""#));
    assert!(dispatch_for(&reg, Some("from-runtime"), "b").contains(r#""ok""#));
}

#[test]
fn an_unlabelled_window_request_uses_the_global_allowlist() {
    let mut reg = registry_with(&["a", "b"]);
    reg.set_allowlist(Some(vec!["a".to_owned()]));
    assert!(dispatch_for(&reg, None, "a").contains(r#""ok""#));
    assert!(dispatch_for(&reg, None, "b").contains(r#""kind":"unknown""#));
}

#[test]
fn a_bodyless_typed_command_reads_as_an_empty_argument_struct() {
    #[derive(crate::serde::Deserialize, Default)]
    #[serde(default, crate = "crate::serde")]
    struct Args {
        title: Option<String>,
    }

    let mut reg = Registry::new();
    reg.add_typed("open_settings", |_h, args: Args| {
        Ok(json!({ "title": args.title }))
    });

    // No body at all: the page called `invoke("open_settings")`.
    assert_eq!(
        reg.dispatch(&empty_handle(), "open_settings", Value::Null),
        r#"{"ok":{"title":null}}"#
    );
    // An explicit empty object behaves the same.
    assert_eq!(
        reg.dispatch(&empty_handle(), "open_settings", json!({})),
        r#"{"ok":{"title":null}}"#
    );
    // A wrong *field type* is still a rejection, not a silent default.
    let parsed: Value = serde_json::from_str(&reg.dispatch(
        &empty_handle(),
        "open_settings",
        json!({ "title": 5 }),
    ))
    .unwrap();
    assert_eq!(parsed["error"]["kind"], "payload");
}

#[test]
fn a_bodyless_call_still_works_for_unit_and_value_arguments() {
    let mut reg = Registry::new();
    reg.add_typed("unit", |_h, _args: ()| Ok(json!("unit")));
    reg.add_typed("raw", |_h, args: Value| Ok(args));

    assert_eq!(
        reg.dispatch(&empty_handle(), "unit", Value::Null),
        r#"{"ok":"unit"}"#
    );
    // `Value` keeps receiving the real payload rather than being rewritten.
    assert_eq!(
        reg.dispatch(&empty_handle(), "raw", json!({ "a": 1 })),
        r#"{"ok":{"a":1}}"#
    );
    assert_eq!(
        reg.dispatch(&empty_handle(), "raw", Value::Null),
        r#"{"ok":null}"#
    );
}

#[test]
fn a_typed_command_round_trips_through_the_envelope() {
    #[derive(crate::serde::Deserialize, crate::serde::Serialize)]
    struct Args {
        name: String,
    }

    let mut reg = Registry::new();
    reg.add_typed("greet", |_h, args: Args| Ok(json!({ "hello": args.name })));

    let out = reg.dispatch(&empty_handle(), "greet", json!({ "name": "Lin" }));
    assert_eq!(out, r#"{"ok":{"hello":"Lin"}}"#);
}

#[test]
fn a_typed_command_rejects_a_wrong_shape_as_a_payload_error() {
    #[derive(crate::serde::Deserialize)]
    struct Args {
        #[allow(dead_code)]
        name: String,
    }

    let mut reg = Registry::new();
    reg.add_typed("greet", |_h, _args: Args| Ok(json!("hi")));

    let out = reg.dispatch(&empty_handle(), "greet", json!({ "nope": 1 }));
    let parsed: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(parsed["error"]["kind"], "payload", "{out}");
    assert_eq!(parsed["error"]["method"], "greet", "{out}");
}

#[test]
fn a_typed_command_still_respects_the_allowlist() {
    #[derive(crate::serde::Deserialize, crate::serde::Serialize)]
    struct Args {
        #[allow(dead_code)]
        name: String,
    }

    let mut reg = Registry::new();
    reg.add_typed("greet", |_h, args: Args| Ok(args));
    reg.set_allowlist(Some(vec!["other".to_owned()]));

    let out = reg.dispatch(&empty_handle(), "greet", json!({ "name": "Lin" }));
    assert!(out.contains(r#""kind":"unknown""#), "{out}");
}

#[test]
fn a_typed_command_can_still_fail_like_any_other() {
    #[derive(crate::serde::Deserialize)]
    struct Args {
        #[allow(dead_code)]
        name: String,
    }

    let mut reg = Registry::new();
    reg.add_typed("greet", |_h, _args: Args| {
        Err::<Value, _>(Error::Handler("nope".to_owned()))
    });

    let parsed: Value =
        serde_json::from_str(&reg.dispatch(&empty_handle(), "greet", json!({ "name": "L" })))
            .unwrap();
    assert_eq!(parsed["error"]["kind"], "handler");
    assert_eq!(parsed["error"]["message"], "nope");
}

#[test]
fn every_failure_kind_carries_its_method_and_a_distinct_kind() {
    let mut reg = Registry::new();
    reg.add("handler", |_h, _p| Err(Error::Handler("bad".to_owned())));
    reg.add("boom", |_h, _p| panic!("exploded"));
    reg.add_typed("typed", |_h, _p: String| Ok(_p));
    reg.add("runtime", |_h, _p| {
        Err::<Value, _>(Error::Runtime("loop".to_owned()))
    });
    reg.set_window_allowlist(HashMap::from([("preview".to_owned(), vec![])]));

    let cases: [(&str, &str, Option<&str>); 6] = [
        ("handler", "handler", None),
        ("boom", "panic", None),
        ("typed", "payload", None),
        ("runtime", "runtime", None),
        ("missing", "unknown", None),
        ("handler", "unknown", Some("preview")),
    ];
    for (method, expected, label) in cases {
        let body = match label {
            Some(label) => dispatch_for_payload(&reg, Some(label), method, json!(1)),
            None => dispatch_for_payload(&reg, None, method, json!(1)),
        };
        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["error"]["kind"], expected, "{method}: {body}");
        assert_eq!(parsed["error"]["method"], method, "{method}: {body}");
        assert!(parsed["error"]["message"].is_string(), "{method}: {body}");
    }
}

#[cfg(feature = "rt")]
#[test]
fn an_async_fn_runs_off_the_caller_thread_and_answers() {
    #[derive(crate::serde::Deserialize)]
    struct Args {
        ms: u64,
    }

    let caller = std::thread::current().id();
    let mut reg = Registry::new();
    reg.add_async_fn("wait", move |_h, args: Args| async move {
        tokio::time::sleep(std::time::Duration::from_millis(args.ms)).await;
        Ok(json!({ "off_caller_thread": std::thread::current().id() != caller }))
    });

    let (tx, rx) = mpsc::channel();
    reg.dispatch_deferred(&empty_handle(), "wait", json!({ "ms": 1 }), move |body| {
        let _ = tx.send(body);
    });
    let body = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
    assert_eq!(body, r#"{"ok":{"off_caller_thread":true}}"#);
}

#[cfg(feature = "rt")]
#[test]
fn an_async_fn_rejects_a_bad_payload_without_touching_the_runtime() {
    #[derive(crate::serde::Deserialize)]
    struct Args {
        #[allow(dead_code)]
        ms: u64,
    }

    let mut reg = Registry::new();
    reg.add_async_fn("wait", |_h, args: Args| async move { Ok(json!(args.ms)) });

    let (tx, rx) = mpsc::channel();
    reg.dispatch_deferred(&empty_handle(), "wait", json!("nope"), move |body| {
        let _ = tx.send(body);
    });
    let body = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
    let parsed: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["error"]["kind"], "payload", "{body}");
    assert_eq!(parsed["error"]["method"], "wait");
}

#[cfg(feature = "rt")]
#[test]
fn a_panicking_async_fn_reports_a_panic_instead_of_hanging_the_page() {
    #[derive(crate::serde::Deserialize)]
    struct Args {
        #[allow(dead_code)]
        ms: u64,
    }

    let mut reg = Registry::new();
    reg.add_async_fn("boom", |_h, _args: Args| async move {
        panic!("async exploded");
        #[allow(unreachable_code)]
        Ok::<Value, Error>(Value::Null)
    });

    let (tx, rx) = mpsc::channel();
    reg.dispatch_deferred(&empty_handle(), "boom", json!({ "ms": 1 }), move |body| {
        let _ = tx.send(body);
    });
    let body = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
    let parsed: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["error"]["kind"], "panic", "{body}");
    assert!(
        parsed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("async exploded"),
        "{body}"
    );
}

#[cfg(feature = "rt")]
#[test]
fn an_async_fn_respects_the_allowlist_like_every_other_command() {
    #[derive(crate::serde::Deserialize)]
    struct Args {
        #[allow(dead_code)]
        ms: u64,
    }

    let mut reg = Registry::new();
    reg.add_async_fn("wait", |_h, args: Args| async move { Ok(json!(args.ms)) });
    reg.set_allowlist(Some(vec!["other".to_owned()]));

    let (tx, rx) = mpsc::channel();
    reg.dispatch_deferred(&empty_handle(), "wait", json!({ "ms": 1 }), move |body| {
        let _ = tx.send(body);
    });
    let body = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
    assert!(body.contains(r#""kind":"unknown""#), "{body}");
}

#[cfg(feature = "rt")]
#[test]
fn many_concurrent_async_fns_all_answer() {
    #[derive(crate::serde::Deserialize)]
    struct Args {
        n: u64,
    }

    let mut reg = std::sync::Arc::new(Registry::new());
    std::sync::Arc::get_mut(&mut reg).unwrap().add_async_fn(
        "double",
        |_h, args: Args| async move { Ok(json!(args.n * 2)) },
    );

    let (tx, rx) = mpsc::channel();
    for n in 0..64u64 {
        let reg = std::sync::Arc::clone(&reg);
        let tx = tx.clone();
        let handle = empty_handle();
        std::thread::spawn(move || {
            reg.dispatch_deferred(&handle, "double", json!({ "n": n }), move |body| {
                let _ = tx.send(body);
            });
        });
    }
    drop(tx);

    let mut seen = std::collections::HashSet::new();
    for body in rx.iter().take(64) {
        let parsed: Value = serde_json::from_str(&body).unwrap();
        let value = parsed["ok"].as_u64().unwrap_or_else(|| panic!("{body}"));
        seen.insert(value / 2);
    }
    assert_eq!(seen.len(), 64, "every request must answer exactly once");
}

#[test]
fn handler_errors_become_error_envelopes() {
    let mut reg = Registry::new();
    reg.add("boom", |_h, _p| Err(Error::Handler("kaboom".into())));
    let out = reg.dispatch(&empty_handle(), "boom", Value::Null);
    let parsed: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(parsed["error"]["message"], "kaboom");
    assert_eq!(parsed["error"]["kind"], "handler");
}

#[test]
fn a_panicking_handler_reports_an_error_instead_of_killing_the_app() {
    let mut reg = Registry::new();
    reg.add("boom", |_h, _p| {
        let empty: Vec<u8> = Vec::new();
        let _ = empty[3];
        Ok(Value::Null)
    });
    let out = reg.dispatch(&empty_handle(), "boom", Value::Null);
    let parsed: Value = serde_json::from_str(&out).unwrap();
    let message = parsed["error"]["message"]
        .as_str()
        .expect("error envelope expected");
    assert!(message.starts_with("command panicked:"), "{message}");
    assert!(message.contains("index out of bounds"), "{message}");
}

#[test]
fn a_panicking_handler_does_not_poison_later_calls() {
    let mut reg = Registry::new();
    reg.add("boom", |_h, _p| panic!("kaboom"));
    reg.add("fine", |_h, _p| Ok(json!("still here")));

    assert!(reg
        .dispatch(&empty_handle(), "boom", Value::Null)
        .contains("error"));
    assert_eq!(
        reg.dispatch(&empty_handle(), "fine", Value::Null),
        r#"{"ok":"still here"}"#
    );
}

#[test]
fn async_handlers_respond_from_a_worker_thread() {
    let caller = std::thread::current().id();
    let mut reg = Registry::new();
    reg.add_async("slow", move |_h, _p| {
        assert_ne!(
            std::thread::current().id(),
            caller,
            "an async handler must not run on the caller's thread"
        );
        Ok(json!("done"))
    });

    let (tx, rx) = mpsc::channel();
    reg.dispatch_deferred(&empty_handle(), "slow", Value::Null, move |body| {
        let _ = tx.send(body);
    });
    assert_eq!(
        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
        r#"{"ok":"done"}"#
    );
}

#[test]
fn async_commands_are_refused_past_the_limit() {
    let mut reg = Registry::new();
    reg.set_async_limit(1);

    // Channels rather than sleeps: the first call can only be refused once
    // its slot is provably taken, and `started` is what proves it.
    let (started_tx, started_rx) = mpsc::channel();
    let started_tx = std::sync::Arc::new(std::sync::Mutex::new(started_tx));
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Arc::new(std::sync::Mutex::new(release_rx));

    reg.add_async("slow", move |_h, _p| {
        started_tx
            .lock()
            .expect("fresh lock")
            .send(())
            .expect("test listening");
        release_rx
            .lock()
            .expect("fresh lock")
            .recv()
            .expect("test releasing");
        Ok(json!("first"))
    });

    let (tx, rx) = mpsc::channel();
    reg.dispatch_deferred(&empty_handle(), "slow", Value::Null, move |body| {
        let _ = tx.send(body);
    });
    started_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("the first call must start");
    assert_eq!(reg.async_inflight(), 1);

    // The only slot is taken, so the second call is answered instead of
    // spawning. This is the DoS guard: without it the call would reserve
    // another thread stack.
    let body = reg.dispatch(&empty_handle(), "slow", Value::Null);
    let parsed: Value = serde_json::from_str(&body).expect("envelope");
    assert_eq!(parsed["error"]["kind"], "busy");
    assert!(
        parsed["error"]["message"]
            .as_str()
            .expect("message is text")
            .contains("at most 1"),
        "the refusal should name the limit: {body}"
    );

    release_tx.send(()).expect("test releasing");
    assert_eq!(
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("reply"),
        r#"{"ok":"first"}"#
    );
    assert_eq!(reg.async_inflight(), 0, "the slot must come back");
}

#[test]
fn a_panicking_worker_does_not_leak_its_slot() {
    let mut reg = Registry::new();
    reg.set_async_limit(1);
    reg.add_async("boom", |_h, _p| panic!("async kaboom"));

    let body = reg.dispatch(&empty_handle(), "boom", Value::Null);
    assert!(
        body.contains("panic"),
        "the page gets the panic envelope: {body}"
    );
    assert_eq!(
        reg.async_inflight(),
        0,
        "the permit is dropped on unwind, so one panic cannot wedge the app"
    );
}

#[test]
fn a_limit_of_zero_is_treated_as_one() {
    let mut reg = Registry::new();
    reg.set_async_limit(0);
    assert_eq!(
        reg.async_limit(),
        1,
        "a zero limit would make every async command permanently uncallable"
    );
}

#[test]
fn the_default_limit_keeps_ordinary_io_parallel() {
    assert!(
        Registry::new().async_limit() >= 4,
        "the floor is four, so a two-core target still overlaps I/O"
    );
}

#[test]
fn the_worker_limit_does_not_gate_sync_commands() {
    let mut reg = Registry::new();
    reg.set_async_limit(1);
    reg.add("ok", |_h, _p| Ok(json!("fine")));
    assert_eq!(
        reg.dispatch(&empty_handle(), "ok", Value::Null),
        r#"{"ok":"fine"}"#,
        "the bound is about thread stacks, not about the command surface"
    );
}

#[test]
fn a_panicking_async_handler_is_isolated_too() {
    let mut reg = Registry::new();
    reg.add_async("boom", |_h, _p| panic!("async kaboom"));

    let (tx, rx) = mpsc::channel();
    reg.dispatch_deferred(&empty_handle(), "boom", Value::Null, move |body| {
        let _ = tx.send(body);
    });
    let body = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    let parsed: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["error"]["message"], "command panicked: async kaboom");
    assert_eq!(parsed["error"]["kind"], "panic");
}

#[test]
fn error_messages_are_json_escaped() {
    let mut reg = Registry::new();
    reg.add("q", |_h, _p| Err(Error::Handler("say \"hi\"\nnow".into())));
    let out = reg.dispatch(&empty_handle(), "q", Value::Null);
    let parsed: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(parsed["error"]["message"], "say \"hi\"\nnow");
}

#[test]
fn endpoint_paths_are_recognised_and_decoded() {
    assert_eq!(
        method_from_path("/__voussoir/greet").as_deref(),
        Some("greet")
    );
    assert_eq!(
        method_from_path("/__voussoir/save%20file").as_deref(),
        Some("save file")
    );
    assert_eq!(method_from_path("/__voussoir/"), None);
    assert_eq!(method_from_path("/index.html"), None);
    assert_eq!(method_from_path("/"), None);
}

#[test]
fn a_multibyte_path_does_not_panic_the_decoder() {
    // Slicing the path string by byte offset used to land inside `€` and
    // abort the request-handling thread. The command cannot exist, so the
    // contract is "no command", not "panic".
    assert_eq!(
        method_from_path("/__voussoir/%\u{20ac}").as_deref(),
        Some("%€")
    );
    assert_eq!(
        method_from_path("/__voussoir/\u{20ac}").as_deref(),
        Some("€")
    );
    assert_eq!(method_from_path("/__voussoir/%"), Some("%".to_owned()));
    assert_eq!(method_from_path("/__voussoir/%4"), Some("%4".to_owned()));
    assert_eq!(
        method_from_path("/__voussoir/a%zz"),
        Some("a%zz".to_owned())
    );
}

#[test]
fn an_encoded_stand_in_for_an_unknown_method_is_refused_not_panicked() {
    // A path that decodes to a name nobody registered answers `unknown`
    // through the normal envelope, including a name with escaped bytes.
    let mut reg = Registry::new();
    reg.add("greet", |_h, _p| Ok(json!("hi")));
    let method = method_from_path("/__voussoir/%67reet").expect("decodes to greet");
    assert_eq!(method, "greet");
    assert_eq!(
        reg.dispatch(&empty_handle(), &method, Value::Null),
        r#"{"ok":"hi"}"#
    );

    let missing = method_from_path("/__voussoir/%00nope").expect("decodes");
    let body = reg.dispatch(&empty_handle(), &missing, Value::Null);
    assert!(body.contains(r#""kind":"unknown""#), "{body}");
}

#[test]
fn a_path_that_decodes_to_invalid_utf8_is_refused() {
    assert_eq!(method_from_path("/__voussoir/%ff"), None);
    assert_eq!(method_from_path("/__voussoir/%c3"), None);
}

#[test]
#[should_panic(expected = "was registered twice")]
fn a_duplicate_registration_fails_in_a_test() {
    // Last-wins is the behaviour, but a silent replacement is data loss: the
    // first handler's command stops existing and the second answers calls meant
    // for it. A test that registers one is reporting a bug in the app, so it
    // fails there rather than shipping.
    let mut reg = Registry::new();
    reg.add("dup", |_h, _p| Ok(json!("first")));
    reg.add("dup", |_h, _p| Ok(json!("second")));
}

#[test]
fn replacing_a_builtin_is_allowed_because_it_is_deliberate() {
    // `__voussoir_*` belongs to the shell, so an app claiming one is overriding it
    // on purpose. That is not the collision the strict check is for, and it must
    // stay possible: `register_builtins` runs after the app's commands.
    let mut reg = Registry::new();
    reg.add("__voussoir_version", |_h, _p| Ok(json!("mine")));
    reg.add("__voussoir_version", |_h, _p| Ok(json!("shell")));
    assert_eq!(
        crate::transport::respond(&reg, &empty_handle(), "__voussoir_version", Value::Null),
        r#"{"ok":"shell"}"#
    );
}

#[test]
fn an_outstanding_request_beyond_the_cap_is_refused() {
    // The worker limit bounds threads, not load: a page that loops on a fast
    // command keeps every slot busy forever. `pending` is the count the shell
    // refuses at, so the rate is set by the app rather than by the page.
    let mut reg = Registry::new();
    reg.set_async_limit(1);
    reg.set_max_pending(1);
    let (release, blocked) = std::sync::mpsc::channel::<()>();
    let blocked = std::sync::Mutex::new(blocked);
    reg.add_async("slow", move |_h, _p| {
        let _ = blocked.lock().unwrap().recv();
        Ok(json!("done"))
    });

    let (tx, rx) = mpsc::channel();
    reg.dispatch_deferred(&empty_handle(), "slow", Value::Null, move |body| {
        let _ = tx.send(body);
    });
    // Wait until the first request is genuinely inside the handler.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while reg.pending() == 0 && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }

    // A second request while the first is held is refused, not queued.
    let (tx2, rx2) = mpsc::channel();
    reg.dispatch_deferred(&empty_handle(), "slow", Value::Null, move |body| {
        let _ = tx2.send(body);
    });
    let body = rx2.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    let parsed: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["error"]["kind"], "busy", "{body}");

    release.send(()).unwrap();
    assert_eq!(
        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
        r#"{"ok":"done"}"#
    );
    assert_eq!(reg.pending(), 0, "the admission is released with the reply");
    assert_eq!(reg.async_inflight(), 0, "and so is the worker slot");
}

#[test]
fn a_pending_cap_of_zero_is_treated_as_one() {
    let mut reg = Registry::new();
    reg.set_max_pending(0);
    assert_eq!(
        reg.max_pending(),
        1,
        "zero would make async commands uncallable"
    );
}

#[test]
fn an_allowlist_typo_is_recorded_instead_of_silently_removing_the_command() {
    // `{"commands": ["gret"]}` currently withholds "greet" and says nothing, so
    // the app looks broken rather than misconfigured. The audit runs once, after
    // every handler exists, and names the command the author probably meant.
    let _guard = crate::diagnostics::test_guard();
    crate::diagnostics::clear();
    let mut reg = Registry::new();
    reg.add("greet", |_h, _p| Ok(serde_json::json!("hi")));
    reg.set_allowlist(Some(vec!["greet".to_owned(), "gret".to_owned()]));
    reg.audit_allowlist();

    let entries = crate::diagnostics::entries();
    let text = entries
        .iter()
        .map(|entry| entry.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("unknown command"), "no audit event: {text}");
    assert!(text.contains("gret"), "the typo should be named: {text}");
    // A name that does exist is not reported, so the signal stays worth reading.
    assert!(
        !text.contains(r#""command":"greet""#),
        "false positive: {text}"
    );
}

#[test]
fn a_window_allowlist_typo_is_recorded() {
    let _guard = crate::diagnostics::test_guard();
    crate::diagnostics::clear();
    let mut reg = Registry::new();
    reg.add("greet", |_h, _p| Ok(serde_json::json!("hi")));
    reg.set_window_allowlist(HashMap::from([(
        "preview".to_owned(),
        vec!["greet".to_owned(), "nope".to_owned()],
    )]));
    reg.audit_allowlist();

    let text = crate::diagnostics::entries()
        .iter()
        .map(|entry| entry.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("window allowlist names an unknown command"),
        "{text}"
    );
    assert!(text.contains("nope") && text.contains("preview"), "{text}");
}
