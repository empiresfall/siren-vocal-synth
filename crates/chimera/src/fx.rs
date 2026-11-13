//! The effects rack: eight effects in a fixed order and a limiter, mono in, stereo out. Each
//! effect has an on/off switch and four controls, 0 … 1, so a screen of four encoders plays it.
//!
//! | effect | what it is | controls |
//! |---|---|---|
//! | punch  | low-pass and a slow-attack compressor: transients through, body up | punch, snap, tone, drive |
//! | nitro  | resonant low- and high-cut filters, the low-pass opened by the level | low cut, high cut, resonance, follow |
//! | phone  | band-pass, sample-rate and bit reduction | tone, rate, bits, mix |
//! | cow    | a delay whose echoes are shifted in frequency each time round (they moo) | time, feedback, shift, mix |
//! | delay  | a ping-pong echo | time, feedback, spread, mix |
//! | grid   | a small lattice of short echoes feeding each other: a metal plate | size, feedback, shape, mix |
//! | spring | a dispersive loop: the drip of a spring tank | length, decay, tone, mix |
//! | golden | a reverb whose eight delays stand in golden ratio, so their echoes never coincide | size, decay, damp, mix |
//!
//! Like the voices it is deterministic, allocates nothing while processing, and uses only
//! [`crate::math`].

use crate::math::{PI, TAU, clamp, cos, exp, pow, sin, tan, tanh};
use crate::sources::harmony::PHI;

pub const N_FX: usize = 8;
pub const N_FX_PARAMS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fx {
    Punch,
    Nitro,
    Phone,
    Cow,
    Delay,
    Grid,
    Spring,
    Golden,
}

/// An effect's name, its controls' names and where they start.
pub struct FxInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub about: &'static str,
    pub params: [&'static str; N_FX_PARAMS],
    pub defaults: [f64; N_FX_PARAMS],
}

pub const FX_INFO: [FxInfo; N_FX] = [
    FxInfo { id: "punch", name: "Punch", about: "low-pass and a slow-attack compressor", params: ["punch", "snap", "tone", "drive"], defaults: [0.6, 0.4, 0.85, 0.25] },
    FxInfo { id: "nitro", name: "Nitro", about: "resonant filters that follow the level", params: ["low cut", "high cut", "reso", "follow"], defaults: [0.1, 0.7, 0.45, 0.35] },
    FxInfo { id: "phone", name: "Phone", about: "band-pass and bit crush", params: ["tone", "rate", "bits", "mix"], defaults: [0.5, 0.45, 0.5, 1.0] },
    FxInfo { id: "cow", name: "Cow", about: "echoes that slide in pitch: they moo", params: ["time", "feedback", "shift", "mix"], defaults: [0.35, 0.6, 0.38, 0.5] },
    FxInfo { id: "delay", name: "Delay", about: "ping-pong echo", params: ["time", "feedback", "spread", "mix"], defaults: [0.4, 0.45, 0.8, 0.35] },
    FxInfo { id: "grid", name: "Grid", about: "a lattice of short echoes: a metal plate", params: ["size", "feedback", "shape", "mix"], defaults: [0.4, 0.6, 0.5, 0.4] },
    FxInfo { id: "spring", name: "Spring", about: "the drip of a spring tank", params: ["length", "decay", "tone", "mix"], defaults: [0.5, 0.55, 0.6, 0.4] },
    FxInfo { id: "golden", name: "Golden", about: "reverb with delays in golden ratio", params: ["size", "decay", "damp", "mix"], defaults: [0.55, 0.6, 0.4, 0.3] },
];

impl Fx {
    pub const ALL: [Fx; N_FX] = [Fx::Punch, Fx::Nitro, Fx::Phone, Fx::Cow, Fx::Delay, Fx::Grid, Fx::Spring, Fx::Golden];
    pub fn index(self) -> usize {
        self as usize
    }
    pub fn from_index(i: usize) -> Option<Fx> {
        Self::ALL.get(i).copied()
    }
    pub fn info(self) -> &'static FxInfo {
        &FX_INFO[self.index()]
    }
    pub fn parse(id: &str) -> Option<Fx> {
        Self::ALL.into_iter().find(|f| f.info().id == id)
    }
}

