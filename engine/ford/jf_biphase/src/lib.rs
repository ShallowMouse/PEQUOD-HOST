//! Bi-Phase: two six-stage phasers with two sweep generators, after the Mu-Tron Bi-Phase in James Ford's rack. Run them in series for a deep 12-stage phase, or one per side for wide stereo.

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
pub const H: u32 = 432;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Route { #[name = "Series"] Series, #[name = "Stereo"] Stereo }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Source { #[name = "Sweep 1"] One, #[name = "Sweep 2"] Two }
const STAGES: usize = 6;


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "rate1"]
    pub rate1: FloatParam,
    #[id = "shape1"]
    pub shape1: FloatParam,
    #[id = "rate2"]
    pub rate2: FloatParam,
    #[id = "shape2"]
    pub shape2: FloatParam,
    #[id = "deptha"]
    pub depth_a: FloatParam,
    #[id = "fba"]
    pub fb_a: FloatParam,
    #[id = "depthb"]
    pub depth_b: FloatParam,
    #[id = "fbb"]
    pub fb_b: FloatParam,
    #[id = "sourceb"]
    pub source_b: EnumParam<Source>,
    #[id = "route"]
    pub route: EnumParam<Route>,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            rate1: rate("Sweep 1 Rate", 0.3, 0.02, 15.0),
            shape1: pct("Sweep 1 Shape", 0.0),
            rate2: rate("Sweep 2 Rate", 0.45, 0.02, 15.0),
            shape2: pct("Sweep 2 Shape", 0.0),
            depth_a: pct("Depth A", 0.7),
            fb_a: pct("Feedback A", 0.4),
            depth_b: pct("Depth B", 0.7),
            fb_b: pct("Feedback B", 0.4),
            source_b: EnumParam::new("Phasor B Sweep", Source::Two),
            route: EnumParam::new("Routing", Route::Stereo),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const RATE1: usize = 0;
pub const SHAPE1: usize = 1;
pub const RATE2: usize = 2;
pub const SHAPE2: usize = 3;
pub const DEPTHA: usize = 4;
pub const FBA: usize = 5;
pub const DEPTHB: usize = 6;
pub const FBB: usize = 7;
pub const SOURCEB: usize = 8;
pub const ROUTE: usize = 9;
pub const OUTPUT: usize = 10;
pub const NPARAMS: usize = 11;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.rate1.as_ptr(),
            self.shape1.as_ptr(),
            self.rate2.as_ptr(),
            self.shape2.as_ptr(),
            self.depth_a.as_ptr(),
            self.fb_a.as_ptr(),
            self.depth_b.as_ptr(),
            self.fb_b.as_ptr(),
            self.source_b.as_ptr(),
            self.route.as_ptr(),
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
struct Phasor {
    z: [f64; STAGES],
    fb: f64,
}

impl Phasor {
    #[inline]
    fn process(&mut self, x: f64, a: f64, fb: f64) -> f64 {
        let mut y = x + self.fb * fb;
        for z in self.z.iter_mut() {
            // first-order allpass: y = -a*x + z; z = x + a*y  (a from the sweep)
            let o = -a * y + *z;
            *z = y + a * o;
            y = o;
        }
        self.fb = y;
        0.5 * (x + y)
    }
}

pub struct Dsp {
    fs: f64,
    ph: [Phasor; 4],
    sweep: [f64; 2],
    v: [f32; NPARAMS],
    out: Smooth,
}

