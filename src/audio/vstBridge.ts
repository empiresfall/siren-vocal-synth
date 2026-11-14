/**
 * Native VST3 / CLAP WebView bridge (`window.__SIREN_VST__`).
 *
 * When SIREN runs inside the macOS VST3/CLAP plugin's WKWebView, audio rendering happens natively
 * on Ableton Live's real-time audio thread in Rust (`crates/chimera-vst`), while the full 1:1
 * React + Three.js 3D Head UI runs here and exchanges `ToWorklet` / `FromWorklet` messages,
 * post-FX PCM spectrum windows, Ableton MIDI notes, and VST parameter automation at 60 Hz.
 */
import type {
  CreatureTelemetryMessage,
  FromWorklet,
  PlannedMessage,
  TelemetryMessage,
  ToWorklet,
} from "../worklet/protocol.ts";
import type { CreatureTelemetry } from "../wasm/types.ts";

export interface SirenVstGlobal {
  send(msg: unknown): void;
  onMessage(fn: (msg: VstInboundMessage) => void): () => void;
  dispatch(msg: VstInboundMessage): void;
}

declare global {
  interface Window {
    __SIREN_VST__?: SirenVstGlobal;
  }
}

export const isVstPlugin: boolean =
  typeof window !== "undefined" && typeof window.__SIREN_VST__ !== "undefined";

export interface WireHumanTelemetry {
  type: "vstTelemetry";
  load: number;
  body: { female: number; child: number };
  nPh: number;
  nOr: number;
  beatStep: number;
  phLen: number;
  orLen: number;
  velumPort: number;
  f0: number;
  voicing: number;
  pLung: number;
  outRms: number;
  noiseLevel: number;
  noiseNode: number;
  gate: number;
  clock: number;
  active: string[];
  /** packed f32 little-endian base64: artic, phAreas, orAreas, pressureRms, flow, glottalArea, glottalFlow */
  nArtic: number;
  nScope: number;
  packedB64: string;
  /** post-FX mono output samples since last tick (f32 little-endian base64) */
  pcmB64?: string;
}

export interface WireCreatureTelemetry {
  type: "vstCreatureTelemetry";
  load: number;
  id: string;
  gate: number;
  outRms: number;
  clock: number;
  call: string | null;
  params: Record<string, number>;
  source: Record<string, number>;
  tubes: { name: string; areasB64: string; rmsB64: string }[];
  pcmB64?: string;
}

export interface VstMidiMessage {
  type: "vstMidi";
  on: boolean;
  note: number;
  velocity: number;
  /** true when UI should run store.noteOn/noteOff (e.g. Sing lyrics or Bestiary call trigger) */
  routeToStore: boolean;
}

export interface VstHostParamsMessage {
  type: "vstHostParams";
  vowelX?: number;
  vowelY?: number;
  engineKind?: number;
  bodyFemale?: number;
  bodyChild?: number;
  pressure?: number;
  breathiness?: number;
  tension?: number;
  brightness?: number;
  fry?: number;
  roughness?: number;
  voiced?: boolean;
  vibratoDepth?: number;
  vibratoRate?: number;
  glide?: number;
  golden?: number;
  velum?: number;
  epilarynx?: number;
  larynx?: number;
  volume?: number;
}

export interface VstRecordedChunk {
  type: "vstRecorded";
  leftB64: string;
  rightB64: string;
  last: boolean;
}

export interface VstRestoreState {
  type: "vstRestore";
  stateJson: string;
}

export type VstInboundMessage =
  | FromWorklet
  | WireHumanTelemetry
  | WireCreatureTelemetry
  | VstMidiMessage
  | VstHostParamsMessage
  | VstRecordedChunk
  | VstRestoreState;

export function decodeF32B64(b64: string): Float32Array {
  if (!b64) return new Float32Array(0);
  const bin = atob(b64);
  const len = bin.length;
  const bytes = new Uint8Array(len);
  for (let i = 0; i < len; i++) bytes[i] = bin.charCodeAt(i);
  return new Float32Array(bytes.buffer, 0, Math.floor(len / 4));
}

function arrayBufferToB64(buf: ArrayBuffer): string {
  const bytes = new Uint8Array(buf);
  let out = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    out += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(out);
}

