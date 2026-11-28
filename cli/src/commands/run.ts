/** `voussoir run --snap <path>`: boot the app once, capture it, exit.
 *
 *  This exists so an agent can see its own work without a human. The shell already
 *  knows how to write a PNG and a JSON dump when `VOUSSOIR_SNAPSHOT` /
 *  `VOUSSOIR_DUMP` are set; this command is the ergonomic wrapper that sets them,
 *  waits for the line the shell prints, and reports the result.
 *
 *  It deliberately does not build. `voussoir run --snap` boots whatever binary is
 *  already there, because the loop it serves is "edit, build, look" and hiding the
 *  build inside the look would make a stale binary look like a wrong layout. The
 *  error message says so when the binary is missing.
 */

import { existsSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { boolFlag, stringFlag, type Args } from "../args.ts";
import { binaryName, project } from "../pipeline.ts";
import { crateDir, type Project } from "../project.ts";

/** Where `voussoir build` put the binary for this project.
 *
 *  Read from the crate's own Cargo.toml rather than assumed: a scaffold names its
 *  package after the app, so `app` is only right for a project someone renamed. */
function binaryPath(p: Project): string {
  const crate = crateDir(p);
  const exe = process.platform === "win32" ? ".exe" : "";
  return join(crate, "target", "release", `${binaryName(crate)}${exe}`);
}

export async function run(args: Args): Promise<number> {
  const snap = stringFlag(args, "snap");
  const dump = stringFlag(args, "dump");
  if (!snap && !dump) {
    console.error(
      "voussoir run: nothing was asked for.\n\n" +
        "  voussoir run --snap out.png        capture the first window\n" +
        "  voussoir run --snap out.png --dump out.json\n\n" +
        "  A capture needs a display; on a headless box use `xvfb-run -a voussoir run --snap out.png`.\n",
    );
    return 1;
  }

  const p = project();
  const binary = binaryPath(p);
  if (!existsSync(binary)) {
    // Say what to run rather than building it here: `voussoir build` is the
    // command that owns building, and a run that silently compiled would report a
    // layout from a binary the caller did not know had changed.
    console.error(
      `voussoir run: no binary at ${binary}\n` +
        `  build it first: voussoir build\n`,
    );
    return 1;
  }

  if (snap) mkdirSync(dirname(resolve(snap)), { recursive: true });
  if (dump) mkdirSync(dirname(resolve(dump)), { recursive: true });

  const env: Record<string, string> = { ...(process.env as Record<string, string>) };
  if (snap) env.VOUSSOIR_SNAPSHOT = resolve(snap);
  if (dump) env.VOUSSOIR_DUMP = resolve(dump);
  // A capture is a one-shot run, so the benchmark hook and any inherited request
  // would be a second thing writing the same files.
  delete env.VOUSSOIR_BENCH;
  delete env.VOUSSOIR_BENCH_CALLS;

  const timeout = Number(stringFlag(args, "timeout") ?? 30_000);
  const proc = Bun.spawn([binary], {
    cwd: p.root,
    env,
    stdout: "pipe",
    stderr: "inherit",
  });

  let line = "";
  const timer = setTimeout(() => proc.kill(), timeout);
  const decoder = new TextDecoder();
  // Line-oriented rather than "read everything then look": the shell exits itself
  // after writing, so the line and the exit race, and reading to completion would
  // sometimes see the exit first and no line at all.
  const reader = proc.stdout.getReader();
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    line += decoder.decode(value, { stream: true });
    if (line.includes("VOUSSOIR_SNAPSHOT ")) break;
  }
  clearTimeout(timer);
  const code = await proc.exited;

  const printed = line.split("\n").find((row) => row.startsWith("VOUSSOIR_SNAPSHOT "));
  if (!printed) {
    console.error(
      `voussoir run: the app exited (${code}) without printing a snapshot line.\n` +
        (code === 0
          ? "  It may have no window, or the page never finished loading.\n"
          : "  Its own error is above. A capture needs a display: try `xvfb-run -a`.\n"),
    );
    return 1;
  }

  console.log(`  ${printed.slice("VOUSSOIR_SNAPSHOT ".length)}`);
  for (const [label, path] of [
    ["png ", snap],
    ["dump", dump],
  ] as const) {
    if (!path) continue;
    const file = Bun.file(resolve(path));
    if (!(await file.exists())) {
      console.error(`  ${label} missing: ${path}`);
      return 1;
    }
    console.log(`  ${label} ${path} (${file.size} bytes)`);
  }

  if (boolFlag(args, "json")) {
    // The dump already is the machine-readable answer; printing it again would be
    // two sources for one fact.
    const path = dump ?? snap;
    if (path) console.log(await Bun.file(resolve(path!)).text());
  }

  return code === 0 ? 0 : 1;
}

/** Keep the export used even when only `build` runs, so a tree-shaking bundler
 *  cannot drop the module's side-effect-free helpers that tests import. */
export { binaryPath };
