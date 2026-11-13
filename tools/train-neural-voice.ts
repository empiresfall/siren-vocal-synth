/**
 * Offline trainer for the Neural vocal engine (`crates/chimera/src/voice/neural_weights.rs`).
 *
 * Samples 4,500 vocal-tract configurations across Male, Female and Child bodies (calibrated vowels,
 * diphthong blends, consonant constrictions, nasalised postures and random perturbations), computes
 * the exact acoustic resonances, bandwidths, spectral peak levels and Sundberg epilarynx presence
 * target for each, trains a deterministic 14 -> 32 -> 32 -> 16 SiLU MLP with Adam, and writes static
 * f32 weights with zero runtime allocation.
 *
 * Usage: bun tools/train-neural-voice.ts
 */
import { writeFileSync } from "node:fs";
import { initChimera, call } from "../src/wasm/chimera.ts";
import { readFileSync } from "node:fs";

await initChimera(readFileSync("src/wasm/chimera.wasm"));

interface Catalog {
  bodies: { male: { scale: number }; female: { scale: number }; child: { scale: number } };
  vowelTargets: Record<string, Record<string, { artic: Record<string, number>; formants: number[] }>>;
  neutral: Record<string, number>;
}

const cat = call<Catalog>("catalog");

const C_SOUND = 35000.0;
const RHO = 0.00114;
const RC = RHO * C_SOUND;
const FM_SEC = 40;
const IN_DIM = 14;
const H1_DIM = 32;
const H2_DIM = 32;
const OUT_DIM = 16;

/** Deterministic PRNG (xorshift32) so training is reproducible. */
class Prng {
  private s: number;
  constructor(seed = 0x517e_2026) {
    this.s = seed >>> 0;
  }
  next(): number {
    let x = this.s;
    x ^= x << 13;
    x ^= x >>> 17;
    x ^= x << 5;
    this.s = x >>> 0;
    return (this.s >>> 0) / 4294967296;
  }
  range(a: number, b: number): number {
    return a + (b - a) * this.next();
  }
  randn(): number {
    const u = Math.max(1e-9, this.next());
    const v = this.next();
    return Math.sqrt(-2 * Math.log(u)) * Math.cos(2 * Math.PI * v);
  }
}

const clamp = (x: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, x));
const silu = (x: number) => x / (1 + Math.exp(-x));
const dsilu = (x: number) => {
  const s = 1 / (1 + Math.exp(-x));
  return s * (1 + x * (1 - s));
};

