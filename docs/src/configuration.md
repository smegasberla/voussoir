# Configuring an app: `voussoir.json`

One file holds everything the shell needs to know before a window exists: the
name, the first window's size, which commands a page may call, and how long a page
waits. Nine keys in total, and every one is optional.

## Theory: why a file and not code

You could set all of this in `main.rs`, and often you will, because `App` has
methods for the window and the entry page. The file exists for the settings that
are *not* code:

- **The name.** It becomes the window title, the data-directory name, and (on
  Linux) the `.desktop` entry's name. It is data, and it is the one thing you change
  when you fork a project.
- **The allowlist.** It is a policy document, so it reads better as a list than as
  a builder call, and it is the thing a reviewer most needs to see at a glance.
- **The update endpoint.** A URL that changes per release channel.

The dividing line is worth stating: **the file describes the app; code describes
the commands.** A command needs a function, so it cannot live in JSON, and a
policy list does not need one, so it should not live in Rust.

## Theory: why it is embedded, and what that costs

The scaffold writes `include_str!("../voussoir.json")`, so the file's contents are
compiled into the binary. Three consequences, and they are all deliberate:

| Consequence | Why it is the right trade |
| --- | --- |
| Editing the file needs a rebuild | a config that decides permissions should not be editable on disk |
| A shipped app cannot be reconfigured | an attacker who can write the install directory still cannot widen the allowlist |
| Parsing happens at startup, once | invalid JSON is fatal and named, not a silent fallback |

**The cost is real:** a user cannot change the window size by editing a file, and
you cannot ship one binary with two configs. That is the correct cost for a
security document and the wrong one for a user preference, which is why
preferences belong in [storage](storage.md) instead.

## Theory: invalid config is fatal, on purpose

`App::config` parses the string and exits the process on failure:

```rust
match Config::from_json(json) {
    Ok(config) => self.config = config,
    Err(message) => {
        eprintln!("voussoir: {message}");
        std::process::exit(1);
    }
}
```

The alternative, a lenient parse that ignores what it cannot read, means a typo in
`"width": "900"` silently produces a default window and the bug surfaces as "the
size I set does nothing" days later. A build that embeds broken config is a
programming error, so it fails where you can see it.

**Unknown keys are ignored.** That is the opposite rule, and the reason is
forward compatibility: a config written for a newer shell still opens on an older
one. A typo in a *known* key's value type is ignored too, which is the sharp edge:
`"width": "900"` is silently default. If a setting seems not to apply, check its
type before anything else.

## The complete key list

```json
{
  "name": "Notes",
  "title": "Notes — untitled",
  "width": 900,
  "height": 680,
  "entry": "index.html",
  "command_timeout_ms": 30000,
  "commands": ["greet", "read_file"],
  "windows": {
    "preview": { "commands": ["greet"] }
  },
  "update": { "url": "https://example.com/releases.json" }
}
```

### `name`

The app's identity. Defaults to `"Voussoir App"`.

Three things read it:

