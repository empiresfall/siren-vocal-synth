//! Neural vocal-tract synthesizer (`LiteKind::Neural`): a trained 14 -> 32 -> 32 -> 16 SiLU MLP
//! (`neural_weights.rs`, fitted offline to 4,500+ airway transfer functions across Male, Female
//! and Child bodies) predicts continuous formant frequencies, bandwidths, parallel clarity weights
//! and epilaryngeal singer's-formant presence directly from the head's log-area profile at control
//! rate, driving a hybrid cascade–parallel resonator bank and aerodynamic constriction noise.
//!
//! Real-time safe: all weights are static `f32` tables, hidden activations live on the stack, and
//! every transcendental call goes through [`crate::math`].

use super::human_airway::HumanAirway;
use super::lite_voice::{CLOSED, K_GLOTTIS, NoiseBand, RC, Ramp, Reson, reynolds, turbulence};
use super::neural_weights::{B1, B2, B3, NN_H1, NN_H2, NN_IN, NN_OUT, W1, W2, W3};
use crate::anatomy::human::HumanBody;
use crate::math::{PI, Rng, clamp, exp, js_max, js_min, ln, smoothstep as smooth, sqrt};
use crate::physics::{C_SOUND, RHO};

const N_FORMANT: usize = 6;
const FM_SEC: usize = 40;

#[inline]
fn silu(x: f64) -> f64 {
    x / (1.0 + exp(-x))
}

/// Forward pass of the 14 -> 32 -> 32 -> 16 SiLU MLP on the stack (zero allocation).
fn mlp_forward(x: &[f64; NN_IN], out: &mut [f64; NN_OUT]) {
    let mut h1 = [0.0; NN_H1];
    for i in 0..NN_H1 {
        let mut s = B1[i] as f64;
        let row = i * NN_IN;
        for j in 0..NN_IN {
            s += (W1[row + j] as f64) * x[j];
        }
        h1[i] = silu(s);
    }
    let mut h2 = [0.0; NN_H2];
    for i in 0..NN_H2 {
        let mut s = B2[i] as f64;
        let row = i * NN_H1;
        for j in 0..NN_H1 {
            s += (W2[row + j] as f64) * h1[j];
        }
        h2[i] = silu(s);
    }
    for i in 0..NN_OUT {
        let mut s = B3[i] as f64;
        let row = i * NN_H2;
        for j in 0..NN_H2 {
            s += (W3[row + j] as f64) * h2[j];
        }
        out[i] = s;
    }
}

#[derive(Clone, Debug)]
pub(super) struct NeuralSynth {
    fs: f64,
    dt: f64,
    scale: f64,
    sec: [f64; FM_SEC],
    nose_profile: [f64; 16],
    nose_len: f64,
    pub formants: [f64; N_FORMANT],
    res: [Reson; N_FORMANT],
    par_f2: Reson,
    singer: Reson,
    nasal: Reson,
    fric: Reson,
    glot_lp: f64,
    bar: f64,
    ramp_left: usize,
    started: bool,
    a_con: Ramp,
    a_port: Ramp,
    w_open: Ramp,
    w_nasal: Ramp,
    w_clarity: Ramp,
    w_singer: Ramp,
    fric_out: Ramp,
    fric_gain: Ramp,
    pub p_oral: f64,
    compliance: f64,
    ug_prev: f64,
    pub ug: f64,
    rng: Rng,
    asp_band: NoiseBand,
    fric_band: NoiseBand,
    pub noise_level: f64,
    pub con_node: i32,
}

