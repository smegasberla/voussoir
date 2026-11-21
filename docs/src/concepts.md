# Concepts

You are about to build a desktop application. Not a website, not a command-line
tool: a window with buttons in it, that a person double-clicks to open.

This chapter builds the mental model you need before writing a line of code. Every
term the rest of the book uses is introduced here, and each section answers three
questions: **what is it, why does it exist, and what goes wrong without it.**

If you have built a website before, you already know the page half and will find
the Rust half new. If you have written Rust before, the reverse. Almost nobody
starts with both, which is why this page exists.

## Theory: the problem every desktop app has to solve

A desktop app needs two things that pull in opposite directions.

**It needs a user interface.** Buttons, text, layout, scrolling, animation, and
font rendering that does not look wrong on a high-DPI screen. Doing that yourself
in a native toolkit means learning a different widget API per operating system:
Win32 or XAML on Windows, Cocoa on macOS, GTK on Linux. Three user interfaces, three
sets of bugs, one application.

**It needs to touch the machine.** Read a file, remember a setting, show a save
dialog, put a badge in the system tray, check for updates. That work has to be in a
real programming language with real error handling and real tests, because it is
where your app can lose a user's data.

Every framework in this space is a different answer to "how do I get both". The
answer Voussoir picks is:

> Write the interface as a web page. Write the machine access as Rust. Connect the
> two with one narrow, checkable channel.

This chapter explains what each half of that sentence costs.

## The shape of a Voussoir app

One executable. Inside it, two halves that run at the same time:

```
  one process on the user's machine
  ┌───────────────────────────────────────────────────────┐
  │  your-app  (a single Rust binary)                     │
  │                                                       │
  │   ┌─────────────────────┐      ┌───────────────────┐  │
  │   │   Rust shell        │      │  OS webview       │  │
  │   │                     │      │                   │  │
  │   │  - opens windows    │ JSON │  your HTML        │  │
  │   │  - serves the page  │◄────►│  your CSS         │  │
  │   │  - your commands    │      │  your TypeScript  │  │
  │   │  - security policy  │      │  window.voussoir  │  │
  │   └─────────────────────┘      └───────────────────┘  │
  └───────────────────────────────────────────────────────┘
```

- The **shell** is a Rust program. It creates the window, embeds your frontend,
  enforces what the page may do, and holds your **commands**: ordinary Rust
  functions with names the page can call.
- The **webview** is the browser engine the operating system already ships. Your
  interface is HTML, CSS and JavaScript running inside it.
- The two talk over **one channel**, described later in this chapter.

That is the entire architecture. There is no second process, no bundled browser, no
Node.js runtime in the shipped app, and no HTTP server listening for the outside
world.

### What a "webview" actually is

A webview is a browser without the browser: the rendering engine and the JavaScript
engine, with no address bar, no tabs, and no navigation controls. The application
owns the window; the engine only draws what the app tells it to draw.

Every operating system ships one, and they are genuinely different engines:

| Operating system | Engine | Who makes it |
| --- | --- | --- |
| Windows | WebView2 | Microsoft, built on Chromium |
| macOS, iOS | WKWebView | Apple, built on WebKit |
| Linux, BSD | WebKitGTK | the WebKit project |
| Android | Android System WebView | Google, Chromium-based, updatable per device |

This table is why "test on each platform you ship" is not bureaucracy. The same
CSS can lay out differently on WebKitGTK and WebView2, because they are different
engines that both follow the same standards imperfectly.

### What a "shell" is, and why that word

The word comes from the idea of a thin outer layer around something bigger. The
shell is the thin Rust layer around the webview: it does not draw your interface,
it holds the window and the authority. When you read "the shell", think *the Rust
side, the part that is allowed to touch the disk*.

## Theory: why the OS webview and not a bundled browser

The obvious alternative is to embed a browser engine in your app, which is what
Electron does: ship Chromium, and your app behaves identically everywhere.

