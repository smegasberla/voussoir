//! The table, the allowlists, and the worker bound.
//!
//! One place answers "may this page call this command": `permitted_for`. Every
//! other method here either fills the table or describes the bound, so the
//! policy is readable without following dispatch.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::handler::Handler;
use super::worker::{default_max_pending, default_worker_limit};

/// Name -> handler table plus the optional config allowlist.
pub struct Registry {
    pub(super) handlers: HashMap<String, Handler>,
    pub(super) allow: Option<Vec<String>>,
    /// Per-window allowlists keyed by window label, from `voussoir.json`'s
    /// `windows` map or from `WindowOptions::allowlist`. A listed window's
    /// surface is the intersection of its list with the global allowlist, so a
    /// config entry can only narrow.
    ///
    /// Shared behind a lock because a window opened *at runtime* registers its
    /// own narrowing after the registry was already moved into the `Arc` the
    /// handlers hold. Without that, a window created from a command would only
    /// be narrowable by pre-declaring its label in a config file.
    pub(super) window_allow: std::sync::Arc<std::sync::RwLock<HashMap<String, Vec<String>>>>,
    /// The hot-reloaded commands library. Present only in `voussoir dev` builds;
    /// a release build never compiles it.
    #[cfg(feature = "hot-reload")]
    pub(super) hot: Option<std::sync::Arc<crate::hot::HotLib>>,
    /// Dispatch latency samples per method, filled only while diagnostics are
    /// on. Behind an `Arc` so an async dispatch can record from its worker
    /// thread without borrowing the registry.
    pub(super) latency: std::sync::Arc<std::sync::Mutex<HashMap<String, Vec<u64>>>>,
    /// How many worker-thread commands may be running at once.
    ///
    /// Without a bound, each `add_async` call reserves a thread stack, so a page
    /// that loops on one slow allowlisted command could reserve memory until the
    /// process dies. That page is inside the threat model: the shell assumes the
    /// webview renders untrusted content. Past the limit a call is refused with
    /// [`Error::Busy`] instead of spawning, which keeps the app alive and tells
    /// the page to back off.
    pub(super) worker_limit: usize,
    /// How many are running now. Behind an `Arc` so the permit a worker holds can
    /// release itself on drop, including when the handler panics.
    pub(super) worker_inflight: std::sync::Arc<AtomicUsize>,
    /// How many async requests may be outstanding at once, threads or not.
    ///
    /// The worker limit bounds stacks, which is not a bound on load: a page that
    /// loops `invoke` on a fast command keeps every slot busy indefinitely, and
    /// each request still holds a socket and a promise until it is answered.
    /// This is the number the shell refuses at, so the rate a page can impose is
    /// set by the app rather than by the page.
    pub(super) max_pending: usize,
    /// How many async requests are outstanding now, queued or running.
    pub(super) pending: std::sync::Arc<AtomicUsize>,
}
impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}
impl Registry {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
            allow: None,
            latency: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
            window_allow: std::sync::Arc::new(std::sync::RwLock::new(HashMap::new())),
            worker_limit: default_worker_limit(),
            worker_inflight: std::sync::Arc::new(AtomicUsize::new(0)),
            max_pending: default_max_pending(default_worker_limit()),
            pending: std::sync::Arc::new(AtomicUsize::new(0)),
            #[cfg(feature = "hot-reload")]
            hot: None,
        }
    }

    /// Change how many `add_async` commands may run at once.
    ///
    /// The default is the machine's parallelism, at least four. Raise it when the
    /// commands are I/O-bound and a burst of concurrent calls is normal; lower it
    /// on a memory-constrained target. A value of 0 is treated as 1, because a
    /// limit of zero would make every async command permanently uncallable.
    pub fn set_async_limit(&mut self, limit: usize) {
        self.worker_limit = limit.max(1);
    }

    /// The current worker-thread limit, for diagnostics and tests.
    pub fn async_limit(&self) -> usize {
        self.worker_limit
    }

    /// How many worker-thread commands are running right now.
    pub fn async_inflight(&self) -> usize {
        self.worker_inflight.load(Ordering::SeqCst)
    }

    /// Cap how many async requests may be outstanding at once.
    ///
    /// Separate from [`Self::set_async_limit`] because the two bound different
    /// things: that one is how many threads may run, this one is how many
    /// requests the shell will hold at all. Lower this when the threat is a page
    /// that loops on a fast command and never lets the queue drain. A value of 0
    /// is treated as 1 for the same reason the worker limit is.
    pub fn set_max_pending(&mut self, limit: usize) {
        self.max_pending = limit.max(1);
    }

    /// The current cap on outstanding async requests.
    pub fn max_pending(&self) -> usize {
        self.max_pending
    }

    /// How many async requests are outstanding right now.
    pub fn pending(&self) -> usize {
        self.pending.load(Ordering::SeqCst)
    }

    /// Attach the hot-reloaded commands library (`voussoir dev`). Static handlers
    /// keep priority; the library only ever answers names no static handler
    /// claimed, and only within the config allowlist like everything else.
    #[cfg(feature = "hot-reload")]
    pub(crate) fn set_hot(&mut self, hot: std::sync::Arc<crate::hot::HotLib>) {
        self.hot = Some(hot);
    }

    pub fn set_allowlist(&mut self, allow: Option<Vec<String>>) {
        self.allow = allow;
    }

    /// Attach per-window allowlists keyed by window label.
    ///
    /// Additive: a label declared in `voussoir.json` keeps its entry, and a runtime
    /// window adds or replaces only its own.
    pub fn set_window_allowlist(&mut self, allow: HashMap<String, Vec<String>>) {
        let mut current = self
            .window_allow
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for (label, list) in allow {
            current.insert(label, list);
        }
    }

    /// Narrow one window's surface at runtime.
    ///
    /// Called when a window with `WindowOptions::allowlist` is created, so the
    /// restriction exists before the page inside it can call anything. It is the
    /// same intersection as a config entry: a runtime list can only ever remove
    /// commands, never add one the app withheld. The guarantee is enforced here,
    /// not only at the caller: a second call for the same label intersects with
    /// the existing entry instead of replacing it, so reusing a live label can
    /// never widen that window's surface.
    pub fn allow_window(&self, label: &str, commands: Vec<String>) {
        let mut allow = self
            .window_allow
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match allow.get(label) {
            Some(existing) => {
                let narrowed: Vec<String> = existing
                    .iter()
                    .filter(|name| commands.contains(*name))
                    .cloned()
                    .collect();
                allow.insert(label.to_owned(), narrowed);
            }
            None => {
                allow.insert(label.to_owned(), commands);
            }
        }
    }

    /// The window labels with a narrowing entry, for diagnostics and tests.
    pub fn narrowed_windows(&self) -> Vec<String> {
        let mut labels: Vec<String> = self
            .window_allow
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .keys()
            .cloned()
            .collect();
        labels.sort_unstable();
        labels
    }

    /// Report allowlist names that no handler can ever answer.
    ///
    /// A typo in `voussoir.json` (`{"commands": ["gret"]}`) currently removes the
    /// command the author meant to keep and says nothing, so the app looks
    /// broken rather than misconfigured. Only a name that is neither a static
    /// handler nor answerable by the hot library is reported: with `hot-reload`
    /// the library's exports are resolved at call time, so it cannot be asked
    /// what it contains, and guessing would produce false alarms on every dev
    /// build.
    ///
    /// Called once from `App::run`, after the builtins are registered, because
    /// that is the first moment the full table exists.
    pub(crate) fn audit_allowlist(&self) {
        // A typo is only actionable when the name is unknown to everything that
        // could answer it. Under hot-reload, any name might be the library's,
        // which cannot be asked what it exports.
        #[cfg(feature = "hot-reload")]
        if self.hot.is_some() {
            return;
        }
        for name in self.allow.iter().flatten() {
            if !self.handlers.contains_key(name) {
                crate::diagnostics::record(
                    crate::diagnostics::Kind::App,
                    "allowlist names an unknown command",
                    serde_json::json!({ "command": name }),
                );
            }
        }
        // The global list and the window lists are independent: an app with no
        // `commands` key can still narrow one window, and that is exactly the
        // case a stray early return would have skipped.
        let allow = self
            .window_allow
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for (label, list) in allow.iter() {
            for name in list {
                if !self.handlers.contains_key(name) {
                    crate::diagnostics::record(
                        crate::diagnostics::Kind::App,
                        "window allowlist names an unknown command",
                        serde_json::json!({ "window": label, "command": name }),
                    );
                }
            }
        }
    }

    /// Whether any handler is registered under `method`, regardless of the
    /// allowlist.
    ///
    /// Registration is global and last-wins, so this is what a plugin checks
    /// before claiming a name: deferring to an app's own handler is the
    /// difference between a plugin and a hijack.
    pub fn contains(&self, method: &str) -> bool {
        self.handlers.contains_key(method)
    }

    /// Whether `method` is registered and allowed, globally or for `label`.
    ///
    /// A window with a config entry can only call what is in *both* the global
    /// allowlist and its own list, so a per-window entry can never widen what
    /// the app already allows.
    pub(super) fn permitted_for(&self, method: &str, label: Option<&str>) -> bool {
        if !self.permitted(method) {
            return false;
        }
        let label = match label {
            Some(label) => label,
            None => return true,
        };
        let allow = self
            .window_allow
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match allow.get(label) {
            Some(list) => list.iter().any(|allowed| allowed == method),
            None => true,
        }
    }
}
