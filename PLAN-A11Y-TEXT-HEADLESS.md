# Plan and status: accessibility, text, and headless tooling

Three axes, drawn from what Rune has that we do not. Each section states what the
platform actually permits, because on two of the three the constraint is the
webview and not the shell, and pretending otherwise produces a feature that
matches Rune's API and not its behaviour.

This file is a record, not a chapter. `voussoir docs --check` validates
`docs/src/SUMMARY.md` against the chapters it lists, and this is absent from both;
it is tracked so the reasoning that shaped the code is reviewable next to it. What
shipped is documented in `docs/src/accessibility.md` and `docs/src/headless.md`.

**Not on this list:** reimplementing text rendering. Parley, swash and a glyph
atlas are Rune's answer to a problem a webview solves for us. Matching that would
mean deleting the reason this project exists.

---

## 1. Accessibility — shipped

### The honest baseline

The webview renders the accessibility tree, not us. On every platform the shell
hands an `<a>`, a `<button>` and a `<div role="switch">` to the engine and the
engine exposes them to the OS screen reader. This is strictly **more** than Rune
gets for free, and it is only as good as the markup.

So the work was never "implement a11y". It was: **make the good state the easy
one, and make the bad state loud.**

Measured constraints (checked against the crates, not assumed):

| Platform | How the engine tree reaches the OS |
| --- | --- |
| Linux GTK | `webkit2gtk` exposes ATK through the GTK widget |
| macOS WKWebView | `WKWebView` carries `NSAccessibility` conformance (`wry-0.57.4/src/wkwebview/ios/WKWebView.rs`) |
| Windows WebView2 | UI Automation is built in |
| Android | WebView → `AccessibilityNodeInfo` |

### Done

- **`voussoir.a11y.announce(message)`** — a live region the shell owns, hidden with
  the clip technique rather than `display: none` (a hidden element has no
  accessibility node). Clears before it sets, so the same message twice in a row is
  still spoken. Shim protocol 4.
- **`a11y.prefers()` and `a11y.onPrefersChange(fn)`** — reduced motion, dark mode
  and contrast as a value, for the case CSS cannot reach: a page drawing to a
  `<canvas>`, or an animation driven from script.
- **`voussoir doctor` reports markup defects** — a control with no accessible name,
  a skipped heading level, no `<main>` landmark, with a line number. A note rather
  than a failure. 16 tests in `cli/src/a11y.test.ts`, including one that asserts
  the shipped template is clean.
- **The scaffold is the baseline** — `<main>`, `<label for>`, `role="status"` +
  `aria-live` on the region a command answers into, `prefers-reduced-motion` and
  `forced-colors` in the stylesheet.

### Not done, deliberately

- A Rust-side accessibility tree. It would be a second source of truth beside the
  DOM and they would drift.
- An ARIA helper component set. `<button>` beats `role="button"` on a `<div>`.
- Contrast auditing. It needs rendered pixels per theme; `--snap` makes that
  possible and the check is not written.

---

## 2. Text — partially shipped

### The honest baseline

Text rendering is the engine's. Parley and a glyph atlas are how you get there
without one; we have one, so our ceiling is the platform's — and the platform's is
higher. What we lacked was **control**.

### Done

- **`WindowEvent::ThemeChanged` is handled.** The loop ignored it, so a page that
  wanted to follow the OS appearance had to poll `matchMedia` — a timer doing what
  an event should do. It now arrives as a `theme` event with
  `{ "theme": "dark" | "light" }`.
- **Scaffold typography** — `text-wrap: balance` on the heading, `pretty` on the
  paragraph, `font-variant-numeric: tabular-nums` where numbers change. Three
  properties that stop text looking accidental.

### Not done

- **Shell-driven zoom** (`Cmd/Ctrl +/-/0`). `webkit2gtk`'s `set_zoom_level` and
  `Settings::set_default_font_size` are confirmed available in the bindings wry
  already links, so this is mechanical rather than exploratory. It is the most
  platform-specific item on either list, which is why it stayed last.
- **A minimum font size floor** (`set_minimum_font_size`). Same mechanism, same
  reason for waiting.
- **A text-scale setting in `voussoir.json`.** Would be applied through the engine
  so it survives a page that sets `font-size` on `:root`; the deliverable is the
  same as zoom and should land with it.

---

## 3. Headless tooling — shipped for the platform that permits it

### What was measured

A throwaway binary against `webkit2gtk` 2.0.2 on this machine:

| Attempt | Result |
| --- | --- |
| `gtk::init` with no `DISPLAY` | fails — there is no display-free GTK |
| window built but not shown, then `snapshot()` | **fails** — a zero-size webview has nothing to capture |
| window shown under a live display, then `snapshot()` | **works** |

So "headless" here means *no window manager and no compositor*, not *no X server*.
`xvfb-run` supplies that, and it is what CI uses. Documented rather than hidden.

### Done

- **`VOUSSOIR_SNAPSHOT=<path>`** — a PNG of the first window, from the first page
  load that finishes, on Linux and the BSDs. The capture runs in the loop rather
  than in the page-load callback, because `snapshot` completes on the GTK main
  context and blocking there would deadlock against the loop that has to deliver it.
- **`VOUSSOIR_DUMP=<path>`** — a JSON description: measured window size, device
  scale, platform, real page origin, shim protocol, and `bridge` **read from inside
  the page**. An origin the shim refused looks exactly like a healthy shell from
  Rust, and only the page can tell the difference.
- **`voussoir run --snap` / `--dump`** — the wrapper. Boots the built binary, waits
  for the printed line, reports the result, exits. Does not build, on purpose.
- **`verify.ts` asserts it** — with a display it checks the PNG is real; without
  one it skips the capture and keeps the dump check.

### Not done

- **macOS, iOS, Windows captures.** Each needs a different region-capture API
  (`WKWebView.takeSnapshotWithConfiguration`, `ICoreWebView2CapturePreview`,
  `View::draw` into a bitmap). Requesting a PNG there exits non-zero and says so,
  rather than writing a black frame that looks like success.
- **Visual diffing.** A PNG is currently for a human. A per-component baseline
  would be the next thing, and it is a much larger commitment than a capture.

---

## What "on par with Rune" ended up meaning

| Axis | Rune | We reached |
| --- | --- | --- |
| Headless | PNG + element tree, no display | PNG + window/page dump, **needs a display** — the honest version of the same loop |
| Accessibility | own AccessKit tree, per-platform adapters | the engine's tree, correct by default, plus announcement and reader settings; markup audited by `doctor` |
| Text | own parley/swash stack | the engine's stack, with theme propagation and typography defaults; zoom and a size floor still open |

Parity on the third was achievable and is shipped, with the constraint named. Parity
on the first two was not, and where the gap closed it was from the other side: a
webview already gives an accessibility tree and a text engine, so the work was
making them correct by default rather than building them.
