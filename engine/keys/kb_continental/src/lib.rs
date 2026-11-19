//! Continental: a transistor combo organ in the manner of the Vox Continental, the mod and beat-group organ of
//! the mid-60s (with its reversed-colour keys and chrome Z-stand). Four drawbars: 16', 8', 4' and IV, a
//! mixture of the higher footages (2 2/3', 2', 1 3/5' and 1') that gives it the bright, reedy edge. Its tone
//! is rounder than a Farfisa's buzz - nearer a sine with a few odd harmonics - keyed on instantly, with a
//! fixed vibrato and a little grit from the transistor preamp, and a swell pedal.

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
pub const H: u32 = 400;

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "d16"]
    pub d16: FloatParam,
    #[id = "d8"]
    pub d8: FloatParam,
    #[id = "d4"]
    pub d4: FloatParam,
    #[id = "div"]
    pub div: FloatParam,
    #[id = "vib"]
    pub vib: FloatParam,
    #[id = "swell"]
    pub swell: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            d16: FloatParam::new("16'", 4.0, FloatRange::Linear { min: 0.0, max: 8.0 }).with_step_size(1.0),
            d8: FloatParam::new("8'", 8.0, FloatRange::Linear { min: 0.0, max: 8.0 }).with_step_size(1.0),
            d4: FloatParam::new("4'", 6.0, FloatRange::Linear { min: 0.0, max: 8.0 }).with_step_size(1.0),
            div: FloatParam::new("IV", 4.0, FloatRange::Linear { min: 0.0, max: 8.0 }).with_step_size(1.0),
            vib: FloatParam::new("Vibrato", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            swell: pct("Swell", 0.8),
            volume: db("Volume", -6.0, -30.0, 6.0),
        }
    }
}

pub const D16: usize = 0;
pub const D8: usize = 1;
pub const D4: usize = 2;
pub const DIV: usize = 3;
pub const VIB: usize = 4;
pub const SWELL: usize = 5;
pub const VOLUME: usize = 6;
pub const NPARAMS: usize = 7;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.d16.as_ptr(),
            self.d8.as_ptr(),
            self.d4.as_ptr(),
            self.div.as_ptr(),
            self.vib.as_ptr(),
            self.swell.as_ptr(),
            self.volume.as_ptr(),
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

/// The Continental's tone: a rounded wave, a sine with a few odd harmonics (the transistor
/// oscillators' shaped output), band-limited by dropping harmonics above 16 kHz.
#[inline]
fn tone(ph: f64, f: f64) -> f64 {
    let w = std::f64::consts::TAU * ph;
    let mut y = w.sin();
    if f * 3.0 < 16000.0 {
        y += 0.22 * (3.0 * w).sin();
    }
    if f * 5.0 < 16000.0 {
        y += 0.07 * (5.0 * w).sin();
    }
    y
}

/// Footages as multiples of the 8' pitch: 16', 8', 4', then the IV mixture.
const MIX: [(f64, f64); 4] = [(3.0, 0.8), (4.0, 0.7), (5.0, 0.55), (8.0, 0.45)];

