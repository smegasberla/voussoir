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
import { boolFlag, listFlag, stringFlag, type Args } from "../args.ts";
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

/**
 * One hop of a delta chain, in the shape the shell's updater parses.
 *
 * `from` names the version this hop upgrades. A platform lists its hops oldest
 * first, so a user on 1.0.0 can reach 1.3.0 through 1.0->1.1, 1.1->1.2 and
 * 1.2->1.3 rather than downloading the full artifact.
 */
export interface ManifestPatch extends SignedEntry {
  from: string;
  url: string;
}

export interface Manifest {
  version: string;
  notes?: unknown;
  downloads: Record<string, ManifestEntry>;
  patches?: Record<string, ManifestPatch[]>;
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

/** Compare two `x.y.z` strings numerically, prerelease suffixes compared as text. */
export function compareVersions(a: string, b: string): number {
  const split = (v: string) => v.split(/[.+-]/).filter((part) => part.length > 0);
  const left = split(a);
  const right = split(b);
  for (let i = 0; i < Math.max(left.length, right.length); i++) {
    const x = left[i];
    const y = right[i];
    if (x === undefined) return -1;
    if (y === undefined) return 1;
    const nx = Number(x);
    const ny = Number(y);
    // Numeric parts compare as numbers so 1.10.0 is newer than 1.9.0; anything
    // else (a `-beta.1` suffix) compares as text, which is stable if not strict.
    const bothNumeric = Number.isFinite(nx) && Number.isFinite(ny);
    const order = bothNumeric ? nx - ny : x < y ? -1 : x > y ? 1 : 0;
    if (order !== 0) return order;
  }
  return 0;
}

/** Oldest first, so a chain's first entry is the furthest back it can reach. */
function sortedByVersion<T extends { version: string }>(entries: T[]): T[] {
  return [...entries].sort((a, b) => compareVersions(a.version, b.version));
}

/**
 * Fold one platform's entry into an existing manifest.
 *
 * CI builds each platform on its own machine, so N jobs each produce a manifest
 * with one `downloads` key. Merging keeps the version fixed: two different
 * versions in one file would let a user on 1.2.0 be offered a 1.1.0 artifact by
 * whichever platform happened to be written last.
 */
export function mergeManifests(existing: Manifest, next: Manifest, platform: string): Manifest {
  if (existing.version !== next.version) {
    throw new Error(
      `refusing to merge ${next.version} into a manifest for ${existing.version}; ` +
        `delete the old manifest or pass the same --version`,
    );
  }
  const downloads = { ...existing.downloads, ...next.downloads };
  const patches: Record<string, ManifestPatch[]> = { ...(existing.patches ?? {}) };
  for (const [key, hops] of Object.entries(next.patches ?? {})) patches[key] = hops;
  const notes = existing.notes ?? next.notes;
  return {
    version: existing.version,
    ...(notes === undefined ? {} : { notes }),
    downloads,
    ...(Object.keys(patches).length === 0 ? {} : { patches }),
  };
}

/** Build the manifest for one platform, given the bytes and their URLs. */
export function manifestFor(input: {
  version: string;
  platform: string;
  artifact: Uint8Array;
  artifactUrl: string;
  privateKey: string | Buffer;
  notes?: unknown;
  previous?: { version: string; bytes: Uint8Array; patchUrl: string }[];
}): { manifest: Manifest; patches: { name: string; bytes: Buffer }[] } {
  const full = artifactFragment(input.artifact, input.privateKey);
  const manifest: Manifest = {
    version: input.version,
    ...(input.notes === undefined ? {} : { notes: input.notes }),
    downloads: {
      [input.platform]: { url: input.artifactUrl, sha256: full.sha256, signature: full.signature },
    },
  };

  const patches: { name: string; bytes: Buffer }[] = [];
  const hops: ManifestPatch[] = [];
  // Every previous artifact gets a patch that rebuilds *this* release from it, so
  // someone on 1.0.0 patches straight to 1.3.0 instead of downloading the full
  // artifact just because they skipped 1.1 and 1.2. The list is ordered oldest
  // first so the first match the updater finds is the furthest back it can go.
  for (const step of sortedByVersion(input.previous ?? [])) {
    const bytes = createPatch(step.bytes, input.artifact);
    const signed = artifactFragment(bytes, input.privateKey);
    hops.push({
      from: step.version,
      url: step.patchUrl,
      sha256: signed.sha256,
      signature: signed.signature,
    });
    patches.push({ name: step.patchUrl.split("/").pop()!, bytes });
  }
  if (hops.length > 0) manifest.patches = { [input.platform]: hops };
  return { manifest, patches };
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

  // `--previous` and `--from` are repeatable and paired by position, so one
  // invocation can offer a direct patch from every version still in the wild.
  const previousPaths = listFlag(args, "previous");
  const fromVersions = listFlag(args, "from");
  if (previousPaths.length !== fromVersions.length) {
    fail("--previous and --from must be repeated together, one pair per previous version");
  }
  const previous = previousPaths.map((path, index) => {
    const previousVersion = fromVersions[index]!;
    if (!existsSync(path)) fail(`no previous artifact at ${path}`);
    return {
      version: previousVersion,
      bytes: readFileSync(path),
      patchUrl: `${base}/${platform}-${previousVersion}-to-${version}.vsd`,
    };
  });

  const { manifest, patches } = manifestFor({
    version,
    platform,
    artifact,
    artifactUrl: `${base}/${name}`,
    privateKey: key,
    notes: stringFlag(args, "notes"),
    previous,
  });

  // Each delta lands beside the manifest so the two travel together; their URLs
  // are derived from the same base the artifacts use.
  for (const { name: patchName, bytes } of patches) {
    writeFileSync(join(dirname(out), patchName), bytes);
    console.log(`  wrote ${patchName} (${bytes.length} bytes)`);
  }

  // `--merge` is for CI: each platform's job writes its own entry into the one
  // manifest, so a single feed serves every download.
  let final = manifest;
  if (boolFlag(args, "merge") && existsSync(out)) {
    try {
      const existing = JSON.parse(readFileSync(out, "utf8")) as Manifest;
      final = mergeManifests(existing, manifest, platform);
    } catch (error) {
      fail(`cannot merge into ${out}: ${(error as Error).message}`);
    }
  }

  writeFileSync(out, `${JSON.stringify(final, null, 2)}\n`);

  const saved = patches.length > 0 ? `, ${patches.length} delta(s)` : "";
  console.log(`  wrote ${out} for ${platform} ${version} (${artifact.length} bytes${saved})`);
  if (Object.keys(final.downloads).length > 1) {
    console.log(`  the manifest now offers ${Object.keys(final.downloads).join(", ")}`);
  }
  console.log(`  update.public_key must be "${artifactFragment(artifact, key).publicKey}"`);
  return 0;
}
