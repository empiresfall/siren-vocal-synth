//! SIREN Vocal & Bestiary Synthesizer — 1:1 VST3 / CLAP instrument plugin for Ableton Live.
//!
//! Hosts the full interactive SIREN web interface (Three.js 3D anatomical head, NORAD phosphor
//! vector display, tube/glottis/spectrum/vowel plots, 6 modes including Russian/English G2P,
//! Band 4-track arranger, and 8-unit FX rack) inside a native macOS `WKWebView` via `siren://`,
//! while rendering all synthesis and studio FX 100% natively on Ableton Live's audio thread.

pub mod b64;
pub mod dsp;
#[cfg(target_os = "macos")]
pub mod webview_macos;

include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));

use chimera::anatomy::human::BodyMix;
use chimera::engine::EngineKind;
use crossbeam_channel::{bounded, Receiver, Sender};
use dsp::DspCore;
use nih_plug::prelude::*;
use parking_lot::Mutex;
use serde_json::Value;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

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
    pub fn is_human(self) -> bool {
        matches!(
            self,
            VoiceMode::HumanFemale
                | VoiceMode::HumanMale
                | VoiceMode::HumanChild
                | VoiceMode::HumanMorph
        )
    }

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
    pub fn call_slot(self) -> Option<usize> {
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
    #[id = "golden"]
    #[name = "Golden Ratio Choir (φ)"]
    GoldenPhi,
    #[id = "cathedral"]
    #[name = "Cathedral 6-Voice"]
    CathedralSix,
}

#[derive(Params)]
pub struct SirenParams {
    /// Full serialized UI state saved inside Ableton `.als` projects.
    #[persist = "ui-state-json"]
    pub ui_state_json: Mutex<String>,

    #[id = "engine"]
    pub engine: EnumParam<EngineChoice>,

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
            ui_state_json: Mutex::new(String::new()),

            engine: EnumParam::new("Engine Model", EngineChoice::Airway),
            voice_mode: EnumParam::new("Voice / Creature", VoiceMode::HumanMale),
            body_female: FloatParam::new("Body Female", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(20.0)),
            body_child: FloatParam::new("Body Child", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(20.0)),
            creature_call: EnumParam::new("Creature Trigger", CreatureCallMode::Sustained),
            drone: BoolParam::new("Audition Drone", false),

            vowel_x: FloatParam::new("Vowel Back (X)", 0.72, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(18.0)),
            vowel_y: FloatParam::new("Vowel Open (Y)", 0.78, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(18.0)),
            nasal: FloatParam::new("Nasal Velum", 0.0, FloatRange::Linear { min: -0.25, max: 1.2 })
                .with_unit(" cm²")
                .with_smoother(SmoothingStyle::Linear(18.0)),
            twang: FloatParam::new("Epilarynx Twang", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(18.0)),
            larynx: FloatParam::new("Larynx Height", 0.0, FloatRange::Linear { min: -1.5, max: 1.5 })
                .with_unit(" cm")
                .with_smoother(SmoothingStyle::Linear(18.0)),

            pressure: FloatParam::new("Lung Pressure", 8.0, FloatRange::Linear { min: 2.0, max: 22.0 })
                .with_unit(" cmH2O")
                .with_smoother(SmoothingStyle::Linear(15.0)),
            breathiness: FloatParam::new("Breathiness", 0.14, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(15.0)),
            tension: FloatParam::new("Fold Tension", 0.45, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(15.0)),
            brightness: FloatParam::new("Brightness", 0.25, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(15.0)),
            fry: FloatParam::new("Vocal Fry", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(15.0)),
            roughness: FloatParam::new("Roughness", 0.08, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(15.0)),
            voiced: BoolParam::new("Voiced Folds", true),

