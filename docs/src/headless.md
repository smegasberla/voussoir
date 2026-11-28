# Headless tooling

`voussoir run --snap`, and what it is honestly allowed to promise.

This chapter exists because an agent that cannot see a window cannot check its own
work. The loop it closes is short: boot the built app, wait for the first page to
finish loading, write a PNG and a JSON description, exit.

## Theory: there is no display-free path to pixels

The engine — WebKit, WebView2, WKWebView — draws into the widget it was given. It
has no offscreen mode that produces a frame, because a webview that is not on
screen has nothing to paint into.

This was measured rather than assumed, on `webkit2gtk` 2.0.2, with three attempts:

| Attempt | Result |
| --- | --- |
| `gtk::init` with no `DISPLAY` | fails; there is no display-free GTK |
| window built but never shown, then `snapshot()` | **fails**; the webview is 0×0 and there is nothing to capture |
| window shown under a live display, then `snapshot()` | works |

So "headless" here means **no window manager and no compositor**, not "no X
server". `xvfb-run` supplies exactly that, and it is what CI uses.

A framework that renders with its own GPU pipeline can capture without a window,
because it is drawing into a texture it chose. A framework that hands the drawing
to the OS cannot, and a product that claims otherwise is either leaning on a
virtual display it has not mentioned or shipping a fake.

## Using it

```bash
voussoir build
voussoir run --snap shot.png --dump shot.json
```

On a box with no display:

```bash
xvfb-run -a voussoir run --snap shot.png
```

`--dump` alone works everywhere, and answers most questions on its own.

## What the dump contains, and why each field is there

```json
{
  "window": { "label": "index.html", "title": "Hello Voussoir",
              "width": 933, "height": 487, "scale": 1.0 },
  "voussoir": { "platform": "linux", "pageOrigin": "voussoir://localhost",
                "protocol": 4, "version": "0.1.0", "bridge": true,
                "reported": { "protocol": 4, "platform": "linux",
                              "pageOrigin": "voussoir://localhost" } },
  "error": null
}
```

- **`window.width` / `height`** are the measured inner size, not what the config
  asked for. They differ once a window manager applies its own constraints, and
  the difference is what makes a layout bug reproducible.
- **`window.scale`** is the device scale factor, which is why a 933-pixel-wide
  window is 466 logical points on a 2× display.
- **`voussoir.bridge`** comes from inside the page. It is `false` when the shim
  refused the origin or never installed, which looks exactly like a working shell
  from Rust.
- **`voussoir.reported`** is the page's own answer for the same fields. If it
  disagrees with the top level, something is wrong with the origin, not the page.
- **`error`** is non-null when the probe itself threw, which means the page ran but
  something in it is broken.

## Theory: why this is not an element tree

A framework that owns its widget tree can dump it: every node, its role, its
layout rect. That is a genuinely useful artifact, and it is not available here —
the tree belongs to the DOM, inside the webview.

Inventing one in Rust would be a second source of truth beside the DOM, and the two
would disagree the first time a page used `display: none` anywhere interesting. So
the dump reports what the page cannot say about itself — the platform, the real
origin, the shim protocol — and leaves the rest to the page, which is reachable
from the same script but not honestly summarisable from here.

If you need a DOM dump, ask the page for one:

```ts
// In your own command, or from the snapshot page itself.
document.documentElement.outerHTML;
```

## What the capture proves, and what it does not

It proves the page rendered, at the right size, on the right platform, with the
bridge installed. It catches the whole class of bug where the window is up and the
content is wrong: a blank page, an unstyled page, a page whose script threw before
it painted.

It does not prove:

- **that a command works.** The capture happens at first paint, which is before a
  user has clicked anything. Command round-trips are `scripts/verify.ts`'s job.
- **that the layout is correct.** A PNG is for a human or a visual diff, and no
  test in this repository diffs one yet.
- **that the accessibility tree is right.** That needs a tree, not pixels.

## The one line, and why it exists

```
VOUSSOIR_SNAPSHOT window=index.html png=/tmp/shot.png dump=/tmp/shot.json bridge=true
```

A driver waits for a line rather than sleeping for a second and hoping. The shell
exits itself once the files are written, so the line and the exit race — which is
why `voussoir run` reads stdout line by line instead of to completion.

## Platforms

| Platform | `--dump` | `--snap` |
| --- | --- | --- |
| Linux, BSD | yes | yes, through `webkit2gtk`'s `snapshot` |
| macOS, iOS | yes | not yet: needs `WKWebView.takeSnapshotWithConfiguration` |
| Windows | yes | not yet: needs `ICoreWebView2CapturePreview` |
| Android | yes | not yet: the activity owns the surface |

A requested capture on a platform without one exits non-zero and says so. It does
not write a black PNG that looks like a successful capture — that failure mode is
worse than a missing file, because it is invisible.

## Next

- [Accessibility](accessibility.md) — the other half of "is the page right".
- [Testing](testing.md) — where the snapshot check lives in CI.
- [Diagnostics](diagnostics.md) — the recorded view of what ran.