// ---------------------------------------------------------------------------- building blocks

/// A delay line read at a fractional distance (linear interpolation).
#[derive(Clone, Debug)]
struct Line {
    buf: Vec<f64>,
    pos: usize,
}

impl Line {
    fn new(max_samples: usize) -> Self {
        Line { buf: vec![0.0; max_samples.max(4)], pos: 0 }
    }
    #[inline]
    fn write(&mut self, x: f64) {
        self.buf[self.pos] = x;
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
    }
    /// The sample written `d` samples ago (1 ≤ d ≤ len − 2).
    #[inline]
    fn read(&self, d: f64) -> f64 {
        let n = self.buf.len();
        let d = if d < 1.0 { 1.0 } else if d > (n - 2) as f64 { (n - 2) as f64 } else { d };
        let i = d as usize;
        let f = d - i as f64;
        let a = (self.pos + n - i) % n;
        let b = (self.pos + n - i - 1) % n;
        self.buf[a] + (self.buf[b] - self.buf[a]) * f
    }
    fn clear(&mut self) {
        self.buf.fill(0.0);
    }
}

/// Zero-delay-feedback state-variable filter: low, band and high outputs.
#[derive(Clone, Copy, Debug, Default)]
struct Svf {
    g: f64,
    k: f64,
    s1: f64,
    s2: f64,
}

