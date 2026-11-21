# The frontend

The page half of a Voussoir app is ordinary HTML, CSS and TypeScript, bundled by
Bun and rendered by the operating system's webview. There is no framework
requirement and no second build system.

This chapter explains what Bun does with your files, what the shell injects into
every page, and which habits to drop because your "browser" has no address bar.

## Theory: why the page is a page and not a widget toolkit

You could build the interface with native widgets: a real GTK or Cocoa button for
each button in your design. You would get perfect platform integration and perfect
fonts, and you would write every layout three times.

Using HTML inverts that. One layout renders on all platforms, CSS gives you the
flexibility people have spent twenty years building tooling for, and the webview
handles text rendering, scrolling and accessibility for you.

What it costs, stated plainly:

| You get | You give up |
| --- | --- |
| one layout for every OS | pixel-identical rendering across engines |
| every CSS technique on the web | the "real native widget" feel some users expect |
| hot reload in development | a browser with navigation, tabs, or cookies |
| TypeScript for the UI | any Node.js API in the page |

The last row of the right column is the one that surprises people, and it is
covered later in this chapter.

## How Bun builds your files

`voussoir dev` and `voussoir build` both run the same underlying command:

```
bun build index.html --outdir dist --target browser
```

Release builds add `--minify`. That is the entire frontend pipeline.

What that one line actually does, step by step, because "the bundler handles it"
is not an explanation:

1. **Bun reads the entry HTML.** That is `voussoir.json`'s `entry`, `index.html`
   by default. HTML is the entry, not a script.
2. **It follows references.** Every `<script src>` and `<link rel="stylesheet">`
   in that HTML is read, and then every `import` inside those files, recursively.
3. **It resolves and transpiles.** TypeScript types and JSX are stripped; `.css`
   files imported from a script become real stylesheets in the output.
4. **It writes `dist/`.** Plain JavaScript, CSS and copied assets, with import
   paths rewritten so the browser can follow them.
5. **The same CLI call turns `dist/` into `native/src/assets.rs`**, a Rust table of
   `(path, bytes)` pairs. That file is what gets compiled into the binary.

Step 5 is the one with no equivalent in web development: **`dist/` is not shipped.**
It exists for a moment so the asset table can be generated from it. You never copy
it anywhere, and deleting it changes nothing.

### What this means in practice

- **TypeScript needs no configuration to build.** Bun strips types; it does not
  check them. The scaffold writes a `tsconfig.json` for your *editor*, not for the
  build. A build can succeed while `bun tsc --noEmit` reports errors.
- **Dependencies are bundled in.** A library you import ends up inside the output.
  There is no `node_modules` at runtime and no module loader resolving packages
  from disk.
- **Node built-ins do not exist in the page.** `import fs from "node:fs"` may
  bundle, but calling it fails in the webview. File access is a Rust command, by
  design: it is visible, testable, and subject to the allowlist.
- **The output is served from memory.** The shell answers requests for your origin
  out of the embedded table, so there is no server process and no filesystem read.

## Theory: what the shim is, and why it is injected

Before any of your code runs, the shell injects a small JavaScript file into every
page and every frame. It defines exactly one global:

```ts
window.voussoir = {
  invoke,      // call a Rust command
  listen,      // subscribe to an event Rust emits
  windows,     // the open window labels
  tray,        // { tooltip, checked, popup }
  menu,        // { checked }
  platform,    // "linux" | "windows" | "macos" | "bsd" | "android" | "ios"
  pageOrigin,  // the origin this page is really served from
  timeout,     // the default command deadline, in ms
  protocol,    // the shim's own version, for feature detection
};
```

It is not a module. Nothing imports it, nothing installs it, and it is not a DOM
element. It exists on `window` before your first statement, which is why
`voussoir.d.ts` can declare it and TypeScript can check your usage without any
import.

**Why a shim, rather than exposing the transport directly?** There are two
transports, and they are genuinely different:

- **Desktop** uses a custom protocol: `invoke` is an HTTP POST to
  `voussoir://localhost/__voussoir/<method>`, answered directly.
- **Android** cannot receive a request body through that mechanism, so calls go over
  `window.ipc` to a native handler and the answer comes back on a later event-loop
  turn.

The shim is the only code that knows which one it is on. Page code calls `invoke`
and never learns the platform.

**A detail that matters for security:** the shim checks `originAllowed()` first. If
the page is not on an allowed origin, it deletes `window.voussoir` and refuses. A
page that somehow got loaded from a foreign origin has no bridge at all, rather
than a bridge that rejects each call.

## `invoke`: the one call

