//! Sperm whale: pneumatic phonic lips (`ClickTrain`) fire high-energy clicks into the 1.4 m
//! spermaceti oil sac; part enters the junk acoustic lens and part reflects off the frontal air
//! sac, producing the signature multi-pulse click echoes, echolocation creaks and social codas.

use super::build::{TubeArea, TubeOpts, fill_areas, tube};
use super::types::{CallDef, CallKey, CreatureDef, CreatureGroup, CreatureSim, Curve, GateTimes, ParamDef};
use crate::airway::network::{AirwayNetwork, PortSpec, Tube};
use crate::math::{js_max, js_min};
use crate::physics::{C_SOUND, RHO, WALL_NASAL, WALL_TRACHEA};
use crate::sources::valves::ClickTrain;

const CLICK_RATE: usize = 0;
const RING: usize = 1;
const RING_DECAY: usize = 2;
const SAC_LEN: usize = 3;
const JUNK_COUPLE: usize = 4;
const DRIVE: usize = 5;

const EV_CLICK: usize = 0;

pub struct SpermWhale {
    net: AirwayNetwork,
    tubes: Vec<(&'static str, Tube)>,
    spermaceti: Tube,
    junk: Tube,
    sperm_area: Vec<f64>,
    phonic_node: usize,
    junk_port: usize,
    clicks: ClickTrain,
    pending_est: usize,
    rate: f64,
    gate: f64,
    drive: f64,
    auto_clicks: bool,
    phase: f64,
    last_pulse: f64,
}

fn create(fs: f64) -> Box<dyn CreatureSim> {
    let mut net = AirwayNetwork::new(fs);
    // Spermaceti organ: long oil cylinder from distal phonic lips to proximal frontal air sac (closed hard reflector)
    let spermaceti = tube(
        &mut net,
        "spermaceti",
        fs,
        135.0,
        TubeArea::Profile(&|u| 18.0 - 4.0 * u),
        TubeOpts { wall: Some(WALL_TRACHEA), coarse: Some(2.1), max: Some(90.0), viscous_factor: Some(0.6), ..Default::default() },
    );
    // Junk: segmented oil lens below the spermaceti that focuses and radiates the click into seawater
    let junk = tube(
        &mut net,
        "junk",
        fs,
        92.0,
        TubeArea::Profile(&|u| 14.0 + 8.0 * u),
        TubeOpts { wall: Some(WALL_NASAL), coarse: Some(2.0), max: Some(70.0), wall_radiation: Some(0.45), ..Default::default() },
    );
    let phonic_node = net.add_pressure_node("phonic");
    let ph_area = 6.0;
    net.add_port(phonic_node, spermaceti.first_node(), PortSpec::new(ph_area, 0.8).kinetic(0.0, 0.0).resistance((0.9 * RHO * C_SOUND) / ph_area));
    // Distal coupling between spermaceti and junk (leaves partial energy to bounce back from spermaceti.last_node())
    let junk_port = net.add_port(spermaceti.first_node(), junk.first_node(), PortSpec::new(7.5, 1.0).kinetic(0.2, 0.5));
    net.add_radiation(junk.last_node());
    net.finalize();

    let mut clicks = ClickTrain::new(fs);
    clicks.width = 0.00045;
    let sperm_n = spermaceti.n;
    let tubes = vec![("spermaceti", spermaceti), ("junk", junk)];
    Box::new(SpermWhale {
        net,
        tubes,
        spermaceti,
        junk,
        sperm_area: vec![0.0; sperm_n],
        phonic_node,
        junk_port,
        clicks,
        pending_est: 0,
        rate: 12.0,
        gate: 0.0,
        drive: 1.0,
        auto_clicks: true,
        phase: 0.99,
        last_pulse: 0.0,
    })
}

impl CreatureSim for SpermWhale {
    fn net(&self) -> Option<&AirwayNetwork> {
        Some(&self.net)
    }
    fn net_mut(&mut self) -> Option<&mut AirwayNetwork> {
        Some(&mut self.net)
    }
    fn tubes(&self) -> &[(&'static str, Tube)] {
        &self.tubes
    }
    fn gain(&self) -> f64 {
        1.35e-7
    }
    fn control(&mut self, p: &[f64], gate: f64, _dt: f64) {
        self.rate = p[CLICK_RATE];
        self.gate = gate;
        self.drive = p[DRIVE];
        self.auto_clicks = self.rate > 0.6;
        self.clicks.ring = p[RING];
        self.clicks.ring_decay = p[RING_DECAY];

        let sac_len = 85.0 + 85.0 * p[SAC_LEN];
        let areas = fill_areas(self.spermaceti, |u| 18.0 - 4.0 * u, &mut self.sperm_area);
        self.net.set_tube_targets(self.spermaceti, areas, Some(sac_len), None);
        self.net.set_port_target(self.junk_port, 2.5 + 10.0 * p[JUNK_COUPLE]);
        let _ = self.junk;
    }
    fn trigger(&mut self, event: usize, amount: f64) {
        if event == EV_CLICK && self.pending_est < 32 {
            self.clicks.click(0.0, js_max(0.1, amount));
            self.pending_est += 1;
        }
    }
    #[inline]
    fn sample(&mut self) -> f64 {
        if self.gate > 0.05 && self.auto_clicks {
            self.phase += self.rate / self.net.fs;
            if self.phase >= 1.0 {
                self.phase -= self.phase.floor();
                if self.pending_est < 32 {
                    self.clicks.click(0.0, self.gate);
                    self.pending_est += 1;
                }
            }
        } else if self.gate <= 0.05 {
            self.phase = 0.99;
        }
        let y = self.clicks.next();
        if y.abs() < 1e-6 && self.pending_est > 0 {
            self.pending_est = 0;
        }
        self.last_pulse = y.abs();
        self.net.set_fixed_pressure(self.phonic_node, y * 24000.0 * self.drive);
        self.net.step()
    }
    fn source_state(&self, out: &mut Vec<(&'static str, f64)>) {
        out.clear();
        out.push(("phonic", js_min(1.0, self.last_pulse)));
    }
}

pub fn def() -> CreatureDef {
    let k = CallKey::new;
    CreatureDef {
        id: "whale",
        name: "Sperm whale",
        latin: "Physeter macrocephalus",
        group: CreatureGroup::Marine,
        blurb: "Pneumatic phonic lips fire clicks through the spermaceti sac and junk lens, echoing off the frontal air sac into multi-pulse codas.",
        role: "Sonar percussion, accelerating buzz risers, metallic sub-clangs",
        oversample: 1,
        gate_times: Some(GateTimes { attack: 0.004, release: 0.04 }),
        params: vec![
            ParamDef::new("clickRate", "Click rate", 0.5, 160.0, 14.0).unit("Hz").log().smooth(0.015).pitch(),
            ParamDef::new("ring", "Clang resonance", 0.0, 2600.0, 1150.0).unit("Hz").smooth(0.02),
            ParamDef::new("ringDecay", "Click ring decay", 0.0008, 0.012, 0.0022).unit("s").log().digits(4).smooth(0.02),
            ParamDef::new("sacLen", "Spermaceti length", 0.0, 1.0, 0.65).smooth(0.04),
            ParamDef::new("junk", "Junk lens coupling", 0.0, 1.0, 0.6).smooth(0.03),
            ParamDef::new("drive", "Phonic pressure", 0.2, 2.0, 1.1).smooth(0.02),
        ],
        calls: vec![
            CallDef::new(
                "coda",
                "5R1 coda",
                "Five-click social coda (1 + 1 + 3 rhythm) echoing in the spermaceti organ",
                None,
                vec![
                    k(0.0, &[("gate", 1.0), ("clickRate", 0.5), ("ring", 1250.0), ("ringDecay", 0.0025), ("!click", 1.0)]),
                    k(0.28, &[("!click", 0.95)]),
                    k(0.72, &[("!click", 1.0)]),
                    k(0.88, &[("!click", 0.9)]),
                    k(1.04, &[("!click", 0.85)]),
                    k(1.35, &[("gate", 0.0), ("clickRate", 14.0)]),
                ],
            ),
            CallDef::new(
                "creak",
                "Foraging creak",
                "Echolocation clicks accelerating into a 110 Hz terminal prey buzz",
                Some(24.0),
                vec![
                    k(0.0, &[("gate", 1.0), ("clickRate", 7.0), ("ring", 1400.0), ("ringDecay", 0.0014)]),
                    k(1.1, &[("clickRate", 38.0)]).curve(Curve::Exp),
                    k(1.85, &[("clickRate", 115.0), ("drive", 0.85)]).curve(Curve::Exp),
                    k(2.15, &[("gate", 0.0), ("clickRate", 14.0), ("drive", 1.1)]),
                ],
            ),
            CallDef::new(
                "clang",
                "Slow clang",
                "Booming, resonant metallic clicks of a mature bull",
                Some(650.0),
                vec![
                    k(0.0, &[("gate", 1.0), ("clickRate", 0.5), ("ring", 650.0), ("ringDecay", 0.01), ("sacLen", 1.0), ("drive", 1.6), ("!click", 1.2)]),
                    k(0.65, &[("!click", 1.15)]),
                    k(1.3, &[("gate", 0.0), ("clickRate", 14.0), ("ring", 1150.0), ("ringDecay", 0.0022), ("sacLen", 0.65), ("drive", 1.1)]),
                ],
            ),
        ],
        events: vec!["click"],
        create,
    }
    .resolved()
}
