//! H3000: Eventide's Ultra-Harmonizer (1986), on every big vocal and guitar rack after it. The programs
//! people used it for: MICROPITCH (each side detuned a few cents and delayed, the classic thickener),
//! DUAL SHIFT (two pitch shifters, left and right, by semitones and cents, with delay and feedback) and
//! REVERSE SHIFT (backwards grains). The 16-bit converters and the H3000's own smearing grain. Estimated
//! from Eventide's documentation.

#![allow(clippy::excessive_precision)]
use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
#[allow(unused_imports)]
use vintage::dsp::*;
#[allow(unused_imports)]
use vintage::fx::*;
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
    #[id = "mode"]
    pub mode: FloatParam,
    #[id = "left"]
    pub left: FloatParam,
    #[id = "right"]
    pub right: FloatParam,
    #[id = "delay"]
    pub delay: FloatParam,
    #[id = "feedback"]
    pub feedback: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            mode: FloatParam::new("Program", 0.0, FloatRange::Linear { min: 0.0, max: 2.0 }).with_step_size(1.0),
            left: FloatParam::new("Left Shift", -9.0, FloatRange::Linear { min: -1200.0, max: 1200.0 }).with_step_size(1.0).with_unit(" ct"),
            right: FloatParam::new("Right Shift", 9.0, FloatRange::Linear { min: -1200.0, max: 1200.0 }).with_step_size(1.0).with_unit(" ct"),
            delay: ms("Delay", 15.0, 0.0, 500.0),
            feedback: pct("Feedback", 0.0),
            mix: pct("Mix", 0.35),
        }
    }
}

