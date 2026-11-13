//! String Ensemble: in the manner of the ARP Solina String Ensemble (SE-IV).
//!
//! Every note on the Solina comes from one top-octave oscillator divided down, so the whole
//! keyboard is locked in tune and in phase - flat and organ-like on its own. What makes it a
//! string section is the ensemble: three bucket-brigade delays swept by a slow and a fast LFO
//! at 0, 120 and 240 degrees. Six section tabs voice the divided-down waves (violin, viola,
//! cello, contrabass in the low notes only, horn and trumpet) and can be combined; CRESCENDO
//! swells each note in, SUSTAIN lets it fade after the key comes up.

#![allow(clippy::excessive_precision)]
use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
#[allow(unused_imports)]
use vintage::dsp::*;
use vintage::fx::StringEnsemble;
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
pub const H: u32 = 380;

const VOICES: usize = 32;

fn tab(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0)
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "violin"]
    pub violin: FloatParam,
    #[id = "viola"]
    pub viola: FloatParam,
    #[id = "cello"]
    pub cello: FloatParam,
    #[id = "bass"]
    pub bass: FloatParam,
    #[id = "horn"]
    pub horn: FloatParam,
    #[id = "trumpet"]
    pub trumpet: FloatParam,
    #[id = "crescendo"]
    pub crescendo: FloatParam,
    #[id = "sustain"]
    pub sustain: FloatParam,
    #[id = "ensemble"]
    pub ensemble: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            violin: tab("Violin", 1.0),
            viola: tab("Viola", 1.0),
            cello: tab("Cello", 0.0),
            bass: tab("Contrabass", 0.0),
            horn: tab("Horn", 0.0),
            trumpet: tab("Trumpet", 0.0),
            crescendo: ms("Crescendo", 120.0, 5.0, 2000.0),
            sustain: ms("Sustain", 900.0, 30.0, 5000.0),
            ensemble: tab("Ensemble", 1.0),
            volume: db("Volume", -6.0, -30.0, 6.0),
        }
    }
}

pub const VIOLIN: usize = 0;
pub const VIOLA: usize = 1;
pub const CELLO: usize = 2;
pub const BASS: usize = 3;
pub const HORN: usize = 4;
pub const TRUMPET: usize = 5;
pub const CRESC: usize = 6;
pub const SUSTAIN: usize = 7;
pub const ENSEMBLE: usize = 8;
pub const VOLUME: usize = 9;
pub const NPARAMS: usize = 10;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.violin.as_ptr(),
            self.viola.as_ptr(),
            self.cello.as_ptr(),
            self.bass.as_ptr(),
            self.horn.as_ptr(),
            self.trumpet.as_ptr(),
            self.crescendo.as_ptr(),
            self.sustain.as_ptr(),
            self.ensemble.as_ptr(),
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

