# Hot reload in development

Hot reload is **desktop-only and development-only**. A release binary never loads a
dynamically linked command library: the same table that was hot-reloaded in dev is
linked statically into the shipped app.

That is checked rather than promised. The loader lives behind the `hot-reload`
cargo feature, which `voussoir dev` enables and `voussoir build` never does, so
`libloading` cannot appear in a release binary's dependency graph.

## Theory: why dynamic loading at all, and why only in dev

Two halves of your app have rebuild costs that differ by two orders of magnitude:

| Half | Tool | Time to rebuild |
| --- | --- | --- |
| the page | Bun | milliseconds |
| the commands crate | Cargo | about a second |
| the shell (window, webview bindings) | Cargo | a minute or more |

The frontend already reloads cheaply, because Bun's dev server re-serves it. If Rust
code also needed a full shell rebuild, then every change to a function that returns
a greeting would cost a minute and, more painfully, **close your windows and lose
your app's state**. You would stop testing small changes.

Hot reload exploits the gap: `voussoir dev` builds the commands crate as a **shared
library** (`cdylib`, so `.so` on Linux, `.dylib` on macOS, `.dll` on Windows) and
the running shell loads the new one when it appears.

**The obvious worry** is that this puts dynamic loading into what is supposed to be
a single static binary. So the mechanism is fenced on three sides:

| Configuration | The commands crate is built as | Who loads it | When |
| --- | --- | --- | --- |
| `voussoir dev` | `cdylib` | the shell, at runtime | while you work |
| `voussoir build` | static library (`rlib`) | the linker, at build time | never |
| release binary | static library | the linker | never |

The cost of the fence is honest: **development and release link your commands
differently**, so a bug in the boundary (a value that cannot cross a `cdylib` edge)
can appear only in dev. That is why the command table is plain JSON-to-JSON data
rather than closures; see [How a project is laid out](layout.md).

## The two loops

`voussoir dev` runs two independent watchers, and knowing which one you are in
explains most "why did nothing happen":

| Loop | Watches | Effect | Cost |
| --- | --- | --- | --- |
| frontend | everything the entry page imports | the page re-renders, in place | milliseconds |
| commands | `native/commands/**/*.rs` | the library is rebuilt and swapped | about a second |

A change to a `.ts`, `.jsx` or `.css` file never touches Rust. A change under
`native/commands` rebuilds only that crate and swaps the resulting library into the
running process: no restart, no lost window state, no lost page state.

**A change to `native/src/main.rs` does neither.** The shell *is* the process; a
change there needs a restart. This is the reason the scaffold puts everything you
iterate on in the commands crate and everything structural in the shell.

## The swap protocol, step by step

This is the part worth understanding, because every failure mode follows from it.

```
1. voussoir dev            cargo build -p commands   (cdylib)
2. dev                     copies the artifact to
                           target/voussoir-hot/commands-<gen>.<ext>
3. dev                     writes latest.json atomically:
                           { "path": "...", "gen": 3, "builtAt": 1699999... }
4. shell (poll, 250 ms)    reads latest.json
5. shell                   if gen > live gen: dlopen the new file
6. shell                   resolves voussoir_dispatch and voussoir_free
7. shell                   installs it, bumps state::reload_generation()
8. shell                   records a [hot] line with rebuild_to_reload_ms
```

Five details, each of which answers a question you would otherwise ask:

**Why a pointer file instead of overwriting the library.** The running process has
the old library mapped. Overwriting a mapped file is undefined behaviour on some
platforms, and on Windows it simply fails because the file is locked. Writing a new
file and moving a pointer is atomic everywhere.

**Why a generation number.** The shell applies a swap only when the announced
generation is newer than the live one, so two builds finishing out of order cannot
regress the shell to older code.

**Why polling instead of a file watcher.** A watcher costs a dependency, a set of
platform quirks, and a class of bugs where an editor's write-and-rename pattern
produces two events or none. A 250 ms poll cannot miss a rename and is cheap for a
directory with two files in it. The cost is up to a quarter second of extra
latency on top of the build.

**Why every loaded library is kept forever.** `HotLib` holds `keep: Vec<Library>`
and never drains it. A command that is still running, or a function pointer a
worker thread already snapshotted, must not have its code unmapped underneath it.
The leak is bounded by how many times you save during one session.

**Why a bad build changes nothing.** A failed compile publishes no pointer, so the
shell keeps the last good library and the compiler error stays on your terminal.
That is the intended workflow: fix the error, save, and the next successful build
swaps in.

## What a swap replaces, and what it cannot

A reload replaces the **code**. It does not replace the process, the windows, the
page, or anything the shell holds.