impl Svf {
    fn set(&mut self, fs: f64, hz: f64, q: f64) {
        self.g = tan(PI * clamp(hz, 10.0, 0.45 * fs) / fs);
        self.k = 1.0 / q;
    }
    /// (low, band, high)
    #[inline]
    fn tick(&mut self, x: f64) -> (f64, f64, f64) {
        let (g, k) = (self.g, self.k);
        let h = (x - (k + g) * self.s1 - self.s2) / (1.0 + g * (g + k));
        let b = g * h + self.s1;
        self.s1 = g * h + b;
        let l = g * b + self.s2;
        self.s2 = g * b + l;
        (l, b, h)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct OnePole {
    a: f64,
    y: f64,
}

impl OnePole {
    fn set(&mut self, fs: f64, hz: f64) {
        self.a = 1.0 - exp(-TAU * clamp(hz, 1.0, 0.49 * fs) / fs);
    }
    #[inline]
    fn tick(&mut self, x: f64) -> f64 {
        self.y += self.a * (x - self.y);
        self.y
    }
}

/// A control that glides to its target (no zipper noise, and delay times bend like tape).
#[derive(Clone, Copy, Debug)]
struct Glide {
    y: f64,
    target: f64,
    a: f64,
}

impl Glide {
    fn new(fs: f64, seconds: f64, x: f64) -> Self {
        Glide { y: x, target: x, a: 1.0 - exp(-1.0 / (seconds * fs)) }
    }
    #[inline]
    fn tick(&mut self) -> f64 {
        self.y += self.a * (self.target - self.y);
        self.y
    }
}

/// A 90° phase splitter (two chains of four all-passes, O. Niemitalo's coefficients): returns a
/// pair whose phases differ by a quarter cycle over nearly the whole band.
#[derive(Clone, Copy, Debug, Default)]
struct Hilbert {
    x1: [[f64; 2]; 4],
    y1: [[f64; 2]; 4],
    x2: [[f64; 2]; 4],
    y2: [[f64; 2]; 4],
    delayed: f64,
}

const HILBERT_A: [f64; 4] = [0.6923878, 0.9360654322959, 0.988_229_522_686, 0.9987488452737];
const HILBERT_B: [f64; 4] = [0.4021921162426, 0.856_171_088_242, 0.9722909545651, 0.9952884791278];

impl Hilbert {
    #[inline]
    fn tick(&mut self, x: f64) -> (f64, f64) {
        let mut a = x;
        for k in 0..4 {
            let y = HILBERT_A[k] * (a + self.y1[k][1]) - self.x1[k][1];
            self.x1[k][1] = self.x1[k][0];
            self.x1[k][0] = a;
            self.y1[k][1] = self.y1[k][0];
            self.y1[k][0] = y;
            a = y;
        }
        let mut b = x;
        for k in 0..4 {
            let y = HILBERT_B[k] * (b + self.y2[k][1]) - self.x2[k][1];
            self.x2[k][1] = self.x2[k][0];
            self.x2[k][0] = b;
            self.y2[k][1] = self.y2[k][0];
            self.y2[k][0] = y;
            b = y;
        }
        let i = self.delayed;
        self.delayed = a;
        (i, b)
    }
}

/// 0 … 1 → a frequency between `lo` and `hi`, evenly in octaves.
#[inline]
fn hz(x: f64, lo: f64, hi: f64) -> f64 {
    lo * pow(hi / lo, clamp(x, 0.0, 1.0))
}

// ---------------------------------------------------------------------------- the effects

#[derive(Clone, Debug)]
struct Punch {
    fs: f64,
    lp: Svf,
    env: f64,
    gain: f64,
    att: f64,
    rel: f64,
    amount: f64,
    drive: f64,
}

impl Punch {
    fn set(&mut self, p: &[f64; 4]) {
        self.amount = p[0];
        // a slow attack lets the front of each sound through before the gain comes down
        self.att = 1.0 - exp(-1.0 / ((0.002 + 0.06 * p[1]) * self.fs));
        self.rel = 1.0 - exp(-1.0 / (0.12 * self.fs));
        self.lp.set(self.fs, hz(p[2], 300.0, 18000.0), 0.8);
        self.drive = p[3];
    }
    #[inline]
    fn tick(&mut self, x: f64) -> f64 {
        let x = self.lp.tick(x).0;
        let a = x.abs();
        self.env += (if a > self.env { 0.5 } else { self.rel }) * (a - self.env);
        // 4:1 … 10:1 above a threshold that falls as the amount rises, with make-up gain
        let thr = 0.4 - 0.36 * self.amount;
        let ratio = 2.0 + 8.0 * self.amount;
        let want = if self.env > thr { pow(self.env / thr, 1.0 / ratio - 1.0) } else { 1.0 };
        self.gain += self.att * (want - self.gain);
        let makeup = 1.0 + 2.2 * self.amount;
        let y = x * self.gain * makeup;
        let d = 1.0 + 5.0 * self.drive;
        tanh(y * d) / tanh(d).max(0.5) * (1.0 - 0.25 * self.drive)
    }
}

#[derive(Clone, Debug)]
struct Nitro {
    fs: f64,
    hp: Svf,
    lp: Svf,
    env: OnePole,
    lp_hz: f64,
    q: f64,
    follow: f64,
    count: u32,
}

impl Nitro {
    fn set(&mut self, p: &[f64; 4]) {
        self.q = 0.6 + 9.0 * p[2] * p[2];
        self.hp.set(self.fs, hz(p[0], 20.0, 4000.0), self.q);
        self.lp_hz = hz(p[1], 120.0, 18000.0);
        self.follow = p[3];
        self.env.set(self.fs, 18.0);
    }
    #[inline]
    fn tick(&mut self, x: f64) -> f64 {
        let e = self.env.tick(x.abs());
        if self.count == 0 {
            // the level opens the low-pass by up to four octaves (retuned every 16 samples)
            self.lp.set(self.fs, self.lp_hz * pow(2.0, 4.0 * self.follow * clamp(e * 6.0, 0.0, 1.0)), self.q);
        }
        self.count = (self.count + 1) & 15;
        let y = self.hp.tick(x).2;
        tanh(self.lp.tick(y).0)
    }
}

#[derive(Clone, Debug)]
struct Phone {
    fs: f64,
    bp1: Svf,
    bp2: Svf,
    hold: f64,
    phase: f64,
    step: f64,
    levels: f64,
    mix: f64,
}

impl Phone {
    fn set(&mut self, p: &[f64; 4]) {
        let f = hz(p[0], 400.0, 3200.0);
        self.bp1.set(self.fs, f * 0.5, 1.2);
        self.bp2.set(self.fs, f * 1.9, 1.2);
        // the line's sample rate: 32 kHz down to 1.5 kHz
        self.step = hz(1.0 - p[1], 1500.0, 32000.0) / self.fs;
        // 12 bits down to 2
        self.levels = pow(2.0, 12.0 - 10.0 * p[2]) * 0.5;
        self.mix = p[3];
    }
    #[inline]
    fn tick(&mut self, x: f64) -> f64 {
        let y = self.bp2.tick(self.bp1.tick(x).2).0 * 1.6;
        self.phase += self.step;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            let q = tanh(y);
            self.hold = (q * self.levels).round() / self.levels;
        }
        x + (self.hold - x) * self.mix
    }
}

#[derive(Clone, Debug)]
struct Cow {
    fs: f64,
    line: Line,
    time: Glide,
    hil: Hilbert,
    osc: f64,
    shift: Glide,
    lp: OnePole,
    dc: f64,
    dc_x: f64,
    feedback: f64,
    mix: f64,
}

impl Cow {
    fn set(&mut self, p: &[f64; 4]) {
        self.time.target = (0.03 + 0.67 * p[0] * p[0]) * self.fs;
        self.feedback = 0.93 * p[1];
        // centre: no shift; left: each echo falls (the moo), right: each rises. Up to ±120 Hz.
        let s = 2.0 * p[2] - 1.0;
        self.shift.target = 120.0 * s * s.abs();
        self.mix = p[3];
        self.lp.set(self.fs, 5200.0);
    }
    /// Returns (left, right): the echo, and its mirror image shifted the other way.
    #[inline]
    fn tick(&mut self, x: f64) -> (f64, f64) {
        let d = self.line.read(self.time.tick());
        let (i, q) = self.hil.tick(d);
        self.osc += self.shift.tick() / self.fs;
        self.osc -= self.osc.floor();
        let (c, s) = (cos(TAU * self.osc), sin(TAU * self.osc));
        // single-sideband: the echo moved by the shift, and its mirror image moved the other way
        let up = i * c + q * s;
        let down = i * c - q * s;
        // round the loop: shifted, darker each time, no DC
        let f = self.lp.tick(up);
        let hp = f - self.dc_x + 0.9985 * self.dc;
        self.dc_x = f;
        self.dc = hp;
        self.line.write(x + tanh(hp * self.feedback));
        (x + self.mix * (up - x * 0.3), x + self.mix * (down - x * 0.3))
    }
}

#[derive(Clone, Debug)]
struct PingPong {
    fs: f64,
    left: Line,
    right: Line,
    time: Glide,
    lp_l: OnePole,
    lp_r: OnePole,
    feedback: f64,
    spread: f64,
    mix: f64,
}

impl PingPong {
    fn set(&mut self, p: &[f64; 4]) {
        self.time.target = (0.02 + 0.98 * p[0] * p[0]) * self.fs;
        self.feedback = 0.92 * p[1];
        self.spread = p[2];
        self.mix = p[3];
        self.lp_l.set(self.fs, 4800.0);
        self.lp_r.set(self.fs, 4800.0);
    }
    #[inline]
    fn tick(&mut self, xl: f64, xr: f64) -> (f64, f64) {
        let t = self.time.tick();
        let dl = self.lp_l.tick(self.left.read(t));
        let dr = self.lp_r.tick(self.right.read(t));
        let m = 0.5 * (xl + xr);
        // spread 0: two equal echoes; 1: the echo bounces left, right, left
        let s = self.spread;
        self.left.write(m + self.feedback * (dr * s + dl * (1.0 - s)));
        self.right.write(m * (1.0 - s) + self.feedback * (dl * s + dr * (1.0 - s)));
        (xl + self.mix * dl, xr + self.mix * dr)
    }
}

const GRID_N: usize = 6;
/// Delay lengths of the lattice at full size (ms): mutually prime-ish, 7 … 43 ms.
const GRID_MS: [f64; GRID_N] = [7.3, 11.9, 17.1, 23.3, 31.7, 43.1];

#[derive(Clone, Debug)]
struct Grid {
    fs: f64,
    lines: Vec<Line>,
    len: [f64; GRID_N],
    lp: [OnePole; GRID_N],
    feedback: f64,
    mix: f64,
}

impl Grid {
    fn set(&mut self, p: &[f64; 4]) {
        let size = 0.15 + 0.85 * p[0];
        for k in 0..GRID_N {
            self.len[k] = GRID_MS[k] * 0.001 * size * self.fs;
            self.lp[k].set(self.fs, 2500.0 + 9000.0 * p[2]);
        }
        self.feedback = 0.97 * p[1];
        self.mix = p[3];
    }
    #[inline]
    fn tick(&mut self, xl: f64, xr: f64) -> (f64, f64) {
        let mut d = [0.0; GRID_N];
        let mut sum = 0.0;
        for k in 0..GRID_N {
            d[k] = self.lp[k].tick(self.lines[k].read(self.len[k]));
            sum += d[k];
        }
        // each line hears the others, turned over (a Householder reflection: lossless)
        let h = (2.0 / GRID_N as f64) * sum;
        let m = 0.5 * (xl + xr);
        for k in 0..GRID_N {
            let inp = if k % 2 == 0 { xl } else { xr };
            self.lines[k].write(0.6 * inp + 0.4 * m + self.feedback * (d[k] - h));
        }
        let wl = d[0] - d[3] + d[4];
        let wr = d[1] - d[2] + d[5];
        (xl + self.mix * 0.6 * wl, xr + self.mix * 0.6 * wr)
    }
}

const SPRING_AP: usize = 14;

/// One spring: a loop of a delay and a chain of all-passes that spread a click into a chirp.
#[derive(Clone, Debug)]
struct Coil {
    line: Line,
    ap_x: [f64; SPRING_AP],
    ap_y: [f64; SPRING_AP],
    lp: OnePole,
    len: f64,
    last: f64,
}

impl Coil {
    #[inline]
    fn tick(&mut self, x: f64, coef: f64, feedback: f64) -> f64 {
        let mut v = x + feedback * self.last;
        for k in 0..SPRING_AP {
            let y = coef * (v - self.ap_y[k]) + self.ap_x[k];
            self.ap_x[k] = v;
            self.ap_y[k] = y;
            v = y;
        }
        self.line.write(v);
        self.last = self.lp.tick(self.line.read(self.len));
        self.last
    }
}

#[derive(Clone, Debug)]
struct Spring {
    fs: f64,
    a: Coil,
    b: Coil,
    coef: f64,
    feedback: f64,
    mix: f64,
}

impl Spring {
    fn set(&mut self, p: &[f64; 4]) {
        let len = 0.018 + 0.05 * p[0];
        self.a.len = len * self.fs;
        self.b.len = len * 1.0 / PHI * 1.37 * self.fs;
        self.coef = 0.45 + 0.3 * p[0];
        self.feedback = -(0.5 + 0.47 * p[1]);
        let tone = hz(p[2], 1200.0, 9000.0);
        self.a.lp.set(self.fs, tone);
        self.b.lp.set(self.fs, tone * 0.9);
        self.mix = p[3];
    }
    #[inline]
    fn tick(&mut self, xl: f64, xr: f64) -> (f64, f64) {
        let m = 0.5 * (xl + xr);
        let a = self.a.tick(m, self.coef, self.feedback);
        let b = self.b.tick(m, self.coef, self.feedback);
        (xl + self.mix * (a + 0.3 * b), xr + self.mix * (b + 0.3 * a))
    }
}

const GOLD_N: usize = 8;

/// Eight delays whose lengths rise by √φ from one to the next (so the longest is φ^3.5 of the
/// shortest, and no two share a period), fed back through a Householder reflection.
#[derive(Clone, Debug)]
struct Golden {
    fs: f64,
    lines: Vec<Line>,
    len: [f64; GOLD_N],
    lp: [OnePole; GOLD_N],
    pre: Line,
    ap: [Line; 2],
    ap_len: [f64; 2],
    size: Glide,
    gain: [f64; GOLD_N],
    decay: f64,
    mix: f64,
}

impl Golden {
    fn set(&mut self, p: &[f64; 4]) {
        self.size.target = 0.25 + 0.75 * p[0];
        // decay time 0.3 … 12 s
        self.decay = 0.3 * pow(40.0, p[1]);
        let damp = hz(1.0 - p[2], 1200.0, 16000.0);
        for k in 0..GOLD_N {
            self.lp[k].set(self.fs, damp);
        }
        self.mix = p[3];
    }
    fn retune(&mut self) {
        let size = self.size.y;
        let base = 0.0113 * size * self.fs;
        for k in 0..GOLD_N {
            self.len[k] = base * pow(PHI, 0.5 * k as f64);
            // each line loses what gives −60 dB after `decay` seconds
            self.gain[k] = pow(10.0, -3.0 * (self.len[k] / self.fs) / self.decay);
        }
        self.ap_len = [0.0047 * size * self.fs, 0.0047 * PHI * size * self.fs];
    }
    #[inline]
    fn tick(&mut self, xl: f64, xr: f64, n: u32) -> (f64, f64) {
        self.size.tick();
        if n & 31 == 0 {
            self.retune();
        }
        let m = 0.5 * (xl + xr);
        self.pre.write(m);
        // two all-passes smear the input before it enters the loop
        let mut v = self.pre.read(0.008 * self.fs);
        for k in 0..2 {
            let d = self.ap[k].read(self.ap_len[k]);
            let w = v + 0.6 * d;
            self.ap[k].write(w);
            v = d - 0.6 * w;
        }
        let mut d = [0.0; GOLD_N];
        let mut sum = 0.0;
        for k in 0..GOLD_N {
            d[k] = self.lp[k].tick(self.lines[k].read(self.len[k])) * self.gain[k];
            sum += d[k];
        }
        let h = (2.0 / GOLD_N as f64) * sum;
        for k in 0..GOLD_N {
            self.lines[k].write(v + d[k] - h);
        }
        let wl = d[0] - d[2] + d[4] - d[6];
        let wr = d[1] - d[3] + d[5] - d[7];
        (xl + self.mix * 0.5 * wl, xr + self.mix * 0.5 * wr)
    }
}

// ---------------------------------------------------------------------------- the rack

#[derive(Clone, Debug)]
pub struct FxChain {
    pub fs: f64,
    on: [bool; N_FX],
    /// 1 while on, gliding to 0 when switched off (and back), so switching never clicks
    wet: [Glide; N_FX],
    params: [[f64; N_FX_PARAMS]; N_FX],
    punch: Punch,
    nitro: Nitro,
    phone: Phone,
    cow: Cow,
    delay: PingPong,
    grid: Grid,
    spring: Spring,
    golden: Golden,
    lim_env: f64,
    lim_rel: f64,
    n: u32,
}

impl FxChain {
    pub fn new(fs: f64) -> Self {
        let s = |sec: f64| (sec * fs) as usize + 8;
        let mut c = FxChain {
            fs,
            on: [false; N_FX],
            wet: [Glide::new(fs, 0.012, 0.0); N_FX],
            params: FX_INFO.each_ref().map(|i| i.defaults),
            punch: Punch { fs, lp: Svf::default(), env: 0.0, gain: 1.0, att: 0.0, rel: 0.0, amount: 0.0, drive: 0.0 },
            nitro: Nitro { fs, hp: Svf::default(), lp: Svf::default(), env: OnePole::default(), lp_hz: 8000.0, q: 1.0, follow: 0.0, count: 0 },
            phone: Phone { fs, bp1: Svf::default(), bp2: Svf::default(), hold: 0.0, phase: 0.0, step: 1.0, levels: 1024.0, mix: 1.0 },
            cow: Cow { fs, line: Line::new(s(0.75)), time: Glide::new(fs, 0.08, 0.2 * fs), hil: Hilbert::default(), osc: 0.0, shift: Glide::new(fs, 0.05, 0.0), lp: OnePole::default(), dc: 0.0, dc_x: 0.0, feedback: 0.0, mix: 0.0 },
            delay: PingPong { fs, left: Line::new(s(1.05)), right: Line::new(s(1.05)), time: Glide::new(fs, 0.1, 0.3 * fs), lp_l: OnePole::default(), lp_r: OnePole::default(), feedback: 0.0, spread: 1.0, mix: 0.0 },
            grid: Grid { fs, lines: (0..GRID_N).map(|k| Line::new(s(GRID_MS[k] * 0.001))).collect(), len: [8.0; GRID_N], lp: [OnePole::default(); GRID_N], feedback: 0.0, mix: 0.0 },
            spring: Spring {
                fs,
                a: Coil { line: Line::new(s(0.08)), ap_x: [0.0; SPRING_AP], ap_y: [0.0; SPRING_AP], lp: OnePole::default(), len: 8.0, last: 0.0 },
                b: Coil { line: Line::new(s(0.08)), ap_x: [0.0; SPRING_AP], ap_y: [0.0; SPRING_AP], lp: OnePole::default(), len: 8.0, last: 0.0 },
                coef: 0.6,
                feedback: -0.7,
                mix: 0.0,
            },
            golden: Golden {
                fs,
                lines: (0..GOLD_N).map(|k| Line::new(s(0.0113 * pow(PHI, 0.5 * k as f64)))).collect(),
                len: [8.0; GOLD_N],
                lp: [OnePole::default(); GOLD_N],
                pre: Line::new(s(0.01)),
                ap: [Line::new(s(0.006)), Line::new(s(0.009))],
                ap_len: [8.0; 2],
                size: Glide::new(fs, 0.15, 0.6),
                gain: [0.0; GOLD_N],
                decay: 2.0,
                mix: 0.0,
            },
            lim_env: 0.0,
            lim_rel: exp(-1.0 / (0.15 * fs)),
            n: 0,
        };
        for f in Fx::ALL {
            c.apply(f);
        }
        c.golden.size.y = c.golden.size.target;
        c.golden.retune();
        c.cow.time.y = c.cow.time.target;
        c.delay.time.y = c.delay.time.target;
        c
    }

