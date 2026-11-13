//! SIREN Vocal & Bestiary Synthesizer — VST3 / CLAP instrument plugin for Ableton Live.
//!
//! Wraps the deterministic physical-modelling `chimera` airway engine, calibrated vowel targets,
//! 6-voice shared-tract harmony choir, 8-creature bestiary, vocal beatbox percussion, and stereo
//! golden-ratio FX rack into a sample-accurate MIDI instrument with a custom vector UI.

mod editor;

use chimera::anatomy::human::{ArticKey, ArticPatch, BodyMix};
use chimera::creatures::types::ParamRole;
use chimera::creatures::CREATURES;
use chimera::engine::{default_f0, Engine, EngineKind};
use chimera::fx::{Fx, FxChain};
use chimera::phonetics::beatbox::BeatStep;
use chimera::phonetics::planner::vowel_artic;
use chimera::sources::harmony::PHI;
use chimera::voice::human_voice::{SourcePatch, VoiceTelemetry};
use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use parking_lot::Mutex;
use std::sync::Arc;

const MAX_NOTES: usize = 16;
const SCOPE_LEN: usize = 192;
const MAX_TRACT_POINTS: usize = 48;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Enum)]
pub enum EngineChoice {
    #[id = "airway"]
    #[name = "Airway FDTD"]
    Airway,
    #[id = "waveguide"]
    #[name = "Waveguide"]
    Waveguide,
    #[id = "formant"]
    #[name = "Formant"]
    Formant,
    #[id = "neural"]
    #[name = "Neural MLP"]
    Neural,
}

impl EngineChoice {
    pub fn to_chimera(self) -> EngineKind {
        match self {
            EngineChoice::Airway => EngineKind::Airway,
            EngineChoice::Waveguide => EngineKind::Waveguide,
            EngineChoice::Formant => EngineKind::Formant,
            EngineChoice::Neural => EngineKind::Neural,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Enum)]
pub enum VoiceMode {
    #[id = "female"]
    #[name = "Human: Female"]
    HumanFemale,
    #[id = "male"]
    #[name = "Human: Male"]
    HumanMale,
    #[id = "child"]
    #[name = "Human: Child"]
    HumanChild,
    #[id = "morph"]
    #[name = "Human: Body Morph"]
    HumanMorph,
    #[id = "beatbox"]
    #[name = "Human: Beatbox Kit"]
    BeatboxKit,
    #[id = "songbird"]
    #[name = "Creature: Songbird"]
    Songbird,
    #[id = "seal"]
    #[name = "Creature: Harbor Seal"]
    HarborSeal,
    #[id = "alligator"]
    #[name = "Creature: Alligator"]
    Alligator,
    #[id = "elephant"]
    #[name = "Creature: Elephant"]
    Elephant,
    #[id = "cat"]
    #[name = "Creature: Cat Purr/Meow"]
    Cat,
    #[id = "bullfrog"]
    #[name = "Creature: Bullfrog"]
    Bullfrog,
    #[id = "elk"]
    #[name = "Creature: Elk Bugle"]
    Elk,
    #[id = "whale"]
    #[name = "Creature: Humpback Whale"]
    HumpbackWhale,
}

impl VoiceMode {
    pub fn creature_index(self) -> Option<usize> {
        match self {
            VoiceMode::Songbird => Some(0),
            VoiceMode::HarborSeal => Some(1),
            VoiceMode::Alligator => Some(2),
            VoiceMode::Elephant => Some(3),
            VoiceMode::Cat => Some(4),
            VoiceMode::Bullfrog => Some(5),
            VoiceMode::Elk => Some(6),
            VoiceMode::HumpbackWhale => Some(7),
            _ => None,
        }
    }

