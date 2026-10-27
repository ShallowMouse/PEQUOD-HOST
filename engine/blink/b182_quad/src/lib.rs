//! Quad Tracker: blink-182's wall of guitars, where every rhythm part is played two or four times and panned wide. From one take it makes two or four 'takes', each a few milliseconds early or late and drifting a few cents, with slightly different tone, spread left and right. Useful on any single rhythm guitar that needs to be a wall.

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
pub const H: u32 = 420;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Takes { #[name = "2 takes"] Two, #[name = "4 takes"] Four }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "takes"]
    pub takes: EnumParam<Takes>,
    #[id = "width"]
    pub width: FloatParam,
    #[id = "loose"]
    pub loose: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "centre"]
    pub centre: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            takes: EnumParam::new("Takes", Takes::Two),
            width: pct("Width", 1.0),
            loose: pct("Looseness", 0.4),
            tone: pct("Tone Spread", 0.4),
            centre: db("Centre (original)", -60.0, -60.0, 0.0),
            output: db("Output", -3.0, -24.0, 12.0),
        }
    }
}

pub const TAKES: usize = 0;
pub const WIDTH: usize = 1;
pub const LOOSE: usize = 2;
pub const TONE: usize = 3;
pub const CENTRE: usize = 4;
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
            self.takes.as_ptr(),
            self.width.as_ptr(),
            self.loose.as_ptr(),
            self.tone.as_ptr(),
            self.centre.as_ptr(),
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

