# Installing Voussoir

Voussoir is two things: a **CLI** you run (`voussoir init`, `voussoir dev`,
`voussoir build`) and a **Rust crate** your app links against. Installing means
putting the CLI on your `PATH`.

There is no published package yet. `cargo install voussoir` and `npm i -g voussoir`
do not work, and any guide that suggests them is describing a release that does not
exist. There are two ways in: the install script, or a checkout.

## Theory: why a script instead of a package

A package manager solves three problems at once. It knows the dependency graph, it
verifies what it installs, and it can remove what it added. Until there are
packages, a shell script has to stand in for all three, and being honest about
which part it fakes is the difference between a useful installer and a mess.

| Promise | What this script does | What a package manager would do |
| --- | --- | --- |
| Find dependencies | asks your OS package manager, skips what is present | resolves the graph |
| Verify what it installed | clones the public repo over TLS and builds from source | checksums and signatures |
| Remove it | you delete a file and a `PATH` line | `remove` |

The middle row is the trade worth understanding. **Building from source** means the
binary you run was produced from code you can read, on your machine, by your
compiler. It also means **nothing is signed and nothing is checked**: there is no
checksum to verify and no maintainer key to trust. That is the current state of this
project, stated rather than implied, and it is why uninstalling is three commands
rather than one.

**Why it installs the system webview for you.** The first failure a new user hits is
a missing WebKitGTK, and it surfaces two minutes into a Rust build as a linker error
from a file they have never opened. Installing it up front, and *warning* rather
than aborting when the package manager refuses, turns that into one line in
`voussoir doctor`'s report.

## What you need before starting

| Tool | Why | Already there on most systems? |
| --- | --- | --- |
| `git` | to clone the source | usually |
| `curl` | the script itself, and package installs | usually |
| `cargo` (Rust) | to build apps, and the CLI's Rust dependencies | no |
| `bun` | to build the CLI and your frontend | no |
| system webview + `pkg-config` | to link the window (Linux/BSD only) | sometimes |

The script installs the last four for you where it can. On Linux and the BSDs it
asks your package manager for the webview development headers, which is the piece
people most often miss:

| Distribution | Package the script asks for |
| --- | --- |
| Debian, Ubuntu | `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libsoup-3.0-dev`, `build-essential`, `pkg-config` |
| Fedora, RHEL | `webkit2gtk4.1-devel`, `gtk3-devel`, `librsvg2-devel`, `gcc`, `make`, `pkgconf-pkg-config` |
| Arch | `webkit2gtk`, `gtk3`, `base-devel` |
| openSUSE | `libwebkit2gtk-4_1-devel`, `gtk3-devel` |
| Alpine | `webkit2gtk-dev`, `gtk3-dev`, `build-base` |
| FreeBSD, DragonFly | `webkit2gtk-4.1`, `gtk3` (via `pkg`) |
| NetBSD, OpenBSD | whatever `pkgin`/`pkg_add` call the webkit package |

On macOS the only system prerequisite is the **Xcode Command Line Tools**, which
the script triggers with `xcode-select --install` and then stops, because that
installer is interactive and cannot be scripted.

On Windows, WebView2 ships with Windows 10 and 11 and is installed by the script if
absent.

## The install script

Linux, macOS and the BSDs:

```bash
curl -fsSL https://raw.githubusercontent.com/smegasberla/voussoir/master/install.sh | sh
```

Windows, in PowerShell:

```powershell
irm https://raw.githubusercontent.com/smegasberla/voussoir/master/install.ps1 | iex
```

Re-running the same command is how you upgrade. Every dependency already present is
skipped, so a second run touches nothing but the build.

### What it does, in order

1. **Picks up tool installers' directories** (`~/.cargo/bin`, `~/.bun/bin`) for the
   rest of the run, so a Rust installed five minutes ago is found.
2. **Detects your OS and package manager** among `apt-get`, `dnf`, `pacman`,
   `zypper`, `apk`, `pkg`, `pkgin` and `pkg_add`, and whether it can escalate: it
   uses `sudo` only when it is not already root and `sudo` exists.
3. **Installs what is missing**: the webview headers, `git`, `curl`, `pkg-config`,
   Rust (via rustup) and Bun.
4. **Finds source.** If the script is running inside a checkout, it builds that. If
   it was piped from `curl`, it clones into
   `${XDG_DATA_HOME:-~/.local/share}/voussoir/src` and reuses the clone next time.
5. **Builds the CLI.** `bun install` then `bun run build` inside `cli/`.
6. **Copies the result** to `$PREFIX/bin/voussoir` (default `$HOME/.local`), mode
   `755`.
7. **Prints the `PATH` line if needed**, and does not edit your shell profile.
8. **Verifies** the installed binary by running `voussoir help`.

### Flags

| Unix | Windows | Effect |
| --- | --- | --- |
| `--prefix DIR` | `-Prefix DIR` | install location, default `$HOME/.local` (`%LOCALAPPDATA%\voussoir`) |
| `--no-deps` | `-NoDeps` | skip system package installation entirely |
| `--repo URL` | `-Repo URL` | git URL to clone when not run from a checkout |
| `--help` | | the usage text |

`VOUSSOIR_PREFIX` and `VOUSSOIR_REPO` do the same as the Unix flags, which is handy
in a Dockerfile where adding arguments to a piped script is awkward.