    pub fn body_mix(self, female: f64, child: f64) -> BodyMix {
        match self {
            VoiceMode::HumanFemale => BodyMix::FEMALE,
            VoiceMode::HumanMale | VoiceMode::BeatboxKit => BodyMix::MALE,
            VoiceMode::HumanChild => BodyMix::CHILD,
            VoiceMode::HumanMorph => BodyMix::new(female, child),
            _ => BodyMix::FEMALE,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Enum)]
pub enum CreatureCallMode {
    #[id = "sustain"]
    #[name = "Sustained Breath"]
    Sustained,
    #[id = "call1"]
    #[name = "Call 1"]
    Call1,
    #[id = "call2"]
    #[name = "Call 2"]
    Call2,
    #[id = "call3"]
    #[name = "Call 3"]
    Call3,
    #[id = "call4"]
    #[name = "Call 4"]
    Call4,
}

impl CreatureCallMode {
    pub fn call_index(self) -> Option<usize> {
        match self {
            CreatureCallMode::Sustained => None,
            CreatureCallMode::Call1 => Some(0),
            CreatureCallMode::Call2 => Some(1),
            CreatureCallMode::Call3 => Some(2),
            CreatureCallMode::Call4 => Some(3),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Enum)]
pub enum HarmonyMode {
    #[id = "off"]
    #[name = "Solo Lead"]
    Off,
    #[id = "sub"]
    #[name = "Sub Octave (0.5x)"]
    SubOctave,
    #[id = "fifth"]
    #[name = "Power Fifth (3:2)"]
    Fifth,
    #[id = "major"]
    #[name = "Major Choir (4:5:6)"]
    MajorTriad,
    #[id = "minor"]
    #[name = "Minor Choir (10:12:15)"]
    MinorTriad,
    #[id = "cathedral"]
    #[name = "Cathedral 4-Voice"]
    Cathedral,
    #[id = "golden"]
    #[name = "Golden Ratio Choir (φ)"]
    GoldenChoir,
}

impl HarmonyMode {
    pub fn ratios(self) -> &'static [f64] {
        const SUB: [f64; 1] = [0.5];
        const FIFTH: [f64; 1] = [1.5];
        const MAJOR: [f64; 2] = [1.25, 1.5];
        const MINOR: [f64; 2] = [1.2, 1.5];
        const CATHEDRAL: [f64; 3] = [0.5, 1.25, 1.5];
        const GOLDEN: [f64; 3] = [1.0 / PHI, PHI, 1.0 / (PHI * PHI)];
        match self {
            HarmonyMode::Off => &[],
            HarmonyMode::SubOctave => &SUB,
            HarmonyMode::Fifth => &FIFTH,
            HarmonyMode::MajorTriad => &MAJOR,
            HarmonyMode::MinorTriad => &MINOR,
            HarmonyMode::Cathedral => &CATHEDRAL,
            HarmonyMode::GoldenChoir => &GOLDEN,
        }
    }
}

#[derive(Clone, Copy)]
pub struct UiSnapshot {
    pub scope: [f32; SCOPE_LEN],
    pub tract_areas: [f32; MAX_TRACT_POINTS],
    pub tract_len: usize,
    pub current_f0: f32,
    pub current_note: i32,
    pub gate: f32,
    pub rms_l: f32,
    pub rms_r: f32,
}

impl Default for UiSnapshot {
    fn default() -> Self {
        Self {
            scope: [0.0; SCOPE_LEN],
            tract_areas: [1.5; MAX_TRACT_POINTS],
            tract_len: 32,
            current_f0: 205.0,
            current_note: -1,
            gate: 0.0,
            rms_l: 0.0,
            rms_r: 0.0,
        }
    }
}

#[derive(Default)]
pub struct SharedUiState {
    inner: Mutex<UiSnapshot>,
}

impl SharedUiState {
    pub fn snapshot(&self) -> UiSnapshot {
        *self.inner.lock()
    }
}

#[derive(Params)]
pub struct SirenParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[id = "engine"]
    pub engine_kind: EnumParam<EngineChoice>,

    #[id = "voice_mode"]
    pub voice_mode: EnumParam<VoiceMode>,

    #[id = "body_female"]
    pub body_female: FloatParam,

    #[id = "body_child"]
    pub body_child: FloatParam,

    #[id = "creature_call"]
    pub creature_call: EnumParam<CreatureCallMode>,

    #[id = "drone"]
    pub drone: BoolParam,

    #[id = "vowel_x"]
    pub vowel_x: FloatParam,

    #[id = "vowel_y"]
    pub vowel_y: FloatParam,

    #[id = "nasal"]
    pub nasal: FloatParam,

    #[id = "twang"]
    pub twang: FloatParam,

    #[id = "larynx"]
    pub larynx: FloatParam,

