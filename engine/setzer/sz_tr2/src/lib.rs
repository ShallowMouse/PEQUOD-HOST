//! TR-2: tremolo after the Boss TR-2 Brian Setzer keeps on his board for the odd song. Rate, depth and a wave knob from smooth triangle to choppy square.

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
pub const H: u32 = 470;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "rate"]
    pub rate: FloatParam,
    #[id = "depth"]
    pub depth: FloatParam,
    #[id = "wave"]
    pub wave: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            rate: rate("Rate", 4.0, 0.5, 12.0),
            depth: pct("Depth", 0.5),
            wave: pct("Wave", 0.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const RATE: usize = 0;
pub const DEPTH: usize = 1;
pub const WAVE: usize = 2;
pub const OUTPUT: usize = 3;
pub const NPARAMS: usize = 4;

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
            self.depth.as_ptr(),
            self.wave.as_ptr(),
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
    phase: f64,
    g: f64,
    gc: f64,
    out: Smooth,
}

impl Dsp {
    /// LFO in -1..1 from triangle (wave 0) to near-square (wave 1).
    pub fn lfo(phase: f64, wave: f64) -> f64 {
        let tri = 1.0 - 4.0 * (phase - 0.5).abs();
        let k = 1.0 + 24.0 * wave * wave;
        (k * tri).tanh() / k.tanh()
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, phase: 0.0, g: 1.0, gc: coeff(fs, 1.0), out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.phase = 0.0;
        self.g = 1.0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let rate = self.v[RATE] as f64;
        let depth = self.v[DEPTH] as f64;
        let wave = self.v[WAVE] as f64;
        let mut last = 0.0;
        for k in 0..n {
            self.phase = (self.phase + rate / self.fs).fract();
            let s = Dsp::lfo(self.phase, wave);
            let target = 1.0 - depth * (0.5 - 0.5 * s);
            // the VCA can't jump instantly (a ~1 ms slew, so the square doesn't click)
            self.g = target + self.gc * (self.g - target);
            let g = self.g * self.out.next();
            for ch in chans.iter_mut() {
                ch[k] = (ch[k] as f64 * g) as f32;
            }
            last = s;
        }
        meters[0] = (0.5 + 0.5 * last) as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Setzer Slow Shimmer", values: &[(RATE, 3.5), (DEPTH, 0.4)] },
        Preset { name: "Surf Trem", values: &[(RATE, 6.0), (DEPTH, 0.7), (WAVE, 0.3)] },
        Preset { name: "Choppy Square", values: &[(RATE, 5.0), (DEPTH, 0.9), (WAVE, 1.0)] },
        Preset { name: "Fast Flutter", values: &[(RATE, 10.0), (DEPTH, 0.5)] },
        Preset { name: "Deep and Slow", values: &[(RATE, 1.5), (DEPTH, 0.85), (WAVE, 0.2)] },
        Preset { name: "Subtle Movement", values: &[(RATE, 4.5), (DEPTH, 0.2)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "TR-2",
        subtitle: ("AFTER THE BOSS TR-2", "TREMOLO · RATE · DEPTH · WAVE"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![

            Item::Knob { p: RATE, x: 130.0, y: 190.0, r: 46.0, label: "RATE", tip: "Tremolo speed.", active: None },
            Item::Knob { p: DEPTH, x: 320.0, y: 190.0, r: 46.0, label: "DEPTH", tip: "How far the volume dips.", active: None },
            Item::Knob { p: WAVE, x: 510.0, y: 190.0, r: 46.0, label: "WAVE", tip: "The TR-2's wave knob: smooth triangle (left) to choppy square (right).", active: None },
            Item::Lamp { m: 0, x: 320.0, y: 296.0, label: "RATE", lo: 0.0, hi: 1.0 },
            Item::Divider { y: 322.0 },
            Item::Knob { p: OUTPUT, x: 320.0, y: 368.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "TR-2 Tremolo", class_id: *b"BSzTR2TremoloPQ1", vendor: "PEQUOD Brian Setzer",
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

    #[test]
    fn depth_and_rate() {
        let p = with(defaults(), &[(RATE, 4.0), (DEPTH, 1.0), (WAVE, 1.0)]);
        let mut input = vec![vec![0.5f32; 48000], vec![0.5f32; 48000]];
        run(&p, &mut input);
        let y = &input[0];
        let (lo, hi) = y.iter().fold((9.0f32, 0.0f32), |(a, b), v| (a.min(*v), b.max(*v)));
        // count rising crossings of the mid level: one per cycle
        let mid = 0.5 * (lo + hi);
        let cyc = y.windows(2).filter(|w| w[0] < mid && w[1] >= mid).count();
        println!("min {lo:.3} max {hi:.3}, {cyc} cycles in 1 s");
        assert!(lo < 0.02 && hi > 0.49 && (cyc as i32 - 4).abs() <= 1);
    }
    #[test]
    fn zero_depth_passes() {
        let p = with(defaults(), &[(DEPTH, 0.0)]);
        let mut input = vec![vec![0.3f32; 4800], vec![0.3f32; 4800]];
        run(&p, &mut input);
        assert!(input[0].iter().all(|v| (v - 0.3).abs() < 1e-4));
    }

}
