//! Pad Chopper: chops long synth pads into rhythmic 16th-note patterns, as Antonoff does with a tremolo plug-in on pads. Locks to the host's tempo and position, with shape, swing, depth and auto-pan.

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


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Div { #[name = "1/4"] Q, #[name = "1/8"] E, #[name = "1/8T"] ET, #[name = "1/16"] S, #[name = "1/16T"] ST, #[name = "1/32"] T }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Sync { #[name = "Free"] Free, #[name = "Host"] Host }
// steps per quarter note
const STEPS: [f64; 6] = [1.0, 2.0, 3.0, 4.0, 6.0, 8.0];


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "sync"]
    pub sync: EnumParam<Sync>,
    #[id = "bpm"]
    pub bpm: FloatParam,
    #[id = "div"]
    pub div: EnumParam<Div>,
    #[id = "depth"]
    pub depth: FloatParam,
    #[id = "shape"]
    pub shape: FloatParam,
    #[id = "duty"]
    pub duty: FloatParam,
    #[id = "swing"]
    pub swing: FloatParam,
    #[id = "pan"]
    pub pan: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            sync: EnumParam::new("Sync", Sync::Host),
            bpm: FloatParam::new("Tempo", 120.0, FloatRange::Linear { min: 40.0, max: 220.0 }).with_step_size(0.1).with_unit(" BPM"),
            div: EnumParam::new("Division", Div::S),
            depth: pct("Depth", 1.0),
            shape: pct("Shape", 0.25),
            duty: pct("Gate Length", 0.55),
            swing: pct("Swing", 0.0),
            pan: pct("Auto-Pan", 0.0),
            mix: pct("Mix", 1.0),
        }
    }
}

