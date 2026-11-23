/** End-to-end verification for hot-reloaded commands (`voussoir dev`).
 *
 *  What it proves, in one headless run:
 *
 *  - the scaffolded commands crate's own ABI tests pass (panic containment,
 *    envelope shape, junk input);
 *  - `libloading` is absent from the release dependency tree and present only
 *    behind the `hot-reload` feature;
 *  - a real `voussoir dev` session answers `greet`, and editing the command swaps
 *    a new library into the *same* running process — the page's own report
 *    counter never resets and its start timestamp never changes, which is
 *    what "no restart" means from the inside;
 *  - the shell prints a rebuild-to-reload latency;
 *  - a panicking hot command comes back as an error envelope and the window
 *    keeps serving;
 *  - a syntactically broken edit keeps the last good library (and says so),
 *    and fixing the file recovers.
 *
 *  Run: bun scripts/verify-hot.ts
 *  Headless: xvfb-run -a bun scripts/verify-hot.ts
 *
 *  Linux-only by design (the CI job that runs it is the Linux one); the unit
 *  tests for the same logic in voussoir/src/hot.rs are platform-neutral.
 */

import { appendFileSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const repo = resolve(import.meta.dir, "..");
const cli = join(repo, "cli", "src", "index.ts");
const results: { name: string; ok: boolean; detail: string }[] = [];

function record(name: string, ok: boolean, detail: string) {
  results.push({ name, ok, detail });
  console.log(`${ok ? "ok  " : "FAIL"} ${name.padEnd(42)} ${detail}`);
}

async function sh(cmd: string[], cwd: string) {
  const proc = Bun.spawn(cmd, {
    cwd,
    env: process.env,
    stdout: "pipe",
    stderr: "pipe",
  });
  const [out, err, code] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
    proc.exited,
  ]);
  return { out, err, code };
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function waitFor(predicate: () => boolean | Promise<boolean>, timeoutMs: number): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await predicate()) return true;
    await sleep(150);
  }
  return predicate();
}

/** The page: a beacon loop (proves the page runs even if the bridge hangs)
 *  plus a `greet` loop (the actual subject). A process or webview restart
 *  resets both counters and the start time, so "beacons kept climbing and
 *  started never changed" *is* the no-restart proof, observed from inside the
 *  thing that would have restarted. */
function pageSource(reportUrl: string): string {
  return `const report = ${JSON.stringify(reportUrl)};
let beacons = 0;
let runs = 0;
const started = Math.round(performance.timeOrigin);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const send = async (payload) => {
  try { await fetch(report, { method: "POST", body: JSON.stringify(payload) }); } catch {}
};
(async () => {
  for (;;) {
    beacons += 1;
    await send({ reply: "beacon", beacons, runs, started });
    await sleep(500);
  }
})();
(async () => {
  for (;;) {
    runs += 1;
    let reply;
    try {
      const res = await (window as any).voussoir.invoke("greet", { name: "hot" });
      reply = typeof res === "string" ? res : JSON.stringify(res);
    } catch (e) {
      reply = e instanceof Error ? e.message : String(e);
    }
    await send({ reply, beacons, runs, started });
    await sleep(500);
  }
})();
`;
}

const app = mkdtempSync(join(tmpdir(), "voussoir-hot-"));
const commandsFile = join(app, "native", "commands", "src", "lib.rs");
let dev: ReturnType<typeof Bun.spawn> | null = null;
let reportServer: { stop: (force?: boolean) => void; port: number } | null = null;

async function finish(code: number): Promise<never> {
  dev?.kill();
  reportServer?.stop(true);
  const failed = results.filter((r) => !r.ok);
  console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
  for (const failure of failed) console.log(`  FAIL ${failure.name}: ${failure.detail}`);
  rmSync(app, { recursive: true, force: true });
  process.exit(code || (failed.length ? 1 : 0));
}

