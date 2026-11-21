# Security model

A Voussoir app treats the page as **untrusted**. Not because you wrote it, but
because it renders content you did not: markdown, remote images, a fetched article,
a link a user pasted. The policy assumes the worst about what runs inside the
webview and gives Rust the final say over what it can reach.

Five rules, each enforced by the shell, each proved by a test:

1. The page loads only from the app's own origin.
2. A response carries a Content-Security-Policy that forbids remote script.
3. Navigation out of the app is refused.
4. Every command call passes an allowlist before a handler is looked up.
5. A window can be given a narrower list than the app.

## Theory: what a threat model is, and why this app needs one

A threat model is a short list of "what could go wrong, and what stops it". You do
not need a hostile user to benefit from one. The interesting attack surface of a
desktop app is usually **the content it displays on someone else's behalf**.

Concretely, three ways a Voussoir app could be attacked:

**1. A foreign page replaces your UI.** If navigation were free, a link, a redirect,
or a badly written `location.assign` could put someone else's page in your window.
There is no address bar and no back button, so the user has no way to tell it is not
you, and no way to leave.

**2. Remote script runs with your app's authority.** If the policy allowed remote
scripts, a compromised CDN, a hijacked dependency, or an injected `<script src>`
would run inside the document that holds the IPC bridge. That script could call
every command you registered, including the one that writes files.

**3. A narrow window escalates.** If a window rendering untrusted content could call
the whole app, then the isolation you intended is decoration: the untrusted content
reaches your file access through the window instead of directly.

This is a **confused deputy**, and it is the pattern behind almost every real
failure here. The shell is a deputy with real authority (filesystem, network,
processes). A deputy is confused when something with less authority asks it to act
on its behalf without the deputy checking *who asked*. The page is the thing with
less authority. Every rule below is a check on who asked.

The rules answer those attacks in order. Each one is a pure function where it can
be, so it is testable without opening a window, which is why this chapter can cite
tests instead of promises.

## Rule 1: one origin, and only that origin

Production serves the page from `voussoir://localhost`. On Windows and Android,
where the webview cannot intercept a custom scheme, the same origin is spelled
`http://voussoir.localhost`. Both are allowed; nothing else is.

```rust
pub fn navigation_allowed(url: &str, dev_origin: Option<&str>) -> bool
```

The policy is short enough to state in full:

| URL | Allowed? | Why |
| --- | --- | --- |
| `voussoir://localhost/index.html` | yes | the app's own origin |
| `http://voussoir.localhost/index.html` | yes | the same origin, rewritten spelling |
| `about:blank` | yes | the initial document; some libraries use it |
| the dev-server origin, when one was configured | yes, dev only | Bun's server, passed in explicitly |
| `https://example.com` | no | a foreign document |
| `file:///etc/passwd` | no | a local file is not your app |
| `voussoir://localhost.evil.com/` | no | a different host that merely starts the same way |

That last row is the one to look at twice. The check compares whole origins, then
allows a path suffix under it, rather than testing `starts_with` on the origin
string. A prefix test would accept `voussoir://localhost.evil.com`, which is exactly
the bug a hand-rolled allowlist usually has.

**`dev_origin` is `None` in a release build.** That is not a default that a caller
can forget to tighten: the value comes from the build configuration, so a shipped
binary has no dev origin to be redirected to.

**Why an origin rather than a file path?** An origin is one of the few things a
webview will not let a page lie about. Paths, headers and referrers can be crafted;
the origin is assigned by the engine from where the bytes came.

See [The two origins](origins.md) for the full story on the spellings.

## Rule 2: a Content-Security-Policy that forbids remote script

Every response carries a CSP header. This is the release policy, quoted from the
source:

```
default-src 'self';
script-src 'self';
style-src 'self' 'unsafe-inline';
img-src 'self' data: blob:;
font-src 'self' data:;
connect-src 'self' voussoir:;
object-src 'none';
base-uri 'none';
form-action 'none';
frame-ancestors 'none'
```

Each directive, and the attack it closes:

| Directive | Forbids | What that stops |
| --- | --- | --- |
| `script-src 'self'` | remote and inline script, `eval` | a compromised CDN or injected tag reaching the bridge |
| `style-src 'self' 'unsafe-inline'` | remote stylesheets | inline `<style>` is normal in bundled CSS and cannot reach the bridge, so it is allowed |
| `img-src 'self' data: blob:` | images from a remote host | an image cannot exfiltrate by itself, so `data:` and `blob:` stay open |
| `connect-src 'self' voussoir:` | `fetch` to another host | a page cannot POST your data to an attacker |
| `object-src 'none'` | plugins and `<embed>` | legacy engine surfaces |
| `base-uri 'none'` | `<base href>` | a rewritten base changing where every relative URL points |
| `form-action 'none'` | form posts | a form is a GET/POST with no JavaScript, and it would bypass `connect-src` |
| `frame-ancestors 'none'` | being framed | clickjacking by another app embedding you |

**`script-src 'self'` matters most.** Without it, a single injected `<script src>`
defeats every other rule in this chapter, because the script runs inside the same
document as your code and inherits the bridge. Every other directive is defence in
depth around that one.

**Android has its own constant**, `ANDROID_RELEASE_CSP`, spelled the same way except
that `connect-src` is `'self'` alone: the bridge there goes over `window.ipc` rather
than an HTTP endpoint, so the `voussoir:` scheme is not needed. It is a separate
constant because the branch that selects a policy is compile-time, so an assertion
inside it would only ever run on the platform that matches.

**In a dev build the policy is relaxed**, and this is the one place where the model
is weaker on purpose:

```
script-src 'self' 'unsafe-inline' 'unsafe-eval' voussoir:;
connect-src 'self' voussoir: http://voussoir.localhost ws: http://127.0.0.1:* http://localhost:*;
```

Bun's hot-reload client uses inline script, an eval-based module loader, and a
websocket. None of that can work under the release policy, and the relaxation is
scoped to a dev build, never to a shipped app. **The practical consequence is a
dev/pop-in-release bug:** code that runs under `voussoir dev` can fail after
`voussoir build` because `eval` and inline script stop working. If something breaks
only in a release build, check this list first.

## Rule 3: navigation out of the app is refused

`navigation_allowed` is consulted for every navigation, and a refusal is recorded as
a `[security]` diagnostic entry rather than swallowed.

What that means for you, in practice:

| You want to… | Do this | Because |
| --- | --- | --- |
| open a web page | `<a target="_blank">` | the shell hands it to the OS browser |
| show your own docs | a second window, or `target="_blank"` | navigation would replace your UI |
| switch views | render a different view in the same document | no load, no state loss |
| load another bundled page | a relative link | same origin, so it is allowed |

**Why refusal rather than a warning:** because there is no back button. A page that
navigated somewhere else would strand the user, and if the destination could hold
the bridge, refusal is also the boundary that keeps authority attached to code you
wrote. This is not a limitation to work around; it is the reason the rest of the
model holds.

## Rule 4: the allowlist gates every request, before the handler lookup

`dispatch_gated` checks the allowlist **before** it looks for a handler:

```rust
if !self.permitted_for(method, label) {
    return done(error_envelope_for(
        method,
        &Error::UnknownMethod(method.to_owned()),
    ));
}
```

Order matters more than it looks. Because the check comes first, and because a
denial answers exactly what a missing command answers, **the page cannot tell a
withheld command from a nonexistent one**. Both are:

```json
{"error": {"kind": "unknown", "method": "storage.delete", "message": "unknown command: storage.delete"}}
```

**Why that is deliberate.** If a denial answered `forbidden`, a page rendering
untrusted content could enumerate what the app *can* do, and file that away for a
later attempt. `unknown` gives it nothing to learn. If your own page needs to know
whether a feature exists, ask about the feature (your own capability command), not
about a command name.

The check also runs on the hot-reloaded library path, so a command cannot become
reachable in development by existing in a library the config does not mention.

**Guidance:** the allowlist is where authority is declared. Declare it in
`voussoir.json`; do not rely on the default. See
[Per-window allowlists](allowlists.md) for the exact rule and the surprising
default.

## Rule 5: a window can be narrower than the app

Each window can carry its own list, and the two are **intersected** rather than
replaced. A window list can only ever remove authority.

```rust
App::new().open_window(WindowOptions {
    label: Some("preview".into()),
    allowlist: Some(vec!["render".into()]),
    ..Default::default()
})
```

A `preview` window that renders user documents can call `render` and nothing else,
while the main window keeps the whole surface. If the document contains a
`<script>`, it runs with `render`'s authority, which is small by construction.

**The label comes from the webview, not the payload.** That is what makes the
narrowing enforceable: a page cannot claim to be a different window, because the
shell reads the label from the webview the request arrived on.

**One asymmetry worth memorising:** a menu item's command is dispatched with no
label, so it is checked against the global list and never against a window's list.
A menu item wired to a command you removed from `commands` fails **silently**,
because the reply is discarded. That is the failure people lose an hour to.

## Theory: what this model does not protect

Being explicit about the limits is part of the model, not a disclaimer.

| Not protected | Why | What to do |
| --- | --- | --- |
| **arguments** | the allowlist is command-level | a command taking a path must validate it itself |
| a **buggy command** | the shell cannot know the handler is wrong | treat each command as an API endpoint |
| **XSS inside your own page** | the bridge is on `window` by design | escape rendered content; the CSP blocks remote script, not a `innerHTML` bug |
| the **dev server** | any local process can connect to it | it is a development tool; do not expose the port |
| the **OS user's own authority** | the app runs as you | it cannot be less privileged than its user |
| **secrets in the bundle** | the frontend is embedded and readable | a secret belongs in Rust or nowhere |
| **signing** | packaging leaves it to you | see [Packaging](packaging.md) |

The third row deserves a second sentence. If your page puts untrusted text into
`innerHTML`, an attacker can run script **inside your origin**, and that script has
the bridge. The CSP does not stop it, because the code is already in a document you
served. `textContent` instead of `innerHTML`, or a templating call that escapes, is
the fix. This is the one vulnerability class the framework genuinely cannot prevent
for you.

The second row is the same lesson from another angle: the allowlist decides *which*
command, and the command decides *what it may do*. `open_file(path)` with no
validation is a confused deputy no matter how short the allowlist is.

## Auditing your own surface

```bash
voussoir doctor          # toolchain and project shape
voussoir dev --verbose   # records refused navigations and slow commands
```

At startup, the registry logs any name in the allowlist that no handler can answer.
That catches the commonest configuration mistake, a typo in `voussoir.json`, as a
line on the terminal instead of a page that mysteriously gets `unknown`.

From the page, `__voussoir_diagnostics` returns the recorded entries, so a support
build can show what was refused without a console.

## Next

- [The two origins](origins.md) — why the origin has two spellings.
- [Per-window allowlists](allowlists.md) — the exact gating rule.
- [Errors the page can branch on](errors.md) — what a denial looks like.
- [Commands & payloads](ipc-contract.md) — the envelope these rules guard.