pub const SYNC: usize = 0;
pub const BPM: usize = 1;
pub const DIV: usize = 2;
pub const DEPTH: usize = 3;
pub const SHAPE: usize = 4;
pub const DUTY: usize = 5;
pub const SWING: usize = 6;
pub const PAN: usize = 7;
pub const MIX: usize = 8;
pub const NPARAMS: usize = 9;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.sync.as_ptr(),
            self.bpm.as_ptr(),
            self.div.as_ptr(),
            self.depth.as_ptr(),
            self.shape.as_ptr(),
            self.duty.as_ptr(),
            self.swing.as_ptr(),
            self.pan.as_ptr(),
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
    host_tempo: Option<f64>,
    host_beats: Option<f64>,
    playing: bool,
    beat: f64,
    g: f64,
    mix: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, host_tempo: None, host_beats: None, playing: false, beat: 0.0, g: 1.0, mix: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.beat = 0.0;
        self.g = 1.0;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.mix.set(v[MIX] as f64);
    }
    fn transport(&mut self, tempo: Option<f64>, beats: Option<f64>, playing: bool) {
        self.host_tempo = tempo;
        self.host_beats = beats;
        self.playing = playing;
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let host = self.v[SYNC] > 0.5;
        let tempo = if host { self.host_tempo.unwrap_or(self.v[BPM] as f64) } else { self.v[BPM] as f64 };
        // lock the phase to the host's song position at the start of each block
        if host && self.playing {
            if let Some(b) = self.host_beats {
                self.beat = b;
            }
        }
        let steps = STEPS[(self.v[DIV].round() as usize).min(5)];
        let (depth, shape, duty, swing, pan) = (self.v[DEPTH] as f64, self.v[SHAPE] as f64, self.v[DUTY] as f64, self.v[SWING] as f64, self.v[PAN] as f64);
        let per_sample = tempo / 60.0 / self.fs;
        let edge = 0.02 + 0.45 * shape;
        let smooth = 1.0 - coeff(self.fs, 1.0);
        let mut low = 1.0f64;
        for k in 0..n {
            let pos = self.beat * steps;
            let step = pos.floor();
            // swing: every second step starts late
            let mut ph = pos - step;
            if (step as i64) % 2 == 1 && swing > 0.0 {
                let s = 0.33 * swing;
                ph = if ph < s { 1.0 } else { (ph - s) / (1.0 - s) };
            }
            // gate with soft edges: shape 0 = hard chop, 1 = smooth tremolo
            let open = if ph >= 1.0 {
                0.0
            } else if ph < edge {
                0.5 - 0.5 * (std::f64::consts::PI * ph / edge).cos()
            } else if ph < duty {
                1.0
            } else if ph < duty + edge {
                0.5 + 0.5 * (std::f64::consts::PI * (ph - duty) / edge).cos()
            } else {
                0.0
            };
            let target = 1.0 - depth * (1.0 - open);
            self.g += (target - self.g) * smooth;
            low = low.min(self.g);
            let mx = self.mix.next();
            // auto-pan: alternate steps lean left and right
            let lean = if (step as i64) % 2 == 0 { -pan } else { pan };
            for (ci, c) in chans.iter_mut().enumerate() {
                let side = if nch < 2 { 1.0 } else if ci == 0 { 1.0 - lean.max(0.0) } else { 1.0 + lean.min(0.0) };
                let x = c[k] as f64;
                c[k] = (x * (1.0 - mx) + x * self.g * side * mx) as f32;
            }
            self.beat += per_sample;
        }
        meters[0] = (1.0 - low) as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "16ths on a Pad", values: &[(DIV, 3.0), (DEPTH, 1.0), (SHAPE, 0.2), (DUTY, 0.55)] },
        Preset { name: "Soft 8ths Pulse", values: &[(DIV, 1.0), (DEPTH, 0.8), (SHAPE, 0.6), (DUTY, 0.5)] },
        Preset { name: "Triplet Shimmer", values: &[(DIV, 4.0), (DEPTH, 0.9), (SHAPE, 0.3), (DUTY, 0.5)] },
        Preset { name: "Hard Chop 32nds", values: &[(DIV, 5.0), (DEPTH, 1.0), (SHAPE, 0.0), (DUTY, 0.4)] },
        Preset { name: "Swung 16ths + Pan", values: &[(DIV, 3.0), (SWING, 0.6), (PAN, 0.7), (SHAPE, 0.25)] },
        Preset { name: "Gentle Tremolo", values: &[(DIV, 3.0), (DEPTH, 0.4), (SHAPE, 1.0), (DUTY, 0.5)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "PAD CHOPPER",
        subtitle: ("TURN LONG PADS INTO 16THS", "TEMPO-SYNCED TREMOLO GATE"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![

            Item::Switch { p: SYNC, x: 90.0, y: 120.0, labels: ["FREE", "HOST"], tip: "HOST locks to your host's tempo and song position. FREE uses the TEMPO knob." },
            Item::Knob { p: BPM, x: 90.0, y: 200.0, r: 26.0, label: "TEMPO", tip: "Tempo when SYNC is FREE (or if the host doesn't send one).", active: Some(ActiveWhen { p: SYNC, min: 0.0, max: 0.4 }) },
            Item::Selector { p: DIV, x: 220.0, y: 165.0, label: "DIVISION", positions: &["1/4", "1/8", "1/8T", "1/16", "1/16T", "1/32"], tip: "How often it chops." },
            Item::Knob { p: DEPTH, x: 350.0, y: 165.0, r: 36.0, label: "DEPTH", tip: "How far down each chop goes.", active: None },
            Item::Knob { p: SHAPE, x: 465.0, y: 165.0, r: 30.0, label: "SHAPE", tip: "Hard chop to smooth tremolo.", active: None },
            Item::Knob { p: DUTY, x: 565.0, y: 165.0, r: 26.0, label: "GATE LENGTH", tip: "How much of each step is open.", active: None },
            Item::Divider { y: 264.0 },
            Item::Knob { p: SWING, x: 140.0, y: 330.0, r: 24.0, label: "SWING", tip: "Delays every second step.", active: None },
            Item::Knob { p: PAN, x: 320.0, y: 330.0, r: 24.0, label: "AUTO-PAN", tip: "Alternate steps lean left and right.", active: None },
            Item::Knob { p: MIX, x: 500.0, y: 330.0, r: 24.0, label: "MIX", tip: "Dry vs chopped.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Pad Chopper", class_id: *b"JAPadChopperPQD1", vendor: "PEQUOD Antonoff",
    categories: [Fx, Modulation]
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
    fn chops_at_sixteenths() {
        let p = with(defaults(), &[(SYNC, 0.0), (BPM, 120.0), (DIV, 3.0), (SHAPE, 0.0), (DUTY, 0.5)]);
        let x: Vec<f32> = vec![0.5; 48000];
        let mut input = vec![x];
        run(&p, &mut input);
        // 120 BPM 16ths = 8 per second: count openings
        let opens = input[0].windows(2).filter(|w| w[0] < 0.25 && w[1] >= 0.25).count();
        println!("openings in 1 s: {opens}");
        assert!((7..=9).contains(&opens));
    }

}