That trade is real, and it is not stupid. It buys rendering that is identical on
every machine, a single engine to learn, and no dependency on what the user's
system happens to have. What it costs:

| | Bundled browser (Electron) | OS webview (Voussoir) |
| --- | --- | --- |
| Download size | Often over 100 MB, mostly engine | A few MB, mostly your code |
| Rendering | Identical everywhere | Differs by OS and version |
| Security patches | You ship them, per release | The OS ships them, continuously |
| Memory | One engine copy per app | Shared with the rest of the system |
| Time to first window | Slower: engine starts with the app | Faster: engine is already there |

Voussoir picks the OS webview for two reasons that matter more than size.

The first is that **the thing you ship is a Rust binary plus your own assets**. A
user downloads an app, not a runtime that contains an app. That affects how people
perceive the download, how updates work, and how much of your release process is
"rebuild a browser".

The second is that **you inherit the platform's security work**. When a
WebKitGTK vulnerability is found, the distribution patches it, and your users get
the fix without you releasing anything. Shipping your own engine means shipping
your own CVE response process.

The cost you accept is the one in the table: there is no such thing as "it looks
the same on Windows and Linux". Treat the webview as a target you test against,
not as a spec you design to.

**What this means for the rest of the book:** because the page is trusted code
with real authority behind it, there is a security policy, and it is on by default.
A framework that ships the user's own UI does not need to defend against the user;
it needs to defend against the user's *content*.

## Theory: why one command channel and not a bridge per feature

The shell and the page must talk. The obvious design gives each feature its own
call: one function for file dialogs, one for the tray, one for windows, one for
storage. The page then has a dozen globals to remember, and each has its own error
convention, its own security story, and its own diagnostics.

Voussoir has exactly one:

```
POST voussoir://localhost/__voussoir/<method>
body:     your payload, as JSON
response: {"ok": <the handler's return value>}
        | {"error": {"message": "...", "kind": "...", "method": "..."}}
```

Every command, yours or built in, is reached this way. The page sees one function:

```ts
const greeting = await window.voussoir.invoke("greet", { name: "Ada" });
```

**Why this is worth the constraint.** When everything is one shape, four things
become true at once:

- **One place enforces security.** The dispatcher checks the allowlist before it
  looks up any handler, so no command can forget to check.
- **One place formats errors.** A page never has to know whether a failure came
  from Rust, from the transport, or from a timeout: the shape is the same.
- **One place measures.** Command latency, refusals, and slow calls are recorded at
  the same chokepoint, so diagnostics cover the whole surface for free.
- **The page is testable without the UI.** A command is a name, a JSON payload, and
  a JSON reply. That can be asserted in a unit test.

The constraint is that a command cannot stream, cannot push a partial result, and
cannot hold a callback. For those, the design uses **events** in the other
direction, which are also just JSON with a name.

## Theory: localhost, the page, and why it is not `file://`

A web page loaded from `file:///home/you/app/index.html` has almost no privileges.
It cannot `fetch` reliably, its origin is "opaque", storage is per-file, and several
security mechanisms behave differently or refuse outright.

So the shell **serves** your frontend itself, over a loopback address, from memory:

```
production (desktop):   voussoir://localhost/...
development:            http://127.0.0.1:5177/...   (the Bun dev server)
```

That is the **origin**: a scheme plus a host, which is what a browser uses to decide
which page is allowed to talk to which. Your app's origin is constant and yours.

One detail that produces a lot of confused bug reports: **on Windows and Android
the webview cannot intercept a custom scheme**, so the shell rewrites
`voussoir://localhost` to `http://voussoir.localhost` and intercepts that instead.
The page is still your app; it is spelled differently. This is why page code reads
`window.voussoir.pageOrigin` rather than hard-coding a base URL.

