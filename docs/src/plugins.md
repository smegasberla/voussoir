# Plugins

A plugin is a crate that implements one trait. There is no plugin system beyond
that: no manifest, no capability file, no discovery at runtime, and no macro.

## Theory: what a plugin is here, and what it is not

In most ecosystems, "plugin" means a binary the host finds and loads, with a
manifest describing what it offers. That model exists because the host does not
know its plugins at compile time.

A Voussoir app *does* know them: they are Cargo dependencies. So the machinery
that usually surrounds plugins buys nothing, and costs a second source of truth
(a manifest that can disagree with the code) plus a version handshake the
compiler already performs.

| | Plugin with manifest | Plugin as a trait (this) |
| --- | --- | --- |
| Discovery | runtime, by path | the linker |
| Version check | handshake | Cargo |
| Missing plugin | runtime error | compile error |
| Trust | must be sandboxed | it is your process |

The trade you accept: a plugin is **not** sandboxed. It can do anything the app
can, because it is the app. If a dependency cannot be trusted with that, it should
not be linked in at all.

## The trait

```rust
use voussoir::{App, Plugin};
use serde_json::Value;

pub struct Sqlite {
    pub path: std::path::PathBuf,
}

impl Plugin for Sqlite {
    fn name(&self) -> &'static str {
        "sqlite"
    }

    fn install(self, app: App) -> App {
        let path = self.path;
        app.command("sqlite.query", move |_app, payload| {
            let _ = &path;
            // ...real work here
            Ok(payload)
        })
    }
}
```

Two methods, each with a reason:

- **`name`** is a short, stable identifier (`"sqlite"`, `"dialog"`). It appears
  in error messages and diagnostics, never in lookup. It is not a namespace.
- **`install(self, app) -> App`** consumes the plugin and returns the app. Taking
  and returning rather than mutating means a plugin composes with the builder
  chain and cannot be installed twice by accident.

The app installs it with one call:

```rust
App::new().plugin(Sqlite { path: "app.db".into() }).run()
```

Order matters only for name collisions, which brings up the one sharp edge.

## Theory: names are global, and last wins

Registration is last-wins. Two plugins that both claim `open` will silently
replace each other, and the app will call whichever was installed last. There is
no error, because the registry is a map.

Two habits prevent it:

1. **Prefix with the plugin name:** `sqlite.query`, `dialog.open`, `tray.sync`.
2. **Defer when someone else got there first**, if that is the correct behaviour:

```rust
fn install(self, app: App) -> App {
    if app.has_command("sqlite.query") {
        return app;
    }
    app.command("sqlite.query", /* ... */)
}
```

`has_command` answers for the registry alone and **ignores the allowlist**, so a
registered command that a config withholds is still registered. That is the right
answer for a plugin deciding whether to install: otherwise the decision would
depend on which window asked, and there is no window at install time.

## Theory: a plugin is not a security boundary

Because a plugin is Rust in your process, it holds the app's authority: it can
read the disk, open a socket, and register any command name. Isolation would mean
a separate process or a wasm runtime, and neither is free.

The consequence for design: **the plugin's commands are the boundary, not the
plugin.** Once registered, they are subject to the allowlist like everything else,
so the app author can decide per window which of a plugin's commands exist.

```json
{
  "commands": ["sqlite.query"],
  "windows": { "preview": { "commands": [] } }
}
```

The preview window cannot reach the database, and the plugin did not have to
cooperate for that to be true.

## Why not a macro

A `#[plugin]` attribute would hide two ordinary lines and add a build-dependency.
The trait is small enough to implement by hand, which is what you want when the
plugin also holds configuration:

```rust
pub struct Sqlite { pub path: std::path::PathBuf, pub read_only: bool }
```

Implementing by hand lets `install` read `self.read_only` and close the write path
before registering anything, which is exactly the kind of decision a macro would
have to special-case.

## Packaging a plugin

There is nothing to package. A plugin is a crate your app depends on, so:

- **It is compiled into the binary.** No files to ship beside the app.
- **It appears in `cargo tree`** like any dependency, which is also how you check
  a release build does not pull something you did not intend.
- **Its commands hot-reload only if they are in the command table**, not when
  registered on `App` in `main.rs`.

## Next

- [Native integration](native-integration.md) — the built-in plugins, in effect.
- [Per-window allowlists](allowlists.md) — gating a plugin's commands per window.
