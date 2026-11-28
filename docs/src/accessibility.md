# Accessibility

An app is accessible to the degree its markup is, and no further. The webview
builds the operating system's accessibility tree from your elements and hands it
to Narrator, VoiceOver, Orca or TalkBack; the shell does not draw a tree of its
own and cannot fix one that is wrong.

That is a better starting position than a graphics framework that has to invent
one, and it is also a harder one to be honest about: everything below is about
your HTML, plus the small surface the shell adds.

## Theory: whose tree is it

There are two shapes a UI framework can take here, and they have opposite failure
modes.

**A framework that draws everything** (a GPU canvas, a custom widget tree) must
build an accessibility tree itself: roles, names, states, focus, and an adapter
per platform. It starts from nothing and gets exactly as far as its team invested.

**A framework that renders HTML** gets the tree for free — every element already
has a role, a name, a focus order and a state — and inherits the engine's bugs.
It starts from "mostly right" and gets exactly as far as its markup goes.

Voussoir is the second kind. So this chapter is not a tour of an API. It is a
list of the four defects that actually ship, and then the two things markup
cannot express that the shell provides.

## The four that ship

### 1. A control with no name

The engine announces an accessibile name for a form control, and if you did not
give it one it falls back to the placeholder — or to nothing, which a reader says
as "edit text".

```html
<!-- announced as "edit text" -->
<input id="name" placeholder="Name" />

<!-- announced as "Name" -->
<label for="name">Name</label>
<input id="name" />
```

A `placeholder` is not a name. It is a hint that disappears the moment someone
types, and it is the single most common a11y defect in hand-written frontends.

`aria-label` and `aria-labelledby` also work, and `aria-label` is the right tool
for an icon-only button. What does not work is nothing.

### 2. A heading level that skips

The heading list is a screen reader user's table of contents. `<h1>` then `<h3>`
gives them a list with a hole in it, which reads as a document that lost a
section.

```html
<h1>Notes</h1>
<h3>Recent</h3>     <!-- should be h2 -->
```

Do not choose a heading level for its size. Choose it for its depth, and change
the size with CSS.

### 3. No landmark

A reader can jump to landmarks — main, navigation, search. `<main>` is the one
almost every app needs, and a page without one has nothing to jump to.

### 4. A live region that is not live

A command's answer appears in a `<p>` that nobody announced. Mark the region you
update:

```html
<p id="result" role="status" aria-live="polite" aria-atomic="true">waiting</p>
```

`polite` waits for the reader to finish the current sentence; `assertive`
interrupts. Prefer `polite` unless the message is an error the user must hear
immediately, because interrupting is how an app becomes exhausting.

`role="status"` and `aria-live="polite"` together are the belt-and-braces form:
some engines honour one and not the other.

## The check

`voussoir doctor` reads your entry HTML and reports all four:

```
ok   a11y markup    labelled controls, ordered headings, a landmark
```

or, when something is wrong:

```
note a11y markup    2 finding(s): control-unlabelled, heading-skipped

  index.html:29  <input> has no label: add <label for="…">, or aria-label
  index.html:26  <h4> follows <h1>: heading levels must not skip
```

It is a **note**, not a failure: an app can be useful and still have one of these,
and a doctor that blocks is a doctor people stop running. It reads the file, not a
runtime tree, so it costs milliseconds and it cannot disagree with the DOM.

The scaffold you get from `voussoir init` passes all four rules, which is the
point — the correct state should be the default state.

## What markup cannot express

Two things, and they are what `voussoir.a11y` is for.

### `announce(message)`

A screen reader speaks a live region when its text *changes*. That covers a result
you render. It does not cover a change with no element behind it: a background
task that finished, a save that succeeded silently, a count that lives only in the
title bar.

```ts
if (window.voussoir.protocol >= 4) {
  window.voussoir.a11y.announce("async task finished");
}
```

Politeness is `polite`: it waits its turn. Announcing the same string twice in a
row works, because the shell clears the region before setting it — the obvious
implementation does not, and the second announcement is silently dropped.

`protocol >= 4` is the feature test. An older shell has no `a11y` member at all,
so a page that wants to support both guards on the number rather than probing.

**What this is not:** a replacement for markup. An element with the right role and
label is announced when it is focused, without any call. Reach for `announce` when
there is no element to focus.

### `prefers()` and `onPrefersChange(fn)`

The reader's system settings, read as a value:

```ts
const { reducedMotion, dark, highContrast } = window.voussoir.a11y.prefers();
window.voussoir.a11y.onPrefersChange((prefs) => {
  motion = prefs.reducedMotion ? 0 : 1;
});
```

`prefers-reduced-motion` and `prefers-color-scheme` are also CSS media queries, and
if your animation is CSS you should use them and never call this. These members
exist for the case CSS cannot reach: a page drawing to a `<canvas>`, or an
animation driven from script.

**Read it, do not cache it.** A user can turn reduce-motion on while the app is
open, which is exactly why `onPrefersChange` exists rather than a single read at
startup.

## Reduced motion in the scaffold

The stylesheet the CLI writes already honours it:

```css
@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after {
    animation-duration: 0.01ms !important;
    transition-duration: 0.01ms !important;
    scroll-behavior: auto !important;
  }
}
```

This is three lines and it is the difference between an app that makes someone
nauseous and one that does not. Vestibular disorders are not rare, and a spring
animation is the first thing that triggers them.

## What is not here, and why

- **No accessibility tree in Rust.** It would be a second source of truth beside
  the DOM, and the two would drift. The engine's tree is the one the OS reads.
- **No ARIA helper components.** A `<button>` is a button; `role="button"` on a
  `<div>` is a button that does not respond to Space, does not focus, and does not
  submit a form. The least code is the semantic element.
- **No automated audit of contrast.** It needs rendered pixels per theme, which is
  what `voussoir run --snap` produces; a check built on it is a good idea and is
  not written yet.

## Next

- [The frontend](frontend.md) — the page this chapter is about.
- [Testing](testing.md) — the harness that asserts these, so they do not rot.
- [Headless tooling](headless.md) — capturing the window to inspect it.