**When to use `--no-deps`:** on a machine you provision yourself, in CI, or where you
have installed the webview through a channel the script cannot detect (a Nix
profile, a container with the libraries pre-baked). The script warns and continues
when a package install fails, so `--no-deps` mainly saves time and avoids an
unwanted `apt-get update`.

## From a checkout

If you have the repository, the script notices the tree it is in and builds that
instead of cloning:

```bash
git clone https://github.com/smegasberla/voussoir.git
cd voussoir
./install.sh
```

The equivalent without the script, when dependencies are already installed:

```bash
cd cli
bun install
bun run build     # -> cli/dist/voussoir, a compiled binary
```

That binary embeds the Bun runtime, so it needs neither Bun nor Node on `PATH` at
runtime. That is a convenience for your machine: **the apps the CLI builds never
contain a JavaScript runtime.**

## Checking the result

```bash
voussoir help        # the command list and every flag
voussoir doctor      # toolchain, webview and project checks
```

`doctor` is the command to run when something is missing, rather than guessing. It
is deliberately cheap: it reads a binary from `PATH`, a file, or a manifest, and
never starts a build.

### Every check `doctor` performs

| Check | Passes when | What a failure means |
| --- | --- | --- |
| `bun` | found on `PATH` and version ≥ 1 | the CLI itself was built with it, so a stale one is worth knowing about |
| `cargo` | found on `PATH` | Rust is not installed, or not on `PATH` |
| `system webview` | `pkg-config --exists webkit2gtk-4.1` succeeds (Linux/BSD) | the window cannot link; install the package from the table above |
| `pkg-config` | found on `PATH` (Linux/BSD) | the webview check cannot run, and neither can a link |
| `system webview` (macOS/Windows) | always | the engine is part of the OS |
| `android NDK` | `ANDROID_NDK_HOME` or `ANDROID_NDK_ROOT` is set and exists | only checked when the project has an `android/` crate |
| `adb` | found on `PATH` | only needed for `voussoir android run` |
| `project` | a `voussoir.json` exists here or in a parent | you are not in a project directory |
| `entry html` | the file `entry` names exists | the window would open on a 404 |
| `native crate` | `native/Cargo.toml` exists | the project is incomplete; re-run `init` |

`--json` emits the same report as `{"ok": bool, "checks": [...]}` for a script to
read, which is what CI does.

## What gets installed

| Path | What |
| --- | --- |
| `$PREFIX/bin/voussoir` | the CLI binary (default `$HOME/.local/bin/voussoir`) |
| `%LOCALAPPDATA%\voussoir\bin\voussoir.exe` | the same, on Windows |
| `%LOCALAPPDATA%\voussoir\src` | the checkout the Windows installer builds from |

On Unix nothing else is touched: no service, no autostart entry, no shell profile
edit. The installer prints the `PATH` line and leaves the decision to you.

Windows is the exception: there is no equivalent of a piped shell export, so the
installer appends its `bin` directory to your **user** `PATH` directly. New
terminals pick that up; the current one needs the script to have run in it, which
piping to `iex` already does.

## Uninstalling

Remove the binary and the `PATH` line the installer printed:

```bash
rm -f "$HOME/.local/bin/voussoir"
```

On Windows the installer owns a directory, so one removal also takes the source tree
it left behind:

```powershell
Remove-Item "$env:LOCALAPPDATA\voussoir" -Recurse
```

Then drop that `bin` path from your user `PATH`.

**What this does not remove:** the Rust and Bun toolchains, because you may have had
them before or want them after. Projects you scaffolded are ordinary directories;
delete them wherever you made them. Nothing was registered with the OS, so there is
no uninstaller to run.

## Common install problems

| Symptom | Cause | Fix |
| --- | --- | --- |
| `voussoir: command not found` after installing | `$PREFIX/bin` is not on `PATH` | add the line the installer printed, then open a new terminal |
| the script stops after `xcode-select --install` | the CLI tools installer is interactive | finish that installer, then re-run the script |
| `apt install failed; install webkit2gtk-4.1 dev packages manually` | no permission, or a broken package index | run with `sudo`, or install the package yourself and re-run with `--no-deps` |
| `no package manager known for <OS>` | an unusual distribution | install the webview headers yourself, then re-run with `--no-deps` |
| `git is required to clone` | the script was piped and `git` is missing | install `git`, or clone manually and run `./install.sh` from the checkout |
| `the installed voussoir failed to run` | a broken build, usually a missing shared library | run `$PREFIX/bin/voussoir help` to see the loader error |
| `bun: command not found` during the build | Bun was installed after the shell started | open a new terminal |
| `doctor` is fine but a build fails | the webview headers changed version | check the `webkit2gtk-4.1` package name for your distribution |

## What it does not do

- **No package-manager integration.** No `.deb`, no brew formula, no AUR package,
  so no automatic upgrades and no signature check.
- **No version pinning.** The script builds whatever is on the default branch, so two
  people installing a week apart may get different code.
- **No rollback.** The previous binary is overwritten, not kept.
- **No uninstall command.** Removal is the two steps above.
- **No Windows ARM64 handling beyond what Cargo does.** If your target is unusual,
  use a checkout and build the CLI yourself.

## Next

- [Command-line reference](cli.md) — every subcommand and flag.
- [Your first app](first-app.md) — from an empty directory to a window.
- [Platform support](platforms.md) — what each OS actually supports.
