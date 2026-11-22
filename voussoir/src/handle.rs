//! The handle a command handler receives, and the events it can raise.
//!
//! Split out of `lib.rs` because it is the public surface an app touches most:
//! `emit`, `open_window`, `invoke_command`, and the tray and menu setters. It
//! depends on the loop only through the proxy type, so it is also the part that
//! can be read without knowing how a window is built.

use std::sync::Arc;

use serde_json::Value;
use tao::event_loop::EventLoopProxy;

use crate::windows::WindowOptions;
use crate::{live, windows, Error, Registry};

/// Everything that can arrive on the UI thread from another thread.
pub(crate) enum UserEvent {
    Emit {
        label: Option<String>,
        event: String,
        payload: String,
    },
    OpenWindow(WindowOptions),
    CloseWindow(String),
    /// A menu item was clicked.
    Menu(String),
    /// A tray item was clicked.
    Tray(String),
    /// An out-of-band signal, e.g. "open devtools".
    Signal(String),
    /// A tray change for the UI thread to apply.
    #[cfg(feature = "tray")]
    TrayUpdate(live::Change),
    /// A menu change for the UI thread to apply.
    #[cfg(feature = "menu")]
    MenuUpdate(live::Change),
    /// Run a script in every window (used to resolve bridge promises).
    Eval(String),
}
/// A handle passed to command handlers. Cheap to clone, safe to send to other
/// threads, and the only way to push data into the page or open a window.
#[derive(Clone)]
pub struct AppHandle {
    proxy: Option<EventLoopProxy<UserEvent>>,
    commands: Option<Arc<Registry>>,
    /// Labels of the open windows, kept in step with the event loop's map.
    open: windows::OpenWindows,
    /// The label an unnamed window falls back to, so `open_window` can resolve
    /// a name before the UI thread builds anything.
    default_entry: String,
    /// What the app declared for its tray and menu, so a runtime update can be
    /// validated here instead of failing on the UI thread later.
    #[cfg_attr(not(any(feature = "tray", feature = "menu")), allow(dead_code))]
    declared: Arc<live::Declared>,
}

impl AppHandle {
    pub(crate) fn new(
        proxy: EventLoopProxy<UserEvent>,
        commands: Arc<Registry>,
        open: windows::OpenWindows,
        default_entry: String,
        declared: Arc<live::Declared>,
    ) -> Self {
        Self {
            proxy: Some(proxy),
            commands: Some(commands),
            open,
            default_entry,
            declared,
        }
    }

    /// A handle with no event loop attached. Only useful with the shell's own
    /// unit tests; every method is a no-op or an error.
    pub fn detached_for_tests() -> Self {
        Self {
            proxy: None,
            commands: None,
            open: windows::OpenWindows::new(),
            default_entry: "index.html".to_owned(),
            declared: Arc::new(live::Declared::empty()),
        }
    }

    /// The shared label registry, for the event loop to keep in step.
    pub(crate) fn window_registry(&self) -> windows::OpenWindows {
        self.open.clone()
    }

    /// Labels of the windows that are open right now, in the order they opened.
    ///
    /// The list is maintained by the shell, so a handler on any thread can read
    /// it without touching a tao handle. A window counts from the moment it is
    /// requested, which is what lets a page open one and immediately address it.
    pub fn windows(&self) -> Vec<String> {
        self.open.labels()
    }

    /// Send an event to every window, invoking `voussoir.listen(event, fn)`.
    ///
    /// Thread-safe: the script runs on the UI thread on the next loop iteration.
    /// Events emitted before a page loads are dropped, so call this from a
    /// handler or `on_ready`, not from `main`.
    pub fn emit(&self, event: &str, payload: Value) {
        self.emit_to(None, event, payload);
    }

