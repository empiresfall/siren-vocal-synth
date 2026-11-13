//! American bullfrog: heavy, low-pitched vocal cords pump air into an inflatable throat vocal sac
//! while the mouth and nostrils stay nearly sealed, so the booming "jug-o-rum" radiates straight
//! through the pulsing skin of the sac.

use super::build::{CMH2O, TubeArea, TubeOpts, fill_areas, tube};
use super::mammal::{MammalAirway, MammalSpec, mammal_airway, mammal_noise, step_glottis};
use super::types::{CallDef, CallKey, CreatureDef, CreatureGroup, CreatureSim, Curve, GateTimes, ParamDef};
use crate::airway::network::{AirwayNetwork, PortSpec, Tube};
use crate::math::{PI, js_min, sin};
use crate::physics::WALL_SOFT;

const PITCH: usize = 0;
const SAC: usize = 1;
const PULSE_RATE: usize = 2;
const PULSE: usize = 3;
const ROUGH: usize = 4;
const PRESSURE: usize = 5;
const MOUTH: usize = 6;

pub struct Bullfrog {
    a: MammalAirway,
    tubes: Vec<(&'static str, Tube)>,
    sac: Tube,
    sac_area: Vec<f64>,
    sac_port: usize,
    pulse_phase: f64,
}

fn create(fs: f64) -> Box<dyn CreatureSim> {
    let mut sac_tube: Option<Tube> = None;
    let mut sac_port = 0usize;
    let mut a = mammal_airway(
        fs,
        MammalSpec { trachea: (6.5, 2.8), pharynx: (5.0, 3.6), oral: (6.5, 3.8), nasal: None, wall: None, wall_radiation: Some(0.55), coarse: None, nasal_radiates: None },
        |net, parts| {
            // Inflatable subgular vocal sac branching off the pharynx, radiating strongly through its thin skin
            let s = tube(
                net,
                "sac",
                fs,
                15.0,
                TubeArea::Profile(&|u| 2.4 + 7.5 * sin(PI * u)),
                TubeOpts { wall: Some(WALL_SOFT), wall_radiation: Some(0.95), perimeter_factor: Some(1.8), ..Default::default() },
            );
            sac_port = net.add_port(parts.pharynx.last_node(), s.first_node(), PortSpec::new(2.5, 0.5).kinetic(0.25, 0.7));
            sac_tube = Some(s);
        },
    );
    let sac = sac_tube.expect("bullfrog has a vocal sac");
    a.glottis_src.params.jitter = 0.006;
    a.glottis_src.params.shimmer = 0.04;
    let sac_n = sac.n;
    let tubes = vec![("trachea", a.trachea), ("pharynx", a.pharynx), ("oral", a.oral), ("sac", sac)];
    Box::new(Bullfrog { a, tubes, sac, sac_area: vec![0.0; sac_n], sac_port, pulse_phase: 0.0 })
}

impl CreatureSim for Bullfrog {
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
        2.1e-7
    }
    fn control(&mut self, p: &[f64], gate: f64, dt: f64) {
        self.pulse_phase += p[PULSE_RATE] * dt;
        self.pulse_phase -= self.pulse_phase.floor();
        let ph = 0.5 + 0.5 * sin(2.0 * PI * self.pulse_phase);
        let env = 1.0 - p[PULSE] + p[PULSE] * ph;

        let a = &mut self.a;
        a.net.set_fixed_pressure(a.lungs, p[PRESSURE] * CMH2O * gate * env);

        let g = &mut a.glottis_src.params;
        g.f0 = p[PITCH];
        g.amplitude = 0.55;
        g.abduction = 0.02;
        g.open_quotient = 0.48;
        g.closure_sharpness = 0.78;
        g.effort = 0.9;
        g.chaos = p[ROUGH];

        let inflate = p[SAC];
        a.net.set_port_target(self.sac_port, 0.3 + 4.2 * inflate);
        let sac_areas = fill_areas(self.sac, |u| (1.6 + 9.5 * inflate) * (0.35 + 0.65 * sin(PI * u)), &mut self.sac_area);
        a.net.set_tube_targets(self.sac, sac_areas, None, None);

        let mouth = p[MOUTH];
        let ph_areas = fill_areas(a.pharynx, |_| 3.6 + 1.4 * inflate, &mut a.phar_area);
        a.net.set_tube_targets(a.pharynx, ph_areas, None, None);
        let or_areas = fill_areas(a.oral, |u| if u > 0.82 { 0.06 + 5.0 * mouth } else { 3.8 }, &mut a.oral_area);
        a.net.set_tube_targets(a.oral, or_areas, None, None);
        mammal_noise(a, 1.8e-6, None);
    }
    #[inline]
    fn sample(&mut self) -> f64 {
        step_glottis(&mut self.a);
        self.a.net.step()
    }
    fn source_state(&self, out: &mut Vec<(&'static str, f64)>) {
        out.clear();
        out.push(("glottis", js_min(1.0, self.a.glottis_src.area / 0.55)));
    }
}

pub fn def() -> CreatureDef {
    let k = CallKey::new;
    CreatureDef {
        id: "bullfrog",
        name: "American bullfrog",
        latin: "Lithobates catesbeianus",
        group: CreatureGroup::Amphibian,
        blurb: "Heavy amphibian vocal cords coupled to an inflatable throat sac whose stretched skin radiates a booming bass drone.",
        role: "Resonant bass stabs, guttural rhythmic pulses, swampy drones",
        oversample: 2,
        gate_times: Some(GateTimes { attack: 0.012, release: 0.05 }),
        params: vec![
            ParamDef::new("pitch", "Pitch", 45.0, 420.0, 96.0).unit("Hz").log().smooth(0.02).pitch(),
            ParamDef::new("sac", "Vocal sac inflation", 0.0, 1.0, 0.85).smooth(0.04),
            ParamDef::new("pulseRate", "Croak pulse rate", 3.0, 35.0, 14.0).unit("Hz").log().smooth(0.02),
            ParamDef::new("pulse", "Pulse depth", 0.0, 1.0, 0.25).smooth(0.02),
            ParamDef::new("rough", "Roughness", 0.0, 0.9, 0.28).smooth(0.04),
            ParamDef::new("pressure", "Breath pressure", 3.0, 32.0, 16.0).unit("cmH₂O").smooth(0.02),
            ParamDef::new("mouth", "Mouth seal ↔ open", 0.0, 1.0, 0.08).smooth(0.03),
        ],
        calls: vec![
            CallDef::new(
                "jugorum",
                "Jug-o-rum",
                "Three booming pond syllables swelling the throat sac",
                Some(96.0),
                vec![
                    k(0.0, &[("gate", 1.0), ("pitch", 104.0), ("sac", 0.7), ("pulse", 0.15), ("mouth", 0.06)]),
                    k(0.24, &[("gate", 0.08)]),
                    k(0.32, &[("gate", 1.0), ("pitch", 98.0), ("sac", 0.85)]),
                    k(0.54, &[("gate", 0.08)]),
                    k(0.64, &[("gate", 1.0), ("pitch", 88.0), ("sac", 1.0), ("rough", 0.38)]),
                    k(1.38, &[("gate", 0.9), ("pitch", 82.0)]),
                    k(1.5, &[("gate", 0.0), ("rough", 0.28)]),
                ],
            ),
            CallDef::new(
                "croak",
                "Pond croak",
                "Single guttural pulsed croak",
                Some(105.0),
                vec![
                    k(0.0, &[("gate", 1.0), ("pitch", 112.0), ("pulse", 0.75), ("pulseRate", 19.0), ("sac", 0.8)]),
                    k(0.52, &[("pitch", 92.0)]).curve(Curve::Exp),
                    k(0.6, &[("gate", 0.0), ("pulse", 0.25)]),
                ],
            ),
            CallDef::new(
                "trill",
                "Chorus trill",
                "Sustained 22 Hz pulsed evening call",
                Some(115.0),
                vec![
                    k(0.0, &[("gate", 1.0), ("pitch", 118.0), ("pulse", 0.9), ("pulseRate", 22.0), ("sac", 0.9)]),
                    k(1.2, &[("gate", 1.0), ("pitch", 108.0)]),
                    k(1.3, &[("gate", 0.0), ("pulse", 0.25)]),
                ],
            ),
            CallDef::new(
                "chirp",
                "Jump squeak",
                "Open-mouthed startle squeak before hitting the water",
                Some(310.0),
                vec![
                    k(0.0, &[("gate", 1.0), ("pitch", 340.0), ("mouth", 0.75), ("sac", 0.2), ("pulse", 0.0)]),
                    k(0.18, &[("pitch", 220.0)]).curve(Curve::Exp),
                    k(0.22, &[("gate", 0.0), ("mouth", 0.08), ("sac", 0.85)]),
                ],
            ),
        ],
        events: vec![],
        create,
    }
    .resolved()
}
