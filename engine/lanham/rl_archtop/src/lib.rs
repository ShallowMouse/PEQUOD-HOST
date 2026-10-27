//! Archtop: turns a solid-body or thin guitar into a big hollow archtop like Roy Lanham's Gibson L-5 (with a DeArmond floating pickup) and later Epiphone Emperor: the body's air and top resonances, a little of the wood's own sound, the PICKUP's voice, and FLATS for flatwound strings. Resonances estimated for 16, 17 and 18-inch bodies.

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
pub const H: u32 = 400;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Size { #[name = "16 inch"] S16, #[name = "17 inch (L-5)"] S17, #[name = "18 inch (Emperor)"] S18 }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Pickup { #[name = "DeArmond floating"] DeArmond, #[name = "Charlie Christian bar"] Christian, #[name = "P-90"] P90 }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "size"]
    pub size: EnumParam<Size>,
    #[id = "body"]
    pub body: FloatParam,
    #[id = "air"]
    pub air: FloatParam,
    #[id = "pickup"]
    pub pickup: EnumParam<Pickup>,
    #[id = "flats"]
    pub flats: BoolParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            size: EnumParam::new("Body Size", Size::S17),
            body: pct("Body", 0.6),
            air: pct("Air", 0.3),
            pickup: EnumParam::new("Pickup", Pickup::DeArmond),
            flats: BoolParam::new("Flatwounds", true),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const SIZE: usize = 0;
pub const BODY: usize = 1;
pub const AIR: usize = 2;
pub const PICKUP: usize = 3;
pub const FLATS: usize = 4;
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
            self.size.as_ptr(),
            self.body.as_ptr(),
            self.air.as_ptr(),
            self.pickup.as_ptr(),
            self.flats.as_ptr(),
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
    body: [Biquad; 3],
    pu: [Biquad; 3],
    flats: Biquad,
    d: DelayLine,
    damp: OnePole,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ch: Vec<Chan>,
    key: Vec<i32>,
    out: Smooth,
}

pub const BODY_F: [[f64; 3]; 3] = [[110.0, 220.0, 400.0], [100.0, 200.0, 370.0], [92.0, 185.0, 340.0]];

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let v = self.v;
        let f = BODY_F[(v[SIZE].round() as usize).min(2)];
        let b = v[BODY] as f64;
        let flats = v[FLATS] > 0.5;
        for c in self.ch.iter_mut() {
            c.body[0].set_peak(fs, f[0], 4.0, 8.0 * b);
            c.body[1].set_peak(fs, f[1], 3.0, 5.0 * b);
            c.body[2].set_peak(fs, f[2], 3.0, 3.0 * b);
            match v[PICKUP].round() as usize {
                0 => {
                    c.pu[0].set_low_shelf(fs, 150.0, 0.7, 0.0);
                    c.pu[1].set_peak(fs, 1600.0, 1.0, 4.0);
                    c.pu[2].set_high_shelf(fs, 4000.0, 0.7, -4.0);
                }
                1 => {
                    c.pu[0].set_low_shelf(fs, 150.0, 0.7, 2.0);
                    c.pu[1].set_peak(fs, 1100.0, 1.0, 5.0);
                    c.pu[2].set_high_shelf(fs, 3000.0, 0.7, -7.0);
                }
                _ => {
                    c.pu[0].set_low_shelf(fs, 150.0, 0.7, 1.0);
                    c.pu[1].set_peak(fs, 2200.0, 1.0, 3.0);
                    c.pu[2].set_high_shelf(fs, 5000.0, 0.7, -2.0);
                }
            }
            if flats {
                c.flats.set_lowpass(fs, 3500.0, 0.6);
            } else {
                c.flats = Biquad::identity();
            }
            c.damp.set_lowpass(fs, 2500.0);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mk = || Chan { body: [Biquad::identity(); 3], pu: [Biquad::identity(); 3], flats: Biquad::identity(), d: DelayLine::new((0.01 * fs) as usize), damp: OnePole::default() };
        let mut d = Self { fs, v, ch: (0..ch.max(2)).map(|_| mk()).collect(), key: vec![], out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        for c in self.ch.iter_mut() {
            c.body.iter_mut().chain(c.pu.iter_mut()).for_each(|b| b.reset());
            c.flats.reset();
            c.d.reset();
        }
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k: Vec<i32> = v[..NPARAMS].iter().map(|x| (x * 100.0) as i32).collect();
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let air = self.v[AIR] as f64;
        let (d1, d2) = (0.0011 * self.fs, 0.0023 * self.fs);
        let last = self.ch.len() - 1;
        for k in 0..n {
            let o = self.out.next();
            for (ci, chn) in chans.iter_mut().enumerate() {
                let c = &mut self.ch[ci.min(last)];
                let x = c.flats.process(chn[k] as f64);
                let y = cascade(&mut c.pu, cascade(&mut c.body, x));
                // the wood: a couple of short, darkened reflections inside the body
                c.d.write(y);
                let w = c.damp.lp(c.d.read(d1) - 0.6 * c.d.read(d2));
                chn[k] = ((y + air * 0.5 * w) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "L-5 with DeArmond", values: &[] },
        Preset { name: "Emperor, Big and Warm", values: &[(SIZE, 2.0), (BODY, 0.75), (AIR, 0.4)] },
        Preset { name: "Charlie Christian Pickup", values: &[(PICKUP, 1.0), (BODY, 0.5)] },
        Preset { name: "Bright Roundwounds", values: &[(FLATS, 0.0), (PICKUP, 2.0), (BODY, 0.4)] },
        Preset { name: "Subtle Hollow", values: &[(BODY, 0.3), (AIR, 0.15)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "ARCHTOP",
        subtitle: ("A BIG HOLLOW ARCHTOP FROM ANY GUITAR", "L-5 · EMPEROR · DEARMOND · FLATWOUNDS"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![

            Item::Selector { p: SIZE, x: 90.0, y: 165.0, label: "BODY SIZE", positions: &["16", "17", "18"], tip: "Body width: the bigger the body, the lower its resonances." },
            Item::Knob { p: BODY, x: 220.0, y: 165.0, r: 36.0, label: "BODY", tip: "How much of the hollow body's resonance.", active: None },
            Item::Knob { p: AIR, x: 350.0, y: 165.0, r: 30.0, label: "AIR", tip: "A little of the wood's own sound.", active: None },
            Item::Selector { p: PICKUP, x: 480.0, y: 165.0, label: "PICKUP", positions: &["DEARMOND", "CHRISTIAN", "P-90"], tip: "The pickup's voice." },
            Item::Divider { y: 245.0 },
            Item::Switch { p: FLATS, x: 170.0, y: 305.0, labels: ["ROUND", "FLATS"], tip: "Flatwound strings: softer and darker." },
            Item::Knob { p: OUTPUT, x: 440.0, y: 305.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Archtop", class_id: *b"RLArchtopBodyPQ1", vendor: "PEQUOD Roy Lanham",
    categories: [Fx, Eq]
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
    fn body_resonance() {
        let lvl = |p: &[f32]| db(rms(&stereo(p, &sine(100.0, 0.1, 24000)).0[12000..]));
        let a = lvl(&with(defaults(), &[(BODY, 0.0), (AIR, 0.0)]));
        let b = lvl(&with(defaults(), &[(BODY, 1.0), (AIR, 0.0)]));
        println!("100 Hz: body 0 {a:.1} dB, body 1 {b:.1} dB");
        assert!(b > a + 5.0);
    }

}
