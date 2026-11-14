use crate::b64::encode_f32_le;
use crate::{EngineChoice, HarmonyMode, SirenParams, VoiceMode};
use chimera::anatomy::human::{ArticKey, ArticPatch, BodyMix, N_ARTIC};
use chimera::control::score::{Gesture, Label};
use chimera::creatures::types::ParamRole;
use chimera::creatures::voice::CreatureTelemetry;
use chimera::creatures::{creature_by_id, CreatureDef, CREATURES};
use chimera::engine::{default_f0, Engine, EngineKind};
use chimera::fx::{Fx, FxChain};
use chimera::phonetics::beatbox::BeatStep;
use chimera::phonetics::planner::vowel_artic;
use chimera::phonetics::PlannedSegment;
use chimera::sources::harmony::PHI;
use chimera::voice::human_voice::{SourcePatch, VoiceTelemetry};
use crossbeam_channel::{Receiver, Sender};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

const MAX_TRACKS: usize = 4;
const MAX_NOTES: usize = 16;
const REC_CHUNK: usize = 12_000;
const PCM_TAP_CAP: usize = 2048;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UiMode {
    Play,
    Speak,
    Sing,
    Bestiary,
    Beatbox,
    Band,
}

impl UiMode {
    pub fn from_str(s: &str) -> Self {
        match s {
            "speak" => Self::Speak,
            "sing" => Self::Sing,
            "bestiary" => Self::Bestiary,
            "beatbox" => Self::Beatbox,
            "band" => Self::Band,
            _ => Self::Play,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn low_shelf(sr: f64, freq: f64, gain_db: f64) -> Self {
        let a = 10.0_f64.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * freq / sr.max(8000.0);
        let cos_w = w0.cos();
        let alpha = w0.sin() * 0.5 * std::f64::consts::SQRT_2;
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w + two_sqrt_a_alpha);
        let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w);
        let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w - two_sqrt_a_alpha);
        let a0 = (a + 1.0) + (a - 1.0) * cos_w + two_sqrt_a_alpha;
        let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w);
        let a2 = (a + 1.0) + (a - 1.0) * cos_w - two_sqrt_a_alpha;

        Self {
            b0: (b0 / a0) as f32,
            b1: (b1 / a0) as f32,
            b2: (b2 / a0) as f32,
            a1: (a1 / a0) as f32,
            a2: (a2 / a0) as f32,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn peaking(sr: f64, freq: f64, q: f64, gain_db: f64) -> Self {
        let a = 10.0_f64.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * freq / sr.max(8000.0);
        let cos_w = w0.cos();
        let alpha = w0.sin() / (2.0 * q.max(0.1));

        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w;
        let a2 = 1.0 - alpha / a;

        Self {
            b0: (b0 / a0) as f32,
            b1: (b1 / a0) as f32,
            b2: (b2 / a0) as f32,
            a1: (a1 / a0) as f32,
            a2: (a2 / a0) as f32,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

/// Close-mic vocal percussion chain matching `AudioEngine.ts` (120 Hz +9 dB low-shelf,
/// 3.5 kHz +3 dB presence, fast 5:1 compressor at -28 dB, 2.2x make-up).
struct CloseMicChain {
    shelf_l: Biquad,
    shelf_r: Biquad,
    pres_l: Biquad,
    pres_r: Biquad,
    env: f32,
    atk_coeff: f32,
    rel_coeff: f32,
}

impl CloseMicChain {
    fn new(sr: f64) -> Self {
        let sr_f = sr.max(8000.0) as f32;
        Self {
            shelf_l: Biquad::low_shelf(sr, 120.0, 9.0),
            shelf_r: Biquad::low_shelf(sr, 120.0, 9.0),
            pres_l: Biquad::peaking(sr, 3500.0, 0.8, 3.0),
            pres_r: Biquad::peaking(sr, 3500.0, 0.8, 3.0),
            env: 0.0,
            atk_coeff: (-1.0 / (0.002 * sr_f)).exp(),
            rel_coeff: (-1.0 / (0.120 * sr_f)).exp(),
        }
    }

    fn reset(&mut self) {
        self.shelf_l.reset();
        self.shelf_r.reset();
        self.pres_l.reset();
        self.pres_r.reset();
        self.env = 0.0;
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let thresh_db = -28.0_f32;
        let slope = 1.0_f32 - 1.0_f32 / 5.0_f32;
        for i in 0..left.len().min(right.len()) {
            let l = self.pres_l.tick(self.shelf_l.tick(left[i]));
            let r = self.pres_r.tick(self.shelf_r.tick(right[i]));
            let peak = l.abs().max(r.abs()).max(1e-7);
            let c = if peak > self.env { self.atk_coeff } else { self.rel_coeff };
            self.env = c * self.env + (1.0 - c) * peak;
            let env_db = 20.0 * self.env.log10();
            let gr_db = if env_db > thresh_db {
                -slope * (env_db - thresh_db)
            } else {
                0.0
            };
            let gain = 10.0_f32.powf(gr_db / 20.0) * 2.2;
            left[i] = (l * gain).clamp(-1.5, 1.5);
            right[i] = (r * gain).clamp(-1.5, 1.5);
        }
    }
}

#[derive(Clone, Copy, Default)]
struct HeldNote {
    note: u8,
    velocity: f32,
}

pub struct DspCore {
    sample_rate: f64,
    tracks: [Engine; MAX_TRACKS],
    active_tracks: usize,
    gains: [f32; MAX_TRACKS],
    fx: FxChain,
    close_mic: CloseMicChain,
    close_mic_on: bool,
    master_volume: f32,

    track_buf: Vec<f32>,
    mix_buf: Vec<f32>,
    tele: VoiceTelemetry,
    packed_tele: Vec<f32>,
    ctele: Option<CreatureTelemetry>,
    active_creature: Option<&'static CreatureDef>,

    // WAV recording state (for Rack Rec button)
    recording: bool,
    rec_left: Vec<f32>,
    rec_right: Vec<f32>,

    // Post-FX mono output tap for UI SpectrumView & Spectrogram
    pcm_tap: Vec<f32>,

    // MIDI state
    held: [HeldNote; MAX_NOTES],
    held_len: usize,
    sustain_pedal: bool,
    sustained: [bool; 128],
    pitch_bend_st: f64,
    mod_wheel: f64,
    expression: f64,
    aftertouch: f64,

    // Base human source values set by UI (modulated by MIDI bend / MW / expression / AT)
    base_f0: f64,
    base_pressure: f64,
    base_vibrato_depth: f64,
    base_brightness: f64,

    // Headless VST parameter caches (used when GUI hasn't taken over or when host automates)
    vowel_corners: [ArticPatch; 4],
    cached_body: BodyMix,
    cached_headless_mode: VoiceMode,
    cached_headless_engine: EngineChoice,
    cached_headless_harmony: HarmonyMode,

    // UI bridge state
    ui_open: Arc<AtomicBool>,
    ui_driven: bool,
    ui_mode: UiMode,
    ui_key_call_active: bool,
    telemetry_enabled: bool,
    frames_since_tele: usize,
    load_ema: f64,

    cmd_rx: Receiver<Value>,
    evt_tx: Sender<String>,
}

impl DspCore {
    pub fn new(
        sample_rate: f64,
        max_buffer_size: usize,
        ui_open: Arc<AtomicBool>,
        cmd_rx: Receiver<Value>,
        evt_tx: Sender<String>,
    ) -> Self {
        chimera::phonetics::beatbox::init();
        let sr = sample_rate.max(8_000.0);
        let cap = max_buffer_size.max(4096);
        let mut tracks = [
            Engine::new(sr),
            Engine::new(sr),
            Engine::new(sr),
            Engine::new(sr),
        ];
        let mut tele = VoiceTelemetry::default();
        tracks[0].telemetry(&mut tele);
        let body = BodyMix::new(0.0, 0.0);

        Self {
            sample_rate: sr,
            tracks,
            active_tracks: 1,
            gains: [1.0, 1.0, 1.0, 1.0],
            fx: FxChain::new(sr),
            close_mic: CloseMicChain::new(sr),
            close_mic_on: false,
            master_volume: 0.8,

            track_buf: vec![0.0; cap],
            mix_buf: vec![0.0; cap],
            tele,
            packed_tele: Vec::with_capacity(1024),
            ctele: None,
            active_creature: None,

            recording: false,
            rec_left: Vec::with_capacity(REC_CHUNK),
            rec_right: Vec::with_capacity(REC_CHUNK),
            pcm_tap: Vec::with_capacity(PCM_TAP_CAP),

            held: [HeldNote::default(); MAX_NOTES],
            held_len: 0,
            sustain_pedal: false,
            sustained: [false; 128],
            pitch_bend_st: 0.0,
            mod_wheel: 0.0,
            expression: 1.0,
            aftertouch: 0.0,

            base_f0: default_f0(body),
            base_pressure: 8.0,
            base_vibrato_depth: 10.0,
            base_brightness: 0.25,

            vowel_corners: load_vowel_corners(body),
            cached_body: body,
            cached_headless_mode: VoiceMode::HumanMale,
            cached_headless_engine: EngineChoice::Airway,
            cached_headless_harmony: HarmonyMode::Off,

            ui_open,
            ui_driven: false,
            ui_mode: UiMode::Play,
            ui_key_call_active: false,
            telemetry_enabled: true,
            frames_since_tele: 0,
            load_ema: 0.0,

            cmd_rx,
            evt_tx,
        }
    }

    pub fn reset(&mut self) {
        self.held_len = 0;
        self.sustain_pedal = false;
        self.sustained = [false; 128];
        self.pitch_bend_st = 0.0;
        self.aftertouch = 0.0;
        self.close_mic.reset();
        for t in &mut self.tracks {
            t.set_gate(0.0);
        }
    }

    fn send_ui(&self, val: &Value) {
        if !self.ui_open.load(Ordering::Relaxed) {
            return;
        }
        if let Ok(s) = serde_json::to_string(val) {
            let _ = self.evt_tx.try_send(s);
        }
    }

    fn send_ready(&self) {
        let info = self.tracks[0].info();
        let creature_id = self.active_creature.map(|c| c.id).unwrap_or("human");
        self.send_ui(&json!({
            "type": "ready",
            "sampleRate": self.sample_rate,
            "body": { "female": info.body.female, "child": info.body.child },
            "nPh": info.n_ph,
            "nOr": info.n_or,
            "creature": creature_id,
            "engine": self.tracks[0].kind().index(),
        }));
    }

    pub fn drain_ui_commands(&mut self, persisted_state_json: &str) {
        while let Ok(msg) = self.cmd_rx.try_recv() {
            self.handle_ui_msg(&msg, persisted_state_json);
        }
    }

    fn handle_ui_msg(&mut self, m: &Value, persisted_state_json: &str) {
        let Some(ty) = m.get("type").and_then(Value::as_str) else {
            return;
        };
        match ty {
            "vstReady" => {
                self.ui_driven = true;
                self.send_ready();
                if !persisted_state_json.is_empty() {
                    self.send_ui(&json!({
                        "type": "vstRestore",
                        "stateJson": persisted_state_json,
                    }));
                }
                return;
            }
            "vstUiMode" => {
                if let Some(mode_s) = m.get("mode").and_then(Value::as_str) {
                    self.ui_mode = UiMode::from_str(mode_s);
                }
                self.ui_key_call_active = m
                    .get("keyCall")
                    .map(|v| !v.is_null() && v.as_str().is_some_and(|s| !s.is_empty()))
                    .unwrap_or(false);
                return;
            }
            "vstVolume" => {
                if let Some(v) = m.get("value").and_then(Value::as_f64) {
                    self.master_volume = (v as f32).clamp(0.0, 1.5);
                }
                return;
            }
            "vstCloseMic" => {
                if let Some(on) = m.get("on").and_then(Value::as_bool) {
                    self.close_mic_on = on;
                }
                return;
            }
            "batch" => {
                if let Some(arr) = m.get("msgs").and_then(Value::as_array) {
                    for sub in arr {
                        self.handle_ui_msg(sub, persisted_state_json);
                    }
                }
                return;
            }
            "tracks" => {
                if let Some(c) = m.get("count").and_then(Value::as_u64) {
                    let next = (c as usize).clamp(1, MAX_TRACKS);
                    for k in next..self.active_tracks {
                        self.tracks[k].set_gate(0.0);
                        self.tracks[k].stop_speech();
                        self.tracks[k].set_beat_playing(false);
                    }
                    self.active_tracks = next;
                }
                return;
            }
            "mix" => {
                if let (Some(tr), Some(g)) = (
                    m.get("track").and_then(Value::as_u64),
                    m.get("gain").and_then(Value::as_f64),
                ) {
                    let idx = tr as usize;
                    if idx < MAX_TRACKS {
                        self.gains[idx] = g as f32;
                    }
                }
                return;
            }
            "fx" => {
                if let Some(fx_idx) = m.get("fx").and_then(Value::as_u64) {
                    if let Some(fx_enum) = Fx::from_index(fx_idx as usize) {
                        if let Some(on) = m.get("on").and_then(Value::as_bool) {
                            self.fx.set_on(fx_enum, on);
                        }
                        if let Some(params) = m.get("params").and_then(Value::as_array) {
                            for pair in params {
                                if let Some(arr) = pair.as_array() {
                                    if arr.len() == 2 {
                                        if let (Some(k), Some(v)) = (arr[0].as_u64(), arr[1].as_f64()) {
                                            self.fx.set_param(fx_enum, k as usize, v);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                return;
            }
            "record" => {
                if let Some(on) = m.get("on").and_then(Value::as_bool) {
                    if on && !self.recording {
                        self.rec_left.clear();
                        self.rec_right.clear();
                        self.recording = true;
                    } else if !on && self.recording {
                        self.flush_recording(true);
                        self.recording = false;
                    }
                }
                return;
            }
            "telemetry" => {
                if let Some(en) = m.get("enabled").and_then(Value::as_bool) {
                    self.telemetry_enabled = en;
                }
                return;
            }
            _ => {}
        }

        // Per-voice commands
        let track = m.get("track").and_then(Value::as_u64).unwrap_or(0) as usize;
        if track >= MAX_TRACKS {
            return;
        }
        let own = track == 0;

        match ty {
            "body" => {
                if let Some(b) = m.get("body") {
                    let female = b.get("female").and_then(Value::as_f64).unwrap_or(0.0);
                    let child = b.get("child").and_then(Value::as_f64).unwrap_or(0.0);
                    let mix = BodyMix::new(female, child);
                    self.tracks[track].set_body(mix);
                    if own {
                        self.cached_body = mix;
                        self.vowel_corners = load_vowel_corners(mix);
                        self.tracks[0].telemetry(&mut self.tele);
                        self.send_ready();
                    }
                }
            }
            "engine" => {
                if let Some(k) = m.get("kind").and_then(Value::as_u64) {
                    if let Some(kind) = EngineKind::from_index(k as usize) {
                        self.tracks[track].set_kind(kind);
                        if own {
                            self.tracks[0].telemetry(&mut self.tele);
                            self.send_ready();
                        }
                    }
                }
            }
            "harmony" => {
                if let Some(arr) = m.get("ratios").and_then(Value::as_array) {
                    let mut ratios = [1.0_f64; 6];
                    let mut n = 0;
                    for r in arr.iter().take(6) {
                        if let Some(v) = r.as_f64() {
                            ratios[n] = v;
                            n += 1;
                        }
                    }
                    self.tracks[track].set_harmony(&ratios[..n]);
                }
            }
            "artic" => {
                let mut patch = ArticPatch::default();
                if let Some(obj) = m.get("values").and_then(Value::as_object) {
                    for (k, val) in obj {
                        if let (Some(ak), Some(x)) = (ArticKey::from_name(k), val.as_f64()) {
                            patch.set(ak, x);
                        }
                    }
                }
                let tip_free = m.get("tipFree").and_then(Value::as_bool);
                self.tracks[track].set_artic(&patch, tip_free);
            }
            "source" => {
                if let Some(obj) = m.get("values").and_then(Value::as_object) {
                    let f0 = obj.get("f0").and_then(Value::as_f64);
                    let pressure = obj.get("pressure").and_then(Value::as_f64);
                    let vibrato_depth = obj.get("vibratoDepth").and_then(Value::as_f64);
                    let brightness = obj.get("brightness").and_then(Value::as_f64);
                    if own {
                        if let Some(v) = f0 {
                            self.base_f0 = v;
                        }
                        if let Some(v) = pressure {
                            self.base_pressure = v;
                        }
                        if let Some(v) = vibrato_depth {
                            self.base_vibrato_depth = v;
                        }
                        if let Some(v) = brightness {
                            self.base_brightness = v;
                        }
                    }
                    let bend_mult = if own && self.pitch_bend_st.abs() > 1e-5 {
                        chimera::math::pow(2.0, self.pitch_bend_st / 12.0)
                    } else {
                        1.0
                    };
                    let patch = SourcePatch {
                        f0: f0.map(|hz| hz * bend_mult),
                        glide: obj.get("glide").and_then(Value::as_f64),
                        pressure: pressure.map(|p| p * self.expression),
                        breathiness: obj.get("breathiness").and_then(Value::as_f64),
                        tension: obj.get("tension").and_then(Value::as_f64),
                        fry: obj.get("fry").and_then(Value::as_f64),
                        roughness: obj.get("roughness").and_then(Value::as_f64),
                        vibrato_depth: vibrato_depth.map(|vd| (vd + 60.0 * self.mod_wheel).clamp(0.0, 120.0)),
                        vibrato_rate: obj.get("vibratoRate").and_then(Value::as_f64),
                        drift: obj.get("drift").and_then(Value::as_f64),
                        voiced: obj.get("voiced").and_then(Value::as_bool),
                        brightness: brightness.map(|br| (br + 0.25 * self.aftertouch).clamp(0.0, 1.0)),
                        golden: obj.get("golden").and_then(Value::as_f64),
                    };
                    self.tracks[track].set_source(&patch);
                }
            }
            "gate" => {
                if let Some(lvl) = m.get("level").and_then(Value::as_f64) {
                    self.tracks[track].set_gate(lvl);
                }
            }
            "schedule" => {
                let gestures: Vec<Gesture> = m
                    .get("gestures")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                let segments: Vec<PlannedSegment> = m
                    .get("segments")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                let end = m.get("end").and_then(Value::as_f64).unwrap_or(0.0);
                let id = m.get("id").and_then(Value::as_u64).unwrap_or(0);
                let replace = m.get("replace").and_then(Value::as_bool).unwrap_or(true);
                let lead = m.get("lead").and_then(Value::as_f64).unwrap_or(0.03);
                let off = self.tracks[track].schedule(&gestures, lead, replace);
                if !segments.is_empty() {
                    let shifted: Vec<PlannedSegment> = segments
                        .into_iter()
                        .map(|mut s| {
                            s.t0 += off;
                            s.t1 += off;
                            s
                        })
                        .collect();
                    self.send_ui(&json!({
                        "type": "planned",
                        "track": track,
                        "id": id,
                        "t0": off,
                        "end": end + off,
                        "segments": shifted,
                    }));
                }
            }
            "stopSpeech" => {
                self.tracks[track].stop_speech();
            }
            "release" => {
                if let Some(tag) = m.get("tag").and_then(Value::as_f64) {
                    self.tracks[track].release(tag);
                }
            }
            "beat" => {
                if let Some(steps_val) = m.get("steps") {
                    if let Ok(steps) = serde_json::from_value::<Vec<Option<BeatStep>>>(steps_val.clone()) {
                        self.tracks[track].set_beat_steps(&steps);
                    }
                }
                let bpm = m.get("bpm").and_then(Value::as_f64);
                let swing = m.get("swing").and_then(Value::as_f64);
                if bpm.is_some() || swing.is_some() {
                    self.tracks[track].set_beat_tempo(bpm, swing);
                }
                if let Some(playing) = m.get("playing").and_then(Value::as_bool) {
                    self.tracks[track].set_beat_playing(playing);
                }
            }
            "beatHit" => {
                if let Some(id_str) = m.get("id").and_then(Value::as_str) {
                    let step = BeatStep {
                        id: Label::new(id_str),
                        note: m.get("note").and_then(Value::as_f64),
                        accent: m.get("accent").and_then(Value::as_f64),
                    };
                    self.tracks[track].beat_hit(&step);
                }
            }
            "creature" if own => {
                let id_str = m.get("id").and_then(Value::as_str).unwrap_or("human");
                if id_str == "human" {
                    self.tracks[0].set_creature(None, &[]);
                    self.active_creature = None;
                    self.ctele = None;
                } else if let Some(def) = creature_by_id(id_str) {
                    let mut initial = Vec::new();
                    if let Some(pobj) = m.get("params").and_then(Value::as_object) {
                        for (k, v) in pobj {
                            if let (Some(idx), Some(val)) = (def.param_index(k), v.as_f64()) {
                                initial.push((idx, val));
                            }
                        }
                    }
                    self.tracks[0].set_creature(Some(def), &initial);
                    self.active_creature = Some(def);
                    if let Some(cv) = self.tracks[0].creature() {
                        let mut ct = cv.new_telemetry();
                        self.tracks[0].creature_telemetry(&mut ct);
                        self.ctele = Some(ct);
                    }
                }
                self.send_ready();
            }
            "cparams" if own => {
                if let (Some(def), Some(obj)) = (self.active_creature, m.get("values").and_then(Value::as_object)) {
                    for (k, v) in obj {
                        if let (Some(idx), Some(val)) = (def.param_index(k), v.as_f64()) {
                            self.tracks[0].set_creature_params(&[(idx, val)]);
                        }
                    }
                }
            }
            "call" if own => {
                if let (Some(def), Some(id_str)) = (self.active_creature, m.get("id").and_then(Value::as_str)) {
                    if let Some(idx) = def.call_index(id_str) {
                        let pitch = m.get("pitch").and_then(Value::as_f64);
                        let amp = m.get("amp").and_then(Value::as_f64);
                        self.tracks[0].play_call(idx, pitch, amp);
                    }
                }
            }
            "stopCall" if own => {
                self.tracks[0].stop_call();
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ MIDI Events (Ableton Live)

    pub fn note_on(&mut self, note: u8, velocity: f32, params: &SirenParams) {
        let vel = velocity.clamp(0.05, 1.0);
        let vel_midi = (vel * 127.0).round() as u8;
        self.sustained[note as usize] = false;
        self.remove_held(note);
        if self.held_len == MAX_NOTES {
            self.held.copy_within(1..MAX_NOTES, 0);
            self.held_len -= 1;
        }
        self.held[self.held_len] = HeldNote { note, velocity: vel };
        self.held_len += 1;

        if self.ui_driven && self.ui_open.load(Ordering::Relaxed) {
            let route_to_store = self.ui_mode == UiMode::Sing
                || (self.ui_mode == UiMode::Bestiary && self.ui_key_call_active);
            self.send_ui(&json!({
                "type": "vstMidi",
                "on": true,
                "note": note,
                "velocity": vel_midi,
                "routeToStore": route_to_store,
            }));
            if route_to_store {
                return;
            }
            // Sample-accurate immediate audio-thread trigger for Play / Beatbox / Bestiary sustain
            match self.ui_mode {
                UiMode::Beatbox => {
                    let step = beatbox_step_for_note(note, vel);
                    self.tracks[0].beat_hit(&step);
                }
                UiMode::Bestiary => {
                    if let Some(def) = self.active_creature {
                        if let Some((idx, pdef)) = def
                            .params
                            .iter()
                            .enumerate()
                            .find(|(_, d)| d.role == Some(ParamRole::Pitch))
                        {
                            let hz = map_midi_to_creature_hz(note, self.pitch_bend_st, pdef.min, pdef.max);
                            self.tracks[0].set_creature_params(&[(idx, hz)]);
                        }
                        self.tracks[0].set_gate(1.0);
                    }
                }
                _ => {
                    let hz = midi_to_hz(note, self.pitch_bend_st);
                    self.base_f0 = midi_to_hz(note, 0.0);
                    let vel_scale = 0.72 + 0.45 * (vel as f64);
                    self.tracks[0].set_source(&SourcePatch {
                        f0: Some(hz),
                        pressure: Some((self.base_pressure * vel_scale * self.expression).clamp(2.0, 22.0)),
                        ..SourcePatch::default()
                    });
                    self.tracks[0].set_gate(1.0);
                }
            }
            return;
        }

        // Headless fallback (when editor hasn't opened yet)
        let mode = params.voice_mode.value();
        if mode == VoiceMode::BeatboxKit {
            let step = beatbox_step_for_note(note, vel);
            self.tracks[0].beat_hit(&step);
        } else if let Some(c_idx) = mode.creature_index() {
            let def: &'static CreatureDef = &CREATURES[c_idx];
            if let Some(call_slot) = params.creature_call.value().call_slot() {
                if !def.calls.is_empty() {
                    let idx = call_slot.min(def.calls.len() - 1);
                    let ratio = chimera::math::pow(2.0, ((note as f64) - 60.0 + self.pitch_bend_st) / 12.0);
                    self.tracks[0].play_call(idx, Some(ratio), Some((0.55 + 0.45 * vel as f64).clamp(0.2, 1.0)));
                }
            }
        }
    }

    pub fn note_off(&mut self, note: u8) {
        if self.sustain_pedal {
            self.sustained[note as usize] = true;
            return;
        }
        self.remove_held(note);

        if self.ui_driven && self.ui_open.load(Ordering::Relaxed) {
            let route_to_store = self.ui_mode == UiMode::Sing
                || (self.ui_mode == UiMode::Bestiary && self.ui_key_call_active);
            self.send_ui(&json!({
                "type": "vstMidi",
                "on": false,
                "note": note,
                "velocity": 0,
                "routeToStore": route_to_store,
            }));
            if route_to_store {
                return;
            }
            if self.held_len > 0 {
                let top = self.held[self.held_len - 1];
                if self.ui_mode == UiMode::Bestiary {
                    if let Some(def) = self.active_creature {
                        if let Some((idx, pdef)) = def
                            .params
                            .iter()
                            .enumerate()
                            .find(|(_, d)| d.role == Some(ParamRole::Pitch))
                        {
                            let hz = map_midi_to_creature_hz(top.note, self.pitch_bend_st, pdef.min, pdef.max);
                            self.tracks[0].set_creature_params(&[(idx, hz)]);
                        }
                    }
                } else if self.ui_mode != UiMode::Beatbox {
                    self.base_f0 = midi_to_hz(top.note, 0.0);
                    self.tracks[0].set_source(&SourcePatch {
                        f0: Some(midi_to_hz(top.note, self.pitch_bend_st)),
                        ..SourcePatch::default()
                    });
                }
            } else if self.ui_mode != UiMode::Speak && self.ui_mode != UiMode::Band {
                self.tracks[0].set_gate(0.0);
            }
        }
    }

    pub fn set_sustain(&mut self, down: bool) {
        self.sustain_pedal = down;
        if !down {
            let mut released = Vec::new();
            for n in 0..128usize {
                if self.sustained[n] {
                    self.sustained[n] = false;
                    released.push(n as u8);
                }
            }
            for n in released {
                self.note_off(n);
            }
        }
    }

    pub fn set_pitch_bend(&mut self, value_01: f32) {
        self.pitch_bend_st = ((value_01 as f64) * 2.0 - 1.0) * 2.0;
        if self.ui_driven && self.ui_mode != UiMode::Bestiary {
            let bend_mult = chimera::math::pow(2.0, self.pitch_bend_st / 12.0);
            self.tracks[0].set_source(&SourcePatch {
                f0: Some(self.base_f0 * bend_mult),
                ..SourcePatch::default()
            });
        }
    }

    pub fn set_mod_wheel(&mut self, value_01: f32) {
        self.mod_wheel = (value_01 as f64).clamp(0.0, 1.0);
        if self.ui_driven {
            self.tracks[0].set_source(&SourcePatch {
                vibrato_depth: Some((self.base_vibrato_depth + 60.0 * self.mod_wheel).clamp(0.0, 120.0)),
                ..SourcePatch::default()
            });
        }
    }

    pub fn set_expression(&mut self, value_01: f32) {
        self.expression = (0.25 + 0.75 * value_01 as f64).clamp(0.2, 1.0);
        if self.ui_driven {
            self.tracks[0].set_source(&SourcePatch {
                pressure: Some((self.base_pressure * self.expression).clamp(2.0, 22.0)),
                ..SourcePatch::default()
            });
        }
    }

    pub fn set_aftertouch(&mut self, value_01: f32) {
        self.aftertouch = (value_01 as f64).clamp(0.0, 1.0);
        if self.ui_driven {
            self.tracks[0].set_source(&SourcePatch {
                brightness: Some((self.base_brightness + 0.25 * self.aftertouch).clamp(0.0, 1.0)),
                ..SourcePatch::default()
            });
        }
    }

    fn remove_held(&mut self, note: u8) {
        let mut i = 0;
        while i < self.held_len {
            if self.held[i].note == note {
                self.held.copy_within((i + 1)..self.held_len, i);
                self.held_len -= 1;
            } else {
                i += 1;
            }
        }
    }

    // ------------------------------------------------------------------ Headless Param Fallback

    pub fn apply_headless_params(&mut self, p: &SirenParams) {
        if self.ui_driven {
            return;
        }
        let eng = p.engine.value();
        if eng != self.cached_headless_engine {
            self.tracks[0].set_kind(eng.to_chimera());
            self.cached_headless_engine = eng;
        }

        let mode = p.voice_mode.value();
        let target_body = match mode {
            VoiceMode::HumanFemale => BodyMix::new(1.0, 0.0),
            VoiceMode::HumanMale => BodyMix::new(0.0, 0.0),
            VoiceMode::HumanChild => BodyMix::new(0.5, 1.0),
            VoiceMode::HumanMorph => BodyMix::new(p.body_female.value() as f64, p.body_child.value() as f64),
            _ => self.cached_body,
        };

        if mode != self.cached_headless_mode {
            if let Some(c_idx) = mode.creature_index() {
                let def: &'static CreatureDef = &CREATURES[c_idx];
                self.tracks[0].set_creature(Some(def), &[]);
                self.active_creature = Some(def);
            } else {
                self.tracks[0].set_creature(None, &[]);
                self.active_creature = None;
                if mode == VoiceMode::BeatboxKit {
                    let ready = chimera::phonetics::beatbox::beat_ready(target_body);
                    self.tracks[0].set_artic(&ready, Some(false));
                }
            }
            self.cached_headless_mode = mode;
        }

        if mode.is_human() && (target_body.female - self.cached_body.female).abs() + (target_body.child - self.cached_body.child).abs() > 0.002 {
            self.tracks[0].set_body(target_body);
            self.vowel_corners = load_vowel_corners(target_body);
            self.cached_body = target_body;
        }

        let harm = p.harmony.value();
        if harm != self.cached_headless_harmony {
            let ratios: &[f64] = match harm {
                HarmonyMode::Off => &[],
                HarmonyMode::SubOctave => &[0.5],
                HarmonyMode::Fifth => &[1.5],
                HarmonyMode::MajorTriad => &[1.25, 1.5],
                HarmonyMode::GoldenPhi => &[1.0 / PHI, PHI],
                HarmonyMode::CathedralSix => &[0.5, 1.25, 1.5, PHI, 2.0, 2.5],
            };
            self.tracks[0].set_harmony(ratios);
            self.cached_headless_harmony = harm;
        }

        let active = if self.held_len > 0 {
            Some(self.held[self.held_len - 1])
        } else if p.drone.value() {
            Some(HeldNote { note: 57, velocity: 0.8 })
        } else {
            None
        };

        if let Some(c_idx) = mode.creature_index() {
            let def: &'static CreatureDef = &CREATURES[c_idx];
            if let Some(h) = active {
                if let Some((idx, pdef)) = def.params.iter().enumerate().find(|(_, d)| d.role == Some(ParamRole::Pitch)) {
                    let hz = map_midi_to_creature_hz(h.note, self.pitch_bend_st, pdef.min, pdef.max);
                    self.tracks[0].set_creature_params(&[(idx, hz)]);
                }
                if p.creature_call.value().call_slot().is_none() {
                    self.tracks[0].set_gate(1.0);
                }
            } else if p.creature_call.value().call_slot().is_none() {
                self.tracks[0].set_gate(0.0);
            }
        } else if mode == VoiceMode::BeatboxKit {
            self.tracks[0].set_gate(0.0);
        } else {
            let u = (p.vowel_x.value() as f64).clamp(0.0, 1.0);
            let v = (p.vowel_y.value() as f64).clamp(0.0, 1.0);
            let mut artic = ArticPatch::blend(&[
                ((1.0 - u) * (1.0 - v), self.vowel_corners[0]),
                (u * (1.0 - v), self.vowel_corners[1]),
                ((1.0 - u) * v, self.vowel_corners[2]),
                (u * v, self.vowel_corners[3]),
            ]);
            artic
                .set(ArticKey::Velum, p.nasal.value() as f64)
                .set(ArticKey::Epilarynx, p.twang.value() as f64)
                .set(ArticKey::Larynx, p.larynx.value() as f64);
            self.tracks[0].set_artic(&artic, None);

            let target_hz = active
                .map(|h| midi_to_hz(h.note, self.pitch_bend_st))
                .unwrap_or_else(|| default_f0(self.cached_body));
            let vel_scale = active.map(|h| 0.70 + 0.45 * h.velocity as f64).unwrap_or(1.0);

            self.tracks[0].set_source(&SourcePatch {
                f0: Some(target_hz),
                glide: Some(p.glide.value() as f64),
                pressure: Some((p.pressure.value() as f64) * vel_scale * self.expression),
                breathiness: Some(p.breathiness.value() as f64),
                tension: Some(p.tension.value() as f64),
                fry: Some(p.fry.value() as f64),
                roughness: Some(p.roughness.value() as f64),
                vibrato_depth: Some(((p.vibrato_depth.value() as f64) + 60.0 * self.mod_wheel).clamp(0.0, 120.0)),
                vibrato_rate: Some(p.vibrato_rate.value() as f64),
                drift: Some(4.5),
                voiced: Some(p.voiced.value()),
                brightness: Some(((p.brightness.value() as f64) + 0.25 * self.aftertouch).clamp(0.0, 1.0)),
                golden: Some(p.golden_voice.value() as f64),
            });
            self.tracks[0].set_gate(if active.is_some() { 1.0 } else { 0.0 });
        }

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
        let dly = p.fx_delay_mix.value() as f64;
        self.fx.set_on(Fx::Delay, dly > 0.01);
        if dly > 0.01 {
            self.fx.set_param(Fx::Delay, 0, p.fx_delay_time.value() as f64);
            self.fx.set_param(Fx::Delay, 3, dly);
        }
        let gld = p.fx_golden_mix.value() as f64;
        self.fx.set_on(Fx::Golden, gld > 0.01);
        if gld > 0.01 {
            self.fx.set_param(Fx::Golden, 1, p.fx_golden_decay.value() as f64);
            self.fx.set_param(Fx::Golden, 3, gld);
        }
    }

    // ------------------------------------------------------------------ Audio Rendering & Telemetry

    pub fn render_slice(&mut self, left: &mut [f32], right: &mut [f32], out_gain: f32) {
        let n = left.len().min(right.len());
        if n == 0 {
            return;
        }
        let t0 = Instant::now();
        if self.track_buf.len() < n {
            self.track_buf.resize(n, 0.0);
            self.mix_buf.resize(n, 0.0);
        }

        let mix = &mut self.mix_buf[..n];
        if self.active_tracks == 1 && (self.gains[0] - 1.0).abs() < 1e-6 {
            self.tracks[0].render(mix);
        } else {
            mix.fill(0.0);
            for k in 0..self.active_tracks {
                let tb = &mut self.track_buf[..n];
                self.tracks[k].render(tb);
                let g = self.gains[k];
                for i in 0..n {
                    mix[i] += g * tb[i];
                }
            }
        }

        if self.fx.is_idle() {
            left[..n].copy_from_slice(mix);
            right[..n].copy_from_slice(mix);
        } else {
            self.fx.process(mix, &mut left[..n], &mut right[..n]);
        }

        if self.close_mic_on {
            self.close_mic.process(&mut left[..n], &mut right[..n]);
        }

        if self.recording {
            if self.rec_left.len() + n > REC_CHUNK {
                self.flush_recording(false);
            }
            self.rec_left.extend_from_slice(&left[..n]);
            self.rec_right.extend_from_slice(&right[..n]);
        }

        let total_gain = if self.ui_driven {
            self.master_volume * out_gain
        } else {
            out_gain
        };

        let ui_open = self.ui_open.load(Ordering::Relaxed);
        for i in 0..n {
            let l = left[i] * total_gain;
            let r = right[i] * total_gain;
            left[i] = l;
            right[i] = r;
            if ui_open && self.pcm_tap.len() < PCM_TAP_CAP {
                self.pcm_tap.push(0.5 * (l + r));
            }
        }

        let elapsed_s = t0.elapsed().as_secs_f64();
        let budget_s = (n as f64) / self.sample_rate;
        self.load_ema += 0.05 * ((elapsed_s / budget_s.max(1e-9)) - self.load_ema);

        self.frames_since_tele += n;
        let tele_interval = (self.sample_rate / 60.0) as usize;
        if self.frames_since_tele >= tele_interval {
            self.frames_since_tele = 0;
            if ui_open && self.telemetry_enabled {
                self.emit_telemetry();
            } else {
                self.pcm_tap.clear();
            }
        }
    }

    fn flush_recording(&mut self, last: bool) {
        if self.rec_left.is_empty() && !last {
            return;
        }
        let left_b64 = encode_f32_le(&self.rec_left);
        let right_b64 = encode_f32_le(&self.rec_right);
        self.rec_left.clear();
        self.rec_right.clear();
        self.send_ui(&json!({
            "type": "vstRecorded",
            "leftB64": left_b64,
            "rightB64": right_b64,
            "last": last,
        }));
    }

    pub fn emit_telemetry(&mut self) {
        let pcm_b64 = encode_f32_le(&self.pcm_tap);
        self.pcm_tap.clear();

        if let Some(def) = self.active_creature {
            let msg = {
                let Some(ct) = self.ctele.as_mut() else {
                    return;
                };
                if !self.tracks[0].creature_telemetry(ct) {
                    return;
                }
                let mut params_map = serde_json::Map::new();
                for (k, v) in &ct.params {
                    params_map.insert((*k).to_string(), json!(*v));
                }
                let mut source_map = serde_json::Map::new();
                for (k, v) in &ct.source {
                    source_map.insert((*k).to_string(), json!(*v));
                }
                let tubes_vec: Vec<Value> = ct
                    .tubes
                    .iter()
                    .map(|tb| {
                        json!({
                            "name": tb.name,
                            "areasB64": encode_f32_le(&tb.areas),
                            "rmsB64": encode_f32_le(&tb.rms),
                        })
                    })
                    .collect();
                json!({
                    "type": "vstCreatureTelemetry",
                    "load": self.load_ema,
                    "id": def.id,
                    "gate": ct.gate,
                    "outRms": ct.out_rms,
                    "clock": ct.clock,
                    "call": ct.call,
                    "params": params_map,
                    "source": source_map,
                    "tubes": tubes_vec,
                    "pcmB64": pcm_b64,
                })
            };
            self.send_ui(&msg);
            return;
        }

        self.tracks[0].telemetry(&mut self.tele);
        let t = &self.tele;
        let info = self.tracks[0].info();
        let n_ph = t.ph_areas.len();
        let n_or = t.or_areas.len();
        let n_artic = t.artic.len().min(N_ARTIC);
        let n_scope = t.glottal_area.len();

        self.packed_tele.clear();
        self.packed_tele.extend_from_slice(&t.artic[..n_artic]);
        self.packed_tele.extend_from_slice(&t.ph_areas);
        self.packed_tele.extend_from_slice(&t.or_areas);
        self.packed_tele.extend_from_slice(&t.pressure_rms);
        self.packed_tele.extend_from_slice(&t.flow);
        self.packed_tele.extend_from_slice(&t.glottal_area);
        self.packed_tele.extend_from_slice(&t.glottal_flow);

        let active_labels: Vec<String> = t.active.iter().map(|l| l.as_str().to_string()).collect();

        self.send_ui(&json!({
            "type": "vstTelemetry",
            "load": self.load_ema,
            "body": { "female": info.body.female, "child": info.body.child },
            "nPh": n_ph,
            "nOr": n_or,
            "beatStep": self.tracks[0].beat_step(),
            "phLen": t.ph_len,
            "orLen": t.or_len,
            "velumPort": t.velum_port,
            "f0": t.f0,
            "voicing": t.voicing,
            "pLung": t.p_lung,
            "outRms": t.out_rms,
            "noiseLevel": t.noise_level,
            "noiseNode": t.noise_node,
            "gate": t.gate,
            "clock": t.clock,
            "active": active_labels,
            "nArtic": n_artic,
            "nScope": n_scope,
            "packedB64": encode_f32_le(&self.packed_tele),
            "pcmB64": pcm_b64,
        }));
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
pub fn midi_to_hz(note: u8, bend_st: f64) -> f64 {
    440.0 * chimera::math::pow(2.0, ((note as f64) - 69.0 + bend_st) / 12.0)
}

fn map_midi_to_creature_hz(note: u8, bend_st: f64, lo: f64, hi: f64) -> f64 {
    let mid = (lo * hi).sqrt();
    let shift = chimera::math::log2(mid / midi_to_hz(60, 0.0)).round();
    let mut f = midi_to_hz(note, bend_st) * chimera::math::pow(2.0, shift);
    while f > hi {
        f *= 0.5;
    }
    while f < lo {
        f *= 2.0;
    }
    f.min(hi)
}

fn beatbox_step_for_note(note: u8, velocity: f32) -> BeatStep {
    let accent = Some((0.6 + 0.8 * velocity as f64).clamp(0.5, 1.5));
    match note {
        35 | 36 | 41 => BeatStep { id: Label::new("B"), note: None, accent },
        37 => BeatStep { id: Label::new("K"), note: None, accent },
        38 => BeatStep { id: Label::new("Pf"), note: None, accent },
        39 => BeatStep { id: Label::new("P!"), note: None, accent },
        40 => BeatStep { id: Label::new("^K"), note: None, accent },
        42 => BeatStep { id: Label::new("t"), note: None, accent },
        44 => BeatStep { id: Label::new("!"), note: None, accent },
        46 => BeatStep { id: Label::new("tss"), note: None, accent },
        49 | 51 => BeatStep { id: Label::new("kch"), note: None, accent },
        _ => BeatStep {
            id: Label::new("m"),
            note: Some(note as f64),
            accent,
        },
    }
}
