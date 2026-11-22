import { describe, expect, test } from "bun:test";
import { streamsLogs } from "./commands/android.ts";
import { parseArgs } from "./args.ts";

describe("android run --wait", () => {
  test("is a launcher by default", () => {
    expect(streamsLogs(parseArgs(["run"]))).toBe(false);
  });

  test("stays attached when asked", () => {
    expect(streamsLogs(parseArgs(["run", "--wait"]))).toBe(true);
  });
});