impl NeuralSynth {
    pub fn new(fs: f64, body: HumanBody) -> Self {
        let k = body.scale;
        let mut nose_profile = [0.0; 16];
        for (i, slot) in nose_profile.iter_mut().enumerate() {
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
            *slot = a * k * k;
        }
        let mut s = NeuralSynth {
            fs,
            dt: 1.0 / fs,
            scale: k,
            sec: [1.0; FM_SEC],
            nose_profile,
            nose_len: 11.4 * k,
            formants: [500.0, 1500.0, 2500.0, 3500.0, 4500.0, 5500.0].map(|f| f / k),
            res: [Reson::default(); N_FORMANT],
            par_f2: Reson::default(),
            singer: Reson::default(),
            nasal: Reson::default(),
            fric: Reson::default(),
            glot_lp: 0.0,
            bar: 0.0,
            ramp_left: 0,
            started: false,
            a_con: Ramp::default(),
            a_port: Ramp::default(),
            w_open: Ramp::default(),
            w_nasal: Ramp::default(),
            w_clarity: Ramp::default(),
            w_singer: Ramp::default(),
            fric_out: Ramp::default(),
            fric_gain: Ramp::default(),
            p_oral: 0.0,
            compliance: 1.5e-3 * k * k * k,
            ug_prev: 0.0,
            ug: 0.0,
            rng: Rng::new(0x517e_7788),
            asp_band: NoiseBand::new(fs, 4200.0, 320.0),
            fric_band: NoiseBand::new(fs, 9200.0, 500.0),
            noise_level: 0.0,
            con_node: -1,
        };
        s.nasal.target(fs, 265.0 / k, 105.0, 1, true);
        s.singer.target(fs, 2950.0 / k, 155.0, 1, true);
        s
    }

