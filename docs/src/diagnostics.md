# Diagnostics

`voussoir dev` prints the dev server URL, each successful rebuild, and each failed
compile. `--verbose` adds four kinds of event that are otherwise invisible, and
records them so a snapshot survives the terminal scrolling.

```bash
voussoir dev --verbose
```

## Theory: why an app needs a recorder, not just log lines

A `println!` is a statement about the present. It is fine while you are watching,
and useless the moment you look away, which is most of the time a bug is
interesting.

Two situations make that concrete:

- **A bug on a phone.** There is no terminal, no scrollback, and no way to attach
  a debugger to a released build. The only channel out of an app is the app.
- **A bug that happened four reloads ago.** By the time you notice the wrong
  behaviour, the cause scrolled past. You need the history, not the present.

So diagnostics here is a **ring buffer**: a fixed-size list of recent events, kept
in memory, readable from the page on any platform. `--verbose` only decides
whether it is filled; the machinery is always present and costs one cached atomic
load when off.

## The five kinds

| Kind | Recorded when | The question it answers |
| --- | --- | --- |
| `hot` | a command library generation goes live | "did my edit actually reach the app?" |
| `dispatch` | a command takes 50 ms or more | "which command makes the window hitch?" |
| `security` | navigation is refused by the policy | "why did that link do nothing?" |
| `bridge` | a bridge message is dropped as malformed | "why does my `postMessage` do nothing?" |
| `app` | a handler panicked | "what crashed, and did the window survive?" |

Each entry carries:

- a **monotonic sequence number**, so a reader can tell entries apart even when
  two share a timestamp,
- **`at_ms`**, milliseconds since process start (not wall-clock: a clock that
  jumps backwards during an NTP sync would make a log lie about ordering),
- the **message**, and a JSON object of **fields**.

One line per entry, so a log can be grepped:

```
voussoir: [hot] t+8124ms hot commands gen 3 active gen=3 path=/…/commands-3.so rebuild_to_reload_ms=1533
voussoir: [dispatch] t+9010ms slow command method=read_file ms=412
voussoir: [security] t+10233ms navigation refused dev_origin=null url=https://example.com/
```

Read the `dispatch` line as: at 9.0 seconds into the run, `read_file` took 412 ms,
which is four frames of a frozen window. The threshold for recording is 50 ms,
three frames at 60 Hz: below that a command is invisible to a person.

## Reading it from the page

This is the part that works where there is no terminal — Android, iOS, or a user's
machine:

```ts
const { verbose, entries } = await window.voussoir.invoke("__voussoir_diagnostics", { tail: 20 });
console.table(entries);
```

- `tail` is optional; omit it for the whole buffer.
- The buffer holds the **last 256 entries** and drops the oldest, so a session
  that runs for hours does not grow the process.
- `__voussoir_diagnostics_enabled` answers only the flag, for a diagnostics screen
  that shows state without fetching the entries.

Why a built-in rather than a JS array in the page? Because the page is the thing
that crashes. An error that kills the webview cannot record its own last words; the
shell can.

## Theory: measuring latency without slowing the app

A naive timer around every call costs a clock read twice, which is on the order of
tens of nanoseconds — irrelevant — but the *recording* is not: appending to a
shared structure from many worker threads means a lock on the hot path.

Three decisions keep that honest:

1. **Nothing is recorded when verbose is off.** The check is one cached atomic
   load, so a release build pays essentially nothing.
2. **Samples are capped at the last 1000 calls per command.** A long session keeps
   a bounded memory footprint, and the most recent behaviour is what you are
   debugging.
3. **Measurement is around the handler, not the transport.** The number answers
   "is my command slow", not "is the webview slow". Mixing them produces a table
   that blames the wrong half.

## Latency on exit

When the app closes, verbose mode prints a per-command table, slowest mean first:

```
voussoir: command latency, slowest first:
  read_file                    p50 410 ms  p95 512 ms  max 610 ms  n 12
  greet                        p50   0 ms  p95   1 ms  max   3 ms  n 48
```

- **p50** is the median: half the calls were faster.
- **p95** is the tail: 95% were faster, so one call in twenty was not. A p95 far
  above p50 means the occasional stall, which users experience as "sometimes it
  hangs".
- **n** is the sample count. Trust a p95 of 3 samples about as much as nought.

## Theory: why the mean is not enough

If nine calls take 1 ms and one takes 1000 ms, the mean is 101 ms and describes no
call that ever happened. Percentiles describe the distribution; the mean hides it.
That is why the table leads with p50 and p95 rather than "average".

## Enabling it without the CLI

`voussoir dev --verbose` sets `VOUSSOIR_VERBOSE=1` for the shell it launches. The
shell honours the variable directly, so `cargo run` in a debug build produces the
same output:

```bash
VOUSSOIR_VERBOSE=1 cargo run --manifest-path native/Cargo.toml
```

`VOUSSOIR_VERBOSE=0` forces it off even when the variable is inherited from a shell
profile, which is the usual reason someone cannot turn it off.

## A worked session

A window freezes for a moment when opening a file. In order:

1. `voussoir dev --verbose`, reproduce, quit.
2. The exit table names `read_file` with p50 410 ms.
3. That tells you the model: a blocking read registered as `command` (sync, UI
   thread) rather than `async_command`. See
   [Commands & payloads](ipc-contract.md).
4. Move it to `async_command`, reproduce again, confirm p50 stays but the window
   no longer freezes.

The point of the chapter is step 3: the number names which half of the app is
wrong, and the chapter on execution models tells you the fix. Guessing produces a
week of "maybe it was the CSS".

## Next

- [Benchmarks](benchmarks.md) — the numbers CI gates on.
- [Troubleshooting](troubleshooting.md) — symptom to cause.
