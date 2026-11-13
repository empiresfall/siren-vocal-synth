//! Extra vocal-fold sources sounding with the lead through the same airway: harmony voices at
//! pitch ratios to the lead, and the golden ensemble (two unison voices a golden comma apart).
//! Each adds only its vibrating area, so the lead's breathiness and leak stay what they were, and
//! each has its own jitter, drift and vibrato phase. With none active the lead is untouched.

use super::glottis::{GlottisParams, ParametricGlottis};
use crate::math::{js_max, pow};

/// Harmony voices the performer can add.
pub const MAX_HARMONY: usize = 6;
/// The golden ratio.
pub const PHI: f64 = 1.618_033_988_749_895;
/// The ensemble's detune (cents): the golden interval (log2 φ octaves, 833.09 cents) over 100.
pub const GOLDEN_CENTS: f64 = 8.330_902_962_915_1;

const SLOTS: usize = MAX_HARMONY + 2;

#[derive(Clone, Debug)]
pub struct Harmony {
    voices: Vec<ParametricGlottis>,
    ratios: [f64; SLOTS],
    gains: [f64; SLOTS],
    /// harmony voices in use (slots 0..n_harmony)
    n_harmony: usize,
    /// golden amount 0..1 (slots MAX_HARMONY, MAX_HARMONY + 1 when > 0)
    golden: f64,
    /// active slot indices
    active: [usize; SLOTS],
    n_active: usize,
}

impl Harmony {
    /// `fs`: the rate the lead's glottis runs at.
    pub fn new(fs: f64) -> Self {
        let voices = (0..SLOTS)
            .map(|i| {
                let mut g = ParametricGlottis::new(fs);
                g.decorrelate(0x1000 + 7919 * i as u32);
                g
            })
            .collect();
        Harmony { voices, ratios: [1.0; SLOTS], gains: [0.0; SLOTS], n_harmony: 0, golden: 0.0, active: [0; SLOTS], n_active: 0 }
    }

    /// Harmony voices at these pitch ratios to the lead (at most [`MAX_HARMONY`]; none: off).
    pub fn set_ratios(&mut self, ratios: &[f64], gain: f64) {
        self.n_harmony = ratios.len().min(MAX_HARMONY);
        for (i, r) in ratios.iter().take(MAX_HARMONY).enumerate() {
            self.ratios[i] = if r.is_finite() { r.clamp(0.125, 8.0) } else { 1.0 };
            self.gains[i] = gain;
        }
        self.rebuild();
    }

    /// The golden ensemble, 0 (off) … 1: two unison voices [`GOLDEN_CENTS`] above and below the
    /// lead, each 1/φ of its size at full amount.
    pub fn set_golden(&mut self, amount: f64) {
        self.golden = if amount.is_finite() { amount.clamp(0.0, 1.0) } else { 0.0 };
        let r = pow(2.0, GOLDEN_CENTS * self.golden / 1200.0);
        self.ratios[MAX_HARMONY] = r;
        self.ratios[MAX_HARMONY + 1] = 1.0 / r;
        self.gains[MAX_HARMONY] = self.golden / PHI;
        self.gains[MAX_HARMONY + 1] = self.golden / PHI;
        self.rebuild();
    }

    fn rebuild(&mut self) {
        self.n_active = 0;
        for i in 0..self.n_harmony {
            self.active[self.n_active] = i;
            self.n_active += 1;
        }
        if self.golden > 0.0 {
            for i in [MAX_HARMONY, MAX_HARMONY + 1] {
                self.active[self.n_active] = i;
                self.n_active += 1;
            }
        }
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.n_active > 0
    }

    pub fn ratios(&self) -> &[f64] {
        &self.ratios[..self.n_harmony]
    }

    pub fn golden(&self) -> f64 {
        self.golden
    }

    /// The extra voices' vibrating area this sample (cm²). They follow the lead's settings
    /// (`lead`, read after the lead's own `next`) at their own pitch, under the same pressure.
    #[inline]
    pub fn next(&mut self, lead: &GlottisParams, dp: f64) -> f64 {
        let mut a = 0.0;
        for k in 0..self.n_active {
            let i = self.active[k];
            let g = &mut self.voices[i];
            g.params = *lead;
            g.params.f0 = lead.f0 * self.ratios[i];
            let total = g.next(dp);
            a += self.gains[i] * js_max(0.0, total - g.statics());
        }
        a
    }
}
