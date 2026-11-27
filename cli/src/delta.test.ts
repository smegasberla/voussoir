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

/** The generator's block hash, replicated so a collision can be constructed. */
function blockHash(block: Buffer): number {
  let h = 0;
  for (const byte of block) h = (Math.imul(h, 0x01000193) + byte) >>> 0;
  return h;
}

describe("a colliding block", () => {
  test("is not copied just because its hash matched", () => {
    // A 32-bit hash over 32-byte blocks collides. The first version of the
    // bounded index trusted the hash alone and emitted a COPY of the wrong
    // block; on a real 896 KB release pair the rebuilt binary differed at byte
    // 20704 while the patch still applied cleanly. Constructing the collision is
    // the only way to keep that from coming back.
    // xorshift32, so the blocks are actually spread over the 32-bit hash space.
    // A patterned generator only visits a few hundred distinct blocks and never
    // collides, which is how the first version of this test failed to test.
    let seed = 0x9e3779b9;
    const nextWord = () => {
      seed ^= seed << 13;
      seed >>>= 0;
      seed ^= seed >>> 17;
      seed ^= seed << 5;
      seed >>>= 0;
      return seed;
    };
    const blocks = new Map<number, Buffer>();
    let pair: [Buffer, Buffer] | undefined;
    for (let attempt = 0; attempt < 400_000 && !pair; attempt++) {
      const block = Buffer.alloc(32);
      for (let i = 0; i < 32; i += 4) block.writeUInt32LE(nextWord(), i);
      const hash = blockHash(block);
      const seen = blocks.get(hash);
      if (seen && !seen.equals(block)) pair = [seen, block];
      else blocks.set(hash, block);
    }
    expect(pair).toBeDefined();
    const [a, b] = pair!;
    expect(a.equals(b)).toBe(false);
    expect(blockHash(a)).toBe(blockHash(b));

    // `a` sits at offset 0 of the old file; `b` at offset 0 of the new one, with
    // a shared tail so a correct patch copies the tail and inserts `b`.
    const tail = Buffer.from("shared tail so the patch has a copy to make".repeat(4));
    const old = Buffer.concat([a, tail]);
    const next = Buffer.concat([b, tail]);

    const patch = createPatch(old, next);
    expect(applyPatch(old, patch)).toEqual(next);
  });
});
