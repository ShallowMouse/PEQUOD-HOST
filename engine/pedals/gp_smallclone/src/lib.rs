//! Small Clone: a bucket-brigade chorus after the Electro-Harmonix Small Clone (Kurt Cobain's chorus): a single modulated BBD voice mixed with the dry, with the DEPTH switch for shallow shimmer or deep warble. WIDTH spreads it into stereo by running the right side's LFO in opposition.

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
use vintage::fx::Bbd;

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
    #[id = "deep"]
    pub deep: BoolParam,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "width"]
    pub width: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            rate: rate("Rate", 0.8, 0.1, 8.0),
            deep: BoolParam::new("Deep", false),
            mix: pct("Mix", 0.5),
            width: pct("Width", 0.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const RATE: usize = 0;
pub const DEEP: usize = 1;
pub const MIX: usize = 2;
pub const WIDTH: usize = 3;
pub const OUTPUT: usize = 4;
pub const NPARAMS: usize = 5;

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
            self.deep.as_ptr(),
            self.mix.as_ptr(),
            self.width.as_ptr(),
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

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    bbd: Vec<Bbd>,
    lfo: Lfo,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let bbd = (0..ch.max(2)).map(|_| {
            let mut b = Bbd::new(fs, 20.0);
            b.set_bandwidth(7000.0);
            b
        }).collect();
        Self { fs, v, bbd, lfo: Lfo::default(), out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.bbd.iter_mut().for_each(|b| b.reset());
        self.lfo = Lfo::default();
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let depth = if self.v[DEEP] > 0.5 { 3.5 } else { 1.2 } * 0.001 * self.fs;
        let base = 0.007 * self.fs;
        let mix = self.v[MIX] as f64;
        let width = self.v[WIDTH] as f64;
        let rate = self.v[RATE] as f64;
        let last = self.bbd.len() - 1;
        for k in 0..n {
            let l = self.lfo.next(self.fs, rate);
            let o = self.out.next();
            for (ci, c) in chans.iter_mut().enumerate() {
                let m = if ci == 1 { l * (1.0 - 2.0 * width) } else { l };
                let x = c[k] as f64;
                let w = self.bbd[ci.min(last)].process(x, base + depth * m);
                c[k] = ((x * (1.0 - 0.5 * mix) + w * mix) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Shallow Shimmer", values: &[] },
        Preset { name: "Come As You Are (deep)", values: &[(DEEP, 1.0), (RATE, 1.2), (MIX, 0.55)] },
        Preset { name: "Slow and Lush", values: &[(DEEP, 1.0), (RATE, 0.35), (WIDTH, 1.0)] },
        Preset { name: "Stereo Clean Chorus", values: &[(RATE, 0.7), (WIDTH, 1.0)] },
        Preset { name: "Seasick Warble", values: &[(DEEP, 1.0), (RATE, 4.5), (MIX, 0.7)] },
        Preset { name: "Thickener", values: &[(RATE, 0.3), (MIX, 0.35)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "SMALL CLONE",
        subtitle: ("AFTER THE EHX SMALL CLONE", "BBD CHORUS · DEPTH SWITCH"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![
            Item::Knob { p: RATE, x: 90.0, y: 190.0, r: 36.0, label: "RATE", tip: "Speed of the modulation.", active: None },
            Item::Switch { p: DEEP, x: 175.0, y: 190.0, labels: ["", "DEEP"], tip: "The Small Clone's depth switch." },
            Item::Knob { p: MIX, x: 320.0, y: 190.0, r: 36.0, label: "MIX", tip: "Chorus level against the dry.", active: None },
            Item::Knob { p: WIDTH, x: 435.0, y: 190.0, r: 36.0, label: "WIDTH", tip: "Right side modulates in opposition: stereo spread.", active: None },
            Item::Knob { p: OUTPUT, x: 550.0, y: 190.0, r: 36.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Small Clone", class_id: *b"PqdSmallCloneCh1", vendor: "PEQUOD Pedals",
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
    fn modulates_and_spreads() {
        let p = with(defaults(), &[(DEEP, 1.0), (MIX, 1.0), (WIDTH, 1.0), (RATE, 1.0)]);
        let (l, r) = stereo(&p, &sine(1000.0, 0.2, 48000));
        let pure = goertzel(&l[24000..], 1000.0) / (rms(&l[24000..]) * 2f64.sqrt());
        let c = corr(&l[24000..], &r[24000..]);
        println!("1 kHz purity {:.1} dB, L/R correlation {c:.2}", db(pure));
        assert!(db(pure) < -3.0 && c < 0.9);
    }

}
