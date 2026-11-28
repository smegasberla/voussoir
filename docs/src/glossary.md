# Glossary

Every term the book uses, in the sense it uses it. Skim it once; come back when a
chapter assumes a word you are not sure about.

## The two halves

**Shell** — the Rust program. It opens windows, decides what a page may load,
holds the command registry, and runs the event loop. In a scaffolded app it is
`native/src/main.rs`.

**Frontend** — the HTML, CSS and JavaScript your page is made of, in `src/` and
`index.html`. Bundled by Bun at build time, never interpreted at runtime.

**Webview** — the browser engine the OS already ships, which renders the frontend:
WebView2 on Windows, WKWebView on macOS and iOS, WebKitGTK on Linux, the Android
System WebView on Android. It is not bundled with your app.

**Bundled browser** — the alternative, where the app ships an engine (as Electron
does). Voussoir does not do this, and the trade is in
[Concepts](concepts.md).

## Talking between them

**Command** — a named function in Rust that the page can call. Registration is the
only way it exists; there is no discovery.

**Invoke** — the page-side call: `window.voussoir.invoke(method, payload)`.

**Envelope** — the one request/response shape everything uses:
`{"ok": ...}` or `{"error": {"message", "kind", "method"}}`.

**Payload** — the JSON value passed to a command, or returned by one.

**Kind** — the error class in an envelope. One of seven, listed in
[Errors the page can branch on](errors.md).

**Emit** — Rust sending a value to the page, as an event. Asynchronous: you cannot
get a reply.

**Listen** — the page-side subscription: `window.voussoir.listen(event, fn)`.

**Shim** — the small JavaScript object the shell injects as `window.voussoir`. It
carries `invoke`, `listen`, `windows`, `tray`, `menu`, `a11y`, `platform`,
`pageOrigin`, `timeout` and `protocol`.

**Shim protocol** — the version of the shim object, independent of the crate
version. Currently 4. See [Compatibility policy](compatibility.md).

**Snapshot** — a PNG capture plus a JSON description of the first window, written
by `voussoir run --snap` / `--dump` so an agent or a CI job can inspect a rendered
app. See [Headless tooling](headless.md).

**Transport** — how an envelope physically travels. A custom scheme on desktop and
iOS, `postMessage` on Android. Never something app code names.

## Where code runs

**Execution model** — where a command's body runs: the UI thread (`command`), a
worker thread (`async_command`), or the shared async runtime (`async_fn`). See
[Commands & payloads](ipc-contract.md).

**UI thread** — the thread that paints the window. Blocking it freezes the app.

**Worker thread** — a thread spawned per call, for blocking work.

**Runtime** — the shared async executor behind `async_fn`. One per process, one
worker per available core, minimum two.

**Command table** — the `COMMANDS` slice in the commands crate. Hot-reloadable.

**Commands crate** — `native/commands`, compiled as a `cdylib` in dev and a static
library in release. Why it is separate is in
[How a project is laid out](layout.md).

## Permissions and safety

**Allowlist** — the set of command names a page may call. Absent means everything.

**Intersection rule** — a window's effective surface is its list **and** the global
list, so a config entry can only narrow. See
[Per-window allowlists](allowlists.md).

**Window label** — the stable string identifying a window, and the key the allowlist
uses. Read from the webview, never from the payload.

**CSP** — Content-Security-Policy. The header that forbids remote script, `eval`
and remote `fetch`.

**Origin** — scheme + host + port. `voussoir://localhost` on desktop and iOS,
`http://voussoir.localhost` on Windows. See [The two origins](origins.md).

**Confused deputy** — a privileged component tricked into using its authority on
behalf of unprivileged content. The problem per-window allowlists solve.

## State

**In-process state** — `voussoir::state`. A JSON map keyed by string, lives as long
as the process, survives a hot reload. See
[In-process state](state.md).

**Storage** — the `storage.*` commands. A JSON file in the platform data
directory, survives a restart, capped at 4 MB. See [Storage](storage.md).

**Reload generation** — a counter incremented each time a new command library
becomes live.

## Building and shipping

**Asset table** — `native/src/assets.rs`, the generated list of frontend files
embedded in the binary. Never edited by hand.

**Hot reload** — swapping the command library into a running dev process. Desktop
and development only.

**Cold start** — process start to the page-load-finished callback. What a user
feels as "instant" or "slow".

**AppImage** — the Linux package: one self-contained file, no root install.

**`.app`** — the macOS bundle: a directory that Finder treats as one icon.

**Notarization** — Apple's automated scan of a signed macOS app. Requires a
developer account; the tool prints the command rather than doing it.

**WebView2 bootstrapper** — the runtime installer for old Windows releases that do
not preinstall WebView2.

## Terms from the wider stack

**Bun** — the JavaScript runtime and bundler the toolchain uses. Development-time
only: no shipped app contains it.

**Cargo** — Rust's build tool and package manager.

**`wry`** — the Rust library that creates a webview per platform.

**`tao`** — the Rust library that creates windows and runs the event loop per
platform.

**`cdylib`** — a shared library crate type, loadable at runtime. What makes hot
reload possible.

**`libloading`** — the crate that loads a `cdylib` at runtime. Absent from release
builds, which `verify.ts` asserts.

## Next

- [Concepts](concepts.md) — the same ideas with the reasoning behind them.
- [Commands & payloads](ipc-contract.md) — where most of these terms are used.
