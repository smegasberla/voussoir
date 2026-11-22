import { expect, test } from "bun:test";
import { ensureDiskSpace } from "./pipeline.ts";

// The volume matters, not `/`: this asserted the wrong mount for any project
// living on another one. `df` does not exist on Windows, where the check is
// skipped instead of throwing.
const hasDf = Boolean(Bun.which("df"));

test("a small requirement never fails", () => {
  expect(() => ensureDiskSpace(0)).not.toThrow();
});

test("an impossible requirement reports the volume the build uses", () => {
  if (!hasDf) return;
  expect(() => ensureDiskSpace(Number.MAX_SAFE_INTEGER)).toThrow(/CARGO_TARGET_DIR|MB free/);
});