- **the window title**, unless `title` overrides it,
- **the data directory** — `~/.local/share/Notes/` on Linux,
  `~/Library/Application Support/Notes/` on macOS, `%APPDATA%\Notes\` on Windows,
- **`storage.path`**, which reports where the JSON store lives.

Setting `name` therefore also sets `title`, and that is why renaming the app *after*
shipping moves the storage file: users lose their settings. Rename before you ship.

### `title`

The window title, when it should differ from the name.

```json
{ "name": "notes", "title": "Notes" }
```

Why both: `name` is a filesystem-friendly identifier (lowercase, no spaces is a
good convention), and `title` is what a person reads in the title bar. On Linux a
lowercase `name` also produces a lowercase `.desktop` file, which some launchers
display.

The scaffold's `main.rs` can also set the title per window through
`WindowOptions`, which wins for windows it opens.

### `width` and `height`

The first window's size in logical pixels. Defaults are set by `Config::default`.

They are `f64`, so **write a number, not a string**: `900`, never `"900"`. A
string is ignored, and the default applies with no warning.

Logical pixels mean the size is scaled by the display's DPI factor, so the same
number is "about this big" on a 4K monitor rather than physically tiny.

### `entry`

The page the first window loads, relative to the served root. Defaults to
`index.html`.

```json
{ "entry": "settings.html" }
```

Leading slashes are trimmed, so `"/index.html"` and `"index.html"` are identical.
The page must exist in the embedded asset table, or the window opens on a 404.

Precedence for the first page, highest first:

1. `App::entry(...)` in `main.rs`
2. `App::window(WindowOptions { entry: Some(...) })`
3. `entry` in this file

Later windows choose their own page through `WindowOptions::entry` when they are
opened; this key only affects the first.

### `command_timeout_ms`

How long the page waits for a command before rejecting the promise. Default
**30000** (30 seconds). `0` disables the deadline.

```json
{ "command_timeout_ms": 300000 }
```

**Why a deadline exists at all:** a promise that never settles is
indistinguishable from a frozen UI, and the user force-quits. A `timeout` is a
better failure than a hang, even when the work is legitimate and long.

Three levels, most specific first:

```ts
// 1. per call, from the page
await window.voussoir.invoke("export", payload, { timeout: 300_000 });
// 2. this file, for every call
// 3. 30 s, if neither is set
```

A timeout rejects the promise but **does not cancel the handler**: the work may
still be running, and may still write. Do not retry a `timeout` blindly. See
[Errors the page can branch on](errors.md).

### `commands`

The global allowlist. **Absent means every registered command is callable.**

```json
{ "commands": ["greet", "read_file", "storage.get"] }
```

This is the key that surprises people: leaving it out does not lock anything
down, it opens everything. It is absent by default because an app under
development wants its new command callable without a config edit.

The security consequence is worth stating plainly: **if you never write
`commands`, your entire registered surface is reachable from the page.** Built-in
commands count, including `__voussoir_open_window` and `storage.set`. For an app
that renders only its own content that is usually acceptable; for one that renders
user content it is not.

**What `commands` does not gate:** the per-window lists. A menu item wired to a
command runs through `run_command`, which dispatches with no window label, and an
unlabelled request skips the `windows` narrowing. So a menu command is checked
against `commands` but never against a window's list.

That asymmetry is deliberate. A window list exists to contain *that window's page*,
and a menu click comes from OS chrome the user already owns. The practical
consequence is worth remembering: **a menu item whose command you removed from
`commands` fails silently**, because `run_command` discards the reply.

### `windows`

Per-label narrowing. Keyed by window label; each entry has its own `commands`
list.

```json
{
  "commands": ["greet", "save", "preview.render"],
  "windows": { "preview": { "commands": ["preview.render"] } }
}
```

**The intersection rule:** a window's effective surface is its list **and** the
global list. Adding a command to `windows.preview` that the global list omits grants
nothing, so a config entry can only ever remove authority.

An empty list is meaningful, and different from omitting the key:

| Setting | Effective surface |
| --- | --- |
| no `windows` entry | the global surface |
| `{ "commands": [] }` | nothing at all |
| `{ "commands": ["a"] }` | `["a"]` ∩ global |

The label is read from the webview the request arrived on, never from the payload,
which is what makes this enforceable. See
[Per-window allowlists](allowlists.md).

### `update.url`

The JSON endpoint `update.check` reads.

```json
{ "update": { "url": "https://example.com/releases/latest.json" } }
```

Absent means the command answers `{"configured": false, "available": false}`
rather than failing, so an app can ship before its update feed exists. The
endpoint's shape and version comparison are in [Updating the app](updater.md).

### `version`

The app's own version, compared against the feed by `update.check` and
`update.apply`.

```json
{ "version": "2.1.0" }
```

Keep it equal to the `version` in your own `Cargo.toml`. Absent, the updater falls
back to the framework's crate version (0.2.0), which is rarely the version your
releases use.

### `update.public_key`

The base64 Ed25519 public key an update artifact must be signed with.

```json
{
  "update": {
    "url": "https://example.com/latest.json",
    "public_key": "Q2hlY2sgdGhlIGtleSB3aXRoIGtleWdlbg=="
  }
}
```

`voussoir keygen` prints this value. `update.apply` refuses without it, because a
digest alone cannot prove who produced an artifact; `update.check` still works so an
app can report a version before its key is set up.

## Theory: what is deliberately not in the file

A config file invites every setting to move into it. These four stay out on
purpose:

| Not configurable here | Where it lives | Why |
| --- | --- | --- |
| the CSP | the shell, compiled in | a policy that can be relaxed by editing a file is not a policy |
| allowed navigation | the shell, compiled in | same reason; the dev origin is the only runtime input |
| the command list itself | `commands/src/lib.rs` | commands need functions |
| theme, language, preferences | `storage` | they are per user, not per build |

The pattern is that anything deciding **what the app may do** is compiled in, and
anything deciding **how it looks to this user** is stored at runtime.

## Verifying a config

There is no `validate` subcommand; the shell itself is the checker, and it fails
at startup with a message. The fastest loop is:

```bash
voussoir dev          # parse + run; an invalid file names the problem
```

To check an allowlist specifically, run the app and read stderr: at startup it
audits every name in `commands` and `windows` against the registry and reports the
ones no handler can answer, which catches a rename.

## Common mistakes

| Mistake | Effect | Fix |
| --- | --- | --- |
| `"width": "900"` | ignored, default size | use a number |
| omitting `commands` | every command is callable | list them |
| `"preview": []` instead of `{"commands": []}` | entry ignored | the value is an object |
| renaming `name` after shipping | storage file moves, settings lost | rename before release |
| expecting a rebuild-free config change | it is embedded | rebuild |
| `"entry": "settings.html"` for a file not in the build | window shows a 404 | check the asset table |

## Next

- [How a project is laid out](layout.md) — every file, including this one.
- [Per-window allowlists](allowlists.md) — the intersection rule in depth.
- [Security model](security.md) — what the allowlist is defending against.
