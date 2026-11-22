import { describe, expect, test } from "bun:test";
import { applyPatch, createPatch } from "./delta.ts";

function roundTrip(old: Buffer, next: Buffer): Buffer {
  return applyPatch(old, createPatch(old, next));
}

describe("patch format", () => {
  test("rebuilds a changed binary", () => {
    const old = Buffer.from("the old binary with some shared tail");
    const next = Buffer.from("the new binary with some shared tail");
    expect(roundTrip(old, next)).toEqual(next);
  });

  test("an unchanged file costs almost nothing", () => {
    const body = Buffer.from("x".repeat(200_000));
    const patch = createPatch(body, body);
    expect(roundTrip(body, body)).toEqual(body);
    expect(patch.length).toBeLessThan(100);
  });

  test("empty inputs are handled", () => {
    expect(roundTrip(Buffer.alloc(0), Buffer.from("new"))).toEqual(Buffer.from("new"));
    expect(roundTrip(Buffer.from("old"), Buffer.alloc(0))).toEqual(Buffer.alloc(0));
  });

  test("a hostile patch is rejected", () => {
    expect(() => applyPatch(Buffer.from("old"), Buffer.from("NOPE"))).toThrow(/VSD1/);
    const copyPastEnd = Buffer.concat([
      Buffer.from("VSD1"),
      Buffer.from([1]),
      Buffer.from([0, 0, 0, 0, 99, 0, 0, 0]),
    ]);
    expect(() => applyPatch(Buffer.from("old"), copyPastEnd)).toThrow(/past the old binary/);
    const unknown = Buffer.concat([Buffer.from("VSD1"), Buffer.from([9])]);
    expect(() => applyPatch(Buffer.from("old"), unknown)).toThrow(/opcode/);
  });
});
