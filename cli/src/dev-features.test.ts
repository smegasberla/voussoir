import { describe, expect, test } from "bun:test";
import { featuresFor } from "./commands/dev.ts";

describe("dev feature flags", () => {
  test("a scaffold with hot reload also gets the inspector", () => {
    expect(featuresFor(true)).toEqual(["cargo", "run", "--features", "hot-reload,devtools"]);
  });

  test("an old scaffold runs plain cargo, which never fails on an unknown feature", () => {
    expect(featuresFor(false)).toEqual(["cargo", "run"]);
  });
});
