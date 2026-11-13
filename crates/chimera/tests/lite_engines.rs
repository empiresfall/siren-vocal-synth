//! The light engines (waveguide, formant) through `Engine`: they speak the same plans as the
//! airway voice at comparable levels, hold at any device rate, don't depend on the host's block
//! size, and the harmony and golden voices work in all three.
use chimera::analysis::pitch::{YinOpts, yin_pitch};
use chimera::engine::{Engine, EngineKind};
use chimera::phonetics::ipa::tokenize_ipa;
use chimera::phonetics::planner::{PlanOptions, plan_utterance, vowel_table};
use chimera::phonetics::Plan;
use chimera::voice::human_voice::SourcePatch;

const FS: f64 = 48000.0;

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}
fn db(x: f64) -> f64 {
    20.0 * (x + 1e-12).log10()
}

fn plan(text: &str) -> Plan {
    plan_utterance(&tokenize_ipa(text), &PlanOptions { body: "male".into(), t0: 0.05, f0: Some(110.0), ..Default::default() })
}

fn say(kind: EngineKind, fs: f64, text: &str, block: usize) -> (Vec<f32>, Plan) {
    let mut e = Engine::new(fs);
    e.set_kind(kind);
    e.set_source(&SourcePatch::f0(110.0));
    let p = plan(text);
    e.schedule(&p.gestures, 0.0, true);
    let mut out = vec![0f32; ((p.end + 0.25) * fs) as usize];
    for b in out.chunks_mut(block) {
        e.render(b);
    }
    (out, p)
}

/// RMS level (dB) of each planned segment.
fn levels(out: &[f32], p: &Plan, fs: f64) -> Vec<(String, f64)> {
    p.segments.iter().map(|s| (s.ipa.clone(), db(rms(&out[(s.t0 * fs) as usize..((s.t1 * fs) as usize).min(out.len())])))).collect()
}

#[test]
fn the_light_engines_speak_at_the_airways_levels() {
    for text in ["ˈɑsɑ", "ˈɑʃɑ", "ˈɑmɑ", "ˈɑlɑ", "ˈɑzɑ", "ˈitu"] {
        let (a, p) = say(EngineKind::Airway, FS, text, 128);
        let la = levels(&a, &p, FS);
        for kind in [EngineKind::Waveguide, EngineKind::Formant, EngineKind::Neural] {
            let (o, _) = say(kind, FS, text, 128);
            assert!(o.iter().all(|x| x.is_finite() && x.abs() <= 1.0), "{kind:?} {text}");
            let lo = levels(&o, &p, FS);
            println!("{text} {kind:?}: {:?}  (airway {:?})", lo.iter().map(|(i, d)| format!("{i} {d:.1}")).collect::<Vec<_>>(), la.iter().map(|(i, d)| format!("{i} {d:.1}")).collect::<Vec<_>>());
            for ((ipa, d), (_, r)) in lo.iter().zip(&la) {
                if *r < -45.0 {
                    // a silent closure in the airway voice is one here
                    assert!(*d < -40.0, "{kind:?} {text}: /{ipa}/ at {d:.1} dB, the airway's is silent ({r:.1} dB)");
                } else {
                    assert!((d - r).abs() < 9.0, "{kind:?} {text}: /{ipa}/ at {d:.1} dB, the airway's at {r:.1} dB");
                }
            }
            // no click: the loudest sample is not far above the stressed vowel's level
            let peak = o.iter().fold(0f32, |m, x| m.max(x.abs())) as f64;
            assert!(peak < 0.7, "{kind:?} {text}: peak {peak:.2}");
        }
    }
}

#[test]
fn a_stop_is_silent_then_bursts_in_every_engine() {
    for kind in EngineKind::ALL {
        let (o, p) = say(kind, FS, "ˈɑtɑ", 128);
        let t = p.segments.iter().find(|s| s.ipa == "t").unwrap();
        let v = &p.segments[0];
        let vowel = db(rms(&o[(v.t0 * FS) as usize..(v.t1 * FS) as usize]));
        // the quietest 15 ms inside the closure
        let (a, b) = ((t.t0 * FS) as usize, (t.t1 * FS) as usize);
        let quiet = (a..b - 720).step_by(120).map(|i| db(rms(&o[i..i + 720]))).fold(f64::INFINITY, f64::min);
        println!("{kind:?}: vowel {vowel:.1} dB, closure {quiet:.1} dB");
        assert!(quiet < vowel - 20.0, "{kind:?}: the closure of /t/ is at {quiet:.1} dB, the vowel at {vowel:.1}");
    }
}

#[test]
fn the_light_engines_hold_at_any_device_rate() {
    for kind in [EngineKind::Waveguide, EngineKind::Formant, EngineKind::Neural] {
        for fs in [16000.0, 22050.0, 32000.0, 44100.0, 48000.0, 88200.0, 96000.0] {
            let (o, p) = say(kind, fs, "ˈɑsɑ ˈmitu", 128);
            assert!(o.iter().all(|x| x.is_finite()), "{kind:?} at {fs} Hz");
            let v = &p.segments[0];
            let level = db(rms(&o[(v.t0 * fs) as usize..(v.t1 * fs) as usize]));
            let peak = o.iter().fold(0f32, |m, x| m.max(x.abs()));
            println!("{kind:?} {fs}: vowel {level:.1} dB peak {peak:.2}");
            assert!(level > -32.0 && level < -8.0 && peak < 0.8, "{kind:?} at {fs} Hz: vowel {level:.1} dB, peak {peak:.2}");
        }
    }
}