    /// Send an event to one window by label.
    pub fn emit_to(&self, label: Option<&str>, event: &str, payload: Value) {
        let Some(proxy) = &self.proxy else { return };
        let _ = proxy.send_event(UserEvent::Emit {
            label: label.map(str::to_owned),
            event: event.to_owned(),
            payload: payload.to_string(),
        });
    }

    pub(crate) fn emit_to_all(&self, event: &str, payload: Value) {
        self.emit(event, payload);
    }

    /// Open a window and return the label it is addressable by.
    ///
    /// Creation happens on the UI thread, so this returns immediately; the new
    /// page is announced through `on_ready`. The label is resolved here rather
    /// than there for two reasons: a caller (the page included) needs an answer
    /// synchronously to be able to `emit_to` the window it just asked for, and a
    /// second unnamed window must not get the same label as the first, which the
    /// reservation below guarantees even if two commands race.
    ///
    /// `WindowOptions::allowlist` is applied before the window exists, so its
    /// page can never make a call it will not be allowed to keep making.
    pub fn open_window(&self, options: WindowOptions) -> Result<String, Error> {
        let Some(proxy) = &self.proxy else {
            return Err(Error::Runtime("no event loop attached".to_owned()));
        };
        let mut options = options;
        let label = options
            .label
            .clone()
            .unwrap_or_else(|| self.open.unique_label(&self.default_entry));
        options.label = Some(label.clone());

        // An explicit label that is already open is refused here, before the
        // allowlist is touched: the loop would fail to build the window, but a
        // silent no-op would still have replaced a live window's narrowing
        // with whatever list the caller passed. Unnamed windows come from
        // `unique_label`, which never collides.
        if self.open.contains(&label) {
            return Err(Error::Runtime(format!(
                "window label `{label}` is already open"
            )));
        }

        if let (Some(commands), Some(allow)) = (&self.commands, options.allowlist.as_ref()) {
            commands.allow_window(&label, allow.clone());
        }

        proxy
            .send_event(UserEvent::OpenWindow(options))
            .map_err(|_| Error::Runtime("the app is shutting down".to_owned()))?;
        // Reserved before the loop runs, so a window that fails to build is the
        // only way this label goes back to being free.
        self.open.insert(&label);
        Ok(label)
    }

    /// Close a window by label. A no-op if it is already gone.
    pub fn close_window(&self, label: &str) {
        if let Some(proxy) = &self.proxy {
            let _ = proxy.send_event(UserEvent::CloseWindow(label.to_owned()));
        }
    }

    /// Retitle the tray icon at runtime.
    ///
    /// Validated before it is queued: an app that declared no tray, or a platform
    /// whose "tray" is a posted notification, gets an error now rather than a log
    /// line later. Returns as soon as the change is queued; the UI thread applies
    /// it on the next loop iteration.
    #[cfg(feature = "tray")]
    pub fn set_tray_tooltip(&self, text: &str) -> Result<(), Error> {
        self.update_tray(live::Wanted::TrayTooltip(text.to_owned()))
    }

    /// Check or uncheck a tray item declared with `.check()` or `.checked(..)`.
    #[cfg(feature = "tray")]
    pub fn set_tray_checked(&self, id: &str, checked: bool) -> Result<(), Error> {
        self.update_tray(live::Wanted::TrayChecked {
            id: id.to_owned(),
            checked,
        })
    }

    /// Raise the tray's own menu, for a page that has its own button.
    #[cfg(feature = "tray")]
    pub fn popup_tray(&self) -> Result<(), Error> {
        self.update_tray(live::Wanted::TrayPopup)
    }

    #[cfg(feature = "tray")]
    fn update_tray(&self, wanted: live::Wanted) -> Result<(), Error> {
        let change = self.declared.request(wanted)?;
        let Some(proxy) = &self.proxy else {
            return Err(Error::Runtime("no event loop attached".to_owned()));
        };
        proxy
            .send_event(UserEvent::TrayUpdate(change))
            .map_err(|_| Error::Runtime("the app is shutting down".to_owned()))
    }

