/**
 * The head follows a voice at the microphone: its pitch (YIN), whether it is voiced, and its vowel
 * (the calibrated vowel whose first two formants are nearest, by LPC). The microphone is only
 * analysed, never played back. The analysis runs in the engine (`analyseFrame`), 25 times a second.
 */
import { create } from "zustand";
import { engine } from "../audio/AudioEngine.ts";
import { analyseFrame, VOWEL_TARGETS } from "../wasm/engine.ts";
import { nearestBody } from "./body.ts";
import { useStore } from "./store.ts";

interface MicState {
  on: boolean;
  error: string | null;
  /** what was heard last: pitch (Hz, 0 when unvoiced), the vowel followed, the level 0 … 1 */
  heard: { f0: number; vowel: string | null; level: number };
}

export const useMic = create<MicState>(() => ({ on: false, error: null, heard: { f0: 0, vowel: null, level: 0 } }));

/** vowels the head may take: the corners and the ones between, not the near-duplicates */
const FOLLOW = ["i", "ɪ", "ɛ", "æ", "ɑ", "ɔ", "o", "u", "ʌ", "ə"];
/** quieter than this (RMS) is silence */
const FLOOR = 0.012;

let timer: ReturnType<typeof setInterval> | null = null;
let buf: Float32Array<ArrayBuffer> | null = null;
let last: string | null = null;
let candidate: string | null = null;
let voicedFrames = 0;

function nearestVowel(f1: number, f2: number): string | null {
  const table = VOWEL_TARGETS[nearestBody(useStore.getState().body)];
  let best: string | null = null, bestD = Infinity;
  for (const ipa of FOLLOW) {
    const t = table[ipa];
    if (!t) continue;
    // distance in octaves; the first formant counts more (it is the mouth's opening)
    const d = 1.5 * Math.abs(Math.log2(f1 / t.formants[0])) + Math.abs(Math.log2(f2 / t.formants[1]));
    if (d < bestD) (bestD = d), (best = ipa);
  }
  return best;
}

function tick(): void {
  const a = engine.micAnalyser;
  if (!a || !buf) return;
  a.getFloatTimeDomainData(buf);
  const r = analyseFrame(buf, engine.sampleRate, 65, 800);
  const st = useStore.getState();
  const loud = r.rms > FLOOR;
  const voiced = loud && r.aperiodicity < 0.3 && r.f0 > 0;
  voicedFrames = voiced ? voicedFrames + 1 : 0;
  if (!loud) {
    engine.gate(0);
    useMic.setState({ heard: { f0: 0, vowel: last, level: 0 } });
    return;
  }
  // breath follows the level; a voice needs two frames in a row to count (a click is not a note)
  engine.gate(Math.min(1.2, 0.55 + 4 * r.rms));
  if (st.source.voiced !== (voicedFrames >= 2 || voicedFrames === 0 ? voiced : st.source.voiced)) st.setSource({ voiced });
  if (voiced && voicedFrames >= 2) {
    st.setSource({ f0: r.f0 });
    const [f1, f2] = r.formants;
    if (f1 && f2) {
      const v = nearestVowel(f1, f2);
      // the same vowel twice before the tongue moves
      if (v && v === candidate && v !== last) {
        last = v;
        const t = VOWEL_TARGETS[nearestBody(st.body)][v];
        if (t) st.setArtic(t.artic, true);
      }
      candidate = v;
    }
  }
  useMic.setState({ heard: { f0: voiced ? r.f0 : 0, vowel: last, level: Math.min(1, r.rms * 6) } });
}

/** Start or stop following the microphone (asks for it the first time). */
export async function setMicFollow(on: boolean): Promise<void> {
  if (on === useMic.getState().on) return;
  if (!on) {
    if (timer) clearInterval(timer);
    timer = null;
    engine.closeMic();
    engine.gate(0);
    const st = useStore.getState();
    if (!st.source.voiced) st.setSource({ voiced: true });
    useMic.setState({ on: false });
    return;
  }
  try {
    await useStore.getState().start();
    const a = await engine.openMic();
    buf = new Float32Array(a.fftSize);
    last = candidate = null;
    voicedFrames = 0;
    useStore.getState().setHold(false);
    timer = setInterval(tick, 40);
    useMic.setState({ on: true, error: null });
  } catch (e) {
    useMic.setState({ on: false, error: e instanceof Error ? e.message : String(e) });
  }
}
