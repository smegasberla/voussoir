# Benchmarks

Two numbers matter, and both are measured on the machine you run them on rather
than quoted from a spec sheet.

```bash
bun scripts/bench.ts             # cold start
bun scripts/bench.ts --dispatch  # the three execution models
```

## Theory: what a benchmark is for, and what it is not

A benchmark is not a score. It is an **experiment with a control**: you measure
something twice, under conditions that differ in exactly one way, and the
difference is the cost of that one thing.

That is why both benchmarks here compare against a *baseline binary* rather than
against a number from a previous run on a different machine. Comparing to a fixed
number answers "is this machine fast today"; comparing to a control answers "what
does Voussoir cost", which is the question an app author actually has.

The other half of the theory is that a benchmark must be **reproducible by a
sceptic**. Every number in this book comes from a command in this chapter, so a
reader who doubts it can run it.

## Cold start

Times process start to the page-load-finished callback, in a release binary,
against a bare `wry`+`tao` binary that does the same work: one event loop, one
window, one webview, one page.

Why "cold start" specifically: it is the number a user feels. A slow first paint is
the difference between an app that feels instant and one that feels like a web page
loading. Everything after the first paint is the webview's job.

Seven samples each, reported as a median:

- **Seven** because the first run of a binary pays page-cache costs that later runs
  do not, and a single sample is noise.
- **Median** rather than mean, for the reason in
  [Diagnostics](diagnostics.md): one 400 ms outlier moves a mean and does not move
  a median.

The script also flags a sample far above the median, which means the machine was
busy (a build, a browser) rather than the shell being slow. That distinction
matters: without it, CI fails at random on shared runners and the gate gets
deleted.

It exits non-zero when the overhead exceeds **+10%**, so CI can gate on a
regression rather than a trend.

## Theory: why 10%, and why a gate at all

A gate at the exact current number fails on every honest change and is removed
within a month. A gate far above it never fires. Ten percent is wide enough to
absorb machine noise and narrow enough to catch a dependency added to the startup
path, which is the usual way a fast shell becomes a slow one.

The decision worth knowing: **the number is compared to the control binary built in
the same run**, so a slow CI runner makes both slow and the ratio holds. Only a real
change moves it.

## Async dispatch

```bash
bun scripts/bench.ts --dispatch
```

Measures the three execution models against each other:

| Path | Model | Cost per call |
| --- | --- | --- |
| `command` | UI thread | a function call |
| `async_command` | thread per call | a thread spawn and join |
| `async_fn` | task on a shared runtime | a task submit |

## Theory: why the last two are the interesting comparison

Every command has to answer one question before you write it: **where does it run?**
The three answers cost different things, and the difference is only visible under
concurrency.

- **Sync** is a function call. Fastest, and the only one that can stutter a window,
  because it runs where the painter runs.
- **Thread per call** needs a stack (typically megabytes of address space) and a
  spawn. Fine for a handful of calls; a page that fires a thousand gets a thousand
  stacks.
- **Task** is a small allocation on a shared runtime. It is the one that has a
  fixed ceiling regardless of load.

The runtime behind `async_fn` starts one worker per core, with a floor of two.
The floor is deliberate: a single worker plus a blocking future inside a task is a
deadlock waiting for the first timeout to be added.

The benchmark prints per-call cost at several concurrency levels, so "the pooled
path scales better" is a number rather than a belief. It also fails when the pooled
path is slower than thread-per-call at the concurrency level where it is supposed
to win, which would mean the runtime is misconfigured rather than merely slower.

Both halves of `scripts/bench.ts` run in CI, so neither number is a snapshot taken
the last time someone remembered. The gate is a ratio between two binaries built in
the same run — which is what makes it safe on a shared runner — and it fails on a
real regression of the startup path rather than on machine noise.

## Reading a result

```
  voussoir shell         median  112 ms    bare wry+tao  median   98 ms   +14.3%
```

- The left column is what you ship; the right is the floor, because a window and a
  webview cost something no framework can remove.
- The percentage is the cost of the framework: the registry, the shim, the policy.
- If the delta is negative and stays negative across runs, the shell is doing less
  work than the control, which usually means the control is not doing the same
  thing any more. Check the control before celebrating.

## What is not benchmarked, and why

- **Startup on Windows and macOS.** The binary is built and tested there, but the
  cold-start number in the README is Linux and says so. Reporting one platform's
  number under a general label is the kind of claim this project tries not to make.
- **Render performance.** That is the webview. Measuring it would measure WebKit
  or Chromium, not Voussoir, and the result would change with an OS update.
- **Binary size over time.** `verify.ts` checks a size bound. It is a gate, not a
  trend, for the same reason the startup gate is: a trend chart is read once and a
  gate runs every commit.

## Running it honestly

- **Close the browser.** A benchmark on a busy machine measures the machine.
- **Run it twice.** A first run after a build pays disk costs.
- **Change one thing.** If you are comparing two branches, check out, build, and
  run without touching anything else in between.
- **Trust the ratio, not the milliseconds.** The absolute number is your hardware.

## Next

- [Operations](operations.md) — the tools that produce these numbers.
- [Diagnostics](diagnostics.md) — measuring a single command while you work.
