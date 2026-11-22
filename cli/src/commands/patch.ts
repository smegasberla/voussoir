/** `voussoir patch`: build the delta a feed can offer for one upgrade. */

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { stringFlag, type Args } from "../args.ts";
import { fail } from "../proc.ts";
import { createPatch } from "../delta.ts";
import { artifactFragment } from "./keys.ts";

export function patch(args: Args): number {
  const [oldPath, newPath] = args.positionals;
  if (!oldPath || !newPath) {
    fail("patch needs two artifacts: voussoir patch <old> <new> --out <file>");
  }
  let oldBytes: Buffer;
  let newBytes: Buffer;
  try {
    oldBytes = readFileSync(oldPath);
    newBytes = readFileSync(newPath);
  } catch (error) {
    fail(`cannot read the artifacts: ${(error as Error).message}`);
  }
  const out = stringFlag(args, "out");
  if (!out) fail("patch needs --out <file>");

  const delta = createPatch(oldBytes, newBytes);
  writeFileSync(out, delta);
  const patchSha = createHash("sha256").update(delta).digest("hex");
  const nextSha = createHash("sha256").update(newBytes).digest("hex");
  const from = stringFlag(args, "from");

  console.log(
    JSON.stringify(
      {
        patch: { sha256: patchSha, ...(from ? { from } : {}) },
        artifact: { sha256: nextSha },
      },
      null,
      2,
    ),
  );

  const keyPath = stringFlag(args, "key");
  if (keyPath) {
    let pem: Buffer;
    try {
      pem = readFileSync(keyPath);
    } catch (error) {
      fail(`cannot read the signing key ${keyPath}: ${(error as Error).message}`);
    }
    const patchSigned = artifactFragment(delta, pem);
    const artifactSigned = artifactFragment(newBytes, pem);
    console.log(
      JSON.stringify(
        {
          patch: { sha256: patchSha, signature: patchSigned.signature, ...(from ? { from } : {}) },
          artifact: { sha256: nextSha, signature: artifactSigned.signature },
        },
        null,
        2,
      ),
    );
    console.log(`\n  signed with the key whose update.public_key is:\n  ${patchSigned.publicKey}`);
  }
  return 0;
}