    pub fn set_on(&mut self, fx: Fx, on: bool) {
        let i = fx.index();
        if on && !self.on[i] && self.wet[i].y < 1e-4 {
            self.clear(fx); // no tail from the last time it was on
        }
        self.on[i] = on;
        self.wet[i].target = if on { 1.0 } else { 0.0 };
    }

    pub fn is_on(&self, fx: Fx) -> bool {
        self.on[fx.index()]
    }

    /// One control, 0 … 1.
    pub fn set_param(&mut self, fx: Fx, index: usize, value: f64) {
        if index < N_FX_PARAMS && value.is_finite() {
            self.params[fx.index()][index] = clamp(value, 0.0, 1.0);
            self.apply(fx);
        }
    }

    pub fn param(&self, fx: Fx, index: usize) -> f64 {
        self.params[fx.index()][index.min(N_FX_PARAMS - 1)]
    }

    fn apply(&mut self, fx: Fx) {
        let p = self.params[fx.index()];
        match fx {
            Fx::Punch => self.punch.set(&p),
            Fx::Nitro => self.nitro.set(&p),
            Fx::Phone => self.phone.set(&p),
            Fx::Cow => self.cow.set(&p),
            Fx::Delay => self.delay.set(&p),
            Fx::Grid => self.grid.set(&p),
            Fx::Spring => self.spring.set(&p),
            Fx::Golden => self.golden.set(&p),
        }
    }