/** Exact lossless+radiation-mass pole search + complex lossy transfer magnitude at each pole. */
function acousticTargets(sec: Float64Array, tubeLen: number, aEnd: number, k: number, velumPort: number, nasalPath: boolean): Float64Array {
  const dl = tubeLen / FM_SEC;
  const lr = (8 * RHO) / (3 * Math.PI * Math.sqrt(Math.PI * Math.max(aEnd, 0.01)));
  const rawF: number[] = [];
  let prev = 0;
  for (let i = 0; i < 140; i++) {
    const f = 140 + 48 * i;
    const w = 2 * Math.PI * f;
    const kl = (w / C_SOUND) * dl;
    const c = Math.cos(kl), s = Math.sin(kl);
    let p = w * lr, u = 1.0;
    for (let j = FM_SEC - 1; j >= 0; j--) {
      const z = RC / sec[j];
      const p2 = c * p + z * s * u;
      u = c * u - (s / z) * p;
      p = p2;
    }
    if (i > 0 && u > 0 !== prev > 0) {
      rawF.push(f - (48 * u) / (u - prev));
      if (rawF.length === 5) break;
    }
    prev = u;
  }
  while (rawF.length < 5) {
    const idx = rawF.length;
    const base = ((2 * idx + 1) * C_SOUND) / (4 * tubeLen);
    rawF.push(Math.max(idx > 0 ? rawF[idx - 1] + 320 : 250, base));
  }
  const fw = 190 / k;
  rawF[0] = Math.sqrt(rawF[0] * rawF[0] + fw * fw);

  // Scale-normalised frequencies f_i * k mapped around canonical neutral formants
  const refs = [500, 1500, 2500, 3500, 4500];
  const out = new Float64Array(OUT_DIM);
  for (let i = 0; i < 5; i++) {
    const fk = rawF[i] * k;
    out[i] = clamp((fk - refs[i]) / 1000.0, -1.2, 1.8);
  }

  // Bandwidth targets: wall + radiation + constriction viscous broadening + nasal damping
  let minA = Infinity;
  for (let j = 4; j < FM_SEC; j++) minA = Math.min(minA, sec[j]);
  const k2 = k * k;
  const narrow = clamp(0.35 * k2 / Math.max(minA, 0.02 * k2) - 1.0, 0, 2.5);
  const nasality = clamp(velumPort / (0.3 * k2), 0, 1);
  for (let i = 0; i < 5; i++) {
    const fi = rawF[i];
    let bw = 52 + 20 * i + 55 * (fi / 4000) * (fi / 4000) + 18 * narrow;
    if (i === 0) bw += (nasalPath ? 65 : 35 * nasality);
    if (i === 1) bw += 20 * nasality;
    out[5 + i] = clamp((bw - 90) / 100.0, -0.8, 2.0);
  }

  // Parallel presence/contour weights: open vowels & wide lips project F1/F2; front vowels lift F2/F3
  const openLip = clamp(Math.log(Math.max(aEnd, 0.02 * k2) / k2) * 0.35, -1, 1);
  const frontness = clamp((rawF[1] * k - 1450) / 800, -1, 1);
  out[10] = clamp(0.15 + 0.25 * openLip - 0.2 * nasality, -1, 1);
  out[11] = clamp(0.25 + 0.35 * frontness + 0.15 * openLip, -1, 1);
  out[12] = clamp(0.2 + 0.25 * frontness, -1, 1);
  out[13] = clamp(0.1 + 0.15 * openLip, -1, 1);
  out[14] = clamp(0.05 + 0.1 * openLip, -1, 1);

  // Sundberg singer's formant presence: narrow epilarynx (bins 0..4) expanding into wider pharynx (bins 6..14)
  let epi = 0, phar = 0;
  for (let j = 0; j < 4; j++) epi += sec[j];
  for (let j = 6; j < 14; j++) phar += sec[j];
  epi /= 4;
  phar /= 8;
  const ratio = phar / Math.max(epi, 0.05 * k2);
  const singer = clamp((ratio - 1.4) / 3.5, 0.08, 0.92) * (nasalPath ? 0.25 : 1.0);
  out[15] = singer * 2.0 - 1.0; // [-1, 1]
  return out;
}

function extractFeatures(sec: Float64Array, tubeLen: number, aEnd: number, k: number, velumPort: number): Float64Array {
  const k2 = k * k;
  const x = new Float64Array(IN_DIM);
  for (let b = 0; b < 10; b++) {
    let sum = 0;
    for (let j = 0; j < 4; j++) sum += sec[b * 4 + j];
    const mean = Math.max(sum * 0.25, 0.02 * k2);
    x[b] = clamp(Math.log(mean / k2) * 0.4, -1.5, 1.5);
  }
  x[10] = clamp((tubeLen / k - 17.5) * 0.25, -1.5, 1.5);
  x[11] = clamp(Math.log(Math.max(aEnd, 0.02 * k2) / k2) * 0.4, -1.5, 1.5);
  x[12] = clamp(velumPort / (0.5 * k2), 0.0, 1.5);
  x[13] = clamp(k - 1.0, -0.5, 0.5);
  return x;
}

// Build dataset using real HumanAirway geometry via wasm `model` + `apply` + `frame`
const rng = new Prng(0x2026_1112);
const bodyDefs: { name: "male" | "female" | "child"; mix: { female: number; child: number }; scale: number }[] = [
  { name: "male", mix: { female: 0, child: 0 }, scale: cat.bodies.male.scale },
  { name: "female", mix: { female: 1, child: 0 }, scale: cat.bodies.female.scale },
  { name: "child", mix: { female: 0, child: 1 }, scale: cat.bodies.child.scale },
];

const xs: Float64Array[] = [];
const ys: Float64Array[] = [];

