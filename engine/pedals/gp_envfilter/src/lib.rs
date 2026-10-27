//! Envelope Filter: an auto-wah after the Musitronics Mu-Tron III: a resonant state-variable filter whose frequency follows how hard you play. Low-pass, band-pass or high-pass, sweeping up or down, with PEAK for the resonance and SENS for how far your picking pushes it.

#![allow(clippy::excessive_precision)]
use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
#[allow(unused_imports)]
use vintage::dsp::*;
use vintage::nih_plug_egui::EguiState;
#[allow(unused_imports)]
use vintage::nih_plug_egui::egui::Align2;
#[allow(unused_imports)]
use vintage::params::*;
#[allow(unused_imports)]
use vintage::ui::{self, ActiveWhen, Item, MeterKind, Panel, Preset};
#[allow(unused_imports)]
use vintage::{Midi, VintageDsp, VintageParams};


pub const W: u32 = 640;
pub const H: u32 = 420;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Mode { #[name = "Low-pass"] Lp, #[name = "Band-pass"] Bp, #[name = "High-pass"] Hp }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Dir { #[name = "Up"] Up, #[name = "Down"] Down }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "mode"]
    pub mode: EnumParam<Mode>,
    #[id = "dir"]
    pub dir: EnumParam<Dir>,
    #[id = "range"]
    pub range: BoolParam,
    #[id = "peak"]
    pub peak: FloatParam,
    #[id = "sens"]
    pub sens: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            mode: EnumParam::new("Mode", Mode::Bp),
            dir: EnumParam::new("Direction", Dir::Up),
            range: BoolParam::new("High Range", false),
            peak: pct("Peak", 0.6),
            sens: pct("Sensitivity", 0.5),
            mix: pct("Mix", 1.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const MODE: usize = 0;
pub const DIR: usize = 1;
pub const RANGE: usize = 2;
pub const PEAK: usize = 3;
pub const SENS: usize = 4;
pub const MIX: usize = 5;
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
            self.mode.as_ptr(),
            self.dir.as_ptr(),
            self.range.as_ptr(),
            self.peak.as_ptr(),
            self.sens.as_ptr(),
            self.mix.as_ptr(),
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

#[derive(Clone, Copy, Default)]
pub struct Svf {
    ic1: f64,
    ic2: f64,
}

impl Svf {
    /// returns (low, band, high)
    #[inline]
    pub fn process(&mut self, x: f64, fs: f64, fc: f64, q: f64) -> (f64, f64, f64) {
        let g = (std::f64::consts::PI * fc.clamp(10.0, fs * 0.45) / fs).tan();
        let k = 1.0 / q.max(0.3);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, v1, x - k * v1 - v2)
    }
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    f: Vec<Svf>,
    env: f64,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, f: vec![Svf::default(); ch.max(2)], env: 0.0, out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.f.iter_mut().for_each(|s| *s = Svf::default());
        self.env = 0.0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let mode = self.v[MODE].round() as usize;
        let down = self.v[DIR] > 0.5;
        let base = if self.v[RANGE] > 0.5 { 400.0 } else { 180.0 };
        let q = 0.8 + 12.0 * (self.v[PEAK] as f64).powi(2);
        let sens = 2.0 + 150.0 * (self.v[SENS] as f64).powi(2);
        let mix = self.v[MIX] as f64;
        let ca = coeff(self.fs, 4.0);
        let cr = coeff(self.fs, 140.0);
        let last = self.f.len() - 1;
        let mut top = 0.0f64;
        for k in 0..n {
            let mut a = 0.0f64;
            for c in chans.iter() {
                a = a.max((c[k] as f64).abs());
            }
            self.env = a + if a > self.env { ca } else { cr } * (self.env - a);
            let e = (self.env * sens).min(1.0);
            let e = if down { 1.0 - e } else { e };
            top = top.max(e);
            let fc = base * 2f64.powf(5.0 * e);
            let o = self.out.next();
            for (ci, c) in chans.iter_mut().enumerate() {
                let x = c[k] as f64;
                let (lp, bp, hp) = self.f[ci.min(last)].process(x, self.fs, fc, q);
                let y = match mode {
                    0 => lp,
                    1 => bp * (1.0 + 0.15 * q).min(3.0) / q.sqrt().max(1.0) * 1.5,
                    _ => hp,
                };
                c[k] = ((y * mix + x * (1.0 - mix)) * o) as f32;
            }
        }
        meters[0] = top as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Funk Quack (band-pass up)", values: &[] },
        Preset { name: "Low-Pass Squelch", values: &[(MODE, 0.0), (PEAK, 0.75)] },
        Preset { name: "Down Sweep", values: &[(DIR, 1.0), (MODE, 0.0)] },
        Preset { name: "Bass Envelope", values: &[(MODE, 0.0), (RANGE, 0.0), (SENS, 0.6), (PEAK, 0.5)] },
        Preset { name: "High and Vocal", values: &[(RANGE, 1.0), (PEAK, 0.8)] },
        Preset { name: "Subtle Touch Wah", values: &[(PEAK, 0.35), (SENS, 0.35), (MIX, 0.6)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "ENVELOPE FILTER",
        subtitle: ("AFTER THE MU-TRON III", "TOUCH-SENSITIVE AUTO-WAH"),
        accent: ui::COBALT,
        size: (W, H),
        items: vec![

            Item::Selector { p: MODE, x: 80.0, y: 150.0, label: "MODE", positions: &["LP", "BP", "HP"], tip: "Filter type." },
            Item::Selector { p: DIR, x: 190.0, y: 150.0, label: "DIRECTION", positions: &["UP", "DOWN"], tip: "Harder picking sweeps the filter up, or down." },
            Item::Switch { p: RANGE, x: 270.0, y: 150.0, labels: ["LO", "HI"], tip: "Range of the sweep." },
            Item::Lamp { m: 0, x: 470.0, y: 135.0, label: "SWEEP", lo: 0.1, hi: 0.9 },
            Item::Divider { y: 215.0 },
            Item::Knob { p: PEAK, x: 90.0, y: 285.0, r: 34.0, label: "PEAK", tip: "Resonance.", active: None },
            Item::Knob { p: SENS, x: 243.3, y: 285.0, r: 34.0, label: "SENS", tip: "How far your picking pushes the filter.", active: None },
            Item::Knob { p: MIX, x: 396.7, y: 285.0, r: 34.0, label: "MIX", tip: "Dry under the filter.", active: None },
            Item::Knob { p: OUTPUT, x: 550.0, y: 285.0, r: 34.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Envelope Filter", class_id: *b"PqdEnvFilterMuII", vendor: "PEQUOD Pedals",
    categories: [Fx, Filter]
);

#[cfg(test)]
#[allow(dead_code)]
pub mod tests {
    use super::*;

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
    pub fn sine(f: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n).map(|k| (amp * (2.0 * std::f64::consts::PI * f * k as f64 / 48000.0).sin()) as f32).collect()
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
            let mut l: Vec<f32> = (0..n).map(|k| if (k / 6000) % 2 == 0 { (nz.next() * 0.5) as f32 } else { 0.0 }).collect();
            let mut r = l.clone();
            {
                let mut input = vec![std::mem::take(&mut l), std::mem::take(&mut r)];
                run(&p, &mut input);
                assert!(input.iter().flatten().all(|v| v.is_finite() && v.abs() < 8.0), "unbounded output for {:?}", p);
            }
        }
    }

    use std::f64::consts::PI;
    fn goertzel(x: &[f32], f: f64) -> f64 {
        let w = 2.0 * PI * f / 48000.0;
        let (mut s1, mut s2) = (0.0, 0.0);
        for v in x {
            let s = *v as f64 + 2.0 * w.cos() * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        ((s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2).max(0.0)).sqrt() / x.len() as f64 * 2.0
    }
    fn stereo(p: &[f32], x: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        let r = input.pop().unwrap();
        (input.pop().unwrap(), r)
    }
    fn corr(a: &[f32], b: &[f32]) -> f64 {
        let ab: f64 = a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum();
        let aa: f64 = a.iter().map(|x| (*x as f64).powi(2)).sum();
        let bb: f64 = b.iter().map(|x| (*x as f64).powi(2)).sum();
        ab / (aa * bb).sqrt().max(1e-12)
    }
    /// short-window rms envelope
    fn env(x: &[f32], w: usize) -> Vec<f64> {
        x.chunks(w).map(rms).collect()
    }

    #[test]
    fn picking_opens_the_filter() {
        let p = with(defaults(), &[(MODE, 0.0), (PEAK, 0.3), (SENS, 0.5)]);
        let quiet = stereo(&p, &sine(2000.0, 0.003, 24000)).0;
        let loud = stereo(&p, &sine(2000.0, 0.3, 24000)).0;
        let gq = db(rms(&quiet[12000..])) - db(0.003 / 2f64.sqrt());
        let gl = db(rms(&loud[12000..])) - db(0.3 / 2f64.sqrt());
        println!("2 kHz gain quiet {gq:.1} dB, loud {gl:.1} dB");
        assert!(gl > gq + 10.0);
    }

}