    fn clear(&mut self, fx: Fx) {
        match fx {
            Fx::Cow => self.cow.line.clear(),
            Fx::Delay => {
                self.delay.left.clear();
                self.delay.right.clear();
            }
            Fx::Grid => self.grid.lines.iter_mut().for_each(Line::clear),
            Fx::Spring => {
                for c in [&mut self.spring.a, &mut self.spring.b] {
                    c.line.clear();
                    c.last = 0.0;
                    c.ap_x = [0.0; SPRING_AP];
                    c.ap_y = [0.0; SPRING_AP];
                }
            }
            Fx::Golden => {
                self.golden.lines.iter_mut().for_each(Line::clear);
                self.golden.pre.clear();
                self.golden.ap.iter_mut().for_each(Line::clear);
            }
            _ => {}
        }
    }

    fn reset_filters(&mut self) {
        for f in [&mut self.punch.lp, &mut self.nitro.hp, &mut self.nitro.lp, &mut self.phone.bp1, &mut self.phone.bp2] {
            f.s1 = 0.0;
            f.s2 = 0.0;
        }
        (self.punch.env, self.punch.gain, self.phone.hold) = (0.0, 1.0, 0.0);
        self.nitro.env.y = 0.0;
        self.cow.hil = Hilbert::default();
        (self.cow.lp.y, self.cow.dc, self.cow.dc_x) = (0.0, 0.0, 0.0);
        (self.delay.lp_l.y, self.delay.lp_r.y) = (0.0, 0.0);
        self.grid.lp.iter_mut().for_each(|f| f.y = 0.0);
        self.golden.lp.iter_mut().for_each(|f| f.y = 0.0);
        (self.spring.a.lp.y, self.spring.b.lp.y) = (0.0, 0.0);
        self.lim_env = 0.0;
    }

