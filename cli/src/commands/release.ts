/** `voussoir release`: assemble one release manifest, with a signed delta.
 *
 *  The pieces exist separately (`patch` builds a delta, `sign` produces the hash
 *  and signature, `keygen` makes the key); this puts them in the one file the
 *  shell's updater reads. The manifest is the artifact that matters: a release is
 *  not "a binary on a server", it is a `latest.json` whose every entry can be
 *  checked before it is used.
 *
 *  Two things are signed, not one: the full artifact, and the patch. The patch is
 *  applied to the running binary and the *result* is then checked against the
 *  artifact's own hash and signature, so a hostile patch can only produce bytes
 *  that fail verification.
 */

import { createHash, createPublicKey, verify as cryptoVerify } from "node:crypto";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { stringFlag, type Args } from "../args.ts";
import { fail } from "../proc.ts";
import { applyPatch, createPatch } from "../delta.ts";
import { artifactFragment } from "./keys.ts";
import { installBytes, platformKey } from "./update.ts";

/** The `sha256` and `signature` pair that makes an artifact trustworthy. */
export interface SignedEntry {
  sha256: string;
  signature: string;
}

/** What `latest.json` holds for one platform. */
export interface ManifestEntry extends SignedEntry {
  url: string;
}

/** A delta from one older version, in the shape the shell's updater parses. */
export interface ManifestPatch extends SignedEntry {
  from: string;
  url: string;
}

export interface Manifest {
  version: string;
  notes?: unknown;
  downloads: Record<string, ManifestEntry>;
  patches?: Record<string, ManifestPatch>;
}

/** Rebuild an Ed25519 public key from the raw 32 bytes `keygen` prints. */
function publicKeyObject(rawBase64: string): ReturnType<typeof createPublicKey> {
  const raw = Buffer.from(rawBase64, "base64");
  if (raw.length !== 32) {
    throw new Error(`a raw Ed25519 public key is 32 bytes, this one is ${raw.length}`);
  }
  // Ed25519 SPKI is a fixed header followed by the raw key.
  const der = Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), raw]);
  return createPublicKey({ key: der, format: "der", type: "spki" });
}

/** SHA-256, lowercase hex, which is the form every entry carries. */
export function sha256Hex(bytes: Uint8Array): string {
  return createHash("sha256").update(bytes).digest("hex");
}

/**
 * Throw unless `bytes` are exactly the artifact the feed promised.
 *
 * The hash is checked before the signature: a signature over bytes that are not
 * the release says nothing useful, and checking it first would report a
 * confusing "bad signature" for what is really a truncated download.
 */
export function verifyArtifact(
  bytes: Uint8Array,
  entry: SignedEntry,
  rawPublicKey: string,
): void {
  const actual = sha256Hex(bytes);
  if (entry.sha256 !== actual) {
    throw new Error(`sha256 mismatch: the feed says ${entry.sha256}, this is ${actual}`);
  }
  if (!entry.signature) {
    throw new Error("the feed entry has no signature");
  }
  const ok = cryptoVerify(
    null,
    Buffer.from(bytes),
    publicKeyObject(rawPublicKey),
    Buffer.from(entry.signature, "base64"),
  );
  if (!ok) {
    throw new Error("the signature does not match the key in update.public_key");
  }
}

/**
 * Apply `patch` to the installed binary and replace it, or change nothing.
 *
 * This is the updater's critical path, so it is ordered for rollback: the
 * reconstructed bytes are verified before the target is touched, and the only
 * write is the atomic rename `installBytes` already does. A patch that fails to
 * apply, a hash mismatch and a bad signature all leave the running binary in
 * place rather than a half-replaced one.
 */
export function applyVerified(
  target: string,
  patch: Uint8Array,
  entry: SignedEntry,
  rawPublicKey: string,
): Uint8Array {
  const current = readFileSync(target);
  const next = applyPatch(current, patch);
  verifyArtifact(next, entry, rawPublicKey);
  installBytes(Buffer.from(next), target);
  return next;
}

