# Command-line reference

Every command and flag `voussoir` accepts, what each one does, and what it
deliberately does not do. `voussoir help` prints the same list from your installed
binary, and that copy is the one that matches your version.

## Theory: why the CLI exists at all

A Voussoir app is two build systems that have to agree. **Bun** bundles the
frontend. **Cargo** compiles the Rust shell. Neither knows the other exists.

The place they meet is small and unforgiving: an asset table generated from the
bundled files, a dev URL the shell loads instead of that table, and a shared library
swapped into a running process. A hand-rolled setup gets these three right the first
time and then drifts, and the drift shows up as a blank window, a stale page, or a
command that "does not exist" until you restart.

The CLI is that meeting point, and nothing more. It is deliberately thin: eight
commands and five subcommands, each a wrapper over a tool you could invoke yourself.
That is a design constraint worth knowing, because it tells you what to do when a
flag you want is missing: call the underlying tool directly, and tell the CLI about
the result through `voussoir.json` or an environment variable.

**Bun is development-time only.** The CLI runs on it, your frontend is bundled by
it, and nothing it produces contains a JavaScript runtime.

## Argument syntax

The parser is deliberately minimal, and knowing its rules avoids a class of
confusion:

```bash
voussoir dev --port 8080        # a flag followed by a value
voussoir dev --port=8080        # the same, inline
voussoir build --size           # a boolean flag: no value
voussoir init notes             # a positional argument
voussoir init --name "My App" notes
```

Three rules:

- **A flag takes the next token as its value unless that token starts with `-`.**
  So `--size --verbose` sets `size` to `true`, while `--name --force` would set
  `name` to `true` and silently not enable `--force`. Quote values that begin with a
  dash.
- **Unknown flags are not an error.** They are parsed and ignored, so a typo like
  `--verbsoe` does nothing and says nothing. If a flag seems to have no effect, check
  its spelling first.
- **`-h` or `--help` anywhere prints the usage text**, and running with no command
  does the same.

## The commands at a glance

| Command | What it does | Needs |
| --- | --- | --- |
| `init [dir]` | scaffold a new app | Rust, Bun, webview |
| `dev` | dev server plus shell against it | the above |
| `build` | bundle the frontend, compile the binary | the above |
| `package` | build, then wrap it in a platform artifact | the above, plus platform tools |
| `android build` | cross-compile and package an APK or AAB | NDK, SDK, JDK |
| `android run` | install and launch on a device; `--wait` keeps its logs attached | `adb`, a device |
| `ios build` | cross-compile and build the simulator app | Xcode |
| `ios run` | install and launch on a booted simulator | Xcode, a simulator |
| `ios verify` | build, install, launch, assert the Rust entry ran | Xcode |
| `docs` | validate `docs/src`, then build the website | Bun |
| `doctor` | check the toolchain and report what is missing | nothing |
| `keygen` | create the Ed25519 key that signs releases | nothing |
| `sign` | print the `sha256` and `signature` for an artifact | the signing key |
| `patch` | build the delta between two artifacts | the signing key, optionally |
| `release` | write `latest.json` with a signed delta | the signing key |
| `update` | rebuild and replace the CLI from its `install.sh` checkout | `--feed` (or `VOUSSOIR_CLI_FEED`) for a prebuilt-binary feed; `--out` to write the binary elsewhere |

## `init`

```bash
voussoir init              # scaffold into the current directory
voussoir init notes        # scaffold into ./notes
```

| Flag | Effect |
| --- | --- |
| `--name <n>` | app name; default comes from the directory name |
| `--force` | write into a directory that already has files |
| `--package <id>` | Android package name; default `com.voussoir.<app>` |
| `--mobile` | Android shell only, no desktop crate |

What it writes is documented file by file in
[How a project is laid out](layout.md). The short version: `voussoir.json`, an entry
HTML page, `src/` for the frontend, and `native/` with the shell crate and the
commands crate.

**Why `--force` is not the default.** Scaffolding overwrites `voussoir.json`,
`main.rs` and the command table. In a directory with an existing app, that is the
silent loss of exactly the files you have edited.

**A caution that has bitten people:** an empty directory is still a directory
containing no files, but `init` in a directory with *anything* in it (a stray
`.git`, an editor folder) asks for `--force`. That is what stops `voussoir init` in
your home directory from scattering a project over it.

## `dev`

```bash
voussoir dev
voussoir dev --port 8080 --verbose
```

| Flag | Effect |
| --- | --- |
| `--port <n>` | dev server port; default `5177` |
| `--verbose` | record reloads, slow commands and refused navigations |

Starts the Bun dev server, launches the shell pointed at it, then watches both:
frontend edits are re-bundled and pushed over hot module replacement, and
`native/commands` edits are compiled to a shared library and swapped into the
running process. Ctrl-C stops both, and so does closing the window.

**What does not reload:** `native/src/main.rs`, `voussoir.json`, and `Cargo.toml`.
Those need a restart, because they are either the process itself or its
configuration. [Hot reload in development](hot-reload.md) explains the swap.

