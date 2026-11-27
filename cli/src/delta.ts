/** Patch generation for the updater.
 *
 *  The Rust side (`voussoir::delta`) applies a patch; this is the other half. The
 *  format is deliberately dumb — copy a run of the old binary, or insert new
 *  bytes — because every patch is verified against the released artifact's own
 *  hash and signature, so a cleverer format would add risk without adding trust.
 */

import { deflateRawSync, inflateRawSync } from "node:zlib";

const MAGIC = Buffer.from("VSD1");
/** Deflated `VSD1` operations. Two magics, so a reader that predates this keeps working. */
const COMPRESSED_MAGIC = Buffer.from("VSC1");
const COPY = 1;
const INSERT = 2;
const MAX_OUTPUT = 256 * 1024 * 1024;

function u32(value: number): Buffer {
  const out = Buffer.alloc(4);
  out.writeUInt32LE(value >>> 0, 0);
  return out;
}

/** Hash helpers for the index.
 *
 *  A 94 MB binary has ~94M 32-byte windows, and a `Map` keyed by base64 strings
 *  needed gigabytes to hold them: the first real release ran out of memory. A
 *  fixed-size open-addressed table in two typed arrays bounds it at ~32 MB, and
 *  a collision costs a longer patch, never a wrong one, because every copy is
 *  checked by extending it against the real bytes.
 */
const BLOCK = 32;
const MAX_SLOTS = 1 << 22;
const PROBES = 8;
const BASE = 0x01000193;
const POW = (() => {
  let p = 1;
  for (let i = 0; i < BLOCK - 1; i++) p = Math.imul(p, BASE) >>> 0;
  return p >>> 0;
})();

function hashAt(buf: Buffer, at: number): number {
  let h = 0;
  for (let i = 0; i < BLOCK; i++) h = (Math.imul(h, BASE) + buf[at + i]!) >>> 0;
  return h;
}

/** Slide the window one byte: drop `out`, add `next`. */
function roll(h: number, out: number, next: number): number {
  return (Math.imul((h - Math.imul(out, POW)) >>> 0, BASE) + next) >>> 0;
}

/** Slot count: one per sampled position where possible, capped at 32 MB. */
function slotsFor(length: number): number {
  let bits = 16;
  while (bits < MAX_SLOTS && 1 << bits < length) bits++;
  return Math.min(1 << bits, MAX_SLOTS);
}

