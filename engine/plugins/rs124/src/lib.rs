//! RS124 — after the EMI RS124, Abbey Road's rebuilt Altec 436B.
//!
//! A variable-mu feedback compressor. The gain stage is a push-pull pair of
//! remote-cutoff triodes (6BC8); the 6AL5 detector rectifies the *output*
//! and pushes their bias further negative, which lowers the gain. Because
//! it's a feedback design with a fixed threshold, there's no ratio or
//! threshold control: the INPUT switch sets how hard it works, and the
//! ratio settles around 2.5:1 with a soft knee (Sound On Sound's
//! measurements of the original). Up to ~30 dB of reduction.
//!
//! EMI's changes from the 436B are here: stepped input and output switches,
//! the six-position RECOVERY switch (about 0.5 to 1.5 s, with HOLD between
//! the numbered positions), and the BALANCE trim for the push-pull stage
//! (unbalanced, the control voltage leaks through as a thump). The
//! original had no attack control and attack varied from unit to unit; the
//! UNIT switch picks a fast, typical or slow example.
//!
//! On Beatles sessions it was the guitar and bass compressor, often used
//! hard (20-30 dB) and in several passes on bass.

use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
use vintage::dsp::*;
use vintage::nih_plug_egui::EguiState;
use vintage::params::*;
use vintage::ui::{self, Item, MeterKind, Panel, Preset};
use vintage::{VintageDsp, VintageParams};

const W: u32 = 640;
const H: u32 = 432;

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Unit {
    #[name = "A (fast)"]
    A,
    #[name = "B (typical)"]
    B,
    #[name = "C (slow)"]
    C,
}

#[derive(Params)]
pub struct RsParams {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "input"]
    input: FloatParam,
    #[id = "output"]
    output: FloatParam,
    #[id = "recovery"]
    recovery: IntParam,
    #[id = "unit"]
    unit: EnumParam<Unit>,
    #[id = "balance"]
    balance: FloatParam,
    #[id = "mix"]
    mix: FloatParam,
}

const RECOVERY_NAMES: [&str; 12] = ["1", "Hold", "2", "Hold", "3", "Hold", "4", "Hold", "5", "Hold", "6", "Hold"];
/// Recovery (release) time constants for positions 1..6 (s).
const RECOVERY_S: [f64; 6] = [0.5, 0.7, 0.9, 1.1, 1.3, 1.5];
/// Attack time constants for the three example units (s).
const ATTACK_S: [f64; 3] = [0.020, 0.040, 0.070];

impl Default for RsParams {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            input: FloatParam::new("Input", 8.0, FloatRange::Linear { min: -6.0, max: 36.0 })
                .with_step_size(2.0)
                .with_unit(" dB"),
            output: FloatParam::new("Output", 0.0, FloatRange::Linear { min: -24.0, max: 12.0 })
                .with_step_size(2.0)
                .with_unit(" dB"),
            recovery: IntParam::new("Recovery", 2, IntRange::Linear { min: 0, max: 11 })
                .with_value_to_string(Arc::new(|v| RECOVERY_NAMES[v.clamp(0, 11) as usize].to_string())),
            unit: EnumParam::new("Unit (attack)", Unit::B),
            balance: pct("Balance Error", 0.0),
            mix: pct("Mix", 1.0),
        }
    }
}

const INPUT: usize = 0;
const OUTPUT: usize = 1;
const RECOVERY: usize = 2;
const UNIT: usize = 3;
const BALANCE: usize = 4;
const MIX: usize = 5;

impl VintageParams for RsParams {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.input.as_ptr(),
            self.output.as_ptr(),
            self.recovery.as_ptr(),
            self.unit.as_ptr(),
            self.balance.as_ptr(),
            self.mix.as_ptr(),
        ]
    }
}

// -------------------------------------------------------------------- dsp --

/// Output peak level (full scale = 1.0) where compression begins: about
/// +4 dBu with 0 dBu = -18 dBFS.
const THRESHOLD: f64 = 0.2;
/// Scale of the remote-cutoff triodes' exponential curve (full-scale units
/// at the grid): sets the push-pull stage's third-harmonic distortion,
/// about 1% at 25 dB of reduction as on the 436 spec sheet.
const VK: f64 = 6.0;
const MAX_GR_DB: f64 = 30.0;