#[test]
fn the_light_engines_do_not_depend_on_the_block_size() {
    for kind in [EngineKind::Waveguide, EngineKind::Formant, EngineKind::Neural] {
        let (reference, _) = say(kind, FS, "ˈɑsɑ", 128);
        for block in [1, 7, 64, 500, 48000] {
            let (o, _) = say(kind, FS, "ˈɑsɑ", block);
            assert!(o == reference, "{kind:?}: blocks of {block} differ from blocks of 128");
        }
    }
}

/// A held vowel: the last `n` samples.
fn held(kind: EngineKind, setup: impl Fn(&mut Engine), n: usize) -> Vec<f32> {
    let mut e = Engine::new(FS);
    e.set_kind(kind);
    e.set_artic(&vowel_table()["male"]["ɑ"].artic, Some(true));
    e.set_source(&SourcePatch::f0(110.0));
    setup(&mut e);
    e.set_gate(1.0);
    let mut out = vec![0f32; 48000];
    e.render(&mut out);
    out[48000 - n..].to_vec()
}

#[test]
fn a_harmony_voice_sings_its_own_pitch_through_the_same_mouth() {
    for kind in EngineKind::ALL {
        let solo = held(kind, |_| {}, 9600);
        // a fifth above: the two together repeat at half the lead's pitch (2:3)
        let duo = held(kind, |e| e.set_harmony(&[1.5]), 9600);
        assert!(duo.iter().all(|x| x.is_finite()));
        let f = |x: &[f32]| yin_pitch(&x.iter().map(|v| *v as f64).collect::<Vec<_>>(), FS, YinOpts { min_f0: Some(40.0), max_f0: Some(400.0), ..Default::default() }).f0;
        let (fs_, fd) = (f(&solo), f(&duo));
        println!("{kind:?}: solo {fs_:.1} Hz, with a fifth {fd:.1} Hz, level {:.1} → {:.1} dB", db(rms(&solo)), db(rms(&duo)));
        assert!((fs_ - 110.0).abs() < 3.0, "{kind:?}: solo at {fs_:.1} Hz");
        assert!((fd - 55.0).abs() < 3.0, "{kind:?}: lead and fifth should repeat at 55 Hz, found {fd:.1}");
        assert!(rms(&duo) > 1.05 * rms(&solo), "{kind:?}: the harmony adds nothing");
        // and off again is the solo voice
        let off = held(kind, |e| { e.set_harmony(&[1.5]); e.set_harmony(&[]); }, 9600);
        assert!(off == solo, "{kind:?}: harmony off is not the solo voice");
    }
}

#[test]
fn the_golden_voice_changes_the_sound_and_zero_is_the_plain_voice() {
    for kind in EngineKind::ALL {
        let plain = held(kind, |_| {}, 9600);
        let zero = held(kind, |e| e.set_source(&SourcePatch { golden: Some(0.0), ..Default::default() }), 9600);
        assert!(zero == plain, "{kind:?}: golden 0 changed the voice");
        let gold = held(kind, |e| e.set_source(&SourcePatch { golden: Some(1.0), ..Default::default() }), 9600);
        assert!(gold.iter().all(|x| x.is_finite()));
        assert!(gold != plain, "{kind:?}: golden 1 is the plain voice");
        println!("{kind:?}: plain {:.1} dB, golden {:.1} dB", db(rms(&plain)), db(rms(&gold)));
    }
}

#[test]
fn switching_engines_keeps_the_performers_settings() {
    let mut e = Engine::new(FS);
    e.set_source(&SourcePatch { f0: Some(180.0), breathiness: Some(0.4), golden: Some(0.5), ..Default::default() });
    e.set_artic(&vowel_table()["male"]["i"].artic, Some(true));
    e.set_harmony(&[1.25, 1.5]);
    e.set_gate(1.0);
    let (n_ph, n_or) = (e.info().n_ph, e.info().n_or);
    for kind in [EngineKind::Formant, EngineKind::Neural, EngineKind::Waveguide, EngineKind::Airway] {
        let info = e.set_kind(kind);
        assert_eq!(e.kind(), kind);
        assert_eq!((info.n_ph, info.n_or), (n_ph, n_or), "the head's sections are the same in every engine");
        let s = e.voice().source();
        assert_eq!((s.f0, s.breathiness, s.golden), (180.0, 0.4, 0.5));
        assert_eq!(e.voice().harmony(), &[1.25, 1.5]);
        let mut out = vec![0f32; 24000];
        e.render(&mut out);
        assert!(db(rms(&out[12000..])) > -35.0, "{kind:?} is silent after the switch");
    }
}