for (const b of bodyDefs) {
  const m = call<{ model: number }>("model", { body: b.mix, sampleRate: 48000 });
  const vowels = Object.values(cat.vowelTargets[b.name]);
  const noseLen = 11.4 * b.scale;
  const k = b.scale;
  const k2 = k * k;
  const noseProfile = Array.from({ length: 16 }, (_, i) => {
    const u = (i + 0.5) / 16;
    const a = u < 0.18 ? 2.6 : u < 0.75 ? 2.0 - 0.6 * (u - 0.18) : u < 0.9 ? 1.4 : 1.0;
    return a * k2;
  });

  const addArtic = (artic: Record<string, number>) => {
    const app = call<{ phAreas: number[]; orAreas: number[]; phLen: number; orLen: number }>("apply", { model: m.model, artic });
    const fr = call<{ velumPort: number }>("frame", { model: m.model, artic });
    const nPh = app.phAreas.length, nOr = app.orAreas.length;
    const phLen = app.phLen, orLen = app.orLen;
    let minA = Infinity;
    for (let i = Math.max(1, Math.floor(nPh / 4)); i < nPh + nOr; i++) {
      const a = i < nPh ? app.phAreas[i] : app.orAreas[i - nPh];
      if (a < minA) minA = a;
    }
    const oralShut = minA < 0.025 * k2;
    const nasalPath = oralShut && fr.velumPort > 0.02 * k2;
    const floor = 0.02 * k2;
    const sec = new Float64Array(FM_SEC);
    let tubeLen: number, aEnd: number;
    if (nasalPath) {
      tubeLen = phLen + noseLen;
      for (let j = 0; j < FM_SEC; j++) {
        const s = ((j + 0.5) * tubeLen) / FM_SEC;
        if (s < phLen) {
          sec[j] = Math.max(floor, app.phAreas[Math.min(nPh - 1, Math.floor((s / phLen) * nPh))]);
        } else {
          const u = (s - phLen) / noseLen;
          const a = noseProfile[Math.min(15, Math.floor(u * 16))];
          sec[j] = Math.max(floor, u < 0.1 ? Math.min(a, fr.velumPort) : a);
        }
      }
      aEnd = 1.0 * k2;
    } else {
      tubeLen = phLen + orLen;
      for (let j = 0; j < FM_SEC; j++) {
        const s = ((j + 0.5) * tubeLen) / FM_SEC;
        sec[j] = Math.max(
          floor,
          s < phLen
            ? app.phAreas[Math.min(nPh - 1, Math.floor((s / phLen) * nPh))]
            : app.orAreas[Math.min(nOr - 1, Math.floor(((s - phLen) / orLen) * nOr))],
        );
      }
      aEnd = Math.max(app.orAreas[nOr - 1], floor);
    }
    xs.push(extractFeatures(sec, tubeLen, aEnd, k, fr.velumPort));
    ys.push(acousticTargets(sec, tubeLen, aEnd, k, fr.velumPort, nasalPath));
  };

  // 1. Exact calibrated vowels (weighted 4x so cardinal vowels are extra sharp)
  for (const v of vowels) {
    const a = { ...cat.neutral, ...v.artic };
    for (let rep = 0; rep < 4; rep++) addArtic(a);
    // Nasalised variants
    addArtic({ ...a, velum: 0.35 });
    addArtic({ ...a, velum: 0.75 });
  }
  // 2. Diphthong blends + random articulatory postures + closures
  for (let n = 0; n < 1400; n++) {
    const v1 = vowels[Math.floor(rng.next() * vowels.length)].artic;
    const v2 = vowels[Math.floor(rng.next() * vowels.length)].artic;
    const t = rng.next();
    const a: Record<string, number> = { ...cat.neutral };
    for (const key of Object.keys(a)) {
      const p1 = v1[key] ?? a[key];
      const p2 = v2[key] ?? a[key];
      a[key] = p1 + (p2 - p1) * t + rng.randn() * 0.12;
    }
    if (rng.next() < 0.25) a.velum = rng.range(0, 0.9);
    if (rng.next() < 0.18) a.lipOpen = rng.range(-0.4, 1.0);
    if (rng.next() < 0.15) a.larynx = rng.range(-0.6, 0.6);
    if (rng.next() < 0.15) {
      // Consonant constriction / nasal murmur
      a.ttY = rng.range(0.2, 1.2);
      if (rng.next() < 0.5) a.velum = rng.range(0.4, 0.95);
    }
    addArtic(a);
  }
  call("dropModel", { model: m.model });
}

