# Security policy

Voussoir runs untrusted web content against a native bridge and ships a
self-updater that can replace its own binary, so a mistake here is not cosmetic.
This document says how to report one and what to expect.

## Reporting a vulnerability

Report privately through GitHub's advisory form:

<https://github.com/smegasberla/voussoir/security/advisories/new>

That route is preferred over a public issue because a public issue is a working
exploit for every deployment that has not updated yet.

If you cannot use GitHub advisories, email <smegasberla@gmail.com> with
`VOUSSOIR SECURITY` in the subject. Please do not open a public issue, and please
do not include a weaponised payload in the first message; a description and the
version is enough to start.

## What to include

- The version (`voussoir --version`, or the crate version) and the platform.
- Whether the webview is WebKitGTK, WebView2 or WKWebView, since the guarantees
  differ per engine.
- The smallest reproduction you have. A command name and the payload it was sent
  beats a hundred lines of prose.

## What to expect

This is a single-maintainer project, and the response times below are stated so
you can plan around them rather than be surprised by them:

| Step | Target |
|---|---|
| Acknowledgement that the report arrived | 7 days |
| An initial assessment: affected versions, whether it is in scope | 14 days |
| A fix, or a documented decision not to fix | 90 days for a high-severity issue |

If a fix is not going to land in that window, the advisory stays open and you get
the reasoning rather than silence. Credit is given in the advisory unless you ask
otherwise.

## Scope

In scope:

- The IPC bridge: the envelope, the per-window allowlist, and anything that lets a
  page call a command it was not granted.
- The content policy: a way to get a remote script into a page that holds the
  bridge, or to navigate away from the app origin.
- The updater: anything that gets unsigned or unverified bytes installed, or that
  makes the verifier trust a feed it should refuse.
- The custom protocol handler and asset resolution: a path that escapes the asset
  table.
- The CLI, when it writes files an app then runs.

Out of scope:

- A vulnerability in the operating system's webview engine itself. Report those to
  the engine vendor; Voussoir can only pin or work around them.
- Anything that requires an attacker who can already write to the app's binary,
  its config, or the user's home directory. That attacker has the machine.
- Denial of service that needs the page to already be trusted code you shipped.
- The absence of a feature (no automatic background updates, no multi-window
  sandboxing on iOS) unless it is exploitable rather than merely missing.

## What is already guaranteed

These are asserted in CI rather than promised in prose, and a report against them
is a real bug:

- Every update artifact is checked against the manifest's SHA-256, and, when a key
  is configured, its Ed25519 signature, before anything is renamed into place.
- Patches are rebuilt from the running binary and then verified exactly as a full
  download would be, so a hostile patch fails the signature instead of installing.
- The update URL comes from the app's config, never from the page.
- A build that does not shut down cleanly is reverted on the next launch.

## Supported versions

While the crate is 0.x, security fixes land on the latest published version only.
Pin a minor version if you need stability, and read the changelog before moving.
