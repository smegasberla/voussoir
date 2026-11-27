import { describe, expect, test } from "bun:test";
import { createHash, generateKeyPairSync, verify } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseArgs } from "./args.ts";
import { applyPatch, createPatch } from "./delta.ts";
import {
  applyVerified,
  compareVersions,
  manifestFor,
  mergeManifests,
  release,
  sha256Hex,
  verifyArtifact,
} from "./commands/release.ts";
import { publicKeyBase64 } from "./commands/keys.ts";

function keyPair(): { pem: Buffer; raw: string } {
  const { privateKey } = generateKeyPairSync("ed25519");
  const pem = Buffer.from(privateKey.export({ type: "pkcs8", format: "pem" }) as string);
  return { pem, raw: publicKeyBase64(pem) };
}

function tempDir(): string {
  return mkdtempSync(join(tmpdir(), "voussoir-release-"));
}

/** An old and a new binary that share most of their bytes, as a rebuild does. */
function artifacts(): { old: Buffer; next: Buffer } {
  const shared = Buffer.from("ELF-ish shared body ".repeat(200));
  return {
    old: Buffer.concat([shared, Buffer.from("version one")]),
    next: Buffer.concat([shared, Buffer.from("version two, genuinely different")]),
  };
}

describe("release manifest", () => {
  test("names the platform, and its signature verifies", () => {
    const { pem, raw } = keyPair();
    const { next } = artifacts();
    const { manifest } = manifestFor({
      version: "1.2.0",
      platform: "linux-x86_64",
      artifact: next,
      artifactUrl: "https://example.invalid/app-1.2.0",
      privateKey: pem,
    });

    expect(manifest.version).toBe("1.2.0");
    const entry = manifest.downloads["linux-x86_64"]!;
    expect(entry.url).toBe("https://example.invalid/app-1.2.0");
    expect(entry.sha256).toBe(createHash("sha256").update(next).digest("hex"));
    // The signature must verify with the raw key `voussoir.json` carries, which
    // is what the shell uses. Verifying with anything else proves nothing.
    const der = Buffer.concat([
      Buffer.from("302a300506032b6570032100", "hex"),
      Buffer.from(raw, "base64"),
    ]);
    const ok = verify(null, next, { key: der, format: "der", type: "spki" }, Buffer.from(entry.signature, "base64"));
    expect(ok).toBe(true);
  });

  test("with --previous it adds a patch that rebuilds the new artifact", () => {
    const { pem } = keyPair();
    const { old, next } = artifacts();
    const { manifest, patches } = manifestFor({
      version: "1.2.0",
      platform: "linux-x86_64",
      artifact: next,
      artifactUrl: "https://example.invalid/app-1.2.0",
      privateKey: pem,
      previous: [{
        version: "1.1.0",
        bytes: old,
        patchUrl: "https://example.invalid/linux-x86_64-1.1.0-to-1.2.0.vsd",
      }],
    });

    expect(patches.length).toBe(1);
    const patch = patches[0]!.bytes;
    const entry = manifest.patches!["linux-x86_64"]![0]!;
    expect(entry.from).toBe("1.1.0");
    expect(entry.url).toBe("https://example.invalid/linux-x86_64-1.1.0-to-1.2.0.vsd");
    // The patch entry must describe the patch, not the artifact.
    expect(entry.sha256).toBe(sha256Hex(patch!));
    // And applying it must produce the artifact the download entry describes.
    expect(applyPatch(old, patch!)).toEqual(next);
    expect(sha256Hex(applyPatch(old, patch!))).toBe(manifest.downloads["linux-x86_64"]!.sha256);
  });

  test("a delta is much smaller than the artifact it replaces", () => {
    const { pem } = keyPair();
    const { old, next } = artifacts();
    const { patches } = manifestFor({
      version: "1.2.0",
      platform: "linux-x86_64",
      artifact: next,
      artifactUrl: "https://example.invalid/app",
      privateKey: pem,
      previous: [{ version: "1.1.0", bytes: old, patchUrl: "https://example.invalid/p.vsd" }],
    });
    // The claim the feature rests on: a small change costs a small download.
    expect(patches[0]!.bytes.length).toBeLessThan(next.length / 2);
  });

  test("no --previous means no patches key, not an empty one", () => {
    // An empty object would be parsed and then have no entry for the platform;
    // omitting the key is the shape the updater treats as "no patch offered".
    const { pem } = keyPair();
    const { next } = artifacts();
    const { manifest } = manifestFor({
      version: "1.0.0",
      platform: "linux-x86_64",
      artifact: next,
      artifactUrl: "https://example.invalid/app",
      privateKey: pem,
    });
    expect("patches" in manifest).toBe(false);
  });
});

