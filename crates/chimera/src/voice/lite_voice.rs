//! The light voices: the same head, articulators, gestures and fold source as [`HumanVoice`], with
//! a cheaper model of the air above the folds.
//!
//! - [`LiteKind::Waveguide`]: a Kelly–Lochbaum travelling-wave tube (the "pink trombone" family)
//!   with a nasal branch. Unlike the classic form it keeps the mean flow: the folds are a
//!   Bernoulli orifice solved against the tube, each junction has the Borda–Carnot expansion
//!   loss, and turbulence comes from the Reynolds number at the constriction, so stops build
//!   pressure and burst, and fricatives hiss, without samples or special cases.
//! - [`LiteKind::Formant`]: a cascade of resonators (the Klatt/DECtalk family) whose formants are
//!   read off the head's own area function every control step, driven by the fold source through
//!   a two-orifice model of the breath (folds, constriction) that gates voicing and makes the
//!   bursts and frication.
//!
//! Both take the geometry from a [`HumanAirway`] whose network never steps: it turns the
//! articulators into slew-limited area functions exactly as the airway voice does, so the three
//! engines share one mouth and one telemetry layout.

use super::human_airway::HumanAirway;
use super::neural::NeuralSynth;
use super::human_voice::{DEFAULT_SOURCE, HARMONY_GAIN, SCOPE_POINTS, SourcePatch, VoiceSource, VoiceTelemetry, oversample_for, source_to_glottis};
use crate::airway::decimator::Decimator;
use crate::anatomy::geom::Vec2;
use crate::anatomy::human::{ArticKey, ArticPatch, HumanBody, N_ARTIC};
use crate::control::articulators::ArticulatorBank;
use crate::control::score::{Blended, ControlKey, Gesture, GestureScore, N_CONTROL};
use crate::math::{PI, Rng, clamp, cos, exp, js_max, js_min, js_round, js_sign, js_trunc_i32, pow, sin, smoothstep as smooth, sqrt, tanh};
use crate::physics::{CM_H2O, C_SOUND, NU, RE_CRIT, RHO};
use crate::sources::glottis::ParametricGlottis;
use crate::sources::harmony::Harmony;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiteKind {
    Waveguide,
    Formant,
    Neural,
}

const SCOPE_LEN: usize = 4096;
/// ρ·c: a tube's wave impedance is this over its area.
pub(super) const RC: f64 = RHO * C_SOUND;
/// An opening narrower than this (cm²) is shut: no flow. Viscosity has throttled the flow to
/// nothing well before (it falls with the area squared), so the last step is inaudible.
pub(super) const CLOSED: f64 = 2e-5;
/// Kinetic loss of the glottal jet (entry 0.37 + exit 1.0, as the airway's glottal port).
pub(super) const K_GLOTTIS: f64 = 1.37;
/// Borda–Carnot expansion loss coefficient.
const K_EXPAND: f64 = 1.0;

// ====================================================================== turbulence noise

/// White noise shaped to a band: first-order high-pass, second-order low-pass (as the airway's).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct NoiseBand {
    hp: f64,
    lp1: f64,
    lp2: f64,
    lp_coef: f64,
    hp_coef: f64,
}

impl NoiseBand {
    pub(super) fn new(fs: f64, lowpass_hz: f64, highpass_hz: f64) -> Self {
        NoiseBand { hp: 0.0, lp1: 0.0, lp2: 0.0, lp_coef: 1.0 - exp((-2.0 * PI * js_min(lowpass_hz, 0.45 * fs)) / fs), hp_coef: exp((-2.0 * PI * highpass_hz) / fs) }
    }
    #[inline]
    pub(super) fn tick(&mut self, rng: &mut Rng) -> f64 {
        let w = rng.bipolar() * 1.732;
        self.hp += (1.0 - self.hp_coef) * (w - self.hp);
        let hp_out = w - self.hp;
        self.lp1 += self.lp_coef * (hp_out - self.lp1);
        self.lp2 += self.lp_coef * (self.lp1 - self.lp2);
        self.lp2
    }
}

/// Turbulence amplitude at Reynolds number `re` (as the airway's: grows with Re² past the
/// critical value, bounded at very high Re).
#[inline]
pub(super) fn turbulence(re: f64) -> f64 {
    if re <= RE_CRIT {
        return 0.0;
    }
    let sat = 12000.0 * 12000.0 - RE_CRIT * RE_CRIT;
    let x = re * re - RE_CRIT * RE_CRIT;
    x / (1.0 + x / sat)
}

/// Reynolds number of flow `u` (cm³/s) through an opening of area `a` (cm²).
#[inline]
pub(super) fn reynolds(u: f64, a: f64) -> f64 {
    (u.abs() * 2.0) / (NU * sqrt(PI * a))
}

// ====================================================================== the waveguide

const MAX_SEC: usize = 128;
const MAX_NOSE: usize = 64;

/// Where the mouth's turbulence is this control step (section indices of the waveguide).
#[derive(Clone, Copy, Debug, Default)]
struct WgNoise {
    /// junction whose flow, and section whose area, drive the source
    src: usize,
    /// junction the noise pressure enters when the flow is toward the lips / toward the folds
    fwd: usize,
    bwd: usize,
    gain: f64,
}

#[derive(Clone, Debug)]
struct Waveguide {
    fs: f64,
    dt: f64,
    /// section length (cm): sound travels one section per step
    dx: f64,
    scale: f64,
    n: usize,
    /// right- and left-going pressure waves per section, and the next step's
    r: Vec<f64>,
    l: Vec<f64>,
    r2: Vec<f64>,
    l2: Vec<f64>,
    area: Vec<f64>,
    area_step: Vec<f64>,
    amin: Vec<f64>,
    amin_step: Vec<f64>,
    ramp_left: usize,
    /// flow through each junction (j: between sections j and j+1), last step
    flow: Vec<f64>,
    psrc: Vec<f64>,
    /// per-section decay per step (wall losses, more in a narrow section)
    dj: Vec<f64>,
    /// Poiseuille resistance of half a section over its area squared
    visc: f64,
    /// first oral section: the nose branches off at the junction before it
    jv: usize,
    // nose
    nn: usize,
    nr: Vec<f64>,
    nl: Vec<f64>,
    nr2: Vec<f64>,
    nl2: Vec<f64>,
    nose_profile: Vec<f64>,
    port: f64,
    port_step: f64,
    nose_rad: (f64, f64, f64),
    // radiation inductor flows and the last radiated flow
    ul_lip: f64,
    ul_nose: f64,
    u_prev: f64,
    damp: f64,
    damp_nose: f64,
    /// The tract's yielding walls, lumped at the velopharyngeal junction: flow into them, the
    /// volume they have given, and the coefficients of their mass–resistance–stiffness update.
    /// They lift the lowest formant (a rigid tube's sits ~15 % low) and take up the breath behind
    /// a closure, so a voiced stop keeps its voice bar.
    wall_i: f64,
    wall_q: f64,
    wall: (f64, f64, f64),
    /// tube steps per tick: the tube has a fixed number of sections, so it runs faster when the
    /// tract is shorter than its resting length (and slower when longer)
    pub rate: f64,
    phase: f64,
    y_prev: f64,
    y_cur: f64,
    // noise
    rng: Rng,
    asp_band: NoiseBand,
    con_band: NoiseBand,
    teeth_band: NoiseBand,
    asp_gain: f64,
    con: WgNoise,
    teeth: WgNoise,
    noise_scale: f64,
    /// telemetry: pressure at the folds' upper side, glottal flow, strongest noise and where
    pub p0: f64,
    pub ug: f64,
    pub noise_level: f64,
    pub noise_at: i32,
}

