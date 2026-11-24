//! Neve 1073: Rupert Neve's class-A microphone preamp and equaliser module (1970), from the BCM10 and
//! A88 desks, the most copied channel strip there is. Gain into the Marinair transformers and the
//! BC184/BD131 class-A stages (the more you drive it, the thicker and the more second and third harmonic),
//! a 3-pole HIGH-PASS (50, 80, 160, 300 Hz), a LOW shelf (35, 60, 110, 220 Hz), a MID bell (360 Hz, 700 Hz,
//! 1.6, 3.2, 4.8, 7.2 kHz) and a fixed 12 kHz HIGH shelf, each +/-16 to 18 dB, OUTPUT trim. Frequencies
//! from Neve's manual; the curves and the drive are estimated.

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
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "hpf"]
    pub hpf: FloatParam,
    #[id = "lf"]
    pub lf: FloatParam,
    #[id = "lgain"]
    pub lgain: FloatParam,
    #[id = "mf"]
    pub mf: FloatParam,
    #[id = "mgain"]
    pub mgain: FloatParam,
    #[id = "hgain"]
    pub hgain: FloatParam,
    #[id = "eq"]
    pub eq: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            drive: db("Gain", 0.0, -10.0, 30.0),
            hpf: FloatParam::new("High Pass", 0.0, FloatRange::Linear { min: 0.0, max: 4.0 }).with_step_size(1.0),
            lf: FloatParam::new("Low Freq", 0.0, FloatRange::Linear { min: 0.0, max: 4.0 }).with_step_size(1.0),
            lgain: db("Low Gain", 0.0, -16.0, 16.0),
            mf: FloatParam::new("Mid Freq", 0.0, FloatRange::Linear { min: 0.0, max: 6.0 }).with_step_size(1.0),
            mgain: db("Mid Gain", 0.0, -18.0, 18.0),
            hgain: db("High Gain", 0.0, -16.0, 16.0),
            eq: FloatParam::new("EQ", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            output: db("Output", 0.0, -20.0, 10.0),
        }
    }
}

pub const DRIVE: usize = 0;
pub const HPF: usize = 1;
pub const LF: usize = 2;
pub const LGAIN: usize = 3;
pub const MF: usize = 4;
pub const MGAIN: usize = 5;
pub const HGAIN: usize = 6;
pub const EQ: usize = 7;
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
            self.drive.as_ptr(),
            self.hpf.as_ptr(),
            self.lf.as_ptr(),
            self.lgain.as_ptr(),
            self.mf.as_ptr(),
            self.mgain.as_ptr(),
            self.hgain.as_ptr(),
            self.eq.as_ptr(),
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

