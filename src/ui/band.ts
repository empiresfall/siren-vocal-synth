/**
 * Band mode: up to four voices at once, each its own engine with its own body. The lead is the
 * app's voice (track 0: the head on screen); the parts sing the same words in harmony with it
 * (thirds and fifths within the song's key, or an octave below), or beatbox the pattern from
 * Beatbox mode in the song's tempo. All parts are planned here and sent as one batch, so they
 * start in the same render quantum.
 */
import { create } from "zustand";
import { engine } from "../audio/AudioEngine.ts";
import { DEMO_SONGS, ENGINES, beatReady, lyricsToSyllables, planSong, type EngineKind, type Syllable } from "../wasm/engine.ts";
import type { ToWorklet } from "../worklet/protocol.ts";
import type { BodyName } from "../worklet/protocol.ts";
import { nearestBody, PRESETS } from "./body.ts";
import { g2p } from "./speech.ts";
import { useStore } from "./store.ts";

export type Role = "third" | "fifth" | "low" | "beat";
export const ROLES: { value: Role; label: string; title: string }[] = [
  { value: "third", label: "3rd", title: "A third above the lead, within the song's key" },
  { value: "fifth", label: "5th", title: "A fifth above the lead, within the song's key" },
  { value: "low", label: "Low", title: "The melody an octave lower" },
  { value: "beat", label: "Beat", title: "Beatboxes the pattern from Beatbox mode, in the song's tempo" },
];

export interface Part {
  on: boolean;
  role: Role;
  body: BodyName;
  engine: EngineKind;
  level: number;
}

interface BandState {
  song: string;
  /** the player's own words for the tune ("": the song's) */
  lyrics: string;
  parts: Part[];
  playing: boolean;
  loop: boolean;
  setSong: (name: string) => void;
  setLyrics: (text: string) => void;
  setPart: (i: number, v: Partial<Part>) => void;
  setLoop: (on: boolean) => void;
}

export const useBand = create<BandState>((set) => ({
  song: DEMO_SONGS[0].name,
  lyrics: "",
  // the light engines by default: three more airway voices would not fit a laptop's audio thread
  parts: [
    { on: true, role: "third", body: "female", engine: "waveguide", level: 0.7 },
    { on: true, role: "low", body: "male", engine: "formant", level: 0.7 },
    { on: false, role: "beat", body: "male", engine: "waveguide", level: 0.8 },
  ],
  playing: false,
  loop: false,
  setSong: (name) => {
    stopBand();
    set({ song: name });
  },
  setLyrics: (text) => set({ lyrics: text }),
  setPart: (i, v) => set((s) => ({ parts: s.parts.map((p, k) => (k === i ? { ...p, ...v } : p)) })),
  setLoop: (on) => set({ loop: on }),
}));

const MAJOR = [0, 2, 4, 5, 7, 9, 11];

/** The major key that holds most of the tune (ties: the one the tune ends on). */
export function keyOf(midi: number[]): number {
  let best = 0, bestScore = -1;
  for (let k = 0; k < 12; k++) {
    let score = midi.filter((m) => MAJOR.includes((((m - k) % 12) + 12) % 12)).length;
    if ((((midi[midi.length - 1] - k) % 12) + 12) % 12 === 0) score += 0.5;
    if (score > bestScore) (bestScore = score), (best = k);
  }
  return best;
}

/** A note moved up by scale steps within a major key (a note outside the key moves from the step below it). */
export function diatonic(midi: number, key: number, steps: number): number {
  const rel = midi - key;
  const oct = Math.floor(rel / 12);
  const pc = rel - oct * 12;
  let deg = 0;
  for (let i = 0; i < 7; i++) if (MAJOR[i] <= pc) deg = i;
  const d = deg + steps;
  return key + (oct + Math.floor(d / 7)) * 12 + MAJOR[((d % 7) + 7) % 7] + (pc - MAJOR[deg]);
}

let token = 0;
let endTimer: ReturnType<typeof setTimeout> | null = null;
let tracksUp = 1;