            vibrato_depth: FloatParam::new("Vibrato Depth", 10.0, FloatRange::Linear { min: 0.0, max: 120.0 })
                .with_unit(" ct")
                .with_smoother(SmoothingStyle::Linear(20.0)),
            vibrato_rate: FloatParam::new("Vibrato Rate", 5.5, FloatRange::Linear { min: 2.0, max: 10.0 })
                .with_unit(" Hz")
                .with_smoother(SmoothingStyle::Linear(20.0)),
            glide: FloatParam::new(
                "Portamento Glide",
                0.06,
                FloatRange::Skewed {
                    min: 0.005,
                    max: 0.60,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" s"),
            golden_voice: FloatParam::new("Golden Pulse (1/φ)", 0.20, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(20.0)),
            harmony: EnumParam::new("Harmony Choir", HarmonyMode::Off),

            fx_punch: FloatParam::new("Punch Comp", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            fx_phone: FloatParam::new("Phone Lo-Fi", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            fx_delay_mix: FloatParam::new("Ping-Pong Delay", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            fx_delay_time: FloatParam::new("Delay Time", 0.38, FloatRange::Linear { min: 0.0, max: 1.0 }),
            fx_golden_mix: FloatParam::new("Golden Reverb", 0.22, FloatRange::Linear { min: 0.0, max: 1.0 }),
            fx_golden_decay: FloatParam::new("Reverb Decay", 0.55, FloatRange::Linear { min: 0.0, max: 1.0 }),

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

pub struct SirenPlugin {
    params: Arc<SirenParams>,
    ui_open: Arc<AtomicBool>,
    host_params_dirty: Arc<AtomicBool>,
    setting_from_ui: Arc<AtomicBool>,
    cmd_tx: Sender<Value>,
    cmd_rx: Receiver<Value>,
    evt_tx: Sender<String>,
    evt_rx: Receiver<String>,
    core: DspCore,
}

impl Default for SirenPlugin {
    fn default() -> Self {
        chimera::init();
        let params = Arc::new(SirenParams::default());
        let ui_open = Arc::new(AtomicBool::new(false));
        let host_params_dirty = Arc::new(AtomicBool::new(false));
        let setting_from_ui = Arc::new(AtomicBool::new(false));
        let (cmd_tx, cmd_rx) = bounded(512);
        let (evt_tx, evt_rx) = bounded(256);
        let core = DspCore::new(
            48_000.0,
            2048,
            ui_open.clone(),
            cmd_rx.clone(),
            evt_tx.clone(),
        );
        Self {
            params,
            ui_open,
            host_params_dirty,
            setting_from_ui,
            cmd_tx,
            cmd_rx,
            evt_tx,
            evt_rx,
            core,
        }
    }
}

impl Plugin for SirenPlugin {
    const NAME: &'static str = "SIREN Vocal Synth";
    const VENDOR: &'static str = "CHIMERA";
    const URL: &'static str = "https://siren-vocal-synth.vercel.app";
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
        #[cfg(target_os = "macos")]
        {
            Some(Box::new(webview_macos::SirenWebViewEditor {
                params: self.params.clone(),
                ui_open: self.ui_open.clone(),
                cmd_tx: self.cmd_tx.clone(),
                evt_rx: self.evt_rx.clone(),
                host_params_dirty: self.host_params_dirty.clone(),
                setting_from_ui: self.setting_from_ui.clone(),
            }))
        }
        #[cfg(not(target_os = "macos"))]
        {
            None
        }
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.core = DspCore::new(
            buffer_config.sample_rate as f64,
            buffer_config.max_buffer_size as usize,
            self.ui_open.clone(),
            self.cmd_rx.clone(),
            self.evt_tx.clone(),
        );
        self.core.apply_headless_params(&self.params);
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

        // Drain pending UI messages (`ToWorklet` + VST lifecycle commands) without blocking
        if !self.cmd_rx.is_empty() {
            let persisted = self
                .params
                .ui_state_json
                .try_lock()
                .map(|g| g.clone())
                .unwrap_or_default();
            self.core.drain_ui_commands(&persisted);
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
                self.core.apply_headless_params(&self.params);
                self.core
                    .render_slice(&mut left[cursor..next_stop], &mut right[cursor..next_stop], gain);
                cursor = next_stop;
            }
        }

        ProcessStatus::KeepAlive
    }
}

impl SirenPlugin {
    fn handle_midi_event(&mut self, event: NoteEvent<()>) {
        match event {
            NoteEvent::NoteOn { note, velocity, .. } => {
                if velocity <= 0.0001 {
                    self.core.note_off(note);
                } else {
                    self.core.note_on(note, velocity, &self.params);
                }
            }
            NoteEvent::NoteOff { note, .. } => {
                self.core.note_off(note);
            }
            NoteEvent::MidiPitchBend { value, .. } => {
                self.core.set_pitch_bend(value);
            }
            NoteEvent::PolyPressure { pressure, .. } | NoteEvent::MidiChannelPressure { pressure, .. } => {
                self.core.set_aftertouch(pressure);
            }
            NoteEvent::MidiCC { cc, value, .. } => match cc {
                1 => self.core.set_mod_wheel(value),
                11 => self.core.set_expression(value),
                64 => self.core.set_sustain(value >= 0.5),
                74 => self.core.set_aftertouch(value),
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
        Some("Physical-modelling articulatory human voice, 3D airway head, choir, beatbox & creature synthesizer");
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
    use serde_json::json;
    use std::sync::atomic::Ordering;

    fn rms(buf: &[f32]) -> f32 {
        let s: f32 = buf.iter().map(|x| x * x).sum();
        (s / buf.len().max(1) as f32).sqrt()
    }

    #[test]
    fn embedded_web_assets_contain_full_ui_and_3d_head() {
        let (mime, html) = lookup_web_asset("/index.html").expect("expected /index.html in embedded dist/");
        assert!(mime.starts_with("text/html"));
        let html_str = std::str::from_utf8(html).unwrap();
        assert!(html_str.contains("root"), "index.html should mount React root");
        assert!(
            lookup_web_asset("/data/cmudict.txt.gz").is_some(),
            "expected cmudict.txt.gz for Speak/Sing G2P inside VST"
        );
    }

    #[test]
    fn ui_bridge_and_midi_render_voiced_stereo_with_60hz_telemetry() {
        chimera::init();
        let params = SirenParams::default();
        let ui_open = Arc::new(AtomicBool::new(true));
        let (cmd_tx, cmd_rx) = bounded(64);
        let (evt_tx, evt_rx) = bounded(64);
        let mut core = DspCore::new(48_000.0, 1024, ui_open.clone(), cmd_rx, evt_tx);

        // Simulate WKWebView startup handshake + Play mode vowel/source
        cmd_tx.send(json!({ "type": "vstReady" })).unwrap();
        cmd_tx
            .send(json!({
                "type": "artic",
                "values": { "jaw": 0.65, "tongueBody": -0.2, "lipOpen": 0.7 }
            }))
            .unwrap();
        cmd_tx
            .send(json!({
                "type": "source",
                "values": { "f0": 220.0, "pressure": 10.0, "voiced": true, "vibratoDepth": 14.0 }
            }))
            .unwrap();
        core.drain_ui_commands("");

        // Play MIDI note C4 from Ableton Live
        core.note_on(60, 0.9, &params);

        let mut left = vec![0.0f32; 4096];
        let mut right = vec![0.0f32; 4096];
        for chunk in 0..4 {
            let a = chunk * 1024;
            let b = a + 1024;
            core.render_slice(&mut left[a..b], &mut right[a..b], 1.0);
        }

        assert!(left.iter().all(|x| x.is_finite()));
        assert!(right.iter().all(|x| x.is_finite()));
        assert!(
            rms(&left[1024..]) > 0.005,
            "expected audible human vocal output, got rms={}",
            rms(&left[1024..])
        );

        // Verify `ready`, `vstMidi`, and `vstTelemetry` events were emitted for the 3D head
        let mut got_ready = false;
        let mut got_midi = false;
        let mut got_tele = false;
        while let Ok(msg_str) = evt_rx.try_recv() {
            let v: Value = serde_json::from_str(&msg_str).unwrap();
            match v.get("type").and_then(Value::as_str) {
                Some("ready") => got_ready = true,
                Some("vstMidi") => got_midi = true,
                Some("vstTelemetry") => {
                    got_tele = true;
                    let b64_str = v.get("packedB64").and_then(Value::as_str).unwrap();
                    let decoded = b64::decode_bytes(b64_str).unwrap();
                    assert!(decoded.len() > 200, "telemetry payload should contain tract + glottal arrays");
                }
                _ => {}
            }
        }
        assert!(got_ready && got_midi && got_tele);
        ui_open.store(false, Ordering::Relaxed);
    }

    #[test]
    fn headless_beatbox_and_creature_modes_produce_clean_audio() {
        chimera::init();
        let params = SirenParams::default();
        let ui_open = Arc::new(AtomicBool::new(false));
        let (_cmd_tx, cmd_rx) = bounded(16);
        let (evt_tx, _evt_rx) = bounded(16);
        let mut core = DspCore::new(48_000.0, 1024, ui_open, cmd_rx, evt_tx);

        // Beatbox pad hit via UI command
        _cmd_tx.send(json!({ "type": "vstReady" })).unwrap();
        _cmd_tx.send(json!({ "type": "beatHit", "id": "B", "accent": 1.1 })).unwrap();
        core.drain_ui_commands("");

        let mut left = vec![0.0f32; 8192];
        let mut right = vec![0.0f32; 8192];
        core.render_slice(&mut left, &mut right, 1.0);
        assert!(rms(&left) > 0.002, "expected audible beatbox kick, got rms={}", rms(&left));

        // Creature switch + call via UI command
        _cmd_tx.send(json!({ "type": "creature", "id": "songbird" })).unwrap();
        _cmd_tx
            .send(json!({ "type": "call", "id": chimera::creatures::CREATURES[0].calls[0].id }))
            .unwrap();
        core.drain_ui_commands("");

        let mut c_left = vec![0.0f32; 8192];
        let mut c_right = vec![0.0f32; 8192];
        core.render_slice(&mut c_left, &mut c_right, 1.0);
        assert!(c_left.iter().all(|x| x.is_finite()));
        assert!(rms(&c_left) > 0.001, "expected audible songbird call, got rms={}", rms(&c_left));
        let _ = params;
    }
}
