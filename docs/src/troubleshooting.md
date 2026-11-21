# Troubleshooting

Symptoms, causes and fixes, in the order they usually happen. Start with
`voussoir doctor`: it names a missing tool instead of letting a compile error
surface two minutes later from a file you have never opened.

## Theory: how to read a Voussoir failure

Every failure belongs to one of four layers, and the message tells you which:

| Layer | Typical message | Who to believe |
| --- | --- | --- |
| Toolchain | `cargo is not on PATH` | `voussoir doctor` |
| Build | `error[E0432]: unresolved import` | the compiler |
| Runtime, Rust | `kind: "handler"` / `"panic"` | your command |
| Runtime, page | `kind: "unknown"` / a blank window | the registry or the bundle |

The first question is always: **did the code run at all?** A `kind: "unknown"`
means your Rust never executed. A `kind: "handler"` means it ran and returned
`Err`. Those two diagnoses lead to completely different fixes, and mixing them up
is the most common waste of an afternoon.

## Install and toolchain

| Symptom | Cause | Fix |
| --- | --- | --- |
| `voussoir: command not found` | not installed, or not on `PATH` | re-run the installer; add `$HOME/.local/bin` to `PATH` |
| `doctor` says `cargo ... not found` | Rust missing | install Rust, or re-run `install.sh` |
| `doctor` says `webkit2gtk ... missing` | no system webview (Linux/BSD) | install `webkit2gtk-4.1` or re-run the installer |
| `pkg-config` missing | needed to link GTK and WebKit | install your distro's `pkg-config` |
| `doctor` says `no voussoir.json` | not in a project directory | `cd` into the project, or run `voussoir init` |
| install script warned about packages | package manager failure | not fatal; `doctor` names what is still missing |

`voussoir doctor --json` prints the same report for a script or a CI log.

## Dev server

| Symptom | Cause | Fix |
| --- | --- | --- |
| `port 5177 already in use` | another `dev` is running | `voussoir dev --port 8080` |
| window opens, then goes blank | the page threw during load | open devtools (`F12`) and read the console |
| page never updates on save | the file is not imported by the entry | import it from `index.html` or `main.ts` |
| Rust edits do nothing | you edited `native/src/main.rs` | restart `dev`; only the commands crate hot-reloads |
| hot reload never lands | build failing, so nothing was published | read the cargo error; the old library stays live |
| HMR websocket disconnects | a proxy or VPN in the middle | use the loopback URL directly |

The blank-window case is worth spelling out: the shell has already started, so
the terminal looks healthy. Devtools is the only place the page's error appears.

## Commands

| Symptom | Cause | Fix |
| --- | --- | --- |
| `kind: "unknown"` | not registered, or withheld by an allowlist | add it to `COMMANDS` / `.command(...)`, or to `commands` in `voussoir.json` |
| `kind: "unknown"` only in one window | that window has a narrower allowlist | see [Per-window allowlists](allowlists.md) |
| `kind: "payload"` | the request did not match a typed command | check the field names and types |
| `kind: "handler"` | your handler returned `Err` | read `message`; it is meant for a human |
| `kind: "panic"` | your handler panicked | the window survived; fix the panic, check the log |
| `kind: "busy"` | every worker-thread slot was taken | retry later, or make fewer async commands |
| `kind: "timeout"` | the shim's deadline expired | raise `command_timeout_ms`, or the command is genuinely stuck |
| `kind: "runtime"` | the webview or event loop failed | fatal for that window; report it |

`kind: "unknown"` covering a denial is deliberate. See
[Errors the page can branch on](errors.md) for why the two are indistinguishable.

## Build and package

| Symptom | Cause | Fix |
| --- | --- | --- |
| `could not compile <random crate>` | disk space ran out mid-build | free space; `build` pre-checks 1.5 GB, `dev` does not |
| `error: linker not found` | no C toolchain | install `build-essential` / Xcode CLT / MSVC build tools |
| binary is huge | a debug build | `voussoir build` (release) rather than `--debug` |
| `package` fails on the icon | wrong format or size | 256×256 PNG, or `.icns` on macOS |
| Windows build needs WebView2 | not preinstalled on old Windows | ship the Evergreen bootstrapper |
| `assets.rs` looks wrong | edited by hand | regenerate: `voussoir build` overwrites it |

The disk-space entry is real and confusing: running out mid-link produces an
error naming whichever crate happened to be compiling, which looks like a code
problem. Delete `target/` first if the errors make no sense.

**Where the check does and does not run.** `voussoir build` refuses to start with
under 1500 MB free and says so in a sentence that names the number, because running
out mid-link produces an error naming whichever crate happened to be compiling.
`voussoir dev` does not run that check, and neither does `doctor`, so a `dev` session
on a nearly full disk still reaches you as a nonsense compiler error. `du -sh target/`
is the quickest confirmation; this stack's `target/` reaches a few gigabytes.

## Page and security

| Symptom | Cause | Fix |
| --- | --- | --- |
| a link does nothing | navigation was refused by policy | `target="_blank"` for external, or route internally |
| `Refused to load the script` | CSP blocks remote scripts | bundle the dependency |
| `eval is not allowed` | CSP has no `unsafe-eval` | ship the code instead of evaluating it |
| `fetch` to a remote host fails | `connect-src 'self'` | do the request in Rust |
| images from a remote host missing | `img-src` allows only `self`, `data:`, `blob:` | bundle, or proxy through a command |
| absolute URL 404s only on Windows | the origin is rewritten there | use a relative URL or `pageOrigin` |

If `--verbose` is on, refusals are recorded as `[security]` entries with the URL,
so "why did that link do nothing" has an answer:
[Diagnostics](diagnostics.md).

## Mobile

| Symptom | Cause | Fix |
| --- | --- | --- |
| `ANDROID_NDK_HOME not set` | no NDK | install the NDK, set the variable |
| `adb: no devices` | emulator not running | start one, or `voussoir android run` |
| iOS simulator build fails | no Xcode CLT | `xcode-select --install` |
| menu/tray calls error on iOS | not supported there | branch on `voussoir.platform` |

## When nothing above fits

1. `voussoir doctor` — rules out the toolchain.
2. `voussoir dev --verbose` — rules out policy and slow commands.
3. `cargo test` and `bun test` — rules out logic you already covered.
4. `voussoir build --debug` — a usable backtrace for a release-only crash.
5. Reduce to the smallest app that still fails, then diff against the scaffold
   from `voussoir init` in a temp directory.

Step 5 catches more real bugs than any other, because it separates "my app" from
"the framework" with evidence instead of a guess.

## Next

- [Diagnostics](diagnostics.md) — what `--verbose` records and how to read it.
- [Testing](testing.md) — turning a bug you just fixed into a test.