    #[id = "pressure"]
    pub pressure: FloatParam,

    #[id = "breath"]
    pub breathiness: FloatParam,

    #[id = "tension"]
    pub tension: FloatParam,

    #[id = "bright"]
    pub brightness: FloatParam,

    #[id = "fry"]
    pub fry: FloatParam,

    #[id = "rough"]
    pub roughness: FloatParam,

    #[id = "voiced"]
    pub voiced: BoolParam,

    #[id = "vib_depth"]
    pub vibrato_depth: FloatParam,

    #[id = "vib_rate"]
    pub vibrato_rate: FloatParam,

    #[id = "glide"]
    pub glide: FloatParam,

    #[id = "golden_v"]
    pub golden_voice: FloatParam,

    #[id = "harmony"]
    pub harmony: EnumParam<HarmonyMode>,

    #[id = "fx_punch"]
    pub fx_punch: FloatParam,

    #[id = "fx_phone"]
    pub fx_phone: FloatParam,

    #[id = "fx_dly_mix"]
    pub fx_delay_mix: FloatParam,

    #[id = "fx_dly_time"]
    pub fx_delay_time: FloatParam,

    #[id = "fx_gld_mix"]
    pub fx_golden_mix: FloatParam,

    #[id = "fx_gld_dec"]
    pub fx_golden_decay: FloatParam,

    #[id = "out_gain"]
    pub output_gain: FloatParam,
}

impl Default for SirenParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(840, 545),

            engine_kind: EnumParam::new("Engine Model", EngineChoice::Airway),
            voice_mode: EnumParam::new("Voice / Creature", VoiceMode::HumanFemale),
            body_female: FloatParam::new("Body Female", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(20.0)),
            body_child: FloatParam::new("Body Child", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(20.0)),
            creature_call: EnumParam::new("Creature Trigger", CreatureCallMode::Sustained),
            drone: BoolParam::new("Audition Drone", false),

            vowel_x: FloatParam::new("Vowel Back (X)", 0.72, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(18.0)),
            vowel_y: FloatParam::new("Vowel Open (Y)", 0.78, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(18.0)),
            nasal: FloatParam::new("Nasal Velum", 0.02, FloatRange::Linear { min: 0.0, max: 1.2 })
                .with_unit(" cm²")
                .with_smoother(SmoothingStyle::Linear(18.0)),
            twang: FloatParam::new("Epilarynx Twang", 0.15, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(18.0)),
            larynx: FloatParam::new("Larynx Height", 0.0, FloatRange::Linear { min: -0.8, max: 0.8 })
                .with_unit(" cm")
                .with_smoother(SmoothingStyle::Linear(18.0)),

            pressure: FloatParam::new("Lung Pressure", 9.5, FloatRange::Linear { min: 2.0, max: 18.0 })
                .with_unit(" cmH2O")
                .with_smoother(SmoothingStyle::Linear(15.0)),
            breathiness: FloatParam::new("Breathiness", 0.14, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(15.0)),
            tension: FloatParam::new("Fold Tension", 0.52, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(15.0)),
            brightness: FloatParam::new("Brightness", 0.35, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(15.0)),
            fry: FloatParam::new("Vocal Fry", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(15.0)),
            roughness: FloatParam::new("Roughness", 0.08, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(15.0)),
            voiced: BoolParam::new("Voiced Folds", true),

