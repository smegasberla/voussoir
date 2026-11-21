# Your first app

This chapter takes you from nothing to a window that talks to Rust, and explains
every step on the way. If you have never built a desktop app, that is the
audience here. If you already have, skip to
[Commands & payloads](ipc-contract.md).

## Theory: what you are about to build

Most "hello world" guides hide the machine from you. This one does not, because
Voussoir's shape decides how you write everything else.

A Voussoir app is **one executable** that contains two things:

1. **A Rust shell.** It opens a window, enforces a security policy, and holds
   your commands, which are plain Rust functions.
2. **A web page.** HTML, CSS and whatever JavaScript you write, rendered by the
   browser engine your operating system already has.

The two talk over one JSON channel. When your page calls `greet`, a Rust
function of that name runs, and its return value comes back as a promise result.
Nothing calls out to a server, and nothing is installed beside your app.

**Why this matters immediately:** if you forget to register a command in Rust,
the page cannot call it. There is no discovery, no manifest, no default set. The
error you will see says `unknown`, which is exactly right.

## Theory: what "build" means here

There are two builds, and confusing them is the most common early mistake:

| Build | Who runs it | What it does |
| --- | --- | --- |
| Frontend | Bun, inside `voussoir dev` / `build` | TypeScript/JSX/CSS → plain JS and CSS |
| Backend | Cargo, inside the same commands | Rust → a binary |

`voussoir dev` runs both, and keeps running them: it serves the frontend over
HTTP so the webview can hot-reload it, and rebuilds your commands when you save
Rust. `voussoir build` runs both once and produces the single file you ship.

You will not normally invoke `bun` or `cargo` yourself. The CLI exists so the
two are never out of step.

## Scaffold a project

```bash
voussoir init notes
cd notes
voussoir dev
```

That is the whole setup. `init` writes a working app with one example command.
`dev` starts the frontend server and launches the shell against it.

If `voussoir` is not installed yet, [Installing Voussoir](install.md) covers it,
including the one-line script that also installs Rust and Bun for you.

When `dev` starts you get:

- a window showing your page,
- a URL in the terminal for the dev server (default `http://127.0.0.1:5177`),
- and a watcher that rebuilds Rust when you save.

Close the window and the process exits. There is no daemon to stop.

## What you got

```
notes/
  voussoir.json                  name, window size, entry page, optional allowlist
  index.html                     entry page, bundled by Bun
  tsconfig.json                  TypeScript settings, for your editor only
  .gitignore                     ignores dist/, target/, node_modules/
  src/main.ts                    frontend code
  src/style.css                  styles
  src/voussoir.d.ts              types for window.voussoir, so the editor knows it
  native/Cargo.toml              the shell crate
  native/src/main.rs             the App builder: config, assets, hooks, run
  native/src/assets.rs           generated asset table, never edit by hand
  native/commands/Cargo.toml     the commands crate (cdylib in dev)
  native/commands/src/lib.rs     your command table, hot-reloaded in dev
  android/                       Gradle project, only if you build for Android
```

Two Rust files matter, for different reasons:

- **`native/commands/src/lib.rs`** holds the command table. `voussoir dev`
  rebuilds only this crate on save and swaps the result into the running window.
  This is where you will spend most of your time.
- **`native/src/main.rs`** builds the `App`: it loads `voussoir.json`, embeds
  the frontend, registers commands that need the app itself (opening a window,
  emitting an event), and calls `run()`. A change here needs a restart, because
  this file *is* the process.

**Why the split?** Recompiling one small crate is seconds; recompiling the shell
that links the webview is a minute. The scaffold puts the code you iterate on in
the crate that is cheap to rebuild.

## The smallest real app

The scaffolded command looks like this, in
`native/commands/src/lib.rs`:

```rust
use serde_json::{json, Value};

fn greet(payload: &Value) -> Result<Value, String> {
    let name = payload
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("world");
    Ok(json!(format!("Hello, {name}! (from Rust)")))
}
```

The `(from Rust)` suffix is there so you can see with your own eyes which half
answered. When you call it from the page and the text changes, the round trip
worked; if you had a mock or a cached value, it would not have.

Line by line, for a first read:

- **`payload: &Value`** is the JSON the page sent. It is `serde_json::Value`,
  the same type you already know from `serde_json`, because the wire format is
  JSON and nothing more specific has been promised yet.
- **`.get("name").and_then(Value::as_str).unwrap_or("world")`** is defensive
  reading. A missing field, or one that is not a string, becomes `"world"`
  instead of a panic. Commands should never panic on bad input.
- **`Result<Value, String>`** is the reply. `Ok` becomes `{"ok": ...}` on the
  wire, `Err(message)` becomes `{"error": {"message": ..., "kind": "handler"}}`.
  A `String` error is a message for a human, which is usually what you want
  first.
- **`json!(...)`** builds the return value. Returning a `String` directly is not
  possible: the reply must be JSON, so it is wrapped.

And the page side, in `src/main.ts`:

```ts
const message = await window.voussoir.invoke<string>("greet", { name: "you" });
document.querySelector("#result")!.textContent = message;
```

- **`window.voussoir`** is installed by the shell before any of your scripts
  run. There is no import, no provider, no bundler plugin.
- **`invoke`** returns a promise. It resolves to your `Ok` value, or rejects
  with an object carrying `kind`, `method` and `message`.
- **The type argument** `invoke<string>` is yours to state; TypeScript cannot see
  across the IPC boundary. It is a note to the reader, not a runtime check.

`index.html` is the entry. It loads `src/main.ts`, and Bun bundles whatever that
imports, so you never list script tags by hand.