/** Stop every part (the extra voices stay built until the mode is left). */
export function stopBand(): void {
  token++;
  if (endTimer) clearTimeout(endTimer);
  endTimer = null;
  if (useBand.getState().playing) {
    const msgs: ToWorklet[] = [];
    for (let t = 0; t < tracksUp; t++) msgs.push({ type: "stopSpeech", track: t }, { type: "beat", playing: false, track: t });
    engine.post({ type: "batch", msgs });
  }
  useBand.setState({ playing: false });
}

/** Leave band mode: stop, and drop the extra voices. */
export function closeBand(): void {
  stopBand();
  if (tracksUp > 1) engine.post({ type: "tracks", count: 1 });
  tracksUp = 1;
}

export async function playBand(): Promise<void> {
  stopBand();
  const mine = token;
  const store = useStore.getState();
  await store.start();
  try {
    await g2p.load();
  } catch {
    // letter-to-sound rules
  }
  if (mine !== token || !useStore.getState().started) return;
  const b = useBand.getState();
  const song = DEMO_SONGS.find((s) => s.name === b.song) ?? DEMO_SONGS[0];
  const syl: Syllable[] = lyricsToSyllables(b.lyrics.trim() || song.lyrics);
  if (!syl.length) return;
  const lead = useStore.getState();
  const key = keyOf(song.notes.map((n) => n[0]));
  const beat = 60 / song.bpm;
  // two beats in, so the beatbox part (which starts now) is on the grid
  const t0 = 2 * beat;
  const vibrato = 35;
  const tune = (shift: number, steps: number) => song.notes.map((n, i) => ({ midi: diatonic(n[0], key, steps) + shift, beats: n[1], syl: syl[i % syl.length] }));

  const parts = b.parts.map((p, i) => ({ p, track: i + 1 })).filter((x) => x.p.on);
  const msgs: ToWorklet[] = [{ type: "tracks", count: 1 + b.parts.length }];
  tracksUp = 1 + b.parts.length;
  // the lead: the app's own voice, as it is set
  const leadPlan = planSong(tune(song.octave[nearestBody(lead.body)] * 12, 0), { body: lead.body, bpm: song.bpm, t0, vibrato });
  let end = leadPlan.end;
  msgs.push({ type: "schedule", gestures: leadPlan.gestures, segments: [], end: leadPlan.end, id: 0, replace: true, lead: 0.05 });
  b.parts.forEach((p, i) => msgs.push({ type: "mix", track: i + 1, gain: p.on ? p.level : 0 }));
  for (const { p, track } of parts) {
    const body = PRESETS[p.body];
    msgs.push({ type: "body", body, track }, { type: "engine", kind: ENGINES.indexOf(p.engine), track });
    if (p.role === "beat") {
      const steps = lead.beat.steps;
      msgs.push(
        { type: "artic", values: beatReady(body), track },
        { type: "source", values: { f0: 100, pressure: 8, voiced: true }, track },
        { type: "gate", level: 0, track },
        { type: "beat", steps, bpm: song.bpm, swing: 0, playing: true, track },
      );
      continue;
    }
    const shift = song.octave[p.body] * 12 + (p.role === "low" ? -12 : 0);
    const notes = tune(shift, p.role === "third" ? 2 : p.role === "fifth" ? 4 : 0);
    const plan = planSong(notes, { body, bpm: song.bpm, t0, vibrato });
    end = Math.max(end, plan.end);
    msgs.push({ type: "source", values: { pressure: 9, tension: 0.55, voiced: true }, track }, { type: "schedule", gestures: plan.gestures, segments: [], end: plan.end, id: 0, replace: true, lead: 0.05, track });
  }
  engine.post({ type: "batch", msgs });
  useBand.setState({ playing: true });
  endTimer = setTimeout(
    () => {
      if (mine !== token) return;
      if (useBand.getState().loop) void playBand();
      else stopBand();
    },
    (end + 0.5) * 1000,
  );
}