#[derive(Clone, Copy, Default)]
struct Key {
    on: bool,
    f: f64,
    gain: f64,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    keys: [Key; 128],
    held: [bool; 128],
    vib_ph: f64,
    t: f64,
    voicing: [Biquad; 2],
    dc: DcBlock,
    swell: Smooth,
    pedal: f64,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut voicing = [Biquad::identity(); 2];
        // the preamp and output: a reedy lift and the top taken off
        voicing[0].set_peak(fs, 1600.0, 0.9, 2.5);
        voicing[1].set_lowpass(fs, 9000.0, 0.7);
        Self { fs, v, keys: [Key::default(); 128], held: [false; 128], vib_ph: 0.0, t: 0.0, voicing, dc: DcBlock::new(fs), swell: Smooth::new(fs, 40.0, 0.8), pedal: 1.0 }
    }
    fn reset(&mut self) {
        self.keys = [Key::default(); 128];
        self.held = [false; 128];
        self.voicing.iter_mut().for_each(|b| b.reset());
        self.dc.reset();
        self.swell.snap(self.v[SWELL] as f64 * self.pedal);
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.swell.set(v[SWELL] as f64 * self.pedal);
    }
    fn midi(&mut self, ev: Midi) {
        match ev {
            Midi::On { note, .. } => {
                // 49 keys, C2 to C6 (the 16' drawbar sounds an octave below); outside them, folded in
                let mut n = note as i32;
                while n < 36 {
                    n += 12;
                }
                while n > 84 {
                    n -= 12;
                }
                let k = &mut self.keys[note as usize & 127];
                k.on = true;
                k.f = 440.0 * 2f64.powf((n as f64 - 69.0) / 12.0);
                self.held[note as usize & 127] = true;
            }
            Midi::Off { note } => self.held[note as usize & 127] = false,
            Midi::AllOff => self.held = [false; 128],
            Midi::Cc { cc: 11, value } | Midi::Cc { cc: 7, value } => {
                // the swell pedal (on top of the knob)
                self.pedal = value as f64;
                self.swell.set(self.v[SWELL] as f64 * self.pedal);
            }
            _ => {}
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let p = self.v;
        let bar = |i: usize| p[i] as f64 / 8.0;
        let (b16, b8, b4, biv) = (bar(D16), bar(D8), bar(D4), bar(DIV));
        let vib = p[VIB] >= 0.5;
        let vol = db_to_gain(p[VOLUME] as f64);
        let key_c = coeff(fs, 1.2);
        let mut peak = 0.0f64;
        for k in 0..n {
            // the vibrato: about 6.5 Hz, bending every oscillator together
            self.vib_ph = (self.vib_ph + 6.5 / fs).fract();
            let bend = if vib { 2f64.powf(18.0 / 1200.0 * (std::f64::consts::TAU * self.vib_ph).sin()) } else { 1.0 };
            self.t += bend / fs;
            let t = self.t;
            let mut x = 0.0;
            for (i, key) in self.keys.iter_mut().enumerate() {
                if !key.on {
                    continue;
                }
                let target = if self.held[i] { 1.0 } else { 0.0 };
                key.gain += key_c * (target - key.gain);
                if !self.held[i] && key.gain < 1e-4 {
                    key.on = false;
                    continue;
                }
                let f = key.f;
                let ph = |m: f64| (m * f * t).fract();
                let mut s = 0.0;
                if b16 > 0.0 {
                    s += b16 * tone(ph(0.5), 0.5 * f);
                }
                if b8 > 0.0 {
                    s += b8 * tone(ph(1.0), f);
                }
                if b4 > 0.0 {
                    s += b4 * tone(ph(2.0), 2.0 * f);
                }
                if biv > 0.0 {
                    for (m, g) in MIX {
                        // the mixture's top ranks fold back an octave high on the keyboard, as divider organs do
                        let mm = if m * f > 6000.0 { m * 0.5 } else { m };
                        s += biv * g * (std::f64::consts::TAU * ph(mm)).sin();
                    }
                }
                x += key.gain * s;
            }
            let sw = self.swell.next();
            // the transistor preamp: a little grit when it's full
            let y = cascade(&mut self.voicing, x * 0.12);
            let y = self.dc.process((y * 1.3).tanh() / 1.3) * vol * (0.15 + 0.85 * sw);
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
        Preset { name: "Beat Group", values: &[] },
        Preset { name: "Bright Mod Stabs", values: &[(D16, 0.0), (D8, 8.0), (D4, 8.0), (DIV, 8.0), (VIB, 0.0)] },
        Preset { name: "Full Continental", values: &[(D16, 8.0), (D8, 8.0), (D4, 8.0), (DIV, 8.0)] },
        Preset { name: "Soft 8' Alone", values: &[(D16, 0.0), (D8, 8.0), (D4, 0.0), (DIV, 0.0)] },
        Preset { name: "Dark 16' and 8'", values: &[(D16, 8.0), (D8, 6.0), (D4, 0.0), (DIV, 0.0)] },
        Preset { name: "Reedy Lead", values: &[(D16, 0.0), (D8, 6.0), (D4, 4.0), (DIV, 7.0)] },
        Preset { name: "No Vibrato, Garage", values: &[(D16, 3.0), (D8, 8.0), (D4, 5.0), (DIV, 5.0), (VIB, 0.0)] },
    ]
}

fn panel() -> Panel {
    let bar = |p, x, label, tip| Item::Knob { p, x, y: 170.0, r: 26.0, label, tip, active: None };
    Panel {
        title: "CONTINENTAL",
        subtitle: ("IN THE MANNER OF THE VOX CONTINENTAL", "FOUR DRAWBARS · IV MIXTURE · VIBRATO"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::Text { x: 320.0, y: 96.0, text: "DRAWBARS", size: 11.0, align: Align2::CENTER_CENTER },
            bar(D16, 110.0, "16'", "16': an octave below."),
            bar(D8, 250.0, "8'", "8': the fundamental."),
            bar(D4, 390.0, "4'", "4': an octave up."),
            bar(DIV, 530.0, "IV", "IV: the mixture of 2 2/3', 2', 1 3/5' and 1' - the bright, reedy edge."),
            Item::Divider { y: 240.0 },
            Item::Switch { p: VIB, x: 140.0, y: 310.0, labels: ["", "VIBRATO"], tip: "The organ's fixed vibrato." },
            Item::Knob { p: SWELL, x: 320.0, y: 310.0, r: 26.0, label: "SWELL", tip: "The swell pedal (also MIDI CC 11 or 7).", active: None },
            Item::Knob { p: VOLUME, x: 500.0, y: 310.0, r: 26.0, label: "VOLUME", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Continental", class_id: *b"PQKeysContinentl", vendor: "PEQUOD Keys",
    instrument, categories: [Instrument, Synth]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        with(defaults(), &[(D16, 0.0), (D4, 0.0), (DIV, 0.0), (VIB, 0.0)])
    }

    include!("../../instrument_tests.rs");
}