impl Waveguide {
    fn new(fs: f64, body: HumanBody) -> Self {
        let k = body.scale;
        let dx = C_SOUND / fs;
        let na_len = 11.4 * k;
        let nn = (js_round(na_len / dx) as usize).clamp(4, MAX_NOSE);
        // the airway's nasal profile
        let nose_profile: Vec<f64> = (0..nn)
            .map(|i| {
                let u = (i as f64 + 0.5) / nn as f64;
                let a = if u < 0.18 {
                    2.6
                } else if u < 0.75 {
                    2.0 - 0.6 * (u - 0.18)
                } else if u < 0.9 {
                    1.4
                } else {
                    1.0
                };
                a * k * k
            })
            .collect();
        let dt = 1.0 / fs;
        let a_n = 1.0 * k * k;
        let rr = (128.0 * RC) / (9.0 * PI * PI * a_n);
        let lr = (8.0 * RHO) / (3.0 * PI * sqrt(PI * a_n));
        let z = vec![0.0; MAX_SEC];
        Waveguide {
            fs,
            dt,
            dx,
            scale: k,
            n: 0,
            r: z.clone(),
            l: z.clone(),
            r2: z.clone(),
            l2: z.clone(),
            area: z.clone(),
            area_step: z.clone(),
            amin: z.clone(),
            amin_step: z.clone(),
            ramp_left: 0,
            flow: z.clone(),
            psrc: z.clone(),
            dj: z,
            visc: 8.0 * PI * crate::physics::MU * 0.5 * dx,
            jv: 2,
            nn,
            nr: vec![0.0; MAX_NOSE],
            nl: vec![0.0; MAX_NOSE],
            nr2: vec![0.0; MAX_NOSE],
            nl2: vec![0.0; MAX_NOSE],
            nose_profile,
            port: 0.0,
            port_step: 0.0,
            nose_rad: (a_n / RC, 1.0 / rr + dt / lr, dt / lr),
            ul_lip: 0.0,
            ul_nose: 0.0,
            u_prev: 0.0,
            // wall and viscous losses as a uniform decay: about 30 Hz of bandwidth, more in the nose
            damp: exp(-PI * 30.0 / fs),
            damp_nose: exp(-PI * 110.0 / fs),
            wall_i: 0.0,
            wall_q: 0.0,
            wall: {
                // acoustic mass, resistance and compliance of ~135 cm² of soft wall (body-scaled)
                let (lw, rw, cw) = (0.02 / k, 12.0 / (k * k), 4.5e-4 * k * k);
                let d = lw / dt + rw + dt / cw;
                ((lw / dt) / d, (1.0 / cw) / d, 1.0 / d)
            },
            rate: 1.0,
            phase: 0.0,
            y_prev: 0.0,
            y_cur: 0.0,
            rng: Rng::new(0x51ed_270b),
            asp_band: NoiseBand::new(fs, 3500.0, 300.0),
            con_band: NoiseBand::new(fs, 2600.0, 350.0),
            teeth_band: NoiseBand::new(fs, 5000.0, 1200.0),
            asp_gain: 3.2e-6,
            con: WgNoise::default(),
            teeth: WgNoise::default(),
            noise_scale: sqrt(96000.0 / fs),
            p0: 0.0,
            ug: 0.0,
            noise_level: 0.0,
            noise_at: -1,
        }
    }

    /// The area function for the next `steps` steps, from the airway's sections.
    fn set_targets(&mut self, aw: &HumanAirway, velum_port: f64, teeth_s: f64, steps: usize) {
        let (n_ph, n_or) = (aw.n_ph, aw.n_or);
        let (ph_len, or_len) = (aw.ph_len, aw.or_len);
        let len = ph_len + or_len;
        // The section count is fixed at the first (resting) length. The tract's length changes a
        // few percent as the lips and larynx move, and following it by adding and dropping sections
        // made every constriction jump a section at each change: a click under a fricative's steady
        // flow. The price is formants up to ~4 % off at the extremes of protrusion.
        let first = self.n == 0;
        if first {
            self.n = (js_round(len / self.dx) as usize).clamp(8, MAX_SEC);
        }
        let n = self.n;
        let inv = 1.0 / steps as f64;
        for j in 0..n {
            let s = (j as f64 + 0.5) * len / n as f64;
            let (a, m) = if s < ph_len {
                let i = ((s / ph_len * n_ph as f64) as usize).min(n_ph - 1);
                (aw.ph_areas[i], aw.ph_min[i])
            } else {
                let i = (((s - ph_len) / or_len * n_or as f64) as usize).min(n_or - 1);
                (aw.or_areas[i], aw.or_min[i])
            };
            let m = js_min(a, m);
            if first {
                self.area[j] = a;
                self.amin[j] = m;
            }
            self.area_step[j] = (a - self.area[j]) * inv;
            self.amin_step[j] = (m - self.amin[j]) * inv;
            self.dj[j] = self.damp;
        }
        self.port_step = (velum_port - self.port) * inv;
        self.ramp_left = steps;
        self.rate = clamp((n as f64 * self.dx) / len, 0.8, 1.25);
        if first {
            // the nose joins at a fixed section: moving the junction with the breath flowing
            // through the nose stepped that flow from one section to the next (a click)
            self.jv = (js_round(ph_len / len * n as f64) as usize).clamp(2, n - 2);
        }

        // turbulence: the narrowest open place, and the teeth in the way of a jet (as the airway's)
        let k = self.scale;
        let k2 = k * k;
        let mut min_a = f64::INFINITY;
        let mut min_j = usize::MAX;
        for j in 0..n {
            let a = self.amin[j] + self.amin_step[j] * steps as f64;
            if a > 0.0 && a < min_a {
                min_a = a;
                min_j = j;
            }
        }
        self.con.gain = 0.0;
        self.teeth.gain = 0.0;
        let w_area = if min_j != usize::MAX { 1.0 - smooth(0.35 * k2, 0.9 * k2, min_a) } else { 0.0 };
        if w_area > 0.0 {
            let off = js_max(1.0, js_round((0.9 * k) / self.dx)) as usize;
            let last = n - 2;
            let src = min_j.min(last);
            let fwd = (min_j + off).min(last);
            let bwd = min_j.saturating_sub(off + 1);
            self.con = WgNoise { src, fwd, bwd, gain: 2.2e-6 * w_area };
            let s_min = (min_j as f64 + 0.5) * len / n as f64;
            let behind = (teeth_s - s_min) / k;
            let w_dist = smooth(0.15, 0.5, behind) * (1.0 - smooth(2.6, 4.0, behind));
            let w_jet = 1.0 - smooth(0.2 * k2, 0.5 * k2, min_a);
            let wt = w_dist * w_jet;
            if wt > 1e-3 {
                let tj = ((teeth_s / len * n as f64) as usize).clamp(1, last);
                self.teeth = WgNoise { src, fwd: tj, bwd, gain: 3.0e-6 * wt };
            }
        }
    }

    /// One tick of the voice's clock: the tube takes `rate` steps on average, and its output is
    /// read between its last two.
    #[inline]
    fn tick(&mut self, pl: f64, ag: f64) -> f64 {
        self.phase += self.rate;
        while self.phase >= 1.0 {
            self.phase -= 1.0;
            self.y_prev = self.y_cur;
            self.y_cur = self.step(pl, ag) * self.rate;
        }
        self.y_prev + (self.y_cur - self.y_prev) * self.phase
    }

