//! Small Stone: a four-stage phaser after the Electro-Harmonix Small Stone: allpass stages swept by an LFO and mixed with the dry for two moving notches. The COLOR switch adds feedback for a deeper, more vocal sweep.

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
pub const H: u32 = 330;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "rate"]
    pub rate: FloatParam,
    #[id = "color"]
    pub color: BoolParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            rate: rate("Rate", 0.6, 0.05, 8.0),
            color: BoolParam::new("Color", false),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const RATE: usize = 0;
pub const COLOR: usize = 1;
pub const OUTPUT: usize = 2;
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
            self.rate.as_ptr(),
            self.color.as_ptr(),
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
struct Ap {
    z: f64,
}

impl Ap {
    #[inline]
    fn process(&mut self, x: f64, a: f64) -> f64 {
        let y = a * x + self.z;
        self.z = x - a * y;
        y
    }
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ap: Vec<[Ap; 4]>,
    fb: Vec<f64>,
    lfo: Lfo,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, ap: vec![[Ap::default(); 4]; ch.max(2)], fb: vec![0.0; ch.max(2)], lfo: Lfo::default(), out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.ap.iter_mut().for_each(|a| *a = [Ap::default(); 4]);
        self.fb.iter_mut().for_each(|f| *f = 0.0);
        self.lfo = Lfo::default();
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fbk = if self.v[COLOR] > 0.5 { 0.55 } else { 0.0 };
        let rate = self.v[RATE] as f64;
        let last = self.ap.len() - 1;
        for k in 0..n {
            let l = self.lfo.next(self.fs, rate);
            let fc = 180.0 * 2f64.powf(3.6 * (0.5 + 0.5 * l));
            let t = (std::f64::consts::PI * fc / self.fs).tan();
            let a = (t - 1.0) / (t + 1.0);
            let o = self.out.next();
            for (ci, c) in chans.iter_mut().enumerate() {
                let i = ci.min(last);
                let x = c[k] as f64;
                let mut y = x + fbk * self.fb[i];
                for s in self.ap[i].iter_mut() {
                    y = s.process(y, a);
                }
                self.fb[i] = y;
                c[k] = (0.5 * (x + y) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Slow Phase", values: &[] },
        Preset { name: "Color Sweep", values: &[(COLOR, 1.0)] },
        Preset { name: "Fast Leslie-ish", values: &[(RATE, 5.0)] },
        Preset { name: "Very Slow Color", values: &[(RATE, 0.12), (COLOR, 1.0)] },
        Preset { name: "Funk Rhythm Phase", values: &[(RATE, 1.5), (COLOR, 1.0)] },
        Preset { name: "Gentle Movement", values: &[(RATE, 0.3)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "SMALL STONE",
        subtitle: ("AFTER THE EHX SMALL STONE", "FOUR-STAGE PHASER · COLOR SWITCH"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![
            Item::Knob { p: RATE, x: 90.0, y: 190.0, r: 40.0, label: "RATE", tip: "Speed of the sweep.", active: None },
            Item::Switch { p: COLOR, x: 290.0, y: 190.0, labels: ["OFF", "COLOR"], tip: "Feedback for a deeper, more vocal sweep." },
            Item::Knob { p: OUTPUT, x: 550.0, y: 190.0, r: 40.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Small Stone", class_id: *b"PqdSmallStonePh1", vendor: "PEQUOD Pedals",
    categories: [Fx, Modulation]
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
    fn notches_sweep() {
        let p = with(defaults(), &[(RATE, 1.0)]);
        let y = stereo(&p, &sine(700.0, 0.2, 96000)).0;
        let e = env(&y[48000..], 960);
        let (lo, hi) = e.iter().fold((f64::MAX, 0.0f64), |(a, b), v| (a.min(*v), b.max(*v)));
        println!("700 Hz level swings {:.1} dB", db(hi) - db(lo));
        assert!(db(hi) - db(lo) > 10.0);
    }

}
