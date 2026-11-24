//! LA-3A: UREI's solid-state version of the LA-2A (1969), the same T4 optical cell behind transistors
//! instead of valves: quicker, brighter and a little more forward, the classic for guitars and snare. GAIN,
//! PEAK REDUCTION and COMPRESS/LIMIT, as on the front. About 1.5 ms to react; the cell lets go in two steps
//! as the LA-2A's does, a little sooner. Class-A transistor make-up and an output transformer. Curves
//! estimated from the published specifications.

#![allow(clippy::excessive_precision)]
use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
#[allow(unused_imports)]
use vintage::dsp::*;
#[allow(unused_imports)]
use vintage::fx::*;
use vintage::nih_plug_egui::EguiState;
#[allow(unused_imports)]
use vintage::nih_plug_egui::egui::Align2;
#[allow(unused_imports)]
use vintage::params::*;
#[allow(unused_imports)]
use vintage::ui::{self, ActiveWhen, Item, MeterKind, Panel, Preset};
#[allow(unused_imports)]
use vintage::{Midi, VintageDsp, VintageParams};

/// Transformer iron: the lows saturate first (a little thickening, then soft limiting),
/// the top rolls gently away.
#[allow(dead_code)]
#[derive(Clone)]
struct Iron {
    lo: OnePole,
    top: OnePole,
    drive: f64,
}

#[allow(dead_code)]
impl Iron {
    fn new(fs: f64, corner: f64, drive: f64, top: f64) -> Self {
        let mut lo = OnePole::default();
        lo.set_lowpass(fs, corner);
        let mut t = OnePole::default();
        t.set_lowpass(fs, top.min(fs * 0.45));
        Self { lo, top: t, drive }
    }
    fn reset(&mut self) {
        self.lo.reset();
        self.top.reset();
    }
    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let l = self.lo.lp(x);
        let d = self.drive;
        let y = x - l + (l * d).tanh() / d;
        self.top.lp(y)
    }
}

/// A valve gain stage: mostly second harmonic, rising with level.
#[allow(dead_code)]
#[inline]
fn valve(x: f64, warmth: f64) -> f64 {
    let y = x + warmth * 0.18 * x * x - warmth * 0.04 * x * x * x;
    (y * (1.0 + warmth * 0.3)).tanh() / (1.0 + warmth * 0.3)
}

/// The T4 optical cell: an electroluminescent panel lit by the side chain, read by a
/// photoresistor. Quick to react; lets go in two steps (about 60 ms to half, then one to
/// several seconds for the rest), slower the longer it has been lit (its memory).
struct T4 {
    light: f64,
    cell: f64,
    memory: f64,
}

impl T4 {
    fn new() -> Self {
        Self { light: 0.0, cell: 0.0, memory: 0.0 }
    }
    /// `drive`: side-chain level (linear); returns gain reduction (dB).
    #[inline]
    fn process(&mut self, fs: f64, drive: f64, attack_ms: f64, fast_ms: f64, slow_s: f64, limit: bool) -> f64 {
        let ca = coeff(fs, attack_ms);
        self.light = if drive > self.light { drive + ca * (self.light - drive) } else { drive + coeff(fs, fast_ms) * (self.light - drive) };
        // the cell follows the light; its slow part remembers how long it was lit
        let target = gain_to_db(1.0 + self.light * 6.0) * if limit { 1.6 } else { 1.0 };
        self.memory = (self.memory + (if target > 1.0 { 1.0 } else { -0.3 }) / fs).clamp(0.0, 4.0);
        let slow = coeff(fs, (slow_s * (0.4 + 0.6 * self.memory / 4.0)) * 1000.0 / 3.0);
        if target > self.cell {
            self.cell = target + ca * (self.cell - target);
        } else {
            // half of it comes back quickly, the rest slowly
            let quick = target + coeff(fs, fast_ms) * (self.cell - target);
            let lag = target + slow * (self.cell - target);
            self.cell = 0.5 * quick + 0.5 * lag;
        }
        self.cell
    }
}

pub const W: u32 = 640;
pub const H: u32 = 432;

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "peak"]
    pub peak: FloatParam,
    #[id = "limit"]
    pub limit: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            gain: db("Gain", 10.0, 0.0, 40.0),
            peak: pct("Peak Reduction", 0.45),
            limit: FloatParam::new("Compress/Limit", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
        }
    }
}

pub const GAIN: usize = 0;
pub const PEAK: usize = 1;
pub const LIMIT: usize = 2;
pub const NPARAMS: usize = 3;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.gain.as_ptr(),
            self.peak.as_ptr(),
            self.limit.as_ptr(),
        ]
    }
}

/// Default plain values in linked order (for tests).
#[allow(dead_code)]
pub fn defaults() -> Vec<f32> {
    let p = P::default();
    p.linked().iter().map(|q| unsafe { q.preview_plain(q.default_normalized_value()) }).collect()
}

