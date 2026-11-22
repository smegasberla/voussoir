import { describe, expect, test } from "bun:test";
import { releaseSizeArgs, withReleaseSize } from "./pipeline.ts";

describe("release size defaults", () => {
  test("LTO, one codegen unit and stripping are always passed", () => {
    expect(releaseSizeArgs()).toEqual([
      "--config",
      "profile.release.lto=true",
      "--config",
      "profile.release.codegen-units=1",
      "--config",
      "profile.release.strip=true",
    ]);
  });
});

describe("release size flags travel with every cargo build", () => {
  test("desktop and mobile invocations are extended, not replaced", () => {
    expect(withReleaseSize(["cargo", "build", "--release", "--target", "aarch64-apple-ios"])).toEqual([
      "cargo",
      "build",
      "--release",
      "--target",
      "aarch64-apple-ios",
      "--config",
      "profile.release.lto=true",
      "--config",
      "profile.release.codegen-units=1",
      "--config",
      "profile.release.strip=true",
    ]);
  });
});
