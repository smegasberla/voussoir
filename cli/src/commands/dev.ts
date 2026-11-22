/** `voussoir dev`: Bun dev server with HMR + the Rust shell pointed at it.
 *
 *  The dev server runs in this process, not as a child. That matters twice:
 *  `voussoir` is distributed as a compiled binary with no `.ts` files on disk, so
 *  spawning `bun dev-server.ts` would break the installed CLI; and an
 *  in-process server means one Ctrl-C stops everything.
 *
 *  HMR comes from importing the project's entry HTML directly. Bun serves the
 *  transformed HTML and watches everything it pulls in, so edits to TS/CSS
 *  reload in the page without touching Rust. `bun --hot` on the CLI process is
 *  what makes the import re-evaluate after an edit.
 *
 *  Rust commands hot-reload through a second loop: this process watches
 *  `native/commands/src`, rebuilds *only* that crate (`--manifest-path`, so a
 *  shell recompile never enters the loop), and publishes each successful build
 *  as `target/voussoir-hot/commands-<n>.<ext>` plus a `latest.json` pointer. The
 *  shell polls that pointer and swaps the library in without restarting. A
 *  failed build publishes nothing, so the running shell keeps the last good
 *  generation and the compiler error is the feedback.
 */

import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, renameSync, statSync, type Dirent, writeFileSync } from "node:fs";
import { join } from "node:path";
import { spawn } from "../proc.ts";
import { writeAssetTable } from "../pipeline.ts";
import { crateDir, requireProject } from "../project.ts";
import { boolFlag, stringFlag, type Args } from "../args.ts";

export async function dev(args: Args): Promise<number> {
  const project = requireProject();
  const port = parsePort(stringFlag(args, "port"));
  const crate = crateDir(project);
  // The shell reads this itself; the CLI only has to hand it over. Turning it
  // off explicitly matters when a shell profile already exports it for every
  // cargo invocation, which would otherwise make `voussoir dev` verbose by accident.
  const verbose = boolFlag(args, "verbose");

  if (!Bun.which("cargo")) {
    console.error("voussoir: `cargo` is not on PATH; install Rust first");
    return 1;
  }

  // The shell embeds a table so `cargo check` works while editing Rust; in dev
  // the webview never reads it.
  writeAssetTable(project);

  const server = await startServer(project.root, project.config.entry, port);

  // A frontend-only (`init --mobile`) scaffold has no `native/` crate, and a
  // scaffold from before hot reload has no `hot-reload` feature to enable. Both
  // run the old way, just without the command loop; reading the manifest
  // unguarded used to crash the whole dev server on the former.
  const manifest = join(crate, "Cargo.toml");
  const supportsHotReload = hotSupported(crate);
  if (!existsSync(manifest)) {
    console.log(
      "  no Rust crate to watch here (frontend-only scaffold); native changes need `voussoir build`",
    );
  }
  const hotDir = join(crate, "target", "voussoir-hot");
  const artifact = join(crate, "target", "debug", dylibName());

  let initial: { gen: number } | null = null;
  let stopWatching = () => {};
  const env: Record<string, string> = devEnv(port, verbose);

  if (verbose) {
    // Said once, so the extra lines below are not mistaken for a problem.
    console.log("  verbose: reloads, slow commands and refused navigations are recorded");
  }

  if (supportsHotReload) {
    // Generation 1 exists before the shell starts, so its first load is
    // deterministic instead of racing this process's own build.
    initial = await rebuild(crate, hotDir, artifact, 1);
    env.VOUSSOIR_HOT_LIB = initial ? pointerPath(hotDir, initial.gen) : artifact;
    env.VOUSSOIR_HOT_DIR = hotDir;
    stopWatching = watchCommands(crate, hotDir, artifact, initial?.gen ?? 0);
  } else {
    console.log("  (native/Cargo.toml has no hot-reload feature; Rust edits need a restart)");
  }

  // `--features` only for a scaffold that declares the feature. An old
  // scaffold would otherwise fail before it starts: cargo errors on the flag.
  const shell = spawn(
    featuresFor(supportsHotReload),
    { cwd: crate, env },
  );

  const shutdown = () => {
    stopWatching();
    server.stop(true);
    shell.kill();
  };
  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);

  const code = await shell.exited;
  stopWatching();
  server.stop(true);
  return code;
}

