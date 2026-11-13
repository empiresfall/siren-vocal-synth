/** Messages between the main thread and the CHIMERA AudioWorklet. */
import type { HumanArtic } from "../wasm/types.ts";
import type { VoiceSource, VoiceTelemetry } from "../wasm/types.ts";
import type { Segment, PlannedSegment } from "../wasm/types.ts";
import type { Gesture } from "../wasm/types.ts";
import type { CreatureTelemetry } from "../wasm/types.ts";
import type { BeatStep, BodyMix } from "../wasm/types.ts";

export interface PhraseMsg {
  segments: Segment[];
  intonation?: "statement" | "question" | "flat";
  pause?: number;
}

export type BodyName = "male" | "female" | "child";

/** Messages for a voice carry the track they are for (0, the app's own voice, when absent). */
type Tracked<T> = T extends unknown ? T & { track?: number } : never;

export type ToWorklet =
  | Tracked<VoiceMsg>
  /** how many voices sound together (1 … 4): track 0 is the app's voice, the rest are band parts */
  | { type: "tracks"; count: number }
  /** a part's level in the mix (1: as rendered) */
  | { type: "mix"; track: number; gain: number }
  /** several messages applied in the same render quantum (parts that must start together) */
  | { type: "batch"; msgs: ToWorklet[] }
  /** the effects rack: switch effect `fx` (its index in the catalogue), and/or set controls [index, value] */
  | { type: "fx"; fx: number; on?: boolean; params?: [number, number][] }
  /** record the output (after the rack): chunks arrive as "recorded" messages */
  | { type: "record"; on: boolean }
  | { type: "telemetry"; enabled: boolean };

type VoiceMsg =
  | { type: "body"; body: BodyMix }
  /** the model of the air above the folds: 0 airway, 1 waveguide, 2 formant */
  | { type: "engine"; kind: number }
  /** harmony voices at these pitch ratios to the lead (at most 6; none: off) */
  | { type: "harmony"; ratios: number[] }
  | { type: "artic"; values: Partial<HumanArtic>; tipFree?: boolean }
  | { type: "source"; values: Partial<VoiceSource> }
  | { type: "gate"; level: number }
  /** gestures planned on the main thread with times relative to "now" (the worklet adds its clock) */
  | { type: "schedule"; gestures: Gesture[]; segments: PlannedSegment[]; end: number; id: number; replace: boolean; lead?: number }
  | { type: "stopSpeech" }
  /** switch the active voice: "human" or a creature id */
  | { type: "creature"; id: string; params?: Record<string, number> }
  | { type: "cparams"; values: Record<string, number> }
  | { type: "call"; id: string; pitch?: number; amp?: number }
  | { type: "stopCall" }
  /** beatbox sequencer: any subset of its state */
  | { type: "beat"; steps?: (BeatStep | null)[]; bpm?: number; swing?: number; playing?: boolean }
  /** end the open-ended gestures with this tag now (a sung note released) */
  | { type: "release"; tag: number }
  /** play one beatbox sound as soon as the mouth can make it */
  | { type: "beatHit"; id: string; note?: number; accent?: number };

export interface TelemetryMessage {
  type: "telemetry";
  t: VoiceTelemetry;
  /** fraction of the render-quantum budget used by the engine (smoothed) */
  load: number;
  body: BodyMix;
  nPh: number;
  nOr: number;
  /** beatbox step under the playhead (−1 when the sequencer is stopped) */
  beatStep: number;
}

export interface PlannedMessage {
  type: "planned";
  /** the voice that planned it (0: the app's own) */
  track: number;
  id: number;
  t0: number;
  end: number;
  segments: PlannedSegment[];
}

export interface CreatureTelemetryMessage {
  type: "ctelemetry";
  t: CreatureTelemetry;
  load: number;
}

export type FromWorklet =
  | { type: "ready"; sampleRate: number; body: BodyMix; nPh: number; nOr: number; creature: string; engine: number }
  /** recorded output since the last chunk (`last`: recording has stopped) */
  | { type: "recorded"; left: Float32Array; right: Float32Array; last: boolean }
  | TelemetryMessage
  | CreatureTelemetryMessage
  | PlannedMessage
  | { type: "error"; message: string };
