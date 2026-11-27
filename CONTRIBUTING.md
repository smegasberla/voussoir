# Contributing

The honest starting point: this is a single-maintainer project written with AI
assistance. Both facts are stated in the README too, and neither is a reason not to
send a patch, but they do describe the review capacity you should expect.

## What a change needs

Every change is held to the checks that run without asking anyone's opinion:

```sh
cargo test -p voussoir --features full
cargo test -p voussoir --features full,rt --lib
cargo clippy -p voussoir --features full,rt,storage --all-targets -- -D warnings
cargo fmt --all --check
bun test                        # in cli/
bun scripts/verify.ts           # needs a display; runs the real IPC path
voussoir docs --check           # if you touched docs/src
```

CI runs those on Linux, Windows, macOS, FreeBSD, NetBSD, an Android emulator and an
iOS simulator, plus `cargo deny` and a build at the declared MSRV. A change that only
passes on Linux is not finished.

## How to make a review easy

- **Say which platform you could not test.** That is more useful than implying you
  tested everything. A reviewed-but-unrun branch is the one that surprises people,
  so it is better to flag it than to hide it.
- **Prefer a test that fails without your change.** Every regression here was caught
  by a test written to reproduce it, including two that were added after a bug
  shipped.
- **Do not add a dependency to save a few lines.** Optional crates sit behind
  features so a minimal app pays nothing, and `deny.toml` refuses licences outside
  its allow list. A new dependency needs an entry there and a reason in the PR.
- **Keep the prose.** The comments explain why, and the docs are checked as
  structure. A patch that deletes the reasoning is harder to review than one that
  deletes code.

## Scope

This project says no to features on purpose. Before writing one, consider whether it
belongs in the app rather than the shell: the shell's job is a window, the platform
webview, and one JSON bridge. Commands, menus, dialogs and updates are the edge of
that scope, and everything past it should live in the app crate.

Security reports do not go here. Use [SECURITY.md](SECURITY.md).

## Licence

Contributions are under the MIT licence in [LICENSE](LICENSE), and there is no CLA.