// ------------------------------------------------------------------ Shared PCM Ring & WebAudio-compatible FFT Analyser

const RING_SIZE = 4096;
const pcmRing = new Float32Array(RING_SIZE);
let pcmWrite = 0;
let pcmEpoch = 0;

export function pushVstPcm(samples: Float32Array): void {
  if (samples.length === 0) return;
  for (let i = 0; i < samples.length; i++) {
    pcmRing[pcmWrite] = samples[i];
    pcmWrite = (pcmWrite + 1) & (RING_SIZE - 1);
  }
  pcmEpoch++;
}

/**
 * Drop-in replacement for WebAudio's `AnalyserNode` (`SpectrumView` & `Spectrogram`),
 * using WebAudio's exact Blackman window, `1/N` normalization, and exponential magnitude smoothing.
 */
export class VstAnalyser {
  readonly fftSize: number;
  readonly frequencyBinCount: number;
  smoothingTimeConstant: number;

  private readonly cos: Float64Array;
  private readonly sin: Float64Array;
  private readonly rev: Uint32Array;
  private readonly win: Float64Array;
  private readonly re: Float64Array;
  private readonly im: Float64Array;
  private readonly mag: Float64Array;
  private lastEpoch = -1;

  constructor(fftSize: number, smoothingTimeConstant: number) {
    this.fftSize = fftSize;
    this.frequencyBinCount = fftSize >> 1;
    this.smoothingTimeConstant = smoothingTimeConstant;
    this.cos = new Float64Array(fftSize >> 1);
    this.sin = new Float64Array(fftSize >> 1);
    this.rev = new Uint32Array(fftSize);
    this.win = new Float64Array(fftSize);
    this.re = new Float64Array(fftSize);
    this.im = new Float64Array(fftSize);
    this.mag = new Float64Array(this.frequencyBinCount);

    const bits = Math.round(Math.log2(fftSize));
    for (let i = 0; i < fftSize; i++) {
      let r = 0;
      for (let b = 0; b < bits; b++) r |= ((i >> b) & 1) << (bits - 1 - b);
      this.rev[i] = r;
      const a = (2 * Math.PI * i) / fftSize;
      this.win[i] = 0.42 - 0.5 * Math.cos(a) + 0.08 * Math.cos(2 * a);
      if (i < fftSize >> 1) {
        this.cos[i] = Math.cos(a);
        this.sin[i] = -Math.sin(a);
      }
    }
  }

  getFloatFrequencyData(out: Float32Array): void {
    const n = this.fftSize;
    const bins = Math.min(out.length, this.frequencyBinCount);
    if (pcmEpoch !== this.lastEpoch) {
      this.lastEpoch = pcmEpoch;
      const re = this.re;
      const im = this.im;
      const start = (pcmWrite - n + RING_SIZE * 2) & (RING_SIZE - 1);
      for (let i = 0; i < n; i++) {
        const j = this.rev[i];
        const sample = pcmRing[(start + j) & (RING_SIZE - 1)];
        re[i] = sample * this.win[j];
        im[i] = 0;
      }
      for (let size = 2; size <= n; size <<= 1) {
        const half = size >> 1;
        const step = n / size;
        for (let s = 0; s < n; s += size) {
          for (let k = 0; k < half; k++) {
            const wr = this.cos[k * step];
            const wi = this.sin[k * step];
            const a = s + k;
            const b = a + half;
            const xr = re[b] * wr - im[b] * wi;
            const xi = re[b] * wi + im[b] * wr;
            re[b] = re[a] - xr;
            im[b] = im[a] - xi;
            re[a] += xr;
            im[a] += xi;
          }
        }
      }
      const invN = 1 / n;
      const sm = Math.max(0, Math.min(0.99, this.smoothingTimeConstant));
      for (let k = 0; k < this.frequencyBinCount; k++) {
        const m = Math.hypot(re[k], im[k]) * invN;
        this.mag[k] = sm * this.mag[k] + (1 - sm) * m;
      }
    }
    for (let k = 0; k < bins; k++) {
      out[k] = 20 * Math.log10(Math.max(1e-12, this.mag[k]));
    }
  }
}

// ------------------------------------------------------------------ Outbound & Inbound Helpers