describe("artifact verification", () => {
  test("accepts the artifact the manifest describes", () => {
    const { pem, raw } = keyPair();
    const { next } = artifacts();
    const { manifest } = manifestFor({
      version: "1.0.0",
      platform: "linux-x86_64",
      artifact: next,
      artifactUrl: "https://example.invalid/app",
      privateKey: pem,
    });
    expect(() => verifyArtifact(next, manifest.downloads["linux-x86_64"]!, raw)).not.toThrow();
  });

  test("rejects one byte of difference before it looks at the signature", () => {
    const { pem, raw } = keyPair();
    const { next } = artifacts();
    const { manifest } = manifestFor({
      version: "1.0.0",
      platform: "linux-x86_64",
      artifact: next,
      artifactUrl: "https://example.invalid/app",
      privateKey: pem,
    });
    const tampered = Buffer.from(next);
    tampered[0] = (tampered[0] ?? 0) ^ 0xff;
    // The message must name the hash: a truncated download is the common case,
    // and "bad signature" would send someone looking at the key.
    expect(() => verifyArtifact(tampered, manifest.downloads["linux-x86_64"]!, raw)).toThrow(
      /sha256 mismatch/,
    );
  });

  test("rejects a signature made by a different key", () => {
    const signer = keyPair();
    const other = keyPair();
    const { next } = artifacts();
    const { manifest } = manifestFor({
      version: "1.0.0",
      platform: "linux-x86_64",
      artifact: next,
      artifactUrl: "https://example.invalid/app",
      privateKey: signer.pem,
    });
    expect(() => verifyArtifact(next, manifest.downloads["linux-x86_64"]!, other.raw)).toThrow(
      /signature does not match/,
    );
  });
});