/// The divider's naive saw, band-limited: phase from the shared master clock, so every key's
/// oscillator is locked to every other.
#[inline]
fn saw(ph: f64, dt: f64) -> f64 {
    let b = if ph < dt {
        let x = ph / dt;
        x + x - x * x - 1.0
    } else if ph > 1.0 - dt {
        let x = (ph - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    };
    2.0 * ph - 1.0 - b
}

#[derive(Clone, Copy, Default)]
struct Voice {
    on: bool,
    held: bool,
    note: u8,
    f: f64,
    env: f64,
    age: u64,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    voices: [Voice; VOICES],
    clock: u64,
    order: u64,
    pedal: bool,
    // section voicing filters, per bus
    violin: [Biquad; 2],
    viola: Biquad,
    cello: Biquad,
    bass: Biquad,
    horn: [Biquad; 2],
    trumpet: [Biquad; 2],
    ens: StringEnsemble,
    dc: [DcBlock; 2],
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut violin = [Biquad::identity(); 2];
        violin[0].set_highpass(fs, 280.0, 0.7);
        violin[1].set_peak(fs, 2600.0, 1.0, 4.0);
        let mut viola = Biquad::identity();
        viola.set_lowpass(fs, 3200.0, 0.7);
        let mut cello = Biquad::identity();
        cello.set_lowpass(fs, 2000.0, 0.7);
        let mut bass = Biquad::identity();
        bass.set_lowpass(fs, 700.0, 0.7);
        let mut horn = [Biquad::identity(); 2];
        horn[0].set_lowpass(fs, 1500.0, 0.7);
        horn[1].set_peak(fs, 800.0, 1.2, 5.0);
        let mut trumpet = [Biquad::identity(); 2];
        trumpet[0].set_peak(fs, 1500.0, 1.0, 7.0);
        trumpet[1].set_lowpass(fs, 5000.0, 0.7);
        Self { fs, v, voices: [Voice::default(); VOICES], clock: 0, order: 0, pedal: false, violin, viola, cello, bass, horn, trumpet, ens: StringEnsemble::new(fs), dc: [DcBlock::new(fs), DcBlock::new(fs)] }
    }
    fn reset(&mut self) {
        self.voices = [Voice::default(); VOICES];
        self.ens.reset();
        for b in self.violin.iter_mut().chain(self.horn.iter_mut()).chain(self.trumpet.iter_mut()) {
            b.reset();
        }
        self.viola.reset();
        self.cello.reset();
        self.bass.reset();
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn midi(&mut self, ev: Midi) {
        match ev {
            Midi::On { note, .. } => {
                // no touch sensitivity: a key is on or off
                self.order += 1;
                let i = self.voices.iter().position(|v| v.on && v.note == note).or_else(|| self.voices.iter().position(|v| !v.on)).unwrap_or_else(|| self.voices.iter().enumerate().min_by_key(|(_, v)| v.age).map(|(i, _)| i).unwrap_or(0));
                let vc = &mut self.voices[i];
                if !(vc.on && vc.note == note) {
                    vc.env = 0.0;
                }
                vc.on = true;
                vc.held = true;
                vc.note = note;
                vc.age = self.order;
                vc.f = 440.0 * 2f64.powf((note as f64 - 69.0) / 12.0);
            }
            Midi::Off { note } => self.voices.iter_mut().filter(|v| v.on && v.note == note).for_each(|v| v.held = false),
            Midi::Cc { cc: 64, value } => self.pedal = value >= 0.5,
            Midi::AllOff => self.voices.iter_mut().for_each(|v| v.on = false),
            _ => {}
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let p = self.v;
        let on = |i: usize| if p[i] >= 0.5 { 1.0 } else { 0.0 };
        let (vn, va, vc_, vb, hn, tr) = (on(VIOLIN), on(VIOLA), on(CELLO), on(BASS), on(HORN), on(TRUMPET));
        let att = 1.0 / (p[CRESC].max(1.0) as f64 * 0.001 * fs);
        let rel = (-6.9 / (p[SUSTAIN].max(1.0) as f64 * 0.001 * fs)).exp();
        let ens_on = p[ENSEMBLE] >= 0.5;
        let vol = db_to_gain(p[VOLUME] as f64);
        let pedal = self.pedal;
        let mut peak = 0.0f64;
        for k in 0..n {
            let t = self.clock as f64 / fs;
            self.clock += 1;
            let (mut b8, mut b16, mut b4, mut bsq, mut blow) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for vc in self.voices.iter_mut().filter(|v| v.on) {
                if vc.held || pedal {
                    vc.env = (vc.env + att).min(1.0);
                } else {
                    vc.env *= rel;
                    if vc.env < 1e-5 {
                        vc.on = false;
                        continue;
                    }
                }
                let e = vc.env;
                let ph8 = (vc.f * t).fract();
                let dt = vc.f / fs;
                let s8 = saw(ph8, dt);
                b8 += s8 * e;
                if vc_ + vb > 0.0 {
                    let s16 = saw((0.5 * vc.f * t).fract(), 0.5 * dt);
                    b16 += s16 * e;
                    // the contrabass only sounds in the bottom of the keyboard
                    if vc.note < 53 {
                        blow += s16 * e;
                    }
                }
                if vn > 0.0 {
                    b4 += saw((2.0 * vc.f * t).fract(), (2.0 * dt).min(0.45)) * e;
                }
                if hn > 0.0 {
                    bsq += if ph8 < 0.5 { e } else { -e };
                }
            }
            let mut strings = 0.0;
            if vn > 0.0 {
                strings += cascade(&mut self.violin, b8 + 0.45 * b4);
            }
            if va > 0.0 {
                strings += 0.9 * self.viola.process(b8);
            }
            if vc_ > 0.0 {
                strings += 0.9 * self.cello.process(b16 + 0.4 * b8);
            }
            let mut brass = 0.0;
            if hn > 0.0 {
                brass += 1.0 * cascade(&mut self.horn, bsq);
            }
            if tr > 0.0 {
                brass += 1.1 * cascade(&mut self.trumpet, b8);
            }
            let bass = if vb > 0.0 { 1.2 * self.bass.process(blow) } else { 0.0 };
            let x = (strings + brass) * 0.12;
            let (l, r) = if ens_on { self.ens.process(x, 0.63, 6.1, 1.0) } else { (x, x) };
            let l = self.dc[0].process(l + bass * 0.12) * vol;
            let r = self.dc[1].process(r + bass * 0.12) * vol;
            peak = peak.max(l.abs()).max(r.abs());
            if chans.len() >= 2 {
                chans[0][k] = l as f32;
                chans[1][k] = r as f32;
            } else if let Some(c) = chans.get_mut(0) {
                c[k] = (0.5 * (l + r)) as f32;
            }
        }
        meters[0] = peak as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Violins and Violas", values: &[(VIOLIN, 1.0), (VIOLA, 1.0), (CELLO, 0.0), (BASS, 0.0), (HORN, 0.0), (TRUMPET, 0.0), (ENSEMBLE, 1.0)] },
        Preset { name: "Full Strings", values: &[(VIOLIN, 1.0), (VIOLA, 1.0), (CELLO, 1.0), (BASS, 1.0), (HORN, 0.0), (TRUMPET, 0.0), (CRESC, 300.0)] },
        Preset { name: "Cellos and Bass", values: &[(VIOLIN, 0.0), (VIOLA, 0.0), (CELLO, 1.0), (BASS, 1.0), (HORN, 0.0), (TRUMPET, 0.0)] },
        Preset { name: "Slow Swell Pad", values: &[(VIOLIN, 1.0), (VIOLA, 1.0), (CELLO, 1.0), (CRESC, 1500.0), (SUSTAIN, 3500.0)] },
        Preset { name: "Strings and Horn", values: &[(VIOLIN, 1.0), (VIOLA, 0.0), (CELLO, 1.0), (HORN, 1.0), (TRUMPET, 0.0)] },
        Preset { name: "Brass Section", values: &[(VIOLIN, 0.0), (VIOLA, 0.0), (CELLO, 0.0), (BASS, 0.0), (HORN, 1.0), (TRUMPET, 1.0), (CRESC, 40.0), (SUSTAIN, 300.0)] },
        Preset { name: "Dry Divider (No Ensemble)", values: &[(VIOLIN, 1.0), (VIOLA, 1.0), (ENSEMBLE, 0.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "STRING ENSEMBLE",
        subtitle: ("IN THE MANNER OF THE ARP SOLINA SE-IV", "DIVIDE-DOWN · SIX SECTIONS · THREE-PHASE ENSEMBLE"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::Text { x: 320.0, y: 92.0, text: "SECTIONS", size: 11.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: VIOLIN, x: 130.0, y: 128.0, labels: ["", "VIOLIN"], tip: "Violins: the bright section, with an octave above mixed in." },
            Item::Switch { p: VIOLA, x: 320.0, y: 128.0, labels: ["", "VIOLA"], tip: "Violas: the same pitch, darker." },
            Item::Switch { p: CELLO, x: 510.0, y: 128.0, labels: ["", "CELLO"], tip: "Cellos: an octave down." },
            Item::Switch { p: BASS, x: 130.0, y: 168.0, labels: ["", "CONTRABASS"], tip: "Contrabass: an octave down, in the lower keys only (below F3), outside the ensemble." },
            Item::Switch { p: HORN, x: 320.0, y: 168.0, labels: ["", "HORN"], tip: "Horn: a mellow square-wave voice." },
            Item::Switch { p: TRUMPET, x: 510.0, y: 168.0, labels: ["", "TRUMPET"], tip: "Trumpet: a bright, nasal voice." },
            Item::Divider { y: 206.0 },
            Item::Knob { p: CRESC, x: 110.0, y: 290.0, r: 30.0, label: "CRESCENDO", tip: "How slowly each note swells in.", active: None },
            Item::Knob { p: SUSTAIN, x: 250.0, y: 290.0, r: 30.0, label: "SUSTAIN", tip: "How long each note fades after the key comes up. The sustain pedal holds them.", active: None },
            Item::Switch { p: ENSEMBLE, x: 400.0, y: 290.0, labels: ["", "ENSEMBLE"], tip: "The ensemble: three swept delays that make the divided-down saws a string section. Off, it's flat and organ-like." },
            Item::Knob { p: VOLUME, x: 560.0, y: 290.0, r: 28.0, label: "VOLUME", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "String Ensemble", class_id: *b"PQKeysStrEnsSoli", vendor: "PEQUOD Keys",
    instrument, categories: [Instrument, Synth]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        with(defaults(), &[(VIOLIN, 0.0), (VIOLA, 1.0), (ENSEMBLE, 0.0), (CRESC, 5.0), (SUSTAIN, 100.0)])
    }

    include!("../../instrument_tests.rs");
}
