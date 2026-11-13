//! Domestic cat: bright, agile feline vocal folds in a 8 cm vocal tract that sweeps `[m-i-a-u]`
//! vowel contours during a meow, switches to a 26 Hz neural laryngeal flutter for purring, and
//! narrows the palatal arch with an open glottis to hiss.

use super::build::{CMH2O, fill_areas};
use super::mammal::{MammalAirway, MammalSpec, mammal_airway, mammal_noise, step_glottis};
use super::types::{CallDef, CallKey, CreatureDef, CreatureGroup, CreatureSim, Curve, GateTimes, ParamDef};
use crate::airway::network::{AirwayNetwork, Tube};
use crate::math::{PI, js_max, js_min, sin};

const PITCH: usize = 0;
const VOWEL: usize = 1;
const PURR: usize = 2;
const PURR_RATE: usize = 3;
const HISS: usize = 4;
const PRESSURE: usize = 5;
const NASAL: usize = 6;

pub struct Cat {
    a: MammalAirway,
    tubes: Vec<(&'static str, Tube)>,
    velum: usize,
    purr_phase: f64,
}

fn create(fs: f64) -> Box<dyn CreatureSim> {
    let mut a = mammal_airway(
        fs,
        MammalSpec { trachea: (9.5, 1.1), pharynx: (3.8, 1.2), oral: (4.6, 1.4), nasal: Some((4.8, 0.65)), wall: None, wall_radiation: Some(0.35), coarse: None, nasal_radiates: None },
        |_, _| {},
    );
    a.glottis_src.params.jitter = 0.004;
    a.glottis_src.params.shimmer = 0.02;
    a.glottis_src.params.drift = 3.0;
    let nasal = a.nasal.expect("cat has a nasal tube");
    let velum = a.velum.expect("cat has a velum");
    let tubes = vec![("trachea", a.trachea), ("pharynx", a.pharynx), ("oral", a.oral), ("nasal", nasal)];
    Box::new(Cat { a, tubes, velum, purr_phase: 0.0 })
}

impl CreatureSim for Cat {
    fn net(&self) -> Option<&AirwayNetwork> {
        Some(&self.a.net)
    }
    fn net_mut(&mut self) -> Option<&mut AirwayNetwork> {
        Some(&mut self.a.net)
    }
    fn tubes(&self) -> &[(&'static str, Tube)] {
        &self.tubes
    }
    fn gain(&self) -> f64 {
        2.2e-7
    }
    fn control(&mut self, p: &[f64], gate: f64, dt: f64) {
        let purr = p[PURR];
        self.purr_phase += p[PURR_RATE] * dt;
        self.purr_phase -= self.purr_phase.floor();
        let flutter = 0.5 + 0.5 * sin(2.0 * PI * self.purr_phase);
        let env = (1.0 - 0.72 * purr) + 0.72 * purr * flutter;

        let a = &mut self.a;
        a.net.set_fixed_pressure(a.lungs, p[PRESSURE] * CMH2O * gate * env);

        let hiss = p[HISS];
        let g = &mut a.glottis_src.params;
        // purring drops the folds into a low (~28 Hz) slack register; meowing sings in the feline treble
        g.f0 = p[PITCH] * (1.0 - purr) + 32.0 * purr;
        g.amplitude = (0.16 + 0.1 * purr) * (1.0 - 0.9 * hiss);
        g.abduction = 0.015 + 0.55 * hiss + 0.06 * purr * (1.0 - flutter);
        g.open_quotient = 0.46 + 0.16 * purr;
        g.closure_sharpness = 0.65 - 0.25 * purr;
        g.effort = 0.75;
        g.chaos = 0.18 * purr;

        // vowel 0..1 sweeps closed [u/m] -> bright front [i] -> wide open [æ/ɑ]
        let v = p[VOWEL];
        let ph_open = 0.9 + 0.9 * (1.0 - v);
        let ph_areas = fill_areas(a.pharynx, |_| ph_open, &mut a.phar_area);
        a.net.set_tube_targets(a.pharynx, ph_areas, None, None);

        let or_areas = fill_areas(
            a.oral,
            |u| {
                if hiss > 0.05 && u > 0.35 && u < 0.65 {
                    // arched tongue palatal constriction for a sharp feline hiss
                    js_max(0.08, 1.3 * (1.0 - 0.92 * hiss))
                } else if u > 0.8 {
                    // lip opening widens across the meow arc
                    0.08 + 2.4 * js_max(v, 0.45 * hiss)
                } else {
                    0.7 + 1.3 * v + 0.4 * sin(PI * u) * (v - 0.5)
                }
            },
            &mut a.oral_area,
        );
        a.net.set_tube_targets(a.oral, or_areas, None, None);
        a.net.set_port_target(self.velum, 0.05 + 0.75 * js_max(p[NASAL], 0.7 * purr));
        mammal_noise(a, 2.5e-6 + 1.8e-5 * hiss, Some(3.2e-5 * hiss));
    }
    #[inline]
    fn sample(&mut self) -> f64 {
        step_glottis(&mut self.a);
        self.a.net.step()
    }
    fn source_state(&self, out: &mut Vec<(&'static str, f64)>) {
        out.clear();
        out.push(("glottis", js_min(1.0, self.a.glottis_src.area / 0.22)));
    }
}

pub fn def() -> CreatureDef {
    let k = CallKey::new;
    CreatureDef {
        id: "cat",
        name: "Domestic cat",
        latin: "Felis catus",
        group: CreatureGroup::Mammal,
        blurb: "Short tract sweeping [m-i-a-u] vowel arcs on a meow, a 26 Hz neural flutter for purring, and a narrow palatal hiss.",
        role: "Expressive vocal leads, rhythmic sub-purr beds, percussive hisses",
        oversample: 2,
        gate_times: Some(GateTimes { attack: 0.01, release: 0.045 }),
        params: vec![
            ParamDef::new("pitch", "Pitch", 180.0, 1400.0, 520.0).unit("Hz").log().smooth(0.015).pitch(),
            ParamDef::new("vowel", "Mouth vowel", 0.0, 1.0, 0.65).smooth(0.025),
            ParamDef::new("purr", "Meow ↔ purr", 0.0, 1.0, 0.0).smooth(0.03),
            ParamDef::new("purrRate", "Purr rate", 18.0, 38.0, 26.0).unit("Hz").smooth(0.03),
            ParamDef::new("hiss", "Palatal hiss", 0.0, 1.0, 0.0).smooth(0.02),
            ParamDef::new("pressure", "Breath pressure", 3.0, 24.0, 11.0).unit("cmH₂O").smooth(0.015),
            ParamDef::new("nasal", "Nasal murmur", 0.0, 1.0, 0.25).smooth(0.03),
        ],
        calls: vec![
            CallDef::new(
                "meow",
                "Meow",
                "Nasal onset opening into a bright feline arc and closing to [u]",
                Some(560.0),
                vec![
                    k(0.0, &[("gate", 0.0), ("purr", 0.0), ("hiss", 0.0), ("pitch", 440.0), ("vowel", 0.08), ("nasal", 0.85)]),
                    k(0.06, &[("gate", 1.0), ("pitch", 640.0), ("vowel", 0.88), ("nasal", 0.2)]).curve(Curve::Exp),
                    k(0.42, &[("gate", 1.0), ("pitch", 560.0), ("vowel", 0.72)]).curve(Curve::Exp),
                    k(0.78, &[("gate", 0.85), ("pitch", 390.0), ("vowel", 0.14), ("nasal", 0.45)]).curve(Curve::Exp),
                    k(0.9, &[("gate", 0.0), ("vowel", 0.65), ("nasal", 0.25)]),
                ],
            ),
            CallDef::new(
                "purr",
                "Purr",
                "Rhythmic 26 Hz laryngeal flutter across two gentle breaths",
                Some(520.0),
                vec![
                    k(0.0, &[("gate", 0.0), ("purr", 1.0), ("hiss", 0.0), ("vowel", 0.18), ("pressure", 13.0)]),
                    k(0.12, &[("gate", 1.0)]),
                    k(1.0, &[("gate", 0.55)]),
                    k(1.25, &[("gate", 1.0)]),
                    k(2.1, &[("gate", 0.0), ("purr", 0.0), ("pressure", 11.0), ("vowel", 0.65)]),
                ],
            ),
            CallDef::new(
                "chirrup",
                "Chirrup",
                "Friendly trilled greeting sweeping upward",
                Some(500.0),
                vec![
                    k(0.0, &[("gate", 1.0), ("purr", 0.55), ("purrRate", 32.0), ("pitch", 420.0), ("vowel", 0.35)]),
                    k(0.22, &[("purr", 0.15), ("pitch", 780.0), ("vowel", 0.82)]).curve(Curve::Exp),
                    k(0.38, &[("gate", 0.0), ("purr", 0.0), ("purrRate", 26.0)]),
                ],
            ),
            CallDef::new(
                "hiss",
                "Hiss",
                "Sharp unvoiced jet across the arched tongue and teeth",
                None,
                vec![
                    k(0.0, &[("gate", 0.0), ("hiss", 1.0), ("purr", 0.0), ("pressure", 18.0), ("vowel", 0.45)]),
                    k(0.04, &[("gate", 1.0)]),
                    k(0.68, &[("gate", 0.9), ("pressure", 14.0)]),
                    k(0.78, &[("gate", 0.0), ("hiss", 0.0), ("pressure", 11.0)]),
                ],
            ),
        ],
        events: vec![],
        create,
    }
    .resolved()
}
