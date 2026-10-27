//! Slicer: a pattern gate after the Boss SL-20 Slicer that Russell Lissack uses for his chopped, stuttering parts (Banquet-era and the warbling tremolo of Virtue). It cuts the sound to one of twelve 16-step rhythm patterns, locked to the host's tempo and bar.

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
pub const H: u32 = 440;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Steps { #[name = "1/8"] E, #[name = "1/16"] S, #[name = "1/16 triplet"] St, #[name = "1/32"] T }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "pattern"]
    pub pattern: IntParam,
    #[id = "step"]
    pub step: EnumParam<Steps>,
    #[id = "depth"]
    pub depth: FloatParam,
    #[id = "edge"]
    pub edge: FloatParam,
    #[id = "bpm"]
    pub bpm: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            pattern: IntParam::new("Pattern", 1, IntRange::Linear { min: 1, max: 12 }),
            step: EnumParam::new("Step", Steps::S),
            depth: pct("Depth", 1.0),
            edge: pct("Edge", 0.25),
            bpm: FloatParam::new("Free BPM", 120.0, FloatRange::Linear { min: 60.0, max: 200.0 }).with_step_size(0.1),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const PATTERN: usize = 0;
pub const STEP: usize = 1;
pub const DEPTH: usize = 2;
pub const EDGE: usize = 3;
pub const BPM: usize = 4;
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
            self.pattern.as_ptr(),
            self.step.as_ptr(),
            self.depth.as_ptr(),
            self.edge.as_ptr(),
            self.bpm.as_ptr(),
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

/// 16-step patterns, bit 15 = first step.
pub const PATTERNS: [u16; 12] = [
    0b1111_1111_1111_1111 ^ 0b0101_0101_0101_0101, // 1: straight chop
    0b1011_0110_1011_0110,                           // 2
    0b1101_1010_1101_1011,                           // 3
    0b1110_1110_1010_1010,                           // 4
    0b1001_0010_0100_1001,                           // 5: sparse stabs
    0b1111_0000_1111_0000,                           // 6: on/off quarters
    0b1101_1101_1101_1101,                           // 7
    0b1010_1010_1010_1010,                           // 8: every other step
    0b1100_1100_1100_1100,                           // 9
    0b1110_1011_0111_0101,                           // 10: broken
    0b1011_1011_1011_1010,                           // 11
    0b1000_1000_1010_1011,                           // 12: build
];

pub fn step_beats(s: usize) -> f64 {
    [0.5, 0.25, 1.0 / 6.0, 0.125][s.min(3)]
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    tempo: Option<f64>,
    beats: Option<f64>,
    playing: bool,
    pos: f64,
    g: f64,
    out: Smooth,
}

impl Dsp {
    pub fn gate(pattern: usize, step: usize) -> bool {
        PATTERNS[(pattern.clamp(1, 12)) - 1] & (1 << (15 - (step % 16))) != 0
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, tempo: None, beats: None, playing: false, pos: 0.0, g: 1.0, out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.pos = 0.0;
        self.g = 1.0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn transport(&mut self, tempo: Option<f64>, beats: Option<f64>, playing: bool) {
        self.tempo = tempo;
        self.beats = beats;
        self.playing = playing;
        // lock to the host's bar position while it plays
        if let (Some(b), true) = (beats, playing) {
            self.pos = b;
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let bpm = self.tempo.unwrap_or(self.v[BPM] as f64).clamp(30.0, 300.0);
        let sb = step_beats(self.v[STEP].round() as usize);
        let depth = self.v[DEPTH] as f64;
        let edge = self.v[EDGE] as f64;
        let pat = self.v[PATTERN].round() as usize;
        // EDGE: how soft each slice starts and stops (0.3 ms to ~15 ms)
        let c = coeff(self.fs, 0.3 + 15.0 * edge * edge);
        let per = bpm / 60.0 / self.fs;
        let mut on_now = false;
        for k in 0..n {
            let step = (self.pos / sb).floor() as i64;
            let on = Dsp::gate(pat, step.rem_euclid(16) as usize);
            on_now = on;
            let t = if on { 1.0 } else { 1.0 - depth };
            self.g = t + c * (self.g - t);
            self.pos += per;
            let o = self.out.next() * self.g;
            for ch in chans.iter_mut() {
                ch[k] = (ch[k] as f64 * o) as f32;
            }
        }
        meters[0] = if on_now { 1.0 } else { 0.0 };
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Straight 16th Chop", values: &[(PATTERN, 8.0)] },
        Preset { name: "Lissack Stutter", values: &[] },
        Preset { name: "Sparse Stabs", values: &[(PATTERN, 5.0), (EDGE, 0.1)] },
        Preset { name: "Virtue Warble (soft)", values: &[(PATTERN, 2.0), (STEP, 3.0), (EDGE, 0.7), (DEPTH, 0.7)] },
        Preset { name: "Broken Rhythm", values: &[(PATTERN, 10.0), (STEP, 1.0)] },
        Preset { name: "Triplet Gate", values: &[(PATTERN, 7.0), (STEP, 2.0)] },
        Preset { name: "Build-Up", values: &[(PATTERN, 12.0), (STEP, 1.0)] },
        Preset { name: "Half-Depth Pulse", values: &[(PATTERN, 9.0), (DEPTH, 0.5), (EDGE, 0.5)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "SLICER",
        subtitle: ("AFTER THE BOSS SL-20 SLICER", "12 PATTERNS · LOCKED TO THE HOST"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![

            Item::Knob { p: PATTERN, x: 110.0, y: 180.0, r: 44.0, label: "PATTERN", tip: "One of twelve 16-step rhythms (the step lamp blinks with it).", active: None },
            Item::Selector { p: STEP, x: 270.0, y: 180.0, label: "STEP", positions: &["1/8", "1/16", "1/16T", "1/32"], tip: "Length of one step, locked to the host's tempo and bar (or FREE BPM when the host isn't playing)." },
            Item::Knob { p: DEPTH, x: 410.0, y: 180.0, r: 34.0, label: "DEPTH", tip: "How far the off steps cut.", active: None },
            Item::Knob { p: EDGE, x: 545.0, y: 180.0, r: 34.0, label: "EDGE", tip: "Hard, clicky chops (left) to soft, warbly pulses (right).", active: None },
            Item::Lamp { m: 0, x: 270.0, y: 255.0, label: "PULSE", lo: 0.0, hi: 1.0 },
            Item::Divider { y: 290.0 },
            Item::Knob { p: BPM, x: 200.0, y: 345.0, r: 24.0, label: "FREE BPM", tip: "Tempo used when the host isn't sending one.", active: None },
            Item::Knob { p: OUTPUT, x: 440.0, y: 345.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Slicer", class_id: *b"BlpSlicerGatePQ1", vendor: "PEQUOD Bloc Party",
    categories: [Fx, Modulation]
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
    fn follows_the_pattern() {
        // pattern 8 (every other 1/16), 120 bpm: steps of 125 ms, on, off, on, off...
        let mut d = Dsp::new(48000.0, 1);
        d.set_params(&with(defaults(), &[(PATTERN, 8.0), (EDGE, 0.0)]));
        d.reset();
        d.transport(Some(120.0), Some(0.0), true);
        let mut x = vec![0.5f32; 48000];
        let mut m = [0.0f32; 1];
        d.process(&mut [&mut x[..]], &mut m);
        let lv: Vec<f32> = (0..8).map(|i| x[i * 6000 + 3000]).collect();
        println!("step levels {:?}", lv);
        assert!(lv[0] > 0.45 && lv[1] < 0.05 && lv[2] > 0.45 && lv[3] < 0.05);
    }

}
