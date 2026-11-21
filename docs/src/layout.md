# How a project is laid out

Every file in a Voussoir project, what writes it, and whether you may edit it.
Read this once when you start and again the first time you wonder where something
belongs.

## Theory: why two languages in one directory

A Voussoir app is a Rust crate and a frontend that share a directory, not because
it is fashionable, but because the two halves have different rebuild costs:

| Half | Tool | Rebuild time | What it owns |
| --- | --- | --- | --- |
| Frontend | Bun | milliseconds | how the app looks |
| Commands | Cargo | seconds | what the app does |
| Shell | Cargo | a minute or more | the window, policy, wiring |

Splitting them into three build units is what makes the dev loop usable: changing
a colour must not recompile a webview bindings crate. The scaffold's layout is
that split made concrete.

## The tree

```
myapp/
  voussoir.json                configuration, embedded into the binary
  index.html                   entry page; Bun bundles it and what it imports
  tsconfig.json                TypeScript settings for your editor
  src/                         frontend: TypeScript, JSX, CSS
    main.ts                    the code your page runs
    style.css                  styles
    voussoir.d.ts              editor types for window.voussoir
  .gitignore                   ignores dist/, target/, node_modules/
  native/
    Cargo.toml                 shell crate; selects voussoir features
    src/main.rs                the App builder: config, assets, hooks, run
    src/assets.rs              generated asset table, never edit by hand
    commands/
      Cargo.toml               cdylib in dev, static lib in release
      src/lib.rs               your command table, hot-reloaded in dev
  android/                     Gradle project, scaffolded by init
  ios/                         Xcode project, scaffolded by init
```

## Who writes what

This is the part that saves confusion: three of these files are machine-owned.

| File | Written by | Edit it? |
| --- | --- | --- |
| `voussoir.json` | you, once at init | yes |
| `index.html`, `src/*` | you | yes |
| `tsconfig.json`, `.gitignore` | `init` | yes, they are yours |
| `native/src/main.rs` | you, after init | yes |
| `native/commands/src/lib.rs` | you, after init | yes |
| `native/src/assets.rs` | `voussoir dev` / `build` | **no**, overwritten |
| `native/Cargo.toml` | `init`, then you for features | carefully |
| `dist/` | `voussoir build` | no, regenerated |
| `android/`, `ios/` | `init` | yes, but regenerate if stuck |

`assets.rs` is the one people edit by accident, because it looks like a normal
Rust list. It is regenerated on every build; your edit vanishes silently.

## Theory: why the frontend is embedded

`dist/` is not shipped. `voussoir build` turns every frontend file into a byte
array in `assets.rs`, and the shell answers requests for your origin from that
table. Three consequences:

- **There is no missing-file bug in a shipped app.** A file the build did not
  include is a compile-time omission, not a 404 on a user's machine.
- **The page cannot be edited by the user.** Nothing is read from disk at runtime,
  so "fixing" the app means replacing the binary.
- **Startup cannot fail on a bad path.** The working directory is irrelevant; the
  app does not need to know where it is installed.

The trade is that the frontend cannot be updated without rebuilding the shell. For
an app whose UI changes every release that is free; for a plugin-shaped app it is
the reason [plugins](plugins.md) are Rust rather than JavaScript.

## `voussoir.json` in full

```json
{
  "name": "Notes",
  "title": "Notes",
  "width": 900,
  "height": 680,
  "entry": "index.html",
  "command_timeout_ms": 30000,
  "commands": ["greet", "storage.get", "storage.set"],
  "windows": { "preview": { "commands": ["greet"] } },
  "update": { "url": "https://example.com/releases.json" }
}
```

Every key is optional. Each one, and what happens when it is absent:

- **`name`** — the app identity. Used for the window title, the data directory, the
  package name and the Android package stem. Absent: `"Voussoir App"`.
- **`title`** — the window title. Absent: falls back to `name`.
- **`width` / `height`** — initial window size in logical pixels. Absent: 800×600,
  which is `Config::default` rather than whatever the scaffold happens to write.
- **`entry`** — the page the first window loads, relative to the served root. A
  leading `/` is stripped. Absent: `index.html`.
- **`command_timeout_ms`** — how long the page waits before rejecting with
  `kind: "timeout"`. Default 30000. `0` disables the deadline. A page can override
  per call: `invoke("slow", payload, { timeout: 300000 })`. The rejection does not
  cancel the handler, so the work may still complete.
- **`commands`** — the global allowlist. **Absent means everything registered**,
  which surprises people: leaving it out does not lock anything down.
- **`windows`** — per-label narrowing. Intersected with `commands`, so it can only
  remove. See [Per-window allowlists](allowlists.md).
- **`update.url`** — the JSON endpoint `update.check` reads. Absent: the command
  answers `{"configured": false, "available": false}` rather than failing, so an app
  can ship before its update feed exists.

`include_str!` embeds the file at compile time, so a shipped app never reads
configuration from disk. A writable working directory cannot redirect it, and
editing `voussoir.json` after a build has no effect until you rebuild.

## Why the commands crate is separate

`native/commands` is a `cdylib` in development and a static library in release.
That is the whole hot-reload mechanism: `voussoir dev` rebuilds *only* that crate
and swaps the shared object into the running process, so you keep the window
state and the webview session.

The cost is one rule that bites everyone once: **statics in that crate do not
survive a reload**, because the library that owned them is unloaded and a new one
is loaded. A `static COUNTER: AtomicU64` silently resets on every save.

State that must survive belongs to the shell:

```rust
voussoir::state::set("count", json!(n));
let n = voussoir::state::get("count");
```

`voussoir::state` is a process-wide JSON map that a reload never touches, and it
works in release builds too, so you do not need two versions of the code. See
[Hot reload in development](hot-reload.md).

## Which features the scaffold enables

`native/Cargo.toml` is generated with:

```toml
voussoir = { path = "...", features = ["full", "storage"] }
```

- `full` is `menu + dialogs + updater + devtools + storage`.
- `storage` is therefore already in `full`, and is listed again so that turning
  `full` down still leaves a store an app can use.

Turn them down if you do not use them: each one links a platform crate. Turn
`tray` on separately; it is not part of `full` because it implies a background
process model (see [Native integration](native-integration.md)).

## Theory: why `include_str!` and not a config file read

`voussoir.json` is compiled in. A shipped app therefore cannot be reconfigured by
editing a file next to the binary, which closes a class of attack: if the config
were read at startup, whoever could write to the install directory could widen the
allowlist or point `update.url` at their server, and the app would load it.

The cost is that changing the config needs a rebuild. That is the right cost for a
setting that decides what the app may do; a setting the user changes at runtime
belongs in [storage](storage.md), not here.

## Adding a page

Two kinds of page, and the difference matters:

1. **A second document** (`settings.html`): add the file, and it is served from
   the same origin. Link to it with a relative `<a href>`. A full load, so
   in-memory state is lost.
2. **A second view** in `index.html`: swap DOM, no load, state kept.

Most apps want (2) and open a *window* rather than navigate when they need a
separate surface; see [Multi-window](multi-window.md).

## Adding a dependency

- **Frontend**: `bun add <pkg>`; Bun bundles it. No Node built-ins at runtime.
- **Rust**: edit `native/Cargo.toml` and rebuild; a change here needs a restart,
  not a hot reload.

A Rust dependency that grows the command surface should be exposed as commands,
not as a new transport. That keeps it behind the allowlist:
[Plugins](plugins.md).

## Next

- [Commands & payloads](ipc-contract.md) — the contract the two halves share.
- [The frontend](frontend.md) — what Bun does with `src/`.