**The consequence to remember:** a link to `https://example.com` inside your page is
not navigation, it is a refusal. Opening a foreign page in your window would replace
your interface with content you do not control, inside a context that can reach your
commands. If you want to show external content, open the user's browser.

Serving the page over a real address has a second effect worth naming: **the assets
are embedded in the binary**, so there is no `dist/` folder to ship beside the app,
and no "file not found" on a user's machine. The frontend is compiled in.

## The life of one command

This is the single most useful thing to understand, and it takes eight steps.
Follow the greeting from the page to Rust and back:

```
1. your page calls   window.voussoir.invoke("greet", { name: "Ada" })
2. the shim           serializes the payload, picks a transport
3. the transport      POST /__voussoir/greet  (or window.ipc on Android)
4. the path decoder   extracts "greet", decodes %XX escapes
5. the dispatcher     asks: may this window call "greet"?   ── no ──► {"error":{"kind":"unknown"}}
6. the registry       finds the handler registered under that name ── none ──► same envelope
7. the handler        runs your Rust function, on the UI thread or a worker
8. the envelope       wraps the result: {"ok": ...} or {"error": {...}}
                      the shim resolves (or rejects) your promise
```

Four things follow from that diagram, and each one explains a class of bug you will
otherwise hit blind:

- **Step 5 comes before step 6.** A command that is denied and a command that does
  not exist produce the *same* answer, `kind: "unknown"`, on purpose. If a page could
  tell them apart, it could enumerate the surface it is not allowed to touch. The
  cost is that during development a fence typo looks exactly like a missing handler.
- **The reply is always an envelope, even on failure.** A handler that returns
  `Err(String)` still produces a successful HTTP response carrying `{"error": ...}`.
  The page's promise rejects; the transport did not fail.
- **A panic does not take down the window.** Handlers run wrapped, so a panic becomes
  `kind: "panic"` and the app keeps painting.
- **The thread in step 7 decides everything about responsiveness.** More on that
  below.

## Threads: the thing that makes an app feel broken

Every program has at least one thread: a sequence of instructions the machine runs.
A GUI program has a special one, variously called the **UI thread**, the main thread,
or the event loop thread. Its rule is absolute:

> The UI thread must return to its loop quickly, or the window stops updating.

"Stops updating" looks to a user like a freeze: no repaint, no cursor change, a
spinning cursor over the window, and eventually the operating system offering to
force-quit your app.

Voussoir gives you three places to run a command, and the choice is the whole
performance story of your app:

| Where | Which registration | Good for | Danger |
| --- | --- | --- | --- |
| UI thread | `command`, `command_typed` | microseconds of work | anything slow freezes the window |
| A worker thread | `async_command` | reading files, network, hashing | one thread per call, so a flood costs memory |
| Shared runtime | `async_fn` (needs `rt`) | awaiting many things at once | an extra dependency and a scheduler |

The worker path has a deliberate bound: at most `available_parallelism()` workers,
never fewer than four. Past the bound, requests are **refused** with `kind: "busy"`
rather than queued. A queue would hold the page's promise open and still cost a
thread later, so the honest answer is "not now, ask again", and a `busy` is
always safe to retry because the handler never started.

The `rt` path exists because a thousand simultaneous awaited requests should cost a
thousand cheap tasks, not a thousand stacks.

## Theory: what "state" means here, and why there are three kinds

Newcomers lose more time to this than to any API. An app has three different kinds
of memory, with three different lifetimes, and using the wrong one produces bugs
that look like magic.

| Kind | Lives | Example | Chapter |
| --- | --- | --- | --- |
| A Rust variable | until the function returns | a decoded payload | this one |
| Process state | until the app exits | which windows are open | [In-process state](state.md) |
| Stored state | until the user deletes it | the user's settings | [Storage](storage.md) |