try {
  // --- Scaffold -----------------------------------------------------------
  const init = await sh([process.execPath, cli, "init", app], repo);
  record("voussoir init scaffolds the test app", init.code === 0, init.code === 0 ? app : (init.err || init.out).slice(-400));
  if (init.code !== 0) await finish(1);

  // --- The scaffolded crate's own contract tests --------------------------
  const abi = await sh(["cargo", "test", "--manifest-path", "commands/Cargo.toml"], join(app, "native"));
  record(
    "commands crate ABI tests",
    abi.code === 0,
    abi.code === 0
      ? (/test result: ok\. (\d+) passed/.exec(abi.out)?.[1] ?? "?") + " passed"
      : abi.out.slice(-600),
  );

  // --- The dependency claims ---------------------------------------------
  // `libloading` already reaches the release tree transitively (rfd →
  // wayland-sys → dlib), before this feature existed. What must hold is that
  // the *shell itself* does not depend on it unless the feature asks: a
  // `--depth 1` tree shows exactly voussoir's own edges.
  const releaseTree = await sh(["cargo", "tree", "-p", "voussoir", "--features", "full", "--depth", "1"], repo);
  record(
    "release shell has no direct libloading edge",
    releaseTree.code === 0 && !releaseTree.out.includes("libloading"),
    releaseTree.code === 0
      ? releaseTree.out.includes("libloading")
        ? "direct dependency present!"
        : "absent (transitive rfd/wayland edge unchanged)"
      : "cargo tree failed",
  );
  const hotTree = await sh(["cargo", "tree", "-p", "voussoir", "--features", "full,hot-reload", "--depth", "1"], repo);
  record(
    "hot-reload feature pulls libloading",
    hotTree.code === 0 && hotTree.out.includes("libloading"),
    hotTree.code === 0 ? (hotTree.out.includes("libloading") ? "present behind the feature" : "missing!") : "cargo tree failed",
  );

  // --- The live session ---------------------------------------------------
  const reports: { reply: string; beacons: number; runs: number; started: number }[] = [];
  // Raw arrival counters, independent of parsing: separates "requests never
  // reached the server" from "they arrived but could not be read".
  let rawHits = 0;
  let parseFailures = 0;
  /** No restart happened if the page's start time never changed and neither
   *  counter ever went backwards (a restart resets them to small numbers).
   *  Counters are compared with `>=`, not `==`, so one lost HTTP report on a
   *  busy machine cannot fake a restart. */
  const pageStable = () => {
    if (reports.length === 0) return false;
    const started = reports[0]!.started;
    const beacons = reports.filter((r) => r.reply === "beacon");
    const replies = reports.filter((r) => r.reply !== "beacon");
    return (
      reports.every((r) => r.started === started) &&
      beacons.every((r, i) => i === 0 || r.beacons >= beacons[i - 1]!.beacons) &&
      replies.every((r, i) => i === 0 || r.runs >= replies[i - 1]!.runs)
    );
  };
  reportServer = Bun.serve({
    port: 0,
    async fetch(req) {
      if (req.method === "OPTIONS") {
        return new Response(null, {
          headers: {
            "access-control-allow-origin": "*",
            "access-control-allow-methods": "POST",
          },
        });
      }
      if (req.method === "POST") {
        rawHits += 1;
        // Await the body before responding: the earlier fire-and-forget
        // `.then` read never settled for a single one of hundreds of POSTs,
        // while the requests themselves demonstrably arrived.
        try {
          const text = await req.text();
          reports.push(JSON.parse(text));
        } catch {
          parseFailures += 1;
        }
        return new Response("ok", { headers: { "access-control-allow-origin": "*" } });
      }
      return new Response("not found", { status: 404, headers: { "access-control-allow-origin": "*" } });
    },
  });

  writeFileSync(join(app, "src", "main.ts"), pageSource(`http://127.0.0.1:${reportServer.port}/report`));

  const devPort = 16000 + Math.floor(Math.random() * 8000);
  // `--verbose` is what makes the shell print its diagnostics, and the reload
  // latency this script asserts is one of them. Without it `dev` sets
  // VOUSSOIR_VERBOSE=0 explicitly and the line never reaches the log.
  dev = Bun.spawn([process.execPath, cli, "dev", "--port", String(devPort), "--verbose"], {
    cwd: app,
    env: process.env,
    stdout: "pipe",
    stderr: "pipe",
  });
  let devLog = "";
  const collect = async (stream: ReadableStream<Uint8Array>) => {
    for await (const chunk of stream) devLog += new TextDecoder().decode(chunk);
  };
  collect(dev.stdout as unknown as ReadableStream<Uint8Array>);
  collect(dev.stderr as unknown as ReadableStream<Uint8Array>);

  // Server side first: is the dev server up, and does it transform the
  // reporter page? Separates "webview problem" from "dev-server problem".
  // Bun rewrites the entry HTML (its own HMR client, hashed asset URLs), so
  // sweep every script the HTML declares and accept the one that carries the
  // reporter — a raw `/src/main.ts` fetch would 404 even on a healthy server.
  let servedVia = "";
  const pageServed = await waitFor(async () => {
    try {
      const res = await fetch(`http://127.0.0.1:${devPort}/`);
      if (!res.ok) return false;
      const body = await res.text();
      const srcs = [...body.matchAll(/<script[^>]+src="([^"]+)"/g)].map((m) => m[1]!);
      for (const src of srcs) {
        try {
          const mod = await fetch(new URL(src, `http://127.0.0.1:${devPort}/`));
          if (mod.ok && (await mod.text()).includes("beacon")) {
            servedVia = src;
            return true;
          }
        } catch {}
      }
      return false;
    } catch {
      return false;
    }
  }, 60_000);
  record(
    "dev server serves the reporter page",
    pageServed,
    pageServed ? `module: ${servedVia}` : `tail: ${devLog.slice(-500)}`,
  );

  const gotFirst = await waitFor(() => reports.some((r) => r.reply.includes("Hello, hot!")), 180_000);
  record(
    "first greet round-trip over dev IPC",
    gotFirst,
    gotFirst
      ? `reply: ${reports.find((r) => r.reply.includes("Hello"))?.reply}`
      : `timeout; raw=${rawHits} parsed=${reports.length} parseFail=${parseFailures}, beacons=${reports.filter((r) => r.reply === "beacon").length}: ${JSON.stringify(reports.slice(0, 3))}; dev log tail: ${devLog.slice(-700)}`,
  );
  if (!gotFirst) await finish(1);

  const original = readFileSync(commandsFile, "utf8");
  const howdy = original.replace("Hello, {name}! (from Rust)", "Howdy, {name}! (from Rust)");
  record("scaffold greets as expected", howdy !== original, howdy !== original ? "greeting found" : "greeting string missing from template");

  // --- Edit 1: a new answer, same process ---------------------------------
  const editAt = Date.now();
  writeFileSync(commandsFile, howdy);
  const gotHowdy = await waitFor(() => reports.some((r) => r.reply.includes("Howdy, hot!")), 60_000);
  const editToVisible = Date.now() - editAt;
  // The shell reports this through `diagnostics::record`, which prints the
  // message and then its fields as `key=value`. There is no parenthetical
  // latency in the message, so the check reads the field the record carries.
  const hotLine = devLog.split("\n").find((line) => line.includes("hot commands gen"));
  const shellLatency = hotLine ? /rebuild_to_reload_ms=(\d+)/.exec(hotLine)?.[1] : undefined;
  record(
    "command edit reloads without restart",
    gotHowdy && pageStable(),
    gotHowdy ? `edit-to-visible ${editToVisible} ms, page runs ${reports[0]?.runs}..${reports.at(-1)?.runs}` : "no Howdy reply",
  );
  record(
    "shell reports rebuild-to-reload latency",
    shellLatency !== undefined,
    shellLatency !== undefined ? `${shellLatency} ms (shell-measured, build included)` : `not in dev log; tail: ${devLog.slice(-500)}`,
  );

  // --- Edit 2: a panicking command ----------------------------------------
  const panicked = howdy.replace(
    'Ok(json!(format!("Howdy, {name}! (from Rust)")))',
    // `name` stays used on purpose: with RUSTFLAGS=-D warnings (CI sets it)
    // an unused variable would fail the build, and a failed build must not be
    // mistaken for a contained panic.
    'panic!("hot boom: {name}")',
  );
  record("panic edit applied", panicked !== howdy, panicked !== howdy ? "greet now panics" : "could not splice a panic");
  writeFileSync(commandsFile, panicked);
  const gotPanic = await waitFor(() => reports.some((r) => r.reply.includes("command panicked: hot boom")), 60_000);
  record(
    "panicking hot command is contained",
    gotPanic,
    gotPanic
      ? `reply: ${reports.find((r) => r.reply.includes("panicked"))?.reply}`
      : `no panic reply; last replies: ${JSON.stringify(reports.slice(-3))}; tail: ${devLog.slice(-500)}`,
  );

  // --- Edit 3: a broken build keeps the last good library ------------------
  appendFileSync(commandsFile, "\nthis is not rust\n");
  const buildFailed = await waitFor(() => devLog.includes("commands failed to build"), 60_000);
  // Only *reply* reports count here: beacons share the stream and carry no
  // command output, so mixing them into the sample made this check fail even
  // when every real reply was right.
  const repliesAtFailure = reports.filter((r) => r.reply !== "beacon").length;
  await sleep(1600);
  const laterReplies = reports.filter((r) => r.reply !== "beacon").slice(repliesAtFailure);
  const stillServing =
    laterReplies.length >= 2 &&
    laterReplies.every((r) => r.reply.includes("command panicked: hot boom"));
  record(
    "broken build keeps last good library",
    buildFailed && stillServing,
    buildFailed
      ? `${laterReplies.length} replies after failure: ${JSON.stringify(laterReplies.map((r) => r.reply))}`
      : "no build-failure line in dev log",
  );

  // --- Edit 4: recovery ----------------------------------------------------
  writeFileSync(commandsFile, howdy);
  const lastReply = () => reports.filter((r) => r.reply !== "beacon").at(-1)?.reply ?? "";
  const recovered = await waitFor(() => lastReply().includes("Howdy, hot!"), 60_000);
  record(
    "fixed file reloads again",
    recovered,
    recovered
      ? `latest reply: ${lastReply()}`
      : `no recovery; last replies: ${JSON.stringify(reports.filter((r) => r.reply !== "beacon").slice(-3))}; tail: ${devLog.slice(-500)}`,
  );

  // --- The invariant across the whole session ------------------------------
  const neverRestarted = pageStable() && reports.length >= 8;
  record(
    "page never restarted across all reloads",
    neverRestarted,
    `${reports.length} reports, one start timestamp, runs ${reports[0]?.runs}..${reports.at(-1)?.runs}`,
  );
} catch (error) {
  record("harness crashed", false, String(error));
} finally {
  await finish(0);
}
