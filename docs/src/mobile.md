# Android and iOS

Both mobile platforms reuse the command contract, registry, allowlist and security
policy. What differs is the transport and who owns the event loop, and therefore
which commands apply.

## Theory: why mobile is a separate build, not a separate API

A desktop app owns its event loop: it calls `run` and blocks. A mobile app does
not. The OS owns a UI thread, hands you lifecycle callbacks, and can suspend or
kill the process whenever it likes. No amount of abstraction makes those two the
same program.

So the choice is between two bad options:

1. **Hide the difference** behind a portable API. You get something that works
   adequately on neither platform, because the lifecycle it hides is the thing
   mobile apps are mostly about.
2. **Keep the same app-facing API** (commands, allowlist, envelope, CSP) and let
   the *implementation* differ per platform.

Voussoir takes (2). A command does not know which platform it is on; the code that
delivers it does.

| Concern | Desktop | Android | iOS |
| --- | --- | --- | --- |
| Event loop owner | your `main` | the `Activity` | `UIApplicationMain` |
| IPC transport | custom protocol | `postMessage` bridge | custom scheme |
| Envelope / errors | same | same | same |
| Allowlist | same | same | same |
| CSP / navigation policy | same | same | same |
| Command table | static, or hot in dev | static | static |
| Hot reload | yes | no | no |

## Android

```bash
voussoir android build            # APK for arm64-v8a
voussoir android build --aab      # signed App Bundle for Play
voussoir android build --abi x86_64 --api 30
voussoir android build --release  # signed release APK
voussoir android run              # install and launch on a device or emulator
voussoir android run --wait       # same, then keep streaming device logs
```

`voussoir init` scaffolds `android/` as a Gradle project, so nothing is generated at
build time except the Kotlin that `wry` writes, which is git-ignored.

The platform is different enough that it lives in its own crate, `voussoir-android`:

- **The activity owns the event loop.** Desktop creates one and runs it; Android is
  handed one from `onCreate` and drives it from the activity's callbacks. There is
  no `main` loop to block on.
- **`invoke` cannot use the custom protocol.** `WebResourceRequest` carries no
  body, so `wry` would deliver every payload as `null`. Android uses `wry`'s
  `postMessage` handler and resolves the page's promise from Rust.
- **Tray is a notification.** It is posted once and cannot be retitled; a runtime
  update reports that rather than pretending it worked.

**Why `postMessage` and not an interception fix?** Because the interception API
receives a request without its body. There is nothing to read. The bridge is what
the platform provides, so it is what the transport uses.

### The transport, in order

```
page:    voussoir.invoke("greet", {name: "Ada"})
         -> postMessage to the app's JS interface
Rust:    parses the envelope, dispatches, gets {"ok": ...}
         -> evaluates a JS call resolving the pending promise by id
page:    promise resolves
```

The id is assigned on the page side and echoed back, because the bridge has no
request/response pairing of its own. That is the whole difference from desktop: the
envelope is identical, the delivery is not.

### Testing

The instrumentation suite runs on an x86_64 emulator in CI. It then launches the
app with no instrumentation attached and fails on a `FATAL EXCEPTION` or an
`UnsatisfiedLinkError`, because an instrumentation pass can hide a load-order bug
that only appears without a debugger.

arm64 is compiled (it is the default ABI for the APK) but not run in CI: there is
no arm64 emulator in the matrix, and saying "tested on both ABIs" would be false.

## iOS

```bash
voussoir ios build                # simulator app
voussoir ios build --target device
voussoir ios run                  # install and launch on a booted simulator
voussoir ios verify               # build, install, launch, assert the Rust entry is reached
```

iOS keeps the custom-scheme transport: `WKURLSchemeHandler` intercepts
`voussoir://localhost` exactly as the desktop does, so the origin and the security
policy are the desktop ones.

**Why iOS can and Android cannot:** `WKURLSchemeHandler` hands you the whole
request; `WebResourceRequest` does not. So the one mobile platform where the
`http://` workaround is unnecessary keeps the desktop's stronger origin.

Two differences matter in practice:

- **The first window is requested through the loop.** `tao` cannot build a window
  before `UIApplicationMain` runs, so the shell asks for the first window through
  the same path a command uses and gets it once the loop is up. App code and the
  `on_ready` contract are identical.
- **Some features report unsupported, and one does not.** `dialog.*` returns an
  error on iOS ("not available on iOS yet"), the tray setters refuse because
  `TRAY_IS_LIVE` is false, and `update.check`'s fetch fails with a message saying
  the App Store owns updates. **Menus are the exception:** `menu::build` returns a
  handle that just stores the groups, and `attach` is a no-op, so declaring a menu
  on iOS succeeds and renders nothing. Branch on `voussoir.platform` for the
  affordance rather than waiting for an error that will not come.

## Theory: why hot reload does not exist on mobile

Loading a new dynamic library into a running app is exactly what both platforms
restrict, and for good reason: it is how malicious code stays resident. Android
forbids loading an unsigned library from a writable directory; iOS forbids it
outright outside a development configuration.

So `voussoir dev` is a desktop command, and a mobile iteration is a full
install-and-launch. That is slower and there is no way around it, which is why the
command table is still a separate crate on mobile: the boundary is the same even
when the hot path is not.

## Theory: what "shared policy" buys you

An attacker reading this page learns something useful: the mobile transports
differ, so if there were a bug it would be in the transport. The defence is that
the *policy* does not differ. The allowlist is checked where the envelope is
parsed, which is after the transport and the same code on every platform.

That is the layering worth remembering: transport is per platform, policy is per
app, authority is per command.

## A first mobile build, in order

1. Install the toolchain: Android SDK + NDK (`ANDROID_NDK_HOME`), or Xcode.
2. `voussoir doctor` on the target machine; it names what is missing.
3. `voussoir init --mobile myapp` for an Android-only shell.
4. `voussoir android run`, or `voussoir ios run` with a booted simulator.
5. `voussoir ios verify` on iOS: it asserts the Rust entry was reached, which
   catches a miswired shell before you debug a blank screen.

Step 5 is worth the comment: a blank iOS window is almost always a wiring problem,
and the verify command distinguishes that from a rendering problem in one run.

## Next

- [Platform support](platforms.md) — the tested matrix, per target.
- [The two origins](origins.md) — why Android's origin differs and iOS's does not.
