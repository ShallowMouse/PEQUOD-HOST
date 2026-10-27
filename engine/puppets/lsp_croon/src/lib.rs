//! Croon Vocal: a vocal chain for the Turner and Kane baritone croon of The Age of the Understatement, after Scott Walker's late-60s records: smooth COMP, WARMTH from up close, a bright PLATE, a SLAP echo, and DOUBLE for two singers on one line. Settings estimated.

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
    #[id = "comp"]
    pub comp: FloatParam,
    #[id = "warmth"]
    pub warmth: FloatParam,
    #[id = "plate"]
    pub plate: FloatParam,
    #[id = "slap"]
    pub slap: FloatParam,
    #[id = "double"]
    pub double: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            comp: pct("Comp", 0.5),
            warmth: pct("Warmth", 0.5),
            plate: pct("Plate", 0.3),
            slap: pct("Slap", 0.15),
            double: pct("Double", 0.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const COMP: usize = 0;
pub const WARMTH: usize = 1;
pub const PLATE: usize = 2;
pub const SLAP: usize = 3;
pub const DOUBLE: usize = 4;
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
            self.comp.as_ptr(),
            self.warmth.as_ptr(),
            self.plate.as_ptr(),
            self.slap.as_ptr(),
            self.double.as_ptr(),
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
    eq: [Biquad; 3],
    fdn: Fdn,
    slap: DelayLine,
    dbl: [DelayLine; 2],
    w: [Wander; 2],
    rms: f64,
    gr: f64,
    key: i32,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let w = self.v[WARMTH] as f64;
        self.eq[0].set_highpass(fs, 70.0, 0.7);
        self.eq[1].set_low_shelf(fs, 180.0, 0.7, 5.0 * w);
        self.eq[2].set_high_shelf(fs, 5000.0, 0.7, 2.0 - 3.0 * w);
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut fdn = Fdn::new(fs, 1.2);
        fdn.configure(1.1, [1.6, 1.8, 1.5], 7000.0);
        let mut d = Self { fs, v, eq: [Biquad::identity(); 3], fdn, slap: DelayLine::new((0.2 * fs) as usize),
            dbl: [DelayLine::new((0.05 * fs) as usize), DelayLine::new((0.05 * fs) as usize)], w: [Wander::new(fs, 0.7, 3), Wander::new(fs, 0.6, 8)],
            rms: 0.0, gr: 0.0, key: -999, out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.eq.iter_mut().for_each(|b| b.reset());
        self.fdn.reset();
        self.slap.reset();
        self.dbl.iter_mut().for_each(|l| l.reset());
        self.rms = 0.0;
        self.gr = 0.0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = (v[WARMTH] * 100.0) as i32;
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let thr = -8.0 - 24.0 * self.v[COMP] as f64;
        let (plate, slap, dbl) = (self.v[PLATE] as f64, self.v[SLAP] as f64, self.v[DOUBLE] as f64);
        let cav = coeff(self.fs, 10.0);
        let ca = coeff(self.fs, 8.0);
        let cr = coeff(self.fs, 250.0);
        let st = 0.13 * self.fs;
        let (d1, d2, sw) = (0.024 * self.fs, 0.031 * self.fs, 0.0012 * self.fs);
        let mut most = 0.0f64;
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            let e = cascade(&mut self.eq, x);
            self.rms = e * e + cav * (self.rms - e * e);
            let t = comp_gr(level_db(self.rms), thr, 3.0, 8.0);
            self.gr = t + if t > self.gr { ca } else { cr } * (self.gr - t);
            most = most.max(self.gr);
            let v = e * db_to_gain(-self.gr + 0.5 * self.gr.min(14.0));
            self.slap.write(v);
            let s = self.slap.read(st);
            self.dbl[0].write(v);
            self.dbl[1].write(v);
            let a = self.dbl[0].read(d1 + sw * self.w[0].next());
            let b = self.dbl[1].read(d2 + sw * self.w[1].next());
            let dx = self.fdn.diffuse(v + 0.5 * s, 3);
            let (pl, pr) = self.fdn.process(dx);
            let o = self.out.next();
            let lead = v + slap * 0.5 * s;
            let l = (lead * (1.0 - 0.3 * dbl) + dbl * 0.8 * a + plate * 0.6 * pl) * o;
            let r = (lead * (1.0 - 0.3 * dbl) + dbl * 0.8 * b + plate * 0.6 * pr) * o;
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
        Preset { name: "Baritone Croon", values: &[] },
        Preset { name: "Two Singers, One Line", values: &[(DOUBLE, 0.7), (PLATE, 0.35)] },
        Preset { name: "Scott Walker Plate", values: &[(PLATE, 0.6), (WARMTH, 0.6), (SLAP, 0.1)] },
        Preset { name: "Slapback Shout", values: &[(SLAP, 0.5), (COMP, 0.7), (PLATE, 0.15)] },
        Preset { name: "Close and Dry", values: &[(PLATE, 0.05), (SLAP, 0.0), (WARMTH, 0.7)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "CROON VOCAL",
        subtitle: ("FOR A BARITONE CROON", "COMP · WARMTH · PLATE · SLAP · DOUBLE"),
        accent: ui::STEEL,
        size: (W, H),
        items: vec![

            Item::Knob { p: COMP, x: 90.0, y: 160.0, r: 38.0, label: "COMP", tip: "Smooth compression.", active: None },
            Item::Knob { p: WARMTH, x: 220.0, y: 160.0, r: 32.0, label: "WARMTH", tip: "Close-mic warmth: more low end, softer top.", active: None },
            Item::Meter { m: 0, x: 300.0, y: 96.0, w: 160.0, h: 110.0, kind: MeterKind::GainReduction, caption: "COMP" },
            Item::Knob { p: PLATE, x: 550.0, y: 160.0, r: 32.0, label: "PLATE", tip: "A bright plate reverb.", active: None },
            Item::Divider { y: 240.0 },
            Item::Knob { p: SLAP, x: 150.0, y: 305.0, r: 26.0, label: "SLAP", tip: "A slap echo (130 ms).", active: None },
            Item::Knob { p: DOUBLE, x: 320.0, y: 305.0, r: 26.0, label: "DOUBLE", tip: "Two singers on one line, left and right.", active: None },
            Item::Knob { p: OUTPUT, x: 490.0, y: 305.0, r: 26.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Croon Vocal", class_id: *b"LSPCroonVocalPQ1", vendor: "PEQUOD Last Shadow Puppets",
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
    fn double_spreads() {
        let (l, r) = stereo(&with(defaults(), &[(DOUBLE, 1.0), (PLATE, 0.0), (SLAP, 0.0)]), &sine(220.0, 0.2, 48000));
        let c = corr(&l[24000..], &r[24000..]);
        println!("L/R corr {c:.2}");
        assert!(c < 0.95);
    }

}