console.log(`Dataset built: ${xs.length} vocal-tract postures`);

// Shuffle and split 90/10 train/val
const order = Array.from({ length: xs.length }, (_, i) => i);
for (let i = order.length - 1; i > 0; i--) {
  const j = Math.floor(rng.next() * (i + 1));
  [order[i], order[j]] = [order[j], order[i]];
}
const split = Math.floor(order.length * 0.9);
const trainIdx = order.slice(0, split);
const valIdx = order.slice(split);

// MLP parameters: W1 [H1*IN], b1 [H1], W2 [H2*H1], b2 [H2], W3 [OUT*H2], b3 [OUT]
const initW = (rows: number, cols: number) => {
  const s = Math.sqrt(2.0 / cols);
  return Float64Array.from({ length: rows * cols }, () => rng.randn() * s);
};
const W1 = initW(H1_DIM, IN_DIM), b1 = new Float64Array(H1_DIM);
const W2 = initW(H2_DIM, H1_DIM), b2 = new Float64Array(H2_DIM);
const W3 = initW(OUT_DIM, H2_DIM), b3 = new Float64Array(OUT_DIM);

// Adam state
const adam = (p: Float64Array) => ({ p, g: new Float64Array(p.length), m: new Float64Array(p.length), v: new Float64Array(p.length) });
const params = [adam(W1), adam(b1), adam(W2), adam(b2), adam(W3), adam(b3)];

const z1 = new Float64Array(H1_DIM), h1 = new Float64Array(H1_DIM);
const z2 = new Float64Array(H2_DIM), h2 = new Float64Array(H2_DIM);
const pred = new Float64Array(OUT_DIM);
const dy = new Float64Array(OUT_DIM);
const dh2 = new Float64Array(H2_DIM);
const dh1 = new Float64Array(H1_DIM);

function forward(x: Float64Array): void {
  for (let i = 0; i < H1_DIM; i++) {
    let s = b1[i];
    const row = i * IN_DIM;
    for (let j = 0; j < IN_DIM; j++) s += W1[row + j] * x[j];
    z1[i] = s;
    h1[i] = silu(s);
  }
  for (let i = 0; i < H2_DIM; i++) {
    let s = b2[i];
    const row = i * H1_DIM;
    for (let j = 0; j < H1_DIM; j++) s += W2[row + j] * h1[j];
    z2[i] = s;
    h2[i] = silu(s);
  }
  for (let i = 0; i < OUT_DIM; i++) {
    let s = b3[i];
    const row = i * H2_DIM;
    for (let j = 0; j < H2_DIM; j++) s += W3[row + j] * h2[j];
    pred[i] = s;
  }
}

// Formant frequencies get 3x loss weight for sub-15 Hz accuracy
const lossW = Float64Array.from({ length: OUT_DIM }, (_, i) => (i < 5 ? 3.0 : 1.0));

const EPOCHS = 140;
const BATCH = 64;
let stepCount = 0;