    /// One step: lung pressure `pl` (dyn/cm²) behind folds of area `ag` (cm²). Returns the
    /// radiated sound (d/dt of the flow leaving the lips and nostrils).
    #[inline]
    fn step(&mut self, pl: f64, ag: f64) -> f64 {
        let n = self.n;
        if self.ramp_left > 0 {
            self.ramp_left -= 1;
            for j in 0..n {
                self.area[j] += self.area_step[j];
                self.amin[j] += self.amin_step[j];
            }
            self.port += self.port_step;
        }
        // ---- turbulence → series pressure sources at junctions
        let mut level = 0.0;
        let mut at = -1i32;
        let asp = {
            let w = self.asp_band.tick(&mut self.rng);
            if ag > CLOSED { self.asp_gain * self.noise_scale * turbulence(reynolds(self.ug, ag)) * w } else { 0.0 }
        };
        if asp != 0.0 {
            self.psrc[1] += if self.ug >= 0.0 { asp } else { -asp };
        }
        let wc = self.con_band.tick(&mut self.rng);
        let wt = self.teeth_band.tick(&mut self.rng);
        for (s, w) in [(self.con, wc), (self.teeth, wt)] {
            if s.gain == 0.0 {
                continue;
            }
            let a = self.amin[s.src.min(n - 1)];
            if a <= CLOSED {
                continue;
            }
            let u = self.flow[s.src];
            let amp = s.gain * self.noise_scale * turbulence(reynolds(u, a));
            if amp > level {
                level = amp;
                at = s.src as i32;
            }
            if u >= 0.0 {
                self.psrc[s.fwd] += amp * w;
            } else {
                self.psrc[s.bwd] -= amp * w;
            }
        }
        self.noise_level = level;
        self.noise_at = at;

        // ---- the folds: a Bernoulli orifice against the first section's wave impedance
        let a0 = js_max(self.area[0], CLOSED);
        let z0 = RC / a0;
        let d = pl - 2.0 * self.l[0];
        let ug = if ag > 1e-7 {
            let ad = d.abs();
            let za = z0 * ag;
            let u = (2.0 * ad * ag) / (za + sqrt(za * za + 2.0 * K_GLOTTIS * RHO * ad));
            if d >= 0.0 { u } else { -u }
        } else {
            0.0
        };
        self.ug = ug;
        self.p0 = 2.0 * self.l[0] + z0 * ug;
        self.r2[0] = (self.l[0] + z0 * ug) * self.dj[0];

        // ---- junctions between sections
        let jv = self.jv;
        for j in 0..n - 1 {
            let a1 = self.r[j];
            let a2 = self.l[j + 1];
            let (m1, m2) = (self.amin[j], self.amin[j + 1]);
            let (d1, d2) = (self.dj[j], self.dj[j + 1]);
            if j + 1 == jv {
                // the velopharyngeal junction: pharynx, mouth and nose meet at one pressure
                let a3 = self.nl[0];
                let pa = js_min(self.port, self.nose_profile[0]);
                let y1 = if m1 > CLOSED { js_max(self.area[j], m1) / RC } else { 0.0 };
                let y2 = if m2 > CLOSED { js_max(self.area[j + 1], m2) / RC } else { 0.0 };
                let y3 = if pa > CLOSED { pa / RC } else { 0.0 };
                let ys = y1 + y2 + y3;
                let (kw, hw, gw) = self.wall;
                let pj = (2.0 * (y1 * a1 + y2 * a2 + y3 * a3) - kw * self.wall_i + hw * self.wall_q) / (ys + gw);
                self.wall_i = kw * self.wall_i - hw * self.wall_q + gw * pj;
                self.wall_q += self.dt * self.wall_i;
                let b1 = if y1 > 0.0 { pj - a1 } else { a1 };
                self.l2[j] = b1 * d1;
                self.r2[j + 1] = (if y2 > 0.0 { pj - a2 } else { a2 }) * d2;
                self.nr2[0] = (if y3 > 0.0 { pj - a3 } else { a3 }) * self.damp_nose;
                self.flow[j] = y1 * (a1 - b1);
                self.psrc[j] = 0.0;
                continue;
            }
            if m1 <= CLOSED || m2 <= CLOSED {
                self.l2[j] = a1 * d1;
                self.r2[j + 1] = a2 * d2;
                self.flow[j] = 0.0;
                self.psrc[j] = 0.0;
                continue;
            }
            let z1 = RC / js_max(self.area[j], m1);
            let z2 = RC / js_max(self.area[j + 1], m2);
            let d = 2.0 * (a1 - a2) + self.psrc[j];
            self.psrc[j] = 0.0;
            let (i1, i2) = (1.0 / m1, 1.0 / m2);
            // wave impedances, plus the viscous resistance of the two half sections
            let zs = z1 + z2 + self.visc * (i1 * i1 + i2 * i2);
            // the jet leaving a narrow section into a wider one loses its kinetic energy
            let (up, down) = if d >= 0.0 { (i1, i2) } else { (i2, i1) };
            let u = if up > down {
                let e = up - down;
                let ad = d.abs();
                let v = (2.0 * ad) / (zs + sqrt(zs * zs + 2.0 * RHO * K_EXPAND * e * e * ad));
                if d >= 0.0 { v } else { -v }
            } else {
                d / zs
            };
            self.l2[j] = (a1 - z1 * u) * d1;
            self.r2[j + 1] = (a2 + z2 * u) * d2;
            self.flow[j] = u;
        }

        // ---- the lips: Flanagan's parallel R–L radiation load
        let al = self.amin[n - 1];
        let a = self.r[n - 1];
        let u_lip;
        if al <= CLOSED {
            self.l2[n - 1] = a * self.dj[n - 1];
            self.ul_lip *= 0.995;
            u_lip = 0.0;
        } else {
            let y = js_max(self.area[n - 1], al) / RC;
            let g_r = (9.0 * PI * PI * al) / (128.0 * RC);
            let g_l = self.dt * (3.0 * PI * sqrt(PI * al)) / (8.0 * RHO);
            let g = g_r + g_l;
            let b = (a * (y - g) - self.ul_lip) / (y + g);
            self.ul_lip += g_l * (a + b);
            u_lip = (a - b) * y;
            self.l2[n - 1] = b * self.dj[n - 1];
        }
        self.flow[n - 1] = u_lip;

        // ---- the nose
        let nn = self.nn;
        let dn = self.damp_nose;
        let pa = js_min(self.port, self.nose_profile[0]);
        for j in 0..nn - 1 {
            let a1 = self.nr[j];
            let a2 = self.nl[j + 1];
            // the port narrows the first two sections
            let s1 = if j < 2 { js_min(pa, self.nose_profile[j]) } else { self.nose_profile[j] };
            let s2 = if j + 1 < 2 { js_min(pa, self.nose_profile[j + 1]) } else { self.nose_profile[j + 1] };
            if s1 <= CLOSED || s2 <= CLOSED {
                self.nl2[j] = a1 * dn;
                self.nr2[j + 1] = a2 * dn;
                continue;
            }
            let (z1, z2) = (RC / s1, RC / s2);
            let u = (2.0 * (a1 - a2)) / (z1 + z2);
            self.nl2[j] = (a1 - z1 * u) * dn;
            self.nr2[j + 1] = (a2 + z2 * u) * dn;
        }
        let u_nose = {
            let (y, g, g_l) = self.nose_rad;
            let a = self.nr[nn - 1];
            let b = (a * (y - g) - self.ul_nose) / (y + g);
            self.ul_nose += g_l * (a + b);
            self.nl2[nn - 1] = b * dn;
            (a - b) * y
        };

        core::mem::swap(&mut self.r, &mut self.r2);
        core::mem::swap(&mut self.l, &mut self.l2);
        core::mem::swap(&mut self.nr, &mut self.nr2);
        core::mem::swap(&mut self.nl, &mut self.nl2);

        let u = u_lip + u_nose;
        let out = (u - self.u_prev) * self.fs;
        self.u_prev = u;
        out
    }
}

// ====================================================================== the formant synth