pub struct RsDsp {
    fs: f64,
    cv: f64,
    gain: f64,
    att: f64,
    rel: f64,
    hold: bool,
    in_hp: [Biquad; 2],
    out_hp: [Biquad; 2],
    lp: [Biquad; 2],
    in_gain: Smooth,
    out_gain: Smooth,
    mix: Smooth,
    balance: f64,
    fb: f64,
}

/// Feedback detector: output level above the threshold, with a +/-3 dB
/// soft knee, in units of the threshold.
#[inline]
fn over(peak: f64) -> f64 {
    let r = peak / THRESHOLD;
    let (lo, hi) = (0.708, 1.413);
    if r <= lo {
        0.0
    } else if r < hi {
        // quadratic blend into the straight line c = r - 1
        let t = (r - lo) / (hi - lo);
        (hi - 1.0) * t * t
    } else {
        r - 1.0
    }
}

/// Gain of the variable-mu stage for a control voltage (in threshold
/// units). (1 + c)^-1.5 with a feedback detector gives a steady 2.5:1.
#[inline]
fn vari_mu_gain(cv: f64) -> f64 {
    (1.0 + cv).powf(-1.5).max(db_to_gain(-MAX_GR_DB))
}

impl RsDsp {
    pub fn gain_reduction_db(&self) -> f64 {
        -gain_to_db(self.gain)
    }
}

impl VintageDsp for RsDsp {
    const METERS: usize = 1;

    fn new(sample_rate: f32, _channels: usize) -> Self {
        let fs = sample_rate as f64;
        let mut in_hp = [Biquad::identity(), Biquad::identity()];
        let mut out_hp = [Biquad::identity(), Biquad::identity()];
        let mut lp = [Biquad::identity(), Biquad::identity()];
        for c in 0..2 {
            // transformers: response falls away below ~80 Hz
            in_hp[c].set_highpass(fs, 55.0, 0.6);
            out_hp[c].set_highpass(fs, 25.0, 0.7);
            lp[c].set_lowpass(fs, 21000.0f64.min(fs * 0.45), 0.7);
        }
        Self {
            fs,
            cv: 0.0,
            gain: 1.0,
            att: 0.0,
            rel: 0.0,
            hold: false,
            in_hp,
            out_hp,
            lp,
            in_gain: Smooth::new(fs, 15.0, db_to_gain(8.0)),
            out_gain: Smooth::new(fs, 15.0, 1.0),
            mix: Smooth::new(fs, 20.0, 1.0),
            balance: 0.0,
            fb: 0.0,
        }
    }

    fn reset(&mut self) {
        self.cv = 0.0;
        self.gain = 1.0;
        self.fb = 0.0;
        for b in self.in_hp.iter_mut().chain(self.out_hp.iter_mut()).chain(self.lp.iter_mut()) {
            b.reset();
        }
    }

