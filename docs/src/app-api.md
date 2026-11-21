# The `App` builder, method by method

`App` is the only type you build directly. Everything else — commands, windows,
menus, hooks — is a method on it, and `run()` is the last one you call.

This chapter is a reference with the reasoning attached: what each method does,
what it costs, and the mistake it prevents.

## Theory: why a builder and not a config struct

The alternative is a struct you fill in and hand over:

```rust
// The shape Voussoir does not use.
let options = AppOptions { /* forty fields, most None */ };
App::start(options)
```

A builder is better here for three reasons, and only the third is about
ergonomics:

1. **Compile-time state.** `run()` consumes `App`, so you cannot register a
   command after starting. That is not a style rule; `run()` moves the registry
   into the event loop.
2. **Per-method validation.** `config()` can reject bad JSON and say which key.
   A struct literal has nowhere to put that check.
3. **Discoverability.** `app.` followed by an editor completion is the reference
   for every method, and it cannot be out of date.

The cost is that the type carries no proof of what has been called. Calling
`.assets()` twice compiles and the second wins. That is the trade: a smaller
surface traded for fewer type-level guarantees.

## Thinking about order

The methods fall into five groups, and within a group order almost never matters:

| Group | Methods | When it takes effect |
| --- | --- | --- |
| Identity | `config`, `assets` | at `run()` |
| Commands | `command`, `command_typed`, `async_command`, `async_fn`, `plugin` | at `run()`, into the registry |
| Windows | `entry`, `window` | at `run()`, for the first window |
| Hooks | `on_ready`, `on_window_close` | registered now, fire later |
| Diagnostics | `report_log`, `report_sink`, `devtools` | at `run()` |

One real ordering rule: **register commands before `run()`, obviously, and
register a hook before the window it is about exists.** A window opened from a
command is covered by `on_ready` only if the hook was installed on the `App` that
is running, which it always is because `run()` consumed it.

## Identity

### `config(&str)`

Takes the contents of `voussoir.json`, normally as `include_str!`.

```rust
App::new().config(include_str!("../../voussoir.json"))
```

**Invalid config exits the process**, printing which key failed:

```rust
match Config::from_json(json) {
    Ok(config) => self.config = config,
    Err(message) => {
        eprintln!("voussoir: {message}");
        std::process::exit(1);
    }
}
```

That is deliberate and worth understanding: a config parsed leniently would let a
typo in `width` silently produce a default window, and the bug would surface as
"the size I set does nothing" days later. A build that embeds a broken config is a
programming error, so it fails at startup where you can see it.

### `assets(AssetTable)`

Takes the generated table of frontend files (see
[How a project is laid out](layout.md)).

```rust
.mod assets;
App::new().assets(assets::ASSETS)
```

Without it the app has no page to serve and answers 404 for every request. It is
not optional, and it is not read from disk: the files were embedded at compile
time by `voussoir build`.

## Commands

Four methods, one difference: **where the body runs.** That is the whole choice,
so it is worth a table before the details.

| Method | Runs on | Payload | Feature | Use for |
| --- | --- | --- | --- | --- |
| `command` | the UI thread | `Value` | always | instant work: formatting, reading state |
| `command_typed` | the UI thread | your `Deserialize` type | always | same, with validated input |
| `async_command` | a worker thread | `Value` | always | blocking work: files, network |
| `async_fn` | the shared runtime | your `Deserialize` type | `rt` | awaited work: many concurrent calls |

### `command(name, handler)`

```rust
App::new().command("greet", |_app, payload| {
    let name = payload.get("name").and_then(|v| v.as_str()).unwrap_or("world");
    Ok(json!(format!("Hello, {name}!")))
})
```

The handler is `Fn(&AppHandle, Value) -> Result<Value, Error>`.

**The UI-thread detail is the one to internalise.** This runs while the window
cannot paint, so a 200 ms handler is a visible stutter and a 5 s handler looks
like a freeze. Keep it to microseconds; if you are unsure, use
`async_command` and measure later.

A panic here does not bring the window down: the handler is wrapped so the page
receives `kind: "panic"` and the app keeps running (see
[Errors the page can branch on](errors.md)).

### `command_typed(name, handler)`

Same thread, typed payload:

```rust
#[derive(voussoir::serde::Deserialize)]
#[serde(crate = "voussoir::serde")]
struct GreetArgs {
    #[serde(default)]
    name: Option<String>,
}

App::new().command_typed("greet", |_app, args: GreetArgs| {
    Ok(json!(format!("Hello, {}!", args.name.unwrap_or_else(|| "world".into()))))
})
```

The reply type is free (`R: Serialize`), so `Ok(format!(...))` also compiles. The
`json!` wrapper here is for consistency: a page that always receives JSON from a
command has one shape to parse, and returning a bare string for one command means
one `typeof` check in the frontend that will be forgotten.

