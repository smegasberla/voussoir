//! One request, from method name to response body.
//!
//! This is where the threat model is enforced: the method is looked up, the
//! window is checked against its narrowing, the worker bound is claimed, and the
//! handler runs isolated. Reading it should not require scrolling past
//! registration, which is why it is its own module.

use serde_json::Value;

use super::envelope::{error_envelope, error_envelope_for, ok_or_error};
use super::handler::{run_isolated, run_isolated_async, Handler};
use super::metrics::record_latency;
use super::worker::{claim_admission, claim_worker_slot};
use crate::{AppHandle, Error};

impl super::registry::Registry {
    /// Run one request and produce the response body.
    ///
    /// Errors are converted into the envelope rather than propagated, so the
    /// page always gets valid JSON it can act on, and a handler failure never
    /// takes down the window.
    pub fn dispatch(&self, handle: &AppHandle, method: &str, payload: Value) -> String {
        let (tx, rx) = std::sync::mpsc::channel();
        self.dispatch_deferred(handle, method, payload, move |body| {
            let _ = tx.send(body);
        });
        rx.recv()
            .unwrap_or_else(|_| error_envelope(&Error::Handler("no response".to_owned())))
    }

    /// Run one request, passing the response body to `done`.
    ///
    /// Sync handlers call `done` before returning; async handlers call it from
    /// the worker thread that runs them, which is what lets the protocol layer
    /// answer after the window has already moved on.
    pub fn dispatch_deferred<F>(&self, handle: &AppHandle, method: &str, payload: Value, done: F)
    where
        F: FnOnce(String) + Send + 'static,
    {
        self.dispatch_gated(handle, None, method, payload, done)
    }

    /// Dispatch a request that claims to come from the window `label`.
    ///
    /// The page cannot be trusted to name its window, so `label` comes from the
    /// webview the request arrived on, not from the payload. `None` means the
    /// request is app-level (a menu item, an Android bridge fallback) and gets
    /// the global allowlist.
    pub fn dispatch_for_window<F>(
        &self,
        handle: &AppHandle,
        label: Option<&str>,
        method: &str,
        payload: Value,
        done: F,
    ) where
        F: FnOnce(String) + Send + 'static,
    {
        self.dispatch_gated(handle, label, method, payload, done)
    }

    pub(super) fn dispatch_gated<F>(
        &self,
        handle: &AppHandle,
        label: Option<&str>,
        method: &str,
        payload: Value,
        done: F,
    ) where
        F: FnOnce(String) + Send + 'static,
    {
        // The allowlists gate everything below, the hot library included:
        // `commands` and per-window lists in voussoir.json can only ever narrow
        // what is reachable.
        if !self.permitted_for(method, label) {
            return done(error_envelope_for(
                method,
                &Error::UnknownMethod(method.to_owned()),
            ));
        }
        // Timed around the handler, not the whole transport: this is the number
        // `voussoir dev --verbose` reports as command latency. Reading a `bool` is
        // free and `Instant::now` is skipped entirely when diagnostics are off.
        let timed = crate::diagnostics::enabled();

        match self.handlers.get(method) {
            Some(Handler::Sync(handler)) => {
                let started = timed.then(std::time::Instant::now);
                let outcome = run_isolated(handler, handle, payload);
                if let Some(started) = started {
                    self.note_latency(method, started.elapsed());
                }
                done(ok_or_error(method, outcome))
            }
            Some(Handler::Async(handler)) => {
                // Two bounds, because they answer different floods. `pending`
                // caps how much the shell will hold at all, so a page looping on
                // a fast command cannot keep every slot saturated forever; the
                // worker slot then caps how many stacks exist at once.
                //
                // Both are refusals rather than queues: a queued request still
                // holds the page's promise open and still costs a thread once a
                // slot frees, so under a flood the honest answer is "busy,
                // retry" and the bound stays a bound.
                let Some(admission) = claim_admission(&self.pending, self.max_pending) else {
                    return done(error_envelope_for(method, &Error::Busy(self.max_pending)));
                };
                let Some(permit) = claim_worker_slot(&self.worker_inflight, self.worker_limit)
                else {
                    return done(error_envelope_for(method, &Error::Busy(self.worker_limit)));
                };
                let handler = std::sync::Arc::clone(handler);
                let handle = handle.clone();
                let method = method.to_owned();
                let samples = std::sync::Arc::clone(&self.latency);
                // The callback has to move into the thread, but a thread that
                // cannot be created must still answer the page. Holding it behind
                // an `Arc<Mutex<Option<_>>>` lets the failed-spawn path take it
                // back: the closure is dropped without running, so the slot still
                // holds it.
                let answer: std::sync::Arc<std::sync::Mutex<Option<F>>> =
                    std::sync::Arc::new(std::sync::Mutex::new(Some(done)));
                let in_thread = std::sync::Arc::clone(&answer);
                let method_for_error = method.clone();
                let spawned = std::thread::Builder::new()
                    .name(format!("voussoir:{method}"))
                    .spawn(move || {
                        // Held for the whole handler: dropped on return or unwind,
                        // so a panic cannot leak a slot.
                        let started = timed.then(std::time::Instant::now);
                        let outcome = run_isolated_async(&handler, handle, payload);
                        if let Some(started) = started {
                            record_latency(&samples, &method, started.elapsed());
                        }
                        // Both guards go before the reply is handed over, not
                        // after it. The outcome exists, so the handler is no
                        // longer running and the shell is no longer holding the
                        // request; the handoff is neither. Releasing them after
                        // `answer` would make the counts change only once the
                        // caller had already been woken, which is a race for
                        // anything that observes them, the tests included.
                        drop(permit);
                        drop(admission);
                        if let Some(answer) = in_thread
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .take()
                        {
                            answer(ok_or_error(&method, outcome));
                        }
                    });
                // A thread that cannot be created is the same failure as
                // saturation, and answering keeps the page's promise resolvable.
                if let Err(error) = spawned {
                    if let Some(answer) = answer
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take()
                    {
                        answer(error_envelope_for(
                            &method_for_error,
                            &Error::Runtime(format!("could not start a worker thread: {error}")),
                        ));
                    }
                }
            }
            #[cfg(feature = "rt")]
            Some(Handler::Deferred(handler)) => {
                // A deferred handler owns its completion, so it can answer from
                // any task and hold the page's promise just as long as an async
                // one. Without claiming here, this arm bypassed both bounds and
                // a page could spawn unbounded tasks through it.
                let Some(admission) = claim_admission(&self.pending, self.max_pending) else {
                    return done(error_envelope_for(method, &Error::Busy(self.max_pending)));
                };
                let Some(permit) = claim_worker_slot(&self.worker_inflight, self.worker_limit)
                else {
                    return done(error_envelope_for(method, &Error::Busy(self.worker_limit)));
                };
                let answer: Box<dyn FnOnce(String) + Send> = Box::new(move |body| {
                    // Released before the reply, same as the async path: the
                    // handler is done, so the slot is free before anyone is
                    // woken by the answer.
                    drop(permit);
                    drop(admission);
                    done(body);
                });
                handler(handle.clone(), payload, answer);
            }
            None => self.dispatch_unclaimed(method, payload, done),
        }
    }