/** Build the manifest for one platform, given the bytes and their URLs. */
export function manifestFor(input: {
  version: string;
  platform: string;
  artifact: Uint8Array;
  artifactUrl: string;
  privateKey: string | Buffer;
  notes?: unknown;
  previous?: { version: string; bytes: Uint8Array; patchUrl: string };
}): { manifest: Manifest; patch?: Buffer } {
  const full = artifactFragment(input.artifact, input.privateKey);
  const manifest: Manifest = {
    version: input.version,
    ...(input.notes === undefined ? {} : { notes: input.notes }),
    downloads: {
      [input.platform]: { url: input.artifactUrl, sha256: full.sha256, signature: full.signature },
    },
  };

  if (input.previous) {
    const patch = createPatch(input.previous.bytes, input.artifact);
    const signed = artifactFragment(patch, input.privateKey);
    manifest.patches = {
      [input.platform]: {
        from: input.previous.version,
        url: input.previous.patchUrl,
        sha256: signed.sha256,
        signature: signed.signature,
      },
    };
    return { manifest, patch };
  }
  return { manifest };
}

export function release(args: Args): number {
  const version = stringFlag(args, "version");
  if (!version) fail("release needs --version <x.y.z>");
  const artifactPath = stringFlag(args, "artifact");
  if (!artifactPath) fail("release needs --artifact <path> to the built binary");

  // A manifest without a URL is not a release, it is a local note: the updater
  // refuses an untrusted scheme, so the base URL is required rather than guessed.
  const baseUrl = stringFlag(args, "base-url");
  if (!baseUrl) fail("release needs --base-url <https://…> where the artifacts will be served");

  const platform = stringFlag(args, "platform") ?? platformKey();
  const keyPath = stringFlag(args, "key") ?? "voussoir-signing-key.pem";
  const out = stringFlag(args, "out") ?? "latest.json";

  let artifact: Buffer;
  try {
    artifact = readFileSync(artifactPath);
  } catch (error) {
    fail(`cannot read ${artifactPath}: ${(error as Error).message}`);
  }
  let key: Buffer;
  try {
    key = readFileSync(keyPath);
  } catch (error) {
    fail(`cannot read the signing key ${keyPath}: ${(error as Error).message}`);
  }

  const name = basename(artifactPath);
  const base = baseUrl.replace(/\/+$/, "");

  let previous: { version: string; bytes: Uint8Array; patchUrl: string } | undefined;
  const previousPath = stringFlag(args, "previous");
  if (previousPath) {
    const previousVersion = stringFlag(args, "from");
    if (!previousVersion) {
      fail("--previous needs --from <x.y.z> so the patch says what it upgrades");
    }
    if (!existsSync(previousPath)) fail(`no previous artifact at ${previousPath}`);
    previous = {
      version: previousVersion,
      bytes: readFileSync(previousPath),
      patchUrl: `${base}/${platform}-${previousVersion}-to-${version}.vsd`,
    };
  }

  const { manifest, patch } = manifestFor({
    version,
    platform,
    artifact,
    artifactUrl: `${base}/${name}`,
    privateKey: key,
    notes: stringFlag(args, "notes"),
    previous,
  });

  // The delta lands beside the manifest so the two travel together; its URL is
  // derived from the same base the artifacts use.
  if (previous && patch) {
    const patchName = `${platform}-${previous.version}-to-${version}.vsd`;
    writeFileSync(join(dirname(out), patchName), patch);
    console.log(`  wrote ${patchName} (${patch.length} bytes)`);
  }

  writeFileSync(out, `${JSON.stringify(manifest, null, 2)}\n`);

  const saved = (size: number) => {
    if (!previous || !patch) return "";
    const ratio = Math.round((1 - patch.length / size) * 100);
    return `, ${ratio}% smaller than the full artifact`;
  };
  const size = artifact.length;
  console.log(`  wrote ${out} for ${platform} ${version} (${size} bytes${saved(size)})`);
  console.log(`  update.public_key must be "${artifactFragment(artifact, key).publicKey}"`);
  return 0;
}