const N_FORMANT: usize = 6;
/// uniform sections the formant analysis resamples the tract to
const FM_SEC: usize = 40;
/// analysis grid: 150 Hz … 6.6 kHz in 50 Hz steps
const FM_F0: f64 = 150.0;
const FM_DF: f64 = 50.0;
const FM_NF: usize = 130;

/// Two-pole resonator with unity gain at DC; its coefficients ramp across a control step.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Reson {
    a: f64,
    b: f64,
    c: f64,
    da: f64,
    db: f64,
    dc: f64,
    y1: f64,
    y2: f64,
}

impl Reson {
    pub(super) fn target(&mut self, fs: f64, f: f64, bw: f64, steps: usize, snap: bool) {
        let f = js_min(f, 0.45 * fs);
        let r = exp(-PI * bw / fs);
        let c = -r * r;
        let b = 2.0 * r * cos(2.0 * PI * f / fs);
        let a = 1.0 - b - c;
        if snap {
            (self.a, self.b, self.c) = (a, b, c);
            (self.da, self.db, self.dc) = (0.0, 0.0, 0.0);
        } else {
            let inv = 1.0 / steps as f64;
            self.da = (a - self.a) * inv;
            self.db = (b - self.b) * inv;
            self.dc = (c - self.c) * inv;
        }
    }
    #[inline]
    pub(super) fn ramp(&mut self) {
        self.a += self.da;
        self.b += self.db;
        self.c += self.dc;
    }
    #[inline]
    pub(super) fn tick(&mut self, x: f64) -> f64 {
        let y = self.a * x + self.b * self.y1 + self.c * self.y2;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// A value that ramps linearly across a control step.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Ramp {
    pub(super) x: f64,
    d: f64,
}

impl Ramp {
    pub(super) fn to(&mut self, target: f64, steps: usize, snap: bool) {
        if snap {
            self.x = target;
            self.d = 0.0;
        } else {
            self.d = (target - self.x) / steps as f64;
        }
    }
    #[inline]
    pub(super) fn tick(&mut self) -> f64 {
        self.x += self.d;
        self.x
    }
}

#[derive(Clone, Debug)]
struct FormantSynth {
    fs: f64,
    dt: f64,
    scale: f64,
    /// cos and sin of k·Δ per analysis frequency are recomputed each step (Δ follows the length)
    sec: [f64; FM_SEC],
    nose_profile: Vec<f64>,
    nose_len: f64,
    pub formants: [f64; N_FORMANT],
    res: [Reson; N_FORMANT],
    nasal: Reson,
    fric: Reson,
    bar: f64,
    ramp_left: usize,
    started: bool,
    /// narrowest oral opening, velum port, lip opening (cm²)
    a_con: Ramp,
    a_port: Ramp,
    /// how much of the fold sound gets out (0 behind a closure)
    w_open: Ramp,
    /// nasal resonance mixed in
    w_nasal: Ramp,
    /// frication: noise pressure → radiated sound, and its weights
    fric_out: Ramp,
    fric_gain: Ramp,
    /// pressure behind the constriction (dyn/cm²), and the compliance it charges (cm⁵/dyn)
    pub p_oral: f64,
    compliance: f64,
    ug_prev: f64,
    pub ug: f64,
    rng: Rng,
    asp_band: NoiseBand,
    fric_band: NoiseBand,
    pub noise_level: f64,
    /// section (of the airway's ph+or) of the constriction, for telemetry
    pub con_node: i32,
}

impl FormantSynth {
    fn new(fs: f64, body: HumanBody) -> Self {
        let k = body.scale;
        let nose_profile: Vec<f64> = (0..16)
            .map(|i| {
                let u = (i as f64 + 0.5) / 16.0;
                let a = if u < 0.18 {
                    2.6
                } else if u < 0.75 {
                    2.0 - 0.6 * (u - 0.18)
                } else if u < 0.9 {
                    1.4
                } else {
                    1.0
                };
                a * k * k
            })
            .collect();
        let mut s = FormantSynth {
            fs,
            dt: 1.0 / fs,
            scale: k,
            sec: [1.0; FM_SEC],
            nose_profile,
            nose_len: 11.4 * k,
            formants: [500.0, 1500.0, 2500.0, 3500.0, 4500.0, 5500.0].map(|f| f / k),
            res: [Reson::default(); N_FORMANT],
            nasal: Reson::default(),
            fric: Reson::default(),
            bar: 0.0,
            ramp_left: 0,
            started: false,
            a_con: Ramp::default(),
            a_port: Ramp::default(),
            w_open: Ramp::default(),
            w_nasal: Ramp::default(),
            fric_out: Ramp::default(),
            fric_gain: Ramp::default(),
            p_oral: 0.0,
            compliance: 1.5e-3 * k * k * k,
            ug_prev: 0.0,
            ug: 0.0,
            rng: Rng::new(0x0f0f_1234),
            asp_band: NoiseBand::new(fs, 3500.0, 300.0),
            fric_band: NoiseBand::new(fs, 9000.0, 500.0),
            noise_level: 0.0,
            con_node: -1,
        };
        s.nasal.target(fs, 270.0 / k, 110.0, 1, true);
        s
    }

    /// The glottis-to-opening volume-velocity transfer has a pole wherever the flow at a closed
    /// glottis vanishes for unit flow at the opening; with lossless equal sections that flow is
    /// real, so the formants are its sign changes along the frequency grid.
    fn analyse(&mut self, len: f64, a_end: f64, out: &mut [f64; N_FORMANT]) -> usize {
        let dl = len / FM_SEC as f64;
        let lr = (8.0 * RHO) / (3.0 * PI * sqrt(PI * js_max(a_end, 0.01)));
        let mut found = 0;
        let mut prev = 0.0;
        for i in 0..FM_NF {
            let f = FM_F0 + FM_DF * i as f64;
            let w = 2.0 * PI * f;
            let kl = (w / C_SOUND) * dl;
            let (c, s) = (cos(kl), sin(kl));
            // at the opening: P = jωL·U (the radiation mass), U = 1
            let mut p = w * lr;
            let mut u = 1.0;
            for j in (0..FM_SEC).rev() {
                let z = RC / self.sec[j];
                let p2 = c * p + z * s * u;
                u = c * u - (s / z) * p;
                p = p2;
            }
            if i > 0 && (u > 0.0) != (prev > 0.0) {
                out[found] = f - FM_DF * u / (u - prev);
                found += 1;
                if found == N_FORMANT {
                    break;
                }
            }
            prev = u;
        }
        found
    }

