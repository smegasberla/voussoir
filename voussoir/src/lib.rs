//! Voussoir: a minimal desktop app shell.
//!
//! One window by default, the OS webview, and a single generic JSON bridge:
//!
//! ```no_run
//! static ASSETS: voussoir::AssetTable = &[
//!     ("index.html", include_bytes!("../examples/hello/index.html")),
//!     ("main.js", include_bytes!("../examples/hello/main.js")),
//! ];
//!
//! fn main() -> Result<(), voussoir::Error> {
//!     voussoir::App::new()
//!         .config(include_str!("../examples/hello/voussoir.json"))
//!         .assets(ASSETS)
//!         .command("greet", |_app, payload| {
//!             let name = payload["name"].as_str().unwrap_or("world");
//!             Ok(voussoir::serde_json::json!(format!("Hello, {name}!")))
//!         })
//!         .run()
//! }
//! ```
//!
//! The page calls `voussoir.invoke("greet", {name: "you"})` and awaits the result.
//! Rust calls `app.emit("event", payload)` to push data the other way.
//!
//! No plugin system, no capability files, no codegen: a command exists iff it
//! was registered here, in Rust.
//!
//! # What a "real app" needs
//!
//! Each integration is a cargo feature, so a minimal app pays nothing for it:
//!
//! | Feature | Gives you |
//! | --- | --- |
//! | `menu` | native menu bar with Cut/Copy/Paste/Quit, plus app items |
//! | `tray` | system tray icon with a context menu (implies `menu`) |
//! | `dialogs` | native open / save / folder dialogs as commands |
//! | `updater` | update check, download, verify and install |
//! | `devtools` | devtools in release builds, bound to F12 / Cmd-Alt-I |
//! | `full` | menu + dialogs + updater + devtools |
//!
//! Always present, because leaving them out is how apps break in production:
//! handler panic isolation, navigation lockdown, CSP, crash reporting, and
//! multi-window support.

mod app;
mod assets;
mod builtins;
mod config;
pub mod diagnostics;
mod dispatch;
mod error;
mod event_loop;
mod handle;
mod hooks;
#[cfg(feature = "hot-reload")]
mod hot;
mod ipc;
mod platform;
pub mod plugin;
mod reporting;
mod security;
mod shell;
pub mod transport;

pub mod windows;

#[cfg(feature = "dialogs")]
pub mod dialogs;
pub mod live;
#[cfg(feature = "menu")]
pub mod menu;
#[cfg(feature = "rt")]
pub mod rt;
pub mod state;
#[cfg(feature = "storage")]
pub mod storage;
#[cfg(feature = "tray")]
pub mod tray;
#[cfg(feature = "updater")]
pub mod updater;
/// Patch application for the updater; the generator lives in the CLI.
#[cfg(feature = "updater")]
pub mod delta;
/// Undoing an update whose build never reaches its first window.
#[cfg(all(feature = "updater", not(any(target_os = "android", target_os = "ios"))))]
pub mod rollback;

pub use app::platform_name;
pub use app::App;
pub use assets::{content_type, lookup, AssetTable};
use builtins::register_builtins;
pub use config::{Config, DEFAULT_COMMAND_TIMEOUT_MS};
pub use error::Error;
pub use handle::AppHandle;
pub(crate) use handle::UserEvent;
pub(crate) use hooks::{BuilderFactory, CloseHook, Fired};
pub use ipc::Registry;
/// Android only: the outer crate has to hand the activity in when the OS
/// creates one, because everything that starts an intent lives on it, and to
/// hand back the answer when the picker it started returns.
#[cfg(target_os = "android")]
pub use platform::{complete_dialog, fail_dialog, init_activity};
pub use platform::{data_dir, lifecycle, menu_clicked, runtime, tray_clicked};
pub use plugin::Plugin;
pub use security::{
    content_security_policy, navigation_allowed, APP_ORIGIN, PAGE_ORIGIN, REWRITTEN_APP_ORIGIN,
};
/// Re-exported so an app crate needs exactly one dependency: `voussoir`.
///
/// `serde` brings the derives a typed command's types need
/// (`#[derive(serde::Serialize, serde::Deserialize)]`), `serde_json` the
/// `Value` the raw API speaks.
pub use serde;
pub use serde_json;
#[cfg(feature = "tray")]
pub(crate) use shell::build_tray;
pub(crate) use shell::{
    build_factory, build_webview, is_devtools_shortcut, open_devtools, print_latency_summary,
};
#[cfg(feature = "menu")]
pub(crate) use shell::{build_menu, menu_command_map};
pub use windows::WindowOptions;

/// The scheme pages are served from in production.
pub const SCHEME: &str = "voussoir";

/// The shim's own protocol version, exposed to the page as `voussoir.protocol`.
///
/// The shell version answers "which build am I running"; this answers "which
/// shim surface can I rely on". Frontend code feature-detects with one number
/// instead of probing for methods, and the package's `voussoir.d.ts` declares the
/// same value, so a mismatch is a visible bug rather than a runtime `undefined`.
///
/// Bumping it is a documented change: it happens when the shim gains a member
/// or changes one's meaning, never for an internal fix.
///
/// 3 added `pageOrigin`, so a page that wants the real spelling of its own origin
/// asks `protocol >= 3` and falls back to a relative URL on an older shell.
pub const SHIM_PROTOCOL: u32 = 3;

#[cfg(test)]
mod tests;