    /// Check or uncheck a menu item declared with `.check()` or `.checked(..)`.
    ///
    /// The menu bar is built once, so this is the only way an item's tick can
    /// follow the app's state; passing a plain item's id is an error naming the
    /// reason rather than a tick that never appears.
    #[cfg(feature = "menu")]
    pub fn set_menu_checked(&self, id: &str, checked: bool) -> Result<(), Error> {
        let change = self.declared.request(live::Wanted::MenuChecked {
            id: id.to_owned(),
            checked,
        })?;
        let Some(proxy) = &self.proxy else {
            return Err(Error::Runtime("no event loop attached".to_owned()));
        };
        proxy
            .send_event(UserEvent::MenuUpdate(change))
            .map_err(|_| Error::Runtime("the app is shutting down".to_owned()))
    }

    /// Run another registered command as if the page had called it.
    ///
    /// Async handlers reply on their own thread, so this never blocks. The
    /// result is discarded, which is the right shape for menu- and tray-driven
    /// actions; use `invoke_command` when you need the value.
    pub fn run_command(&self, method: &str, payload: Value) {
        if let Some(commands) = &self.commands {
            commands.dispatch_deferred(self, method, payload, |_| {});
        }
    }

    /// Run a registered command and wait for its result.
    ///
    /// Do not call this from inside another handler for the same method.
    pub fn invoke_command(&self, method: &str, payload: Value) -> Result<Value, Error> {
        let Some(commands) = &self.commands else {
            return Err(Error::Runtime("no registry attached".to_owned()));
        };
        parse_envelope(&commands.dispatch(self, method, payload))
    }

    /// Evaluate a script in every window, on the UI thread.
    ///
    /// Commands answered from a worker thread must not touch a webview directly,
    /// so the script is queued as a user event like any other emit.
    pub(crate) fn eval_all(&self, script: &str) {
        let Some(proxy) = &self.proxy else { return };
        let _ = proxy.send_event(UserEvent::Eval(script.to_owned()));
    }

    /// Open devtools on every window. Needs the `devtools` feature in release
    /// builds; always available in debug builds.
    pub fn open_devtools(&self) {
        self.emit_signal("devtools");
    }

    /// Internal bridge for crate modules that need to wake the loop.
    pub(crate) fn emit_signal(&self, signal: &str) {
        let Some(proxy) = &self.proxy else { return };
        // Menu and tray ids share this entry point but not their event name, so
        // the prefix decides which one fires; a single `signal` event would make
        // a page unable to tell a tray click from anything else on the platform.
        let event = if let Some(id) = signal.strip_prefix("menu:") {
            UserEvent::Menu(id.to_owned())
        } else if let Some(id) = signal.strip_prefix("tray:") {
            UserEvent::Tray(id.to_owned())
        } else {
            UserEvent::Signal(signal.to_owned())
        };
        let _ = proxy.send_event(event);
    }
}
/// Turn a response envelope back into a value, for `invoke_command`.
///
/// The reply is parsed rather than trusted: a handler that answers with something
/// that is not an envelope is a bug on this side of the bridge, and it should be
/// reported as a runtime error instead of surfacing as a confusing `ok` of the
/// wrong shape.
pub(crate) fn parse_envelope(body: &str) -> Result<Value, Error> {
    let value: Value = serde_json::from_str(body)
        .map_err(|e| Error::Runtime(format!("malformed response: {e}")))?;
    match (value.get("ok"), value.get("error")) {
        (_, Some(error)) => {
            // The structured envelope carries {message, kind}; the plain-string
            // form is still accepted so an older reply rejects with readable text.
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| error.as_str())
                .unwrap_or("command failed");
            Err(Error::Handler(message.to_owned()))
        }
        (Some(ok), None) => Ok(ok.clone()),
        _ => Err(Error::Runtime("malformed response envelope".to_owned())),
    }
}
