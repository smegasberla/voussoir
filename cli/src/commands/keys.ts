/** Release signing: the key half of the updater's trust model.
 *
 *  The shell verifies an artifact's Ed25519 signature with the base64 public key
 *  in `voussoir.json`. This is where that key, and the signature, come from; it
 *  uses Node's Ed25519 support so the CLI needs no crypto dependency.
 */

import {
  createHash,
  createPrivateKey,
  createPublicKey,
  generateKeyPairSync,
  sign as cryptoSign,
} from "node:crypto";
import { readFileSync, writeFileSync, chmodSync } from "node:fs";
import { join } from "node:path";
import { stringFlag, type Args } from "../args.ts";
import { fail } from "../proc.ts";

/** The raw 32-byte public key, base64, which is what `update.public_key` holds. */
export function publicKeyBase64(privateKeyPem: string | Buffer): string {
  const der = createPublicKey(privateKeyPem).export({ type: "spki", format: "der" });
  // Ed25519 SPKI is a fixed 12-byte header followed by the raw key.
  return Buffer.from(der.subarray(der.length - 32)).toString("base64");
}

/** The `sha256` and `signature` a feed entry needs for one artifact. */
export function artifactFragment(
  artifact: Uint8Array,
  privateKeyPem: string | Buffer,
): { sha256: string; signature: string; publicKey: string } {
  const key = createPrivateKey(privateKeyPem);
  return {
    sha256: createHash("sha256").update(artifact).digest("hex"),
    signature: cryptoSign(null, artifact, key).toString("base64"),
    publicKey: publicKeyBase64(privateKeyPem),
  };
}

/** `voussoir keygen`: write a signing key and print the public half. */
export function keygen(args: Args): number {
  const out = stringFlag(args, "out") ?? "voussoir-signing-key.pem";
  const { privateKey } = generateKeyPairSync("ed25519");
  const pem = privateKey.export({ type: "pkcs8", format: "pem" });
  writeFileSync(out, pem, { mode: 0o600 });
  chmodSync(out, 0o600);
  console.log(`  wrote the private key to ${out} (keep it out of the repo and back it up)`);
  console.log(`  update.public_key: "${publicKeyBase64(pem)}"`);
  return 0;
}

/** `voussoir sign`: print the feed fields for one artifact. */
export function sign(args: Args): number {
  const artifactPath = args.positionals[0];
  if (!artifactPath) {
    fail("sign needs an artifact path: voussoir sign <artifact> --key <pem>");
  }
  const keyPath = stringFlag(args, "key") ?? "voussoir-signing-key.pem";
  let artifact: Buffer;
  let key: Buffer;
  try {
    artifact = readFileSync(artifactPath);
  } catch (error) {
    fail(`cannot read ${artifactPath}: ${(error as Error).message}`);
  }
  try {
    key = readFileSync(keyPath);
  } catch (error) {
    fail(`cannot read the signing key ${keyPath}: ${(error as Error).message}`);
  }
  const fragment = artifactFragment(artifact, key);
  console.log(JSON.stringify({ sha256: fragment.sha256, signature: fragment.signature }, null, 2));
  console.log(`\n  signed with the key whose update.public_key is:\n  ${fragment.publicKey}`);
  return 0;
}
