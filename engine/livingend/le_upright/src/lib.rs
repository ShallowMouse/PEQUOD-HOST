//! Owen Upright: a processor for a recorded double bass played Scott Owen-style (slapped, fast and loud): blends a piezo PICKUP sound (the honky mids) with a MIC sound (round, woody lows), lifts the SLAP clicks on each attack, adds GROWL and evens it out with COMP so it sits under loud guitars.

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
    #[id = "blend"]
    pub blend: FloatParam,
    #[id = "slap"]
    pub slap: FloatParam,
    #[id = "growl"]
    pub growl: FloatParam,
    #[id = "comp"]
    pub comp: FloatParam,
    #[id = "body"]
    pub body: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            blend: pct("Pickup / Mic", 0.4),
            slap: pct("Slap", 0.5),
            growl: pct("Growl", 0.2),
            comp: pct("Comp", 0.5),
            body: db("Body", 2.0, -6.0, 9.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const BLEND: usize = 0;
pub const SLAP: usize = 1;
pub const GROWL: usize = 2;
pub const COMP: usize = 3;
pub const BODY: usize = 4;
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
            self.blend.as_ptr(),
            self.slap.as_ptr(),
            self.growl.as_ptr(),
            self.comp.as_ptr(),
            self.body.as_ptr(),
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
    pk: [Biquad; 2],
    mic: [Biquad; 2],
    body: Biquad,
    click: Biquad,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ch: Vec<Chan>,
    fast: f64,
    slow: f64,
    rms: f64,
    gr: f64,
    key: i32,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let b = self.v[BODY] as f64;
        for c in self.ch.iter_mut() {
            c.pk[0].set_highpass(fs, 60.0, 0.7);
            c.pk[1].set_peak(fs, 1200.0, 1.0, 5.0);
            c.mic[0].set_low_shelf(fs, 110.0, 0.7, 3.0);
            c.mic[1].set_lowpass(fs, 6000.0, 0.7);
            c.body.set_peak(fs, 120.0, 1.0, b);
            c.click.set_highpass(fs, 2000.0, 0.7);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mk = || Chan { pk: [Biquad::identity(); 2], mic: [Biquad::identity(); 2], body: Biquad::identity(), click: Biquad::identity() };
        let mut d = Self { fs, v, ch: (0..ch.max(2)).map(|_| mk()).collect(), fast: 0.0, slow: 0.0, rms: 0.0, gr: 0.0, key: -999, out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        for c in self.ch.iter_mut() {
            c.pk.iter_mut().chain(c.mic.iter_mut()).for_each(|b| b.reset());
            c.body.reset();
            c.click.reset();
        }
        self.fast = 0.0;
        self.slow = 0.0;
        self.rms = 0.0;
        self.gr = 0.0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = (v[BODY] * 10.0) as i32;
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().max(1) as f64;
        let blend = self.v[BLEND] as f64;
        let slap = self.v[SLAP] as f64;
        let growl = 1.0 + 6.0 * self.v[GROWL] as f64;
        let thr = -8.0 - 24.0 * self.v[COMP] as f64;
        let cf = coeff(self.fs, 1.0);
        let cs = coeff(self.fs, 40.0);
        let cav = coeff(self.fs, 10.0);
        let ca = coeff(self.fs, 5.0);
        let cr = coeff(self.fs, 120.0);
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
            // on each attack the fast envelope leaps above the slow one: lift the clicks then
            let att = ((self.fast / (self.slow + 1e-6)) - 1.0).clamp(0.0, 3.0);
            self.rms = key + cav * (self.rms - key);
            let lev = 10.0 * self.rms.max(1e-12).log10() + 3.0;
            let t = comp_gr(lev, thr, 4.0, 6.0);
            self.gr = t + if t > self.gr { ca } else { cr } * (self.gr - t);
            most = most.max(self.gr);
            let g = db_to_gain(-self.gr + 0.5 * self.gr.min(12.0));
            let o = self.out.next();
            for (ci, c) in chans.iter_mut().enumerate() {
                let s = &mut self.ch[ci.min(last)];
                let x = c[k] as f64;
                let p = cascade(&mut s.pk, x);
                let m = cascade(&mut s.mic, x);
                let y = p * blend + m * (1.0 - blend);
                let y = s.body.process(y);
                let y = (y * growl).tanh() / growl.tanh().max(0.3);
                let y = y + s.click.process(x) * slap * 2.5 * att;
                c[k] = (y * g * o) as f32;
            }
        }
        meters[0] = most as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Scott Owen Slap", values: &[] },
        Preset { name: "More Pickup Honk", values: &[(BLEND, 0.75), (GROWL, 0.3)] },
        Preset { name: "Woody and Round", values: &[(BLEND, 0.1), (SLAP, 0.3), (BODY, 4.0)] },
        Preset { name: "Punk Bass Wall", values: &[(GROWL, 0.6), (COMP, 0.8), (SLAP, 0.6)] },
        Preset { name: "Rockabilly Click", values: &[(SLAP, 0.9), (COMP, 0.4)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "OWEN UPRIGHT",
        subtitle: ("FOR A SLAPPED DOUBLE BASS", "PICKUP / MIC · SLAP · GROWL · COMP"),
        accent: ui::MUSTARD,
        size: (W, H),
        items: vec![

            Item::Knob { p: BLEND, x: 90.0, y: 160.0, r: 38.0, label: "PICKUP / MIC", tip: "Mic (round, woody) to piezo pickup (honky, cutting).", active: None },
            Item::Knob { p: SLAP, x: 220.0, y: 160.0, r: 32.0, label: "SLAP", tip: "Lifts the slap clicks on each attack.", active: None },
            Item::Meter { m: 0, x: 300.0, y: 96.0, w: 160.0, h: 110.0, kind: MeterKind::GainReduction, caption: "COMP" },
            Item::Knob { p: COMP, x: 550.0, y: 160.0, r: 32.0, label: "COMP", tip: "Evens out the level.", active: None },
            Item::Divider { y: 240.0 },
            Item::Knob { p: GROWL, x: 150.0, y: 305.0, r: 26.0, label: "GROWL", tip: "Light drive.", active: None },
            Item::Knob { p: BODY, x: 320.0, y: 305.0, r: 26.0, label: "BODY", tip: "Weight at 120 Hz.", active: None },
            Item::Knob { p: OUTPUT, x: 490.0, y: 305.0, r: 26.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Owen Upright", class_id: *b"LEUprightSlapPQ1", vendor: "PEQUOD The Living End",
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

    #[test]
    fn slap_lifts_attacks() {
        let mut nz = vintage::dsp::Noise::new(2);
        let mut x = vec![0.0f32; 48000];
        for h in 0..4 {
            for k in 0..12000 {
                let t = k as f64 / 48000.0;
                let click = if k < 200 { 0.3 * nz.next() } else { 0.0 };
                x[h * 12000 + k] = ((0.3 * (2.0 * std::f64::consts::PI * 82.0 * t).sin() * (-t / 0.4).exp()) + click) as f32;
            }
        }
        let hi = |p: &[f32]| {
            let mut input = vec![x.clone(), x.clone()];
            run(p, &mut input);
            let mut b = vintage::dsp::Biquad::identity();
            b.set_highpass(48000.0, 3000.0, 0.7);
            let y: Vec<f32> = input[0][12000..12000 + 400].iter().map(|v| b.process(*v as f64) as f32).collect();
            db(rms(&y))
        };
        let a = hi(&with(defaults(), &[(SLAP, 0.0), (COMP, 0.0)]));
        let b = hi(&with(defaults(), &[(SLAP, 1.0), (COMP, 0.0)]));
        println!("attack highs: slap 0 {a:.1} dB, slap 1 {b:.1} dB");
        assert!(b > a + 4.0);
    }

}
