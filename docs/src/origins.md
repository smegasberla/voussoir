# The two origins

A page's origin is one of the few things a webview will not let you lie about, and
Voussoir's whole policy is built on it. There are two spellings, and knowing which
you are on explains three otherwise confusing behaviours.

## Theory: what an origin is, and why it is the unit of policy

An origin is the triple *scheme + host + port* — for example
`https://example.com:443`. The web security model treats it as the boundary
between "my code" and "someone else's code": storage, scripts and permissions are
scoped to it, and a page cannot read another origin's data.

That is why Voussoir serves your page from its own origin rather than from
`file://`:

- `file://` has an opaque origin on several engines, so the policy cannot name it
  reliably, and it collides with the user's own files.
- A custom scheme gives one stable string that means exactly "your app".

The shell intercepts requests for that scheme and answers them from the embedded
asset table. Nothing listens on a port, and nothing is reachable from outside the
process.

## Desktop

Pages are served from `voussoir://localhost`, a custom scheme the shell
intercepts itself. `window.location.origin` is `voussoir://localhost`, and the
security policy lists exactly that.

## Windows

WebView2 cannot intercept a custom scheme. `wry` therefore rewrites
`voussoir://localhost` to `http://voussoir.localhost` and intercepts that instead.
Same handler, same document, different origin string.

This is the one place a cross-platform app can notice the platform in its own
URLs. Two consequences:

- A page that hard-codes `voussoir://localhost/asset.png` in HTML gets a 404 on
  Windows. Use relative URLs, or read `voussoir.pageOrigin` when an absolute URL
  is genuinely needed: it carries the spelling this page was actually served
  from, so the same code is correct on every target.
- `voussoir.platform === "windows"` is available when a page genuinely needs the
  branch, but it should be rare: the shim's own `invoke` already talks to the
  right endpoint on both.

**Why `voussoir.localhost` and not `localhost`?** Earlier versions used
`http://localhost`, which collided with any local dev server and made the CSP
ambiguous: `connect-src 'self'` would permit a page to reach whatever happened to
be listening. `voussoir.localhost` is reserved for this purpose, so it cannot
collide with a real host.

## Android

`WebResourceRequest` carries no body, so the custom protocol cannot carry an
`invoke` payload at all. Android uses the webview's `postMessage` bridge instead,
and Rust resolves the page's promise from the other side.

The envelope, registry, allowlist and policy are shared; only the transport
differs. That is why `invoke` looks identical in page code on every platform:

```
desktop, iOS:   POST voussoir://localhost/__voussoir/<method>
Android:        postMessage + a promise resolved from Rust
```

The consequence for app code is zero: a command behaves the same. The consequence
for a page is that `fetch("voussoir://localhost/__voussoir/…")` is not the
transport on Android, so call `window.voussoir.invoke` rather than reaching for the
endpoint by hand.

That second point is not stylistic. Reaching for the endpoint is the one thing
that makes an app desktop-only by accident.

## iOS

`WKURLSchemeHandler` intercepts `voussoir://localhost` the way the desktop does, so
the origin and the policy are the desktop ones. This is deliberate: it is the one
mobile platform where the insecure-by-default `http://` workaround is not needed.

## Table: which origin each target serves from

| Target | Engine | Origin the page runs on | Why |
| --- | --- | --- | --- |
| Linux, BSD | WebKitGTK | `voussoir://localhost` | the custom scheme can be intercepted |
| macOS | WKWebView | `voussoir://localhost` | `WKURLSchemeHandler` exists |
| Windows | WebView2 | `http://voussoir.localhost` | WebView2 cannot intercept a custom scheme, so the host form is rewritten and intercepted |
| Android | system WebView | `http://voussoir.localhost` | same limitation |
| iOS | WKWebView | `voussoir://localhost` | `WKURLSchemeHandler` exists |

So the answer to "which origin am I on" is not "the mobile one" or "the desktop
one"; it is a property of the engine, and the two mobile platforms differ from each
other.

## The check behind all of this

`navigation_allowed` compares whole origins, then allows a path under one:

```rust
[APP_ORIGIN, REWRITTEN_APP_ORIGIN]
    .iter()
    .any(|origin| url == *origin || url.starts_with(&format!("{origin}/")))
```

The `format!("{origin}/")` in the prefix is the whole trick. Testing
`starts_with(APP_ORIGIN)` would accept `voussoir://localhost.evil.com`, whose text
begins with the right characters and whose host is somebody else's. There is a unit
test named `prefix_match_does_not_leak_across_hosts` that pins exactly that case, so
the bug cannot come back quietly.

## Theory: why not unify the spellings

Because each engine's constraint is different, and hiding the difference would
mean spoofing an origin, which is precisely what a webview prevents. The honest
move is to give page code one thing that works everywhere (`invoke`, relative
URLs) and one escape hatch (`pageOrigin`) for the rare case that needs the string.

## What to write in a page

Relative URLs, and `window.voussoir.invoke`. Both work on every platform, and
neither requires the page to know which origin it is on.

When you cannot avoid an absolute URL, build it from `voussoir.pageOrigin` rather
than from a literal:

```ts
const url = new URL("asset.png", window.voussoir.pageOrigin).href;
```

`voussoir.pageOrigin` arrived in shim protocol 3, so the honest feature test is:

```ts
const base = window.voussoir.protocol >= 3
  ? window.voussoir.pageOrigin
  : window.location.origin;
```

Feature-test once, at the boundary, rather than in every call site.

## Next

- [Security model](security.md) — what the origin is used for.
- [Platform support](platforms.md) — which origins apply where.
