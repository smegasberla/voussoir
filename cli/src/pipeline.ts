/** The one place that knows how a Voussoir project turns into a Rust binary.
 *
 *  `voussoir dev` and `voussoir build` differ in exactly two things: whether the
 *  frontend is bundled to disk or served by Bun, and the cargo profile. Both
 *  funnel through `writeAssetTable` so the Rust side is identical either way.
 */

import { readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { assetTable } from "./assets.ts";
import { crateDir, requireProject, type Project } from "./project.ts";
import { run } from "./proc.ts";

export const OUT_DIR = "dist";

/** Bundle the frontend to `dist/`. Bun handles TS, JSX, CSS, and HTML entries. */
export async function bundle(project: Project, minify: boolean): Promise<void> {
  const outDir = join(project.root, OUT_DIR);
  rmSync(outDir, { recursive: true, force: true });
  const args = ["build", join(project.root, project.config.entry), "--outdir", OUT_DIR, "--target", "browser"];
  if (minify) args.push("--minify");
  await run(["bun", ...args], { cwd: project.root });
}

/** Regenerate `native/src/assets.rs` from the current `dist/`. */
/** Generate the asset table for one crate.
 *
 *  `crate` defaults to the desktop crate; the Android crate passes its own path so
 *  both platforms embed the same bundled frontend from their own `src/assets.rs`.
 */
export function writeAssetTable(project: Project, crate?: string): number {
  const dir = crate ?? crateDir(project);
  const outDir = join(project.root, OUT_DIR);
  const target = join(dir, "src", "assets.rs");
  const { entries, source } = assetTable(outDir, join(dir, "src"));
  writeFileSync(target, source);
  return entries;
}

/** Fail early with a useful message instead of a wall of cargo errors. */
export function ensureToolchain(): void {
  for (const bin of ["bun", "cargo"]) {
    if (!Bun.which(bin)) {
      throw new Error(`\`${bin}\` is not on PATH; install it first`);
    }
  }
  ensureDiskSpace();
}

/** A Rust build needs a few hundred megabytes; running out mid-compile produces
 *  "could not compile <random crate>" errors that look like code problems.
 *
 *  The check follows the volume the build writes to: `CARGO_TARGET_DIR` when it
 *  is set, else the working directory. Asking `df` about `/` was wrong whenever
 *  the project lived on another mount, and `df` does not exist on Windows, where
 *  spawning it used to throw and fail every build. There the check is skipped.
 */
export function ensureDiskSpace(needMb = 1500): void {
  if (!Bun.which("df")) return;
  const target = process.env.CARGO_TARGET_DIR
    ? resolve(process.env.CARGO_TARGET_DIR)
    : process.cwd();
  let available: number;
  try {
    const free = Bun.spawnSync(["df", "-Pm", target]);
    if (free.exitCode !== 0) return;
    const line = free.stdout.toString().trim().split("\n")[1] ?? "";
    available = Number(line.split(/\s+/)[3]);
  } catch {
    // A platform without a runnable `df` must not fail the build over a check
    // whose whole job is a friendlier error.
    return;
  }
  if (Number.isFinite(available) && available < needMb) {
    throw new Error(
      `only ${available} MB free where the build runs, need about ${needMb} MB.\n` +
        `Free space, or point CARGO_TARGET_DIR at a larger volume.`,
    );
  }
}

export function project(): Project {
  const p = requireProject();
  if (!Bun.which("cargo")) throw new Error("`cargo` is not on PATH; install Rust first");
  return p;
}

/** Build the Rust binary, optionally with the release profile. */
export async function cargoBuild(project: Project, release: boolean): Promise<string> {
  const crate = crateDir(project);
  const args = ["cargo", "build"];
  if (release) args.push("--release");
  await run(args, { cwd: crate });
  // Windows suffixes cargo binaries with .exe; stat'ing the bare name reports
  // "cargo finished but ... is missing" on every successful build there.
  const suffix = process.platform === "win32" ? ".exe" : "";
  const bin = join(crate, "target", release ? "release" : "debug", `${binaryName(crate)}${suffix}`);
  if (!statSync(bin, { throwIfNoEntry: false })) {
    throw new Error(`cargo finished but ${bin} is missing`);
  }
  return bin;
}

/** The crate's binary name, read from Cargo.toml to avoid guessing. */
function binaryName(crate: string): string {
  const manifest = readFileSync(join(crate, "Cargo.toml"), "utf8");
  const named = /\[\[bin\]\][\s\S]*?name\s*=\s*"([^"]+)"/.exec(manifest);
  if (named) return named[1]!;
  const pkg = /\[package\][\s\S]*?name\s*=\s*"([^"]+)"/.exec(manifest);
  if (!pkg) throw new Error("could not read package name from native/Cargo.toml");
  return pkg[1]!;
}
