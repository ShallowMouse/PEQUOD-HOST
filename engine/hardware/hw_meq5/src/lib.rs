//! Pultec MEQ-5: the midrange companion to the EQP-1A, passive with a valve make-up stage. Three bands
//! as on the front: a LOW PEAK (200, 300, 500, 700 Hz, 1 kHz) up to +10 dB, a DIP across the middle
//! (200 Hz to 7 kHz) down to -10 dB, and a HIGH PEAK (1.5, 2, 3, 4, 5 kHz) up to +10 dB. The broad
//! inductor curves are what make it musical on vocals and guitars. From the published charts; estimated.

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
    #[id = "lpeak"]
    pub lpeak: FloatParam,
    #[id = "df"]
    pub df: FloatParam,
    #[id = "dip"]
    pub dip: FloatParam,
    #[id = "hf"]
    pub hf: FloatParam,
    #[id = "hpeak"]
    pub hpeak: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            lf: FloatParam::new("Low Peak Freq", 1.0, FloatRange::Linear { min: 0.0, max: 4.0 }).with_step_size(1.0),
            lpeak: FloatParam::new("Low Peak", 0.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            df: FloatParam::new("Dip Freq", 4.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(1.0),
            dip: FloatParam::new("Dip", 0.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            hf: FloatParam::new("High Peak Freq", 2.0, FloatRange::Linear { min: 0.0, max: 4.0 }).with_step_size(1.0),
            hpeak: FloatParam::new("High Peak", 0.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            output: db("Output", 0.0, -12.0, 12.0),
        }
    }
}

pub const LF: usize = 0;
pub const LPEAK: usize = 1;
pub const DF: usize = 2;
pub const DIP: usize = 3;
pub const HF: usize = 4;
pub const HPEAK: usize = 5;
pub const OUTPUT: usize = 6;
pub const NPARAMS: usize = 7;

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
            self.lpeak.as_ptr(),
            self.df.as_ptr(),
            self.dip.as_ptr(),
            self.hf.as_ptr(),
            self.hpeak.as_ptr(),
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

const LF_HZ: [f64; 5] = [200.0, 300.0, 500.0, 700.0, 1000.0];
const DF_HZ: [f64; 11] = [200.0, 300.0, 500.0, 700.0, 1000.0, 1500.0, 2000.0, 3000.0, 4000.0, 5000.0, 7000.0];
const HF_HZ: [f64; 5] = [1500.0, 2000.0, 3000.0, 4000.0, 5000.0];

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    bands: Vec<[Biquad; 3]>,
    iron: Vec<Iron>,
    iron2: Vec<Iron>,
    out: f64,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let v = self.v;
        for b in self.bands.iter_mut() {
            b[0].set_peak(fs, LF_HZ[(v[LF].round() as usize).min(4)], 0.7, v[LPEAK] as f64);
            b[1].set_peak(fs, DF_HZ[(v[DF].round() as usize).min(10)], 0.8, -(v[DIP] as f64));
            b[2].set_peak(fs, HF_HZ[(v[HF].round() as usize).min(4)], 0.9, v[HPEAK] as f64);
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
        let mut d = Self { fs, v, bands: vec![[Biquad::identity(); 3]; n], iron: vec![Iron::new(fs, 35.0, 1.3, 30000.0); n],
                           iron2: vec![Iron::new(fs, 30.0, 1.6, 24000.0); n], out: 1.0 };
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
                let y = valve(cascade(&mut self.bands[j], x), 0.12);
                *s = (self.iron2[j].process(y) * self.out) as f32;
            }
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Vocal Presence (3 kHz)", values: &[(HF, 2.0), (HPEAK, 4.0), (DF, 1.0), (DIP, 3.0)] },
        Preset { name: "Guitar: Less Box (500 Hz dip)", values: &[(DF, 2.0), (DIP, 5.0), (HF, 3.0), (HPEAK, 3.0)] },
        Preset { name: "Snare Body (200 Hz)", values: &[(LF, 0.0), (LPEAK, 4.0), (HF, 4.0), (HPEAK, 3.0)] },
        Preset { name: "Piano Clarity", values: &[(DF, 1.0), (DIP, 3.0), (HF, 1.0), (HPEAK, 2.5)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "PULTEC MEQ-5",
        subtitle: ("MID-RANGE EQUALIZER", "PEAK, DIP, PEAK: PASSIVE, VALVE MAKE-UP"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![
            Item::Selector { p: LF, x: 110.0, y: 140.0, label: "LOW PEAK", positions: &["200", "300", "500", "700", "1K"], tip: "Hz." },
            Item::Knob { p: LPEAK, x: 110.0, y: 320.0, r: 32.0, label: "PEAK", tip: "Up to +10 dB.", active: None },
            Item::Selector { p: DF, x: 320.0, y: 140.0, label: "DIP", positions: &["200", "300", "500", "700", "1K", "1.5K", "2K", "3K", "4K", "5K", "7K"], tip: "Hz." },
            Item::Knob { p: DIP, x: 320.0, y: 320.0, r: 32.0, label: "DIP", tip: "Down to -10 dB.", active: None },
            Item::Selector { p: HF, x: 530.0, y: 140.0, label: "HIGH PEAK", positions: &["1.5K", "2K", "3K", "4K", "5K"], tip: "Hz." },
            Item::Knob { p: HPEAK, x: 530.0, y: 320.0, r: 32.0, label: "PEAK", tip: "Up to +10 dB.", active: None },
            Item::Knob { p: OUTPUT, x: 420.0, y: 230.0, r: 20.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Pultec MEQ-5", class_id: *b"HWPultecMEQ5PQD1", vendor: "PEQUOD Hardware",
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
        let flat = level(&[], 500.0);
        let set = level(&[(DF, 2.0), (DIP, 10.0)], 500.0);
        println!("at 500.0 Hz: {:.1} dB -> {:.1} dB", flat, set);
        assert!(set - flat < -7.0, "{flat} {set}");
    }

}
