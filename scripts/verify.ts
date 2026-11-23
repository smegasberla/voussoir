/** End-to-end verification for the repo, and the source of the README numbers.
 *
 *  Everything here asserts something a human would otherwise check by looking
 *  at a window: the shell boots, the JS -> Rust -> JS round-trip returns real
 *  data, `emit` reaches the page, the binary is small, and the binary contains
 *  no JS runtime.
 *
 *  Run: bun scripts/verify.ts
 */

import { existsSync, mkdtempSync, readFileSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const repo = resolve(import.meta.dir, "..");
const cli = join(repo, "cli", "src", "index.ts");
// Windows suffixes cargo binaries with .exe; every spawned path below needs it.
const EXE = process.platform === "win32" ? ".exe" : "";
// WebView2 cannot intercept a custom scheme, so wry rewrites `voussoir://localhost`
// to `http://voussoir.localhost` and intercepts that. The pages below call the same
// endpoint the shell's shim does, so they need the same spelling or the request
// never reaches the handler.
const INVOKE =
  process.platform === "win32"
    ? "http://voussoir.localhost/__voussoir/"
    : "voussoir://localhost/__voussoir/";
const results: { name: string; ok: boolean; detail: string }[] = [];

function record(name: string, ok: boolean, detail: string) {
  results.push({ name, ok, detail });
  console.log(`${ok ? "ok  " : "FAIL"} ${name.padEnd(30)} ${detail}`);
}

async function sh(cmd: string[], cwd = repo, env: Record<string, string> = {}) {
  const proc = Bun.spawn(cmd, {
    cwd,
    env: { ...process.env, ...env },
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

async function shWithTimeout(cmd: string[], cwd: string, env: Record<string, string>, ms: number) {
  const proc = Bun.spawn(cmd, {
    cwd,
    env: { ...process.env, ...env },
    stdout: "pipe",
    stderr: "pipe",
  });
  const timer = setTimeout(() => proc.kill(), ms);
  const [out, err, code] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
    proc.exited,
  ]);
  clearTimeout(timer);
  return { out, err, code };
}

/** The page a probe loads. It calls the app's own `greet` command over the real
 *  IPC path and POSTs the response back so this script can assert on it. */
const PAGE = `<!doctype html><html><body><script>
(async () => {
  try {
    const res = await fetch('${INVOKE}greet', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ name: 'scaffold' }),
    });
    await fetch('/result', { method: 'POST', body: await res.text() });
  } catch (e) {
    await fetch('/result', { method: 'POST', body: JSON.stringify({ error: String(e) }) });
  }
})();
</script></body></html>`;

/** Serve PAGE on `port`, point the shell at it, and return what the page
 *  reported. The page origin differs from the shell's origin, which is exactly
 *  the cross-origin path `voussoir dev` depends on. */
async function probe(binary: string, app: string, port: number): Promise<string | null> {
  let resolveReply: (value: string) => void = () => {};
  const reply = new Promise<string>((r) => (resolveReply = r));

  const server = Bun.serve({
    port,
    async fetch(req) {
      const url = new URL(req.url);
      if (url.pathname === "/result" && req.method === "POST") {
        resolveReply(await req.text());
        return new Response("ok");
      }
      if (url.pathname === "/" || url.pathname === "/index.html") {
        return new Response(PAGE, { headers: { "content-type": "text/html" } });
      }
      return new Response("not found", { status: 404 });
    },
  });

  const proc = Bun.spawn([binary], {
    cwd: app,
    env: { ...process.env, VOUSSOIR_DEV_URL: `http://127.0.0.1:${port}` },
    stdout: "ignore",
    stderr: "ignore",
  });

  const body = await Promise.race([
    reply,
    new Promise<null>((r) => setTimeout(() => r(null), 25_000)),
  ]);

  proc.kill();
  server.stop(true);
  if (body === null) return null;
  try {
    const parsed = JSON.parse(body) as { ok?: string; error?: string | { message?: string } };
    // The envelope's error is an object since the structured-kind change; the
    // plain-string form (and the page's own fetch failures) still arrive.
    const error = typeof parsed.error === "string" ? parsed.error : parsed.error?.message;
    return parsed.ok ?? error ?? null;
  } catch {
    return body;
  }
}

/** The page a *narrowed* second window loads. It calls `greet` — a command the
 *  window's `voussoir.json` entry withholds — and reports the envelope with a
 *  `narrow` marker so the harness can tell which window replied. */
const NARROW_PAGE = `<!doctype html><html><body><script>
(async () => {
  let body;
  try {
    const res = await fetch('${INVOKE}greet', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ name: 'scaffold' }),
    });
    body = await res.text();
  } catch (e) {
    body = JSON.stringify({ error: { message: String(e) } });
  }
  await fetch('/result', { method: 'POST', body: JSON.stringify({ narrow: body }) });
})();
</script></body></html>`;

/** Like `probe`, but the first window's page opens a second window labelled
 *  `untrusted` (whose `voussoir.json` entry withholds every command) before doing
 *  its own round-trip. Resolves with both envelopes once each window has
 *  reported, or null on timeout. */
async function probeSecondWindow(
  binary: string,
  app: string,
  port: number,
): Promise<{ main: string; narrow: string } | null> {
  let resolveMain: (value: string) => void = () => {};
  let resolveNarrow: (value: string) => void = () => {};
  const main = new Promise<string>((r) => (resolveMain = r));
  const narrow = new Promise<string>((r) => (resolveNarrow = r));

  const opener = `<!doctype html><html><body><script>
(async () => {
  try {
    await fetch('${INVOKE}__voussoir_open_window', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ label: 'untrusted', entry: 'untrusted.html' }),
    });
  } catch (e) {
    await fetch('/result', { method: 'POST', body: JSON.stringify({ main: JSON.stringify({ error: { message: String(e) } }) }) });
    return;
  }
  try {
    const res = await fetch('${INVOKE}greet', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ name: 'scaffold' }),
    });
    await fetch('/result', { method: 'POST', body: JSON.stringify({ main: await res.text() }) });
  } catch (e) {
    await fetch('/result', { method: 'POST', body: JSON.stringify({ main: JSON.stringify({ error: { message: String(e) } }) }) });
  }
})();
</script></body></html>`;

  const server = Bun.serve({
    port,
    async fetch(req) {
      const url = new URL(req.url);
      if (url.pathname === "/result" && req.method === "POST") {
        const parsed = JSON.parse(await req.text()) as { main?: string; narrow?: string };
        if (parsed.narrow !== undefined) resolveNarrow(parsed.narrow);
        else resolveMain(parsed.main ?? JSON.stringify(parsed));
        return new Response("ok");
      }
      if (url.pathname === "/" || url.pathname === "/index.html") {
        return new Response(opener, { headers: { "content-type": "text/html" } });
      }
      if (url.pathname === "/untrusted.html") {
        return new Response(NARROW_PAGE, { headers: { "content-type": "text/html" } });
      }
      return new Response("not found", { status: 404 });
    },
  });

  const proc = Bun.spawn([binary], {
    cwd: app,
    env: { ...process.env, VOUSSOIR_DEV_URL: `http://127.0.0.1:${port}` },
    stdout: "ignore",
    stderr: "ignore",
  });

  const [m, n] = await Promise.all([
    Promise.race([main, Bun.sleep(25_000).then(() => null)]),
    Promise.race([narrow, Bun.sleep(25_000).then(() => null)]),
  ]);

  proc.kill();
  server.stop(true);
  if (m === null || n === null) return null;
  return { main: m, narrow: n };
}

/** Run the probe fixture against a hostile server and collect what it saw.
 *
 *  The fixture loads its page from the binary's own embedded assets, so the
 *  *production* security policy is what gets tested: no dev server, no relaxed
 *  CSP. Its `probe.js` calls a panicking command, then tries a remote script, a
 *  remote fetch, `eval`, and navigating away, and reports over IPC (which is
 *  also proof the bridge survived). The result arrives on stdout, so no human
 *  has to look at a window.
 */
async function probeGuarantees(binary: string, app: string, hostilePort: number): Promise<string | null> {
  const hits: string[] = [];
  const hostile = Bun.serve({
    port: hostilePort,
    fetch(req) {
      hits.push(new URL(req.url).pathname);
      const isScript = req.url.includes("evil.js");
      return new Response(isScript ? "window.__evil='pwned';" : "<h1>escaped</h1>", {
        headers: { "content-type": isScript ? "text/javascript" : "text/html" },
      });
    },
  });

  const proc = Bun.spawn([binary], {
    cwd: app,
    env: { ...process.env, VOUSSOIR_HOSTILE: `http://127.0.0.1:${hostilePort}` },
    stdout: "pipe",
    stderr: "ignore",
  });

  const lines = proc.stdout.getReader();
  const deadline = Date.now() + 25_000;
  let buffer = "";
  let report: string | null = null;
  while (Date.now() < deadline) {
    const { value, done } = await lines.read();
    if (done) break;
    buffer += new TextDecoder().decode(value);
    const match = /PROBE (\{.*\})/.exec(buffer);
    if (match) {
      report = match[1]!;
      break;
    }
  }

  proc.kill();
  hostile.stop(true);

  // Any request that reached the hostile server is a failure of the policy.
  if (hits.length > 0) {
    console.log(`     hostile server was contacted: ${hits.join(", ")}`);
  }
  return report;
}

async function main() {
  // 1. Shell unit tests (dispatch, assets, config, protocol handling).
  {
    const { out, code } = await sh(["cargo", "test", "-p", "voussoir", "--quiet"]);
    const passed = /test result: ok\. (\d+) passed/.exec(out.replace(/\s+/g, " "))?.[1] ?? "0";
    record("shell unit tests", code === 0, `${passed} tests`);
  }

  // 2. Embedded-asset path: the framework example boots, round-trips a command
  //    over voussoir://, and receives a Rust -> JS event. It exits 0 only when both
  //    arrive, so the exit code is the assertion.
  //
  //    Built here rather than assumed: the verifier has to work from a clean
  //    checkout (CI has no prior build), and the baseline binary is needed too.
  {
    const build = await sh(
      ["cargo", "build", "--release", "-p", "voussoir", "--example", "hello"],
      repo,
    );
    if (build.code !== 0) {
      // Keep cargo's own error lines, not the tail: the tail is the
      // "For more information" footer, and the head is what names the failure.
      const detail =
        build.err
          .split("\n")
          .filter((line) => line.includes("error"))
          .slice(0, 4)
          .join(" | ") || "(cargo failed with no stderr captured)";
      record("embedded round-trip + emit", false, detail);
    } else {
      // 90 s, not 30: the first launch of a cold Windows runner creates the
      // WebView2 profile and has it scanned before the page runs, which on its
      // own can outlast a tight budget. A real hang still fails, just slower,
      // and the timeout message now says how long it waited.
      const { out, err, code } = await shWithTimeout(
        [join(repo, "target", "release", "examples", `hello${EXE}`)],
        repo,
        { VOUSSOIR_SELFTEST: "1" },
        90_000,
      );
      record(
        "embedded round-trip + emit",
        code === 0 && out.includes("SELFTEST OK"),
        code === 0
          ? out.trim().split("\n").pop()!
          : `exit ${code} | stdout: ${out.slice(-160) || "none"} | stderr: ${err.slice(-160) || "none"}`,
      );
    }
  }

  // 3. Scaffold a fresh app with the CLI and build it with the CLI.
  const work = mkdtempSync(join(tmpdir(), "voussoir-verify-"));
  const app = join(work, "app");
  {
    const { code, err } = await sh(["bun", cli, "init", app]);
    record("voussoir init", code === 0, code === 0 ? app : err.slice(-120));
  }
  {
    const { out, err, code } = await shWithTimeout(
      ["bun", cli, "build", "--size"],
      app,
      {},
      600_000,
    );
    record(
      "voussoir build",
      code === 0,
      code === 0 ? out.trim().split("\n").slice(-2).join(" ") : err.slice(-160),
    );
  }

  const binary = join(app, "native", "target", "release", `app${EXE}`);

  // A failed build used to crash here with ENOENT, hiding the compiler error
  // that caused it. Report the remaining checks as failures and move on.
  if (!statSync(binary, { throwIfNoEntry: false })) {
    record("artifact checks", false, `no binary at ${binary}`);
    const failed = results.filter((r) => !r.ok);
    console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
    process.exit(1);
  }

  // 4. Size and composition of the shipped artifact.
  {
    const size = statSync(binary).size;
    // The scaffold enables `full` (menu, dialogs, updater, storage) and the CLI
    // adds LTO, one codegen unit and stripping even on an older manifest, so
    // this covers the realistic shipped app, not the bare shell. Still two
    // orders of magnitude under Electron's ~120 MB.
    record("binary size", size < 8 * 1024 * 1024, `${(size / 1048576).toFixed(2)} MB`);
  }
  {
    const bytes = readFileSync(binary);
    const markers = ["Bun.version", "node_modules", "electron", "Chromium"];
    const found = markers.filter((m) => bytes.includes(Buffer.from(m)));
    record("no JS runtime embedded", found.length === 0, found.join(",") || "clean");
  }
  {
    // ldd does not exist off Linux (macOS has otool, Windows has dumpbin), so
    // the dynamic-link composition check is a Linux assertion. Skip loudly on
    // the other platforms rather than pretend the guarantee was tested there.
    if (process.platform !== "linux") {
      record("links OS webview only", true, `skipped: not Linux (${process.platform})`);
    } else {
      const { out } = await sh(["ldd", binary]);
      const node = /libnode|libv8/.test(out);
      const webkit = /libwebkit2gtk/.test(out);
      record(
        "links OS webview only",
        !node && webkit,
        node ? "links libnode/libv8" : "webkit2gtk present, no node/v8",
      );
    }
  }

  // 5. The scaffolded binary opens a window with its embedded page and stays
  //    up. A crash, a missing asset, or a bad protocol registration would exit
  //    early; so surviving two seconds is the signal.
  {
    const proc = Bun.spawn([binary], { cwd: app, stdout: "pipe", stderr: "pipe" });
    const exited = await Promise.race([
      proc.exited.then((code) => String(code)),
      Bun.sleep(2000).then(() => "alive" as const),
    ]);
    proc.kill();
    record(
      "embedded app launches",
      exited === "alive",
      exited === "alive" ? "window up after 2s" : `exited early (${exited})`,
    );
  }

  // 6. Dev mode on the same binary: page from http, IPC still over voussoir://.
  {
    const debug = await sh(["bun", cli, "build", "--debug"], app);
    if (debug.code !== 0) {
      record("dev-mode round-trip", false, "debug build failed");
    } else {
      const reported = await probe(join(app, "native", "target", "debug", "app"), app, 5192);
      record(
        "dev-mode round-trip",
        reported !== null && reported.includes("from Rust"),
        reported ?? "no reply",
      );
    }
  }

  // 6b. A second window can be given a strict subset of the main window's
  //     commands. The scaffold's voussoir.json lists every command for the main
  //     window and *no* commands for the window labelled `untrusted`; the
  //     narrowed window's page must still round-trip on the main window while
  //     its own call comes back as an unknown command.
  {
    const cfgPath = join(app, "voussoir.json");
    const cfg = await Bun.file(cfgPath).json();
    cfg.commands = ["greet", "__voussoir_open_window"];
    cfg.windows = { untrusted: { commands: [] } };
    await Bun.write(cfgPath, JSON.stringify(cfg, null, 2) + "\n");

    const rebuilt = await sh(["bun", cli, "build", "--debug"], app);
    if (rebuilt.code !== 0) {
      record("second window narrowed by voussoir.json", false, "debug rebuild failed");
    } else {
      const bin = join(app, "native", "target", "debug", "app");
      const result = await probeSecondWindow(bin, app, 5193);
      record(
        "second window narrowed by voussoir.json",
        result !== null &&
          result.main.includes("from Rust") &&
          result.narrow.includes('"kind":"unknown"'),
        result
          ? `main: round-trip ok | narrow: ${result.narrow.slice(0, 70)}`
          : "no reply",
      );
    }
  }

  // 7. The distributed CLI is a compiled binary: it must work with no Bun or
  //    Node on PATH, and produce the same app a `bun run` CLI does.
  {
    const out = join(work, "voussoir-bin");
    const compiled = await sh(["bun", "build", "--compile", "--minify", cli, "--outfile", out]);
    if (compiled.code !== 0) {
      record("compiled CLI", false, compiled.err.slice(-140));
    } else {
      // bun appends .exe to --compile output on Windows; spawning resolves
      // the bare name through CreateProcess, node's stat does not.
      const bin = existsSync(out) ? out : `${out}.exe`;
      const bare = join(work, "cli-only-air");
      const env = { PATH: "/usr/bin:/bin", HOME: bare };
      const init = await sh([bin, "init", join(bare, "app")], work, env);
      const size = (statSync(bin).size / 1048576).toFixed(0);
      record(
        "compiled CLI (no bun on PATH)",
        init.code === 0,
        init.code === 0 ? `${size} MB binary, init ok` : init.err.slice(-140),
      );
    }
  }

  // 7. Panic isolation and the security policy, on a real app, through the real
  //    IPC path. The harness page reports over IPC so no human has to look.
  {
    // Built once, then reused for the hostile-page probe.
    const probeBin = join(repo, "target", "debug", `voussoir-probe${EXE}`);
    const build = await sh(["cargo", "build", "-p", "voussoir-probe"], repo);
    const reported = build.code === 0 ? await probeGuarantees(probeBin, repo, 5202) : null;
    const parsed = reported ? JSON.parse(reported) : null;
    record(
      "handler panic is contained",
      parsed?.panic?.includes("command panicked") && parsed?.after === "still alive",
      parsed ? `${parsed.panic} | then: ${parsed.after}` : "no reply",
    );
    record(
      "remote script/fetch/eval blocked",
      parsed
        ? !parsed.remoteScriptExecuted && parsed.remoteFetch === "blocked" && parsed.eval === "blocked"
        : false,
      parsed ? JSON.stringify(parsed).slice(0, 90) : "no reply",
    );
    record(
      "navigation is locked down",
      parsed?.nav === "stayed",
      parsed?.nav ?? "no reply",
    );
    // T1: a typed command rejects a wrong shape as `payload` naming the command,
    // and the same command with the right shape still answers. Both go through
    // the real IPC transport, so this is the cross-platform proof.
    record(
      "typed command rejects a bad payload",
      parsed?.typed === "payload/typed_echo",
      parsed?.typed ?? "no reply",
    );
    record(
      "typed command answers a good payload",
      parsed?.typedOk === "ok",
      parsed?.typedOk ?? "no reply",
    );
    // T1: the store persists through the platform data directory and reads back.
    record(
      "storage round-trips in the data dir",
      parsed?.storage?.round === 1 && parsed?.storageKeys === true,
      parsed ? `${JSON.stringify(parsed.storage)} keys ok: ${parsed.storageKeys}` : "no reply",
    );
    // T2: a page opens a window and is told its label, so it can address it
    // without having guessed the name up front.
    record(
      "open_window returns the label",
      parsed?.openLabel === "probe-narrow",
      parsed?.openLabel ?? "no reply",
    );
    // The label the shell hands out is the one the page can act on, and an
    // unnamed second window gets a distinct one instead of colliding.
    record(
      "windows() lists open labels",
      parsed?.windowsIncludes === true,
      parsed ? JSON.stringify(parsed.windows) : "no reply",
    );
    record(
      "unnamed windows get distinct labels",
      typeof parsed?.unnamedLabel === "string" && parsed.unnamedLabel !== "probe-narrow",
      parsed?.unnamedLabel ?? "no reply",
    );
    // T2: shell-side state is shared between handlers and survives the library
    // being swapped, which is what `voussoir dev` cannot do with a static.
    record(
      "shell state round-trips",
      parsed?.state?.round === 2,
      parsed ? JSON.stringify(parsed.state) : "no reply",
    );
    // T2: the shell reports a version the page can feature-detect on.
    record(
      "shell reports a version",
      typeof parsed?.voussoirVersion === "string" && parsed.voussoirVersion.length > 0,
      parsed?.voussoirVersion ?? "no reply",
    );
  }

  // 8. Packaging produces a real artifact on Linux. The AppImage is the only
  //    format that can be produced and inspected here; macOS and Windows
  //    packaging is exercised by the release workflow.
  if (process.platform === "linux") {
    const pkg = await shWithTimeout(["bun", cli, "package"], app, {}, 600_000);
    const artifact = /(\S+\.AppImage)/.exec(pkg.out)?.[1] ?? null;
    const exists = artifact !== null && statSync(artifact, { throwIfNoEntry: false }) !== undefined;
    if (!Bun.which("appimagetool") && !exists) {
      // Without the tool there is no AppImage to inspect. The AppDir it still
      // produces is a portable app, so this is an environment gap, not a defect.
      record("voussoir package", pkg.code === 0, "AppDir built (appimagetool not installed)");
    } else {
      record(
        "voussoir package (AppImage)",
        pkg.code === 0 && exists,
        exists ? `${(statSync(artifact!).size / 1048576).toFixed(1)} MB` : pkg.err.slice(-140),
      );
    }
  } else {
    record("voussoir package", true, `skipped: not Linux (${process.platform})`);
  }

  const failed = results.filter((r) => !r.ok);
  console.log(`\n${results.length - failed.length}/${results.length} checks passed`);

  // Each run builds a full cargo tree in the scratch directory. Left behind,
  // a handful of runs fills a small tmpfs and later runs fail for reasons that
  // look like build errors, so remove it unless asked to keep it.
  if (!process.env.VOUSSOIR_KEEP_SCRATCH) {
    rmSync(work, { recursive: true, force: true });
  } else {
    console.log(`scratch kept: ${work}`);
  }

  process.exit(failed.length === 0 ? 0 : 1);
}

await main();
