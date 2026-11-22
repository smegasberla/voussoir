import { describe, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  CLI_VERSION,
  compareVersions,
  ensureTrustedUrl,
  installBytes,
  platformKey,
  update,
} from "./commands/update.ts";
import { parseArgs } from "./args.ts";

describe("CLI version comparison", () => {
  test("compares numeric segments", () => {
    expect(compareVersions("1.10.0", "1.9.0")).toBe(1);
    expect(compareVersions("2.0.0", "2.0.0")).toBe(0);
    expect(compareVersions("1.0.0", "1.0.1")).toBe(-1);
    expect(compareVersions("1.0.0-beta.1", "1.0.0")).toBe(-1);
  });

  test("tracks package.json", () => {
    const pkg = JSON.parse(readFileSync(join(import.meta.dir, "..", "package.json"), "utf8"));
    expect(CLI_VERSION).toBe(pkg.version);
  });
});

describe("CLI platform keys", () => {
  test("uses the Rust updater's names", () => {
    expect(platformKey("linux", "x64")).toBe("linux-x86_64");
    expect(platformKey("win32", "x64")).toBe("windows-x86_64");
    expect(platformKey("darwin", "arm64")).toBe("macos-arm64");
  });
});

describe("CLI download trust", () => {
  test("https and loopback pass, anything else is refused", () => {
    expect(() => ensureTrustedUrl("https://example.com/app")).not.toThrow();
    expect(() => ensureTrustedUrl("http://127.0.0.1:9/app")).not.toThrow();
    expect(() => ensureTrustedUrl("http://localhost:9/app")).not.toThrow();
    expect(() => ensureTrustedUrl("http://example.com/app")).toThrow(/use https/);
    expect(() => ensureTrustedUrl("not a url")).toThrow(/not a URL/);
  });
});

describe("CLI install", () => {
  test("replaces the file atomically and leaves no temp behind", () => {
    const dir = mkdtempSync(join(tmpdir(), "voussoir-cli-update-"));
    try {
      const target = join(dir, "voussoir");
      writeFileSync(target, "old");
      installBytes(Buffer.from("new"), target);
      expect(readFileSync(target, "utf8")).toBe("new");
      expect(statSync(target).mode & 0o111).not.toBe(0);
      expect(readdirSync(dir).filter((name) => name.includes(".tmp"))).toEqual([]);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe("voussoir update", () => {
  test("installs a newer, verified artifact", async () => {
    const artifact = Buffer.from("cli-binary-v2");
    const key = platformKey();
    let port = 0;
    const server = Bun.serve({
      port: 0,
      fetch(request: Request): Response {
        const path = new URL(request.url).pathname;
        if (path === "/feed") {
          return Response.json({
            version: "9.9.9",
            downloads: {
              [key]: {
                url: `http://127.0.0.1:${port}/app`,
                sha256: createHash("sha256").update(artifact).digest("hex"),
              },
            },
          });
        }
        if (path === "/app") return new Response(artifact);
        return new Response("not found", { status: 404 });
      },
    });
    port = server.port ?? 0;
    const dir = mkdtempSync(join(tmpdir(), "voussoir-cli-update-"));
    try {
      const target = join(dir, "voussoir");
      writeFileSync(target, "old");
      const log = console.log;
      console.log = () => {};
      try {
        await update(
          parseArgs(["update", "--feed", `http://127.0.0.1:${server.port}/feed`, "--out", target]),
        );
      } finally {
        console.log = log;
      }
      expect(readFileSync(target)).toEqual(artifact);
    } finally {
      rmSync(dir, { recursive: true, force: true });
      server.stop(true);
    }
  });

  test("a failed hash leaves the old CLI in place", async () => {
    const artifact = Buffer.from("cli-binary-v2");
    let port = 0;
    const server = Bun.serve({
      port: 0,
      fetch(request: Request): Response {
        const path = new URL(request.url).pathname;
        if (path === "/feed") {
          return Response.json({
            version: "9.9.9",
            url: `http://127.0.0.1:${port}/app`,
            sha256: "0".repeat(64),
          });
        }
        return new Response(artifact);
      },
    });
    port = server.port ?? 0;
    const dir = mkdtempSync(join(tmpdir(), "voussoir-cli-update-"));
    try {
      const target = join(dir, "voussoir");
      writeFileSync(target, "old");
      await expect(
        update(parseArgs(["update", "--feed", `http://127.0.0.1:${server.port}/feed`, "--out", target])),
      ).rejects.toThrow(/does not match its sha256/);
      expect(readFileSync(target, "utf8")).toBe("old");
    } finally {
      rmSync(dir, { recursive: true, force: true });
      server.stop(true);
    }
  });

  test("--check does not download or replace", async () => {
    const server = Bun.serve({
      port: 0,
      fetch() {
        return Response.json({ version: "9.9.9", url: "http://127.0.0.1/x", sha256: "0".repeat(64) });
      },
    });
    const dir = mkdtempSync(join(tmpdir(), "voussoir-cli-update-"));
    try {
      const target = join(dir, "voussoir");
      writeFileSync(target, "old");
      const logged: string[] = [];
      const log = console.log;
      console.log = (...parts: unknown[]) => logged.push(parts.join(" "));
      try {
        await update(
          parseArgs(["update", "--feed", `http://127.0.0.1:${server.port}/feed`, "--check", "--out", target]),
        );
      } finally {
        console.log = log;
      }
      expect(logged.join("\n")).toContain("update available");
      expect(readFileSync(target, "utf8")).toBe("old");
    } finally {
      rmSync(dir, { recursive: true, force: true });
      server.stop(true);
    }
  });
});
