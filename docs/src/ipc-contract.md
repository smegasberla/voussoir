# Commands & payloads

Every call between the page and Rust goes through one envelope, in both
directions, over one endpoint. This chapter is the contract itself; everything
else in the book builds on it.

If you read only one chapter, read this one.

## Theory: why one shape for everything

The obvious design is a different channel per feature: a callback for file
dialogs, a global for the tray, a browser API for storage. Every one of those
would need its own error convention, its own security story and its own
diagnostics, and each would be a fresh place for a page to reach something it
should not.

Voussoir instead has **one request shape and one response shape**, and every
feature is expressed as a command:

```
POST voussoir://localhost/__voussoir/<method>
body:     <payload as JSON, or null>
response: {"ok": <json>}
        | {"error": {"message": "<text>", "kind": "<class>", "method": "<name>"}}
```

From the page, that is always the same call:

```ts
const result = await window.voussoir.invoke("any.method", payload);
```

Three properties follow, and each one is why a later chapter has a rule:

1. **A command exists only if Rust registered it.** No manifest, no capability
   file, no runtime discovery. Registration *is* existence.
2. **One envelope means one error shape.** A bad payload, a returned `Err`, a
   panic and a policy denial all arrive as `{error}`, told apart by `kind`.
3. **The registry is a chokepoint.** Because every call passes through it, a
   single allowlist can gate the whole surface, per window, without the page
   cooperating.

Remove any of those and the security model in this book stops working.

## The `kind` field

`kind` is a stable, machine-readable class. Branch on it; never match `message`
text, which is written for a human and may change.

| `kind` | Meaning | What the page should do |
| --- | --- | --- |
| `unknown` | no handler, or one withheld by an allowlist | treat the command as absent; do not retry |
| `payload` | the request did not match the handler's argument type | a bug in the page; log it |
| `handler` | the handler returned `Err` | show the message; it is meant for a user |
| `panic` | the handler panicked; the app kept running | show the message, report it |
| `busy` | too many worker-thread commands in flight | retry shortly; nothing was started |
| `timeout` | the shim gave up waiting | the command may still be running |
| `runtime` | the webview or event loop failed | fatal for that window; offer a restart |

`busy` and `runtime` come from the shell. `timeout` is produced by the shim
itself, so it is the one kind that can arrive for a command that never reached
Rust at all.

**Why is a denial reported as `unknown`?** Because a page that can distinguish
"not allowed" from "does not exist" has learned part of the app's surface. A
preview window with a small allowlist should not be able to enumerate what it is
missing, so both answers are identical from inside the webview.

## Theory: three execution models

A webview paints on a UI thread. A command that blocks that thread freezes the
window, so *where* a command runs is a decision you make per command, not a
detail:

| Model | Registered with | Runs on | Use for |
| --- | --- | --- | --- |
| Sync | `.command` | UI thread | work under a frame: formatting, lookups |
| Worker | `.async_command` | a thread pool | blocking I/O: files, sockets, subprocesses |
| Async | `.async_fn` | the shared async runtime | `await`-friendly libraries |

```rust
App::new()
    // Cheap, UI thread.
    .command("greet", |_app, payload| Ok(payload))
    // Blocking I/O on a worker thread: the window keeps painting.
    .async_command("read_file", |_app, payload| {
        let path = payload["path"].as_str().unwrap_or_default();
        // `?` will not do: Error has no From<io::Error>, because the message
        // travels to the page and an OS error string is not one a person reads.
        let text = std::fs::read_to_string(path)
            .map_err(|e| Error::Handler(format!("could not read {path}: {e}")))?;
        Ok(json!(text))
    })
    // A real async fn on the shared runtime (needs the `rt` feature).
    // `?` will not do here either: `Error` implements `From` for wry's error
    // only, so a database error has to be converted and named.
    .async_fn("lookup", |_app, payload| async move {
        let row = db
            .get(payload)
            .await
            .map_err(|e| Error::Handler(format!("lookup failed: {e}")))?;
        Ok(json!(row))
    })
```

The trade behind the table:

- **Sync is fastest** (a function call) and the only one that can stutter a
  window. Keep it under ~10 ms.
- **Worker spawns a thread per call.** Fine for a handful of concurrent calls;
  wasteful for a thousand. When the pool is full you get `kind: "busy"`.
- **Async submits a task** to one shared runtime, so it costs a task rather than
  a stack. This is the one that scales, at the price of a runtime in the binary
  (the `rt` feature).

Picking wrong is not a correctness bug. It is a responsiveness bug (sync) or a
capacity bug (worker), and both are visible in `--verbose`:
[Diagnostics](diagnostics.md).

## Registering commands two ways

There are two places a command can live, and knowing which you need saves time.

### The command table, in the commands crate

This is where most commands belong, because it is what hot-reloads:

