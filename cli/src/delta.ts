/** Patch generation for the updater.
 *
 *  The Rust side (`voussoir::delta`) applies a patch; this is the other half. The
 *  format is deliberately dumb — copy a run of the old binary, or insert new
 *  bytes — because every patch is verified against the released artifact's own
 *  hash and signature, so a cleverer format would add risk without adding trust.
 */

const MAGIC = Buffer.from("VSD1");
const COPY = 1;
const INSERT = 2;
const MAX_OUTPUT = 256 * 1024 * 1024;

function u32(value: number): Buffer {
  const out = Buffer.alloc(4);
  out.writeUInt32LE(value >>> 0, 0);
  return out;
}

/** Build `patch` such that `applyPatch(old, patch)` equals `next`. */
export function createPatch(old: Uint8Array, next: Uint8Array): Buffer {
  const oldBuf = Buffer.from(old);
  const nextBuf = Buffer.from(next);
  const block = 32;

  // Candidate offsets for each 32-byte run of the old binary. The cap keeps a
  // pathological file (all zeros) from turning this into a quadratic scan.
  const index = new Map<string, number[]>();
  for (let i = 0; i + block <= oldBuf.length; i++) {
    const key = oldBuf.subarray(i, i + block).toString("base64");
    const list = index.get(key);
    if (!list) index.set(key, [i]);
    else if (list.length < 16) list.push(i);
  }

  const parts: Buffer[] = [MAGIC];
  let pending: Buffer[] = [];
  let pendingLength = 0;
  const flush = () => {
    if (pendingLength === 0) return;
    parts.push(Buffer.from([INSERT]), u32(pendingLength), ...pending);
    pending = [];
    pendingLength = 0;
  };

  let i = 0;
  while (i < nextBuf.length) {
    let bestLength = 0;
    let bestOffset = 0;
    if (i + block <= nextBuf.length) {
      const key = nextBuf.subarray(i, i + block).toString("base64");
      for (const offset of index.get(key) ?? []) {
        let length = block;
        while (
          offset + length < oldBuf.length &&
          i + length < nextBuf.length &&
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
    if (bestLength >= block) {
      flush();
      parts.push(Buffer.from([COPY]), u32(bestOffset), u32(bestLength));
      i += bestLength;
    } else {
      pending.push(nextBuf.subarray(i, i + 1));
      pendingLength++;
      i++;
      if (pendingLength >= 1 << 20) flush();
    }
  }
  flush();
  return Buffer.concat(parts);
}

/** Apply `patch` to `old`. Mirrors `voussoir::delta::apply`; used by the tests. */
export function applyPatch(old: Uint8Array, patch: Uint8Array): Buffer {
  const oldBuf = Buffer.from(old);
  let cursor = Buffer.from(patch);
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
