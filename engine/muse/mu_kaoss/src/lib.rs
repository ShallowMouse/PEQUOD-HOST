//! Kaoss Pad: the Korg Kaoss Pad (KP2 / KP3 class) that Matt Bellamy has built into his Manson guitars: an X-Y touch
//! pad playing one effect program at a time, sounding only while the pad is touched. Filters, pitch, delay, flanger,
//! decimator, slicer and ring modulator, with X and Y as the two hands-on controls (automate them or map them to MIDI).

#![allow(clippy::excessive_precision)]
use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
#[allow(unused_imports)]
use vintage::dsp::*;
use vintage::nih_plug_egui::EguiState;
#[allow(unused_imports)]
use vintage::params::*;
#[allow(unused_imports)]
use vintage::ui::{self, ActiveWhen, Item, MeterKind, Panel, Preset};
#[allow(unused_imports)]
use vintage::{Midi, VintageDsp, VintageParams};

pub const W: u32 = 640;
pub const H: u32 = 420;

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum Program {
    #[name = "Low-Pass Filter"]
    Lpf,
    #[name = "High-Pass Filter"]
    Hpf,
    #[name = "Pitch Shift"]
    Pitch,
    #[name = "Delay"]
    Delay,
    #[name = "Flanger"]
    Flanger,
    #[name = "Decimator"]
    Decimator,
    #[name = "Slicer"]
    Slicer,
    #[name = "Ring Mod"]
    Ring,
}

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum Touch {
    #[name = "Off"]
    Off,
    #[name = "On"]
    On,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "program"]
    pub program: EnumParam<Program>,
    #[id = "x"]
    pub x: FloatParam,
    #[id = "y"]
    pub y: FloatParam,
    #[id = "touch"]
    pub touch: EnumParam<Touch>,
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
            program: EnumParam::new("Program", Program::Lpf),
            x: pct("Pad X", 0.5),
            y: pct("Pad Y", 0.5),
            touch: EnumParam::new("Touch", Touch::On),
            depth: pct("FX Depth", 1.0),
            output: db("Output", 0.0, -12.0, 6.0),
        }
    }
}

pub const PROGRAM: usize = 0;
pub const X: usize = 1;
pub const Y: usize = 2;
pub const TOUCH: usize = 3;
pub const DEPTH: usize = 4;
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
        vec![self.program.as_ptr(), self.x.as_ptr(), self.y.as_ptr(), self.touch.as_ptr(), self.depth.as_ptr(), self.output.as_ptr()]
    }
}

/// Default plain values in linked order (for tests).
#[allow(dead_code)]
pub fn defaults() -> Vec<f32> {
    let p = P::default();
    p.linked().iter().map(|q| unsafe { q.preview_plain(q.default_normalized_value()) }).collect()
}

// -------------------------------------------------------------------- dsp --

use vintage::fx::PitchShift;

/// Topology-preserving state-variable filter (safe to sweep every sample).
#[derive(Clone, Copy, Default)]
struct Svf {
    s1: f64,
    s2: f64,
}
impl Svf {
    /// Returns (low, band, high).
    #[inline]
    fn process(&mut self, x: f64, g: f64, k: f64) -> (f64, f64, f64) {
        let hp = (x - (2.0 * k + g) * self.s1 - self.s2) / (1.0 + 2.0 * k * g + g * g);
        let bp = g * hp + self.s1;
        self.s1 = g * hp + bp;
        let lp = g * bp + self.s2;
        self.s2 = g * bp + lp;
        (lp, bp, hp)
    }
}

struct Chan {
    svf: Svf,
    ps: PitchShift,
    dl: DelayLine,
    fl: DelayLine,
    fb: f64,
    hold: f64,
    held: f64,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ch: [Chan; 2],
    x: Smooth,
    y: Smooth,
    wet: Smooth,
    out: Smooth,
    ph: f64,
    prog: usize,
}

