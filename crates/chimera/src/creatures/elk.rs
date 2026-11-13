//! Rocky Mountain elk: true mammalian biphonia — low laryngeal vocal folds roar (~135 Hz) at the
//! same time a high aerodynamic palatal whistle (~2.1 kHz, `Whistle` van der Pol oscillator)
//! couples to local acoustic pressure and hops across vocal-tract resonances as the neck stretches.

use super::build::{CMH2O, fill_areas};
use super::mammal::{MammalAirway, MammalSpec, mammal_airway, mammal_noise, step_glottis};
use super::types::{CallDef, CallKey, CreatureDef, CreatureGroup, CreatureSim, Curve, GateTimes, ParamDef};
use crate::airway::network::{AirwayNetwork, PortSpec, Tube};
use crate::math::{js_max, js_min};
use crate::physics::{C_SOUND, RHO};
use crate::sources::valves::Whistle;

const WHISTLE_PITCH: usize = 0;
const ROAR_PITCH: usize = 1;
const WHISTLE_MIX: usize = 2;
const STRETCH: usize = 3;
const ROUGH: usize = 4;
const PRESSURE: usize = 5;
const MOUTH: usize = 6;

pub struct Elk {
    a: MammalAirway,
    tubes: Vec<(&'static str, Tube)>,
    whistle: Whistle,
    whistle_node: usize,
    velum: usize,
    drive: f64,
}

fn create(fs: f64) -> Box<dyn CreatureSim> {
    let mut whistle_node = 0usize;
    let mut a = mammal_airway(
        fs,
        MammalSpec { trachea: (40.0, 7.5), pharynx: (18.0, 6.5), oral: (24.0, 6.0), nasal: Some((20.0, 2.4)), wall: None, wall_radiation: Some(0.35), coarse: Some(1.2), nasal_radiates: None },
        |net, parts| {
            // Supraglottal palatal whistle pressure source coupled into the anterior oral/palatal junction
            whistle_node = net.add_pressure_node("whistle");
            let area = 1.4;
            net.add_port(whistle_node, parts.oral.first_node(), PortSpec::new(area, 0.5).kinetic(0.0, 0.0).resistance((1.4 * RHO * C_SOUND) / area));
        },
    );
    a.glottis_src.params.jitter = 0.008;
    a.glottis_src.params.shimmer = 0.04;
    let nasal = a.nasal.expect("elk has a nasal tube");
    let velum = a.velum.expect("elk has a velum");
    let tubes = vec![("trachea", a.trachea), ("pharynx", a.pharynx), ("oral", a.oral), ("nasal", nasal)];
    Box::new(Elk { a, tubes, whistle: Whistle::new(fs), whistle_node, velum, drive: 0.0 })
}

impl CreatureSim for Elk {
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
        1.6e-7
    }
    fn control(&mut self, p: &[f64], gate: f64, _dt: f64) {
        let a = &mut self.a;
        a.net.set_fixed_pressure(a.lungs, p[PRESSURE] * CMH2O * gate);

        let w_mix = p[WHISTLE_MIX];
        self.drive = gate * w_mix * js_min(1.0, p[PRESSURE] / 14.0);
        self.whistle.f = p[WHISTLE_PITCH];
        self.whistle.coupling = 0.65;

        let g = &mut a.glottis_src.params;
        g.f0 = p[ROAR_PITCH];
        g.amplitude = 0.85 * (1.0 - 0.35 * w_mix);
        g.abduction = 0.03 + 0.12 * w_mix;
        g.open_quotient = 0.52;
        g.closure_sharpness = 0.72;
        g.effort = 0.9;
        g.chaos = p[ROUGH];

        let stretch = p[STRETCH];
        let ph_len = 15.0 + 7.5 * stretch;
        let ph_areas = fill_areas(a.pharynx, |_| 6.2, &mut a.phar_area);
        a.net.set_tube_targets(a.pharynx, ph_areas, Some(ph_len), None);

        let mouth = p[MOUTH];
        let or_areas = fill_areas(
            a.oral,
            |u| {
                if u > 0.25 && u < 0.45 {
                    // palatal arch narrowing that excites the aerodynamic whistle
                    js_max(0.45, 3.2 - 2.2 * w_mix)
                } else if u > 0.85 {
                    0.35 + 7.5 * mouth
                } else {
                    5.8
                }
            },
            &mut a.oral_area,
        );
        a.net.set_tube_targets(a.oral, or_areas, Some(22.0 + 5.0 * stretch), None);
        a.net.set_port_target(self.velum, 0.35);
        mammal_noise(a, 3.5e-6, Some(8.0e-6 * w_mix));
    }
    #[inline]
    fn sample(&mut self) -> f64 {
        step_glottis(&mut self.a);
        let p_local = self.a.net.p[self.a.oral.first_node()];
        let pw = self.whistle.next(self.drive, p_local, 3800.0);
        self.a.net.set_fixed_pressure(self.whistle_node, pw);
        self.a.net.step()
    }
    fn source_state(&self, out: &mut Vec<(&'static str, f64)>) {
        out.clear();
        out.push(("glottis", js_min(1.0, self.a.glottis_src.area / 0.85)));
        out.push(("whistle", js_min(1.0, self.whistle.out.abs() / 3200.0)));
    }
}

pub fn def() -> CreatureDef {
    let k = CallKey::new;
    CreatureDef {
        id: "elk",
        name: "Rocky Mountain elk",
        latin: "Cervus canadensis",
        group: CreatureGroup::Mammal,
        blurb: "Biphonic bugle: a low laryngeal roar sounds simultaneously with a 2 kHz palatal whistle that locks onto tract resonances.",
        role: "Haunting biphonic horn leads, wild overtone swells, rhythmic grunts",
        oversample: 2,
        gate_times: Some(GateTimes { attack: 0.015, release: 0.055 }),
        params: vec![
            ParamDef::new("whistlePitch", "Whistle pitch", 900.0, 3400.0, 2050.0).unit("Hz").log().smooth(0.025).pitch(),
            ParamDef::new("roarPitch", "Laryngeal roar", 65.0, 320.0, 138.0).unit("Hz").log().smooth(0.03),
            ParamDef::new("whistleMix", "Roar ↔ whistle", 0.0, 1.0, 0.82).smooth(0.03),
            ParamDef::new("stretch", "Neck extension", 0.0, 1.0, 0.6).smooth(0.05),
            ParamDef::new("rough", "Roar roughness", 0.0, 1.0, 0.35).smooth(0.04),
            ParamDef::new("pressure", "Breath pressure", 4.0, 38.0, 20.0).unit("cmH₂O").smooth(0.02),
            ParamDef::new("mouth", "Mouth flare", 0.0, 1.0, 0.65).smooth(0.03),
        ],
        calls: vec![
            CallDef::new(
                "bugle",
                "Bugle & grunts",
                "Rising biphonic whistle and roar, followed by two staccato chest grunts",
                Some(2050.0),
                vec![
                    k(0.0, &[("gate", 0.0), ("whistleMix", 0.4), ("whistlePitch", 1250.0), ("roarPitch", 115.0), ("stretch", 0.2), ("rough", 0.3)]),
                    k(0.08, &[("gate", 1.0)]),
                    k(0.55, &[("whistleMix", 0.92), ("whistlePitch", 2350.0), ("roarPitch", 155.0), ("stretch", 0.9)]).curve(Curve::Exp),
                    k(1.35, &[("gate", 1.0), ("whistlePitch", 2150.0)]),
                    k(1.62, &[("whistleMix", 0.25), ("whistlePitch", 1150.0), ("roarPitch", 98.0)]).curve(Curve::Exp),
                    k(1.7, &[("gate", 0.0)]),
                    // two rhythmic rutting grunts ("yelps")
                    k(1.82, &[("gate", 1.0), ("whistleMix", 0.0), ("roarPitch", 110.0), ("rough", 0.65)]),
                    k(1.98, &[("gate", 0.0)]),
                    k(2.1, &[("gate", 1.0), ("roarPitch", 96.0)]),
                    k(2.3, &[("gate", 0.0), ("whistleMix", 0.82), ("rough", 0.35)]),
                ],
            ),
            CallDef::new(
                "bark",
                "Alarm bark",
                "Explosive laryngeal warning bark",
                Some(180.0),
                vec![
                    k(0.0, &[("gate", 1.0), ("whistleMix", 0.0), ("roarPitch", 210.0), ("rough", 0.55), ("pressure", 28.0), ("mouth", 0.9)]),
                    k(0.22, &[("roarPitch", 125.0)]).curve(Curve::Exp),
                    k(0.28, &[("gate", 0.0), ("whistleMix", 0.82), ("pressure", 20.0)]),
                ],
            ),
            CallDef::new(
                "mew",
                "Cow mew",
                "High nasal contact glide",
                Some(1500.0),
                vec![
                    k(0.0, &[("gate", 1.0), ("whistleMix", 0.7), ("whistlePitch", 1680.0), ("roarPitch", 260.0), ("mouth", 0.3)]),
                    k(0.65, &[("whistlePitch", 1180.0), ("roarPitch", 190.0)]).curve(Curve::Exp),
                    k(0.75, &[("gate", 0.0), ("mouth", 0.65)]),
                ],
            ),
        ],
        events: vec![],
        create,
    }
    .resolved()
}
