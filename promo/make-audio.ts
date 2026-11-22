// Generates public/music.wav: a rhythmic, keynote-style bed, synced to the cut.
//
// The timeline is 48 s over eight scenes starting at 0, 5, 11, 17, 24, 30, 36,
// 42 s (matching the durations in src/scenes.tsx). At 120 BPM every one of
// those is a whole number of beats, so every scene lands on a bar line and the
// audio can hit exactly where the video cuts.
//
// Layers: kick on the beat, off-beat hats that enter with the third scene, a
// plucked 16th-note arpeggio following the chord, a low root, a quiet pad, a
// riser into every scene change, and an impact on the changes themselves.
// Run: bun make-audio.ts
import { writeFileSync } from "node:fs";

const SR = 44100;
const DUR = 48;
const BPM = 120;
const BEAT = 60 / BPM; // 0.5 s
const N = SR * DUR;

// Scene boundaries (s) — must match src/scenes.tsx durations.
const CUTS = [0, 5, 11, 17, 24, 30, 36, 42, 48];

// Chord per scene: [bass, pad1, pad2, arp notes...]. Am, F, C, G cycle.
const CHORDS: { bass: number; pad: number[]; arp: number[] }[] = [
  { bass: 55.0, pad: [220.0, 261.63, 329.63], arp: [440.0, 523.25, 659.26] }, // Am
  { bass: 43.65, pad: [174.61, 220.0, 261.63], arp: [349.23, 440.0, 523.25] }, // F
  { bass: 65.41, pad: [261.63, 329.63, 392.0], arp: [523.25, 659.26, 783.99] }, // C
  { bass: 49.0, pad: [196.0, 246.94, 293.66], arp: [392.0, 493.88, 587.33] }, // G
  { bass: 55.0, pad: [220.0, 261.63, 329.63], arp: [440.0, 523.25, 659.26] },
  { bass: 43.65, pad: [174.61, 220.0, 261.63], arp: [349.23, 440.0, 523.25] },
  { bass: 65.41, pad: [261.63, 329.63, 392.0], arp: [523.25, 659.26, 783.99] },
  { bass: 49.0, pad: [196.0, 246.94, 293.66], arp: [392.0, 493.88, 587.33] },
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

// Scene index at time t.
const sceneAt = (t: number) => {
  for (let s = CUTS.length - 1; s >= 0; s--) if (t >= CUTS[s]) return Math.min(s, 7);
  return 0;
};

for (let b = 0; b * BEAT < DUR; b++) {
  const t = b * BEAT;
  const s = sceneAt(t);
  const ch = CHORDS[s];

  // Kick on every beat, softer in the intro scene, full from scene 2 on.
  add(t, (p) => {
    const f = 42 + 90 * Math.exp(-p * 26);
    const ph = 2 * Math.PI * (f / 26) * (1 - Math.exp(-p * 26)) * (1 / 1);
    const amp = Math.exp(-p * 16);
    const v = Math.sin(ph) * amp * 0.5;
    return [v, v];
  }, 0.3, s === 0 ? 0.5 : 1);

  // Off-beat hats, entering at scene 3 (17 s) — the moment the code appears.
  if (s >= 2) {
    add(t + BEAT / 2, (p) => {
      const n = Math.random() * 2 - 1;
      const amp = Math.exp(-p * 90) * (s >= 4 ? 0.09 : 0.06);
      return [n * amp, -n * amp];
    }, 0.06);
  }

  // Bass: root note sustained through the beat, swells between scene halves.
  add(t, (p, u) => {
    const gate = Math.min(1, p / 0.01) * Math.exp(-p * 1.4) + 0.4 * Math.exp(-p * 0.4);
    const v = (Math.sin(2 * Math.PI * ch.bass * p) + 0.35 * Math.sin(4 * Math.PI * ch.bass * p)) * gate * 0.3;
    return [v, v];
  }, BEAT);

  // Arp: 16th-note plucks, up-down pattern; octave shimmer in scenes 5-7.
  const steps = 4;
  const pattern = [0, 1, 2, 1];
  for (let k = 0; k < steps; k++) {
    const note = ch.arp[pattern[k % pattern.length]] * (s >= 4 && k % 4 === 2 ? 2 : 1);
    add(t + (k * BEAT) / steps, (p) => {
      const v = Math.sin(2 * Math.PI * note * p) * Math.exp(-p * 11) * 0.14;
      return [v * 0.95, v];
    }, 0.28);
  }
}

// Pad: quiet chord bed per scene, swells in and out so cuts feel placed.
for (let s = 0; s < 8; s++) {
  const start = CUTS[s];
  const len = CUTS[s + 1] - CUTS[s];
  for (const f of CHORDS[s].pad) {
    add(start, (p) => {
      const e =
        Math.min(1, p / 1.4) * Math.min(1, Math.max(0, (len - p) / 1.6)) * 0.05;
      const v =
        Math.sin(2 * Math.PI * f * p) + 0.2 * Math.sin(2 * Math.PI * f * 2.003 * p);
      return [v * e, v * e * 0.96];
    }, len + 1.4);
  }
}

// Riser into each scene change (not into the first, not past the end), plus an
// impact exactly on each cut.
for (let s = 1; s < 8; s++) {
  const cut = CUTS[s];
  add(cut - 1.5, (p, u) => {
    const n = Math.random() * 2 - 1;
    const amp = u * u * 0.05;
    return [n * amp, -n * amp];
  }, 1.5);
  add(cut, (p) => {
    const boom = Math.sin(2 * Math.PI * 58 * p) * Math.exp(-p * 6) * 0.34;
    const splash = (Math.random() * 2 - 1) * Math.exp(-p * 14) * 0.09;
    return [boom + splash, boom + splash * 0.9];
  }, 1.2);
}

// Global polish: 0.8 s fade-in, 2.5 s fade-out, soft clip.
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