impl Dsp {
    #[inline]
    fn coef(&self, m: f64, depth: f64) -> f64 {
        // sweep the notch region between ~200 Hz and ~4 kHz (exponential, like the opto cells)
        let f = 200.0 * 2f64.powf(4.3 * (0.5 + 0.5 * m * depth) + 4.3 * (1.0 - depth) * 0.3);
        let t = (std::f64::consts::PI * f / self.fs).tan();
        (1.0 - t) / (1.0 + t)
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, ph: [Phasor::default(); 4], sweep: [0.0, 0.25], v, out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.ph = [Phasor::default(); 4];
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let p = |i: usize| self.v[i] as f64;
        let (r1, s1, r2, s2) = (p(RATE1), p(SHAPE1), p(RATE2), p(SHAPE2));
        let (da, fa, db_, fb) = (p(DEPTHA), p(FBA) * 0.85, p(DEPTHB), p(FBB) * 0.85);
        let b_from_2 = p(SOURCEB) > 0.5;
        let series = p(ROUTE) < 0.5;
        let mut peak = 0.0f64;
        let shape = |ph: f64, s: f64| {
            let sine = (2.0 * std::f64::consts::PI * ph).sin();
            // shape: sine -> squarer (the Bi-Phase's "square" sweep is a rounded square)
            let sq = (sine * 6.0).tanh();
            sine * (1.0 - s) + sq * s
        };
        for k in 0..n {
            self.sweep[0] = (self.sweep[0] + r1 / self.fs).fract();
            self.sweep[1] = (self.sweep[1] + r2 / self.fs).fract();
            let m1 = shape(self.sweep[0], s1);
            let m2 = shape(self.sweep[1], s2);
            let ma = m1;
            let mb = if b_from_2 { m2 } else { m1 };
            let ca = self.coef(ma, da);
            let cb = self.coef(mb, db_);
            let o = self.out.next();
            let xl = chans[0][k] as f64;
            let xr = if nch > 1 { chans[1][k] as f64 } else { xl };
            let (yl, yr) = if series {
                let a0 = self.ph[0].process(xl, ca, fa);
                let l = self.ph[1].process(a0, cb, fb);
                let a2 = self.ph[2].process(xr, ca, fa);
                let r = self.ph[3].process(a2, cb, fb);
                (l, r)
            } else {
                // A on the left, B on the right (from a mono sum, as the unit's stereo outs)
                let m = 0.5 * (xl + xr);
                (self.ph[0].process(m, ca, fa), self.ph[2].process(m, cb, fb))
            };
            let (yl, yr) = (yl * o, yr * o);
            peak = peak.max(yl.abs()).max(yr.abs());
            if nch > 1 {
                chans[0][k] = yl as f32;
                chans[1][k] = yr as f32;
            } else {
                chans[0][k] = (0.5 * (yl + yr)) as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Slow Stereo Swirl", values: &[(RATE1, 0.2), (RATE2, 0.27), (ROUTE, 1.0), (SOURCEB, 1.0)] },
        Preset { name: "Deep 12-Stage", values: &[(ROUTE, 0.0), (SOURCEB, 0.0), (RATE1, 0.15), (DEPTHA, 0.9), (DEPTHB, 0.9), (FBA, 0.6), (FBB, 0.6)] },
        Preset { name: "Square Sweep Chop", values: &[(RATE1, 2.0), (SHAPE1, 1.0), (ROUTE, 1.0), (SOURCEB, 0.0)] },
        Preset { name: "Rotary-ish", values: &[(RATE1, 5.5), (RATE2, 6.3), (DEPTHA, 0.4), (DEPTHB, 0.4), (FBA, 0.1), (FBB, 0.1)] },
        Preset { name: "Synth Pad Phase", values: &[(RATE1, 0.08), (RATE2, 0.11), (FBA, 0.7), (FBB, 0.7)] },
        Preset { name: "Drum Bus Jet", values: &[(ROUTE, 0.0), (RATE1, 0.12), (FBA, 0.85), (FBB, 0.85), (DEPTHA, 1.0), (DEPTHB, 1.0)] },
        Preset { name: "Gentle Wobble", values: &[(RATE1, 0.6), (DEPTHA, 0.3), (DEPTHB, 0.3), (FBA, 0.0), (FBB, 0.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "BI-PHASE",
        subtitle: ("AFTER THE MU-TRON BI-PHASE", "TWO 6-STAGE PHASERS · TWO SWEEPS"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![

            Item::Knob { p: RATE1, x: 70.0, y: 130.0, r: 26.0, label: "SWEEP 1", tip: "Sweep generator 1 rate.", active: None },
            Item::Knob { p: SHAPE1, x: 150.0, y: 130.0, r: 18.0, label: "SHAPE", tip: "Sweep 1 shape: sine to square.", active: None },
            Item::Knob { p: RATE2, x: 230.0, y: 130.0, r: 26.0, label: "SWEEP 2", tip: "Sweep generator 2 rate.", active: None },
            Item::Knob { p: SHAPE2, x: 310.0, y: 130.0, r: 18.0, label: "SHAPE", tip: "Sweep 2 shape.", active: None },
            Item::Switch { p: SOURCEB, x: 420.0, y: 130.0, labels: ["SWEEP 1", "SWEEP 2"], tip: "Which sweep drives Phasor B. Phasor A always follows Sweep 1." },
            Item::Switch { p: ROUTE, x: 550.0, y: 130.0, labels: ["SERIES", "STEREO"], tip: "Series: A into B (12 stages, deep). Stereo: A left, B right." },
            Item::Divider { y: 200.0 },
            Item::Text { x: 160.0, y: 228.0, text: "PHASOR A", size: 13.0, align: Align2::CENTER_CENTER },
            Item::Text { x: 440.0, y: 228.0, text: "PHASOR B", size: 13.0, align: Align2::CENTER_CENTER },
            Item::Knob { p: DEPTHA, x: 110.0, y: 300.0, r: 28.0, label: "DEPTH", tip: "Phasor A sweep depth.", active: None },
            Item::Knob { p: FBA, x: 210.0, y: 300.0, r: 28.0, label: "FEEDBACK", tip: "Phasor A feedback (sharper notches).", active: None },
            Item::Knob { p: DEPTHB, x: 390.0, y: 300.0, r: 28.0, label: "DEPTH", tip: "Phasor B sweep depth.", active: None },
            Item::Knob { p: FBB, x: 490.0, y: 300.0, r: 28.0, label: "FEEDBACK", tip: "Phasor B feedback.", active: None },
            Item::Knob { p: OUTPUT, x: 590.0, y: 300.0, r: 20.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Bi-Phase", class_id: *b"JFBiPhasePQD0001", vendor: "PEQUOD James Ford",
    categories: [Fx, Modulation]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
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
    fn notches_move() {
        let p = with(defaults(), &[(ROUTE, 0.0), (RATE1, 0.5)]);
        let mut nz = vintage::dsp::Noise::new(3);
        let x: Vec<f32> = (0..96000).map(|_| (nz.next() * 0.3) as f32).collect();
        let mut input = vec![x.clone(), x];
        run(&p, &mut input);
        let band = |w: &[f32]| {
            let mut b = vintage::dsp::Biquad::identity();
            b.set_bandpass(48000.0, 800.0, 6.0);
            let y: Vec<f32> = w.iter().map(|v| b.process(*v as f64) as f32).collect();
            rms(&y[200..])
        };
        let levels: Vec<f64> = (0..8).map(|i| db(band(&input[0][48000 + i * 6000..48000 + (i + 1) * 6000]))).collect();
        let spread = levels.iter().cloned().fold(f64::MIN, f64::max) - levels.iter().cloned().fold(f64::MAX, f64::min);
        println!("800 Hz band over one sweep: {spread:.1} dB range");
        assert!(spread > 6.0);
    }

}
