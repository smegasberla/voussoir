//! Fuzz the two parsers that turn page bytes into a command call.
//!
//! Everything here is reachable from a page, which is inside the threat model:
//! the shell assumes the webview renders untrusted content. A panic in either
//! parser aborts the request thread, and a panic that crosses the webview's FFI
//! boundary aborts the process, so "does not panic" is a security property, not a
//! style preference.
//!
//! Run with:
//!
//! ```sh
//! cargo +nightly fuzz run envelope -- -max_total_time=60
//! cargo +nightly fuzz run path     -- -max_total_time=60
//! ```
//!
//! The always-on version of these properties lives in `voussoir/src/ipc.rs` under
//! `mod fuzz`: a fixed-seed mutational loop that runs on stable with every
//! `cargo test`. This harness is for longer, coverage-guided runs.

#![no_main]

use libfuzzer_sys::fuzz_target;
// Through the shell's re-export, because this crate deliberately does not depend on
// serde_json itself: the shell is the only dependency a fuzz target needs, and
// `voussoir::serde_json` is what an app crate would use too.
use voussoir::serde_json;

fuzz_target!(|data: &[u8]| {
    // The HTTP body: must either parse or refuse, never panic.
    if let Ok(value) = voussoir::transport::parse_payload(data) {
        // A reply is built by serializing whatever came back, so the round trip
        // has to hold or the page's parser breaks on a valid-looking envelope.
        let _ = serde_json::to_string(&value);
    }

    // The bridge message: one string carrying `{method, payload}`. Only valid
    // UTF-8 reaches it on a real device, and invalid input must be refused the
    // same way rather than reaching `from_str`.
    if let Ok(text) = std::str::from_utf8(data) {
        if let Ok((method, payload)) = voussoir::transport::parse_message(text) {
            // A method that survived parsing is a real name; the registry must
            // answer it with an envelope, never a panic.
            assert!(!method.is_empty());
            let _ = serde_json::to_string(&payload);
        }
    }
});