The distinction that matters: **a hot reload replaces your command code but not the
process**, so anything the *page* holds in a variable survives a Rust edit but not a
page reload. And the reverse, which is the part that surprises people: a `static`
in the commands crate **does not** survive a reload, because the library that owned
it is replaced. A counter kept in a `static` resets every time you save, while the
same code is correct in release, where the crate is linked once. Put anything that
must outlive an edit in [process state](state.md).

Configuration is a fourth thing, and it is not state at all: `voussoir.json` is
compiled into the binary, so it changes only when you rebuild.

## Theory: Cargo features, and why the crate has them

A Rust **feature** is a compile-time switch. `voussoir = { features = ["full"] }`
means "compile the parts of this crate that the `full` feature names". Code behind a
feature you did not enable is not in your binary at all.

Voussoir uses features because its surface pulls in heavy platform libraries, and an
app should not pay for what it does not use:

| Feature | Adds | Why it is separate |
| --- | --- | --- |
| `full` | menu, dialogs, updater, devtools, storage | most apps want these |
| `storage` | the `storage.*` commands | a minimal shell may want no filesystem story |
| `rt` | a shared async runtime, so `async_fn` exists | it pulls in a task scheduler |
| `tray` | tray icon support | it implies a background process model |
| `devtools` | the ability to open devtools | shipping it is a debugging hazard |
| `hot-reload` | loading the commands library at runtime | dev only, never in a release build |

Two consequences to internalise. First, **a feature you did not enable makes its
commands not exist**: calling `storage.get` without `storage` answers `unknown`, not
"disabled". Second, **`tray` and `rt` are not in `full`**, even though `full` includes
`storage`. A tray icon changes what closing the last window means, and a runtime is a
scheduler your app may never use; both are too large a decision to be implied by a
convenience flag.

## What Voussoir is not

Stated once, so you can stop looking:

- **Not a browser.** Your page cannot navigate to websites, open tabs, or use an
  address bar. That is policy, not a missing feature.
- **Not a web server.** The loopback address is answered from memory inside your own
  process. Nothing listens on the network.
- **Not a Node.js app.** There is no `fs`, no `path`, no `require` in the page. File
  access happens in Rust, deliberately, so it is visible and testable.
- **Not a mobile-first framework.** Desktop is the primary target; Android and iOS
  work and have documented limits, including no hot reload.
- **Not a way to reuse a React app unchanged.** You can use a framework, but there is
  no router and no URL, so an app built around routing needs rethinking.

## Vocabulary

Every term, defined once. The chapter that explains it in depth is linked.

| Term | Means |
| --- | --- |
| **shell** | the Rust binary: window, policy, and your commands |
| **webview** | the OS browser engine your page runs in |
| **shim** | the small JavaScript layer defining `window.voussoir`; injected, not installed |
| **command** | a Rust function the page reaches by name |
| **registry** | the table mapping command names to handlers |
| **dispatcher** | the code that checks the allowlist, then calls the handler |
| **envelope** | the `{ok}` / `{error}` JSON shape, in both directions |
| **origin** | scheme plus host the page is served from |
| **entry page** | the HTML file loaded first, `index.html` by default |
| **asset table** | your frontend compiled into the binary as byte arrays |
| **allowlist** | the set of command names a page may call |
| **label** | a window's identifier, used to address it |
| **UI thread** | the one thread that may touch the window; must stay fast |
| **worker thread** | a thread that runs one blocking command |
| **hot reload** | swapping the commands library into a running dev shell |
| **cdylib** | a Rust library compiled to be loaded at runtime, `.so`/`.dylib`/`.dll` |
| **feature** | a compile-time switch that includes or excludes code |
| **tray** | the icon in the system's notification area |
| **devtools** | the webview's inspector, for debugging your page |

## Where to go next

- Never built either half? [Your first app](first-app.md) goes from an empty
  directory to a window that talks to Rust.
- Want the contract in detail? [Commands & payloads](ipc-contract.md).
- Deciding whether this suits your project? [Platform support](platforms.md) and
  [Compatibility policy](compatibility.md) are the honest limits.