            vibrato_depth: FloatParam::new("Vibrato Depth", 0.18, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(20.0)),
            vibrato_rate: FloatParam::new("Vibrato Rate", 5.6, FloatRange::Linear { min: 2.0, max: 10.0 })
                .with_unit(" Hz")
                .with_smoother(SmoothingStyle::Linear(20.0)),
            glide: FloatParam::new(
                "Portamento Glide",
                0.055,
                FloatRange::Skewed {
                    min: 0.005,
                    max: 0.50,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" s"),
            golden_voice: FloatParam::new("Golden Pulse (1/φ)", 0.20, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(20.0)),
            harmony: EnumParam::new("Harmony Choir", HarmonyMode::Off),

            fx_punch: FloatParam::new("Punch Comp", 0.35, FloatRange::Linear { min: 0.0, max: 1.0 }),
            fx_phone: FloatParam::new("Phone Lo-Fi", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            fx_delay_mix: FloatParam::new("Ping-Pong Delay", 0.15, FloatRange::Linear { min: 0.0, max: 1.0 }),
            fx_delay_time: FloatParam::new("Delay Time", 0.42, FloatRange::Linear { min: 0.0, max: 1.0 }),
            fx_golden_mix: FloatParam::new("Golden Reverb", 0.28, FloatRange::Linear { min: 0.0, max: 1.0 }),
            fx_golden_decay: FloatParam::new("Reverb Decay", 0.58, FloatRange::Linear { min: 0.0, max: 1.0 }),

            output_gain: FloatParam::new(
                "Output Gain",
                util::db_to_gain(0.0),
                FloatRange::Skewed {
                    min: util::db_to_gain(-24.0),
                    max: util::db_to_gain(12.0),
                    factor: FloatRange::gain_skew_factor(-24.0, 12.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(15.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(1))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
        }
    }
}

#[derive(Clone, Copy)]
struct HeldNote {
    note: u8,
    velocity: f32,
}

/// Core synthesizer state shared between the plugin processor and unit tests.
pub struct SynthCore {
    pub engine: Engine,
    pub fx: FxChain,
    mono_buf: Vec<f32>,
    telemetry: VoiceTelemetry,

    held: [HeldNote; MAX_NOTES],
    held_len: usize,
    pitch_bend_st: f64,
    mod_wheel: f64,
    aftertouch: f64,
    expression: f64,

    last_engine: EngineChoice,
    last_mode: VoiceMode,
    last_body: BodyMix,
    last_harmony: HarmonyMode,
    vowel_corners: [ArticPatch; 4],

    scope_ring: [f32; SCOPE_LEN],
    scope_pos: usize,
}

impl SynthCore {
    pub fn new(sample_rate: f64, max_block: usize) -> Self {
        chimera::init();
        let mut engine = Engine::new(sample_rate);
        let body = BodyMix::FEMALE;
        engine.set_body(body);
        let fx = FxChain::new(sample_rate);
        let vowel_corners = load_vowel_corners(body);
        Self {
            engine,
            fx,
            mono_buf: vec![0.0; max_block.max(512)],
            telemetry: VoiceTelemetry::default(),
            held: [HeldNote { note: 60, velocity: 0.0 }; MAX_NOTES],
            held_len: 0,
            pitch_bend_st: 0.0,
            mod_wheel: 0.0,
            aftertouch: 0.0,
            expression: 1.0,
            last_engine: EngineChoice::Airway,
            last_mode: VoiceMode::HumanFemale,
            last_body: body,
            last_harmony: HarmonyMode::Off,
            vowel_corners,
            scope_ring: [0.0; SCOPE_LEN],
            scope_pos: 0,
        }
    }

    pub fn reset(&mut self) {
        self.held_len = 0;
        self.pitch_bend_st = 0.0;
        self.aftertouch = 0.0;
        self.engine.set_gate(0.0);
        self.engine.stop_call();
        self.engine.stop_speech();
    }

    pub fn note_on(&mut self, note: u8, velocity: f32, mode: VoiceMode, call_mode: CreatureCallMode) {
        let vel = velocity.clamp(0.05, 1.0);
        // Deduplicate existing note on stack
        if let Some(idx) = (0..self.held_len).find(|&i| self.held[i].note == note) {
            for j in idx..self.held_len - 1 {
                self.held[j] = self.held[j + 1];
            }
            self.held_len -= 1;
        }
        if self.held_len < MAX_NOTES {
            self.held[self.held_len] = HeldNote { note, velocity: vel };
            self.held_len += 1;
        } else {
            self.held[MAX_NOTES - 1] = HeldNote { note, velocity: vel };
        }

        if mode == VoiceMode::BeatboxKit {
            let step = beatbox_step_for_note(note, vel);
            self.engine.beat_hit(&step);
            return;
        }

        if let (Some(c_idx), Some(call_idx)) = (mode.creature_index(), call_mode.call_index()) {
            let def = &CREATURES[c_idx];
            if !def.calls.is_empty() {
                let c = &def.calls[call_idx.min(def.calls.len() - 1)];
                let ref_hz = c.pitch_ref.unwrap_or(261.625565);
                let played_hz = midi_to_hz(note, self.pitch_bend_st);
                self.engine
                    .play_call(call_idx.min(def.calls.len() - 1), Some(played_hz / ref_hz), Some(vel as f64));
            }
        }
    }

    pub fn note_off(&mut self, note: u8) {
        if let Some(idx) = (0..self.held_len).find(|&i| self.held[i].note == note) {
            for j in idx..self.held_len - 1 {
                self.held[j] = self.held[j + 1];
            }
            self.held_len -= 1;
        }
    }

    pub fn apply_params(&mut self, p: &SirenParams) {
        let eng = p.engine_kind.value();
        if eng != self.last_engine {
            self.engine.set_kind(eng.to_chimera());
            self.last_engine = eng;
        }

        let mode = p.voice_mode.value();
        let desired_body = mode.body_mix(p.body_female.value() as f64, p.body_child.value() as f64);

        if mode != self.last_mode {
            if let Some(c_idx) = mode.creature_index() {
                self.engine.set_creature(Some(&CREATURES[c_idx]), &[]);
            } else {
                self.engine.set_creature(None, &[]);
                self.engine.set_body(desired_body);
                self.vowel_corners = load_vowel_corners(desired_body);
                self.last_body = desired_body;
            }
            self.last_mode = mode;
        } else if mode == VoiceMode::HumanMorph
            && ((desired_body.female - self.last_body.female).abs() > 0.02
                || (desired_body.child - self.last_body.child).abs() > 0.02)
        {
            self.engine.set_body(desired_body);
            self.vowel_corners = load_vowel_corners(desired_body);
            self.last_body = desired_body;
        }

        let harm = p.harmony.value();
        if harm != self.last_harmony {
            self.engine.set_harmony(harm.ratios());
            self.last_harmony = harm;
        }

        // Active MIDI note or Audition Drone
        let active = if self.held_len > 0 {
            Some(self.held[self.held_len - 1])
        } else if p.drone.value() {
            Some(HeldNote { note: 57, velocity: 0.82 })
        } else {
            None
        };

        let target_hz = match active {
            Some(h) => midi_to_hz(h.note, self.pitch_bend_st),
            None => default_f0(self.last_body),
        };
        let vel_scale = active.map(|h| 0.55 + 0.45 * h.velocity as f64).unwrap_or(1.0);

        if let Some(c_idx) = mode.creature_index() {
            let def = &CREATURES[c_idx];
            if p.creature_call.value() == CreatureCallMode::Sustained {
                let ratio = target_hz / 261.625565;
                let mut pairs = [(0usize, 0.0f64); 8];
                let mut n = 0;
                for (idx, pd) in def.params.iter().enumerate() {
                    if pd.role == Some(ParamRole::Pitch) && n < pairs.len() {
                        pairs[n] = (idx, (pd.default * ratio).clamp(pd.min, pd.max));
                        n += 1;
                    }
                }
                if n > 0 {
                    self.engine.set_creature_params(&pairs[..n]);
                }
                self.engine.set_gate(if active.is_some() {
                    (vel_scale * self.expression).clamp(0.0, 1.0)
                } else {
                    0.0
                });
            } else if active.is_none() && !p.drone.value() {
                self.engine.set_gate(0.0);
            }
        } else if mode == VoiceMode::BeatboxKit {
            self.engine.set_gate(0.0);
        } else {
            // Human articulatory vocal synthesis
            let vx = (p.vowel_x.value() as f64).clamp(0.0, 1.0);
            let vy = ((p.vowel_y.value() as f64) + 0.35 * self.aftertouch).clamp(0.0, 1.0);
            let w00 = (1.0 - vx) * (1.0 - vy);
            let w10 = vx * (1.0 - vy);
            let w01 = (1.0 - vx) * vy;
            let w11 = vx * vy;
            let mut artic = ArticPatch::blend(&[
                (w00, self.vowel_corners[0]),
                (w10, self.vowel_corners[1]),
                (w01, self.vowel_corners[2]),
                (w11, self.vowel_corners[3]),
            ]);
            artic
                .set(ArticKey::Velum, p.nasal.value() as f64)
                .set(ArticKey::Epilarynx, p.twang.value() as f64)
                .set(ArticKey::Larynx, p.larynx.value() as f64);
            self.engine.set_artic(&artic, None);

            let src = SourcePatch {
                f0: Some(target_hz),
                glide: Some(p.glide.value() as f64),
                pressure: Some((p.pressure.value() as f64) * vel_scale * self.expression),
                breathiness: Some(p.breathiness.value() as f64),
                tension: Some(p.tension.value() as f64),
                fry: Some(p.fry.value() as f64),
                roughness: Some(p.roughness.value() as f64),
                vibrato_depth: Some(((p.vibrato_depth.value() as f64) + 0.65 * self.mod_wheel).clamp(0.0, 1.0)),
                vibrato_rate: Some(p.vibrato_rate.value() as f64),
                drift: Some(4.5),
                voiced: Some(p.voiced.value()),
                brightness: Some(((p.brightness.value() as f64) + 0.25 * self.aftertouch).clamp(0.0, 1.0)),
                golden: Some(p.golden_voice.value() as f64),
            };
            self.engine.set_source(&src);
            self.engine.set_gate(if active.is_some() { 1.0 } else { 0.0 });
        }

        // Studio FX Rack
        let punch = p.fx_punch.value() as f64;
        self.fx.set_on(Fx::Punch, punch > 0.01);
        if punch > 0.01 {
            self.fx.set_param(Fx::Punch, 0, punch);
        }

        let phone = p.fx_phone.value() as f64;
        self.fx.set_on(Fx::Phone, phone > 0.01);
        if phone > 0.01 {
            self.fx.set_param(Fx::Phone, 3, phone);
        }

        let dly_mix = p.fx_delay_mix.value() as f64;
        self.fx.set_on(Fx::Delay, dly_mix > 0.01);
        if dly_mix > 0.01 {
            self.fx.set_param(Fx::Delay, 0, p.fx_delay_time.value() as f64);
            self.fx.set_param(Fx::Delay, 3, dly_mix);
        }

        let gld_mix = p.fx_golden_mix.value() as f64;
        self.fx.set_on(Fx::Golden, gld_mix > 0.01);
        if gld_mix > 0.01 {
            self.fx.set_param(Fx::Golden, 1, p.fx_golden_decay.value() as f64);
            self.fx.set_param(Fx::Golden, 3, gld_mix);
        }
    }

    pub fn render_slice(&mut self, left: &mut [f32], right: &mut [f32], gain: f32) {
        let n = left.len().min(right.len());
        if n == 0 {
            return;
        }
        if self.mono_buf.len() < n {
            self.mono_buf.resize(n, 0.0);
        }
        let mono = &mut self.mono_buf[..n];
        self.engine.render(mono);
        self.fx.process(mono, &mut left[..n], &mut right[..n]);
        for i in 0..n {
            left[i] *= gain;
            right[i] *= gain;
            if (i & 3) == 0 {
                self.scope_ring[self.scope_pos] = 0.5 * (left[i] + right[i]);
                self.scope_pos = (self.scope_pos + 1) % SCOPE_LEN;
            }
        }
    }

    pub fn write_ui_snapshot(&mut self, ui: &SharedUiState, left: &[f32], right: &[f32], drone: bool) {
        let Some(mut guard) = ui.inner.try_lock() else {
            return;
        };
        for i in 0..SCOPE_LEN {
            guard.scope[i] = self.scope_ring[(self.scope_pos + i) % SCOPE_LEN];
        }
        let n = left.len().max(1) as f32;
        let sum_l: f32 = left.iter().map(|x| x * x).sum();
        let sum_r: f32 = right.iter().map(|x| x * x).sum();
        guard.rms_l = (sum_l / n).sqrt();
        guard.rms_r = (sum_r / n).sqrt();

        self.engine.telemetry(&mut self.telemetry);
        guard.current_f0 = self.telemetry.f0 as f32;
        guard.gate = self.telemetry.gate as f32;
        guard.current_note = if self.held_len > 0 {
            self.held[self.held_len - 1].note as i32
        } else if drone {
            57
        } else {
            -1
        };

        let mut idx = 0;
        for &a in self.telemetry.ph_areas.iter().chain(self.telemetry.or_areas.iter()) {
            if idx < MAX_TRACT_POINTS {
                guard.tract_areas[idx] = a;
                idx += 1;
            }
        }
        if idx >= 2 {
            guard.tract_len = idx;
        }
    }
}

fn load_vowel_corners(body: BodyMix) -> [ArticPatch; 4] {
    [
        vowel_artic("i", body).unwrap_or_default(),
        vowel_artic("u", body).unwrap_or_default(),
        vowel_artic("æ", body).unwrap_or_default(),
        vowel_artic("ɑ", body).unwrap_or_default(),
    ]
}

#[inline]
fn midi_to_hz(note: u8, bend_st: f64) -> f64 {
    440.0 * chimera::math::pow(2.0, ((note as f64) - 69.0 + bend_st) / 12.0)
}

fn beatbox_step_for_note(note: u8, velocity: f32) -> BeatStep {
    let accent = Some((0.6 + 0.8 * velocity as f64).clamp(0.5, 1.5));
    match note {
        35 | 36 | 41 => BeatStep { id: chimera::control::score::Label::new("B"), note: None, accent },
        37 => BeatStep { id: chimera::control::score::Label::new("K"), note: None, accent },
        38 => BeatStep { id: chimera::control::score::Label::new("Pf"), note: None, accent },
        39 => BeatStep { id: chimera::control::score::Label::new("P!"), note: None, accent },
        40 => BeatStep { id: chimera::control::score::Label::new("^K"), note: None, accent },
        42 => BeatStep { id: chimera::control::score::Label::new("t"), note: None, accent },
        44 => BeatStep { id: chimera::control::score::Label::new("!"), note: None, accent },
        46 => BeatStep { id: chimera::control::score::Label::new("tss"), note: None, accent },
        49 | 51 => BeatStep { id: chimera::control::score::Label::new("kch"), note: None, accent },
        _ => BeatStep {
            id: chimera::control::score::Label::new("m"),
            note: Some(note as f64),
            accent,
        },
    }
}

pub struct SirenPlugin {
    params: Arc<SirenParams>,
    ui_state: Arc<SharedUiState>,
    core: SynthCore,
}

impl Default for SirenPlugin {
    fn default() -> Self {
        Self {
            params: Arc::new(SirenParams::default()),
            ui_state: Arc::new(SharedUiState::default()),
            core: SynthCore::new(48_000.0, 2048),
        }
    }
}

impl Plugin for SirenPlugin {
    const NAME: &'static str = "SIREN Vocal Synth";
    const VENDOR: &'static str = "CHIMERA";
    const URL: &'static str = "https://github.com/empiresfall/siren-vocal-synth";
    const EMAIL: &'static str = "siren@chimera.audio";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(
            self.params.clone(),
            self.ui_state.clone(),
            self.params.editor_state.clone(),
        )
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.core = SynthCore::new(
            buffer_config.sample_rate as f64,
            buffer_config.max_buffer_size as usize,
        );
        self.core.apply_params(&self.params);
        true
    }

    fn reset(&mut self) {
        self.core.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let num_samples = buffer.samples();
        if num_samples == 0 {
            return ProcessStatus::Normal;
        }

        let gain = self.params.output_gain.smoothed.next();
        let out = buffer.as_slice();
        if out.len() < 2 {
            return ProcessStatus::Normal;
        }
        let (first, rest) = out.split_at_mut(1);
        let left = &mut first[0][..num_samples];
        let right = &mut rest[0][..num_samples];

        let mut cursor = 0usize;
        let mut next_event = context.next_event();

        while cursor < num_samples {
            while let Some(ev) = next_event {
                let ev_time = (ev.timing() as usize).min(num_samples);
                if ev_time > cursor {
                    break;
                }
                self.handle_midi_event(ev);
                next_event = context.next_event();
            }

            let next_stop = next_event
                .map(|ev| (ev.timing() as usize).clamp(cursor, num_samples))
                .unwrap_or(num_samples);

            if next_stop > cursor {
                self.core.apply_params(&self.params);
                self.core
                    .render_slice(&mut left[cursor..next_stop], &mut right[cursor..next_stop], gain);
                cursor = next_stop;
            }
        }

        if self.params.editor_state.is_open() {
            self.core
                .write_ui_snapshot(&self.ui_state, left, right, self.params.drone.value());
        }

        ProcessStatus::KeepAlive
    }
}

impl SirenPlugin {
    fn handle_midi_event(&mut self, event: NoteEvent<()>) {
        let mode = self.params.voice_mode.value();
        let call_mode = self.params.creature_call.value();
        match event {
            NoteEvent::NoteOn { note, velocity, .. } => {
                if velocity <= 0.0001 {
                    self.core.note_off(note);
                } else {
                    self.core.note_on(note, velocity, mode, call_mode);
                }
            }
            NoteEvent::NoteOff { note, .. } => {
                self.core.note_off(note);
            }
            NoteEvent::MidiPitchBend { value, .. } => {
                // Standard +/- 2 semitone bend around 0.5
                self.core.pitch_bend_st = ((value as f64) - 0.5) * 4.0;
            }
            NoteEvent::PolyPressure { pressure, .. } | NoteEvent::MidiChannelPressure { pressure, .. } => {
                self.core.aftertouch = (pressure as f64).clamp(0.0, 1.0);
            }
            NoteEvent::MidiCC { cc, value, .. } => match cc {
                1 => self.core.mod_wheel = (value as f64).clamp(0.0, 1.0),
                11 => self.core.expression = (value as f64).clamp(0.15, 1.0),
                74 => self.core.aftertouch = (value as f64).clamp(0.0, 1.0),
                120 | 123 => self.core.reset(),
                _ => {}
            },
            _ => {}
        }
    }
}

impl ClapPlugin for SirenPlugin {
    const CLAP_ID: &'static str = "audio.chimera.siren";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Physical-modelling articulatory human voice, choir, beatbox & creature synthesizer");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for SirenPlugin {
    const VST3_CLASS_ID: [u8; 16] = *b"ChimeraSirenVST3";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Synth,
        Vst3SubCategory::Stereo,
    ];
}

