//! Fuzz the endpoint path decoder.
//!
//! `method_from_path` turns a page-controlled URL into a command name before
//! any allowlist check runs, so it is the first thing untrusted input touches. A
//! panic here aborts the request thread; the original implementation sliced the
//! path `&str` by byte index and did exactly that for `/__voussoir/%€`.
//!
//! Reached through `voussoir::transport`, which is the module that owns this seam:
//! the decoder turns a URL into a command name before anything else touches it.
//!
//! Run with:
//!
//! ```sh
//! cargo +nightly fuzz run path -- -max_total_time=60
//! ```
//!
//! CI runs both targets on a schedule (`.github/workflows/fuzz.yml`); the
//! always-on version is `voussoir/src/ipc/fuzz.rs` under `cargo test`.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Only valid UTF-8 pages can be served, so only valid UTF-8 reaches the
    // decoder in production. The harness still hands it anything, and requires a
    // refusal rather than a panic for the invalid case.
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };

    // Both shapes: the full path a page would request, and the bare suffix.
    let with_prefix = format!("/__voussoir/{text}");
    let decoded = voussoir::transport::method_from_path(&with_prefix);

    if let Some(method) = decoded {
        // A decoded name must be a usable command name: non-empty, and if it has
        // no escapes left it must survive a second pass unchanged.
        assert!(!method.is_empty(), "empty method");
        if !method.contains('%') {
            assert_eq!(
                voussoir::transport::method_from_path(&format!("/__voussoir/{method}")),
                Some(method),
                "decoding changed a name that had nothing to decode"
            );
        }
    }

    // A path without the prefix is never a command, whatever the input.
    assert!(
        voussoir::transport::method_from_path(text).is_none() || text.starts_with("/__voussoir/")
    );
});
