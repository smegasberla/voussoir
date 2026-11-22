/** `voussoir build`: bundle the frontend, embed it, compile the shell. */

import { mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import {
  bundle,
  cargoBuild,
  ensureToolchain,
  project,
  writeAssetTable,
} from "../pipeline.ts";
import { boolFlag, stringFlag, type Args } from "../args.ts";
import { checkParity } from "../parity.ts";

export async function build(args: Args): Promise<number> {
  ensureToolchain();
  const p = project();
  const release = !boolFlag(args, "debug");

  console.log(`voussoir build: ${p.config.name}`);

  // The frontend is shared across platforms, so a command registered on only one
  // shell is a runtime failure on the other. Report it here rather than at launch.
  const parity = checkParity(p.root);
  if (!parity.ok) {
    console.warn(`  warning: command parity between desktop and android: ${parity.message}`);
  }
  await bundle(p, true);
  const count = writeAssetTable(p);
  console.log(`  embedded ${count} frontend file(s)`);

  const binary = await cargoBuild(p, release);
  console.log(`\n  ${binary}`);

  if (boolFlag(args, "size")) {
    const { size } = Bun.file(binary);
    console.log(`  ${(size / 1024 / 1024).toFixed(2)} MB`);
  }

  const out = stringFlag(args, "out");
  if (out) {
    await Bun.write(ensureOutDir(out), Bun.file(binary));
    console.log(`  copied to ${out}`);
  }

  return 0;
}

/** Create the parent of `--out` so `--out release/app` works on the first run. */
export function ensureOutDir(out: string, cwd = process.cwd()): string {
  const destination = join(cwd, out);
  mkdirSync(dirname(destination), { recursive: true });
  return destination;
}
