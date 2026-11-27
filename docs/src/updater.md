# Updating the app

Voussoir answers "is there a newer version" and, on desktop, can download,
verify and install the build the feed names. Installing stays a separate command
the page calls on purpose, because replacing a running binary is the one moment
everything before it has to be right. This chapter covers both commands, the
endpoint they read, and the trust model install rests on.

## Theory: why checking and installing are separate

An app that can replace itself is an app that can be made to run arbitrary code by
whoever controls the update feed. That is not a hypothetical: it is the mechanism
supply-chain attacks need, and it is why so much security advice about desktop
software is "keep it updated" and "check for updates" at the same time.

Splitting the two makes each decision explicit:

| Decision | Who makes it | Why |
| --- | --- | --- |
| Is there a newer version? | `update.check` | cheap, read-only, safe to automate |
| Should this user install it now? | your app | needs consent, disk space, a restart story |
| How is it downloaded and verified? | `update.apply` | the feed must name a `sha256`; the bytes must match it |

So the built-in command stops before the risky part and hands you the facts.

## The command

```ts
const result = await window.voussoir.invoke<{
  configured: boolean;
  current?: string;
  latest?: string;
  available?: boolean;
  notes?: string | null;
  url?: string | null;
}>("update.check");
```

Four possible outcomes, and all four are normal:

| Outcome | Response |
| --- | --- |
| no endpoint configured | `{configured: false, available: false}` |
| endpoint is current | `{configured: true, current, latest, available: false, ...}` |
| endpoint is newer | `{configured: true, current, latest, available: true, ...}` |
| endpoint is invalid | `kind: "handler"` with a message naming the problem |

**`current` is your app's version when `voussoir.json` has one.** Put the same
string in the config's `version` as in your own `Cargo.toml`:

```json
{ "version": "2.1.0", "update": { "url": "https://example.com/releases/latest.json" } }
```

Without a `version` the command falls back to `env!("CARGO_PKG_VERSION")` evaluated
inside the `voussoir` crate, which is the framework's version (0.1.0) and almost
never what your feed tracks. Older docs told you to work around that by writing your
own comparison; that is no longer necessary.

**Why "not configured" is a success, not an error:** an app should be shippable
before its update feed exists, and a first-run error dialog about a missing
endpoint teaches users to ignore error dialogs. The command answers honestly and
your page decides whether to show anything.

**Why an invalid endpoint is an error:** a feed that returns bad JSON or no version
is a bug in your deployment, not a state the app can act on. Failing loudly is what
makes it visible.

## The endpoint

`voussoir.json`:

```json
{ "update": { "url": "https://example.com/releases/latest.json" } }
```

The file it fetches:

```json
{
  "version": "1.4.0",
  "notes": "Fixes the export dialog on Windows.",
  "url": "https://example.com/releases/1.4.0/app.AppImage"
}
```

- **`version`** is required. Its absence is the invalid-endpoint error.
- **`notes`** is optional and passed through untouched, so it can be Markdown.
- **`url`** is optional and passed through untouched. The command does not fetch it.

## Theory: how versions are compared

`is_newer` splits both strings on `.` and compares the parts as numbers, left to
right:

```
1.4.2  vs  1.4.10   ->  1.4.10 wins (10 > 2, numerically)
1.4    vs  1.4.0    ->  not newer (missing parts count as 0)
1.4.0-beta  vs  1.4.0  ->  not newer (a non-numeric part counts as 0)
```

Three properties follow, and the third is a limitation worth knowing:

1. **Numeric, not lexical.** `1.4.10` is newer than `1.4.2`, which string
   comparison gets backwards.
2. **Padding is free.** `1.4` and `1.4.0` are the same version.
3. **Pre-release tags do not order.** `1.4.0-beta` is treated as `1.4.0`, so a
   beta feed will not upgrade a user to a final release. If you need semver
   precedence, compare yourself: `notes` and `latest` are both handed to you.

That last one is a deliberate scope decision. Full semver precedence is a spec with
its own edge cases, and most apps release plain `x.y.z`. A build that needs
`1.0.0-rc.1 < 1.0.0` should not have that decided by a convenience helper.

## A page that offers the update

```ts
async function checkForUpdate(): Promise<void> {
  let info;
  try {
    info = await window.voussoir.invoke("update.check");
  } catch {
    return; // an update check must never interrupt the user
  }
  if (!info.configured || !info.available) return;

  const go = await confirm(`Version ${info.latest} is available. Open the download page?`);
  if (go && info.url) {
    // target="_blank" hands it to the OS browser; the CSP blocks navigation.
    window.open(info.url, "_blank");
  }
}
```