    /// True when nothing is on and every tail has faded: `process` would copy its input.
    pub fn is_idle(&self) -> bool {
        self.on.iter().all(|x| !x) && self.wet.iter().all(|w| w.y < 1e-4)
    }

    /// Mono in, stereo out. The three slices have the same length.
    pub fn process(&mut self, input: &[f32], left: &mut [f32], right: &mut [f32]) {
        for ((x, ol), or) in input.iter().zip(left.iter_mut()).zip(right.iter_mut()) {
            self.n = self.n.wrapping_add(1);
            let mut l = *x as f64;
            // the mono effects, each cross-faded in by its switch
            let w = self.wet[0].tick();
            if w > 1e-4 {
                l += w * (self.punch.tick(l) - l);
            }
            let w = self.wet[1].tick();
            if w > 1e-4 {
                l += w * (self.nitro.tick(l) - l);
            }
            let w = self.wet[2].tick();
            if w > 1e-4 {
                l += w * (self.phone.tick(l) - l);
            }
            let mut r = l;
            let w = self.wet[3].tick();
            if w > 1e-4 {
                let (a, b) = self.cow.tick(l);
                r = l + w * (b - l);
                l += w * (a - l);
            }
            let w = self.wet[4].tick();
            if w > 1e-4 {
                let (a, b) = self.delay.tick(l, r);
                l += w * (a - l);
                r += w * (b - r);
            }
            let w = self.wet[5].tick();
            if w > 1e-4 {
                let (a, b) = self.grid.tick(l, r);
                l += w * (a - l);
                r += w * (b - r);
            }
            let w = self.wet[6].tick();
            if w > 1e-4 {
                let (a, b) = self.spring.tick(l, r);
                l += w * (a - l);
                r += w * (b - r);
            }
            let w = self.wet[7].tick();
            if w > 1e-4 {
                let (a, b) = self.golden.tick(l, r, self.n);
                l += w * (a - l);
                r += w * (b - r);
            }
            // limiter: the louder channel's peak sets one gain for both
            let pk = l.abs().max(r.abs());
            if pk.is_finite() {
                self.lim_env = if pk > self.lim_env { pk } else { pk + (self.lim_env - pk) * self.lim_rel };
            } else {
                // a filter blew up (it should not): silence rather than a bang, and start clean
                l = 0.0;
                r = 0.0;
                for f in Fx::ALL {
                    self.clear(f);
                }
                self.reset_filters();
            }
            if self.lim_env > 0.89 {
                let g = 0.89 / self.lim_env;
                l *= g;
                r *= g;
            }
            *ol = l as f32;
            *or = r as f32;
        }
    }
}
