// Generates public/music.wav: a quiet bed plus a layer of designed sound
// effects, synced to the cut.
//
// The bed stays out of the way (pads, a soft pulse, a sparse arp); the event
// layer does the storytelling: a whoosh and an impact on every scene cut,
// typing ticks under every typed line, pops for chips, dings for results, and
// a bigger final hit into the outro. The video is 55 s over nine scenes
// starting at 0, 5, 11, 17, 24, 31, 37, 43, 49 s (matching src/scenes.tsx at
// 120 BPM, so every cut lands on a bar).
//
// Cue times below are absolute seconds, derived from each scene's frame
// offsets; keep them in sync with src/scenes.tsx when timing changes.
// Run: bun make-audio.ts
import { writeFileSync } from "node:fs";

const SR = 44100;
const BPM = 120;
const BEAT = 60 / BPM; // 0.5 s

// Scene boundaries (s) — must match src/scenes.tsx durations. The track lasts
// exactly as long as the video: the last cut is the end.
const CUTS = [0, 5, 11, 17, 24, 31, 37, 43, 49, 55];
const DUR = CUTS[CUTS.length - 1];
const N = SR * DUR;

// Chord per scene: [bass, pad, arp]. Am, F, C, G cycle; the outro is Cadd9.
const CHORDS: { bass: number; pad: number[]; arp: number[] }[] = [
  { bass: 55.0, pad: [220.0, 261.63, 329.63], arp: [440.0, 523.25, 659.26] }, // Am
  { bass: 43.65, pad: [174.61, 220.0, 261.63], arp: [349.23, 440.0, 523.25] }, // F
  { bass: 65.41, pad: [261.63, 329.63, 392.0], arp: [523.25, 659.26, 783.99] }, // C
  { bass: 49.0, pad: [196.0, 246.94, 293.66], arp: [392.0, 493.88, 587.33] }, // G
  { bass: 55.0, pad: [220.0, 261.63, 329.63], arp: [440.0, 523.25, 659.26] }, // Am
  { bass: 43.65, pad: [174.61, 220.0, 261.63], arp: [349.23, 440.0, 523.25] }, // F
  { bass: 65.41, pad: [261.63, 329.63, 392.0], arp: [523.25, 659.26, 783.99] }, // C
  { bass: 49.0, pad: [196.0, 246.94, 293.66], arp: [392.0, 493.88, 587.33] }, // G
  { bass: 65.41, pad: [261.63, 329.63, 392.0, 587.33], arp: [523.25, 587.33, 783.99] }, // Cadd9
];

const left = new Float64Array(N);
const right = new Float64Array(N);

const add = (t: number, gen: (p: number, ti: number) => [number, number], dur: number, w = 1) => {
  const i0 = Math.floor(t * SR);
  const n = Math.floor(dur * SR);
  for (let j = 0; j < n; j++) {
    const i = i0 + j;
    if (i < 0 || i >= N) continue;
    const [l, r] = gen(j / SR, j / n);
    left[i] += l * w;
    right[i] += r * w;
  }
};

const sceneAt = (t: number) => {
  for (let s = CUTS.length - 1; s >= 0; s--) if (t >= CUTS[s]) return Math.min(s, 8);
  return 0;
};

// ---------------------------------------------------------------- bed

for (let b = 0; b * BEAT < DUR; b++) {
  const t = b * BEAT;
  const s = sceneAt(t);
  const ch = CHORDS[s];

  // Pulse: present, but well under the type.
  add(t, (p) => {
    const f = 42 + 90 * Math.exp(-p * 26);
    const ph = 2 * Math.PI * (f / 26) * (1 - Math.exp(-p * 26));
    const v = Math.sin(ph) * Math.exp(-p * 16) * 0.22;
    return [v, v];
  }, 0.3, s === 0 ? 0.7 : s === 8 ? 0.6 : 1);

  // Bass root, gentle.
  add(t, (p) => {
    const gate = Math.min(1, p / 0.01) * Math.exp(-p * 1.4) + 0.4 * Math.exp(-p * 0.4);
    const v =
      (Math.sin(2 * Math.PI * ch.bass * p) + 0.35 * Math.sin(4 * Math.PI * ch.bass * p)) *
      gate *
      0.18;
    return [v, v];
  }, BEAT);

  // Sparse arp: only the two strong 8ths. The outro rolls its own shape.
  const outro = s === 8;
  const steps = 4;
  const pattern = outro ? [0, 2, 1, 2] : [0, 1, 2, 1];
  for (let k = 0; k < steps; k++) {
    if (outro && k % 2 === 1) continue;
    if (!outro && k % 2 === 1) continue;
    const note = ch.arp[pattern[k % pattern.length]];
    add(t + (k * BEAT) / steps, (p) => {
      const v =
        Math.sin(2 * Math.PI * note * p) *
        Math.exp(-p * (outro ? 6 : 12)) *
        (outro ? 0.09 : 0.08);
      return [v * 0.95, v];
    }, outro ? 1.8 : 0.5);
  }
  if (outro) {
    add(t, (p) => {
      const v = Math.sin(2 * Math.PI * ch.arp[2] * 2 * p) * Math.exp(-p * 3) * 0.035;
      return [v, v * 0.9];
    }, 1.8);
  }
}