    fn set_params(&mut self, v: &[f32]) {
        self.in_gain.set(db_to_gain(v[INPUT] as f64));
        self.out_gain.set(db_to_gain(v[OUTPUT] as f64));
        let pos = (v[RECOVERY].round() as usize).min(11);
        self.hold = pos % 2 == 1;
        let rec = RECOVERY_S[pos / 2];
        let unit = (v[UNIT].round() as usize).min(2);
        self.att = 1.0 - (-1.0 / (ATTACK_S[unit] * self.fs)).exp();
        self.rel = 1.0 - (-1.0 / (rec * self.fs)).exp();
        self.balance = v[BALANCE] as f64;
        self.mix.set(v[MIX] as f64);
    }

    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(2);
        let mut max_gr: f64 = 0.0;
        for k in 0..n {
            // the 6AL5 charges the timing cap from the (previous) output
            let target = over(self.fb);
            if target > self.cv {
                self.cv += (target - self.cv) * self.att;
            } else if !self.hold {
                self.cv += (target - self.cv) * self.rel;
            }
            let g = vari_mu_gain(self.cv);
            self.gain = g;
            max_gr = max_gr.max(-gain_to_db(g));
            let gi = self.in_gain.next();
            let go = self.out_gain.next();
            let m = self.mix.next();
            let mut peak: f64 = 0.0;
            for c in 0..nch {
                let x = chans[c][k] as f64;
                let s = self.in_hp[c].process(x * gi);
                // push-pull exponential triodes: sinh, odd harmonics
                let u = (s / VK).clamp(-3.0, 3.0);
                let mut y = g * VK * u.sinh();
                // an unbalanced pair lets the control voltage (and some
                // even harmonics) through
                y += self.balance * (0.25 * (1.0 - g) + 0.08 * g * s * s);
                // 6CG7 output stage running out of swing near +24 dBm
                y = 2.0 * (y * 0.5).tanh();
                y = self.lp[c].process(self.out_hp[c].process(y));
                peak = peak.max(y.abs());
                let out = x + m * (y * go - x);
                chans[c][k] = out as f32;
            }
            self.fb = peak;
            // a mono-in-stereo slot: copy channel 1 if the host gave more
            for c in nch..chans.len() {
                chans[c][k] = chans[0][k];
            }
        }
        meters[0] = max_gr as f32;
    }
}

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Gentle Rhythm (Norman Smith)", values: &[(INPUT, 4.0), (RECOVERY, 4.0), (OUTPUT, 0.0)] },
        Preset { name: "Guitar Squash", values: &[(INPUT, 18.0), (RECOVERY, 2.0), (OUTPUT, -8.0)] },
        Preset { name: "Rain Bass (heavy)", values: &[(INPUT, 24.0), (RECOVERY, 4.0), (UNIT, 2.0), (OUTPUT, -12.0)] },
        Preset { name: "Bass Pass 1 of 2", values: &[(INPUT, 12.0), (RECOVERY, 6.0), (UNIT, 2.0), (OUTPUT, -4.0)] },
        Preset { name: "Acoustic Guitar Level", values: &[(INPUT, 10.0), (RECOVERY, 8.0), (OUTPUT, -2.0)] },
        Preset { name: "Bus / Reduction Mix", values: &[(INPUT, 6.0), (RECOVERY, 10.0), (OUTPUT, 0.0)] },
        Preset { name: "Parallel Crush", values: &[(INPUT, 30.0), (RECOVERY, 0.0), (UNIT, 0.0), (OUTPUT, -14.0), (MIX, 0.4)] },
        Preset { name: "Unbalanced Unit (thump)", values: &[(INPUT, 16.0), (RECOVERY, 0.0), (BALANCE, 0.6), (OUTPUT, -6.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "RS124",
        subtitle: ("AFTER THE EMI RS124", "COMPRESSOR AMPLIFIER · VARIABLE-MU"),
        accent: ui::MUSTARD,
        size: (W, H),
        items: vec![
            Item::Knob { p: INPUT, x: 120.0, y: 172.0, r: 50.0, label: "INPUT",
                tip: "Stepped input. The threshold is fixed, so this is how much it compresses: up = more squash.", active: None },
            Item::Meter { m: 0, x: 222.0, y: 92.0, w: 196.0, h: 136.0, kind: MeterKind::GainReduction, caption: "COMPRESSION dB" },
            Item::Knob { p: OUTPUT, x: 520.0, y: 172.0, r: 50.0, label: "OUTPUT", tip: "Stepped output level.", active: None },
            Item::Divider { y: 266.0 },
            Item::Selector { p: RECOVERY, x: 130.0, y: 330.0, label: "RECOVERY",
                positions: &["1", "H", "2", "H", "3", "H", "4", "H", "5", "H", "6", "H"],
                tip: "Release: 1 (~0.5 s) to 6 (~1.5 s). The positions between (H) hold the gain reduction where it is." },
            Item::Selector { p: UNIT, x: 260.0, y: 330.0, label: "UNIT", positions: &["A", "B", "C"],
                tip: "The original has no attack control and attack varied from unit to unit. A ~20 ms, B ~40 ms, C ~70 ms (slow: lets bass attacks through)." },
            Item::Knob { p: BALANCE, x: 390.0, y: 330.0, r: 22.0, label: "BALANCE",
                tip: "Push-pull balance error. 0 = trimmed (as EMI kept them). Up = the control voltage thumps through on hard attacks, plus some even harmonics.", active: None },
            Item::Knob { p: MIX, x: 510.0, y: 330.0, r: 22.0, label: "MIX", tip: "Parallel compression: blend with the uncompressed signal.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, RsParams, RsDsp, panel,
    name: "RS124", class_id: *b"S2RS124CompDIY01", vendor: "PEQUOD Beatles",
    categories: [Fx, Dynamics]
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn run(level_db: f64, input_db: f32, recovery: f32, f: f64) -> (f64, f64, f64) {
        let fs = 48000.0;
        let mut d = RsDsp::new(fs as f32, 1);
        d.set_params(&[input_db, 0.0, recovery, 1.0, 0.0, 1.0]);
        let n = 96000;
        let a = db_to_gain(level_db);
        let mut x: Vec<f32> = (0..n).map(|k| (a * (2.0 * PI * f * k as f64 / fs).sin()) as f32).collect();
        let mut m = [0.0f32];
        for c in x.chunks_mut(256) {
            d.process(&mut [c], &mut m);
        }
        let tail = &x[n / 2..];
        let (mut re, mut im, mut h3r, mut h3i) = (0.0, 0.0, 0.0, 0.0);
        for (k, v) in tail.iter().enumerate() {
            let ph = 2.0 * PI * f * (k + n / 2) as f64 / fs;
            re += *v as f64 * ph.sin();
            im += *v as f64 * ph.cos();
            h3r += *v as f64 * (3.0 * ph).sin();
            h3i += *v as f64 * (3.0 * ph).cos();
        }
        let fund = (re * re + im * im).sqrt() * 2.0 / tail.len() as f64;
        let h3 = (h3r * h3r + h3i * h3i).sqrt() * 2.0 / tail.len() as f64;
        (gain_to_db(fund), d.gain_reduction_db(), h3 / fund)
    }

    #[test]
    fn ratio_is_about_two_and_a_half_to_one() {
        // above threshold, 10 dB more in gives ~4 dB more out
        let (o1, g1, _) = run(-20.0, 20.0, 0.0, 1000.0);
        let (o2, g2, _) = run(-10.0, 20.0, 0.0, 1000.0);
        let ratio = 10.0 / (o2 - o1);
        println!("out {o1:.1} -> {o2:.1} dB (GR {g1:.1} -> {g2:.1}), ratio {ratio:.2}:1");
        assert!((2.0..3.2).contains(&ratio));
    }

    #[test]
    fn distortion_rises_with_compression_but_stays_low() {
        let (_, gr_light, thd_light) = run(-24.0, 8.0, 0.0, 1000.0);
        let (_, gr_hard, thd_hard) = run(-16.0, 30.0, 0.0, 1000.0);
        println!("GR {gr_light:.1} dB: H3 {:.2}%   GR {gr_hard:.1} dB: H3 {:.2}%", thd_light * 100.0, thd_hard * 100.0);
        assert!(gr_hard > 15.0);
        assert!(thd_hard > thd_light && thd_hard < 0.03);
    }

    #[test]
    fn hold_keeps_the_reduction() {
        let fs = 48000.0;
        for (pos, held) in [(0.0f32, false), (1.0, true)] {
            let mut d = RsDsp::new(fs as f32, 1);
            d.set_params(&[24.0, 0.0, pos, 1.0, 0.0, 1.0]);
            let mut x: Vec<f32> = (0..48000).map(|k| (0.5 * (k as f64 * 0.13).sin()) as f32).collect();
            let mut m = [0.0f32];
            d.process(&mut [&mut x[..]], &mut m);
            let before = d.gain_reduction_db();
            let mut z = vec![0.0f32; 96000];
            d.process(&mut [&mut z[..]], &mut m);
            let after = d.gain_reduction_db();
            println!("pos {pos}: GR {before:.1} -> {after:.1} dB after 2 s silence");
            if held {
                assert!(after > before * 0.95);
            } else {
                assert!(after < 1.0);
            }
        }
    }

    #[test]
    fn slower_unit_lets_attacks_through() {
        let fs = 48000.0;
        let burst = |unit: f32| {
            let mut d = RsDsp::new(fs as f32, 1);
            d.set_params(&[24.0, 0.0, 0.0, unit, 0.0, 1.0]);
            let mut x: Vec<f32> = (0..4800).map(|k| (0.5 * (k as f64 * 0.13).sin()) as f32).collect();
            d.process(&mut [&mut x[..]], &mut [0.0]);
            x[200..1200].iter().fold(0.0f32, |a, v| a.max(v.abs()))
        };
        let (fast, slow) = (burst(0.0), burst(2.0));
        println!("first 25 ms peak: unit A {fast:.3}, unit C {slow:.3}");
        assert!(slow > fast * 1.2);
    }
}