const HPF_HZ: [f64; 5] = [0.0, 50.0, 80.0, 160.0, 300.0];
const LF_HZ: [f64; 5] = [0.0, 35.0, 60.0, 110.0, 220.0];
const MF_HZ: [f64; 7] = [0.0, 360.0, 700.0, 1600.0, 3200.0, 4800.0, 7200.0];

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    bands: Vec<[Biquad; 5]>,
    iron: Vec<Iron>,
    iron2: Vec<Iron>,
    drive: f64,
    out: f64,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let v = self.v;
        let eq = v[EQ] > 0.5;
        let h = HPF_HZ[(v[HPF].round() as usize).min(4)];
        let lf = LF_HZ[(v[LF].round() as usize).min(4)];
        let mf = MF_HZ[(v[MF].round() as usize).min(6)];
        for b in self.bands.iter_mut() {
            // 18 dB/octave: a second-order and a first-order section
            if h > 0.0 { b[0].set_highpass(fs, h, 0.75); b[1].set_highpass1(fs, h); } else { b[0] = Biquad::identity(); b[1] = Biquad::identity(); }
            // the inductor low shelf has a little bump just above its frequency
            if eq && lf > 0.0 { b[2].set_low_shelf(fs, lf * 1.3, 0.9, v[LGAIN] as f64); } else { b[2] = Biquad::identity(); }
            if eq && mf > 0.0 { b[3].set_peak(fs, mf, 1.4, v[MGAIN] as f64); } else { b[3] = Biquad::identity(); }
            if eq { b[4].set_high_shelf(fs, 12000.0f64.min(fs * 0.42), 0.6, v[HGAIN] as f64); } else { b[4] = Biquad::identity(); }
        }
        self.drive = db_to_gain(v[DRIVE] as f64);
        self.out = db_to_gain(v[OUTPUT] as f64 - v[DRIVE] as f64 * 0.8);
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let n = ch.max(2);
        let mut d = Self { fs, v, bands: vec![[Biquad::identity(); 5]; n], iron: vec![Iron::new(fs, 80.0, 1.8, 40000.0); n],
                           iron2: vec![Iron::new(fs, 60.0, 2.2, 26000.0); n], drive: 1.0, out: 1.0 };
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
                // the Marinair input transformer, then the class-A gain stages
                let x = self.iron[j].process(*s as f64 * self.drive);
                let x = x + 0.06 * x * x - 0.05 * x * x * x;
                let y = cascade(&mut self.bands[j], x);
                let y = (y * 1.1).tanh() / 1.1;
                *s = (self.iron2[j].process(y) * self.out) as f32;
            }
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Vocal (HPF 80, 12k +4)", values: &[(HPF, 2.0), (HGAIN, 4.0), (MF, 3.0), (MGAIN, 2.0)] },
        Preset { name: "Kick (60 +6, 3.2k +3)", values: &[(LF, 2.0), (LGAIN, 6.0), (MF, 4.0), (MGAIN, 3.0)] },
        Preset { name: "Snare (220 +3, 4.8k +4)", values: &[(LF, 4.0), (LGAIN, 3.0), (MF, 5.0), (MGAIN, 4.0), (HPF, 1.0)] },
        Preset { name: "Driven Guitar", values: &[(DRIVE, 18.0), (HPF, 2.0), (MF, 3.0), (MGAIN, 3.0)] },
        Preset { name: "Bass Body (110 +4)", values: &[(LF, 3.0), (LGAIN, 4.0), (MF, 2.0), (MGAIN, -3.0)] },
        Preset { name: "Just the Preamp, Hot", values: &[(DRIVE, 20.0), (EQ, 0.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "NEVE 1073",
        subtitle: ("CLASS-A MIC PRE AND EQUALISER, 1970", "MARINAIR TRANSFORMERS, INDUCTOR EQ"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![
            Item::Knob { p: DRIVE, x: 80.0, y: 140.0, r: 34.0, label: "GAIN", tip: "Into the transformer and the class-A stages: more drive, thicker.", active: None },
            Item::Selector { p: HPF, x: 210.0, y: 140.0, label: "HIGH PASS", positions: &["OFF", "50", "80", "160", "300"], tip: "18 dB per octave, Hz." },
            Item::Knob { p: HGAIN, x: 330.0, y: 140.0, r: 28.0, label: "12 KHZ", tip: "High shelf at 12 kHz.", active: None },
            Item::Switch { p: EQ, x: 450.0, y: 140.0, labels: ["EQ OUT", "EQ IN"], tip: "The equaliser in or out (the preamp stays)." },
            Item::Knob { p: OUTPUT, x: 565.0, y: 140.0, r: 26.0, label: "OUTPUT", tip: "Output trim.", active: None },
            Item::Divider { y: 230.0 },
            Item::Selector { p: MF, x: 100.0, y: 330.0, label: "MID FREQ", positions: &["OFF", ".36", ".7", "1.6", "3.2", "4.8", "7.2"], tip: "kHz." },
            Item::Knob { p: MGAIN, x: 220.0, y: 330.0, r: 30.0, label: "MID", tip: "Mid bell, +/-18 dB.", active: None },
            Item::Selector { p: LF, x: 380.0, y: 330.0, label: "LOW FREQ", positions: &["OFF", "35", "60", "110", "220"], tip: "Hz." },
            Item::Knob { p: LGAIN, x: 510.0, y: 330.0, r: 30.0, label: "LOW", tip: "Low shelf, +/-16 dB.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Neve 1073", class_id: *b"HWNeve1073ChanPQ", vendor: "PEQUOD Hardware",
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
        let flat = level(&[], 1600.0);
        let set = level(&[(MF, 3.0), (MGAIN, 12.0)], 1600.0);
        println!("at 1600.0 Hz: {:.1} dB -> {:.1} dB", flat, set);
        assert!(set - flat > 9.0, "{flat} {set}");
    }

}