    fn set_targets(&mut self, aw: &HumanAirway, velum_port: f64, teeth_s: f64, glottal_leak: f64, steps: usize) {
        let k = self.scale;
        let k2 = k * k;
        let (n_ph, n_or) = (aw.n_ph, aw.n_or);
        let (ph_len, or_len) = (aw.ph_len, aw.or_len);
        let len = ph_len + or_len;
        // the narrowest oral place (above the epilarynx: the folds' own tube is not a consonant)
        let mut min_a = f64::INFINITY;
        let mut min_i = 0usize;
        let skip = (n_ph / 4).max(1);
        for i in skip..n_ph + n_or {
            let a = if i < n_ph { aw.ph_min[i] } else { aw.or_min[i - n_ph] };
            if a < min_a {
                min_a = a;
                min_i = i;
            }
        }
        let min_a = js_max(0.0, min_a);
        self.con_node = min_i as i32;
        let s_min = if min_i < n_ph { (min_i as f64 + 0.5) * ph_len / n_ph as f64 } else { ph_len + ((min_i - n_ph) as f64 + 0.5) * or_len / n_or as f64 };
        let oral_shut = min_a < 0.02 * k2;
        let nasal_path = oral_shut && velum_port > 0.02 * k2;

        // ---- the tube the formants are read from
        let floor = 0.02 * k2;
        let (tube_len, a_end) = if nasal_path {
            // mouth shut, velum open: the sound leaves through the nose
            let total = ph_len + self.nose_len;
            for j in 0..FM_SEC {
                let s = (j as f64 + 0.5) * total / FM_SEC as f64;
                self.sec[j] = js_max(
                    floor,
                    if s < ph_len {
                        aw.ph_areas[((s / ph_len * n_ph as f64) as usize).min(n_ph - 1)]
                    } else {
                        let u = (s - ph_len) / self.nose_len;
                        let a = self.nose_profile[((u * 16.0) as usize).min(15)];
                        if u < 0.1 { js_min(a, velum_port) } else { a }
                    },
                );
            }
            (total, 1.0 * k2)
        } else {
            for j in 0..FM_SEC {
                let s = (j as f64 + 0.5) * len / FM_SEC as f64;
                self.sec[j] = js_max(
                    floor,
                    if s < ph_len { aw.ph_areas[((s / ph_len * n_ph as f64) as usize).min(n_ph - 1)] } else { aw.or_areas[(((s - ph_len) / or_len * n_or as f64) as usize).min(n_or - 1)] },
                );
            }
            (len, js_max(aw.or_min[n_or - 1], floor))
        };
        let mut f = [0.0; N_FORMANT];
        let found = self.analyse(tube_len, a_end, &mut f);
        for (i, x) in f.iter_mut().enumerate().skip(found) {
            // a formant the grid did not reach: the uniform tube's
            *x = js_max(if i > 0 { self.formants_floor(i, tube_len) } else { 0.0 }, (2.0 * i as f64 + 1.0) * C_SOUND / (4.0 * tube_len));
        }
        // yielding walls lift the lowest formant (the lossless tube has rigid ones)
        let fw = 190.0 / k;
        f[0] = sqrt(f[0] * f[0] + fw * fw);
        self.formants = f;
        let snap = !self.started;
        self.started = true;
        // bandwidths: wall and radiation losses grow with frequency; an open glottis damps F1
        let leak = js_min(1.0, glottal_leak / (0.2 * k2));
        for i in 0..N_FORMANT {
            let fi = f[i];
            let mut bw = 55.0 + 22.0 * i as f64 + 60.0 * (fi / 4000.0) * (fi / 4000.0);
            if i == 0 {
                bw += 140.0 * leak + if nasal_path { 60.0 } else { 0.0 };
            }
            self.res[i].target(self.fs, fi, bw, steps, snap);
        }

        // ---- the breath: what escapes, and the frication it makes
        let lip = js_max(0.0, aw.or_min[n_or - 1]);
        let _ = lip;
        self.a_con.to(min_a, steps, snap);
        self.a_port.to(js_max(0.0, velum_port), steps, snap);
        let a_out = min_a + js_max(0.0, velum_port);
        self.w_open.to(a_out / (a_out + 0.1 * k2), steps, snap);
        let nasality = smooth(0.0, 0.25 * k2, velum_port);
        self.w_nasal.to(if nasal_path { 0.0 } else { 0.45 * nasality }, steps, snap);
        // the cavity in front of the constriction rings at its quarter wavelength; none (the lips,
        // the teeth) leaves the noise broad and weak
        let front = js_max(0.0, len - s_min) + 0.8 * sqrt(js_max(a_end, floor) / PI);
        let f_front = clamp(C_SOUND / (4.0 * front), 1300.0, 0.4 * self.fs);
        let behind = (teeth_s - s_min) / k;
        let sibilant = smooth(0.15, 0.5, behind) * (1.0 - smooth(2.6, 4.0, behind)) * (1.0 - smooth(0.2 * k2, 0.5 * k2, min_a));
        self.fric.target(self.fs, f_front, 0.22 * f_front + 400.0, steps, snap);
        let w_area = 1.0 - smooth(0.35 * k2, 0.9 * k2, min_a);
        self.fric_gain.to((2.2e-6 + 9.0e-6 * sibilant) * w_area, steps, snap);
        // a pressure source in series with the front cavity's air mass: dU/dt = p·A/(ρ·l)
        let a_front = js_max(0.3 * k2, js_min(2.0 * k2, a_end));
        self.fric_out.to(a_front / (RHO * js_max(front, 0.6 * k)), steps, snap);
        self.ramp_left = steps;
    }

    fn formants_floor(&self, i: usize, _len: f64) -> f64 {
        self.formants[i - 1] + 300.0
    }