// -------------------------------------------------------------------- dsp --

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    t4: T4,
    iron: Vec<Iron>,
    gain: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let n = ch.max(2);
        Self { fs, v, t4: T4::new(), iron: vec![Iron::new(fs, 35.0, 1.5, 24000.0); n], gain: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.t4 = T4::new();
        self.iron.iter_mut().for_each(|i| i.reset());
        self.gain.snap(db_to_gain(self.v[GAIN] as f64 - 10.0));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.gain.set(db_to_gain(v[GAIN] as f64 - 10.0));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let sc = db_to_gain(-36.0 + 46.0 * self.v[PEAK] as f64);
        let limit = self.v[LIMIT] > 0.5;
        let mut most = 0.0f64;
        for k in 0..n {
            let g = self.gain.next();
            let peak = chans.iter().map(|c| (c[k] as f64).abs()).fold(0.0, f64::max);
            let gr = self.t4.process(self.fs, peak * sc, 1.5, 45.0, 0.9, limit);
            most = most.max(gr);
            let a = db_to_gain(-gr);
            for (ci, ch) in chans.iter_mut().enumerate() {
                let j = ci.min(self.iron.len() - 1);
                let x = ch[k] as f64 * a * g;
                // class-A transistors: a touch of odd harmonics, a harder edge than valves
                let y = x - 0.035 * x * x * x;
                ch[k] = self.iron[j].process(y.clamp(-1.6, 1.6)) as f32;
            }
        }
        meters[0] = most as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Electric Guitar", values: &[(PEAK, 0.55)] },
        Preset { name: "Snare Snap", values: &[(PEAK, 0.65), (LIMIT, 1.0)] },
        Preset { name: "Acoustic Even", values: &[(PEAK, 0.4), (GAIN, 12.0)] },
        Preset { name: "Bass DI", values: &[(PEAK, 0.6), (GAIN, 13.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "LA-3A",
        subtitle: ("UREI AUDIO LEVELER, 1969", "THE T4 OPTICAL CELL, SOLID STATE"),
        accent: ui::GRAPHITE,
        size: (W, H),
        items: vec![
            Item::Knob { p: GAIN, x: 130.0, y: 300.0, r: 40.0, label: "GAIN", tip: "Make-up gain.", active: None },
            Item::Meter { m: 0, x: 235.0, y: 90.0, w: 170.0, h: 120.0, kind: MeterKind::GainReduction, caption: "GAIN REDUCTION" },
            Item::Switch { p: LIMIT, x: 320.0, y: 300.0, labels: ["COMPRESS", "LIMIT"], tip: "Compress or limit." },
            Item::Knob { p: PEAK, x: 510.0, y: 300.0, r: 40.0, label: "PEAK REDUCTION", tip: "How much it reduces.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "LA-3A", class_id: *b"HWLA3ALevelerPQD", vendor: "PEQUOD Hardware",
    categories: [Fx, Dynamics]
);

#[cfg(test)]
#[allow(dead_code)]
pub mod tests {
    use super::*;
    use std::f64::consts::PI;

    pub fn run(p: &[f32], input: &mut [Vec<f32>]) -> Vec<f32> {
        let mut d = Dsp::new(48000.0, input.len());
        d.set_params(p);
        d.reset();
        let mut m = vec![0.0f32; Dsp::METERS.max(1)];
        let n = input[0].len();
        let mut k = 0;
        while k < n {
            let e = (k + 512).min(n);
            d.set_params(p);
            let mut chans: Vec<&mut [f32]> = input.iter_mut().map(|c| &mut c[k..e]).collect();
            d.process(&mut chans, &mut m);
            k = e;
        }
        m
    }
    pub fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
    }
    pub fn db(x: f64) -> f64 {
        20.0 * x.max(1e-12).log10()
    }
    pub fn with(mut p: Vec<f32>, set: &[(usize, f32)]) -> Vec<f32> {
        for (i, v) in set {
            p[*i] = *v;
        }
        p
    }
    pub fn sine(f: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n).map(|k| (amp * (2.0 * PI * f * k as f64 / 48000.0).sin()) as f32).collect()
    }
    pub fn noise(n: usize, amp: f64) -> Vec<f32> {
        let mut nz = vintage::dsp::Noise::new(11);
        (0..n).map(|_| (nz.next() * amp) as f32).collect()
    }
    /// Both channels out for a mono input.
    pub fn go2(p: &[f32], x: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        let r = input.pop().unwrap();
        (input.pop().unwrap(), r)
    }
    pub fn go(p: &[f32], x: &[f32]) -> Vec<f32> {
        go2(p, x).0
    }
    /// Steady-state level of a sine at f through the settings, in dB.
    pub fn level(set: &[(usize, f32)], f: f64) -> f64 {
        let p = with(defaults(), set);
        let y = go(&p, &sine(f, 0.1, 48000));
        db(rms(&y[24000..]))
    }
    /// Every preset runs finite and bounded on noise bursts.
    #[test]
    fn presets_run_clean() {
        let base = defaults();
        let mut all = vec![base.clone()];
        for pr in presets() {
            all.push(with(base.clone(), pr.values));
        }
        for p in all {
            let mut nz = vintage::dsp::Noise::new(7);
            let n = 48000;
            let l: Vec<f32> = (0..n).map(|k| if (k / 6000) % 2 == 0 { (nz.next() * 0.5) as f32 } else { 0.0 }).collect();
            let mut input = vec![l.clone(), l];
            run(&p, &mut input);
            assert!(input.iter().flatten().all(|v| v.is_finite() && v.abs() < 8.0), "unbounded output for {:?}", p);
        }
    }

    /// Louder in comes out much less louder: it compresses.
    fn spread(set: &[(usize, f32)]) -> f64 {
        let p = with(defaults(), set);
        let a = db(rms(&go(&p, &sine(1000.0, 0.03, 48000))[24000..]));
        let b = db(rms(&go(&p, &sine(1000.0, 0.9, 48000))[24000..]));
        b - a
    }
    #[test]
    fn compresses() {
        let s = spread(&[(PEAK, 0.8)]);
        println!("+29.5 dB in -> {:.1} dB out", s);
        assert!(s < 24.0, "{s}");
    }

}