pub const MODE: usize = 0;
pub const LEFT: usize = 1;
pub const RIGHT: usize = 2;
pub const DELAY: usize = 3;
pub const FEEDBACK: usize = 4;
pub const MIX: usize = 5;
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
            self.mode.as_ptr(),
            self.left.as_ptr(),
            self.right.as_ptr(),
            self.delay.as_ptr(),
            self.feedback.as_ptr(),
            self.mix.as_ptr(),
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
    shift: [PitchShift; 2],
    line: [DelayLine; 2],
    fb: [f64; 2],
    rev: [DelayLine; 2],
    rev_phase: f64,
    mix: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let len = (fs * 0.6) as usize + 8;
        Self { fs, v, shift: [PitchShift::new(fs, 40.0), PitchShift::new(fs, 43.0)], line: [DelayLine::new(len), DelayLine::new(len)],
               fb: [0.0; 2], rev: [DelayLine::new((fs * 0.3) as usize), DelayLine::new((fs * 0.3) as usize)], rev_phase: 0.0,
               mix: Smooth::new(fs, 20.0, 0.35) }
    }
    fn reset(&mut self) {
        self.shift.iter_mut().for_each(|s| s.reset());
        self.line.iter_mut().chain(self.rev.iter_mut()).for_each(|l| l.reset());
        self.fb = [0.0; 2];
        self.mix.snap(self.v[MIX] as f64);
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.mix.set(v[MIX] as f64);
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nc = chans.len();
        let mode = self.v[MODE].round() as usize;
        // MICROPITCH: fixed small detunes and short delays; DUAL SHIFT: as set
        let cents = if mode == 0 { [-(self.v[LEFT].abs().min(30.0)) as f64, self.v[RIGHT].abs().min(30.0) as f64] } else { [self.v[LEFT] as f64, self.v[RIGHT] as f64] };
        let ratio = [2f64.powf(cents[0] / 1200.0), 2f64.powf(cents[1] / 1200.0)];
        let d = [self.v[DELAY] as f64, self.v[DELAY] as f64 * if mode == 0 { 1.4 } else { 1.0 }];
        let fb = self.v[FEEDBACK] as f64 * 0.9;
        let grain = 0.12 * self.fs;
        for k in 0..n {
            let m = self.mix.next();
            let x = if nc >= 2 { (chans[0][k] + chans[1][k]) as f64 * 0.5 } else { chans[0][k] as f64 };
            let mut wet = [0.0; 2];
            for s in 0..2 {
                let into = quantise((x + self.fb[s] * fb) * 0.5, 16) * 2.0;
                let y = if mode == 2 {
                    // REVERSE SHIFT: play back the last grain backwards, crossfaded
                    self.rev[s].write(into);
                    let ph = (self.rev_phase + s as f64 * 0.5).fract();
                    let g = (std::f64::consts::PI * ph).sin();
                    self.rev[s].read(2.0 + (1.0 - ph) * grain) * g
                } else {
                    self.shift[s].process(into, ratio[s])
                };
                self.line[s].write(y);
                let out = self.line[s].read((d[s] * 0.001 * self.fs).max(1.0));
                self.fb[s] = out;
                wet[s] = out;
            }
            self.rev_phase = (self.rev_phase + 1.0 / grain).fract();
            for c in 0..nc {
                let dry = chans[c][k] as f64;
                chans[c][k] = (dry * (1.0 - m) + wet[c.min(1)] * m) as f32;
            }
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Micropitch Vocal (+/-9 ct)", values: &[(MODE, 0.0), (LEFT, -9.0), (RIGHT, 9.0), (DELAY, 12.0), (MIX, 0.3)] },
        Preset { name: "Micropitch Wide Guitar", values: &[(MODE, 0.0), (LEFT, -15.0), (RIGHT, 15.0), (DELAY, 25.0), (MIX, 0.45)] },
        Preset { name: "Octave Down", values: &[(MODE, 1.0), (LEFT, -1200.0), (RIGHT, -1200.0), (DELAY, 0.0), (MIX, 0.4)] },
        Preset { name: "Fifths (L) and Octave (R)", values: &[(MODE, 1.0), (LEFT, 700.0), (RIGHT, 1200.0), (DELAY, 30.0), (MIX, 0.35)] },
        Preset { name: "Rising Echoes", values: &[(MODE, 1.0), (LEFT, 100.0), (RIGHT, 100.0), (DELAY, 280.0), (FEEDBACK, 0.6), (MIX, 0.35)] },
        Preset { name: "Reverse Shift", values: &[(MODE, 2.0), (DELAY, 0.0), (MIX, 0.5)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "H3000",
        subtitle: ("EVENTIDE ULTRA-HARMONIZER, 1986", "MICROPITCH, DUAL SHIFT, REVERSE SHIFT"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![
            Item::Selector { p: MODE, x: 110.0, y: 160.0, label: "PROGRAM", positions: &["MICRO", "DUAL", "REVERSE"], tip: "Micropitch (a few cents each side), Dual Shift (as set), Reverse Shift." },
            Item::Knob { p: LEFT, x: 270.0, y: 160.0, r: 36.0, label: "LEFT", tip: "Pitch shift, cents (Micropitch: up to 30).", active: None },
            Item::Knob { p: RIGHT, x: 400.0, y: 160.0, r: 36.0, label: "RIGHT", tip: "Pitch shift, cents.", active: None },
            Item::Knob { p: MIX, x: 540.0, y: 160.0, r: 34.0, label: "MIX", tip: "Dry vs. effect.", active: None },
            Item::Divider { y: 255.0 },
            Item::Knob { p: DELAY, x: 250.0, y: 335.0, r: 30.0, label: "DELAY", tip: "Delay after the shift.", active: None },
            Item::Knob { p: FEEDBACK, x: 400.0, y: 335.0, r: 30.0, label: "FEEDBACK", tip: "Back into the shifter: rising or falling echoes.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "H3000", class_id: *b"HWEventideH3000Q", vendor: "PEQUOD Hardware",
    categories: [Fx, PitchShift]
);

#[cfg(test)]
#[allow(dead_code)]
pub mod tests {
    use super::*;
    use std::f64::consts::PI;

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
    pub fn sine(f: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n).map(|k| (amp * (2.0 * PI * f * k as f64 / 48000.0).sin()) as f32).collect()
    }
    pub fn noise(n: usize, amp: f64) -> Vec<f32> {
        let mut nz = vintage::dsp::Noise::new(11);
        (0..n).map(|_| (nz.next() * amp) as f32).collect()
    }
    /// Both channels out for a mono input.
    pub fn go2(p: &[f32], x: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        let r = input.pop().unwrap();
        (input.pop().unwrap(), r)
    }
    pub fn go(p: &[f32], x: &[f32]) -> Vec<f32> {
        go2(p, x).0
    }
    /// Steady-state level of a sine at f through the settings, in dB.
    pub fn level(set: &[(usize, f32)], f: f64) -> f64 {
        let p = with(defaults(), set);
        let y = go(&p, &sine(f, 0.1, 48000));
        db(rms(&y[24000..]))
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
            let l: Vec<f32> = (0..n).map(|k| if (k / 6000) % 2 == 0 { (nz.next() * 0.5) as f32 } else { 0.0 }).collect();
            let mut input = vec![l.clone(), l];
            run(&p, &mut input);
            assert!(input.iter().flatten().all(|v| v.is_finite() && v.abs() < 8.0), "unbounded output for {:?}", p);
        }
    }

    #[test]
    fn makes_sound() {
        let p = with(defaults(), &[(MIX, 0.5)]);
        let y = go(&p, &sine(440.0, 0.2, 48000));
        let l = db(rms(&y[24000..]));
        println!("level {:.1} dB", l);
        assert!(l > -40.0 && y.iter().all(|v| v.is_finite()));
    }

}
