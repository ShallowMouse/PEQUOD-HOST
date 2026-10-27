//! Auto-Pan: a stereo auto-panner that sweeps your guitar between the speakers, free-running or locked to the host's tempo, with sine, triangle or square movement.

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


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Shape { #[name = "Sine"] Sine, #[name = "Triangle"] Tri, #[name = "Square"] Sq }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Sync { #[name = "Free"] Free, #[name = "1 bar"] Bar, #[name = "1/2"] Half, #[name = "1/4"] Q, #[name = "1/8"] E, #[name = "1/16"] S }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "rate"]
    pub rate: FloatParam,
    #[id = "sync"]
    pub sync: EnumParam<Sync>,
    #[id = "shape"]
    pub shape: EnumParam<Shape>,
    #[id = "depth"]
    pub depth: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            rate: rate("Rate", 1.0, 0.05, 10.0),
            sync: EnumParam::new("Sync", Sync::Free),
            shape: EnumParam::new("Shape", Shape::Sine),
            depth: pct("Depth", 1.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const RATE: usize = 0;
pub const SYNC: usize = 1;
pub const SHAPE: usize = 2;
pub const DEPTH: usize = 3;
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
            self.sync.as_ptr(),
            self.shape.as_ptr(),
            self.depth.as_ptr(),
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

/// phase 0..1 -> -1..1; shape 0 sine, 1 triangle, 2 square (softened)
#[inline]
pub fn shape(phase: f64, s: usize) -> f64 {
    match s {
        0 => (2.0 * std::f64::consts::PI * phase).sin(),
        1 => 1.0 - 4.0 * (phase - 0.25).rem_euclid(1.0).sub_half_abs(),
        _ => ((2.0 * std::f64::consts::PI * phase).sin() * 6.0).tanh(),
    }
}
trait SubHalf {
    fn sub_half_abs(self) -> f64;
}
impl SubHalf for f64 {
    #[inline]
    fn sub_half_abs(self) -> f64 {
        (self - 0.5).abs()
    }
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ph: f64,
    tempo: Option<f64>,
    beats: Option<f64>,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, ph: 0.0, tempo: None, beats: None, out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.ph = 0.0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn transport(&mut self, tempo: Option<f64>, beats: Option<f64>, _playing: bool) {
        self.tempo = tempo;
        self.beats = beats;
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let sync = self.v[SYNC].round() as usize;
        let shp = self.v[SHAPE].round() as usize;
        let depth = self.v[DEPTH] as f64;
        let beats_per_cycle = [0.0, 4.0, 2.0, 1.0, 0.5, 0.25][sync.min(5)];
        let hz = match (sync, self.tempo) {
            (s, Some(t)) if s > 0 => t / 60.0 / beats_per_cycle,
            _ => self.v[RATE] as f64,
        };
        if sync > 0 {
            if let Some(b) = self.beats {
                self.ph = (b / beats_per_cycle).rem_euclid(1.0);
            }
        }
        let nch = chans.len();
        for k in 0..n {
            self.ph = (self.ph + hz / self.fs).fract();
            let p = (shape(self.ph, shp) * depth).clamp(-1.0, 1.0);
            let th = (p + 1.0) * std::f64::consts::FRAC_PI_4;
            let (gl, gr) = (th.cos() * std::f64::consts::SQRT_2, th.sin() * std::f64::consts::SQRT_2);
            let o = self.out.next();
            if nch >= 2 {
                let x = 0.5 * (chans[0][k] as f64 + chans[1][k] as f64);
                chans[0][k] = (x * gl * o) as f32;
                chans[1][k] = (x * gr * o) as f32;
            } else if nch == 1 {
                chans[0][k] = (chans[0][k] as f64 * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Slow Sweep", values: &[(RATE, 0.5)] },
        Preset { name: "Quarter-Note Pan (synced)", values: &[(SYNC, 3.0)] },
        Preset { name: "Eighth-Note Ping (square)", values: &[(SYNC, 4.0), (SHAPE, 2.0)] },
        Preset { name: "One Bar Drift", values: &[(SYNC, 1.0), (DEPTH, 0.7)] },
        Preset { name: "Fast Triangle", values: &[(RATE, 4.0), (SHAPE, 1.0)] },
        Preset { name: "Gentle Width", values: &[(RATE, 0.3), (DEPTH, 0.4)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "AUTO-PAN",
        subtitle: ("STEREO AUTO-PANNER", "FREE OR TEMPO-SYNCED"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![
            Item::Knob { p: RATE, x: 90.0, y: 190.0, r: 34.0, label: "RATE", tip: "Speed when not synced.", active: None },
            Item::Selector { p: SYNC, x: 205.0, y: 190.0, label: "SYNC", positions: &["FREE", "1", "1/2", "1/4", "1/8", "1/16"], tip: "Lock to the host tempo." },
            Item::Selector { p: SHAPE, x: 320.0, y: 190.0, label: "SHAPE", positions: &["SINE", "TRI", "SQR"], tip: "Movement shape." },
            Item::Knob { p: DEPTH, x: 435.0, y: 190.0, r: 34.0, label: "DEPTH", tip: "How far it pans.", active: None },
            Item::Knob { p: OUTPUT, x: 550.0, y: 190.0, r: 34.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Auto-Pan", class_id: *b"PqdAutoPanStereo", vendor: "PEQUOD Pedals",
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
    fn pans() {
        let (l, r) = stereo(&with(defaults(), &[(RATE, 2.0)]), &sine(500.0, 0.2, 48000));
        let el: Vec<f32> = env(&l, 480).iter().map(|v| *v as f32).collect();
        let er: Vec<f32> = env(&r, 480).iter().map(|v| *v as f32).collect();
        let m = |e: &[f32]| e.iter().map(|v| *v as f64).sum::<f64>() / e.len() as f64;
        let (a, b) = (m(&el), m(&er));
        let x: Vec<f32> = el.iter().map(|v| (*v as f64 - a) as f32).collect();
        let y: Vec<f32> = er.iter().map(|v| (*v as f64 - b) as f32).collect();
        let c = corr(&x, &y);
        println!("L/R envelope correlation {c:.2}");
        assert!(c < -0.8);
    }

}
