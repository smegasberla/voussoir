import { describe, expect, test } from "bun:test";
import { existsSync, mkdtempSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ensureOutDir } from "./commands/build.ts";

describe("--out", () => {
  test("creates the parent directories on the first run", () => {
    const cwd = mkdtempSync(join(tmpdir(), "voussoir-out-"));
    try {
      const destination = ensureOutDir("release/linux/app", cwd);
      expect(destination).toBe(join(cwd, "release/linux/app"));
      expect(statSync(join(cwd, "release/linux")).isDirectory()).toBe(true);
      expect(existsSync(destination)).toBe(false);
    } finally {
      rmSync(cwd, { recursive: true, force: true });
    }
  });

  test("is fine when the directory already exists", () => {
    const cwd = mkdtempSync(join(tmpdir(), "voussoir-out-"));
    try {
      ensureOutDir("dist/app", cwd);
      const again = ensureOutDir("dist/app", cwd);
      expect(again).toBe(join(cwd, "dist/app"));
    } finally {
      rmSync(cwd, { recursive: true, force: true });
    }
  });

  test("a bare filename needs no directory", () => {
    const cwd = mkdtempSync(join(tmpdir(), "voussoir-out-"));
    try {
      expect(ensureOutDir("app", cwd)).toBe(join(cwd, "app"));
    } finally {
      rmSync(cwd, { recursive: true, force: true });
    }
  });
});
