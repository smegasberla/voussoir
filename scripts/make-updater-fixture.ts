#!/usr/bin/env bun
/**
 * Regenerate the updater contract fixture.
 *
 * The CLI generates manifests and patches; the shell parses and applies them.
 * Neither side can see the other's tests, so a format change on one side passed
 * both suites while the shipped pair was broken. This script writes real output
 * from the CLI's own functions into `voussoir/tests/fixtures`, and the Rust
 * integration test consumes it: if the two halves drift, that test fails.
 *
 * The key is derived from the seed the Rust tests already use
 * (`SigningKey::from_bytes(&[42u8; 32])`), so both sides sign with the same key
 * without storing a private key in the repository.
 *
 * Usage: bun scripts/make-updater-fixture.ts
 */

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { createHash } from "node:crypto";
import { createPatch } from "../cli/src/delta.ts";
import { manifestFor, mergeManifests, type Manifest } from "../cli/src/commands/release.ts";
import { publicKeyBase64 } from "../cli/src/commands/keys.ts";

const out = join(import.meta.dir, "..", "voussoir", "tests", "fixtures");
mkdirSync(out, { recursive: true });

// PKCS#8 wrapping for a raw Ed25519 seed: the fixed 16-byte prefix plus the seed.
// This is the same key `ed25519_dalek::SigningKey::from_bytes(&[42; 32])` makes.
const SEED = Buffer.alloc(32, 42);
const PEM = Buffer.from(
  [
    "-----BEGIN PRIVATE KEY-----",
    Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), SEED]).toString("base64"),
    "-----END PRIVATE KEY-----",
    "",
  ].join("\n"),
);

/** A small stand-in for a binary: long shared runs and a few changed bytes. */
function binary(tag: string): Buffer {
  const parts: Buffer[] = [];
  for (let block = 0; block < 40; block++) {
    parts.push(Buffer.from(`section-${block}-`.padEnd(64, "x")));
  }
  parts.push(Buffer.from(`\nversion: ${tag}\n`));
  for (let block = 0; block < 40; block++) {
    parts.push(Buffer.from(`trailer-${block}-`.padEnd(64, "y")));
  }
  return Buffer.concat(parts);
}

const old = binary("1.0.0");
const next = binary("1.1.0");

// Two previous versions, so the fixture pins the chain shape rather than only
// the single-hop case: 1.0.0 -> 1.1.0 and 0.9.0 -> 1.1.0 are both offered.
const older = binary("0.9.0");

const base = "http://127.0.0.1:8080";

// Every platform key the shell can ask for points at the same fixture bytes, so
// the contract test runs on Linux, macOS and Windows rather than only on the
// machine that generated the fixture.
const PLATFORMS = [
  "linux-x86_64",
  "linux-aarch64",
  "macos-x86_64",
  "macos-aarch64",
  "windows-x86_64",
  "windows-aarch64",
];

// 0.9.0 is offered too, which pins the chain: two patches from two versions.
const previous = [
  { version: "0.9.0", bytes: older },
  { version: "1.0.0", bytes: old },
].map((step) => ({
  ...step,
  patchUrl: `${base}/any-${step.version}-to-1.1.0.vsd`,
}));

const written: { name: string; bytes: Buffer }[] = [];
let manifest: Manifest | undefined;
for (const platform of PLATFORMS) {
  const built = manifestFor({
    version: "1.1.0",
    platform,
    artifact: next,
    artifactUrl: `${base}/app-1.1.0`,
    privateKey: PEM,
    notes: "contract fixture",
    previous,
  });
  manifest = manifest === undefined ? built.manifest : mergeManifests(manifest, built.manifest, platform);
  if (written.length === 0) written.push(...built.patches);
}
if (!manifest) throw new Error("no manifest was produced");
const patches = written;

writeFileSync(join(out, "old.bin"), old);
writeFileSync(join(out, "older.bin"), older);
writeFileSync(join(out, "new.bin"), next);
for (const patch of patches) writeFileSync(join(out, patch.name), patch.bytes);
writeFileSync(join(out, "latest.json"), `${JSON.stringify(manifest, null, 2)}\n`);

// A feed in the single-object shape older publishers wrote, which the parser
// still has to read.
const legacy = {
  version: "1.1.0",
  url: `${base}/app-1.1.0`,
  sha256: createHash("sha256").update(next).digest("hex"),
  signature: manifest.downloads[PLATFORMS[0]!]!.signature,
  patches: Object.fromEntries(
    PLATFORMS.map((platform) => [platform, manifest!.patches![platform]![0]]),
  ),
};
writeFileSync(join(out, "legacy.json"), `${JSON.stringify(legacy, null, 2)}\n`);

writeFileSync(join(out, "public-key.txt"), `${publicKeyBase64(PEM)}\n`);

console.log(`  wrote the fixture to ${out}`);
console.log(`  patches: ${patches.map((patch) => `${patch.name} (${patch.bytes.length}B)`).join(", ")}`);