| Thing | Survives a reload? | Why |
| --- | --- | --- |
| window position and size | yes | the shell owns them |
| the page's DOM and JS state | yes | the webview is untouched |
| data in `voussoir::state` | yes | the map lives in the shell |
| a `static` in the commands crate | **no** | the library that owned it is replaced |
| a captured closure or thread | **no** | belongs to the old library |
| `voussoir.json` changes | no | embedded at compile time |

### Theory: the static that resets

This is the single most surprising consequence of the design, so it is worth stating
plainly:

```rust
// Compiles, "works", and is wrong: this resets every time you save.
static CALLS: AtomicU64 = AtomicU64::new(0);
```

The library is loaded anew, so every `static` in it starts at its initial value.
During development that looks like a bug in your counting logic; in release the same
code is correct, because the crate is linked once. A difference between dev and
release is the worst kind, so the framework offers the shell-owned alternative:

```rust
fn bump() -> u64 {
    let n = voussoir::state::get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        + 1;
    voussoir::state::set("count", voussoir::serde_json::json!(n));
    n
}
```

`voussoir::state` is a process-wide JSON map owned by the shell, so a reload never
touches it, and it works in release builds too: code written against it does not
need two versions. See [In-process state](state.md).

**When the static is fine:** pure constants, lookup tables built from `include_str!`,
and anything derived only from its inputs. The rule is not "never use a static", it
is "a static that accumulates across calls is a dev/release difference".

### `reload_generation()`

`voussoir::state::reload_generation()` returns which library generation is live: 0
for a release build and for the first dev load, then incremented by each successful
swap. Use it when something must be rebuilt once per generation rather than on
every call:

```rust
static POOL_GEN: AtomicU64 = AtomicU64::new(0);

fn pool() -> &'static Pool {
    let gen = voussoir::state::reload_generation();
    // Rebuild when the generation changed, not on every call.
    // ...
}
```

A connection pool is the worked example: it should not be rebuilt per call, and it
must be rebuilt when the code that built it changed.

## Reading the reload log

Every swap prints one line, and `--verbose` also records it as a `[hot]` diagnostic
entry with the same fields:

```
voussoir: [hot] t+8124ms hot commands gen 3 active gen=3 path=/…/commands-3.so rebuild_to_reload_ms=1533
```

| Field | Means |
| --- | --- |
| `gen` | the generation that is now live |
| `rebuild_to_reload_ms` | from the build starting to the swap finishing, so it includes the poll |
| `path` | which artifact was loaded |

A failure prints instead of a success line and keeps the previous generation:

```
voussoir: hot reload FAILED (gen 4): <reason> — keeping gen 3
```

**The reason a swap can fail without a compile error** is worth knowing, because
the message names it: the artifact exists but cannot be loaded. Usually that is a
missing symbol (the crate stopped exporting `voussoir_dispatch`) or a dynamic
library the new code needs that the process has not got. The shell stays on the old
generation, so your app keeps running and you keep the error.

## What does not reload

- **The shell crate.** `native/src/main.rs` needs a restart.
- **`voussoir.json`.** It is compiled in, so a change needs a shell rebuild.
- **Mobile.** Android and iOS restrict loading dynamic code; `voussoir dev` is a
  desktop command and the mobile scaffold never enables the feature.
- **Release builds.** The feature is off, so the code that could load a library is
  not in the binary at all.
- **A new Cargo dependency.** Adding one changes the crate's metadata and needs the
  watcher to rebuild, which it does, but a dependency with a native build step can
  take long enough that you should expect a pause.
- **A new command that the allowlist excludes.** The code reloads, the command
  exists, and the page still gets `unknown`. The config did not change.

## When a swap does not seem to happen

| Symptom | Cause | Fix |
| --- | --- | --- |
| edits never appear | the build is failing | read the cargo error on your terminal |
| a command still answers with old behaviour | it is registered in `main.rs`, not the table | restart `dev` |
| counters reset on every save | a `static` in the commands crate | move it to `voussoir::state` |
| the terminal says the feature is missing | an old scaffold without `hot-reload` | add it to `native/Cargo.toml` |
| the first save after start is slow | the first `cdylib` build has no cache | expected |
| a reload is announced but nothing changes | you edited a file the crate does not include | check the path; the watcher follows the crate directory |

## What it does not do

- **No state migration.** A reload cannot run a function against the old values; if
  a change alters a shape in `voussoir::state`, delete the key or version it.
- **No partial reload.** The whole library is replaced, so all commands change
  generation together.
- **No rollback command.** The previous library is still mapped but not selected;
  reverting means fixing the code and saving.
- **No reload of the page's own JS module graph beyond what Bun's HMR handles.** A
  module that does not accept HMR reloads the page, which loses page state.

## Next

- [How a project is laid out](layout.md) — why the commands crate is separate.
- [In-process state](state.md) — what survives a swap.
- [Testing](testing.md) — `scripts/verify-hot.ts` in CI.
