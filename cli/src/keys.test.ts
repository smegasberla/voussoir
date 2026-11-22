import { describe, expect, test } from "bun:test";
import { createPublicKey, verify } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { artifactFragment, publicKeyBase64 } from "./commands/keys.ts";
import { keygen } from "./commands/keys.ts";
import { parseArgs } from "./args.ts";

function tempDir(): string {
  return mkdtempSync(join(tmpdir(), "voussoir-keys-"));
}

describe("release signing", () => {
  test("a generated key pairs with the printed public key", () => {
    const dir = tempDir();
    try {
      const out = join(dir, "release.pem");
      const log = console.log;
      let printed = "";
      console.log = (...parts: unknown[]) => {
        printed += parts.join(" ") + "\n";
      };
      try {
        keygen(parseArgs(["keygen", "--out", out]));
      } finally {
        console.log = log;
      }
      const pem = readFileSync(out);
      const base64 = publicKeyBase64(pem);
      expect(printed).toContain(base64);
      // The printed key is the raw 32 bytes, not the SPKI wrapper.
      expect(Buffer.from(base64, "base64").length).toBe(32);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("the fragment verifies with the public key it names", () => {
    const dir = tempDir();
    try {
      const out = join(dir, "release.pem");
      keygen(parseArgs(["keygen", "--out", out]));
      const pem = readFileSync(out);
      const artifact = Buffer.from("shell binary bytes");
      const fragment = artifactFragment(artifact, pem);
      expect(fragment.sha256).toHaveLength(64);
      const publicKey = createPublicKey(pem);
      expect(verify(null, artifact, publicKey, Buffer.from(fragment.signature, "base64"))).toBe(true);
      expect(verify(null, Buffer.from("tampered"), publicKey, Buffer.from(fragment.signature, "base64"))).toBe(false);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