Two rules in that snippet carry most of the value:

- **A failed check is silent.** No dialog, no toast. The user did not ask for an
  update check and should not learn about your server being down.
- **The install is not silent.** `update.apply` only runs when your page calls it,
  and it refuses a feed whose artifact has no digest. If you would rather hand the
  artifact to the OS, opening the release page in the browser still works: the
  installer shows the real URL and the publisher's certificate.

## Theory: where to call it

Once, at startup, after the first paint. Not before: a network call in front of the
first frame makes every launch feel slower than it is, and the result is not needed
to render anything.

If you need to respect a release cadence, store the last check time in
[key/value storage](storage.md) and skip the call for a day. That is one `storage.get`
and one `storage.set`, and it turns a per-launch request into a per-day one.

## Applying the update

Two commands, desktop only. On Linux, macOS and Windows:

```ts
const result = await window.voussoir.invoke<{
  installed: boolean;
  version?: string;
  path?: string;
  notes?: unknown;
  reason?: string;
}>("update.apply");
```

`update.apply` fetches the feed from `update.url` (never from the page), requires a
newer `version`, downloads the artifact with a deadline and a 256 MiB cap, checks
its SHA-256 against the feed, writes it beside the running binary and renames it into
place. Nothing is installed when any of those steps fails, and the old binary keeps
running. Pass `{ "force": true }` to reinstall the same version.

```ts
await window.voussoir.invoke("update.relaunch");
```

`update.relaunch` starts the installed binary and exits this process, after the
reply has been delivered. It is a separate command so the page can show "restarting"
first. It refuses unless `update.apply` installed something in this process, so a
page cannot ask the shell to run an arbitrary path.

**The manifest.** Either one artifact at the top level, or one per platform:

```json
{
  "version": "2.1.0",
  "notes": "Fixes a crash on save.",
  "url": "https://example.com/releases/app-linux-x86_64",
  "sha256": "9f2c... (64 hex characters)",
  "signature": "base64 Ed25519 over the artifact bytes"
}
```

```json
{
  "version": "2.1.0",
  "downloads": {
    "linux-x86_64": { "url": "https://example.com/app-linux-x86_64", "sha256": "...", "signature": "..." },
    "macos-aarch64": { "url": "https://example.com/app-macos-aarch64", "sha256": "...", "signature": "..." },
    "windows-x86_64": { "url": "https://example.com/app.exe", "sha256": "...", "signature": "..." }
  }
}
```

A build reads `linux-x86_64`, `linux-aarch64`, `macos-x86_64`, `macos-aarch64`,
`windows-x86_64` or `windows-aarch64`. `sha256` is mandatory and is checked before
anything touches the disk.

**The trust model, plainly.** The digest proves the bytes are the ones the feed
named; the Ed25519 signature proves the publisher produced them, which is what a
digest alone cannot do. `update.apply` refuses unless `update.public_key` is set and
the feed carries a `signature`; `https` is still required everywhere except loopback
for fetching the feed.

Generate the key once per project with `voussoir keygen --out release.pem`. It prints
the base64 key to paste into `voussoir.json`, and writes the private half with mode
`0600`. Sign each artifact with `voussoir sign dist/app --key release.pem` and paste
the `sha256` and `signature` it prints into the feed. The key is raw 32-byte
Ed25519; the signature is over the artifact's exact bytes, not over the JSON, which
removes canonicalization from the trust path.

**Patch updates.** A feed may offer deltas from several older versions, oldest
first:

```json
"patches": {
  "linux-x86_64": [
    { "from": "1.9.0", "url": "https://example.com/1.9.0-to-2.1.0.patch", "sha256": "...", "signature": "..." },
    { "from": "2.0.0", "url": "https://example.com/2.0.0-to-2.1.0.patch", "sha256": "...", "signature": "..." }
  ]
}
```

When the running `version` matches one of the `from` values, `update.apply`
downloads that patch, checks it, applies it to the running binary, and then verifies
the reconstructed file against the feed's `sha256` **and** `signature`, exactly as a
full download would be. A patch that produces anything else is refused and the old
binary stays.

