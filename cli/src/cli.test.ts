import { describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const entry = join(import.meta.dir, "index.ts");

function runCli(args: string[], cwd: string) {
  const result = Bun.spawnSync(["bun", entry, ...args], {
    cwd,
    stdout: "pipe",
    stderr: "pipe",
  });
  return {
    code: result.exitCode,
    output: result.stdout.toString() + result.stderr.toString(),
  };
}

describe("command errors", () => {
  test("an expected failure is one clean line, not a stack trace", () => {
    const cwd = mkdtempSync(join(tmpdir(), "voussoir-cli-"));
    try {
      const { code, output } = runCli(["init", "Notes", "--package", "com.Bad"], cwd);
      expect(code).toBe(1);
      expect(output).toContain("voussoir: ");
      expect(output).toContain("is not a valid package");
      expect(output).not.toContain("    at ");
    } finally {
      rmSync(cwd, { recursive: true, force: true });
    }
  });

  test("a failed command writes nothing into the project directory", () => {
    const cwd = mkdtempSync(join(tmpdir(), "voussoir-cli-"));
    try {
      runCli(["init", "Notes", "--package", "com.Bad"], cwd);
      expect(readNames(cwd)).toEqual([]);
    } finally {
      rmSync(cwd, { recursive: true, force: true });
    }
  });
});

function readNames(cwd: string): string[] {
  try {
    return [...new Bun.Glob("*").scanSync({ cwd, onlyFiles: false })];
  } catch {
    return [];
  }
}