Prefer this as soon as a payload has more than one field. The difference shows up
the first time a page sends `{"name": 7}`: untyped code silently falls back to
`"world"`, typed code returns `kind: "payload"` naming the command and the field.
One is a mystery bug, the other is a diagnosis.

**`#[serde(crate = "voussoir::serde")]` is required on the derive.** An app depends
on `voussoir` only, never on `serde` directly, so the generated code has to be told
where the crate lives. `voussoir::serde` is a re-export for exactly this purpose.

`#[serde(default)]` on a field is worth knowing as a design decision: it says
"omitted means this value, not an error". Without it, a page that leaves the field
out gets a rejection. Both are defensible; be explicit about which you meant.

### `async_command(name, handler)`

```rust
App::new().async_command("read_file", |_app, payload| {
    let path = payload
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Payload("read_file needs a \"path\"".to_owned()))?;

    // `?` does not work on io::Error: the only `From` impl on Error is for
    // wry::Error, because a handler error is a message for the page and an OS
    // error string is not one. Convert explicitly and say what failed.
    let text = std::fs::read_to_string(path)
        .map_err(|e| Error::Handler(format!("could not read {path}: {e}")))?;

    Ok(json!(text))
})
```

**Why there is no `From<io::Error>`:** `Error::Handler` carries text that goes
straight to the page as `message`, and `os error 13` is not a sentence a person can
act on. Requiring the conversion forces you to decide what the user should read,
which is the one thing the framework cannot decide for you.

Runs on a worker thread, so blocking is the point rather than a sin. The reply is
delivered through the same envelope, so the page cannot tell the difference.

Three facts that decide when to use it:

- **A thread is spawned per call.** Cheap enough for a handful, wasteful for
  hundreds. Stacks are megabytes of address space each.
- **There is a cap**, `available_parallelism` with a floor of 4. Past it, requests
  are refused with `kind: "busy"` rather than queued forever.
- **`busy` is retryable.** The request was refused before any work began, so a
  retry cannot double a side effect. See
  [Errors the page can branch on](errors.md).

### `async_fn(name, handler)` — needs `rt`

```rust
// Cargo.toml: voussoir = { features = ["rt"] }
App::new().async_fn("count_to", |_app, limit: u64| async move {
    let mut last = 0;
    for n in 0..limit {
        // Any library you like can be awaited here; this stand-in keeps the
        // example dependency-free.
        tokio::task::yield_now().await;
        last = n;
    }
    Ok(last)
})
```

The signature is `Fn(AppHandle, P) -> Future<Output = Result<R, Error>>`, so the
closure returns a future and the arguments are already deserialized. `P` is
deserialized on the **calling** thread, so a malformed argument is answered
immediately instead of costing a task.

A task on one shared multi-threaded runtime instead of a thread per call, which is
what makes a thousand concurrent calls reasonable.

**Why is this behind a feature?** Because it pulls in tokio's scheduler and timer
wheel. An app with only synchronous commands should not pay for a runtime it never
uses, and a feature is the only way to make that choice yours.

The runtime starts one worker per core, minimum two. The floor is not arbitrary: a
single worker plus a blocking future inside a task is a deadlock waiting for the
first timeout anyone adds.

### `plugin(plugin)`

Installs a `Plugin`, which is a crate implementing one trait. Covered on its own
page: [Plugins](plugins.md).

### `has_command(&self, name)`

```rust
if !app.has_command("sqlite.query") {
    // register a fallback, or skip wiring that needs it
}
```

Exists for plugins that should defer to an existing handler rather than replace it.
It answers for the **registry only** and ignores the allowlist, so it means
"exists", not "this window may call it". That separation is useful: a missing
handler is a bug, a withheld one is policy.

## Windows

### `entry(&str)`

The page the first window loads, relative to the served root.

```rust
App::new().entry("settings.html")
```

Overrides `entry` from `voussoir.json`. Leading slashes are trimmed, so
`"/settings.html"` and `"settings.html"` are the same page.

### `window(WindowOptions)`

Overrides the first window's label, title, size and entry in one call:

```rust
App::new().window(WindowOptions {
    label: Some("main".to_owned()),
    title: Some("Notes".to_owned()),
    width: Some(1100.0),
    height: Some(760.0),
    entry: None,
    allowlist: None,
})
```

`WindowOptions` is the same type `app.open_window(...)` takes later, because the
first window and a later one should not need two vocabularies. Fields are
`Option` because "unset" means "use the config value", which is different from
"use zero".

Note `allowlist: None` here means "the global surface", and a list can only remove.
See [Per-window allowlists](allowlists.md).

## Hooks

### `on_ready(handler)`

```rust
App::new().on_ready(|app, label| {
    app.emit_to(Some(label), "boot", json!(null));
})
```

Fires once per window, after that window's page has loaded and the shim is
installed, which is the earliest moment an event can reach a listener. It covers
windows opened later too.