nih_export_clap!(SirenPlugin);
nih_export_vst3!(SirenPlugin);

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(buf: &[f32]) -> f32 {
        let s: f32 = buf.iter().map(|x| x * x).sum();
        (s / buf.len().max(1) as f32).sqrt()
    }

    #[test]
    fn human_midi_note_renders_voiced_stereo_audio() {
        let params = SirenParams::default();
        let mut core = SynthCore::new(48_000.0, 512);
        core.note_on(60, 0.9, VoiceMode::HumanFemale, CreatureCallMode::Sustained);
        core.apply_params(&params);

        let mut left = vec![0.0f32; 4096];
        let mut right = vec![0.0f32; 4096];
        for chunk in 0..4 {
            let a = chunk * 1024;
            let b = a + 1024;
            core.render_slice(&mut left[a..b], &mut right[a..b], 1.0);
        }

        assert!(left.iter().all(|x| x.is_finite()));
        assert!(right.iter().all(|x| x.is_finite()));
        assert!(rms(&left[1024..]) > 0.005, "expected audible human vocal output, got rms={}", rms(&left[1024..]));
    }

    #[test]
    fn beatbox_and_creature_modes_produce_clean_finite_audio() {
        let params = SirenParams::default();
        let mut core = SynthCore::new(48_000.0, 512);

        // Beatbox Kick hit
        core.last_mode = VoiceMode::BeatboxKit;
        core.note_on(36, 1.0, VoiceMode::BeatboxKit, CreatureCallMode::Sustained);
        let mut left = vec![0.0f32; 8192];
        let mut right = vec![0.0f32; 8192];
        core.render_slice(&mut left, &mut right, 1.0);
        assert!(rms(&left) > 0.002, "expected audible beatbox kick, got rms={}", rms(&left));

        // Songbird call
        core.engine.set_creature(Some(&CREATURES[0]), &[]);
        core.last_mode = VoiceMode::Songbird;
        core.note_on(72, 0.9, VoiceMode::Songbird, CreatureCallMode::Call1);
        let mut c_left = vec![0.0f32; 8192];
        let mut c_right = vec![0.0f32; 8192];
        core.render_slice(&mut c_left, &mut c_right, 1.0);
        assert!(c_left.iter().all(|x| x.is_finite()));
        assert!(rms(&c_left) > 0.001, "expected audible songbird call, got rms={}", rms(&c_left));
        let _ = params;
    }
}
