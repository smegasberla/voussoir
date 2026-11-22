import { describe, expect, test } from "bun:test";
import { ndkHostDir } from "./commands/android.ts";

describe("ndkHostDir", () => {
  test("picks the prebuilt directory the NDK ships for each host", () => {
    const entries = ["darwin-x86_64", "linux-x86_64", "windows-x86_64"];
    expect(ndkHostDir("linux", entries)).toBe("linux-x86_64");
    expect(ndkHostDir("darwin", entries)).toBe("darwin-x86_64");
    expect(ndkHostDir("win32", entries)).toBe("windows-x86_64");
  });

  test("prefers an arch-specific directory for the same host", () => {
    expect(ndkHostDir("darwin", ["darwin-arm64", "darwin-x86_64"])).toBe("darwin-arm64");
    expect(ndkHostDir("linux", ["linux-x86_64", "linux-aarch64"])).toBe("linux-x86_64");
  });

  test("falls back to the platform's conventional directory when the listing is empty", () => {
    expect(ndkHostDir("linux", [])).toBe("linux-x86_64");
    expect(ndkHostDir("darwin", [])).toBe("darwin-x86_64");
    expect(ndkHostDir("win32", [])).toBe("windows-x86_64");
  });

  test("never borrows another host's directory when this host has none", () => {
    // A listing with no linux entry means this host's toolchain is missing;
    // the caller then fails on the conventional path instead of trying to run
    // a Darwin or Windows binary.
    expect(ndkHostDir("linux", ["windows-x86_64", "darwin-x86_64"])).toBe("linux-x86_64");
  });
});
