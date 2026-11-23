//! iOS bootstrap for Voussoir.
//!
//! The desktop shell owns its event loop because it owns `main`. On iOS *the
//! process* owns it, but not through Rust: the Xcode project's Swift entry
//! point starts the app, and tao's `EventLoop::run` is what calls
//! `UIApplicationMain` with its own `AppDelegate`. That shapes this crate:
//!
//! - the app's Swift code calls one exported function in the *app* crate as
//!   the first thing the process does, before any UIKit object exists (tao
//!   asserts `UIApplication` is not yet created);
//! - that function hands an `App` to [`run`], which blocks for the lifetime
//!   of the app, exactly like `App::run` on desktop;
//! - the exported `#[no_mangle]` symbols themselves live in the app crate,
//!   not here, for the same reason Android's JNI exports do: their names and
//!   signatures belong to the app's package, and keeping them there is what
//!   lets `voussoir init` scaffold them.
//!
//! Page requests do not cross this crate. wry's `WKURLSchemeHandler` path
//! means the page is served the same `voussoir://localhost` way as desktop, so
//! the envelope, registry, allowlists, CSP and navigation policy are the
//! shared ones; whether iOS keeps that transport or needs the message bridge
//! is what the IOS-PLAN.md spike settles, not something this crate decides.
//!
//! This crate deliberately does not invent a lifecycle model. Backgrounding
//! and foregrounding become the same `lifecycle` events Android emits, and a
//! command can be interrupted by process death exactly as the docs say.

#![cfg(target_os = "ios")]

/// Re-exported so an app needs only `voussoir-ios` in its manifest.
pub use tao;
pub use voussoir;
pub use wry;

use std::sync::Arc;

use voussoir::{App, AppHandle, Registry};

/// Return the live runtime, if the app has started.
///
/// Forwarded rather than kept here: `App::run` publishes the registry and
/// handle it actually built, and a second copy would answer with a registry
/// that has no commands in it. One publisher, one answer.
pub fn runtime() -> Option<(&'static Arc<Registry>, &'static AppHandle)> {
    voussoir::runtime()
}

/// Start a Voussoir app from the iOS entry point.
///
/// Call this from the exported function the app's Swift entry invokes, and
/// only from there: tao's `EventLoop::run` calls `UIApplicationMain` on iOS,
/// so this must run before the application object exists and it never
/// returns while the process is alive.
///
/// Nothing is published here. `App::run` hands its own registry and handle to
/// `voussoir::platform` before the loop starts, and [`runtime`] reads them back;
/// the shell cannot be duplicated by the crate that starts it without the
/// webview and the native side disagreeing about which commands exist.
pub fn run(app: App) {
    if let Err(error) = app.run() {
        // Silently dropping this made two very different failures identical: a
        // run that never got a window and a page that loaded and stayed quiet
        // both ended with the entry marker and then nothing. NSLog rather than
        // stdout because the simulator console is the only channel iOS leaves.
        log(&format!("voussoir: run failed: {error}"));
        // NSLog goes to the system log, which `simctl launch --console-pty` does
        // not show. stdout does, and that is the channel the verify reads.
        println!("VOUSSOIR_IOS_RUN failed: {error}");
    }
}

/// Write a line to the system log via `NSLog`.
///
/// Kept private and tiny like its Android twin: it exists so iOS bootstrap
/// diagnostics are visible at the point where stdout is not yet wired up,
/// and the console output the simulator shows is what `voussoir ios verify`
/// greps for.
pub fn log(message: &str) {
    __ns_log(message);
}

fn __ns_log(message: &str) {
    use std::os::raw::c_char;
    extern "C" {
        fn NSLog(format: *const c_char, ...);
    }
    // A format string from data could contain `%`, so it is passed as the
    // sole argument of a fixed `%s` format.
    let format = b"%s\0";
    let mut text = message.as_bytes().to_vec();
    text.push(0);
    // SAFETY: both pointers are NUL-terminated and outlive the call.
    unsafe {
        NSLog(
            format.as_ptr() as *const c_char,
            text.as_ptr() as *const c_char,
        );
    }
}
