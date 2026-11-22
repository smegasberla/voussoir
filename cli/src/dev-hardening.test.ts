import { describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { hotSupported, parsePort, serveErrorText, snapshot } from "./commands/dev.ts";

describe("dev port", () => {
  test("defaults to 5177", () => {
    expect(parsePort(undefined)).toBe(5177);
  });

  test("accepts a whole number, trimmed", () => {
    expect(parsePort("8080")).toBe(8080);
    expect(parsePort(" 8080 ")).toBe(8080);
  });

  test("refuses what the server would throw on", () => {
    for (const bad of ["abc", "0", "65536", "-1", "80.5", ""]) {
      expect(() => parsePort(bad)).toThrow(/port/);
    }
  });
});

describe("serve error text", () => {
  test("an occupied port points at --port", () => {
    const text = serveErrorText(5177, new Error("listen EADDRINUSE: address already in use"));
    expect(text).toContain("port 5177 is already in use");
    expect(text).toContain("--port");
  });

  test("any other failure keeps its own words", () => {
    expect(serveErrorText(80, new Error("permission denied"))).toBe(
      "could not start the dev server on port 80: permission denied",
    );
  });
});

describe("hot reload support", () => {
  test("a frontend-only scaffold has no crate to read", () => {
    const dir = mkdtempSync(join(tmpdir(), "voussoir-dev-"));
    try {
      expect(hotSupported(dir)).toBe(false);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("a pre-hot-reload crate runs plain cargo", () => {
    const dir = mkdtempSync(join(tmpdir(), "voussoir-dev-"));
    try {
      writeFileSync(join(dir, "Cargo.toml"), "[package]\nname = \"app\"\n");
      expect(hotSupported(dir)).toBe(false);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("a crate that declares the feature is watched", () => {
    const dir = mkdtempSync(join(tmpdir(), "voussoir-dev-"));
    try {
      writeFileSync(
        join(dir, "Cargo.toml"),
        "[features]\nhot-reload = [\"voussoir/hot-reload\"]\n",
      );
      expect(hotSupported(dir)).toBe(true);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe("snapshot", () => {
  test("a path that cannot be listed is unchanged, not a throw", () => {
    // The poller calls this from setInterval; an ENOTDIR used to be an uncaught
    // exception that killed the dev process.
    const dir = mkdtempSync(join(tmpdir(), "voussoir-snap-"));
    try {
      const file = join(dir, "not-a-dir.rs");
      writeFileSync(file, "fn main() {}");
      expect(snapshot(file).size).toBe(0);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  test("a missing directory is an empty map", () => {
    expect(snapshot(join(tmpdir(), "voussoir-nope", "missing")).size).toBe(0);
  });

  test("finds .rs files with their mtimes and ignores the rest", () => {
    const dir = mkdtempSync(join(tmpdir(), "voussoir-snap-"));
    try {
      mkdirSync(join(dir, "src"));
      writeFileSync(join(dir, "src", "main.rs"), "fn main() {}");
      writeFileSync(join(dir, "src", "notes.txt"), "not rust");
      const found = snapshot(dir);
      expect(found.size).toBe(1);
      expect([...found.keys()][0]).toBe(join(dir, "src", "main.rs"));
      expect(Number.isFinite([...found.values()][0])).toBe(true);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