/** The environment the shell is launched with.
 *
 *  Exported so a test can assert the two flags this decides: `VOUSSOIR_DEV_URL` must
 *  point at this port, and `VOUSSOIR_VERBOSE` must be set *either way*. Leaving it
 *  unset when off would inherit a developer's shell export and silently turn
 *  diagnostics on for everybody's `voussoir dev`.
 */
export function devEnv(port: number, verbose: boolean, base: Record<string, string | undefined> = process.env): Record<string, string> {
  return {
    ...base,
    VOUSSOIR_DEV_URL: `http://127.0.0.1:${port}`,
    VOUSSOIR_VERBOSE: verbose ? "1" : "0",
  } as Record<string, string>;
}

/** The shell launch, feature flags included only when the scaffold has them.
 *
 *  Exported so a test can pin the difference: a pre-hot-reload scaffold must run
 *  plain `cargo run`, while a current one asks for `--features hot-reload`.
 *  Devtools belongs here, not in `full`: development wants the inspector, and a
 *  release build should not carry it. An old scaffold keeps its old behavior
 *  (no flags), which also means no devtools until it adds the feature.
 */
export function featuresFor(supportsHotReload: boolean): string[] {
  return supportsHotReload
    ? ["cargo", "run", "--features", "hot-reload,devtools"]
    : ["cargo", "run"];
}

/** The platform's cdylib filename for the `commands` lib target. */
function dylibName(): string {
  const extension =
    process.platform === "win32" ? "dll" : process.platform === "darwin" ? "dylib" : "so";
  return process.platform === "win32" ? `commands.${extension}` : `libcommands.${extension}`;
}

function pointerPath(hotDir: string, gen: number): string {
  const name = dylibName().replace(/(\.[^.]+)$/, `-${gen}$1`);
  return join(hotDir, name);
}

/** Rebuild the commands crate alone and publish it as the next generation.
 *
 *  Returns `null` on failure — which is the feature, not an error state: no
 *  pointer is written, so the running shell keeps serving the last good
 *  library and cargo's own output (inherited stdio) is the report. */
async function rebuild(
  crate: string,
  hotDir: string,
  artifact: string,
  gen: number,
): Promise<{ gen: number } | null> {
  const started = Date.now();
  const build = spawn(["cargo", "build", "--manifest-path", "commands/Cargo.toml"], { cwd: crate });
  const code = await build.exited;
  if (code !== 0) {
    console.error("voussoir: commands failed to build — keeping the last good build");
    return null;
  }
  if (!existsSync(artifact)) {
    console.error(`voussoir: build succeeded but produced no ${artifact}`);
    return null;
  }

  mkdirSync(hotDir, { recursive: true });
  const target = pointerPath(hotDir, gen);
  // Copy and rename via temporary names: the shell polls these files, and a
  // rename is atomic where a plain write is not.
  const artifactTmp = `${target}.tmp`;
  copyFileSync(artifact, artifactTmp);
  renameSync(artifactTmp, target);
  const pointerTmp = join(hotDir, "latest.json.tmp");
  // `builtAt` is the *start* of the build, so the shell's report is the true
  // rebuild-to-reload latency, build included.
  writeFileSync(pointerTmp, JSON.stringify({ gen, path: target, builtAt: started }));
  renameSync(pointerTmp, join(hotDir, "latest.json"));

  console.log(`  commands rebuilt in ${Date.now() - started} ms (gen ${gen})`);
  return { gen };
}

/** Watch the commands crate's sources; rebuild on change until the tree stops
 *  moving (an edit saved mid-build triggers one more pass).
 *
 *  A periodic mtime walk rather than fs.watch: it is recursive without
 *  platform quirks, cheap for a crate this size, and it cannot miss a change
 *  that an editor's write-and-rename dance would hide from a watcher. */