**This is the correct place to emit initial state.** An `emit` before the page is
listening is dropped silently, because delivery is a script evaluation in the
webview. The hook exists to give you an ordering guarantee that the API cannot give
you otherwise.

### `on_window_close(handler)`

```rust
App::new().on_window_close(|app, label| {
    let open: Vec<_> = app.windows();
    state::set("open_windows", json!(open));
})
```

Fires once as a window goes away, whichever side closed it. It runs **before** the
window leaves the open list, so `app.windows()` inside the hook still includes it.

**Why not a `Drop` impl?** Because `Drop` runs when Rust drops the value, which is
not when the window closes. A captured value outlives the window; on some platforms
nothing is dropped at all. See [Lifecycle hooks](lifecycle.md).

## Diagnostics

### `report_log(path)` and `report_sink(f)`

`report_log` chooses **where** reports are written:

```rust
App::new().report_log("/tmp/notes.log")
```

Without it, reports are still recorded in memory and readable through
`__voussoir_reports`; they are just not persisted. `report_sink` takes a closure
instead, for when "write to a file" is not what you want.

Neither of them *records* anything, and there is no public function that does: the
reporting module is private. The IPC layer and the panic hook record for you, and
the three ways to read what they recorded are the `__voussoir_reports` command,
your `report_sink` closure, and the file `report_log` names.

### `devtools(bool)`

```rust
App::new().devtools(true)      // F12 / Cmd-Alt-I, even in release
```

Two independent conditions must hold: the `devtools` cargo feature is compiled in,
**and** this is called with `true`. That split means a library enabling the feature
does not hand an app a debug key. See
[Native integration](native-integration.md).

## Running

### `run() -> Result<(), Error>`

The end of the chain, and it does more than start a loop. In order:

1. Installs the panic hook, so a panic is recorded rather than only printed.
2. Reads the diagnostics environment (`VOUSSOIR_VERBOSE`).
3. Builds the event loop.
4. Sets the entry page from `entry`, `window`, or the config, in that precedence.
5. Registers the built-in commands.
6. Applies the global and per-window allowlists from the config.
7. **Audits the allowlist**, logging names no handler can answer.
8. Attaches the hot library in a dev build.
9. Runs the loop.

Step 7 is the one that turns a common mistake into a log line: an allowlist that
names a command you renamed says so at startup instead of looking like a missing
command at runtime.

### `hot_commands_from_env()`

Developed and documented in [Hot reload in development](hot-reload.md). You never
call it by hand: `voussoir dev` arranges for it to run.

## A complete `main`, annotated

This is the scaffold's `main.rs`, trimmed, with each line's reason:

```rust
fn main() -> Result<(), Error> {
    let mut app = App::new()
        .config(include_str!("../../voussoir.json"))
        .assets(assets::ASSETS)
        .command_typed("open_settings", |app: &AppHandle, args: SettingsArgs| {
            let label = app.open_window(WindowOptions {
                label: Some("settings".to_owned()),
                title: Some(args.title.unwrap_or_else(|| "Settings".to_owned())),
                width: Some(420.0),
                height: Some(320.0),
                entry: None,
                allowlist: None,
            })?;
            Ok(())
        })
        .devtools(true);

    for command in commands() {
        app = match command.kind {
            Kind::Sync => App::command(app, command.name, |_app, payload| {
                (command.run)(&payload).map_err(Error::Handler)
            }),
            Kind::Async => App::async_command(app, command.name, |_app, payload| {
                (command.run)(&payload).map_err(Error::Handler)
            }),
        };
    }

    app.run()
}
```

Four things worth noticing, each of which looks like a quirk and is not:

- **`App::command(app, ...)` rather than `app.command(...)`.** Commands from the
  table are registered in a loop, so the method is called as an associated function
  to keep `app` reusable. The loop must not look like a named registration,
  because the parity checker reads names from literals in the commands crate.
- **`.map_err(Error::Handler)`.** The table's `run` returns `Result<Value, String>`,
  a `String` error. Converting it once here keeps the table free of a dependency
  on the error type, which is what lets it cross the `cdylib` boundary.
- **`command_typed` in `main.rs` rather than the commands crate.** Shell-side
  commands take real types because they are compiled together with the shell;
  table commands stay JSON-to-JSON.
- **`.devtools(true)` last.** Order does not matter for it; placing it last reads as
  "a build setting", which is what it is.

## What is not on `App`

- **No `start`, `stop`, `shutdown`.** The process ends when the loop ends.
- **No `set_allowlist`.** The allowlist is config, or per window
  (`open_window`), because a page must never be able to change what it may call.
- **No window list.** That is `app.windows()` on an `AppHandle`, available from a
  command or a hook where a handle exists.
- **No `emit` on `App`.** Emitting needs a running loop; `App` is the thing that
  starts one.

## Next

- [Commands & payloads](ipc-contract.md) — the contract behind these handlers.
- [Multi-window](multi-window.md) — `open_window`, `windows()`, `close_window`.