    /// One sample: lung pressure `pl`, fold area `ag`. Returns the radiated sound.
    #[inline]
    fn step(&mut self, pl: f64, ag: f64) -> f64 {
        if self.ramp_left > 0 {
            self.ramp_left -= 1;
            for r in self.res.iter_mut() {
                r.ramp();
            }
            self.fric.ramp();
            self.a_con.tick();
            self.a_port.tick();
            self.w_open.tick();
            self.w_nasal.tick();
            self.fric_out.tick();
            self.fric_gain.tick();
        }
        // ---- two orifices in series (folds, then the constriction and the velum port) around
        // the compliance of the mouth's air and walls
        let a_con = js_max(0.0, self.a_con.x);
        let a_out = a_con + js_max(0.0, self.a_port.x);
        let po = self.p_oral;
        let dp = pl - po;
        let v_g = sqrt(2.0 * dp.abs() / (K_GLOTTIS * RHO));
        let ug = if dp >= 0.0 { ag * v_g } else { -ag * v_g };
        let v_c = sqrt(2.0 * po.abs() / RHO);
        let uc = if po >= 0.0 { a_out * v_c } else { -a_out * v_c };
        let den = ag * ag + K_GLOTTIS * a_out * a_out;
        let p_static = if den > 1e-12 { pl * ag * ag / den } else { po };
        let mut p_new = po + (self.dt / self.compliance) * (ug - uc);
        if (po - p_static) * (p_new - p_static) <= 0.0 {
            p_new = p_static; // the pressure settles; it never swings past its resting value
        }
        self.p_oral = p_new;
        self.ug = ug;

        // ---- the fold sound through the formants
        let mut x = (ug - self.ug_prev) * self.fs;
        self.ug_prev = ug;
        let wa = self.asp_band.tick(&mut self.rng);
        if ag > CLOSED {
            // aspiration: a pressure source just above the folds, into the tract's air mass
            x += 3.2e-6 * turbulence(reynolds(ug, ag)) * wa * (2.0 * self.scale / RHO) * 0.08;
        }
        let mut y = x;
        for r in self.res.iter_mut() {
            y = r.tick(y);
        }
        let wn = self.w_nasal.x;
        let yn = self.nasal.tick(x);
        y = y * (1.0 - 0.5 * wn) + yn * wn;
        // behind a closure only the walls radiate: a murmur below a few hundred hertz
        self.bar += 0.03 * (x - self.bar);
        let w = self.w_open.x;
        let voiced = y * w + self.bar * 0.015 * (1.0 - w);

        // ---- frication and bursts: turbulence at the constriction, rung by the front cavity
        let wf = self.fric_band.tick(&mut self.rng);
        let mut fr = 0.0;
        self.noise_level = 0.0;
        if a_con > CLOSED {
            let u_con = a_con * v_c;
            let amp = self.fric_gain.x * turbulence(reynolds(u_con, a_con));
            self.noise_level = amp;
            fr = amp * wf * self.fric_out.x;
        }
        let fr = self.fric.tick(fr) * 0.6 + fr * 0.15;
        voiced + fr
    }
}

// ====================================================================== the voice

#[derive(Clone, Debug)]
enum Tract {
    Waveguide(Box<Waveguide>),
    Formant(Box<FormantSynth>),
    Neural(Box<NeuralSynth>),
}

#[derive(Clone, Debug)]
pub struct LiteVoice {
    pub kind: LiteKind,
    pub fs_out: f64,
    pub m: usize,
    pub fs: f64,
    /// the head: articulators → area functions (its network never steps)
    pub airway: HumanAirway,
    pub glottis: ParametricGlottis,
    pub harmony: Harmony,
    pub artic: ArticulatorBank,
    pub source: VoiceSource,
    tract: Tract,
    dec: Decimator,
    ctrl_every: usize,
    ctrl_counter: i64,
    pub gate: f64,
    gate_target: f64,
    score_breath_hold: f64,
    p_lung: f64,
    p_lung_now: f64,
    p_lung_step: f64,
    f0: f64,
    pub output_gain: f64,
    dc_x: f64,
    dc_y: f64,
    dc_coef: f64,
    rms_acc: Vec<f64>,
    rms_count: u32,
    flow_sm: Vec<f64>,
    scope_area: Vec<f32>,
    scope_flow: Vec<f32>,
    scope_pos: usize,
    cycle_starts: [usize; 8],
    cycle_idx: usize,
    last_phase: f64,
    out_acc: f64,
    out_count: u32,
    sub: u32,
    pub score: GestureScore,
    pub clock: f64,
    blend: [Blended; N_CONTROL],
    base_abduction: f64,
    base_closure: f64,
    score_f0: Option<f64>,
    lim_env: f64,
    lim_hold: i64,
    lim_hold_n: i64,
    lim_release: f64,
    pub max_penetration: f64,
}

impl LiteVoice {
    pub fn new(kind: LiteKind, fs_out: f64, body: HumanBody) -> Self {
        // the head is built as the airway voice builds it, so the section counts (and with them
        // the telemetry and the drawings) are the same whichever engine sounds
        let head_fs = fs_out * oversample_for(fs_out) as f64;
        let airway = HumanAirway::new(head_fs, body);
        let m = match kind {
            LiteKind::Waveguide => oversample_for(fs_out),
            LiteKind::Formant | LiteKind::Neural => 1,
        };
        let fs = fs_out * m as f64;
        let tract = match kind {
            LiteKind::Waveguide => Tract::Waveguide(Box::new(Waveguide::new(fs, body))),
            LiteKind::Formant => Tract::Formant(Box::new(FormantSynth::new(fs, body))),
            LiteKind::Neural => Tract::Neural(Box::new(NeuralSynth::new(fs, body))),
        };
        let n_tract = airway.n_ph + airway.n_or;
        let mut v = LiteVoice {
            kind,
            fs_out,
            m,
            fs,
            airway,
            glottis: ParametricGlottis::new(fs),
            harmony: Harmony::new(fs),
            artic: ArticulatorBank::new(),
            source: DEFAULT_SOURCE,
            tract,
            dec: Decimator::new(m),
            ctrl_every: 64 * m,
            ctrl_counter: 0,
            gate: 0.0,
            gate_target: 0.0,
            score_breath_hold: 0.0,
            p_lung: 0.0,
            p_lung_now: 0.0,
            p_lung_step: 0.0,
            f0: DEFAULT_SOURCE.f0,
            // the airway voice's gain, less what the lossless tubes ring louder by
            output_gain: 5.5e-7 * match kind {
                LiteKind::Waveguide => 0.7,
                LiteKind::Formant => 0.85,
                LiteKind::Neural => 0.82,
            },
            dc_x: 0.0,
            dc_y: 0.0,
            dc_coef: exp((-2.0 * PI * 25.0) / fs_out),
            rms_acc: vec![0.0; n_tract],
            rms_count: 0,
            flow_sm: vec![0.0; n_tract],
            scope_area: vec![0.0; SCOPE_LEN],
            scope_flow: vec![0.0; SCOPE_LEN],
            scope_pos: 0,
            cycle_starts: [0; 8],
            cycle_idx: 0,
            last_phase: 0.0,
            out_acc: 0.0,
            out_count: 0,
            sub: 0,
            score: GestureScore::new(),
            clock: 0.0,
            blend: [Blended::default(); N_CONTROL],
            base_abduction: 0.0,
            base_closure: 0.0,
            score_f0: None,
            lim_env: 0.0,
            lim_hold: 0,
            lim_hold_n: js_round(0.025 * fs_out) as i64,
            lim_release: exp(-1.0 / (0.2 * fs_out)),
            max_penetration: 0.1,
        };
        let x = v.artic.x;
        v.airway.apply(&x, 0.0);
        let sc = v.airway.body.scale;
        v.output_gain *= if sc >= 1.0 {
            1.0
        } else if sc <= 0.9 {
            0.55
        } else {
            0.55 + 0.45 * (sc - 0.9) / 0.1
        };
        v.apply_source();
        v
    }

    pub fn set_articulation(&mut self, values: &ArticPatch, tip_free: Option<bool>) {
        if let Some(t) = tip_free {
            self.artic.tip_free = t;
        }
        self.artic.set_target(values);
    }

    pub fn set_source(&mut self, values: &SourcePatch) {
        values.apply(&mut self.source);
        self.apply_source();
    }

    pub fn set_harmony(&mut self, ratios: &[f64]) {
        self.harmony.set_ratios(ratios, HARMONY_GAIN);
    }

    pub fn schedule(&mut self, gestures: &[Gesture]) {
        self.score.add_all(gestures);
    }

    pub fn release(&mut self, tag: f64) {
        self.score.release(tag, self.clock);
    }

    pub fn cancel_score(&mut self) {
        self.score.clear();
    }

    pub fn set_gate(&mut self, level: f64) {
        self.gate_target = clamp(level, 0.0, 1.5);
    }

    fn apply_source(&mut self) {
        let k2 = self.airway.body.scale * self.airway.body.scale;
        self.base_abduction = source_to_glottis(&self.source, k2, &mut self.glottis.params);
        self.base_closure = 0.0;
        self.harmony.set_golden(self.source.golden);
    }

    /// The formants the formant engine is sounding (Hz); none for the waveguide.
    pub fn formants(&self) -> Option<&[f64]> {
        match &self.tract {
            Tract::Formant(f) => Some(&f.formants),
            Tract::Neural(n) => Some(&n.formants),
            Tract::Waveguide(_) => None,
        }
    }

    pub fn render(&mut self, out: &mut [f32]) {
        let m = self.m;
        for o in out.iter_mut() {
            for _ in 0..m {
                if self.ctrl_counter <= 0 {
                    self.control();
                }
                self.ctrl_counter -= 1;
                self.p_lung_now += self.p_lung_step;
                let pl = self.p_lung_now;
                let (p_above, y, ug);
                match &mut self.tract {
                    Tract::Waveguide(w) => {
                        let dp = pl - w.p0;
                        let mut a = self.glottis.next(dp);
                        if self.harmony.is_active() {
                            a += self.harmony.next(&self.glottis.params, dp);
                        }
                        y = w.tick(pl, a);
                        ug = w.ug;
                        p_above = a;
                    }
                    Tract::Formant(f) => {
                        let dp = pl - f.p_oral;
                        let mut a = self.glottis.next(dp);
                        if self.harmony.is_active() {
                            a += self.harmony.next(&self.glottis.params, dp);
                        }
                        y = f.step(pl, a);
                        ug = f.ug;
                        p_above = a;
                    }
                    Tract::Neural(n) => {
                        let dp = pl - n.p_oral;
                        let mut a = self.glottis.next(dp);
                        if self.harmony.is_active() {
                            a += self.harmony.next(&self.glottis.params, dp);
                        }
                        y = n.step(pl, a);
                        ug = n.ug;
                        p_above = a;
                    }
                }
                let a = p_above;
                if self.dec.push(y) {
                    let mut v = self.dec.value * self.output_gain;
                    let hp = v - self.dc_x + self.dc_coef * self.dc_y;
                    self.dc_x = v;
                    self.dc_y = hp;
                    v = hp;
                    // the airway voice's output limiter
                    let av = v.abs();
                    if av >= self.lim_env {
                        self.lim_env = av;
                        self.lim_hold = self.lim_hold_n;
                    } else if self.lim_hold > 0 {
                        self.lim_hold -= 1;
                    } else {
                        self.lim_env *= self.lim_release;
                    }
                    if self.lim_env > 0.75 {
                        v *= 0.75 / self.lim_env;
                    }
                    let al = v.abs();
                    if al > 0.85 {
                        v = js_sign(v) * (0.85 + 0.15 * tanh((al - 0.85) / 0.15));
                    }
                    if !v.is_finite() {
                        v = 0.0;
                    }
                    *o = v as f32;
                    self.out_acc += v * v;
                    self.out_count += 1;
                    self.scope_area[self.scope_pos] = a as f32;
                    self.scope_flow[self.scope_pos] = ug as f32;
                    let ph = self.glottis.cycle_phase();
                    if ph < self.last_phase {
                        self.cycle_starts[self.cycle_idx & 7] = self.scope_pos;
                        self.cycle_idx += 1;
                    }
                    self.last_phase = ph;
                    self.scope_pos = (self.scope_pos + 1) % SCOPE_LEN;
                }
                self.sub = self.sub.wrapping_add(1);
                if self.sub & 3 == 0 {
                    self.accumulate_pressure();
                }
            }
        }
    }

