// Injected before every page load, on every frame.
//
// The whole frontend-side runtime: keep it dependency-free and small.
//
// Two transports, because the platforms genuinely differ:
//
//   - Desktop: `invoke` is a POST to a custom protocol, which answers directly.
//   - Android: `WebResourceRequest` carries no body, so wry's custom protocol
//     cannot receive a payload there. Instead requests go over `window.ipc`, which
//     wry wires to a native handler; the reply comes back through
//     `__voussoir_resolve` on the next event-loop turn.
//
// The page sees one API either way: `voussoir.invoke(method, payload)`.
(() => {
  const base = "__INVOKE_BASE__";
  const allowed = __ALLOWED_ORIGINS__;
  const bridge = __BRIDGE__;
  // Rejecting at a deadline is what makes a deadlock visible: a fetch with no
  // timeout waits forever, so a handler that hung looked exactly like one that
  // was slow, and the page spun with no signal. 0 disables it.
  const defaultTimeout = __TIMEOUT__;
  const listeners = new Map();
  const pending = new Map();
  let nextId = 1;
  // A page without a viewport meta renders at the browser's 980px fallback
  // viewport on a phone, which reads as a page that slides sideways under the
  // reader's finger. Add the meta only when the page did not declare one, so an
  // app that pins its own scale is never overridden.
  //
  // This runs at document start, where WKWebView and WebView2 have neither
  // `document.head` nor `document.documentElement` yet. Touching one threw, the
  // rest of this script never ran, and `window.voussoir` was therefore missing:
  // the page's own call failed with a ReferenceError that its catch turned into
  // a text line, so the window looked fine and no request was ever made. Wait
  // for the element instead, and never let the decoration take the API down.
  const addViewport = () => {
    try {
      if (document.querySelector('meta[name="viewport"]')) return;
      const root = document.head || document.documentElement;
      if (!root) return;
      const meta = document.createElement("meta");
      meta.name = "viewport";
      meta.content = "width=device-width, initial-scale=1";
      root.appendChild(meta);
    } catch {
      // A viewport hint is worth less than a working bridge.
    }
  };
  if (document.documentElement) addViewport();
  else document.addEventListener("DOMContentLoaded", addViewport, { once: true });

  function originAllowed() {
    if (allowed.length === 0) return true;
    const here = location.origin;
    if (allowed.indexOf(here) !== -1) return true;
    // Custom protocols report an opaque origin on some platforms; fall back to
    // the scheme the page was actually loaded from.
    return location.protocol === "voussoir:";
  }

  function teardown() {
    listeners.clear();
    pending.clear();
    try {
      delete window.voussoir;
    } catch {
      window.voussoir = undefined;
    }
  }

  if (!originAllowed()) {
    teardown();
    return;
  }

  /** Resolve a bridge request. Called from Rust via evaluate_script. */
  window.__voussoir_resolve = (id, envelope) => {
    const entry = pending.get(id);
    if (!entry) return;
    pending.delete(id);
    if (envelope.error !== undefined) entry.reject(toError(envelope.error));
    else entry.resolve(envelope.ok);
  };

  // Errors arrive as {message, kind, method} since the structured envelope
  // shipped; the plain-string form is still accepted so an older shell or the
  // hot-reload dylib answering a newer page rejects with readable text either
  // way. The kind rides along on the Error so frontends can branch on it.
  function toError(error) {
    if (typeof error === "string") return new Error(error);
    const err = new Error((error && error.message) || "command failed");
    if (error && error.kind) err.kind = error.kind;
    if (error && error.method) err.method = error.method;
    return err;
  }

  function invokeOverBridge(method, payload, onPending) {
    if (typeof window.ipc === "undefined" || !window.ipc.postMessage) {
      return Promise.reject(new Error("voussoir: bridge unavailable"));
    }
    const id = nextId++;
    if (onPending) onPending(id);
    const message = JSON.stringify({
      id,
      method,
      payload: payload === undefined ? null : payload,
    });
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      window.ipc.postMessage(message);
    });
  }

  /** The deadline for one call: the per-call option, else the app default. */
  function timeoutFor(options) {
    const asked = options && options.timeout;
    if (asked === undefined || asked === null) return defaultTimeout;
    return asked;
  }

  function withDeadline(method, ms, run, cancel) {
    if (!ms || ms <= 0) return run();
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        if (cancel) cancel();
        reject(toError({ message: "command timed out: " + method, kind: "timeout", method }));
      }, ms);
      run().then(
        (value) => { clearTimeout(timer); resolve(value); },
        (error) => { clearTimeout(timer); reject(error); },
      );
    });
  }

  async function invokeOverProtocol(method, payload, onController) {
    const init = {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(payload === undefined ? null : payload),
    };
    // AbortController is what actually frees the request: a deadline that only
    // rejected would leave the socket open for as long as the handler blocked.
    if (typeof AbortController !== "undefined") {
      const controller = new AbortController();
      init.signal = controller.signal;
      onController(controller);
    }
    const res = await fetch(base + encodeURIComponent(method), init);
    const text = await res.text();
    let envelope;
    try {
      envelope = JSON.parse(text);
    } catch {
      throw new Error("voussoir: malformed response for " + method);
    }
    if (envelope.error !== undefined) throw toError(envelope.error);
    return envelope.ok;
  }

  async function invoke(method, payload, options) {
    if (!originAllowed()) {
      teardown();
      throw new Error("voussoir: bridge unavailable on this origin");
    }
    const ms = timeoutFor(options);
    if (bridge) {
      // The reply arrives on a later event-loop turn, so the deadline has to
      // forget the pending entry too: a late reply would otherwise resolve a
      // promise that already rejected.
      let id = 0;
      return withDeadline(
        method,
        ms,
        () => invokeOverBridge(method, payload, (assigned) => { id = assigned; }),
        () => pending.delete(id),
      );
    }
    let controller = null;
    return withDeadline(
      method,
      ms,
      () => invokeOverProtocol(method, payload, (c) => { controller = c; }),
      () => controller && controller.abort(),
    );
  }

  function listen(event, fn) {
    let set = listeners.get(event);
    if (!set) listeners.set(event, (set = new Set()));
    set.add(fn);
    return () => set.delete(fn);
  }

  window.__voussoir_emit = (event, payload) => {
    const set = listeners.get(event);
    if (set) for (const fn of set) fn(payload);
  };

  // The open window labels, in the order they opened. A page that opens a window
  // gets its label back, but it still needs to know what closed.
  async function windows() {
    const reply = await invoke("__voussoir_windows");
    return (reply && reply.windows) || [];
  }

  // Tray and menu state, so a page changes a tick without re-declaring anything.
  // Each resolves to null; a platform without the affordance rejects with the
  // shell's reason rather than pretending it worked.
  const tray = {
    tooltip: (text) => invoke("__voussoir_tray_tooltip", { text }),
    checked: (id, checked) => invoke("__voussoir_tray_checked", { id, checked }),
    popup: () => invoke("__voussoir_tray_popup"),
  };
  const menu = {
    checked: (id, checked) => invoke("__voussoir_menu_checked", { id, checked }),
  };

  // The page is shared across platforms, so it needs to know which one it is on.
  // Injected at build time rather than sniffed: a WebView user-agent lies more
  // often than it helps.
  // `protocol` is the shim's own version, not the shell's. Frontend code
  // feature-detects with it instead of probing for methods: a page that needs
  // `windows()` asks `voussoir.protocol >= 2` and degrades once, in one place.
  window.voussoir = {
    invoke,
    listen,
    windows,
    tray,
    menu,
    platform: "__PLATFORM__",
    // The origin this page is actually served from. On Windows it is the
    // rewritten host form, because WebView2 cannot intercept a custom scheme, so
    // a page that builds an absolute URL from a literal is correct on four
    // targets and broken on one. Read it here instead of assuming.
    pageOrigin: "__PAGE_ORIGIN__",
    timeout: defaultTimeout,
    protocol: __SHIM_PROTOCOL__,
  };
})();