export function sendToVst(msg: ToWorklet | Record<string, unknown>): void {
  window.__SIREN_VST__?.send(msg);
}

export function decodeHumanTelemetry(m: WireHumanTelemetry): TelemetryMessage {
  if (m.pcmB64) pushVstPcm(decodeF32B64(m.pcmB64));
  const all = decodeF32B64(m.packedB64);
  let o = 0;
  const take = (len: number) => {
    const end = Math.min(all.length, o + len);
    const slice = all.slice(o, end);
    o = end;
    return slice;
  };
  const nTotal = m.nPh + m.nOr;
  const artic = take(m.nArtic);
  const phAreas = take(m.nPh);
  const orAreas = take(m.nOr);
  const pressureRms = take(nTotal);
  const flow = take(nTotal);
  const glottalArea = take(m.nScope);
  const glottalFlow = take(m.nScope);
  return {
    type: "telemetry",
    t: {
      artic,
      phAreas,
      orAreas,
      phLen: m.phLen,
      orLen: m.orLen,
      pressureRms,
      flow,
      velumPort: m.velumPort,
      glottalArea,
      glottalFlow,
      f0: m.f0,
      voicing: m.voicing,
      pLung: m.pLung,
      outRms: m.outRms,
      noiseLevel: m.noiseLevel,
      noiseNode: m.noiseNode,
      gate: m.gate,
      clock: m.clock,
      active: m.active,
    },
    load: m.load,
    body: m.body,
    nPh: m.nPh,
    nOr: m.nOr,
    beatStep: m.beatStep,
  };
}

export function decodeCreatureTelemetry(m: WireCreatureTelemetry): CreatureTelemetryMessage {
  if (m.pcmB64) pushVstPcm(decodeF32B64(m.pcmB64));
  const tubes: CreatureTelemetry["tubes"] = {};
  for (const tb of m.tubes) {
    tubes[tb.name] = {
      areas: decodeF32B64(tb.areasB64),
      rms: decodeF32B64(tb.rmsB64),
    };
  }
  return {
    type: "ctelemetry",
    load: m.load,
    t: {
      kind: "creature",
      id: m.id,
      params: m.params,
      gate: m.gate,
      outRms: m.outRms,
      clock: m.clock,
      tubes,
      source: m.source,
      call: m.call,
    },
  };
}

let chromeInstalled = false;

export function installVstChromeHooks(): void {
  if (!isVstPlugin || chromeInstalled) return;
  chromeInstalled = true;

  document.documentElement.classList.add("siren-vst");

  document.addEventListener("contextmenu", (e) => {
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
    e.preventDefault();
  });

  document.addEventListener(
    "click",
    (e) => {
      const a = (e.target as HTMLElement | null)?.closest?.("a");
      if (!a) return;
      const href = a.getAttribute("href") ?? "";
      const download = a.getAttribute("download");
      if (download !== null && href.startsWith("blob:")) {
        e.preventDefault();
        void fetch(href)
          .then((r) => r.arrayBuffer())
          .then((buf) => {
            sendToVst({
              type: "vstSaveWav",
              name: download || "siren-take.wav",
              b64: arrayBufferToB64(buf),
            });
          });
        return;
      }
      if (href.startsWith("http://") || href.startsWith("https://")) {
        e.preventDefault();
        sendToVst({ type: "vstOpenUrl", url: href });
      }
    },
    true,
  );
}

export type VstEventCallbacks = {
  onMidi: (m: VstMidiMessage) => void;
  onHostParams: (m: VstHostParamsMessage) => void;
  onRestore: (stateJson: string) => void;
  onPlanned?: (m: PlannedMessage) => void;
};

let vstCallbacks: VstEventCallbacks | null = null;

export function registerVstCallbacks(cb: VstEventCallbacks): void {
  vstCallbacks = cb;
}

export function dispatchExtraVstMessage(m: VstInboundMessage): m is VstMidiMessage | VstHostParamsMessage | VstRestoreState {
  if (m.type === "vstMidi") {
    vstCallbacks?.onMidi(m);
    return true;
  }
  if (m.type === "vstHostParams") {
    vstCallbacks?.onHostParams(m);
    return true;
  }
  if (m.type === "vstRestore") {
    vstCallbacks?.onRestore(m.stateJson);
    return true;
  }
  return false;
}
