# Storage

Small apps need to remember small things: the last opened file, a theme, a
window size, whether the user dismissed a hint. Voussoir ships one tiny store for
that, and deliberately nothing more.

## Theory: why a JSON file and not a database

Adding SQLite to every app costs a dependency, a migration story, a file format
and a backup story. Most apps need a map from a string to a JSON value, and a
single file written atomically does that with none of those costs.

The trade you accept:

| | One JSON file | Embedded database |
| --- | --- | --- |
| Cost | none beyond std | a native dependency |
| Size limit | 4 MB, enforced | gigabytes |
| Concurrency | one process, serialized | multi-reader |
| Queries | none: read the whole map | SQL |
| Corruption risk | low, atomic rename | lower |

The decision rule: if you are tempted to work around the 4 MB cap or the lack of
queries, you have outgrown it. Add SQLite behind your own commands; that is
supported and the allowlist treats it like any other command.

## Where the file is

```
<platform data dir>/<app name>/storage.json
```

- Linux: `~/.local/share/<app name>/storage.json`
- macOS: `~/Library/Application Support/<app name>/storage.json`
- Windows: `%APPDATA%\<app name>\storage.json`
- Android: the app's private data directory
- iOS: the app's Documents container

`<app name>` is `name` in `voussoir.json`. Renaming the app therefore moves the
store, so rename before you ship, not after.

Ask the app where it is rather than reconstructing the path:

```ts
const { path } = await window.voussoir.invoke<{ path: string }>("storage.path");
```

## The five commands

They are registered by the `storage` feature. `voussoir init` enables it for you
(`features = ["full", "storage"]` in `native/Cargo.toml`), so a scaffolded app has
them immediately. On a hand-written dependency, add it or the commands do not
exist and every call answers `kind: "unknown"`:

| Command | Payload | Resolves to |
| --- | --- | --- |
| `storage.get` | `{ key }` | the stored value, or `null` |
| `storage.set` | `{ key, value }` | the previous value, or `null` |
| `storage.delete` | `{ key }` | `true` if something was removed |
| `storage.keys` | none | the list of keys |
| `storage.path` | none | the file's absolute path |

```ts
await window.voussoir.invoke("storage.set", { key: "theme", value: "dark" });
const theme = await window.voussoir.invoke<string | null>("storage.get", { key: "theme" });
```

`storage.set` answering with the *previous* value is deliberate: it lets you
undo, and it means a UI that mirrors the store never needs a read before a write.

Values are any JSON value, nested objects included:

```ts
await window.voussoir.invoke("storage.set", {
  key: "window",
  value: { w: 900, h: 680 },
});
```

## Theory: when a write becomes durable

`storage.set` writes the whole map to disk and flushes it before it resolves. So
when your promise resolves, the bytes are on disk, not merely buffered.

The write is three steps, and each one exists to make a crash survivable:

1. **Serialize** the whole map, pretty-printed.
2. **Check the cap** against those bytes, before touching the disk.
3. **Write `storage.json.tmp`, `sync_all`, then rename** over `storage.json`.

The rename is the commit point. On every platform `rename` within one filesystem is
atomic, so a reader sees either the old file or the new one, never a half-written
one. Without it, a crash mid-write would leave a truncated JSON file and the app
would start with no settings at all, which reads to a user as "it forgot everything".

`sync_all` before the rename is what makes the claim honest: it waits for the bytes
to reach the device rather than the operating system's cache, so a power loss after
the promise resolves does not lose the write. That is also why each `set` costs a
real disk flush: fine for a settings toggle, wrong for something that fires per
keystroke.

For anything frequent, keep the value in memory and write on a boundary:

```ts
let draft = "";
input.addEventListener("input", (e) => { draft = (e.target as HTMLInputElement).value; });

// On blur, or on a save button, or in on_window_close from Rust.
input.addEventListener("blur", () => {
  void window.voussoir.invoke("storage.set", { key: "draft", value: draft });
});
```

**Every read parses the file, and every write replaces it.** Nothing is cached
between calls: `store()` caches only the *handle* (the resolved path), not the
contents, so `get` re-reads and re-parses `storage.json` each time.

That is a deliberate trade. A cache would mean deciding when to invalidate it, and
the failure mode of a stale cache (a setting that refuses to change until restart)
is worse than the cost of parsing a settings file. It also means hand-editing
`storage.json` while the app runs *does* show up on the next `get`, and that two
processes are not protected from each other by a lock.

The cost lands on the pattern from the previous section: do not call `get` in a
render loop. Read once when the page loads and keep the value in a variable.

## Theory: the 4 MB cap

A write that would take the file past 4 MB is refused with an error rather than
silently filling the user's disk. The reason is that a store that large is
almost always a bug: a key derived from user input, or an array that grows once per
launch.

The cap is measured on the **serialized, pretty-printed** body, not on the sum of
your values, so indentation counts against you. A map of many small keys reaches
it sooner than the raw bytes suggest.

When you hit it, the fix is not a bigger cap. It is one of:

- store a path or an ID, and keep the payload in a file you own,
- keep large data in a real database behind your own command,
- delete keys you no longer need (`storage.keys` then `storage.delete`).

## Reading and writing from Rust

The Rust API is the same store, which is what you want when a command writes
state the page will later read:

```rust
use voussoir::storage::Store;

let store = Store::open("Notes")?;      // app name, same as voussoir.json
store.set("theme", serde_json::json!("dark"))?;
let theme = store.get("theme")?;
```

`Store::at(path)` opens a specific file, which the crate's own tests use.

## Theory: do not put secrets here

`storage.json` is a plain file in the user's data directory. It is readable by
the user and by anything running as the user, and on a desktop it is not
encrypted. Treat it as public to the person at the keyboard.

- Never store tokens, passwords or keys in it.
- If you must persist a credential, use the OS keychain (a separate crate and a
  command of your own).
- Anything you would not print to the terminal does not belong in storage.

## Clearing it

Removing the file resets the app, which is also what a "reset settings" feature
should do:

```rust
std::fs::remove_file(store.path())?;
```

The store tolerates a missing file and starts empty, so this is safe while the
app runs as long as you remove the file before the next write, not during one.

## Next

- [Lifecycle hooks](lifecycle.md) — where to save per-window state on close.
- [Security model](security.md) — why this file is not a secret store.