    pub fn set_targets(&mut self, aw: &HumanAirway, velum_port: f64, teeth_s: f64, glottal_leak: f64, steps: usize) {
        let k = self.scale;
        let k2 = k * k;
        let (n_ph, n_or) = (aw.n_ph, aw.n_or);
        let (ph_len, or_len) = (aw.ph_len, aw.or_len);
        let len = ph_len + or_len;

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
        let s_min = if min_i < n_ph {
            (min_i as f64 + 0.5) * ph_len / n_ph as f64
        } else {
            ph_len + ((min_i - n_ph) as f64 + 0.5) * or_len / n_or as f64
        };
        let oral_shut = min_a < 0.02 * k2;
        let nasal_path = oral_shut && velum_port > 0.02 * k2;
        let floor = 0.02 * k2;

        let (tube_len, a_end) = if nasal_path {
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
                    if s < ph_len {
                        aw.ph_areas[((s / ph_len * n_ph as f64) as usize).min(n_ph - 1)]
                    } else {
                        aw.or_areas[(((s - ph_len) / or_len * n_or as f64) as usize).min(n_or - 1)]
                    },
                );
            }
            (len, js_max(aw.or_min[n_or - 1], floor))
        };

        // ---- 14-D feature vector -> MLP forward pass
        let mut x = [0.0; NN_IN];
        for b in 0..10 {
            let mut sum = 0.0;
            for j in 0..4 {
                sum += self.sec[b * 4 + j];
            }
            let mean = js_max(sum * 0.25, floor);
            x[b] = clamp(ln(mean / k2) * 0.4, -1.5, 1.5);
        }
        x[10] = clamp((tube_len / k - 17.5) * 0.25, -1.5, 1.5);
        x[11] = clamp(ln(js_max(a_end, floor) / k2) * 0.4, -1.5, 1.5);
        x[12] = clamp(velum_port / (0.5 * k2), 0.0, 1.5);
        x[13] = clamp(k - 1.0, -0.5, 0.5);

        let mut y = [0.0; NN_OUT];
        mlp_forward(&x, &mut y);

        // Decode F1..F5 (enforcing strict separation so resonators never collide) + F6 ceiling
        let refs = [500.0, 1500.0, 2500.0, 3500.0, 4500.0];
        let mut f = [0.0; N_FORMANT];
        for i in 0..5 {
            let fk = clamp(refs[i] + 1000.0 * y[i], 200.0 + 320.0 * i as f64, 1400.0 + 1150.0 * i as f64);
            let fi = fk / k;
            f[i] = if i == 0 { fi } else { js_max(fi, f[i - 1] + 240.0 / k) };
        }
        f[5] = js_max(f[4] + 650.0 / k, (11.0 * C_SOUND) / (4.0 * tube_len));
        self.formants = f;

        let snap = !self.started;
        self.started = true;
        let leak = js_min(1.0, glottal_leak / (0.2 * k2));
        for i in 0..5 {
            let bw_pred = clamp(90.0 + 100.0 * y[5 + i], 42.0, 320.0);
            let bw = bw_pred + if i == 0 { 130.0 * leak } else { 0.0 };
            self.res[i].target(self.fs, f[i], bw, steps, snap);
        }
        self.res[5].target(self.fs, f[5], 240.0, steps, snap);

        // Parallel clarity & Sundberg epilarynx presence resonators predicted by the MLP
        self.par_f2.target(self.fs, f[1], 95.0, steps, snap);
        let f_singer = clamp(0.5 * (f[2] + f[3]), 2550.0 / k, 3450.0 / k);
        self.singer.target(self.fs, f_singer, 145.0, steps, snap);

        let clarity = clamp(0.5 * (y[11] + 1.0), 0.0, 1.0);
        let singer_amt = clamp(0.5 * (y[15] + 1.0), 0.0, 1.0);
        self.w_clarity.to(0.16 * clarity, steps, snap);
        self.w_singer.to(0.14 * singer_amt, steps, snap);

        // Breath & constriction frication
        self.a_con.to(min_a, steps, snap);
        self.a_port.to(js_max(0.0, velum_port), steps, snap);
        let a_out = min_a + js_max(0.0, velum_port);
        self.w_open.to(a_out / (a_out + 0.1 * k2), steps, snap);
        let nasality = smooth(0.0, 0.25 * k2, velum_port);
        self.w_nasal.to(if nasal_path { 0.0 } else { 0.42 * nasality }, steps, snap);

        let front = js_max(0.0, len - s_min) + 0.8 * sqrt(js_max(a_end, floor) / PI);
        let f_front = clamp(C_SOUND / (4.0 * front), 1300.0, 0.4 * self.fs);
        let behind = (teeth_s - s_min) / k;
        let sibilant = smooth(0.15, 0.5, behind) * (1.0 - smooth(2.6, 4.0, behind)) * (1.0 - smooth(0.2 * k2, 0.5 * k2, min_a));
        self.fric.target(self.fs, f_front, 0.22 * f_front + 380.0, steps, snap);
        let w_area = 1.0 - smooth(0.35 * k2, 0.9 * k2, min_a);
        self.fric_gain.to((2.2e-6 + 9.0e-6 * sibilant) * w_area, steps, snap);
        let a_front = js_max(0.3 * k2, js_min(2.0 * k2, a_end));
        self.fric_out.to(a_front / (RHO * js_max(front, 0.6 * k)), steps, snap);
        self.ramp_left = steps;
    }

    #[inline]
    pub fn step(&mut self, pl: f64, ag: f64) -> f64 {
        if self.ramp_left > 0 {
            self.ramp_left -= 1;
            for r in self.res.iter_mut() {
                r.ramp();
            }
            self.par_f2.ramp();
            self.singer.ramp();
            self.fric.ramp();
            self.a_con.tick();
            self.a_port.tick();
            self.w_open.tick();
            self.w_nasal.tick();
            self.w_clarity.tick();
            self.w_singer.tick();
            self.fric_out.tick();
            self.fric_gain.tick();
        }
        // Two orifices in series (folds, then constriction + velum) around oral compliance
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
            p_new = p_static;
        }
        self.p_oral = p_new;
        self.ug = ug;

        // Glottal excitation with studio presence tilt + breath-coupled aspiration
        let raw = (ug - self.ug_prev) * self.fs;
        self.ug_prev = ug;
        self.glot_lp += 0.14 * (raw - self.glot_lp);
        let mut x = raw + 0.16 * (raw - self.glot_lp);
        let wa = self.asp_band.tick(&mut self.rng);
        if ag > CLOSED {
            x += 3.0e-6 * turbulence(reynolds(ug, ag)) * wa * (2.0 * self.scale / RHO) * 0.08;
        }

        // Hybrid cascade + MLP-weighted parallel presence branch
        let mut y = x;
        for r in self.res.iter_mut() {
            y = r.tick(y);
        }
        let y_par = self.w_clarity.x * self.par_f2.tick(x) + self.w_singer.x * self.singer.tick(x);
        y = y * 0.88 + y_par;

        let wn = self.w_nasal.x;
        let yn = self.nasal.tick(x);
        y = y * (1.0 - 0.5 * wn) + yn * wn;

        self.bar += 0.03 * (x - self.bar);
        let w = self.w_open.x;
        let voiced = y * w + self.bar * 0.015 * (1.0 - w);

        // Frication & stop bursts
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
        let _ = RC;
        voiced + fr
    }
}