    /// Pressure per airway section for the drawings (every fourth step).
    fn accumulate_pressure(&mut self) {
        let n_t = self.airway.n_ph + self.airway.n_or;
        match &self.tract {
            Tract::Waveguide(w) => {
                let n = w.n;
                for k in 0..n_t {
                    let j = ((k as f64 + 0.5) / n_t as f64 * n as f64) as usize;
                    let j = j.min(n - 1);
                    let p = w.r[j] + w.l[j];
                    self.rms_acc[k] += p * p;
                }
            }
            Tract::Formant(f) => {
                // no waves to show: the breath pressure behind the constriction, and the fold
                // sound's swing everywhere
                let con = f.con_node.max(0) as usize;
                let swing = f.ug * RC / 3.0;
                for k in 0..n_t {
                    let p = if k <= con { f.p_oral } else { 0.0 } + swing * (1.0 - k as f64 / n_t as f64);
                    self.rms_acc[k] += p * p;
                }
            }
            Tract::Neural(n) => {
                let con = n.con_node.max(0) as usize;
                let swing = n.ug * RC / 3.0;
                for k in 0..n_t {
                    let p = if k <= con { n.p_oral } else { 0.0 } + swing * (1.0 - k as f64 / n_t as f64);
                    self.rms_acc[k] += p * p;
                }
            }
        }
        self.rms_count += 1;
    }

    fn control(&mut self) {
        let dt = self.ctrl_every as f64 / self.fs;
        let t = self.clock;
        let mut breath_act = 0.0;
        let mut breath_val = 0.0;
        let mut f0_target = self.source.f0;
        if self.score.size() > 0 {
            self.score.blend_all(t, &mut self.blend);
            let b = &self.blend;
            self.artic.step(dt, Some(&b[..]), 0.05);
            let gp = &mut self.glottis.params;
            let k2 = self.airway.body.scale * self.airway.body.scale;
            let ab = b[ControlKey::ABDUCTION.index()];
            let ab_act = if ab.weight > 0.0 { js_min(1.0, ab.weight / 2.0) } else { 0.0 };
            gp.abduction = self.base_abduction + ab.value * ab_act * k2;
            let cl = b[ControlKey::CLOSURE.index()];
            gp.closure = js_max(self.base_closure, if cl.weight > 0.0 { cl.value * js_min(1.0, cl.weight / 2.0) } else { 0.0 });
            let br = b[ControlKey::BREATH.index()];
            if br.weight > 0.0 {
                breath_act = js_min(1.0, br.weight);
                breath_val = br.value;
            }
            let vb = b[ControlKey::VIBRATO.index()];
            gp.vibrato_depth = if vb.weight > 0.0 { (self.source.vibrato_depth * 0.05 + vb.value * vb.weight) / (0.05 + vb.weight) } else { self.source.vibrato_depth };
            let pf = b[ControlKey::F0.index()];
            if pf.weight > 0.0 {
                f0_target = (self.source.f0 * 0.02 + pf.value * pf.weight) / (0.02 + pf.weight);
                self.score_f0 = Some(f0_target);
            } else {
                self.score_f0 = None;
            }
            if js_trunc_i32(self.clock * 10.0) != js_trunc_i32((self.clock - dt) * 10.0) {
                self.score.prune(t - 0.2);
            }
        } else {
            self.artic.step(dt, None, 1.0);
            self.glottis.params.abduction = self.base_abduction;
            self.glottis.params.closure = self.base_closure;
            self.score_f0 = None;
        }
        self.clock += dt;
        let x = self.artic.x;
        self.airway.apply(&x, dt);
        if self.enforce_contacts() {
            let x = self.artic.x;
            self.airway.reapply(&x);
        }
        let (velum_port, teeth_s) = self.airway.frame.as_ref().map_or((0.0, 0.0), |f| (f.velum_port, f.teeth_s));
        let steps = self.ctrl_every;
        match &mut self.tract {
            Tract::Waveguide(w) => w.set_targets(&self.airway, velum_port, teeth_s, steps),
            Tract::Formant(f) => f.set_targets(&self.airway, velum_port, teeth_s, self.glottis.statics(), steps),
            Tract::Neural(n) => n.set_targets(&self.airway, velum_port, teeth_s, self.glottis.statics(), steps),
        }
        self.ctrl_counter = self.ctrl_every as i64;
        let score_breath = breath_val * breath_act;
        let gate_goal = if score_breath < 0.0 { score_breath } else { js_max(self.gate_target, score_breath) };
        self.score_breath_hold = if breath_act > 0.0 { 0.15 } else { js_max(0.0, self.score_breath_hold - dt) };
        let tau = if gate_goal > self.gate {
            0.025
        } else if self.score_breath_hold > 0.0 {
            0.02
        } else {
            0.07
        };
        self.gate += (gate_goal - self.gate) * (1.0 - exp(-dt / tau));
        self.p_lung = self.source.pressure * CM_H2O * self.gate;
        self.p_lung_step = (self.p_lung - self.p_lung_now) / self.ctrl_every as f64;
        self.glottis.params.effort = clamp(((self.source.pressure * self.gate).abs() - 3.0) / 10.0, 0.0, 1.0);
        let g = js_max(0.002, if self.score_f0.is_some() { 0.03 } else { self.source.glide });
        self.f0 *= pow(f0_target / self.f0, 1.0 - exp(-dt / g));
        self.glottis.params.f0 = self.f0;
        // mean flow per section, for the particles
        let a = 1.0 - exp(-dt / 0.03);
        let n_t = self.airway.n_ph + self.airway.n_or;
        match &self.tract {
            Tract::Waveguide(w) => {
                for k in 0..n_t {
                    let j = (((k as f64 + 0.5) / n_t as f64 * w.n as f64) as usize).min(w.n - 1);
                    self.flow_sm[k] += a * (w.flow[j] - self.flow_sm[k]);
                }
            }
            Tract::Formant(f) => {
                for k in 0..n_t {
                    self.flow_sm[k] += a * (f.ug - self.flow_sm[k]);
                }
            }
            Tract::Neural(n) => {
                for k in 0..n_t {
                    self.flow_sm[k] += a * (n.ug - self.flow_sm[k]);
                }
            }
        }
    }