```rust
// native/commands/src/lib.rs
// The commands crate depends on `serde_json` and nothing else: keeping wry, tao
// and gtk out of its dependency tree is what makes a dev rebuild take a second.
use serde_json::{json, Value};

// `Command` and `Kind` are declared just above this table, in this file.

fn greet(payload: &Value) -> Result<Value, String> {
    let name = payload.get("name").and_then(Value::as_str).unwrap_or("world");
    Ok(json!(format!("Hello, {name}!")))
}

pub static COMMANDS: &[Command] = &[
    Command { name: "greet", kind: Kind::Sync, run: greet },
];
```

`Command` and `Kind` are **generated into your crate**, not exported by the
framework, because this file is compiled twice: as a `cdylib` the dev shell loads
at runtime, and as a static library in release. A `cdylib` cannot share Rust
types with the shell, so the table is a plain data structure and the payloads are
JSON.

- **`name`** is what the page calls. It is global; two commands with one name
  silently replace each other, last one winning. Prefix (`sqlite.query`,
  `dialog.open`).
- **`kind`** is `Sync` or `Async`, and only affects release builds: under
  `voussoir dev` every command runs on a worker anyway, so a slow command cannot
  freeze the window mid-reload.
- **`run`** is `fn(&Value) -> Result<Value, String>`. Returning `Err(message)`
  becomes `kind: "handler"` with that message, so write it for a user.

### On the `App`, in `main.rs`

Use this when the handler needs the app itself, or a typed argument:

```rust
// native/src/main.rs
App::new()
    .command("open_settings", |app, _payload| {
        let label = app.open_window(Default::default())?;
        Ok(json!({ "label": label }))
    })
    .run()
```

`App::command` handlers receive `&AppHandle`, which is how a command emits an
event or opens a window. They do not hot-reload: changing one needs a restart.

**The rule of thumb:** stateless logic goes in the table; anything that needs the
app goes on `App`. A command that must be both is written as two: the table entry
takes `Value`, and a thin `App` wrapper calls into it.

## The `App` builder, in order

```rust
mod assets;   // the generated table: `pub static ASSETS: AssetTable = &[...]`

fn main() -> Result<(), voussoir::Error> {
    voussoir::App::new()
        .config(include_str!("../../voussoir.json"))   // 1. configuration
        .assets(assets::ASSETS)                        // 2. embedded frontend
        .on_ready(|app, label| { /* 3. per-window setup */ })
        .command("greet", |_app, payload| Ok(payload)) // 4. commands
        .run()                                         // 5. run the loop
}
```

`assets::ASSETS` is the generated table, not the type: `AssetTable` is an alias for
`&'static [(&'static str, &'static [u8])]`, so the argument is a slice of
`(path, bytes)` pairs. On a fresh scaffold the table is **empty** until
`voussoir dev` or `voussoir build` fills it, which is why a bare `cargo build`
succeeds on a new project and simply has no page to serve.

Each step is a value transformation, not a mutation, which is why the chain
reads top to bottom and why a plugin can slot into the middle
([Plugins](plugins.md)).

`run()` blocks on the event loop until the last window closes. Everything before
it is configuration; nothing before it needs a window to exist.

## Emitting events: Rust to page

The page calls Rust with `invoke`; Rust calls the page with an event:

```rust
app.emit("theme", json!({ "dark": true }));                     // every window
app.emit_to(Some("settings"), "saved", json!({ "path": "..." })); // one window
app.emit_to(None, "shutdown", json!({ "in": 5 }));               // every window
```

```ts
window.voussoir.listen<{ dark: boolean }>("theme", (payload) => {
  document.documentElement.dataset.theme = payload.dark ? "dark" : "light";
});
```

`emit_to(Some(label), ...)` reaches one window; `emit(...)` and
`emit_to(None, ...)` both broadcast to every window (`emit` is literally
`emit_to(None, ...)` with a shorter name).

All three go through the event loop proxy, so they are safe from any thread and
are **queued**, not synchronous: the handler runs on the page's next turn, not
inside your Rust function.

Queued is not the same as buffered, though. Delivery is a script evaluation in
each matching webview, with a no-op guard in the shim, so an event sent before a
page has installed its listener is dropped silently. That is why initial state
goes out from `on_ready` and not from `main`.

That has one consequence worth internalising: emitting is not a way to get a
value back. If you need an answer, that is a command the page calls.

## Why not just expose functions

A reasonable question: why not let the page call Rust functions directly, as some
frameworks do? Because a generated binding per function makes the command surface
implicit: it grows whenever you add a public function, and the allowlist has
nothing stable to name. With one registry, the surface is exactly the set of
names you registered, which is what makes
[Per-window allowlists](allowlists.md) enforceable.

## Next

- [Typed commands](typed-commands.md) — moving deserialization in front of the handler.
- [Errors the page can branch on](errors.md) — every kind, from the page's side.
- [Security model](security.md) — what the chokepoint buys you.
