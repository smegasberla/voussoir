<div align="center">

<img src="brand/voussoir.png" alt="Voussoir" width="140" height="140">

# Voussoir

**A desktop and mobile app framework where the shipped artifact is a Rust binary.**
One window, the operating system's own webview, and a single JSON bridge between
Rust and the page.

[![CI](https://github.com/smegasberla/voussoir/actions/workflows/ci.yml/badge.svg)](https://github.com/smegasberla/voussoir/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](#license)
[![Platforms](https://img.shields.io/badge/platforms-Linux%20%7C%20macOS%20%7C%20Windows%20%7C%20Android%20%7C%20iOS-lightgrey.svg)](#platform-support)

</div>

---

> ### Built with AI
>
> This project was written with AI assistance, and this is stated here rather
> than buried. The code was largely produced by AI and is held to checks that
> run without asking anyone's opinion: `cargo test`, `bun test`, and
> `scripts/verify.ts` against a real window on the real IPC path, in CI on six
> targets. Read the claims below as claims that are *checked* rather than as a
> CV: the numbers are reproducible from a checkout, and where something is
> unverified this README says so.

A *voussoir* is the wedge-shaped stone an arch is built from, and the one at the
crown is the keystone that locks the rest in place. This project is that
arrangement: a thin load-bearing shell holding a page and native code together,
and holding nothing else.

## Why

This started from a reaction to what frameworks hand you when you run `init`. The
project appears, and before you have written a line you are reading a directory:
a bundler config, a plugin manifest, capability files, a folder per feature, a
`build` folder deeper than the app itself. Most of it you will never open, and
none of it is yours. The cost is not the kilobytes. It is that nobody can hold
the structure in their head, so every change begins with archaeology: what wrote
this file, what reads it, what breaks if I delete it.

So the first thing this project is measured on is its own scaffold.
`voussoir init` gives a desktop app **12 files**. Three are configuration or
housekeeping (`tsconfig.json`, `voussoir.json`, `.gitignore`) and the other nine
are the app: two Rust crates, one page, one stylesheet, one script, and the
generated types for the bridge. There is no bundler config because Bun needs
none, no capability directory because registered handlers are the allowlist, and
no plugin manifest because a plugin is just a crate. Every file has an owner and
a reason, and you can read all twelve in one sitting. Mobile folders are
generated too, but only for the platforms you asked for, and they sit in their
own directories rather than thickening the desktop one.

Most desktop shells sit on one side or the other of a line. Electron ships a
browser. Tauri is a full application platform, with plugins, capability files and
a build step per feature. Voussoir aims at the narrow middle: **the whole runtime
is three direct dependencies**, and everything optional is a cargo feature a
minimal app does not pay for.

```
voussoir 0.2.0
├── serde_json
├── tao
└── wry
```

It is built directly on [`wry`](https://github.com/tauri-apps/wry) (webview) and
[`tao`](https://github.com/tauri-apps/tao) (windowing), the same low-level crates
Tauri sits on, but not on Tauri itself. With no features it is a **785 KB**
binary; the `full` set is **~2.4 MB**. The frontend is compiled into the
executable, so the artifact contains no Chromium, no Node, no Bun, no V8 and no
`node_modules`. The webview is the OS's own:

| Platform | Webview |
| --- | --- |
| Linux | WebKitGTK 4.1 |
| macOS | WKWebView |
| Windows | WebView2 |
| Android | `WebView`, via wry's IPC handler |
| iOS | WKWebView, via `WKURLSchemeHandler` |

## Quickstart

```sh
voussoir init myapp
cd myapp
voussoir dev
```

`voussoir dev` starts Bun's dev server in-process and launches the shell against
it. Editing a `.ts` or `.css` file reloads the page; editing a command rebuilds
only that crate and swaps the new library into the running window, keeping the
last good one if the build fails.

```sh
voussoir build --size             # release binary with the frontend inside
voussoir package --icon icon.png  # AppImage / .app / .zip
voussoir doctor                   # toolchain and project checks
```

## Install

One script installs the system webview, Git, Bun and Rust, builds the CLI, and
puts `voussoir` on `PATH`.

Linux, macOS and the BSDs:

```curl
curl -fsSL https://raw.githubusercontent.com/smegasberla/voussoir/master/install.sh | sh
```

Windows:

```irm
irm https://raw.githubusercontent.com/smegasberla/voussoir/master/install.ps1 | iex
```

Both read the script straight from the default branch, so there is no download
step and nothing to keep up to date. They are the same for a first install and
for an upgrade: re-running skips every dependency that is already present, and
builds whatever is on the branch.

From an existing checkout, run the script in place — it notices the tree you are
in and builds that instead of cloning:

```sh
./install.sh                                  # Linux, macOS, BSD
powershell -ExecutionPolicy Bypass -File install.ps1   # Windows
```

| Flag | Both | Effect |
| --- | --- | --- |
| `--prefix` / `-Prefix` | yes | install location, default `$HOME/.local` (`%LOCALAPPDATA%\voussoir`) |
| `--no-deps` / `-NoDeps` | yes | skip system package installation |
| `--repo` / `-Repo` | yes | git URL to clone when not run from a checkout |

`VOUSSOIR_PREFIX` and `VOUSSOIR_REPO` do the same as the Unix flags. A failed
package install warns instead of aborting, so `voussoir doctor` can report the
remaining gap by name rather than the installer dying halfway through.

The compiled CLI needs neither Bun nor Node on `PATH`: `bun build --compile`
embeds the runtime it was built with. Voussoir is not on crates.io yet, so these
scripts and a checkout are the only supported ways to get it.

## The contract

One endpoint, one envelope, in both directions:

```
POST voussoir://localhost/__voussoir/<method>
body:     <payload as JSON, or null>
response: {"ok": <json>}
        | {"error": {"message": "<text>", "kind": "<class>", "method": "<name>"}}
```

`kind` is one of `unknown` (unregistered, or withheld by an allowlist), `payload`
(the request did not match the handler's argument type), `handler` (the handler
returned `Err`), `panic` (the handler panicked; the app kept running) or
`runtime` (the webview or event loop failed). A page branches on the kind instead
of matching message text, and `method` names the command that failed even when
the call came from a nested helper.

The page gets a shim before any of its own scripts run:

```ts
const greeting = await window.voussoir.invoke<string>("greet", { name: "you" });
const off = window.voussoir.listen<{ value: number }>("count", (c) => render(c));
```

Commands live in one table, in `native/commands/src/lib.rs`:

```rust
pub static COMMANDS: &[Command] = &[
    Command { name: "greet", kind: Kind::Sync, run: greet },
];

fn greet(payload: &Value) -> Result<Value, String> {
    let name = payload.get("name").and_then(|v| v.as_str()).unwrap_or("world");
    Ok(json!(format!("Hello, {name}!")))
}
```

A release links that table into the binary; `voussoir dev` hot-reloads it. A
command that needs the app itself, such as opening a window, is registered on
`App` in `main.rs` with `.command("name", …)` and stays static in both modes.
`tap.emit("count", json!(1))` pushes the other way through the event loop proxy,
so it is thread-safe and never touches the webview off the UI thread.

Typed commands are one derive away. The re-export is what keeps an app on a
single dependency:

```rust
#[derive(voussoir::serde::Deserialize)]
#[serde(crate = "voussoir::serde")]
struct ShiftArgs { #[serde(default)] delta: i64 }

app.command_typed("shift", |_app, args: ShiftArgs| {
    Ok(json!({ "value": args.delta }))
});
```

### Why a custom protocol

wry's IPC handler is fire-and-forget on every platform: it returns no value.
Building a request/response bridge on top of it needs a second hop through
`evaluate_script`, which means serializing every reply into JavaScript and losing
the error type on the way. A `POST` to a registered scheme handler has a body and
a response, so the reply is the reply.

### What is deliberately absent

- **No plugin system.** A plugin is a crate that calls `emit` and registers
  commands; the core needs nothing for that to work.
- **No ACL or capability files.** Registered handlers are the allowlist, decided
  at compile time. `voussoir.json` can narrow it per window, and only narrow.
- **No codegen macro.** `app.command("name", fn)` instead of `#[tauri::command]`
  plus `invoke_handler![generate!]`.
- **No upload anywhere.** A panic hook writes to a local ring buffer readable
  through `__voussoir_reports`. Nothing leaves the machine.

## What is guaranteed

Each of these is asserted by `scripts/verify.ts` against a real window on the
real IPC path, not by inspection.

| Guarantee | How |
| --- | --- |
| A panicking command does not kill the app | every handler is wrapped in `catch_unwind`; the page gets a `panic` error and the window keeps its unsaved state |
| Slow work does not freeze the window | `async_command` runs on a worker thread; with `rt`, `async_fn` runs futures on one shared runtime rather than a thread per call |
| Settings persist without a crate of your own | `storage.*` writes JSON with an atomic rename, so a crash mid-save cannot truncate it |
| Lifecycle hooks fire once | `on_ready` and `on_window_close` are guarded against duplicate platform events |
| Crashes leave a trace | a panic hook records the location to an optional log file and a ring buffer |

## Features

Optional capabilities are cargo features, so a minimal app links none of them.

| Feature | Adds | Why it is opt-in |
| --- | --- | --- |
| `menu` | `muda` | native menu bar with cut/copy/paste and quit |
| `tray` | `tray-icon` | system tray icon (implies `menu`) |
| `dialogs` | `rfd` | native open/save/folder dialogs |
| `updater` | `ureq` + rustls | update checks against your JSON endpoint |
| `devtools` | `wry/devtools` | devtools in release builds; `voussoir dev` enables it |
| `storage` | — | JSON settings file in the platform data directory |
| `rt` | `tokio` (scheduler only) | real `async fn` commands on a shared runtime |
| `full` | several of the above | menu + dialogs + updater + storage; devtools is dev-only |

`voussoir init` scaffolds with `full`. The Android example in this repository adds
`tray` on top, because its notification is what proves that path on a device.
`rt` and `hot-reload` stay out of `full` on purpose: a synchronous app should not
pay for a scheduler, and a released shell never loads code dynamically.

## Platform support

| Platform | Build | Tests | Notes |
| --- | --- | --- | --- |
| Linux x86_64 | yes | yes | WebKitGTK 4.1 |
| Windows x86_64 | `cargo check` clean | yes (CI) | WebView2, preinstalled on Win10 2020+ and 11 |
| macOS x86_64 / arm64 | `cargo check` clean | yes (CI) | WKWebView |
| FreeBSD | CI (VM) | CI (VM) | GTK 3 + WebKit2GTK, same code path as Linux |
| NetBSD | `cargo check` | graph only | **unverified at runtime** |
| Android arm64 / x86_64 | yes | x86_64 only (emulator, CI) | wry's IPC handler, same commands and policy |
| iOS aarch64 (simulator) | yes | yes (simulator, CI) | desktop origin and policy, no platform branch |

BSD works because wry and tao already treat it as a webkitgtk/GTK platform. Two
things blocked it, tao's default `dbus` feature and a Linux-only `gtk`
dependency, and both are fixed. The remaining cost is that the `-sys` crates need
GTK and WebKit on the target, so it builds and runs on BSD rather than
cross-compiling from Linux.

**Android** is a separate crate, `voussoir-android`, because the platform is
genuinely different. The activity owns the event loop, and `invoke` cannot use the
custom protocol: `WebResourceRequest` carries no body, so wry would deliver every
payload as `null`. Android therefore uses wry's IPC handler and resolves the
page's promise from Rust. The envelope, registry, allowlist and security policy
are shared through a transport seam, so app code is identical everywhere. CI runs
the instrumentation suite on an emulator, then launches the app with no
instrumentation attached and fails on a `FATAL EXCEPTION` or an
`UnsatisfiedLinkError`.

**iOS** keeps the custom-scheme transport, since `WKURLSchemeHandler` intercepts
`voussoir://localhost` the way the desktop does, so the origin and policy are the
desktop ones byte for byte; the only mobile branch in `security.rs` is Android's.
What changes is who owns the process: there is no Rust `main`, because the event
loop must own the main thread. Swift's `main.swift` calls the staticlib's
exported `main_rs` first, and tao calls `UIApplicationMain` from inside it.
`voussoir ios build|run|verify` mirror the Android commands; `--target device`
builds unsigned, since installing on hardware needs the app owner's provisioning
profile.

## Packaging

`voussoir package` wraps a build in the format users on that platform install:

| Platform | Output |
| --- | --- |
| Linux | `.AppImage`, one file, no install, no root |
| macOS | `.app` bundle with `Info.plist` |
| Windows | `.exe` in a `.zip`, with the WebView2 requirement noted |

An icon is optional: `--icon icon.png` uses yours, and without one the packager
draws a default so a build never fails on missing artwork.

Signing is **not** attempted. It needs credentials the tool cannot invent, and a
package that looks signed but is not is worse than an honestly unsigned one.
`voussoir package` prints the exact command for the platform instead. CI builds
and uploads artifacts for a tag, and signs or notarizes only when the
corresponding secrets exist.

## Compatibility

The crate is 0.x, so [Cargo's 0.x rules](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html)
already permit a minor bump to break. On top of that, this is promised:

**Breaking** (needs a 0.x bump and a `CHANGELOG.md` entry): command names and the
`{"ok"|"error"}` envelope; `App` and `AppHandle` public signatures; the
`voussoir.json` schema; the `window.voussoir` shim (`invoke`, `listen` and their
argument shapes); the meaning of an existing cargo feature.

**Not breaking**: internal module layout; new opt-in features, built-in commands
or `voussoir.json` keys; patch bumps.

Every user-visible change goes into `CHANGELOG.md` in Keep a Changelog format, so
"is this upgrade safe for my app?" is answerable from the changelog plus the lists
above.

## Repository

```
voussoir/              runtime shell crate (what ships)
  examples/hello.rs    minimal app: embedded assets, round-trip, emit
  examples/studio.rs   full-featured: menus, tray, dialogs, updater, multi-window
voussoir-android/      Android platform crate
voussoir-ios/          iOS bootstrap crate
probe/                 test fixture: commands that panic on purpose, for verify.ts
baseline/              bare wry+tao app: the measurement baseline
cli/                   Bun CLI, dev tooling only, never in the artifact
  template/            scaffold files, inlined into the CLI at compile time
  src/icon.ts          PNG encoder and default icon, no image dependency
brand/                 the mark: voussoir.svg plus PNG renders
docs/                  the documentation, plain Markdown in docs/src
website/               the site: renders docs/src, no mdBook and no deps
scripts/verify.ts      end-to-end checks, and the source of the numbers above
```

The book in `docs/src` is the narrative version: installing, your first app,
"Commands & payloads", "Security model", "Multi-window", "Packaging".
`voussoir docs --check` validates its structure and every cross-chapter link in
CI, and `voussoir docs` renders the same files into `website/dist`. The published
copy is at **https://voussoir-app.pages.dev**.

## Contributing

The gates CI runs are `cargo test`, `bun test` in `cli/`, and
`scripts/verify.ts`. A change to the shell should also keep
`cargo clippy --all-targets -- -D warnings` and `cargo fmt --all --check` clean.
A bug report with a reproduction is worth more than a feature request; this
project says no to features on purpose.

The framework checkout stays free of `node_modules`: the CLI needs no packages
to run or test, and `scripts/typecheck-cli.sh` installs TypeScript and Bun's
declarations in a temporary directory, pointing `tsc` at them. CI fails if a
dependency tree appears in `cli/` or `promo/`.

## License

[MIT](LICENSE). Do what you want with it: use it, modify it, sell it, ship it
closed. The only obligation is keeping the copyright notice and the permission
text in copies you redistribute, which is the whole of the license.

There is no separate contributor agreement. Unless you state otherwise, anything
you intentionally submit for inclusion is licensed the same way, with no
additional terms.
