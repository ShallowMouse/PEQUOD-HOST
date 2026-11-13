//! Pianet: in the manner of the Hohner Pianet (N and T).
//!
//! The Pianet has no hammers. Under each key a sticky leather pad rests on a steel reed; pressing
//! the key lifts the pad, which pulls the reed up with it until the reed tears free and rings. Let
//! the key go and the pad drops back onto the reed and stops it. So it's plucked, not struck:
//! short, woody and bright, with little difference between soft and hard playing, and no way to
//! hold a note past the key (no sustain pedal stops the pads).
//!
//! Two models: the N, with an electrostatic pickup like the Wurlitzer's (a little reedy and
//! lopsided), and the T, with a magnetic pickup under each reed (rounder and cleaner).

#![allow(clippy::excessive_precision)]
use nih_plug::prelude::*;
use std::f64::consts::TAU;
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
pub const H: u32 = 300;

const VOICES: usize = 16;

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "model"]
    pub model: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "sustain"]
    pub sustain: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            model: FloatParam::new("Model", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            tone: pct("Tone", 0.5),
            sustain: pct("Sustain", 0.5),
            volume: db("Volume", -6.0, -30.0, 6.0),
        }
    }
}

pub const MODEL: usize = 0;
pub const TONE: usize = 1;
pub const SUSTAIN: usize = 2;
pub const VOLUME: usize = 3;
pub const NPARAMS: usize = 4;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![self.model.as_ptr(), self.tone.as_ptr(), self.sustain.as_ptr(), self.volume.as_ptr()]
    }
}

/// Default plain values in linked order (for tests).
#[allow(dead_code)]
pub fn defaults() -> Vec<f32> {
    let p = P::default();
    p.linked().iter().map(|q| unsafe { q.preview_plain(q.default_normalized_value()) }).collect()
}

// -------------------------------------------------------------------- dsp --

#[derive(Clone, Copy, Default)]
struct Voice {
    on: bool,
    held: bool,
    note: u8,
    age: usize,
    ph: f64,
    inc: f64,
    ph2: f64,
    inc2: f64,
    amp: f64,
    k: f64,
    over: f64,
    k_over: f64,
    thunk: f64,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    voices: [Voice; VOICES],
    noise: Noise,
    thunk_lp: OnePole,
    tone: OnePole,
    body: Biquad,
    dc: DcBlock,
}

