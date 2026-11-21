# In-process state

`voussoir::state` is a process-wide map from a string to a JSON value. It is small
on purpose, and it solves exactly one problem: state that must outlive a hot reload
but must not survive the process.

## Theory: three lifetimes, three tools

An app has state with three different lifetimes, and using the wrong tool for one of
them is the most common source of "it works, then it doesn't":

| Lifetime | Survives | Tool |
| --- | --- | --- |
| an expression | nothing | a local variable |
| a hot reload | a reload, not a restart | `voussoir::state` |
| a restart | the process, not a reinstall | `storage` |

`voussoir::state` is the middle one, and that middle exists for one reason: the
command crate is a shared library in development, so a `static` inside it is
destroyed and recreated on every save. See
[Hot reload in development](hot-reload.md).

Without this module, every counter, cache and connection pool in a command crate
would silently reset each time the developer pressed Ctrl-S, and the bug would
appear only in dev, only sometimes.

## The API

```rust
use voussoir::state;
use voussoir::serde_json::json;

state::set("count", json!(3));
let n = state::get("count");              // Option<Value>
state::remove("count");                   // Option<Value>, the old value
state::contains("count");                 // bool
state::keys();                            // Vec<String>, sorted
state::clear();
state::reload_generation();               // u64, the live library generation
```

Seven public functions, no generics, no lifetimes. Values are `serde_json::Value`,
so anything you can express as JSON you can store, and nothing else.

`clear()` is public but you should not call it in an app: its own documentation says
the shell calls it in tests, so a test cannot be affected by state another test left
behind. There is no per-key namespace either, so a plugin and the app that installs
it share one map and can collide on a key name.

**Why JSON and not `Any`:** because the caller reads it back as a different type
than it wrote. A typed store would need a lifetime parameter or unsafe casts, and
would still have to answer "what if the code that reads differs from the code that
wrote". JSON makes that question explicit: parse, and handle the failure.

**Why no removal on read:** `get` clones. If you want take-and-delete, that is
`remove`, which is a separate call so a panic between the two cannot lose the value
without the caller knowing.

## Theory: what a generation is

`reload_generation()` increments each time a new command library becomes live. It is
the honest answer to "is the code I am running the code I just saved".

The worked use is a connection pool: it should not be rebuilt on every call, and it
must be rebuilt when the function that built it changed.

```rust
use std::sync::{Mutex, OnceLock};
use voussoir::state;

static POOL: OnceLock<Mutex<(u64, Pool)>> = OnceLock::new();

fn pool() -> std::sync::MutexGuard<'static, (u64, Pool)> {
    let gen_now = state::reload_generation();
    let mut guard = POOL.get_or_init(|| Mutex::new((0, Pool::new()))).lock().unwrap();
    if guard.0 != gen_now {
        *guard = (gen_now, Pool::new());
    }
    guard
}
```

The `POOL` static itself lives in the command library, so on a reload it is
recreated empty and the generation check is redundant there. It earns its place in
the other direction: a value the *shell* created and the command crate only borrows
cannot be rebuilt by a reload, so the crate needs a way to tell "the code holding me
changed" without throwing the borrowed value away. That is the case this pattern is
for: the generation says the code that asked differs, and you decide whether that
matters.

## Theory: why this is not a global in the bad sense

Mutable global state has a bad reputation, and it is deserved: globals make tests
order-dependent and concurrency invisible. Three things keep this one honest:

1. **It is keyed.** Not one global, a namespace. Two features can hold state without
   knowing about each other.
2. **It is explicitly scoped.** The name says when it dies: with the process. There
   is no pretence that it is durable.
3. **It is opt-in per value.** Nothing in the framework stores anything there for
   you, so the set of keys is exactly what your app put in it.

What it still is: unsynchronised between commands. `set` and `get` are individually
safe, but a read-modify-write across two calls is not atomic:

```rust
// Two commands can interleave here and lose one increment.
let n = state::get("count").and_then(|v| v.as_u64()).unwrap_or(0);
state::set("count", json!(n + 1));
```

When that matters, hold a `Mutex` in the shell and pass it through, or keep the
counter behind one command. The state module is storage, not a lock.

## When to use it

| Situation | Use |
| --- | --- |
| a counter or cache in a command | `state` |
| a connection pool built by the shell | `state`, keyed by something |
| a setting the user chose | [storage](storage.md): it must survive a restart |
| a value one function uses | a local |
| shared mutable state under concurrency | your own `Mutex`, in the shell |

The third row is the one people get wrong. `state` is not persistence; it is memory
with a longer life than a `static` in a rebuilt library.

## Debugging it

```rust
// Print every key and its size, which is usually enough to spot the leak.
for key in state::keys() {
    let v = state::get(&key).unwrap_or_default();
    eprintln!("{key}: {}", v.to_string().len());
}
```

Two symptoms with the same cause:

- **Memory grows across a long session.** A key is being appended to rather than
  replaced.
- **A value looks stale after a reload.** It is in `state`, which is by design; if
  you wanted it rebuilt, keep it in a `static`.

## Next

- [Hot reload in development](hot-reload.md) — why the middle lifetime exists.
- [Storage](storage.md) — the durable one.