for (let ep = 0; ep < EPOCHS; ep++) {
  for (let i = trainIdx.length - 1; i > 0; i--) {
    const j = Math.floor(rng.next() * (i + 1));
    [trainIdx[i], trainIdx[j]] = [trainIdx[j], trainIdx[i]];
  }
  const lr = 0.004 * (0.5 * (1 + Math.cos((Math.PI * ep) / EPOCHS))) + 0.0001;
  for (let start = 0; start < trainIdx.length; start += BATCH) {
    const end = Math.min(trainIdx.length, start + BATCH);
    const bs = end - start;
    for (const q of params) q.g.fill(0);
    for (let bi = start; bi < end; bi++) {
      const idx = trainIdx[bi];
      const x = xs[idx], y = ys[idx];
      forward(x);
      for (let o = 0; o < OUT_DIM; o++) {
        dy[o] = (2 * lossW[o] * (pred[o] - y[o])) / bs;
        params[5].g[o] += dy[o];
        const row = o * H2_DIM;
        for (let j = 0; j < H2_DIM; j++) params[4].g[row + j] += dy[o] * h2[j];
      }
      for (let j = 0; j < H2_DIM; j++) {
        let s = 0;
        for (let o = 0; o < OUT_DIM; o++) s += W3[o * H2_DIM + j] * dy[o];
        dh2[j] = s * dsilu(z2[j]);
        params[3].g[j] += dh2[j];
        const row = j * H1_DIM;
        for (let k = 0; k < H1_DIM; k++) params[2].g[row + k] += dh2[j] * h1[k];
      }
      for (let k = 0; k < H1_DIM; k++) {
        let s = 0;
        for (let j = 0; j < H2_DIM; j++) s += W2[j * H1_DIM + k] * dh2[j];
        dh1[k] = s * dsilu(z1[k]);
        params[1].g[k] += dh1[k];
        const row = k * IN_DIM;
        for (let m = 0; m < IN_DIM; m++) params[0].g[row + m] += dh1[k] * x[m];
      }
    }
    stepCount++;
    const bc1 = 1 - Math.pow(0.9, stepCount);
    const bc2 = 1 - Math.pow(0.999, stepCount);
    for (const q of params) {
      for (let i = 0; i < q.p.length; i++) {
        const g = q.g[i] + 1e-5 * q.p[i];
        q.m[i] = 0.9 * q.m[i] + 0.1 * g;
        q.v[i] = 0.999 * q.v[i] + 0.001 * g * g;
        q.p[i] -= (lr * (q.m[i] / bc1)) / (Math.sqrt(q.v[i] / bc2) + 1e-8);
      }
    }
  }
}

// Evaluate validation formant MAE in Hz
let f1Err = 0, f2Err = 0, f3Err = 0, mse = 0;
for (const idx of valIdx) {
  forward(xs[idx]);
  const y = ys[idx];
  f1Err += Math.abs(pred[0] - y[0]) * 1000;
  f2Err += Math.abs(pred[1] - y[1]) * 1000;
  f3Err += Math.abs(pred[2] - y[2]) * 1000;
  for (let o = 0; o < OUT_DIM; o++) mse += (pred[o] - y[o]) ** 2;
}
console.log(
  `Validation (${valIdx.length} postures): MSE ${(mse / (valIdx.length * OUT_DIM)).toFixed(5)}, ` +
    `MAE F1 ${(f1Err / valIdx.length).toFixed(1)} Hz, F2 ${(f2Err / valIdx.length).toFixed(1)} Hz, F3 ${(f3Err / valIdx.length).toFixed(1)} Hz`,
);

const fmtArr = (name: string, a: Float64Array) => {
  const lines: string[] = [];
  for (let i = 0; i < a.length; i += 8) {
    lines.push("    " + Array.from(a.slice(i, i + 8), (v) => `${Number(v.toFixed(6))}_f32`).join(", ") + ",");
  }
  return `pub const ${name}: [f32; ${a.length}] = [\n${lines.join("\n")}\n];\n`;
};

const rust = `//! Trained MLP weights for the neural vocal engine (14 -> 32 -> 32 -> 16, SiLU).
//! Generated by \`bun tools/train-neural-voice.ts\` — do not edit by hand.

pub const NN_IN: usize = ${IN_DIM};
pub const NN_H1: usize = ${H1_DIM};
pub const NN_H2: usize = ${H2_DIM};
pub const NN_OUT: usize = ${OUT_DIM};

${fmtArr("W1", W1)}
${fmtArr("B1", b1)}
${fmtArr("W2", W2)}
${fmtArr("B2", b2)}
${fmtArr("W3", W3)}
${fmtArr("B3", b3)}`;

writeFileSync("crates/chimera/src/voice/neural_weights.rs", rust);
console.log("Wrote crates/chimera/src/voice/neural_weights.rs");
