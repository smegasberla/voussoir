# Operations

This section is the unglamorous half of a shipped app: what it costs to start,
what it tells you when something goes wrong, and how it behaves on each target.
None of it is needed to build a first screen, and all of it is what decides
whether a bug report is a five-minute fix or a week.

## Theory: why this is a section and not an appendix

Small projects tend to treat observability as something added after launch. That
ordering rarely works, because the decisions that make an app observable are
structural:

- whether the shell keeps a record of what happened (it does, in a ring buffer),
- whether a command runs on the UI thread or a worker (a per-command choice),
- whether a claim in the docs is measured or asserted (a per-claim choice).

Retrofitting any of those is a rewrite. Doing them from the start costs a table
row and a paragraph, which is what this section is.

## The three tools

Each is measured rather than asserted:

| Tool | Answers |
| --- | --- |
| `bun scripts/verify.ts` | does the shipped behaviour actually hold? |
| `bun scripts/bench.ts` | what does it cost to start, and per call? |
| `voussoir dev --verbose` | what happened in the last few seconds of this session? |

### `verify.ts`

Boots a real window on the real IPC path and asserts, by name:

- the shell's unit tests pass, as a precondition for anything below,
- an embedded page round-trips and an emitted event reaches it,
- `voussoir init` scaffolds a project that builds,
- the artifact exists and is **under 8 MB**,
- no `libnode` or `libv8` is linked, so the shipped app has no JS runtime,
- on Linux only, the binary links `libwebkit2gtk` and nothing else surprising,
- a dev-mode round trip works, and a **second window narrowed by
  `voussoir.json`** is refused the command it does not have,
- the compiled CLI runs, and `voussoir package` produces an AppDir.

**What is not here:** the CSP, `eval` and remote-`fetch` refusals, the navigation
veto, the panic envelope and the 4 MB storage cap are **unit tests** in the crate,
not end-to-end checks. That distinction is deliberate: those are pure functions and
a unit test pins them more precisely than a window can. When a chapter says
"asserted", check [Testing](testing.md) for which harness.

### `bench.ts`

Cold start against a bare `wry`+`tao` control, and with `--dispatch` the cost of
the three execution models. Both run in CI and both gate. See
[Benchmarks](benchmarks.md).

There is a fourth check that is not a tool you run: the libFuzzer harness in
`fuzz/` gets a weekly schedule rather than a place in this table, because it needs
nightly and a time budget. Every push still compiles it.

### `--verbose`

Reloads, slow commands, refused navigations and panics, recorded so a snapshot
survives the terminal. See [Diagnostics](diagnostics.md).

## Theory: what "verified" means here

A claim in this book is either measured by one of those tools or it is not made.
The distinction shows up in the [Platform support](platforms.md) table, where a
cell says `cargo check` when the code compiles for the target but no window was
opened.

That vocabulary exists because "supported" is an ambiguous word, and the ambiguity
hides in the interesting place: a compile-only target is where runtime bugs live.
A table that marks it `yes` is not lying about effort, it is lying about
knowledge.

## Coverage

```bash
./scripts/coverage.sh
```

Reports line coverage for the library's own tests and fails under a floor of
**65%**. The measured baseline is **67.58%** (2026-11-21, default features, test and
fuzz modules excluded), so the number describes the shell rather than its
assertions.

The gap between 65 and 67.58 is the point, not slack. `VOUSSOIR_COV_FLOOR=0
./scripts/coverage.sh` reports without failing, which is what you want while
investigating.

## Theory: why a floor and not a target

Coverage is not a quality score. A fully covered function can be entirely wrong;
an uncovered one can be trivially correct. What coverage does measure is the thing
that decays quietly: **unexercised code paths accumulate**.

Three decisions make the gate useful rather than annoying:

1. **The floor sits a point or two under the measured number**, not at it. A gate
   set at the exact current value fails on honest changes and gets deleted.
2. **Test and fuzz modules are excluded from the denominator**, because counting
   assertions as covered code inflates the number and hides the gap.
3. **Raising is a decision; lowering is not.** Raise when a release closes a real
   gap. Never lower it to make a build pass, because that converts a measurement
   into a decoration.

It uses the `llvm-profdata` and `llvm-cov` the toolchain already links against, so
there is nothing to install and CI runs the same script as a developer.

## Theory: what an exit code is worth

Every tool here fails with a non-zero exit when its claim does not hold. That is
what lets CI be a review rather than a ritual: the build is green because the
claims were checked, not because nobody looked.

The corollary is a rule worth adopting in your own app: **a test that cannot fail
is documentation**. If removing the implementation still passes, the test is
asserting that the process did not crash.

## In this section

- [Diagnostics](diagnostics.md) — the `--verbose` mode and what it records.
- [Benchmarks](benchmarks.md) — the numbers and how to reproduce them.

## Next

- [Troubleshooting](troubleshooting.md) — symptom to cause, by layer.
- [Testing](testing.md) — writing the assertions this section relies on.
