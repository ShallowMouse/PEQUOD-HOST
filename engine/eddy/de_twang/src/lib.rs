//! Twang Booster: for twangy melodies on the bass strings of a guitar, a baritone or a six-string bass, Duane Eddy-style (he found the low strings recorded more powerfully, and picked hard near the bridge). COMP evens and sustains the notes, PLUCK sharpens each pick attack, LOWS adds weight and BITE the bridge-pickup midrange.

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

pub const W: u32 = 640;
pub const H: u32 = 410;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "comp"]
    pub comp: FloatParam,
    #[id = "pluck"]
    pub pluck: FloatParam,
    #[id = "lows"]
    pub lows: FloatParam,
    #[id = "bite"]
    pub bite: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            comp: pct("Comp", 0.5),
            pluck: pct("Pluck", 0.5),
            lows: db("Lows", 3.0, -6.0, 9.0),
            bite: db("Bite", 3.0, -3.0, 9.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const COMP: usize = 0;
pub const PLUCK: usize = 1;
pub const LOWS: usize = 2;
pub const BITE: usize = 3;
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
            self.comp.as_ptr(),
            self.pluck.as_ptr(),
            self.lows.as_ptr(),
            self.bite.as_ptr(),
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

struct Chan {
    eq: [Biquad; 2],
    pk: Biquad,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ch: Vec<Chan>,
    fast: f64,
    slow: f64,
    rms: f64,
    gr: f64,
    key: (i32, i32),
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let (lo, bi) = (self.v[LOWS] as f64, self.v[BITE] as f64);
        for c in self.ch.iter_mut() {
            c.eq[0].set_low_shelf(fs, 120.0, 0.7, lo);
            c.eq[1].set_peak(fs, 1500.0, 0.9, bi);
            c.pk.set_bandpass(fs, 3000.0, 0.8);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mk = || Chan { eq: [Biquad::identity(); 2], pk: Biquad::identity() };
        let mut d = Self { fs, v, ch: (0..ch.max(2)).map(|_| mk()).collect(), fast: 0.0, slow: 0.0, rms: 0.0, gr: 0.0, key: (-999, -999), out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        for c in self.ch.iter_mut() {
            c.eq.iter_mut().for_each(|b| b.reset());
            c.pk.reset();
        }
        self.fast = 0.0;
        self.slow = 0.0;
        self.rms = 0.0;
        self.gr = 0.0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = ((v[LOWS] * 10.0) as i32, (v[BITE] * 10.0) as i32);
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().max(1) as f64;
        let thr = -8.0 - 26.0 * self.v[COMP] as f64;
        let pluck = self.v[PLUCK] as f64;
        let cf = coeff(self.fs, 1.0);
        let cs = coeff(self.fs, 40.0);
        let cav = coeff(self.fs, 8.0);
        let ca = coeff(self.fs, 6.0);
        let cr = coeff(self.fs, 200.0);
        let last = self.ch.len() - 1;
        let mut most = 0.0f64;
        for k in 0..n {
            let mut key = 0.0;
            let mut a = 0.0f64;
            for c in chans.iter() {
                let x = c[k] as f64;
                key += x * x;
                a = a.max(x.abs());
            }
            key /= nch;
            self.fast = a + cf * (self.fast - a);
            self.slow = a + cs * (self.slow - a);
            let att = ((self.fast / (self.slow + 1e-6)) - 1.0).clamp(0.0, 3.0);
            self.rms = key + cav * (self.rms - key);
            let lev = 10.0 * self.rms.max(1e-12).log10() + 3.0;
            let t = comp_gr(lev, thr, 4.0, 6.0);
            self.gr = t + if t > self.gr { ca } else { cr } * (self.gr - t);
            most = most.max(self.gr);
            let g = db_to_gain(-self.gr + 0.5 * self.gr.min(14.0));
            let o = self.out.next();
            for (ci, c) in chans.iter_mut().enumerate() {
                let s = &mut self.ch[ci.min(last)];
                let x = c[k] as f64;
                let y = cascade(&mut s.eq, x);
                let y = y + s.pk.process(x) * pluck * 2.0 * att;
                c[k] = (y * g * o) as f32;
            }
        }
        meters[0] = most as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Rebel Rouser Twang", values: &[] },
        Preset { name: "Baritone Melody", values: &[(LOWS, 1.0), (BITE, 4.0), (COMP, 0.6)] },
        Preset { name: "Six-String Bass", values: &[(LOWS, 4.0), (PLUCK, 0.7), (BITE, 5.0)] },
        Preset { name: "Hard Pick, Bridge", values: &[(PLUCK, 0.9), (BITE, 6.0), (COMP, 0.4)] },
        Preset { name: "Smooth Sustain", values: &[(COMP, 0.85), (PLUCK, 0.2)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "TWANG BOOSTER",
        subtitle: ("FOR TWANG ON THE BASS STRINGS", "GUITAR · BARITONE · SIX-STRING BASS"),
        accent: ui::MUSTARD,
        size: (W, H),
        items: vec![

            Item::Knob { p: COMP, x: 90.0, y: 160.0, r: 38.0, label: "COMP", tip: "Evens and sustains the notes.", active: None },
            Item::Knob { p: PLUCK, x: 220.0, y: 160.0, r: 32.0, label: "PLUCK", tip: "Sharpens each pick attack.", active: None },
            Item::Meter { m: 0, x: 300.0, y: 96.0, w: 160.0, h: 110.0, kind: MeterKind::GainReduction, caption: "COMP" },
            Item::Knob { p: BITE, x: 550.0, y: 160.0, r: 32.0, label: "BITE", tip: "Bridge-pickup midrange at 1.5 kHz.", active: None },
            Item::Divider { y: 240.0 },
            Item::Knob { p: LOWS, x: 220.0, y: 305.0, r: 26.0, label: "LOWS", tip: "Weight below 120 Hz.", active: None },
            Item::Knob { p: OUTPUT, x: 420.0, y: 305.0, r: 26.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Twang Booster", class_id: *b"DETwangBoosterQ1", vendor: "PEQUOD Duane Eddy",
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
    fn comp_evens() {
        let p = with(defaults(), &[(COMP, 0.9), (PLUCK, 0.0)]);
        let loud = db(rms(&stereo(&p, &sine(110.0, 0.5, 48000)).0[24000..]));
        let quiet = db(rms(&stereo(&p, &sine(110.0, 0.05, 48000)).0[24000..]));
        println!("20 dB in -> {:.1} dB out", loud - quiet);
        assert!(loud - quiet < 12.0);
    }

}
