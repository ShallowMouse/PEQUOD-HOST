//! Lang PEQ-1: Langevin's passive programme equaliser (1950s), the Pultec's rival in American studios:
//! passive LC curves with a valve make-up stage. A LOW band (20, 30, 60, 100 Hz) to boost or attenuate, a
//! PRESENCE boost (1.5, 2, 3, 4, 5, 6 kHz), a HIGH boost (3 to 16 kHz) with its bandwidth, and a HIGH
//! ATTEN shelf. Firmer, a little more forward than the Pultec. Frequencies after period documentation;
//! curves estimated.

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

pub const W: u32 = 640;
pub const H: u32 = 432;

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "lf"]
    pub lf: FloatParam,
    #[id = "lboost"]
    pub lboost: FloatParam,
    #[id = "latten"]
    pub latten: FloatParam,
    #[id = "mf"]
    pub mf: FloatParam,
    #[id = "mboost"]
    pub mboost: FloatParam,
    #[id = "hf"]
    pub hf: FloatParam,
    #[id = "hboost"]
    pub hboost: FloatParam,
    #[id = "hatten"]
    pub hatten: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            lf: FloatParam::new("Low Freq", 2.0, FloatRange::Linear { min: 0.0, max: 3.0 }).with_step_size(1.0),
            lboost: FloatParam::new("Low Boost", 0.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            latten: FloatParam::new("Low Atten", 0.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            mf: FloatParam::new("Presence Freq", 2.0, FloatRange::Linear { min: 0.0, max: 5.0 }).with_step_size(1.0),
            mboost: FloatParam::new("Presence", 0.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            hf: FloatParam::new("High Freq", 3.0, FloatRange::Linear { min: 0.0, max: 5.0 }).with_step_size(1.0),
            hboost: FloatParam::new("High Boost", 0.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            hatten: FloatParam::new("High Atten", 0.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            output: db("Output", 0.0, -12.0, 12.0),
        }
    }
}

pub const LF: usize = 0;
pub const LBOOST: usize = 1;
pub const LATTEN: usize = 2;
pub const MF: usize = 3;
pub const MBOOST: usize = 4;
pub const HF: usize = 5;
pub const HBOOST: usize = 6;
pub const HATTEN: usize = 7;
pub const OUTPUT: usize = 8;
pub const NPARAMS: usize = 9;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.lf.as_ptr(),
            self.lboost.as_ptr(),
            self.latten.as_ptr(),
            self.mf.as_ptr(),
            self.mboost.as_ptr(),
            self.hf.as_ptr(),
            self.hboost.as_ptr(),
            self.hatten.as_ptr(),
            self.output.as_ptr(),
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

const LF_HZ: [f64; 4] = [20.0, 30.0, 60.0, 100.0];
const MF_HZ: [f64; 6] = [1500.0, 2000.0, 3000.0, 4000.0, 5000.0, 6000.0];
const HF_HZ: [f64; 6] = [3000.0, 5000.0, 8000.0, 10000.0, 12000.0, 16000.0];

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    bands: Vec<[Biquad; 5]>,
    iron: Vec<Iron>,
    iron2: Vec<Iron>,
    out: f64,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let v = self.v;
        let f = LF_HZ[(v[LF].round() as usize).min(3)];
        for b in self.bands.iter_mut() {
            b[0].set_low_shelf(fs, f * 1.2, 0.8, v[LBOOST] as f64 * 1.4);
            b[1].set_low_shelf(fs, f * 2.0, 0.8, -(v[LATTEN] as f64) * 1.6);
            b[2].set_peak(fs, MF_HZ[(v[MF].round() as usize).min(5)], 1.1, v[MBOOST] as f64 * 1.0);
            b[3].set_peak(fs, HF_HZ[(v[HF].round() as usize).min(5)].min(fs * 0.42), 0.8, v[HBOOST] as f64 * 1.5);
            b[4].set_high_shelf(fs, 10000.0f64.min(fs * 0.42), 0.7, -(v[HATTEN] as f64) * 1.5);
        }
        self.out = db_to_gain(v[OUTPUT] as f64);
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let n = ch.max(2);
        let mut d = Self { fs, v, bands: vec![[Biquad::identity(); 5]; n], iron: vec![Iron::new(fs, 40.0, 1.4, 30000.0); n],
                           iron2: vec![Iron::new(fs, 35.0, 1.7, 22000.0); n], out: 1.0 };
        d.design();
        d
    }

    fn reset(&mut self) {
        self.bands.iter_mut().flatten().for_each(|b| b.reset());
        self.iron.iter_mut().chain(self.iron2.iter_mut()).for_each(|i| i.reset());
        self.design();
    }
    fn set_params(&mut self, v: &[f32]) {
        let changed = self.v[..] != v[..NPARAMS];
        self.v.copy_from_slice(&v[..NPARAMS]);
        if changed {
            self.design();
        }
    }

    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        for (ci, ch) in chans.iter_mut().enumerate() {
            let j = ci.min(self.bands.len() - 1);
            for s in ch.iter_mut() {
                let x = self.iron[j].process(*s as f64);
                let y = valve(cascade(&mut self.bands[j], x), 0.16);
                *s = (self.iron2[j].process(y) * self.out) as f32;
            }
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Vocal Forward (3 kHz)", values: &[(MF, 2.0), (MBOOST, 4.0), (HF, 4.0), (HBOOST, 2.0)] },
        Preset { name: "Bass Floor (60 Hz)", values: &[(LF, 2.0), (LBOOST, 5.0), (LATTEN, 2.0)] },
        Preset { name: "Drum Bus Weight and Crack", values: &[(LF, 1.0), (LBOOST, 3.0), (MF, 3.0), (MBOOST, 3.0)] },
        Preset { name: "Darker Mix", values: &[(HATTEN, 3.0), (LF, 3.0), (LBOOST, 2.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "LANG PEQ-1",
        subtitle: ("PROGRAM EQUALIZER", "PASSIVE LC CURVES, VALVE MAKE-UP"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![
            Item::Selector { p: LF, x: 80.0, y: 140.0, label: "LOW", positions: &["20", "30", "60", "100"], tip: "Hz." },
            Item::Knob { p: LBOOST, x: 190.0, y: 140.0, r: 28.0, label: "BOOST", tip: "Low boost.", active: None },
            Item::Knob { p: LATTEN, x: 290.0, y: 140.0, r: 28.0, label: "ATTEN", tip: "Low cut.", active: None },
            Item::Selector { p: MF, x: 420.0, y: 140.0, label: "PRESENCE", positions: &["1.5", "2", "3", "4", "5", "6"], tip: "kHz." },
            Item::Knob { p: MBOOST, x: 545.0, y: 140.0, r: 28.0, label: "BOOST", tip: "Presence boost.", active: None },
            Item::Divider { y: 235.0 },
            Item::Selector { p: HF, x: 80.0, y: 330.0, label: "HIGH", positions: &["3", "5", "8", "10", "12", "16"], tip: "kHz." },
            Item::Knob { p: HBOOST, x: 190.0, y: 330.0, r: 28.0, label: "BOOST", tip: "High boost.", active: None },
            Item::Knob { p: HATTEN, x: 300.0, y: 330.0, r: 28.0, label: "ATTEN", tip: "High shelf cut from 10 kHz.", active: None },
            Item::Knob { p: OUTPUT, x: 545.0, y: 330.0, r: 26.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Lang PEQ-1", class_id: *b"HWLangPEQ1EqPQD1", vendor: "PEQUOD Hardware",
    categories: [Fx, Eq]
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

    #[test]
    fn shapes_as_set() {
        let flat = level(&[], 3000.0);
        let set = level(&[(MF, 2.0), (MBOOST, 10.0)], 3000.0);
        println!("at 3000.0 Hz: {:.1} dB -> {:.1} dB", flat, set);
        assert!(set - flat > 7.0, "{flat} {set}");
    }

}
