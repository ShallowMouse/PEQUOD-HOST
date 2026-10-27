//! Orchestra Bus: a bus processor for strings and orchestra in the manner of The Age of the Understatement, where Owen Pallett's arrangements for the 22-piece London Metropolitan Orchestra meet 60s Scott Walker and Morricone records: TONE from dark, ribbon-miked 60s to open and modern, WARMTH (tape and low mids), gentle COMP, a HALL and WIDTH. Settings estimated.

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
use vintage::fx::comp_gr;
use vintage::verb::Fdn;

pub const W: u32 = 640;
pub const H: u32 = 410;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "warmth"]
    pub warmth: FloatParam,
    #[id = "comp"]
    pub comp: FloatParam,
    #[id = "hall"]
    pub hall: FloatParam,
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
            tone: pct("Tone", 0.35),
            warmth: pct("Warmth", 0.4),
            comp: pct("Comp", 0.3),
            hall: pct("Hall", 0.3),
            width: pct("Width", 0.6),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const TONE: usize = 0;
pub const WARMTH: usize = 1;
pub const COMP: usize = 2;
pub const HALL: usize = 3;
pub const WIDTH: usize = 4;
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
            self.tone.as_ptr(),
            self.warmth.as_ptr(),
            self.comp.as_ptr(),
            self.hall.as_ptr(),
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

#[inline]
fn level_db(ms: f64) -> f64 {
    10.0 * ms.max(1e-12).log10() + 3.0
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    eq: Vec<[Biquad; 3]>,
    fdn: Fdn,
    rms: f64,
    gr: f64,
    key: Vec<i32>,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let t = self.v[TONE] as f64;
        let w = self.v[WARMTH] as f64;
        for e in self.eq.iter_mut() {
            e[0].set_high_shelf(fs, 6000.0, 0.7, -8.0 + 11.0 * t);
            e[1].set_peak(fs, 250.0, 0.8, 4.0 * w);
            e[2].set_peak(fs, 3500.0, 1.0, -2.0 + 3.0 * t);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut fdn = Fdn::new(fs, 3.2);
        fdn.configure(3.0, [2.4, 2.2, 1.5], 4000.0);
        fdn.mod_depth = 0.002;
        let mut d = Self { fs, v, eq: vec![[Biquad::identity(); 3]; ch.max(2)], fdn, rms: 0.0, gr: 0.0, key: vec![], out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.eq.iter_mut().for_each(|e| e.iter_mut().for_each(|b| b.reset()));
        self.fdn.reset();
        self.rms = 0.0;
        self.gr = 0.0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k: Vec<i32> = [v[TONE], v[WARMTH]].iter().map(|x| (x * 100.0) as i32).collect();
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let warm = 1.0 + 2.0 * self.v[WARMTH] as f64;
        let thr = -10.0 - 20.0 * self.v[COMP] as f64;
        let hall = self.v[HALL] as f64;
        let width = self.v[WIDTH] as f64 * 2.0;
        let cav = coeff(self.fs, 30.0);
        let ca = coeff(self.fs, 30.0);
        let cr = coeff(self.fs, 600.0);
        let last = self.eq.len() - 1;
        let mut most = 0.0f64;
        for k in 0..n {
            let mut key = 0.0;
            for c in chans.iter() {
                key += (c[k] as f64).powi(2);
            }
            key /= nch.max(1) as f64;
            self.rms = key + cav * (self.rms - key);
            let t = comp_gr(level_db(self.rms), thr, 2.0, 10.0);
            self.gr = t + if t > self.gr { ca } else { cr } * (self.gr - t);
            most = most.max(self.gr);
            let g = db_to_gain(-self.gr + 0.5 * self.gr);
            let mut y = [0.0f64; 2];
            for (ci, c) in chans.iter().enumerate().take(2) {
                let e = cascade(&mut self.eq[ci.min(last)], c[k] as f64);
                y[ci] = (e * warm * g).tanh() / warm.tanh();
            }
            if nch == 1 {
                y[1] = y[0];
            }
            let mono = 0.5 * (y[0] + y[1]);
            let dx = self.fdn.diffuse(mono, 4);
            let (rl, rr) = self.fdn.process(dx);
            let m = 0.5 * (y[0] + y[1]);
            let s = 0.5 * (y[0] - y[1]) * width;
            let o = self.out.next();
            let l = (m + s + hall * 0.8 * rl) * o;
            let r = (m - s + hall * 0.8 * rr) * o;
            if nch >= 2 {
                chans[0][k] = l as f32;
                chans[1][k] = r as f32;
            } else {
                chans[0][k] = (0.5 * (l + r)) as f32;
            }
        }
        meters[0] = most as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Age of the Understatement Strings", values: &[] },
        Preset { name: "Dark 60s Orchestra", values: &[(TONE, 0.1), (WARMTH, 0.6), (HALL, 0.4)] },
        Preset { name: "Modern and Open", values: &[(TONE, 0.8), (WARMTH, 0.2), (WIDTH, 0.75)] },
        Preset { name: "Big Hall Swell", values: &[(HALL, 0.7), (COMP, 0.4), (WIDTH, 0.8)] },
        Preset { name: "Dry and Close", values: &[(HALL, 0.0), (WIDTH, 0.5), (TONE, 0.45)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "ORCHESTRA BUS",
        subtitle: ("FOR STRINGS AND ORCHESTRA", "60s DARK TO MODERN · WARMTH · HALL · WIDTH"),
        accent: ui::STEEL,
        size: (W, H),
        items: vec![

            Item::Knob { p: TONE, x: 90.0, y: 160.0, r: 38.0, label: "TONE", tip: "Dark, ribbon-miked 60s (left) to open and modern (right).", active: None },
            Item::Knob { p: WARMTH, x: 220.0, y: 160.0, r: 32.0, label: "WARMTH", tip: "Tape saturation and low-mid weight.", active: None },
            Item::Meter { m: 0, x: 300.0, y: 96.0, w: 160.0, h: 110.0, kind: MeterKind::GainReduction, caption: "COMP" },
            Item::Knob { p: COMP, x: 550.0, y: 160.0, r: 32.0, label: "COMP", tip: "Slow, gentle compression.", active: None },
            Item::Divider { y: 240.0 },
            Item::Knob { p: HALL, x: 150.0, y: 305.0, r: 26.0, label: "HALL", tip: "A concert-hall reverb.", active: None },
            Item::Knob { p: WIDTH, x: 320.0, y: 305.0, r: 26.0, label: "WIDTH", tip: "Stereo width (50% = as recorded).", active: None },
            Item::Knob { p: OUTPUT, x: 490.0, y: 305.0, r: 26.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Orchestra Bus", class_id: *b"LSPOrchestraBus1", vendor: "PEQUOD Last Shadow Puppets",
    categories: [Fx, Dynamics]
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
    fn width_widens() {
        let l = sine(440.0, 0.2, 24000);
        let r = sine(523.0, 0.2, 24000);
        let side = |p: &[f32]| {
            let mut input = vec![l.clone(), r.clone()];
            run(p, &mut input);
            let s: Vec<f32> = input[0][12000..].iter().zip(&input[1][12000..]).map(|(a, b)| a - b).collect();
            db(rms(&s))
        };
        let narrow = side(&with(defaults(), &[(WIDTH, 0.1), (HALL, 0.0)]));
        let wide = side(&with(defaults(), &[(WIDTH, 1.0), (HALL, 0.0)]));
        println!("side level: width 10% {narrow:.1} dB, 100% {wide:.1} dB");
        assert!(wide > narrow + 12.0);
    }

}