    fn enforce_contacts(&mut self) -> bool {
        let tract = &self.airway.tract;
        let k = tract.scale();
        let lim = -self.max_penetration * k;
        let mut changed = false;
        let mut fix = |artic: &mut ArticulatorBank, d: f64, n: Vec2, kx: ArticKey, ky: ArticKey| {
            if !(d < lim) {
                return;
            }
            changed = true;
            let depth = lim - d;
            let sx = (if artic.x.get(kx) > 0.5 { 1.0 / 0.86 } else { 1.0 }) / k;
            let sy = 1.0 / k;
            *artic.x.get_mut(kx) += n.x * depth * sx;
            *artic.x.get_mut(ky) += n.y * depth * sy;
            let (ix, iy) = (kx.index(), ky.index());
            let vn = artic.v[ix] * n.x / sx + artic.v[iy] * n.y / sy;
            if vn < 0.0 {
                artic.v[ix] -= vn * n.x * sx;
                artic.v[iy] -= vn * n.y * sy;
            }
        };
        let (ct, ctn, cb, cbn) = (tract.contact_tip, tract.contact_tip_n, tract.contact_body, tract.contact_body_n);
        fix(&mut self.artic, ct, ctn, ArticKey::TtX, ArticKey::TtY);
        fix(&mut self.artic, cb, cbn, ArticKey::TbX, ArticKey::TbY);
        changed
    }

    pub fn telemetry(&mut self, out: &mut VoiceTelemetry) {
        let n_t = self.airway.n_ph + self.airway.n_or;
        out.artic.clear();
        out.artic.extend(ArticKey::ALL.iter().map(|k| self.artic.x.get(*k) as f32));
        debug_assert_eq!(out.artic.len(), N_ARTIC);
        let c = js_max(1.0, self.rms_count as f64);
        out.pressure_rms.clear();
        out.pressure_rms.extend((0..n_t).map(|k| sqrt(self.rms_acc[k] / c) as f32));
        self.rms_acc.fill(0.0);
        self.rms_count = 0;
        let mut start = (self.scope_pos + SCOPE_LEN - 1024) % SCOPE_LEN;
        let mut len = 1024usize;
        if self.cycle_idx >= 3 && self.glottis.voicing > 0.2 {
            let c2 = self.cycle_starts[(self.cycle_idx - 3) & 7];
            let c0 = self.cycle_starts[(self.cycle_idx - 1) & 7];
            let span = (c0 + SCOPE_LEN - c2) % SCOPE_LEN;
            if span > 8 && span < SCOPE_LEN / 2 {
                start = c2;
                len = span;
            }
        }
        out.glottal_area.clear();
        out.glottal_flow.clear();
        for i in 0..SCOPE_POINTS {
            let idx = (start + (i * len) / SCOPE_POINTS) % SCOPE_LEN;
            out.glottal_area.push(self.scope_area[idx]);
            out.glottal_flow.push(self.scope_flow[idx]);
        }
        let (noise_level, noise_node) = match &self.tract {
            Tract::Waveguide(w) => (w.noise_level, if w.noise_at >= 0 { ((w.noise_at as f64 + 0.5) / w.n as f64 * n_t as f64) as i32 } else { -1 }),
            Tract::Formant(f) => (f.noise_level, if f.noise_level > 0.0 { f.con_node } else { -1 }),
            Tract::Neural(n) => (n.noise_level, if n.noise_level > 0.0 { n.con_node } else { -1 }),
        };
        let out_rms = sqrt(self.out_acc / js_max(1.0, self.out_count as f64));
        self.out_acc = 0.0;
        self.out_count = 0;
        out.ph_areas.clear();
        out.ph_areas.extend(self.airway.ph_areas.iter().map(|x| *x as f32));
        out.or_areas.clear();
        out.or_areas.extend(self.airway.or_areas.iter().map(|x| *x as f32));
        out.ph_len = self.airway.ph_len;
        out.or_len = self.airway.or_len;
        out.flow.clear();
        out.flow.extend(self.flow_sm.iter().map(|x| *x as f32));
        out.velum_port = self.airway.frame.as_ref().map_or(0.0, |f| f.velum_port);
        out.f0 = self.glottis.f0_now;
        out.voicing = self.glottis.voicing;
        out.p_lung = self.p_lung_now / CM_H2O;
        out.out_rms = out_rms;
        out.noise_level = noise_level;
        out.noise_node = noise_node;
        out.gate = self.gate;
        out.clock = self.clock;
        out.active.clear();
        out.active.reserve(4);
        for g in self.score.active_at(self.clock) {
            if out.active.len() == 4 {
                break;
            }
            if let Some(l) = g.label
                && !l.is_empty()
                && l.as_str() != "breath"
                && l.as_str() != "pitch"
            {
                out.active.push(l);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anatomy::human::{BODY_FEMALE, BODY_MALE};
    use crate::phonetics::planner::vowel_table;

    /// Resonances of the waveguide with the folds shut: peaks of its impulse response's spectrum.
    fn tube_peaks(w: &mut Waveguide, n_peaks: usize) -> Vec<f64> {
        let n = 1 << 14;
        let mut y = vec![0.0; n];
        w.r[0] = 1000.0;
        for v in y.iter_mut() {
            *v = w.step(0.0, 0.0);
        }
        // direct DFT magnitude on a 10 Hz grid up to 5 kHz
        let mut mag = Vec::new();
        let mut f = 100.0;
        while f < 5000.0 {
            let (mut re, mut im) = (0.0, 0.0);
            for (i, v) in y.iter().enumerate() {
                let win = 0.5 - 0.5 * cos(2.0 * PI * i as f64 / n as f64);
                let ph = 2.0 * PI * f * i as f64 / w.fs;
                re += v * win * cos(ph);
                im -= v * win * sin(ph);
            }
            // the output is a derivative: undo its tilt
            mag.push((f, sqrt(re * re + im * im) / f));
            f += 10.0;
        }
        let mut peaks = Vec::new();
        for i in 1..mag.len() - 1 {
            if mag[i].1 > mag[i - 1].1 && mag[i].1 >= mag[i + 1].1 {
                peaks.push(mag[i].0);
            }
        }
        peaks.truncate(n_peaks);
        peaks
    }

    #[test]
    fn the_waveguide_resonates_where_the_formant_analysis_says() {
        for (name, body) in [("male", BODY_MALE), ("female", BODY_FEMALE)] {
            for ipa in ["i", "ɑ", "u", "ɛ"] {
                let mut v = LiteVoice::new(LiteKind::Waveguide, 48000.0, body);
                v.set_articulation(&vowel_table()[name][ipa].artic, Some(true));
                v.artic.snap(&vowel_table()[name][ipa].artic);
                let x = v.artic.x;
                v.airway.apply(&x, 0.0);
                let mut f = FormantSynth::new(48000.0, body);
                f.set_targets(&v.airway, 0.0, 0.0, 0.0, 64);
                let mut w = Waveguide::new(96000.0, body);
                w.set_targets(&v.airway, 0.0, 0.0, 1);
                w.step(0.0, 0.0);
                let peaks: Vec<f64> = tube_peaks(&mut w, 8).iter().map(|p| p * w.rate).collect();
                println!("{name} {ipa}: waveguide {:?}  analysis {:?}  table {:?}", peaks, &f.formants[..3], &vowel_table()[name][ipa].formants[..3]);
                for k in 0..3 {
                    // the nearest peak (the spectrum can show a small extra one between formants)
                    let rel = peaks.iter().map(|p| (p - f.formants[k]).abs() / f.formants[k]).fold(f64::INFINITY, f64::min);
                    assert!(rel < 0.08, "{name} {ipa} F{}: waveguide {:?} vs analysis {}", k + 1, peaks, f.formants[k]);
                }
            }
        }
    }
}