    /// A name no static handler claimed. Release: unknown command, exactly as
    /// before. `voussoir dev` with an attached library: a call into the current
    /// commands dylib, answered from a worker thread so a slow hot command
    /// cannot freeze the window either.
    #[cfg(feature = "hot-reload")]
    pub(super) fn dispatch_unclaimed<F>(&self, method: &str, payload: Value, done: F)
    where
        F: FnOnce(String) + Send + 'static,
    {
        match &self.hot {
            Some(hot) => {
                // A hot-library call is a worker-thread call like any other, so
                // it claims the same two bounds. Without that, a page looping on
                // a hot command spawned threads without limit, each reserving a
                // stack, and `max_pending`/`worker_limit` never applied.
                let Some(admission) = claim_admission(&self.pending, self.max_pending) else {
                    return done(error_envelope_for(method, &Error::Busy(self.max_pending)));
                };
                let Some(permit) = claim_worker_slot(&self.worker_inflight, self.worker_limit)
                else {
                    return done(error_envelope_for(method, &Error::Busy(self.worker_limit)));
                };
                let hot = std::sync::Arc::clone(hot);
                let method = method.to_owned();
                let method_for_error = method.clone();
                let answer: std::sync::Arc<std::sync::Mutex<Option<F>>> =
                    std::sync::Arc::new(std::sync::Mutex::new(Some(done)));
                let in_thread = std::sync::Arc::clone(&answer);
                let spawned = std::thread::Builder::new()
                    .name("voussoir:hot".to_owned())
                    .spawn(move || {
                        // Belt over the dylib's own catch_unwind: the page gets
                        // a response even if this outer layer is what failed.
                        let body = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            hot.call(&method, &payload)
                        }))
                        .unwrap_or_else(|_| {
                            error_envelope(&Error::Handler("hot command call failed".to_owned()))
                        });
                        // Both guards go before the reply, so the counts have
                        // moved by the time the page is woken.
                        drop(permit);
                        drop(admission);
                        if let Some(answer) = in_thread
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .take()
                        {
                            answer(body);
                        }
                    });
                if let Err(error) = spawned {
                    if let Some(answer) = answer
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take()
                    {
                        answer(error_envelope_for(
                            &method_for_error,
                            &Error::Runtime(format!("could not start a worker thread: {error}")),
                        ));
                    }
                }
            }
            None => done(error_envelope_for(
                method,
                &Error::UnknownMethod(method.to_owned()),
            )),
        }
    }

    #[cfg(not(feature = "hot-reload"))]
    pub(super) fn dispatch_unclaimed<F>(&self, method: &str, _payload: Value, done: F)
    where
        F: FnOnce(String) + Send + 'static,
    {
        done(error_envelope_for(
            method,
            &Error::UnknownMethod(method.to_owned()),
        ))
    }
}
