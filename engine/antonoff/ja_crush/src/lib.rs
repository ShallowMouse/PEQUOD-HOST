//! Room Crush: room and extra drum mics squashed and overdriven, as on Antonoff's short, compressed, distorted drums: a vari-mu compressor with the six Fairchild 670 time-constant positions (the Fairman TMC on Sisk's room mics), plus an overdriven mic blend.

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



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "input"]
    pub input: FloatParam,
    #[id = "thresh"]
    pub thresh: FloatParam,
    #[id = "tc"]
    pub tc: IntParam,
    #[id = "drive"]
    pub drive: FloatParam,
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
            input: db("Input", 0.0, -12.0, 24.0),
            thresh: pct("Threshold", 0.5),
            tc: IntParam::new("Time Constant", 2, IntRange::Linear { min: 1, max: 6 }),
            drive: pct("Overdrive", 0.3),
            mix: pct("Mix", 1.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const INPUT: usize = 0;
pub const THRESH: usize = 1;
pub const TC: usize = 2;
pub const DRIVE: usize = 3;
pub const MIX: usize = 4;
pub const OUTPUT: usize = 5;
pub const NPARAMS: usize = 6;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.input.as_ptr(),
            self.thresh.as_ptr(),
            self.tc.as_ptr(),
            self.drive.as_ptr(),
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

// Fairchild 670 time constants: (attack ms, release ms, second slower release for 5/6)
const TCS: [(f64, f64, f64); 6] = [(0.2, 300.0, 0.0), (0.2, 800.0, 0.0), (0.4, 2000.0, 0.0), (0.4, 5000.0, 0.0), (0.4, 2000.0, 10000.0), (0.2, 300.0, 10000.0)];

pub struct Dsp {
    fs: f64,
    gr_fast: f64,
    gr_slow: f64,
    os: [Oversampler; 2],
    v: [f32; NPARAMS],
    input: Smooth,
    mix: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, gr_fast: 0.0, gr_slow: 0.0, os: [Oversampler::new(), Oversampler::new()], v, input: Smooth::new(fs, 20.0, 1.0), mix: Smooth::new(fs, 20.0, 1.0), out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.gr_fast = 0.0;
        self.gr_slow = 0.0;
        self.os.iter_mut().for_each(|o| o.reset());
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.input.set(db_to_gain(v[INPUT] as f64));
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let (a, r1, r2) = TCS[((self.v[TC].round() as usize).clamp(1, 6)) - 1];
        let ca = coeff(self.fs, a);
        let cr1 = coeff(self.fs, r1);
        let cr2 = if r2 > 0.0 { coeff(self.fs, r2) } else { cr1 };
        let thr = -4.0 - 36.0 * self.v[THRESH] as f64;
        let drive = self.v[DRIVE] as f64;
        let dg = 1.0 + 15.0 * drive;
        let mut grmax = 0.0f64;
        for k in 0..n {
            let gi = self.input.next();
            let mut lvl = 0.0f64;
            for c in chans.iter() {
                lvl = lvl.max((c[k] as f64 * gi).abs());
            }
            // vari-mu: the ratio rises the further over threshold (soft, wide knee)
            let over = (gain_to_db(lvl) - thr).max(0.0);
            let target = over * (over / (over + 6.0)) * 0.9;
            // programme-dependent (5/6): fast release for peaks, slow for sustained reduction
            self.gr_fast = if target > self.gr_fast { target + ca * (self.gr_fast - target) } else { target + cr1 * (self.gr_fast - target) };
            self.gr_slow = if target > self.gr_slow { target + coeff(self.fs, 50.0) * (self.gr_slow - target) } else { target + cr2 * (self.gr_slow - target) };
            let gr = if r2 > 0.0 { self.gr_fast.max(self.gr_slow * 0.7) } else { self.gr_fast };
            grmax = grmax.max(gr);
            let g = db_to_gain(-gr + 0.6 * gr.min(20.0));
            let mx = self.mix.next();
            let o = self.out.next();
            for c in 0..nch.min(2) {
                let x = chans[c][k] as f64;
                let y = x * gi * g;
                // valve make-up stage, and the overdriven extra mics blended in
                let y = self.os[c].process(y, |s| {
                    let clean = tube(s, 0.12);
                    let dirty = (s * dg).tanh() / dg.sqrt();
                    clean * (1.0 - drive) + dirty * drive
                });
                chans[c][k] = ((x * (1.0 - mx) + y * mx) * o) as f32;
            }
        }
        meters[0] = grmax as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Room Mics (Sisk, TC 2)", values: &[(THRESH, 0.55), (TC, 2.0), (DRIVE, 0.1)] },
        Preset { name: "Short Compressed Distorted Drums", values: &[(INPUT, 8.0), (THRESH, 0.8), (TC, 1.0), (DRIVE, 0.6), (OUTPUT, -6.0)] },
        Preset { name: "Parallel Smash", values: &[(INPUT, 10.0), (THRESH, 0.9), (TC, 1.0), (DRIVE, 0.7), (MIX, 0.4), (OUTPUT, -4.0)] },
        Preset { name: "Breathing Room (TC 6)", values: &[(THRESH, 0.7), (TC, 6.0), (DRIVE, 0.2)] },
        Preset { name: "Gentle Glue (TC 5)", values: &[(THRESH, 0.35), (TC, 5.0), (DRIVE, 0.0)] },
        Preset { name: "Overdriven Extra Mic", values: &[(THRESH, 0.4), (TC, 2.0), (DRIVE, 1.0), (OUTPUT, -6.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "ROOM CRUSH",
        subtitle: ("VARI-MU ROOM MICS + OVERDRIVE", "SHORT · COMPRESSED · DISTORTED DRUMS"),
        accent: ui::MUSTARD,
        size: (W, H),
        items: vec![

            Item::Knob { p: INPUT, x: 90.0, y: 165.0, r: 40.0, label: "INPUT", tip: "Input level.", active: None },
            Item::Knob { p: THRESH, x: 200.0, y: 165.0, r: 30.0, label: "THRESHOLD", tip: "Threshold: up = more compression.", active: None },
            Item::Meter { m: 0, x: 270.0, y: 96.0, w: 200.0, h: 128.0, kind: MeterKind::GainReduction, caption: "VARI-MU" },
            Item::Selector { p: TC, x: 550.0, y: 165.0, label: "TIME CONSTANT", positions: &["1", "2", "3", "4", "5", "6"], tip: "The Fairchild's six attack/release positions: 1-4 fixed (0.3 s to 5 s release), 5 and 6 programme-dependent." },
            Item::Divider { y: 264.0 },
            Item::Knob { p: DRIVE, x: 140.0, y: 330.0, r: 24.0, label: "OVERDRIVE", tip: "Blend of the overdriven extra mics.", active: None },
            Item::Knob { p: MIX, x: 320.0, y: 330.0, r: 24.0, label: "MIX", tip: "Dry vs crushed (parallel).", active: None },
            Item::Knob { p: OUTPUT, x: 500.0, y: 330.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Room Crush", class_id: *b"JARoomCrushPQD01", vendor: "PEQUOD Antonoff",
    categories: [Fx, Dynamics]
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
    fn more_threshold_more_reduction() {
        let gr = |t: f32| {
            let p = with(defaults(), &[(THRESH, t), (DRIVE, 0.0)]);
            let mut input = vec![sine(200.0, 0.3, 48000)];
            run(&p, &mut input)[0]
        };
        let (a, b) = (gr(0.2), gr(0.9));
        println!("GR thresh 0.2 {a:.1}, 0.9 {b:.1}");
        assert!(b > a + 6.0);
    }

}