```ts
const value = await window.voussoir.invoke<T>("method", payload, options?);
```

- **Resolves** to whatever the Rust handler returned inside `Ok`.
- **Rejects** with an `Error` carrying `kind`, `method` and `message`. `kind` is
  the part you branch on; every value is listed in
  [Errors the page can branch on](errors.md).
- **`options.timeout`** overrides the deadline for this one call, in milliseconds.
  `0` means no deadline.

Three behaviours worth knowing before they surprise you:

**The payload is serialized with `JSON.stringify`.** A `Map`, a `Date`, a `Set` or a
class instance does not survive the trip and arrives as `{}` or a string. Send plain
objects, arrays, numbers, strings, booleans and `null`.

**`invoke<T>` is an assertion, not a check.** TypeScript cannot see across the
boundary. `invoke<string>` tells your editor what to expect; if Rust returns a
number, you get a number at runtime and a lie in your types. Keeping payload types
in sync with the Rust struct is your job, which is why
[Typed commands](typed-commands.md) exists.

**A timeout rejects the promise but does not cancel the handler.** The work may
still be running and may still write. Do not retry a `timeout` blindly.

### Why a promise, and not a callback

Every command is asynchronous from the page's point of view, even a synchronous
Rust function: the request has to cross into the shell. A callback would push error
handling into a second argument everyone forgets, so the API returns a promise and
`try`/`catch` works the way a JavaScript reader already expects.

## `listen`: the other direction

```ts
const off = window.voussoir.listen<{ dark: boolean }>("theme", (payload) => {
  document.documentElement.dataset.theme = payload.dark ? "dark" : "light";
});

// later, when the component goes away
off();
```

`listen` returns its own unsubscribe function. Rust reaches it with
`app.emit("theme", ...)` (every window) or `app.emit_to(Some("main"), ...)` (one).

**Event names are plain strings and nothing validates them.** A typo on either side
produces a listener that never fires, with no error anywhere. Two habits prevent
that: define names as constants in one module, and add a test that asserts a
listener fires for the name Rust emits.

**And an event can arrive before you are listening.** Delivery works by evaluating
a script in the webview, with a guard that does nothing if the shim is not there
yet. There is no queue. That is why initial state is emitted from `on_ready`, which
fires after your page has loaded, and not from `main`. See
[Lifecycle hooks](lifecycle.md).

## Platform detection, and what to do with it

```ts
if (window.voussoir.platform === "android") {
  // the menu bar does not exist here
}
```

Use this for **affordances**, never for behaviour. A command behaves the same on
every platform. Where the UI differs:

| Capability | Windows | macOS | Linux | Android | iOS |
| --- | --- | --- | --- | --- | --- |
| menu bar | yes | yes | yes | no, raised as a `PopupMenu` | no |
| tray icon | yes | yes | yes | posted notification, not editable | no |
| multiple windows | yes | yes | yes | no | no |
| devtools | yes | yes | yes | no | no |

The Android menu is the row people get wrong in both directions. There is no menu
*bar*, because the platform expects app chrome (an overflow button or a drawer)
rather than OS chrome. But the same menu groups exist and are raised on demand:

```ts
// Android only: show the app's menu at the current position.
await window.voussoir.invoke("__voussoir_show_menu");
```

So the branch is about *where the affordance lives*, not whether it exists. Hide a
button that cannot work; do not hide a capability that is merely spelled
differently.

## Feature detection with `protocol`

```ts
if (window.voussoir.protocol >= 2) {
  const open = await window.voussoir.windows();
}
if (window.voussoir.protocol >= 3) {
  const base = window.voussoir.pageOrigin;
}
```

`protocol` is the shim's version, not the crate's, so it changes only when the
page-visible API does. Version 2 added `windows()`, `tray` and `menu`; version 3
added `pageOrigin`. The current version is **3**.

**Why detect a version rather than check for the member?** They answer different
questions. `if (voussoir.tray)` asks "is this one thing here", which is right for a
single optional member. The version asks "what shape is this object", which is right
when you use several members together, because one guard covers all of them.

## Theory: the page cannot navigate, and that is a feature

In a normal web app, setting `window.location` leaves your code and loads someone
else's. In a Voussoir app that navigation is **refused by the shell** and recorded
as a `[security]` entry, because there is no back button and no address bar: the
user would be stranded in a page that is not your app.

The reason it is a refusal rather than a nuisance is authority. Your page can call
your commands. If it could navigate to a remote page, that remote page would inherit
a bridge into your app's filesystem access. Refusing navigation is what keeps the
authority attached to code you wrote.

Practical consequences:

| You want to… | Do this |
| --- | --- |
| show a web page | `<a target="_blank">`, which the shell hands to the OS browser |
| link to your own docs page | `target="_blank"` as well, or a second window |
| change views | render a different view in the same document |
| load another bundled page | a relative link; it is the same origin |

## Theory: routing without a router

Because navigation is refused, a client-side router must never assign to
`location`. Most router libraries assume a URL; yours has no URL. Two patterns that
work:

**In-memory views**, the common choice:

```ts
type View = "list" | "detail";
let view: View = "list";

function render() {
  root.innerHTML = view === "list" ? listHtml() : detailHtml();
}
```

A tiny store plus a `render()` function is enough for most apps, and it has no
dependency, no history, and nothing to break when a user presses a key you did not
anticipate.

**One page per window**, when views are truly separate documents:

```ts
await window.voussoir.invoke("open_settings");
```

A second window has its own document, its own memory, and its own allowlist. That
is the closest thing to a route, and it is a stronger boundary than a route: a
window can be denied commands the main window has. See
[Multi-window](multi-window.md).

## Theory: pull or push, and how to choose

Rust can push (events) and the page can pull (commands). The right answer is decided
by **who finds out first**:

- **Pull** when the page decides it needs something: "what is the current document?"
  Retry and error handling stay in one place, and nothing happens while the user is
  not looking.
- **Push** when Rust knows first: a file changed on disk, a timer fired, a menu item
  was clicked. The page cannot poll for a fact it does not know exists.

The mistake to avoid is seeding the page from `main`. That is a push before anyone
is listening, and it is silently dropped. Seed from `on_ready`:

```rust
App::new().on_ready(|app, label| {
    app.emit_to(Some(label), "boot", json!({ "theme": "dark" }));
})
```

## Types across the boundary

`invoke<T>` and `listen<T>` are notes to your editor. Rust cannot enforce them,
because JSON does not carry types. Two habits keep the two sides honest:

1. **Define the payload shape next to the Rust struct it mirrors**, and change both
   in one commit. A comment naming the Rust type is worth more than a clever
   generic.
2. **Prefer a typed Rust command** (`command_typed`) so a wrong shape fails on the
   Rust side with `kind: "payload"`, naming the command and the field, instead of
   reaching your handler as a missing value.

`voussoir init` writes `src/voussoir.d.ts`, which declares `window.voussoir` for the
editor. It is a convenience: if it drifts, the runtime is unaffected and your types
are merely wrong, which is still worth avoiding.

## Assets and URLs

- **Reference assets relatively** (`./logo.png`). Absolute URLs are correct on four
  platforms and wrong on Windows, where the origin is rewritten.
- **`data:` and `blob:` images are allowed** by the release policy, so inlining a
  small icon is fine.
- **Build absolute URLs from `pageOrigin`** when you truly need one, never from a
  literal.

## What the page deliberately cannot do

| Missing | Why | What to do instead |
| --- | --- | --- |
| `fs`, `path`, `process` | there is no Node runtime | a Rust command |
| remote `<script src>` | the release CSP allows `script-src 'self'` | bundle the dependency |
| `eval`, `new Function` | the release CSP has no `unsafe-eval` | ship the code |
| `fetch` to another host | `connect-src 'self' voussoir:` | a Rust command performs the request |
| remote cookies | there is no remote origin | the `storage.*` commands |
| `<form action="...">` posting out | `form-action 'none'` | handle `submit` and call a command |
| being framed by another page | `frame-ancestors 'none'` | nothing; this is a defence |

Every row is the same trade: the page has less reach, and Rust holds the authority.
If you find yourself fighting this list, the feature belongs in Rust.

**One exception worth knowing:** in a dev build the CSP is relaxed, because Bun's
hot-reload client uses inline scripts and an eval-based module loader. Code can
therefore work under `voussoir dev` and fail in a release build. If something breaks
only after `voussoir build`, check this table first.

## What it does not do

- **No server-side rendering, no hydration.** The page is static files plus your
  JavaScript; there is no Node process at build or runtime to render HTML.
- **No code splitting you can rely on.** Everything is embedded, so lazy `import()`
  saves no download; it only defers execution.
- **No service worker.** There is no network origin to intercept and nothing to
  cache; the assets are already local.
- **No `postMessage` contract of your own.** Use commands and events, which have
  error handling and are visible to diagnostics.

## Next

- [Commands & payloads](ipc-contract.md) — the contract behind `invoke`.
- [Errors the page can branch on](errors.md) — every rejection you can receive.
- [The two origins](origins.md) — why the origin string differs on Windows.