describe("installing a delta over an installed binary", () => {
  test("replaces the binary with the bytes the feed signed", () => {
    const { pem, raw } = keyPair();
    const { old, next } = artifacts();
    const { manifest, patches } = manifestFor({
      version: "1.2.0",
      platform: "linux-x86_64",
      artifact: next,
      artifactUrl: "https://example.invalid/app",
      privateKey: pem,
      previous: [{ version: "1.1.0", bytes: old, patchUrl: "https://example.invalid/p.vsd" }],
    });

    const dir = tempDir();
    try {
      // The "running binary": the installed copy, replaced by rename underneath.
      const installed = join(dir, "app");
      writeFileSync(installed, old);

      applyVerified(installed, patches[0]!.bytes, manifest.downloads["linux-x86_64"]!, raw);
      expect(new Uint8Array(readFileSync(installed))).toEqual(new Uint8Array(next));
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("a bad signature leaves the installed binary untouched", () => {
    const signer = keyPair();
    const attacker = keyPair();
    const { old, next } = artifacts();
    // The attacker signs their own payload, so only the key check can stop it.
    const evil = Buffer.concat([Buffer.from("backdoor"), next.subarray(8)]);
    const forged = manifestFor({
      version: "9.9.9",
      platform: "linux-x86_64",
      artifact: evil,
      artifactUrl: "https://example.invalid/evil",
      privateKey: attacker.pem,
    }).manifest;
    const patch = createPatch(old, evil);

    const dir = tempDir();
    try {
      const installed = join(dir, "app");
      writeFileSync(installed, old);

      // The shell trusts signer's key, so the forged artifact must not install.
      expect(() =>
        applyVerified(installed, patch, forged.downloads["linux-x86_64"]!, signer.raw),
      ).toThrow();
      expect(new Uint8Array(readFileSync(installed))).toEqual(new Uint8Array(old));
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("a patch that does not yield the signed bytes leaves the binary untouched", () => {
    const { pem, raw } = keyPair();
    const { old, next } = artifacts();
    const { manifest } = manifestFor({
      version: "1.2.0",
      platform: "linux-x86_64",
      artifact: next,
      artifactUrl: "https://example.invalid/app",
      privateKey: pem,
    });
    // A patch for a different upgrade: applies cleanly, produces wrong bytes.
    const wrongPatch = createPatch(old, Buffer.from("something else entirely"));

    const dir = tempDir();
    try {
      const installed = join(dir, "app");
      writeFileSync(installed, old);

      expect(() => applyVerified(installed, wrongPatch, manifest.downloads["linux-x86_64"]!, raw)).toThrow(
        /sha256 mismatch/,
      );
      expect(new Uint8Array(readFileSync(installed))).toEqual(new Uint8Array(old));
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("a malformed patch is refused and nothing is written", () => {
    const { pem, raw } = keyPair();
    const { old, next } = artifacts();
    const { manifest } = manifestFor({
      version: "1.2.0",
      platform: "linux-x86_64",
      artifact: next,
      artifactUrl: "https://example.invalid/app",
      privateKey: pem,
    });

    const dir = tempDir();
    try {
      const installed = join(dir, "app");
      writeFileSync(installed, old);
      expect(() =>
        applyVerified(installed, Buffer.from("NOPE"), manifest.downloads["linux-x86_64"]!, raw),
      ).toThrow(/VSD1/);
      expect(new Uint8Array(readFileSync(installed))).toEqual(new Uint8Array(old));
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe("release command", () => {
  test("writes the manifest, and the delta beside it with the URL it advertises", () => {
    const { pem, raw } = keyPair();
    const { old, next } = artifacts();
    const dir = tempDir();
    try {
      const keyPath = join(dir, "key.pem");
      writeFileSync(keyPath, pem);
      const oldPath = join(dir, "app-1.1.0");
      const nextPath = join(dir, "app-1.2.0");
      writeFileSync(oldPath, old);
      writeFileSync(nextPath, next);
      const out = join(dir, "latest.json");

      const log = console.log;
      console.log = () => {};
      try {
        release(
          parseArgs([
            "release",
            "--version",
            "1.2.0",
            "--artifact",
            nextPath,
            "--base-url",
            "https://example.invalid/releases/",
            "--platform",
            "linux-x86_64",
            "--previous",
            oldPath,
            "--from",
            "1.1.0",
            "--key",
            keyPath,
            "--out",
            out,
          ]),
        );
      } finally {
        console.log = log;
      }

      const manifest = JSON.parse(readFileSync(out, "utf8"));
      const entry = manifest.downloads["linux-x86_64"];
      // The base URL's trailing slash is normalised, not doubled.
      expect(entry.url).toBe("https://example.invalid/releases/app-1.2.0");
      expect(manifest.patches["linux-x86_64"][0].url).toBe(
        "https://example.invalid/releases/linux-x86_64-1.1.0-to-1.2.0.vsd",
      );

      // The file the manifest points at must exist, and be the patch it names.
      const patchName = manifest.patches["linux-x86_64"][0].url.split("/").pop();
      const written = readFileSync(join(dir, patchName!));
      expect(sha256Hex(written)).toBe(manifest.patches["linux-x86_64"][0].sha256);
      expect(applyPatch(old, written)).toEqual(next);
      expect(() => verifyArtifact(next, entry, raw)).not.toThrow();
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  // `fail()` exits the process, so these run the CLI the way a user does. An
  // in-process throw would prove nothing about the exit code a wrapper script sees.
  const runCli = (args: string[], cwd: string) => {
    const result = Bun.spawnSync(["bun", join(import.meta.dir, "index.ts"), ...args], {
      cwd,
      stdout: "pipe",
      stderr: "pipe",
    });
    return { code: result.exitCode, output: result.stdout.toString() + result.stderr.toString() };
  };

  test("refuses to build a manifest with no base URL", () => {
    // Without one there is nothing for the updater to fetch, and guessing a host
    // would produce a manifest that silently 404s.
    const dir = tempDir();
    try {
      writeFileSync(join(dir, "app"), "x");
      const { code, output } = runCli(
        ["release", "--version", "1.0.0", "--artifact", "app", "--key", "k"],
        dir,
      );
      expect(code).toBe(1);
      expect(output).toContain("base-url");
      // An expected failure is one clean line, not a stack trace.
      expect(output).not.toContain("    at ");
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("refuses --previous without --from", () => {
    // A patch with no `from` cannot be matched to an installed version, and the
    // shell would offer it to every user.
    const dir = tempDir();
    try {
      writeFileSync(join(dir, "app"), "x");
      writeFileSync(join(dir, "k"), "unused: the flag is rejected before the key is read");
      const { code, output } = runCli(
        [
          "release",
          "--version",
          "1.0.0",
          "--artifact",
          "app",
          "--base-url",
          "https://example.invalid",
          "--previous",
          "app",
          "--key",
          "k",
        ],
        dir,
      );
      expect(code).toBe(1);
      expect(output).toContain("--from");
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe("patch chains", () => {
  test("one patch is offered from every version still in the wild", () => {
    const { pem } = keyPair();
    const { old, next } = artifacts();
    const oldest = Buffer.concat([Buffer.from("very old build"), old.subarray(14)]);

    const { manifest, patches } = manifestFor({
      version: "2.0.0",
      platform: "linux-x86_64",
      artifact: next,
      artifactUrl: "https://example.invalid/app-2.0.0",
      privateKey: pem,
      previous: [
        { version: "1.9.0", bytes: old, patchUrl: "https://example.invalid/a.vsd" },
        { version: "1.0.0", bytes: oldest, patchUrl: "https://example.invalid/b.vsd" },
      ],
    });

    const hops = manifest.patches!["linux-x86_64"]!;
    // Oldest first, so the first hop the updater matches is the furthest back it
    // can go: someone three versions behind does not download the full artifact.
    expect(hops.map((hop) => hop.from)).toEqual(["1.0.0", "1.9.0"]);
    // Each hop rebuilds this release from its own source, not from the next one.
    for (const hop of hops) {
      const source = hop.from === "1.0.0" ? oldest : old;
      const patch = patches.find((entry) => hop.url.endsWith(entry.name))!.bytes;
      expect(applyPatch(source, patch)).toEqual(next);
    }
  });

  test("versions sort numerically, so 1.10 comes after 1.9", () => {
    expect(compareVersions("1.10.0", "1.9.0")).toBeGreaterThan(0);
    expect(compareVersions("1.9.0", "1.10.0")).toBeLessThan(0);
    expect(compareVersions("1.0.0", "1.0.0")).toBe(0);
  });
});

describe("merging per-platform manifests", () => {
  const built = (platform: string, version: string, key: Buffer) => {
    const { next } = artifacts();
    return manifestFor({
      version,
      platform,
      artifact: next,
      artifactUrl: `https://example.invalid/${platform}`,
      privateKey: key,
    }).manifest;
  };

  test("two CI jobs produce one feed serving both downloads", () => {
    const { pem } = keyPair();
    const linux = built("linux-x86_64", "1.2.0", pem);
    const windows = built("windows-x86_64", "1.2.0", pem);

    const merged = mergeManifests(linux, windows, "windows-x86_64");
    expect(Object.keys(merged.downloads).sort()).toEqual(["linux-x86_64", "windows-x86_64"]);
    expect(merged.version).toBe("1.2.0");
  });

  test("merging a different version is refused", () => {
    // Two versions in one file would offer a user whichever artifact the last
    // job wrote, which is how an update silently downgrades.
    const { pem } = keyPair();
    expect(() =>
      mergeManifests(built("linux-x86_64", "1.2.0", pem), built("windows-x86_64", "1.1.0", pem), "windows-x86_64"),
    ).toThrow(/refusing to merge 1\.1\.0 into a manifest for 1\.2\.0/);
  });
});
