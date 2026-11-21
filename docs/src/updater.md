# Updating the app

Voussoir gives you one command that answers "is there a newer version", and
deliberately nothing that installs one. This chapter covers the command, the
endpoint it reads, and why the download is left to you.

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
| How is it downloaded and verified? | you | needs a signature, a progress bar, a rollback |

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

**`current` is not your app's version.** It is `env!("CARGO_PKG_VERSION")` evaluated
inside the `voussoir` crate, so it reports the framework's version (0.2.0), not the
`version` in your own `Cargo.toml`. `env!` is resolved by the compiler in the crate
that contains it, and the crate that contains it here is `voussoir`, not yours.

That makes `available` a comparison between the framework's version and your feed,
which is almost never what you want. Two honest options:

| You want | Do this |
| --- | --- |
| to know a new app version exists | write your own check: a command that fetches the same JSON and compares against your own version |
| the built-in to be meaningful | keep the feed's shape and treat `latest` as data, ignoring `available` |

Passing your own version in is not possible: `UpdateConfig::from_config` takes the
string, but the only caller is `register`, which supplies the crate's constant.

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
- **The download opens in the browser.** Navigation inside the app is refused by
  policy (see [Security model](security.md)), and an OS browser shows the real URL
  and the publisher's certificate.

## Theory: where to call it

Once, at startup, after the first paint. Not before: a network call in front of the
first frame makes every launch feel slower than it is, and the result is not needed
to render anything.

If you need to respect a release cadence, store the last check time in
[key/value storage](storage.md) and skip the call for a day. That is one `storage.get`
and one `storage.set`, and it turns a per-launch request into a per-day one.

## Android and iOS

`update.check` works on mobile too, but a store may forbid an app from prompting for
a side-loaded update: Play and the App Store both have policies about it. On those
platforms the honest use is to *report* the version, or to disable the prompt and
let the store do the updating.

The fetch goes through the platform's HTTP stack on Android, not a bundled TLS
library, because the platform store already carries trusted roots. On iOS the
updater's endpoint fetch reports unsupported; the command says so rather than
silently failing.

## What is not built in

- **No download, no checksum, no signature check.** If you ship self-updating, that
  is your code and your responsibility.
- **No delta patching.**
- **No progress UI.**

Each of these is a decision with a security implication, and a framework that makes
it for you is a framework that makes it wrong for someone.

## Next

- [Packaging](packaging.md) — what a version number means once it ships.
- [Compatibility policy](compatibility.md) — what an upgrade costs.
