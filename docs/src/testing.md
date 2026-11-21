# Testing

Voussoir apps are tested at three levels, and knowing which level a problem
belongs to is most of the skill. This chapter covers all three, plus the traps
specific to an app whose UI is a webview.

## Theory: what is testable, and what is not

The parts of an app are not equally testable, and pretending otherwise produces
slow, flaky tests that get deleted within a month:

| Layer | Testable without a window | How |
| --- | --- | --- |
| Command logic | yes | plain Rust unit tests |
| Payload parsing and errors | yes | plain Rust unit tests |
| The registry and allowlist | yes | `Registry` in unit tests |
| The shim's JS | yes | run it under Bun with a fake bridge |
| The real IPC transport | no | needs a webview: `scripts/verify.ts` |
| Rendering and CSS | no | a human, or a screenshot diff |

The rule that follows: **put logic where the first four rows can reach it.** A
command that is a thin wrapper around a well-tested function is worth more than a
command that contains the logic, because the logic can then be tested in
milliseconds instead of seconds.

This is not a testing preference, it is an architectural one, and it is worth
stating as a design rule: if a piece of logic can only be tested by opening a
window, it is in the wrong place.

## Theory: what a test is actually for

A test is a claim that something is true, written so that a machine re-checks it
whenever the code changes. Three things follow, and each one changes how you write
it:

1. **A test that cannot fail is documentation.** If deleting the implementation
   still passes the test, the test asserts nothing. This sounds obvious and it is
   the most common defect in a test suite.
2. **A test's value is in its failure message.** A failing test should say what
   broke. `assert!(result.is_ok())` tells you nothing; `assert_eq!(reply, json!("Hello, ada!"))`
   tells you the value, and often the cause.
3. **A slow test is a test that gets skipped.** Milliseconds invite running the
   suite constantly; minutes invite running it before a release, which is when it
   is least useful.

So the aim is not coverage. It is: for every behaviour that would cost real money to
break, a test that fails loudly and runs fast.

## Level 1: Rust unit tests