/** Build `patch` such that `applyPatch(old, patch)` equals `next`. */
export function createPatch(old: Uint8Array, next: Uint8Array): Buffer {
  const oldBuf = Buffer.from(old);
  const nextBuf = Buffer.from(next);
  const oldLen = oldBuf.length;
  const nextLen = nextBuf.length;

  const slots = slotsFor(oldLen);
  const mask = slots - 1;
  const hashes = new Uint32Array(slots);
  const offsets = new Int32Array(slots).fill(-1);

  const put = (h: number, offset: number) => {
    const start = h & mask;
    for (let probe = 0; probe < PROBES; probe++) {
      const at = (start + probe) & mask;
      if (offsets[at] === -1) {
        hashes[at] = h;
        offsets[at] = offset;
        return;
      }
      if (hashes[at] === h) return; // first occurrence wins: the lowest offset
    }
  };

  if (oldLen >= BLOCK) {
    // Sampling keeps the index inside its cap; a stride of 1 covers small files
    // and a large one still gets an anchor every few dozen bytes.
    const stride = Math.max(1, Math.ceil(oldLen / slots));
    let h = hashAt(oldBuf, 0);
    for (let i = 0; i + BLOCK <= oldLen; i++) {
      if (i % stride === 0) put(h, i);
      if (i + BLOCK < oldLen) h = roll(h, oldBuf[i]!, oldBuf[i + BLOCK]!);
    }
  }

  const parts: Buffer[] = [MAGIC];
  let insertStart = -1;
  const flushInsert = (end: number) => {
    if (insertStart < 0) return;
    parts.push(
      Buffer.from([INSERT]),
      u32(end - insertStart),
      Buffer.from(nextBuf.subarray(insertStart, end)),
    );
    insertStart = -1;
  };

  let i = 0;
  let h = nextLen >= BLOCK ? hashAt(nextBuf, 0) : 0;
  while (i < nextLen) {
    let bestLength = 0;
    let bestOffset = 0;
    if (i + BLOCK <= nextLen) {
      const start = h & mask;
      for (let probe = 0; probe < PROBES; probe++) {
        const at = (start + probe) & mask;
        if (offsets[at] === -1) break;
        if (hashes[at] !== h) continue;
        const offset = offsets[at]!;
        // The hash only narrows the candidates: a 32-bit digest collides, and a
        // false match copied the wrong block while the tail happened to agree
        // (found on a real 896 KB release pair). The block itself must match
        // before it can be copied. It always fits, since only offsets with a
        // full block were indexed.
        let same = true;
        for (let k = 0; k < BLOCK; k++) {
          if (oldBuf[offset + k] !== nextBuf[i + k]) {
            same = false;
            break;
          }
        }
        if (!same) continue;

        let length = BLOCK;
        while (
          offset + length < oldLen &&
          i + length < nextLen &&
          oldBuf[offset + length] === nextBuf[i + length]
        ) {
          length++;
        }
        if (length > bestLength) {
          bestLength = length;
          bestOffset = offset;
        }
      }
    }
    if (bestLength >= BLOCK) {
      flushInsert(i);
      parts.push(Buffer.from([COPY]), u32(bestOffset), u32(bestLength));
      i += bestLength;
      // The window jumped, so the rolling hash cannot slide into place.
      if (i + BLOCK <= nextLen) h = hashAt(nextBuf, i);
    } else {
      if (insertStart < 0) insertStart = i;
      i++;
      if (i + BLOCK <= nextLen) h = roll(h, nextBuf[i - 1]!, nextBuf[i + BLOCK - 1]!);
    }
  }
  flushInsert(nextLen);

  // Raw deflate, no container: the payload is the only thing in the file, so a
  // gzip/zlib header would be bytes for nothing. A patch is mostly inserted
  // machine code with long repeats, which is why this is worth doing: on a real
  // 962 KB release pair a build with genuine code changes produced 435 KB raw
  // and 217 KB deflated, 45% of the artifact down to 23%.
  const raw = Buffer.concat(parts);
  const deflated = deflateRawSync(raw, { level: 9 });
  // A tiny patch (a one-constant rebuild is ~60 bytes) can grow under deflate's
  // own overhead, so the smaller of the two wins and the magic says which.
  if (deflated.length >= raw.length) return raw;
  return Buffer.concat([COMPRESSED_MAGIC, deflated]);
}

/** Apply `patch` to `old`. Mirrors `voussoir::delta::apply`; used by the tests. */
export function applyPatch(old: Uint8Array, patch: Uint8Array): Buffer {
  const oldBuf = Buffer.from(old);
  let cursor = Buffer.from(patch);
  if (cursor.length >= 4 && cursor.subarray(0, 4).equals(COMPRESSED_MAGIC)) {
    // The cap is applied after inflation as well as by the inflater itself, so a
    // crafted patch cannot expand past what the applier accepts.
    cursor = inflateRawSync(cursor.subarray(4), { maxOutputLength: MAX_OUTPUT + 4 });
  } else if (!(cursor.length >= 4 && cursor.subarray(0, 4).equals(MAGIC))) {
    throw new Error("not a Voussoir patch (missing VSD1 header)");
  }
  if (cursor.length < 4 || !cursor.subarray(0, 4).equals(MAGIC)) {
    throw new Error("not a Voussoir patch (missing VSD1 header)");
  }
  cursor = cursor.subarray(4);
  const out: Buffer[] = [];
  let length = 0;
  while (cursor.length > 0) {
    const tag = cursor[0]!;
    cursor = cursor.subarray(1);
    if (tag === COPY) {
      if (cursor.length < 8) throw new Error("patch ends in a copy header");
      const offset = cursor.readUInt32LE(0);
      const count = cursor.readUInt32LE(4);
      cursor = cursor.subarray(8);
      if (offset + count > oldBuf.length) throw new Error("copy reads past the old binary");
      if (length + count > MAX_OUTPUT) throw new Error("patch output exceeds the cap");
      out.push(oldBuf.subarray(offset, offset + count));
      length += count;
    } else if (tag === INSERT) {
      if (cursor.length < 4) throw new Error("patch ends in an insert header");
      const count = cursor.readUInt32LE(0);
      cursor = cursor.subarray(4);
      if (count > cursor.length) throw new Error("insert is shorter than its length");
      if (length + count > MAX_OUTPUT) throw new Error("patch output exceeds the cap");
      out.push(cursor.subarray(0, count));
      length += count;
      cursor = cursor.subarray(count);
    } else {
      throw new Error(`unknown opcode ${tag}`);
    }
  }
  return Buffer.concat(out);
}