// Pad: quiet chord bed per scene.
for (let s = 0; s < 9; s++) {
  const start = CUTS[s];
  const len = CUTS[s + 1] - CUTS[s];
  for (const f of CHORDS[s].pad) {
    add(start, (p) => {
      const e = Math.min(1, p / 1.4) * Math.min(1, Math.max(0, (len - p) / 1.6)) * 0.028;
      const v = Math.sin(2 * Math.PI * f * p) + 0.2 * Math.sin(2 * Math.PI * f * 2.003 * p);
      return [v * e, v * e * 0.96];
    }, len + 1.4);
  }
}

// ---------------------------------------------------------------- SFX

// Whoosh ending exactly on the cut, then a low impact. The final one is the
// biggest, so the end card lands.
for (let s = 1; s < CUTS.length - 1; s++) {
  const cut = CUTS[s];
  const final = s === CUTS.length - 2;
  add(cut - 1.4, (p, u) => {
    const n = Math.random() * 2 - 1;
    const amp = u * u * (final ? 0.13 : 0.10);
    return [n * amp, -n * amp];
  }, 1.4);
  add(cut, (p) => {
    const boom =
      Math.sin(2 * Math.PI * (final ? 50 : 58) * p) * Math.exp(-p * (final ? 4.5 : 6)) * (final ? 0.38 : 0.28);
    const splash = (Math.random() * 2 - 1) * Math.exp(-p * 14) * (final ? 0.10 : 0.07);
    return [boom + splash, boom + splash * 0.9];
  }, final ? 1.6 : 1.2);
}

// Keyboard ticks under each typed line. Window = [start, end) in seconds;
// cps in src/scenes.tsx maps 1:1 to chars per second.
const TYPING: [number, number][] = [
  [24.933, 26.0], // commands, line 1
  [26.067, 26.75], // commands, line 2
  [37.933, 38.8], // hot reload, line 1
  [38.533, 39.4], // hot reload, line 2
  [39.133, 39.2], // hot reload, line 3
  [50.6, 52.0], // outro, unix command
  [52.267, 53.45], // outro, windows command
];
for (const [from, to] of TYPING) {
  for (let t = from; t < to; t += 0.07) {
    add(t, (p) => {
      const n = Math.random() * 2 - 1;
      const v = (Math.sin(2 * Math.PI * 2600 * p) * 0.5 + n * 0.5) * Math.exp(-p * 300) * 0.07;
      return [v, v * 0.85];
    }, 0.02);
  }
}

// Chip pops and result dings, absolute seconds.
const pop = (t: number, f = 520) =>
  add(t, (p) => {
    const v = Math.sin(2 * Math.PI * f * p) * Math.exp(-p * 34) * 0.06;
    return [v, v];
  }, 0.09);
const ding = (t: number, f = 1318) =>
  add(t, (p) => {
    const a = Math.sin(2 * Math.PI * f * p) * Math.exp(-p * 8) * 0.10;
    const b = Math.sin(2 * Math.PI * f * 1.5 * p) * Math.exp(-p * 12) * 0.05;
    return [a + b, a + b * 0.9];
  }, 0.9);

pop(1.333);
pop(1.833);
pop(2.333);
pop(29.6);
pop(32.0);
pop(32.267);
pop(32.533);
pop(32.8);
pop(33.067);
pop(33.333);
pop(43.733);
pop(44.6);
ding(27.167); // the "Hello, you" result chip
ding(41.333); // the "0 restarts" counter

// ---------------------------------------------------------------- master

for (let i = 0; i < N; i++) {
  const t = i / SR;
  let g = 1;
  if (t < 0.8) g *= t / 0.8;
  if (t > DUR - 2.5) g *= Math.max(0, (DUR - t) / 2.5);
  left[i] = Math.tanh(left[i] * g);
  right[i] = Math.tanh(right[i] * g);
}

const bytes = Buffer.alloc(44 + N * 4);
bytes.write("RIFF", 0);
bytes.writeUInt32LE(36 + N * 4, 4);
bytes.write("WAVEfmt ", 8);
bytes.writeUInt32LE(16, 16);
bytes.writeUInt16LE(1, 20);
bytes.writeUInt16LE(2, 22);
bytes.writeUInt32LE(SR, 24);
bytes.writeUInt32LE(SR * 4, 28);
bytes.writeUInt16LE(4, 32);
bytes.writeUInt16LE(16, 34);
bytes.write("data", 36);
bytes.writeUInt32LE(N * 4, 40);
for (let i = 0; i < N; i++) {
  bytes.writeInt16LE(Math.max(-32768, Math.min(32767, Math.round(left[i] * 32767))), 44 + i * 4);
  bytes.writeInt16LE(Math.max(-32768, Math.min(32767, Math.round(right[i] * 32767))), 46 + i * 4);
}
writeFileSync("public/music.wav", bytes);
console.log("public/music.wav:", (bytes.length / 1e6).toFixed(1), "MB");