## Making the promise fail on purpose

The fastest way to understand the contract is to break it:

```ts
try {
  await window.voussoir.invoke("does_not_exist");
} catch (e) {
  console.log(e.kind);   // "unknown"
  console.log(e.method); // "does_not_exist"
  console.log(e.message);
}
```

`unknown` covers both "no such command" and "an allowlist withheld it". That is
deliberate: a page must not be able to probe which commands exist but were not
granted to it. [Errors the page can branch on](errors.md) lists every kind.

## Adding your own command

Two edits, one in each half.

In `native/commands/src/lib.rs`, add the function and put it in the table:

```rust
fn save(payload: &Value) -> Result<Value, String> {
    let text = payload
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("");
    Ok(json!({ "bytes": text.len() }))
}

pub static COMMANDS: &[Command] = &[
    Command {
        name: "greet",
        kind: Kind::Sync,
        run: greet,
    },
    Command {
        name: "save",
        kind: Kind::Async,
        run: save,
    },
];
```

The scaffold's table is written one field per line because the shell loops over it
in `main.rs`, not because tooling parses it.

**What `voussoir build` does check is a different thing, and worth not confusing
with this.** It compares the command names registered in the **desktop** shell with
those in the **Android** shell, because one frontend and two shells means a command
missing from one of them fails only on the platform nobody tested. That check reads
`.command("name", ...)` call sites, not this `name: "..."` table, and it does not read
the frontend at all. So a typo in `src/main.ts` is not caught by it; it arrives as
`kind: "unknown"` at runtime. That is the `unknown` row in the failure table below.

The table already exists above `greet`, with `greet` in it; you are adding one
entry, not writing the list from scratch. `Command`, `Kind` and `COMMANDS` are
declared in this same file by `init`, which is why there is no `use` line for them:

```rust
pub struct Command {
    pub name: &'static str,
    pub kind: Kind,
    pub run: fn(&Value) -> Result<Value, String>,
}

pub enum Kind { Sync, Async }
```

That is the whole registration format: a name, a kind, and a function pointer. It
is a plain data table rather than a builder because this file is compiled as a
`cdylib` in development, and a shared library cannot hand the shell a Rust closure
with a concrete type. JSON in, JSON out is the boundary.

In `src/main.ts`, call it:

```ts
const { bytes } = await window.voussoir.invoke<{ bytes: number }>("save", {
  text: "hello",
});
```

**Why `Kind::Async` for `save` and `Kind::Sync` for `greet`?** `Sync` runs on the
UI thread, which is right for work under a frame. `Async` runs on a worker, which
is right for anything that touches the disk, because blocking the UI thread
freezes the window. If you are unsure, ask: could this take more than ~10 ms? If
yes, make it async.

Save the file. `dev` rebuilds the commands crate, swaps it into the live process,
and your new command exists within a few hundred milliseconds, with the window's
state intact. See [Hot reload in development](hot-reload.md) for why that works
and what it cannot preserve.

## Run, build, package

| Command | What it does |
| --- | --- |
| `voussoir dev` | Bun dev server with HMR, shell watching for Rust edits |
| `voussoir dev --verbose` | the same, plus structured diagnostics on stderr |
| `voussoir dev --port 8080` | a different dev server port |
| `voussoir build` | bundle the frontend, compile the release binary |
| `voussoir build --size` | the same, then print the binary size |
| `voussoir build --debug` | a debug-profile binary, for a usable backtrace |
| `voussoir package` | build, then wrap it in an installer for this OS |
| `voussoir doctor` | check the toolchain and say what is missing |

`voussoir doctor` is the first command to run when something does not build. It
reports the missing piece by name (Rust, Bun, the platform webview) instead of
letting a linker error surface two minutes later from a file you have not opened.

## Theory: what a release build actually contains

After `voussoir build`, the result is one binary. The frontend is not a folder
next to it; `build` writes `native/src/assets.rs`, a table of `(path, bytes)`
pairs that Cargo compiles in. That has three consequences:

1. You cannot break the app by moving it, because there is nothing relative to
   find.
2. Startup does no filesystem read for your page.
3. Editing the built file does not change the app; you must rebuild.

It also means the shipped app has no Node, no Bun and no bundled browser. The
only thing it borrows is the OS webview, already installed. `voussoir build
--size` exists so that claim stays checkable.

## Common first-run problems

| Symptom | Cause | Fix |
| --- | --- | --- |
| `voussoir: command not found` | not installed, or not on `PATH` | re-run the installer, or add `$HOME/.local/bin` |
| window opens blank | the entry page failed to bundle | read the terminal; Bun prints the error |
| `unknown` on a command you wrote | not in `COMMANDS`, or not saved | add it to the table, wait for the rebuild line |
| `doctor` names a missing webview | Linux without WebKitGTK | run the installer, or install `webkit2gtk-4.1` |
| a Rust edit does nothing | you edited `native/src/main.rs` | restart `dev`; only the commands crate hot-reloads |
| port already in use | another dev server | `voussoir dev --port 8080` |

A fuller list is in [Troubleshooting](troubleshooting.md).

## Where to go next

You now have the mental model. The order that matches how the pieces depend on
each other:

1. [How a project is laid out](layout.md) — every file, and why it is there.
2. [Commands & payloads](ipc-contract.md) — the one contract everything uses.
3. [The frontend](frontend.md) — what Bun does for you, and what it does not.
4. [Security model](security.md) — the policy you got for free.
5. [Multi-window](multi-window.md) — labels, allowlists, lifecycle hooks.