function watchCommands(
  crate: string,
  hotDir: string,
  artifact: string,
  firstGen: number,
): () => void {
  const source = join(crate, "commands", "src");
  let known = snapshot(source);
  let gen = firstGen;
  let building = false;

  const timer = setInterval(async () => {
    if (building) return;
    building = true;
    try {
      for (;;) {
        const current = snapshot(source);
        const changed =
          current.size !== known.size ||
          [...current].some(([file, mtime]) => known.get(file) !== mtime);
        if (!changed) break;
        known = current;
        const result = await rebuild(crate, hotDir, artifact, gen + 1);
        if (result) gen = result.gen;
      }
    } finally {
      building = false;
    }
  }, 300);

  return () => clearInterval(timer);
}

/** Every `.rs` file under `dir` with its mtime; missing dir is an empty map.
 *
 *  The poller runs inside `setInterval`, where a throw is an uncaught exception
 *  that kills the dev process. A file deleted between the directory listing and
 *  the `stat` (an editor swap file, `git checkout`) is normal, so a vanished
 *  entry is skipped and an unreadable directory ends that branch of the walk.
 */
export function snapshot(dir: string): Map<string, number> {
  const found = new Map<string, number>();
  const walk = (current: string) => {
    let entries: Dirent[];
    try {
      entries = readdirSync(current, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      const full = join(current, entry.name);
      if (entry.isDirectory()) {
        walk(full);
      } else if (entry.name.endsWith(".rs")) {
        try {
          found.set(full, statSync(full).mtimeMs);
        } catch {
          // Gone between the listing and this stat; not a reason to crash.
        }
      }
    }
  };
  if (existsSync(dir)) walk(dir);
  return found;
}

/** Import the entry HTML and serve it. Bun's `development: true` gives the
 *  transform pipeline plus the HMR client; it is zero-config by design. */
async function startServer(root: string, entry: string, port: number) {
  let page: unknown;
  try {
    page = (await import(join(root, entry))).default;
  } catch (error) {
    console.error(`voussoir: could not load ${entry}: ${(error as Error).message}`);
    process.exit(1);
  }
  if (page === undefined) {
    console.error(`voussoir: ${entry} has no default export after bundling`);
    process.exit(1);
  }

  try {
    const server = Bun.serve({
      port,
      development: true,
      routes: {
        "/": page as never,
        [`/${entry}`]: page as never,
      },
      fetch(req) {
        return new Response(`not found: ${new URL(req.url).pathname}`, { status: 404 });
      },
    });

    console.log(`  dev server http://127.0.0.1:${server.port}/`);
    return server;
  } catch (error) {
    // Bun throws raw EADDRINUSE, which surfaced as a stack trace instead of the
    // one thing the user needs to know: pick another port.
    throw new Error(serveErrorText(port, error));
  }
}

/** The user-facing text for a port Bun would not listen on. */
export function serveErrorText(port: number, error: unknown): string {
  const text = error instanceof Error ? error.message : String(error);
  if (/EADDRINUSE|address already in use/i.test(text)) {
    return `port ${port} is already in use; try \`--port <another number>\``;
  }
  return `could not start the dev server on port ${port}: ${text}`;
}

/** Parse `--port`, refusing instead of handing Bun a value it will throw on. */
export function parsePort(raw: string | undefined): number {
  const text = (raw ?? "5177").trim();
  if (!/^[0-9]+$/.test(text)) {
    throw new Error(`\`--port ${raw}\` is not a port number; give a whole number from 1 to 65535`);
  }
  const port = Number(text);
  if (port < 1 || port > 65535) {
    throw new Error(`\`--port ${raw}\` is out of range; give a whole number from 1 to 65535`);
  }
  return port;
}

/** Whether the crate at `crate` opted into the reload loop.
 *
 *  A `--mobile` scaffold has no crate at all, so the read used to throw and take
 *  the whole dev server down; "no manifest" is simply "no hot reload".
 */
export function hotSupported(crate: string): boolean {
  const manifest = join(crate, "Cargo.toml");
  return existsSync(manifest) && readFileSync(manifest, "utf8").includes("hot-reload");
}