**Why 5177.** It avoids the ports other tools default to (3000, 5173, 8000), so a
running dev server is rarely a surprise collision. If it is taken, use `--port`.

**`--verbose` is the diagnostic mode**, not a verbosity knob in the usual sense: it
turns on recording of command latency, refused navigations and reloads, which is
what `[security]` and `[ipc]` lines in the terminal come from. See
[Diagnostics](diagnostics.md).

## `build`

```bash
voussoir build
voussoir build --debug --size
voussoir build --out ./dist/myapp
```

| Flag | Effect |
| --- | --- |
| `--debug` | cargo debug profile instead of release |
| `--size` | print the resulting binary size |
| `--out <path>` | copy the binary somewhere |

Bundles the frontend, regenerates `native/src/assets.rs`, and compiles the binary.
The output lands in `target/release/<name>`.

**`build` checks disk space first**, because running out mid-link produces a
compiler error naming whichever crate happened to be compiling, which reads like a
code problem. Under 1500 MB free it refuses to start and says the number.

**`--debug` builds a different thing on purpose:** symbols kept, no LTO, no
stripping. Use it for a backtrace from a crash that only happens in release; do not
ship it, and do not compare its size to a release build.

**`--out` exists for CI**, so a workflow that uploads an artifact does not need to
know the target triple's path layout.

## `package`

```bash
voussoir package
voussoir package --icon icon.png --identifier com.example.notes
```

| Flag | Effect |
| --- | --- |
| `--icon <path>` | 256×256 PNG, or `.icns` on macOS |
| `--identifier <id>` | reverse-DNS id; default derived from the app name |
| `--debug` | package the debug build, for testing the layout |

| Platform | Artifact |
| --- | --- |
| Linux | `.AppImage`, or a runnable `AppDir` if `appimagetool` is missing |
| macOS | a `.app` bundle |
| Windows | a `.zip` containing the `.exe` and a note about WebView2 |

The Linux fallback is deliberate rather than a failure: without `appimagetool` the
AppDir still runs, so the command says what it left and where to get the tool,
instead of producing nothing. See [Packaging](packaging.md) for what each artifact
contains and why signing is left to you.

## `android`

```bash
voussoir android build
voussoir android build --abi x86_64 --api 30
voussoir android build --release
voussoir android build --aab
voussoir android run
```

| Flag | Effect |
| --- | --- |
| `--abi <abi>` | `arm64-v8a` (default), `x86_64`, `armeabi-v7a` |
| `--api <n>` | target API level |
| `--release` | signed release APK instead of a debug one |
| `--aab` | signed Android App Bundle, for Play upload |
| `--wait` | (run) stay attached instead of returning |

`--release` and `--aab` need a signing key. Without one the command explains what is
missing instead of producing an unsigned artifact that cannot be installed, which is
a failure you want at build time rather than after an upload. `--aab` implies
release signing.

`android run` installs and launches on the attached device or the running emulator.

Without `--wait` it returns once the app is up, which is what a script wants.
`--wait` keeps the device log stream attached until Ctrl-C.
See [Android and iOS](mobile.md).

## `ios`

```bash
voussoir ios build
voussoir ios build --target device
voussoir ios run
voussoir ios verify
```

| Flag | Effect |
| --- | --- |
| `--target sim` | simulator build (default) |
| `--target device` | device build, unsigned |
| `--no-build` | (run) install what is already built |

`ios verify` exists because a blank iOS window is almost always a wiring problem
rather than a rendering problem. It builds, installs, launches, and asserts the Rust
entry was actually reached, so one command separates the two cases.

`--target device` produces an unsigned build: signing an iOS app needs your
certificate and a provisioning profile, which the CLI will not hold.

## `docs`

```bash
voussoir docs
voussoir docs --check
```

| Flag | Effect |
| --- | --- |
| `--check` | validate and stop; build nothing |

Without `--check` it validates, then renders the website into `website/dist`.

**What `--check` enforces**, and why each rule exists:

| Rule | The rot it prevents |
| --- | --- |
| every entry in `SUMMARY.md` exists | a renamed file leaving a sidebar link to a 404 |
| every `.md` file appears in `SUMMARY.md` | a new chapter that is unreachable from the sidebar |
| no relative link points at an unlisted chapter | a link to a page the build never generated |

The third rule matters because the renderer rewrites `x.md` to `x.html`. A link to a
file outside the summary becomes a link to a page that was never produced, which is
exactly the failure mode a static site generator reports as a missing page three
minutes later. See [Writing docs](writing-docs.md).

## `doctor`

```bash
voussoir doctor
voussoir doctor --json
```

| Flag | Effect |
| --- | --- |
| `--json` | machine-readable report on one line of JSON |

Checks Bun, Cargo, `pkg-config` and the platform webview, plus, inside a project,
the entry page and the native manifest. Every check reads something real: a binary on
`PATH`, a file, a manifest. It never starts a build, so it is safe to run first.

**Run it first when anything is odd.** It is faster than reading a compiler error
that is two layers downstream of a tool that was never installed. The full list of
checks is in [Installing Voussoir](install.md).