Commands are ordinary functions, so they test like ordinary functions. This is the
payoff for keeping the command thin:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    // The commands crate depends on serde_json alone, so this is the direct
    // import; `voussoir::serde_json` would not resolve here.
    use serde_json::json;

    #[test]
    fn greet_uses_the_supplied_name() {
        let reply = greet(&json!({ "name": "ada" })).unwrap();
        assert_eq!(reply, json!("Hello, ada!"));
    }

    #[test]
    fn greet_falls_back_when_the_name_is_missing() {
        assert_eq!(greet(&json!({})).unwrap(), json!("Hello, world!"));
    }

    #[test]
    fn greet_ignores_a_name_that_is_not_a_string() {
        assert_eq!(greet(&json!({ "name": 7 })).unwrap(), json!("Hello, world!"));
    }
}
```

What each test is doing, because the pattern matters more than the code:

- **The first is a success path.** One is enough per command; you are checking the
  wiring, not re-testing `format!`.
- **The second is a missing field.** This is the case a page will actually hit
  first, because the page is written after the command.
- **The third is a wrong type.** A page is untrusted input like any other, and
  `{"name": 7}` is a real thing a bug will send.

Run them with:

```bash
cargo test                        # the whole workspace
cargo test -p commands            # just your command crate
cargo test greet_uses             # one test, by substring
cargo test greet_uses -- --nocapture   # and show println output
```

The `-p commands` form is the one you will use most: your command crate is where
iteration happens, and it compiles in a second.

**What to cover:** every failure branch, and one success path per command. Do not
aim for 100%, aim for "a wrong reply fails a test" on the commands that touch files,
money, or the user's data.

### Testing a failure

An error is a return value, so a failure test asserts on it directly:

```rust
#[test]
fn save_rejects_a_payload_without_a_path() {
    let err = save(&json!({})).unwrap_err();
    assert_eq!(err.kind(), "payload");
    assert!(err.to_string().contains("path"));
}
```

`Error::kind()` returns the same string that travels on the wire, so the test
asserts the exact value a page will branch on rather than a Rust-only enum. That is
deliberate: if the wire value changes, the test fails here instead of in a shipped
frontend.

Asserting the **kind** rather than only the message is the part that survives a
reword. The message assertion is a second, looser check that the text names the
field the caller forgot; it is worth having, and it will need updating the day you
improve the wording, which is the right time to notice it.

### Testing something that touches the disk

Use a temporary directory, and let the test own it:

```rust
#[test]
fn round_trips_a_note() {
    let dir = std::env::temp_dir().join(format!("notes-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("note.txt");

    write_note(&path, "hello").unwrap();
    assert_eq!(read_note(&path).unwrap(), "hello");

    std::fs::remove_dir_all(&dir).unwrap();
}
```

Two details that make this reliable:

- **Include the process id in the name**, because `cargo test` runs tests in
  parallel threads of one process and two runs can overlap on CI.
- **Delete at the end of the test body, not in a `Drop` impl**, so a panic leaves
  the directory behind for you to inspect. A test that cleans up after a panic has
  destroyed the evidence.

## Level 2: the registry

Whether a command is reachable at all is worth testing, because a missing table
entry is a silent `kind: "unknown"` in production:

```rust
#[test]
fn every_command_in_the_table_is_registered() {
    let app = yourapp::build_app();   // a helper that returns the configured App
    for command in commands::COMMANDS {
        assert!(app.has_command(command.name), "{} is not registered", command.name);
    }
}
```

`commands::COMMANDS` is the table generated into **your** commands crate by
`voussoir init`, not something the framework exports: it is compiled as a `cdylib`
in dev, so the crate declares the table itself. The `voussoir` crate has no commands
module to import.

`has_command` answers for the registry alone and **ignores the allowlist**, so this
asserts "exists", not "granted". That distinction is deliberate and useful: a
command that exists but is withheld is a policy decision, while a command that does
not exist is a bug.

The `build_app` helper is worth writing for this alone. It means the test builds the
same `App` that `main` builds, so the test cannot pass while the real app is missing
a registration.

### Testing the allowlist

There is no `allows(label, name)` predicate to assert on: the intersection is
applied inside dispatch, so a denial is observable as an error envelope. Assert
that, because it is also what the page branches on:

```rust
#[test]
fn a_preview_window_cannot_reach_delete() {
    let mut registry = voussoir::Registry::new();
    registry.set_allowlist(Some(vec!["greet".to_owned(), "delete".to_owned()]));
    registry.set_window_allowlist(std::collections::HashMap::from([(
        "preview".to_owned(),
        vec!["greet".to_owned()],
    )]));

    registry.add("greet", |_app, _payload| Ok(json!("hi")));
    registry.add("delete", |_app, _payload| Ok(json!(true)));

    // Dispatch as the "preview" window and read the envelope.
    // assert it contains "greet" ok and `"kind":"unknown"` for delete.
}
```

The crate's own `ipc` module tests are the worked example: they dispatch from a
named window and assert on the serialized envelope. Copy the shape from there
rather than inventing a helper.

## Level 3: the real webview

Some things can only be proven with a window: that the shim installed, that the
custom protocol answered, that a panic became an `{error}` envelope instead of
killing the process, that navigation was actually refused.

`scripts/verify.ts` does exactly this. It boots a real window against a real page
and asserts:

- a command round-trip over the real transport,
- a hostile page's script does not execute,
- `eval` and remote `fetch` are blocked,
- navigation away from the app is refused,
- a panicking handler returns `kind: "panic"` and the window survives,
- the storage round-trip works,
- a narrowed window is denied.

Run it locally:

```bash
bun scripts/verify.ts
```

It needs a display. On a headless Linux box, `xvfb-run -a bun scripts/verify.ts` is
what CI uses: `xvfb` is a virtual framebuffer, so a window can be created with no
monitor attached.

**Why not unit-test those?** Because the thing being tested is the webview's
behaviour, not yours. A mock would assert that your mock works, and the bug you are
looking for lives in the part you mocked away.

## Theory: why three levels instead of one

Each level costs a different amount and proves a different thing:

| Level | Cost per test | Proves |
| --- | --- | --- |
| Rust unit | milliseconds | your logic |
| Registry | milliseconds | your wiring |
| Real webview | seconds, needs a display | the platform's behaviour |

A suite made only of level 3 is too slow to run while working, so it gets deferred
and then distrusted. A suite made only of level 1 proves your logic and nothing
about whether the app starts.

The distribution to aim for is the same as the cost: many unit tests, a handful of
registry tests, and one end-to-end run that exercises the contract as a whole.

## Testing the page

The shim is deliberately replaceable, which makes page code testable without a
window. The trick is a test double with the same shape:

```ts
// A test double that matches the shape the shim installs.
function fakeVoussoir(replies: Record<string, unknown>) {
  return {
    invoke: async (method: string) => {
      if (!(method in replies)) {
        throw Object.assign(new Error("unknown"), { kind: "unknown", method });
      }
      return replies[method];
    },
    listen: () => () => {},
    windows: async () => [],
    tray: {},
    menu: {},
    platform: "linux" as const,
    pageOrigin: "voussoir://localhost",
    protocol: 3,
    timeout: 30000,
  };
}

beforeEach(() => {
  (globalThis as any).window = { voussoir: fakeVoussoir({ greet: "hi" }) };
});
```

The `throw` with a `kind` matters: a double that resolves `undefined` for an unknown
method teaches your tests that every command succeeds, which is the opposite of
production.

Keep the double in one file. When the shim gains a member, that file is the only
place to update, and a missing member shows up as a type error rather than a
mystery `undefined` in one test.

## What CI runs

| Job | Command | Catches |
| --- | --- | --- |
| Rust tests | `cargo test` | logic, registry, panic boundary |
| CLI tests | `bun test` in `cli/` | scaffolder, docs check, dev server helpers |
| End to end | `bun scripts/verify.ts` | the real transport, policy, panic path |
| Hot reload | `bun scripts/verify-hot.ts` | the swap, and no `libloading` in release |
| Coverage | `./scripts/coverage.sh` | untested code, against a floor |
| Docs | `voussoir docs --check` | stale links and summary drift |

Run the first two before opening a pull request. The rest are cheap enough to let
CI do, but a red coverage job is worth reading: it usually means a new branch has no
test at all, not that the percentage moved.

## The traps

- **A green test that never ran the handler.** If you test through the registry,
  assert the reply, not just that no error came back. `assert!(result.is_ok())`
  passes for a handler that returns early.
- **Timing in a webview test.** "Wait 100 ms for the page to load" is a flake
  waiting to happen, and it fails on a slow CI runner first. Wait for a signal the
  page sends (an `invoke` that only happens after load) instead.
- **Testing the panic path with `catch_unwind` in your own code.** You cannot: the
  shell owns the boundary, and a command that catches its own panics is not what
  the guarantee is about. Assert it end to end.
- **Snapshotting HTML.** It breaks on whitespace and teaches nobody anything.
  Assert the text a user reads.
- **One shared temp directory.** Two tests that write the same file pass alone and
  fail together, which is the most expensive kind of flake to diagnose.
- **A test double that never fails.** If your fake `invoke` always resolves, every
  test of your error handling is dead code.

## Next

- [Benchmarks](benchmarks.md) — measuring what tests cannot assert.
- [Diagnostics](diagnostics.md) — what to record when a test cannot help.
