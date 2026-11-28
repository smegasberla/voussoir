# Changelog

All notable changes to the `voussoir` crate and the `voussoir` CLI. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html) with the 0.x rule
described under [Compatibility](../README.md#compatibility) in the README.

## [Unreleased]

### Fixed

- `--features full` did not compile for Android, which is the scaffold's default
  on every platform. `updater` is compiled on all targets, but its artifact half is
  desktop work, and the gates were applied to the items inside `install.rs` and
  `manifest.rs` while the `use` statements at the top of those files stayed
  ungated. Gating the **modules** instead is one fact in one place rather than
  thirty, and it removes the class of bug: a file whose body is gated and whose
  prologue is not compiles everywhere the prologue does.

- The MSRV was a false claim. `rust-version` said 1.85 while the tree needed 1.92
  (`rfd` sets it; `muda` wants 1.90, two `icu` families and `time` want 1.88), so
  the job that compiles at the declared version failed on every push. It now says
  1.92 and is verified at 1.92, because pinning nine crates backwards would have
  made the number true for one commit and rotted on the next `cargo update`.

- `deny.toml` banned `ring`, and `ring` is the only crypto provider in the tree, so
  the check failed against its own dependency graph. The list now names the
  *alternatives* (`openssl`, `native-tls`, and rustls's other provider, `aws-lc-rs`)
  which is the version of "one crypto stack" that can actually be checked.

- Two advisories cleared by `cargo update` (`cookie` 0.18.2 -> 0.18.3,
  `url` 2.5.8 -> 2.5.9). The third, `proc-macro-error`, has no fixed release: it is
  named in one `ignore` entry with the reachability argument, rather than by
  silencing unmaintained notices generally.

- `voussoir init` in the iOS CI job did not pass `--ios`, which `init` needs before
  it writes an `ios/` shell, so the job ran `ios build` against a directory that
  was never created.

### Changed

- The event-loop dispatch is split by event class: `window_event` for what the
  platform raises, `user_event` for what another thread posted. They come from
  opposite sides of the process and are debugged differently, and the single match
  had grown past the length the rest of the crate holds itself to.

- `scripts/check-mobile-targets.sh` cross-compiles for both Android ABIs and
  reports iOS as unverified when there is no Apple SDK, rather than printing a pass
  it did not earn. `scripts/check-updater-gating.sh` catches the `cfg`-mismatch
  shape in seconds, without an NDK. Both run in CI, and an `android-check` job runs
  the first so a break fails on the commit that caused it instead of nine minutes
  later in the emulator job.

- `baseline/` and `probe/` declare `license = "MIT"` instead of being exempted in
  `deny.toml`: an exemption list is a place a real unlicensed dependency could
  hide, and both crates are ours.

### Added

- `VOUSSOIR_SNAPSHOT=<path>` and `VOUSSOIR_DUMP=<path>`: the shell captures the
  first window's first finished page load to a PNG and/or writes a JSON
  description of it, then exits. `voussoir run --snap` / `--dump` is the wrapper
  that sets them and waits. Both are unset in every normal run, so a release
  binary carries the code and never takes the branch.

  The capture is Linux and the BSDs only, because that is where the engine exposes
  one (`webkit2gtk`'s `snapshot` through cairo). Requesting a PNG on a platform
  without a capture exits non-zero and says so rather than writing a black frame
  that looks like success. The dump works everywhere, and its `voussoir.bridge`
  field is read from the page, so a shim that refused an origin is visible instead
  of looking like a healthy shell from Rust.

- `voussoir.a11y`: `announce(message)`, `prefers()` and `onPrefersChange(fn)`, and
  shim protocol 4. `announce` writes to a live region the shell owns, hidden from
  sight with the clip technique rather than `display: none`, because a hidden
  element has no accessibility node at all. It clears before it sets, so the same
  message twice in a row is still spoken. `prefers` reports the reader's
  reduced-motion, dark-mode and contrast settings for the case CSS cannot reach:
  a page drawing to a canvas.

- `voussoir doctor` checks the entry HTML for the three defects that ship in
  hand-written frontends — a control with no accessible name, a skipped heading
  level, no `<main>` landmark — and prints them with a line number. A finding is a
  note rather than a failure, because an app can be useful and still have one.

- `WindowEvent::ThemeChanged` is handled. The loop ignored it, so a page that
  wanted to follow the OS appearance had to poll `matchMedia`; it now arrives as a
  `theme` event with `{ "theme": "dark" | "light" }`.

### Changed

- The scaffold's markup is the accessibility baseline rather than a demo: a
  `<main>` landmark, a real `<label>` for the input, `role="status"` and
  `aria-live` on the region a command's answer lands in. The stylesheet honours
  `prefers-reduced-motion` and `forced-colors`, and uses `text-wrap: balance` on
  the heading and `pretty` on the paragraph — the two properties that most cheaply
  stop text looking accidental.

- `build_factory` takes a `FactoryParts` struct instead of eight positional
  arguments. Adding the snapshot request made it eight, which is where a reordering
  hazard starts for the next reader.


- `App::run` is four named steps instead of one 280-line function, split by concern
  the way `ipc` already is: `install_startup_guards`, `restore_after_a_failed_update`,
  `Shell::assemble` (the builder resolved against the configuration, once),
  `Natives::build` (first window, webview, menu, tray) and `run_event_loop`. The
  order is unchanged and the pieces are private; `run` is still the only entry point
  and its signature is identical. Reading what happens at startup no longer means
  reading everything else that happens at startup. The `Shell` that falls out of
  this also removed an accidental duplication: the webview factory, the devtools
  flag and the command timeout were derived in two places, and are now derived once.

- `voussoir::transport` re-exports `method_from_path`. The endpoint decoder is the
  first thing page-controlled input touches, so it belongs beside the other parsers
  in that seam rather than behind `ipc`; `voussoir::method_from_path` is unchanged
  and still public, so nothing an app uses moved.

### Fixed

- The libFuzzer harness did not compile, and nothing noticed: `cargo fuzz` was not
  run by CI at all, so both targets had drifted. `envelope.rs` used `serde_json` as
  if it were a direct dependency, and `path.rs` called a function the crate root
  does not export. Both are fixed, and the harness is now compiled on every push
  (`cargo check --manifest-path fuzz/Cargo.toml --bins`) with the coverage-guided
  runs on a weekly schedule. A fuzz target that does not build is worse than none:
  it reports coverage nobody has.

### Added

- CI runs both benchmarks, and both gate. Cold start is compared against the bare
  `wry`+`tao` control built in the same run and fails past +10%, which is what the
  README already claimed and nothing enforced. `bench.ts --dispatch` gates too, on
  the pooled path staying competitive with thread-per-call.

- `.github/workflows/fuzz.yml`: a weekly libFuzzer run of both targets, with the
  corpus carried between runs by cache and a finding uploaded as an artifact. The
  always-on substitute for push-to-push coverage is the deterministic module in
  `voussoir/src/ipc/fuzz.rs`, which needs no nightly and no budget.

## [0.1.0] - 2026-11-21

First release. Nothing before this was published, so this section absorbs the
unreleased history of the crate instead of starting from an empty list. The 0.x
rule applies from here: the shim is protocol 3, and a breaking change to the
page API or to `Error` bumps the minor version rather than the patch.

### Changed

- Renamed from Kiln to Voussoir. The crate is `voussoir`, the binary and CLI are
  `voussoir`, the config file is `voussoir.json`, the page bridge is
  `window.voussoir`, the reserved command prefix is `__voussoir_`, the custom
  protocol is `voussoir://localhost`, and the environment variables are
  `VOUSSOIR_*`. The reason is availability rather than taste: `kiln` on crates.io
  has been taken and yanked since 2019, so the crate could not be published under
  it. There is no compatibility shim, since 0.1.0 is the first release meant for
  use outside this repository and an app that wants to keep the old names can
  alias them itself.

### Added

- `brand/voussoir.svg`: the mark. An archway lit from the crown, cut out of a
  near-black tile, which is the figure the name describes rather than a generic
  window or box. Two paths and no strokes, so it is a kilobyte, needs no font and
  stays crisp from 16px upward. Every coordinate is a multiple of 8 and the tile
  margin is symmetric, so edges land on whole pixels at the sizes an icon is
  rasterised at and cannot fringe. The CLI's generated fallback icon draws the
  same figure.

- `App` builder with compile-time embedded config (`voussoir.json`) and assets, and a
  cloneable `AppHandle` for `emit`, `emit_to`, `eval`, and `open_window`.
- One JSON IPC endpoint (`POST voussoir://localhost/__voussoir/<method>`), with sync
  handlers on the UI thread and `async_command` handlers on worker threads.
- Panic containment for every handler: a panicking command returns
  `{"error":"command panicked: ..."}` and the window survives.
- Navigation allowlist and Content-Security-Policy served with every response,
  plus the origin-scoped `window.voussoir` shim.
- Optional features: `menu`, `tray`, `dialog`, `updater`, `devtools`, `full`.
- Crash reporting to an in-memory ring buffer, an optional log file, and the
  `__voussoir_reports` command.
- `voussoir-android` crate, driven by an activity that owns the event loop and uses
  wry's IPC handler instead of the custom protocol.
- CLI: `init`, `dev`, `build`, `package`, `android build`, `android run`,
  `doctor`, compiled with `bun build --compile` so end users need no JS runtime.

- Worker-thread commands are bounded. `add_async` spawns one thread per call, and
  a page looping on one slow allowlisted command could reserve stacks until the
  process died; that page is inside the threat model, since the shell assumes the
  webview renders untrusted content. The registry now holds a permit for each
  running worker (default: the machine's parallelism, at least four, changeable
  with `Registry::set_async_limit`), refuses past the limit instead of spawning,
  and releases the permit on return or unwind so a panic cannot leak a slot. The
  page sees a new `Error::Busy` with `kind: "busy"`, which means "retry shortly,
  nothing ran"; `Registry::async_limit` and `async_inflight` expose the state for
  diagnostics and tests.

- `window.voussoir.pageOrigin` (shim protocol 3): the origin the page is actually
  served from. Windows rewrites the app origin to a host form, so a page that built
  an absolute URL from a literal was correct on four targets and broken on one.
  Combined with the existing `protocol` number, the honest feature test is
  `voussoir.protocol >= 3` with a relative URL as the fallback.

- A transport conformance test over the IPC seam, plus a fuzz target for the
  envelope parsers. The two transports already share `transport::respond`; the
  test now runs nine cases (round-trip, absent body, typed reply, malformed typed
  payload, handler error, panic, unknown command, withheld command, refusal of a
  malformed body) through both paths and requires identical bodies, so a drift
  shows up on a Linux run instead of as a red Android or iOS job.

- `fuzz/` (libFuzzer, nightly) with `envelope` and `path` targets, and a
  deterministic stand-in that runs on stable with every `cargo test`: a fixed-seed
  mutational loop over all three untrusted-input parsers. CI reports coverage with
  `cargo llvm-cov`; the threshold is deliberately not a gate yet, because a bound
  picked before a published number is a guess.

- `voussoir dev --verbose`: structured diagnostics for development. Records each hot
  library swap with its rebuild-to-reload latency, every dispatch at or above
  50 ms, refused navigations, dropped bridge messages, and handler panics, keeps
  the last 256 in a bounded buffer, and prints a per-command latency table when the
  app exits. The buffer is reachable from the page as `__voussoir_diagnostics` (and
  `__voussoir_diagnostics_enabled` for the flag alone), which is the only way to read
  it on a platform with no terminal. Everything is off unless `VOUSSOIR_VERBOSE` is
  set, so a release build pays a single cached atomic load per dispatch and
  allocates nothing; `Registry::latencies` and `latency_report` expose the samples
  to a shell or a test.

- `voussoir::rt::sleep`: sleep on the shared async runtime without an app depending on
  a particular executor. The choice of runtime behind `async_fn` is `voussoir`'s, and a
  handler that needs a delay should not have to name it.

- The documentation book in `docs/`, and `voussoir docs` to build it. Eighteen
  chapters from "Your first app" through packaging, the security model,
  multi-window, hot reload, diagnostics and the platform matrix, rendered by mdBook
  with no external asset so it works offline. `voussoir docs --check` validates the
  structure without mdBook installed: every chapter listed in `SUMMARY.md`, every
  link real, no duplicate entries, no orphan pages. CI runs the check. When mdBook
  is missing it names the exact release asset for the host (tar.gz on Unix, zip on
  Windows, arm64 and x64) rather than only linking the releases page, because the
  machine that cannot `cargo install mdbook` is the one that needs the download.

- `bun scripts/bench.ts --dispatch`: measures `command` (UI thread),
  `async_command` (thread per call) and `async_fn` (task per call) on the real
  registry path at concurrency 1, 8, 64 and 512, reports the raw dispatch floor of
  each, finds the first concurrency where the pool wins, and gates on it. The
  measured answer is in the book's benchmark chapter.

- Per-window command allowlists at runtime: `WindowOptions.allowlist` (also
  accepted as `commands` from the page) narrows one window's surface, intersected
  with the global `commands` list, so a window that renders untrusted content can
  be given a strict subset without declaring its label in `voussoir.json` up front.

- Multi-window bookkeeping: `AppHandle::open_window` returns the window's label,
  `__voussoir_open_window` answers `{"label": "..."}`, unnamed windows get a distinct
  label (`index.html`, `index.html-2`, …) instead of colliding, `__voussoir_windows`
  lists the open labels in opening order, and `voussoir.windows()` wraps it. The shim
  protocol stays at 2; `windows()` is the only method protocol 2 promises.

- Runtime tray and menu state: `TrayItem::check()`/`checked(bool)` and
  `MenuItem::check()`/`checked(bool)` declare a check item, and
  `AppHandle::set_tray_tooltip`, `set_tray_checked`, `popup_tray` and
  `set_menu_checked` (plus the `__voussoir_tray_*`/`__voussoir_menu_checked` built-ins and
  the `voussoir.tray`/`voussoir.menu` shim helpers) update it after the app started. Each
  validates before queueing, so a plain item, an undeclared tray, or a platform
  whose tray is a posted notification gets an error naming the reason rather than
  a silent no-op. The native call runs on the UI thread, which is where the
  objects live.

- `voussoir::state`: a shell-owned `serde_json` store that survives a hot reload, with
  `get`/`set`/`remove`/`contains`/`keys`/`clear` and `reload_generation()`. The
  hot loader bumps the generation on each successful swap, so reloaded code can
  see which library is live and keep its state instead of losing it with the
  dylib. The README's hot-reload section now shows the pattern; the 250 ms
  polling latency is documented next to it.

- `command_typed` / `App::command_typed`: a command whose argument and reply are
  concrete types. The payload is deserialized before the body runs, so a wrong
  shape is rejected as `kind: "payload"` naming the command instead of becoming a
  silently defaulted field. The raw `Value` API is unchanged and still the base
  layer; `voussoir` re-exports `serde` so the derives need no extra dependency.

- The `rt` feature and `App::async_fn`: register a real `async fn` command. Its
  future runs on one shared multi-threaded runtime rather than a thread per call,
  so in-flight load costs tasks instead of stacks. A panic inside the future is
  caught and reported as `kind: "panic"` instead of leaving the page waiting
  forever. Off by default; the minimal shell pays nothing and its binary size is
  unchanged.

- The error envelope now carries `method` alongside `message` and `kind`, so a
  page can tell which command failed without parsing text. Every failure path
  (unknown, allowlist denial, bad payload, handler `Err`, panic, runtime) is
  covered by a test asserting both fields and a distinct kind.

- `on_ready` gains the window label and fires exactly once per window, including
  windows opened later; a page reload no longer fires it again.
  **Breaking:** the closure now takes `(&AppHandle, &str)`. The label is what
  makes the hook usable in a multi-window app, and adding it later would have been
  a second break; `.on_ready(|app, _window| …)` is the one-line migration.
  `on_window_close(|app, label| …)` is new and fires once as a window goes away.
  Both are documented builder methods and both are duplicate-event guarded.

- The `storage` feature (part of `full`): `storage.get|set|delete|keys|path`
  backed by a JSON file in the platform data directory, written with an atomic
  rename and capped at 4 MiB. A missing data directory or corrupt file is an error
  envelope the page can handle, never a silently empty store.

- The probe fixture and `scripts/verify.ts` now assert the typed-command rejection
  and the storage round-trip through the real IPC transport on every OS, so the
  T1 surface is proven on WebKitGTK, WebView2 and WKWebView rather than assumed.

- Per-window command allowlists: a `windows` map in `voussoir.json`, keyed by window
  label, narrows what that window may call to the intersection of its own list
  and the global `commands` list. The label is resolved by the shell from the
  calling webview, never from the request, so a page cannot claim another
  window's surface; a window with no entry keeps the global allowlist.

- Install scripts: `install.sh` for Linux, macOS and the BSDs, `install.ps1`
  for Windows. Each installs the toolchain dependencies (system webview dev
  files, Git, Bun, Rust) when they are missing, builds the CLI from source —
  from the checkout when run inside one, a fresh clone otherwise — and puts
  `voussoir` on the user's PATH, finishing with an execution check of the
  installed binary. Dependency steps are skipped when the tool is already
  present, and a failed package install warns instead of aborting, so
  `voussoir doctor` reports any remaining gap by name.

- iOS support through the simulator, per Phases I-1 to I-4 of `IOS-PLAN.md`:
  the `voussoir-ios` bootstrap crate, platform gates with honest fallbacks (menus,
  dialogs, tray and the updater's fetch decline on iOS), UIKit lifecycle mapped
  onto the shared `lifecycle` events, an `ios/` shell scaffold (staticlib,
  XcodeGen project, one-symbol bridging header), and `voussoir ios build|run|verify`
  — built and round-trip-verified on a booted simulator, green in CI since run
  44788618663.
- Hot-reloadable commands behind a cargo feature: the scaffold now ships a
  `commands` crate with one `static COMMANDS: &[Command]` table (with
  `Kind::Sync` / `Kind::Async`), and `voussoir dev` watches its `src/`, rebuilds
  only that crate, and swaps the new library into the running shell. The
  harness (`scripts/verify-hot.ts`) proves the swap leaves the page running —
  one `performance.timeOrigin` across command edits, a panic, a broken build
  and the recovery — with rebuild-to-reload latency printed by the shell.
  Release builds are untouched: the feature is never enabled for them, so
  `libloading` stays out of the release dependency tree.
- `App::report_sink`: install an application sink that receives every crash and
  error record, so an app can forward reports to its own collector without
  forking the shell. The framework still uploads nothing by itself.
- Crash-log rotation: the log file is capped at 512 KiB and rotated to
  `<name>.old`, so a panic loop cannot fill a user's disk.
- `CHANGELOG.md` and a README "Compatibility policy" section stating what a
  breaking change is.

### Changed

- `voussoir init` scaffolds commands as a table in a separate crate instead of
  inline `.command()` calls in `main.rs`, so the same code hot-reloads in dev
  and links statically in release. `.command()` remains for commands that need
  the `App` itself; release still builds with every feature, as it did.
- Documentation now states that hot reload is desktop-only and that statics
  inside the commands crate do not survive a reload.
- The Android release CSP no longer allows `'unsafe-inline'` in `script-src`; it
  now matches the desktop release policy. The removal is backed by an emulator
  measurement (android-36, system WebView 133) in which the injected bridge shim
  round-tripped a command with the tightened policy; a guard test asserts the
  Android `script-src`.
- The desktop↔Android parity checker warns when a command registration is not a
  statically readable string literal, and documents its known limitations.
- Command parity is now three-way (desktop, Android, iOS), and a `#[cfg(test)]`
  command table no longer counts as a registration — it had made every fresh
  desktop+mobile scaffold fail parity on commands that only exist in tests.

### CI

- The Windows and macOS jobs now run `scripts/verify.ts`, so the panic
  containment, remote-script and navigation guarantees are asserted on WebView2
  and WKWebView too, not only on WebKitGTK. The harness gained a Windows `.exe`
  suffix and explicit per-platform skip reporting for Linux-only checks.

### Fixed

- WebView2 could not reach a command at all. It cannot intercept a custom
  scheme, so wry rewrites `voussoir://localhost` to `http://voussoir.localhost`, the
  same way it does on Android; Voussoir only taught Android about the rewritten
  origin, so on Windows the shim allowlisted an origin the page never reports,
  `invoke` posted to an endpoint nothing intercepts, and the first navigation
  was refused. The page origin now follows the target, and the dev CSP, the
  probe fixture and the harness pages use it too.

- The Linux job pins `appimagetool` to the 1.9.1 release by SHA-256 instead of
  the moving `continuous` tag, and runs `cargo test --workspace` verbatim now
  that `voussoir-android`'s dependencies are target-gated to Android.

### Added

- `command_timeout_ms` in `voussoir.json` (default 30 s, `0` disables) and a
  per-call `invoke(method, payload, { timeout })` override. The shim's `fetch`
  had no deadline, so a handler that deadlocked left the page awaiting a promise
  that never settled, and a hang was indistinguishable from slowness. The
  deadline aborts the request rather than only rejecting, so the socket is freed
  too, and rejects with `kind: "timeout"` so a page branches on the class. The
  value is exposed as `voussoir.timeout` so frontend code does not have to guess it.

- `set_max_pending` / `max_pending` / `pending` bound how many async requests
  the shell will hold at once, separately from the worker limit. The worker
  limit bounds thread stacks, not load: a page looping on a command that
  finishes quickly kept every slot saturated indefinitely, one blocked `fetch`
  and promise per request. The default matches the worker limit, so no existing
  app behaves differently; the knob exists so a deployment can be stricter.

- An allowlist entry that names no registered command is recorded at startup,
  for `commands` and for each window's list independently. A typo
  (`{"commands": ["gret"]}`) used to withhold the command the author meant to
  keep and say nothing, so the app looked broken rather than misconfigured. The
  check is skipped under `hot-reload`, where any name may be the library's and
  the library cannot be asked what it exports.

- A second registration of the same command name is now recorded as a
  diagnostics event. Registration stays last-wins, but silent replacement is
  data loss: the first handler's command stops existing and the second answers
  calls meant for it, and the registry cannot report it afterwards because only
  the survivor is left in the map. Two plugins claiming `open` is the realistic
  case, and `callable()` cannot see it.

- Plugins: a crate implements `Plugin` and an app installs it with
  `App::plugin(...)`. There is no manifest, no codegen and no new dependency,
  because Cargo already resolves versions and the linker already fails when a
  crate is missing. `install` takes the app and returns it, so plugins compose
  with the builder and each other. `App::has_command` and `Registry::contains`
  let a plugin defer to a handler the app already registered instead of silently
  replacing it, which is the one real hazard of last-wins registration in a flat
  namespace.

### Changed

- `ipc.rs` is a directory of eight modules rather than one 1662-line file. The
  split is by concern, not by size: `registry` holds the table, both allowlists
  and the worker bound, `register` the `add*` surface, `dispatch` the path from a
  method name to a response body (where the threat model is enforced), `handler`
  the handler kinds and panic isolation, `envelope` the bytes a page receives,
  `metrics` the latency samples, `worker` the permit that bounds thread-per-call
  handlers, and `path` the endpoint decoder. The tests and the deterministic fuzz
  module moved beside what they test. Nothing about the envelope, the allowlist
  or the worker bound changed; the consts and re-exports at `ipc` are unchanged,
  so `Registry` and `method_from_path` are still where they were.

- The crate root is a map of the shell again, not a thousand-line module. `lib.rs`
  holds the public re-exports, `SCHEME` and `SHIM_PROTOCOL`; everything else moved
  beside it: `app.rs` (the builder, `run`, and `platform_name`), `handle.rs`
  (`AppHandle`, `UserEvent`, envelope parsing), `hooks.rs` (`Fired` and the hook
  types), `builtins.rs` (the commands the shell registers), `shell.rs` (webview
  factory, menu and tray builders, the injected shim, the devtools shortcut),
  `transport.rs` (the IPC seam) and `tests.rs`. No behaviour changed and no item
  left the public API; the split is what makes the next change to any one of them
  reviewable on its own.

### Fixed

- `method_from_path` panicked on a multi-byte path. It decoded `%XX` escapes by
  slicing the path `&str` at byte offsets, so `/__voussoir/%` followed by any
  multi-byte character landed inside a char boundary and aborted the request
  thread. It now decodes bytes, refuses a result that is not valid UTF-8, and has
  three regression tests plus a fuzz target.

- Lock poisoning no longer takes the shell down. Fifteen non-test `unwrap()` sites
  across `platform.rs` and `reporting.rs` panicked if a mutex had been poisoned by
  an earlier panic; they now recover the guard, because the data behind them is
  plain and consistent after a panic. The remaining response-builder sites in
  `dispatch.rs` use `expect` with the invariant written down, and `rt.rs`'s runtime
  construction documents why a lazy panic is the right report there. Six
  documented `expect` calls remain outside tests, none on a reachable failure path.