## Release signing

The shell's updater requires an Ed25519 signature, and these commands produce it.
The private key stays on the machine that cuts the release.

```bash
voussoir keygen --out release.pem          # prints the base64 key for voussoir.json
voussoir sign dist/app --key release.pem   # sha256 + signature, paste into the feed
voussoir patch old-app new-app --out app.patch --from 2.0.0 --key release.pem
```

`keygen` writes the PKCS#8 private key with mode `0600` and prints
`update.public_key` for `voussoir.json`. `sign` prints JSON with a `sha256` and a
`signature` over the artifact's exact bytes. `patch` additionally writes the delta
and prints both field sets: the patch's own hash/signature and the rebuilt
artifact's, because the updater checks the restored binary against the artifact's
signature. The shell's side is in [Updating the app](updater.md).

`release` is those two in one step, producing the manifest the updater reads:

```sh
voussoir release --version 2.1.0 --artifact target/release/myapp \
  --base-url https://downloads.example.com/myapp \
  --previous myapp-2.0.0 --from 2.0.0 \
  --key release.pem --out latest.json
```

It writes `latest.json` with the artifact's `url`, `sha256` and `signature`, and
with `--previous` a `patches` entry whose delta is written beside the manifest
and advertised under the URL it will be served from. Nothing is uploaded: the
manifest names where you say it will live, and serving the files is yours.

`--previous` and `--from` repeat, one pair per previous version, and every hop
rebuilds *this* release from its own artifact. That is what lets someone three
releases behind still download a delta instead of the whole binary:

```sh
voussoir release --version 2.1.0 --artifact dist/app \
  --base-url https://downloads.example.com/app \
  --previous dist/app-1.9.0 --from 1.9.0 \
  --previous dist/app-2.0.0 --from 2.0.0 \
  --key release.pem --out latest.json
```

`--merge` writes this platform's entries into an existing manifest instead of
replacing it, so a CI matrix can build one `latest.json` from one job per
platform. Merging a different `--version` is refused: two versions in one file
would offer a user whichever artifact the last job wrote.

The delta is the reason to use it, and the saving depends on how much changed.
Measured on a 962 KB release build of `examples/hello`: changing one constant
(21 bytes of the binary) produced a **62-byte** patch, and a build carrying real
code changes produced a patch at about **45%** of the artifact, so even the bad
case halves the download. The updater verifies the reconstructed binary against
the artifact's own hash and signature, so a patch cannot introduce anything the
full download would not have. A patch that reconstructs the wrong bytes, carries
a bad signature, or is malformed replaces nothing. Generation takes well under a
second and is bounded in memory, because the index is a fixed 32 MB table rather
than one entry per 32-byte window.

Signing and parsing are now checked against each other: `scripts/make-updater-fixture.ts`
writes a manifest and patches with the CLI's own code, and
`voussoir/tests/updater_contract.rs` parses them and applies every hop, so a
format change on one side fails there instead of in a user's update.

## Theory: what the CLI will never do

These are decisions, not gaps waiting to be filled:

- **No `install` command.** An artifact is run in place: an AppImage is a file, a
  `.app` is dragged to Applications, a `.exe` is unzipped. There is no system-wide
  install step to own.
- **No `publish` command.** Uploading to a store or a registry needs credentials,
  and a build tool should not hold yours.
- **No implicit self-update.** `voussoir update` exists, but it only runs when you
  ask; there is no background check. The default is to fast-forward the checkout
  `install.sh` built the CLI from and rebuild it, because this project ships from
  source rather than from published releases. It rebuilds only when `cli/` changed
  since that commit, so committing a `.gitignore` does not recompile the CLI.
  `--feed` is still there for anyone
  serving prebuilt binaries of their own, and that path verifies the download's
  `sha256` before renaming it into place. `--check` reports without changing
  anything on either path.
- **No global state.** Every command reads and writes the project directory and the
  standard toolchain locations, nothing else. Two projects never interfere, and a
  CI cache is the only thing that survives a clean checkout. The one exception is
  `update`, which reads `source` and `built` from the installer's share directory
  (`~/.local/share/voussoir` on Unix, `%LOCALAPPDATA%\voussoir` on Windows, and
  `$XDG_DATA_HOME/voussoir` when that is set). The first names the checkout it was
  installed from; the second the commit it was built from, which is what lets a
  docs-only commit skip the rebuild.

## What it does not do

- **No `--version` subcommand.** It is a flag-shaped omission, not an oversight: the
  CLI is built from a branch, so a version string would mislead more than inform.
- **No watch mode for anything but `dev`.** `docs` and `build` run once.
- **No monorepo support.** One `voussoir.json` per project, found by walking up from
  the current directory.
- **No interactive prompts.** Every input is a flag or a positional, which is what
  makes the commands usable from a script.

## Next

- [Installing Voussoir](install.md) — how the CLI gets on your machine.
- [Troubleshooting](troubleshooting.md) — when a command fails.
- [Operations](operations.md) — running these in CI.
