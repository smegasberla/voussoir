# Compatibility policy

The crate is 0.x, so [Cargo's 0.x
rules](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html)
already allow any minor bump to break. What Voussoir promises on top of that is
written down here, so "is this upgrade safe for my app?" has an answer that does not
depend on reading the diff.

## Theory: what a compatibility policy is for

A compatibility policy is a promise about the **cost of upgrading**, and its value
is entirely in what it lets you skip reading. If a project says "semver" and
nothing else, you must read every changelog entry to find the one that matters.

So the useful shape is a list of *what counts as breaking*, because that turns
"review everything" into "review these five categories". Everything outside the
list is safe to take without reading.

## Breaking changes

These require a 0.x bump and a CHANGELOG entry under its own heading:

- command names and the IPC envelope shape (`{"ok"|"error"}`)
- the public `App` and `AppHandle` signatures
- the `voussoir.json` schema
- the `window.voussoir` shim surface (`invoke`, `listen`, and their argument shapes)
- the meaning of an existing cargo feature

The items on that list share a property worth naming: they are the boundaries a
**page** crosses as well as a Rust crate. A command name is a string in JavaScript;
an envelope shape is a branch in a `catch`; a feature's meaning is a build
configuration. Those are what an app actually depends on, so they are what the
policy protects.

## Not breaking

- internal module layout
- new opt-in features, built-in commands, or `voussoir.json` keys (additive)
- patch bumps, which add no API change

Additive is a real promise and worth spelling out: a new command does not break a
page that does not call it, and a new optional `voussoir.json` key does not change
an app that omits it.

There is one honest caveat. Because an absent `commands` key means "every
registered command", adding a built-in command to such an app *does* widen its
surface. That is additive by this policy and still a widening, so an app that cares
lists its commands explicitly: see [Per-window allowlists](allowlists.md).

## Theory: version the shim separately from the crate

There are two independent versions, and mixing them up is the most common
compatibility mistake:

| Version | Where | Changes when |
| --- | --- | --- |
| crate | `Cargo.toml` | the Rust API changes |
| shim protocol | `window.voussoir.protocol` | the page-facing object changes |

Why two: a page can outlive a shell upgrade as long as it only uses members that
still exist, and a frontend can be rebuilt without touching the crate. A single
version would make every page change look like a crate change and vice versa.

The shim carries its own version so a page can **feature-detect** instead of
probing for methods:

```ts
if (window.voussoir.protocol >= 2) {
  const open = await window.voussoir.windows();
}
if (window.voussoir.protocol >= 3) {
  const base = window.voussoir.pageOrigin;
}
if (window.voussoir.protocol >= 4) {
  window.voussoir.a11y.announce("saved");
}
```

The current protocol is **4**, and the bump is documented in `CHANGELOG.md` with
what each version added: 2 added `windows`, `tray` and `menu`; 3 added
`pageOrigin`; 4 added `a11y`.

- **Protocol 2** added `windows()`, `tray` and `menu`.
- **Protocol 3** added `pageOrigin`.

The current version is **3**. Each guard names the version that introduced the
member it protects, because a page that asks for `>= 2` and then reads
`pageOrigin` is checking the wrong number on a version-2 shell.

**Why detect a version instead of `if (voussoir.tray)`?** Both work, and they
answer different questions. The version answers "what shape is this object", which
is what you need when you use three members together. Probing answers "is this one
member here". Use the version for a block of related API, and probe for a single
optional member.

## Theory: why the app version and the crate version are separate

The version in `voussoir.json` and the version of the `voussoir` crate are
deliberately unrelated. An app does not have to adopt a new shell to keep its
frontend working, and a shell upgrade does not have to carry an app version bump.

The practical consequence is two numbers in a bug report. Ask for both: "it broke
after `cargo update`" and "it broke after a rebuild" are different stories, and
knowing which one happened saves the first hour.

## How a change is recorded

Every user-visible change is in `CHANGELOG.md` in Keep a Changelog format, so the
upgrade question is answerable from the changelog plus the lists above. A change in
a breaking category gets a heading of its own; an addition is filed under Added and
needs no further review.

## Upgrading, in order

1. Read the changelog for the whole range you are crossing, not the last entry.
2. If nothing in the breaking list appears, bump and rebuild.
3. Run `bun scripts/verify.ts`. It covers the envelope, the policy and the
   multi-window contract, which is most of what a page depends on.
4. If the page uses the shim, check that `protocol` did not fall behind its guards.

## Next

- [Platform support](platforms.md) — what each version is tested on.
- [Packaging](packaging.md) — what a version means once it has shipped.