impl Dsp {
    fn pluck(&mut self, note: u8, vel: f64) {
        let fs = self.fs;
        let i = self.voices.iter().position(|v| v.on && v.note == note).or_else(|| self.voices.iter().position(|v| !v.on)).unwrap_or_else(|| self.voices.iter().enumerate().max_by_key(|(_, v)| v.age).map(|(i, _)| i).unwrap_or(0));
        // 61 keys, F1 to F6
        let mut n = note as i32;
        while n < 29 {
            n += 12;
        }
        while n > 89 {
            n -= 12;
        }
        let f = 440.0 * 2f64.powf((n as f64 - 69.0) / 12.0);
        let sus = self.v[SUSTAIN] as f64;
        let t60 = (2.5 * (110.0 / f).powf(0.55)).clamp(0.35, 4.0) * (0.4 + 1.2 * sus);
        let k = |t: f64| 10f64.powf(-3.0 / (t * fs));
        let vc = &mut self.voices[i];
        *vc = Voice::default();
        vc.on = true;
        vc.held = true;
        vc.note = note;
        // the pad lets the reed go from where it pulled it: it starts at full swing (cosine phase)
        vc.ph = 0.25;
        vc.inc = f / fs;
        let f2 = 6.3 * f;
        vc.inc2 = if f2 < 0.45 * fs { f2 / fs } else { 0.0 };
        vc.ph2 = 0.25;
        // the pad lets go at about the same point however hard the key goes down
        vc.amp = 0.55 + 0.45 * vel.sqrt();
        vc.k = k(t60);
        vc.over = vc.amp * 0.18;
        vc.k_over = k(0.03);
        vc.thunk = 0.0;
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut thunk_lp = OnePole::default();
        thunk_lp.set_lowpass(fs, 400.0);
        let mut body = Biquad::identity();
        body.set_peak(fs, 1800.0, 0.8, 3.0);
        Self { fs, v, voices: [Voice::default(); VOICES], noise: Noise::new(8), thunk_lp, tone: OnePole::default(), body, dc: DcBlock::new(fs) }
    }
    fn reset(&mut self) {
        self.voices = [Voice::default(); VOICES];
        self.thunk_lp.reset();
        self.tone.reset();
        self.body.reset();
        self.dc.reset();
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn midi(&mut self, ev: Midi) {
        match ev {
            Midi::On { note, vel } => self.pluck(note, vel as f64),
            Midi::Off { note } => {
                // the pad drops back onto the reed: a soft thunk and the note stops
                self.voices.iter_mut().filter(|v| v.on && v.note == note && v.held).for_each(|v| {
                    v.held = false;
                    v.thunk = 0.05;
                })
            }
            Midi::AllOff => self.voices.iter_mut().for_each(|v| v.on = false),
            _ => {}
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let magnetic = self.v[MODEL] >= 0.5;
        self.tone.set_lowpass(fs, 1800.0 * 2f64.powf(3.0 * self.v[TONE] as f64));
        let damp = 10f64.powf(-3.0 / (0.06 * fs));
        let vol = db_to_gain(self.v[VOLUME] as f64);
        let mut peak = 0.0f64;
        for k in 0..n {
            let mut sum = 0.0;
            let mut thunk = 0.0;
            for vc in self.voices.iter_mut().filter(|v| v.on) {
                let x = vc.amp * (TAU * vc.ph).sin() + vc.over * (TAU * vc.ph2).sin();
                sum += if magnetic {
                    // T: the magnetic pickup, nearly linear - a touch of 2nd harmonic
                    x + 0.08 * x * x
                } else {
                    // N: electrostatic, lopsided like the Wurlitzer's
                    let y = (0.45 * x).clamp(-0.9, 0.9);
                    y / (1.0 - y) / 0.45
                };
                thunk += vc.thunk;
                vc.thunk *= 0.995;
                vc.ph = (vc.ph + vc.inc).fract();
                vc.ph2 = (vc.ph2 + vc.inc2).fract();
                vc.amp *= vc.k * if vc.held { 1.0 } else { damp };
                vc.over *= vc.k_over;
                vc.age += 1;
                if vc.age % 64 == 0 && vc.amp < 1e-5 {
                    vc.on = false;
                }
            }
            let y = self.dc.process(sum * 0.3 + self.thunk_lp.lp(self.noise.next() * thunk));
            let y = self.body.process(self.tone.lp(y)) * vol;
            peak = peak.max(y.abs());
            for c in chans.iter_mut() {
                c[k] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Pianet T", values: &[(MODEL, 1.0), (TONE, 0.5), (SUSTAIN, 0.5)] },
        Preset { name: "Pianet N", values: &[(MODEL, 0.0), (TONE, 0.55), (SUSTAIN, 0.5)] },
        Preset { name: "Short and Woody", values: &[(MODEL, 1.0), (TONE, 0.35), (SUSTAIN, 0.15)] },
        Preset { name: "Bright Reedy N", values: &[(MODEL, 0.0), (TONE, 0.85), (SUSTAIN, 0.6)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "PIANET",
        subtitle: ("IN THE MANNER OF THE HOHNER PIANET N AND T", "PLUCKED REEDS · STICKY PADS · TWO PICKUPS"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::Switch { p: MODEL, x: 110.0, y: 170.0, labels: ["N", "T"], tip: "N: the electrostatic pickup, a little reedy and lopsided. T: the magnetic pickups, rounder and cleaner." },
            Item::Knob { p: TONE, x: 270.0, y: 170.0, r: 32.0, label: "TONE", tip: "Darker or brighter.", active: None },
            Item::Knob { p: SUSTAIN, x: 400.0, y: 170.0, r: 32.0, label: "SUSTAIN", tip: "How long the reeds ring while the key is down. Letting go drops the pad back on and stops it.", active: None },
            Item::Knob { p: VOLUME, x: 540.0, y: 170.0, r: 28.0, label: "VOLUME", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Pianet", class_id: *b"PQKeysPianetHohn", vendor: "PEQUOD Keys",
    instrument, categories: [Instrument, Piano]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        with(defaults(), &[(SUSTAIN, 1.0)])
    }

    include!("../../instrument_tests.rs");
}
