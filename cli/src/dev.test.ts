import { describe, expect, test } from "bun:test";
import { featuresFor } from "./commands/dev.ts";

describe("dev shell launch", () => {
  test("asks for hot reload when the scaffold declares it", () => {
    expect(featuresFor(true)).toEqual(["cargo", "run", "--features", "hot-reload,devtools"]);
  });

  test("runs plain cargo for a pre-hot-reload scaffold", () => {
    // The flag must disappear entirely: cargo errors on a feature the
    // manifest does not declare, which made `voussoir dev` unusable on old
    // scaffolds.
    expect(featuresFor(false)).toEqual(["cargo", "run"]);
  });
});