struct Take {
    dl: DelayLine,
    w: Wander,
    base: f64,
    eq: Biquad,
    pan: (f64, f64),
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    takes: Vec<Take>,
    key: (i32, i32),
    out: Smooth,
    centre: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let tone = self.v[TONE] as f64;
        let width = self.v[WIDTH] as f64;
        let four = self.v[TAKES] > 0.5;
        for (i, t) in self.takes.iter_mut().enumerate() {
            // takes alternate darker / brighter, and left / right (inner pair narrower)
            let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
            t.eq.set_high_shelf(fs, 2500.0, 0.7, sign * 2.5 * tone);
            let p = width * if i % 2 == 0 { -1.0 } else { 1.0 } * if i < 2 { 1.0 } else { 0.6 };
            let p = if !four && i >= 2 { 0.0 } else { p };
            let th = (p + 1.0) * std::f64::consts::FRAC_PI_4;
            t.pan = (th.cos(), th.sin());
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let bases = [9.0, 17.0, 13.0, 23.0];
        let rates = [0.31, 0.23, 0.37, 0.27];
        let takes = (0..4).map(|i| Take { dl: DelayLine::new((0.06 * fs) as usize), w: Wander::new(fs, rates[i], 11 + i as u32 * 7), base: bases[i] * 0.001 * fs, eq: Biquad::identity(), pan: (0.7, 0.7) }).collect();
        let mut d = Self { fs, v, takes, key: (-1, -1), out: Smooth::new(fs, 20.0, 0.7), centre: Smooth::new(fs, 20.0, 0.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        for t in self.takes.iter_mut() {
            t.dl.reset();
            t.eq.reset();
        }
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
        self.centre.snap(db_to_gain(self.v[CENTRE] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = ((v[TONE] * 100.0) as i32 + 1000 * v[TAKES] as i32, (v[WIDTH] * 100.0) as i32);
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
        self.centre.set(if v[CENTRE] <= -59.9 { 0.0 } else { db_to_gain(v[CENTRE] as f64) });
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().max(1);
        let loose = self.v[LOOSE] as f64;
        let count = if self.v[TAKES] > 0.5 { 4 } else { 2 };
        let norm = 1.0 / (count as f64).sqrt() * 1.2;
        // drift: up to ~6 ms of timing wander, which also moves the pitch by a few cents
        let swing = 0.006 * self.fs * loose;
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch as f64;
            let (mut l, mut r) = (0.0, 0.0);
            for t in self.takes.iter_mut().take(count) {
                t.dl.write(x);
                let d = t.base * (0.5 + 0.5 * loose) + swing * t.w.next();
                let y = t.eq.process(t.dl.read(d.max(2.0))) * norm;
                l += y * t.pan.0;
                r += y * t.pan.1;
            }
            let o = self.out.next();
            let c = self.centre.next();
            if nch >= 2 {
                chans[0][k] = ((l + x * c) * o) as f32;
                chans[1][k] = ((r + x * c) * o) as f32;
            } else {
                chans[0][k] = ((0.5 * (l + r) + x * c) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Double-Tracked Rhythm (L/R)", values: &[] },
        Preset { name: "Quad Wall (Enema style)", values: &[(TAKES, 1.0), (LOOSE, 0.45), (TONE, 0.5)] },
        Preset { name: "Tight Doubles", values: &[(LOOSE, 0.15), (TONE, 0.2)] },
        Preset { name: "Sloppy Punk Doubles", values: &[(LOOSE, 0.9), (TONE, 0.6)] },
        Preset { name: "Wall Plus Centre", values: &[(TAKES, 1.0), (CENTRE, -6.0)] },
        Preset { name: "Narrow Doubles", values: &[(WIDTH, 0.5)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "QUAD TRACKER",
        subtitle: ("BLINK-182'S WALL OF GUITARS", "2 OR 4 TAKES FROM ONE · WIDE"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![

            Item::Selector { p: TAKES, x: 90.0, y: 165.0, label: "TAKES", positions: &["2", "4"], tip: "How many takes to make from one: 2 (left and right) or 4 (a wall)." },
            Item::Knob { p: WIDTH, x: 220.0, y: 165.0, r: 36.0, label: "WIDTH", tip: "How far apart the takes are panned.", active: None },
            Item::Knob { p: LOOSE, x: 350.0, y: 165.0, r: 36.0, label: "LOOSENESS", tip: "How much the takes drift in timing (and so in pitch) against each other.", active: None },
            Item::Knob { p: TONE, x: 480.0, y: 165.0, r: 36.0, label: "TONE SPREAD", tip: "Takes alternate a little darker and brighter, as different takes and mics do.", active: None },
            Item::Divider { y: 250.0 },
            Item::Knob { p: CENTRE, x: 200.0, y: 315.0, r: 24.0, label: "CENTRE", tip: "The original take in the middle (fully left = off).", active: None },
            Item::Knob { p: OUTPUT, x: 440.0, y: 315.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Quad Tracker", class_id: *b"B182QuadTrackPQ1", vendor: "PEQUOD blink-182",
    categories: [Fx, Delay]
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
    fn sine_amp(f: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n).map(|k| (amp * (2.0 * PI * f * k as f64 / 48000.0).sin()) as f32).collect()
    }
    fn goertzel(x: &[f32], f: f64) -> f64 {
        let w = 2.0 * PI * f / 48000.0;
        let (mut s1, mut s2) = (0.0, 0.0);
        for v in x {
            let s = *v as f64 + 2.0 * w.cos() * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        ((s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2).max(0.0)).sqrt() / x.len() as f64
    }
    fn go(p: &[f32], x: &[f32]) -> Vec<f32> {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        input.remove(0)
    }
    /// harmonics (2..6) relative to the fundamental, dB
    fn thd(p: &[f32], f: f64, amp: f64) -> f64 {
        let y = go(p, &sine_amp(f, amp, 48000));
        let a = goertzel(&y[24000..], f);
        let h: f64 = (2..7).map(|k| goertzel(&y[24000..], f * k as f64).powi(2)).sum::<f64>().sqrt();
        db(h / a.max(1e-12))
    }

    #[test]
    fn wide_and_different() {
        let p = defaults();
        let x = sine_amp(330.0, 0.3, 48000);
        let mut input = vec![x.clone(), x];
        run(&p, &mut input);
        let (l, r) = (&input[0][24000..], &input[1][24000..]);
        let dot: f64 = l.iter().zip(r).map(|(a, b)| *a as f64 * *b as f64).sum();
        let corr = dot / (rms(l) * rms(r) * l.len() as f64);
        println!("L/R correlation {corr:.2}");
        assert!(corr < 0.8 && rms(l) > 0.05 && rms(r) > 0.05);
    }

}