Every entry rebuilds *this* release from its own version, so a user who skipped
releases still downloads a delta rather than the whole artifact: someone on `1.9.0`
reaches `2.1.0` through the first entry without visiting `2.0.0`. The list is
ordered oldest first, and the first match wins, which is the furthest back the
publisher supports. A single object instead of a list is still read, so a feed
written before this shape keeps working.

`voussoir release` writes this per platform and signs every hop. `voussoir patch`
builds one delta by hand when you are assembling a feed yourself. The format is
copy-runs-and-inserts, documented in `voussoir/src/delta.rs`. Inserted machine code
repeats, so a patch is deflated (raw, no container) when that is smaller, marked
`VSC1` instead of `VSD1`. A tiny patch stays uncompressed, both forms are read, and
inflation is bounded by the same output cap. On a real 962 KB release pair a build
with genuine code changes went from 45% of the artifact to 23%.

**Progress.** While `update.apply` downloads it emits
`voussoir.listen("update-progress", ({received, total}) => ...)`, at most every
150 ms plus one final event whose `received` equals `total`. `total` is `null` when
the server sends no `Content-Length`.

On Unix the new file takes the old one's mode and is renamed over it, which a running
process allows. Windows refuses to replace a running executable but allows renaming
it, so the old file steps aside as `<name>.old`, is restored if the follow-up rename
fails, and is swept up by the next install. That strategy is an ordinary function
rather than something inside a `cfg(windows)` block, so the suite exercises it on
every platform: the branch that only runs on Windows would otherwise be reviewed
and never run, and it is the path that can lose a working binary.

**Launch and crash rollback.** Renaming safely protects the install, not the launch.
A build that is the wrong architecture, misses a library, panics before its first
window, or dies at any point during use replaces a working app with a broken one, and
nothing at install time can tell.

So an install leaves two files beside the binary: `<name>.voussoir-previous`, a copy
of the build that was running, and `<name>.voussoir-pending`, naming the version just
installed. The first launch of the new build increments that marker and proceeds.

The marker is deleted on exactly one event: a **clean shutdown**, from the event
loop's teardown. Only a deliberate exit proves a build runs, so a session that ends
any other way leaves the marker where it is, whether the build never reached a
window, panicked later, was killed, or lost power. The next launch therefore has
something to act on: the previous copy is renamed back over the build, started, and
one line on stderr says which version was restored and, when the build managed to
record a panic, why it stopped.

Two launches, not one, because a launch can fail for reasons unrelated to the
update: no display, a locked profile, a kill from the OS. Rolling back on the first
failure would undo good updates. The same trade applies to the crash case, and it is
worth stating plainly: an app killed from outside looks exactly like one that
crashed, so a good build can be reverted once after a kill. It is then clean, the
marker is gone, and the update is simply offered again. That is the intended
trade, because a build that crashes is worth reverting even at the price of
reverting one that was merely interrupted. When no copy could be kept, the marker is
never written and the update simply cannot be undone.

## Android and iOS

`update.check` works on mobile too, but `update.apply` and `update.relaunch` are not
registered there: Play and the App Store own installation, and both have policies
about prompting for a side-loaded update. On those platforms the honest use is to
*report* the version, or to disable the prompt and let the store do the updating.

The fetch goes through the platform's HTTP stack on Android, not a bundled TLS
library, because the platform store already carries trusted roots. On iOS the
updater's endpoint fetch reports unsupported; the command says so rather than
silently failing.

## What is not built in

- **No key rotation.** One `update.public_key` per config; switching keys means
  shipping a build with the new one.
- **No rollback while the app is still running.** A build that misbehaves without
  exiting (a broken page, a command that returns the wrong data) is a bug to fix,
  not an install to undo; the reversion happens on the next start, and only when the
  previous session did not shut down cleanly.
- **No background updates.** `update.apply` runs when the page asks; nothing checks
  on a timer, because that decision belongs to the app's UX and its store policy.

Each of these is a decision with a security implication, and a framework that makes
it for you is a framework that makes it wrong for someone.

## Next

## How mature is this?

The mechanics are tested: a signed artifact, a decompressed patch, and a rollback
after a launch that never confirmed each have a test that fails without them. What
cannot be tested in a checkout is field exposure. This path shipped in 0.1.0, so it
has not yet carried a genuinely broken release to users and back.

Treat it accordingly: run your own update against a staging feed once, deliberately,
before you need it to work. Silently trusting an untried update path is how a
self-updating app becomes unrecoverable.

- [Packaging](packaging.md) — what a version number means once it ships.
- [Compatibility policy](compatibility.md) — what an upgrade costs.
