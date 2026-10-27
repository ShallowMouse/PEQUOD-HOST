//! Dimension: after the Boss DC-2 Dimension C (and the Roland Dimension D before it): two short modulated delays with opposite LFOs, cross-mixed so the movement cancels in the middle: a wide, still stereo shimmer rather than a wobble. Four modes, deeper as they go.

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


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Mode { #[name = "1"] A, #[name = "2"] B, #[name = "3"] C, #[name = "4"] D }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "mode"]
    pub mode: EnumParam<Mode>,
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
            mode: EnumParam::new("Mode", Mode::B),
            mix: pct("Mix", 1.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const MODE: usize = 0;
pub const MIX: usize = 1;
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
            self.mode.as_ptr(),
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

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    bbd: [Bbd; 2],
    phase: f64,
    out: Smooth,
}

const DEPTH_MS: [f64; 4] = [0.6, 1.0, 1.6, 2.4];
const RATE_HZ: [f64; 4] = [0.25, 0.25, 0.5, 0.5];

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mk = || {
            let mut b = Bbd::new(fs, 20.0);
            b.set_bandwidth(9000.0);
            b
        };
        Self { fs, v, bbd: [mk(), mk()], phase: 0.0, out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.bbd.iter_mut().for_each(|b| b.reset());
        self.phase = 0.0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let m = (self.v[MODE].round() as usize).min(3);
        let depth = DEPTH_MS[m] * 0.001 * self.fs;
        let base = 0.006 * self.fs;
        let mix = self.v[MIX] as f64;
        let nch = chans.len();
        for k in 0..n {
            self.phase = (self.phase + RATE_HZ[m] / self.fs).fract();
            let tri = 1.0 - 4.0 * (self.phase - 0.5).abs();
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            let w1 = self.bbd[0].process(x, base + depth * tri);
            let w2 = self.bbd[1].process(x, base - depth * tri);
            let o = self.out.next();
            if nch >= 2 {
                let (dl, dr) = (chans[0][k] as f64, chans[1][k] as f64);
                chans[0][k] = ((dl + mix * (0.6 * w1 - 0.35 * w2)) * o) as f32;
                chans[1][k] = ((dr + mix * (0.6 * w2 - 0.35 * w1)) * o) as f32;
            } else {
                chans[0][k] = ((x + mix * 0.25 * (w1 + w2)) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Mode 2 (classic)", values: &[] },
        Preset { name: "Mode 1 (subtle)", values: &[(MODE, 0.0)] },
        Preset { name: "Mode 3", values: &[(MODE, 2.0)] },
        Preset { name: "Mode 4 (widest)", values: &[(MODE, 3.0)] },
        Preset { name: "Clean Arpeggios", values: &[(MODE, 1.0), (MIX, 0.8)] },
        Preset { name: "Half Mix Width", values: &[(MODE, 2.0), (MIX, 0.5)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "DIMENSION",
        subtitle: ("AFTER THE BOSS DC-2 DIMENSION C", "STILL, WIDE STEREO SHIMMER"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![
            Item::Selector { p: MODE, x: 90.0, y: 190.0, label: "MODE", positions: &["1", "2", "3", "4"], tip: "Four depths, like the DC-2's buttons." },
            Item::Knob { p: MIX, x: 320.0, y: 190.0, r: 40.0, label: "MIX", tip: "Amount of the effect.", active: None },
            Item::Knob { p: OUTPUT, x: 550.0, y: 190.0, r: 40.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Dimension", class_id: *b"PqdDimensionC2Q1", vendor: "PEQUOD Pedals",
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
    fn wide() {
        let p = with(defaults(), &[(MODE, 3.0)]);
        let (l, r) = stereo(&p, &sine(800.0, 0.2, 48000));
        let c = corr(&l[24000..], &r[24000..]);
        println!("L/R correlation {c:.2}");
        assert!(c < 0.97);
    }

}