impl Dsp {
    fn chan(fs: f64) -> Chan {
        Chan { svf: Svf::default(), ps: PitchShift::new(fs, 50.0), dl: DelayLine::new((fs * 0.8) as usize), fl: DelayLine::new((fs * 0.012) as usize), fb: 0.0, hold: 0.0, held: 0.0 }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 2;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, ch: [Self::chan(fs), Self::chan(fs)], x: Smooth::new(fs, 15.0, 0.5), y: Smooth::new(fs, 15.0, 0.5), wet: Smooth::new(fs, 8.0, 1.0), out: Smooth::new(fs, 20.0, 1.0), ph: 0.0, prog: 0 }
    }
    fn reset(&mut self) {
        let fs = self.fs;
        self.ch = [Self::chan(fs), Self::chan(fs)];
        self.ph = 0.0;
        self.x.snap(self.v[X] as f64);
        self.y.snap(self.v[Y] as f64);
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let p = (v[PROGRAM].round() as usize).min(7);
        if p != self.prog {
            self.prog = p;
            for c in self.ch.iter_mut() {
                c.svf = Svf::default();
                c.fb = 0.0;
            }
        }
        self.x.set(v[X] as f64);
        self.y.set(v[Y] as f64);
        self.wet.set(if v[TOUCH] > 0.5 { v[DEPTH] as f64 } else { 0.0 });
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(2);
        let fs = self.fs;
        let touched = self.v[TOUCH] > 0.5;
        for k in 0..n {
            let (x, y, wet, o) = (self.x.next(), self.y.next(), self.wet.next(), self.out.next());
            for c in 0..nch {
                let dry = chans[c][k] as f64;
                let st = &mut self.ch[c];
                // the delay keeps ringing after you let go; everything else stops with your finger
                let inp = if touched { dry } else { 0.0 };
                let w = match self.prog {
                    0 | 1 => {
                        let f = 60.0 * (16000.0f64 / 60.0).powf(x);
                        let g = (std::f64::consts::PI * f.min(fs * 0.45) / fs).tan();
                        let k = 1.0 / (0.6 + 14.0 * y * y);
                        let (lp, _, hp) = st.svf.process(dry, g, k);
                        let s = if self.prog == 0 { lp } else { hp };
                        (s * 0.8).tanh() / 0.8
                    }
                    2 => {
                        let semis = (x * 24.0 - 12.0).round() * 0.15 + (x * 24.0 - 12.0) * 0.85;
                        let s = st.ps.process(dry, 2f64.powf(semis / 12.0));
                        y * s + (1.0 - y) * dry
                    }
                    3 => {
                        let d = (0.03 + 0.72 * x * x) * fs;
                        let r = st.dl.read(d);
                        st.dl.write(inp + (0.95 * y) * r);
                        dry + r
                    }
                    4 => {
                        let lfo = 0.5 + 0.5 * (2.0 * std::f64::consts::PI * self.ph).sin();
                        let d = (0.0003 + 0.008 * x * (0.6 + 0.4 * lfo)) * fs;
                        let r = st.fl.read(d.max(1.0));
                        st.fl.write(dry + (1.8 * y - 0.9) * r);
                        0.6 * (dry + r)
                    }
                    5 => {
                        let rate = 600.0 * (44100.0f64 / 600.0).powf(x);
                        st.hold += rate / fs;
                        if st.hold >= 1.0 {
                            st.hold -= st.hold.floor();
                            let bits = 16.0 - 14.0 * y;
                            let q = 2f64.powf(bits - 1.0);
                            st.held = (dry * q).round() / q;
                        }
                        st.held
                    }
                    6 => {
                        let rate = 1.0 + 15.0 * x;
                        let ph = (self.ph * rate / 0.5).fract();
                        let gate = if ph < 0.5 { 1.0 } else { 1.0 - y };
                        st.fb += (gate - st.fb) * 0.02;
                        dry * st.fb
                    }
                    _ => {
                        let f = 30.0 * 100f64.powf(x);
                        let m = (2.0 * std::f64::consts::PI * (self.ph * f / 0.5).fract()).sin();
                        y * dry * m + (1.0 - y) * dry
                    }
                };
                let y_out = if self.prog == 3 { dry + (w - dry) * if touched { wet } else { 1.0 } } else { dry + (w - dry) * wet };
                chans[c][k] = (y_out * o) as f32;
            }
            // shared clock for the modulated programs (0.5 Hz base, scaled per program)
            self.ph = (self.ph + 0.5 / fs).fract();
        }
        meters[0] = self.v[X];
        meters[1] = self.v[Y];
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Filter Sweep Solo", values: &[(X, 0.35), (Y, 0.6)] },
        Preset { name: "Supermassive Wobble (filter)", values: &[(X, 0.25), (Y, 0.8)] },
        Preset { name: "Pitch Bend Dive", values: &[(PROGRAM, 2.0), (X, 0.0), (Y, 1.0)] },
        Preset { name: "Octave Up Lead", values: &[(PROGRAM, 2.0), (X, 1.0), (Y, 0.6)] },
        Preset { name: "Runaway Delay", values: &[(PROGRAM, 3.0), (X, 0.4), (Y, 0.85)] },
        Preset { name: "Jet Flange", values: &[(PROGRAM, 4.0), (X, 0.6), (Y, 0.9)] },
        Preset { name: "Bitcrush Glitch", values: &[(PROGRAM, 5.0), (X, 0.2), (Y, 0.7)] },
        Preset { name: "Stutter Slicer", values: &[(PROGRAM, 6.0), (X, 0.45), (Y, 1.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "KAOSS PAD",
        subtitle: ("AFTER THE KORG KAOSS PAD IN BELLAMY'S MANSONS", "X-Y PAD · ONE PROGRAM AT A TIME"),
        accent: ui::INDIGO,
        size: (W, H),
        items: vec![
            Item::Selector { p: PROGRAM, x: 110.0, y: 160.0, label: "PROGRAM", positions: &["LPF", "HPF", "PITCH", "DELAY", "FLANGE", "DECIM", "SLICE", "RING"], tip: "The effect program the pad plays." },
            Item::Knob { p: X, x: 290.0, y: 160.0, r: 48.0, label: "PAD X", tip: "Pad X (left to right): cutoff, pitch, delay time, flanger delay, sample rate, slice rate or ring frequency.", active: None },
            Item::Knob { p: Y, x: 450.0, y: 160.0, r: 48.0, label: "PAD Y", tip: "Pad Y (bottom to top): resonance, mix, feedback, bit depth or slice depth.", active: None },
            Item::Switch { p: TOUCH, x: 570.0, y: 160.0, labels: ["LIFTED", "TOUCHED"], tip: "Finger on the pad: the effect only sounds while touched (a delay's echoes ring on after you let go)." },
            Item::Divider { y: 284.0 },
            Item::Knob { p: DEPTH, x: 220.0, y: 345.0, r: 22.0, label: "FX DEPTH", tip: "How much of the effect replaces the dry guitar while touched.", active: None },
            Item::Knob { p: OUTPUT, x: 420.0, y: 345.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Kaoss Pad", class_id: *b"MuseKaossPadPQD1", vendor: "PEQUOD Muse",
    categories: [Fx, Filter]
);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_program_runs() {
        for prog in 0..8 {
            let mut d = Dsp::new(48000.0, 2);
            let mut v = defaults();
            v[PROGRAM] = prog as f32;
            v[Y] = 0.9;
            d.set_params(&v);
            let mut l: Vec<f32> = (0..48000).map(|k| (0.3 * (k as f64 * 0.05).sin()) as f32).collect();
            let mut r = l.clone();
            let mut m = [0.0f32; 2];
            d.process(&mut [&mut l[..], &mut r[..]], &mut m);
            assert!(l.iter().all(|x| x.is_finite() && x.abs() < 8.0), "program {prog}");
        }
    }
}
