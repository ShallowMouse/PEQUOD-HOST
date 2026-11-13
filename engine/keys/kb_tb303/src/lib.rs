//! Bass Line: in the manner of the Roland TB-303 Bass Line.
//!
//! One oscillator (saw, or the square the 303 makes from it), a resonant four-pole diode-ladder
//! filter, and two envelopes: the filter's, whose decay is the DECAY knob, and the amp's, which
//! holds the note. The 303's character is in its accent and slide:
//!  - ACCENT (a note played hard, velocity 100 and up) is louder, forces the filter envelope to
//!    its shortest decay, and adds a sweep through a capacitor that charges up over a run of
//!    accented notes - the "wow" that climbs as accents repeat.
//!  - SLIDE (a note played before the last one is let go) glides to the new pitch in about 60 ms
//!    without starting the envelopes again.

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
pub const H: u32 = 320;

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "wave"]
    pub wave: FloatParam,
    #[id = "tuning"]
    pub tuning: FloatParam,
    #[id = "cutoff"]
    pub cutoff: FloatParam,
    #[id = "reso"]
    pub reso: FloatParam,
    #[id = "envmod"]
    pub envmod: FloatParam,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "accent"]
    pub accent: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            wave: FloatParam::new("Waveform", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            tuning: FloatParam::new("Tuning", 0.0, FloatRange::Linear { min: -100.0, max: 100.0 }).with_step_size(1.0).with_unit(" ct"),
            cutoff: pct("Cut Off Freq", 0.35),
            reso: pct("Resonance", 0.6),
            envmod: pct("Env Mod", 0.5),
            decay: pct("Decay", 0.4),
            accent: pct("Accent", 0.6),
            volume: db("Volume", -6.0, -30.0, 6.0),
        }
    }
}

pub const WAVE: usize = 0;
pub const TUNING: usize = 1;
pub const CUTOFF: usize = 2;
pub const RESO: usize = 3;
pub const ENVMOD: usize = 4;
pub const DECAY: usize = 5;
pub const ACCENT: usize = 6;
pub const VOLUME: usize = 7;
pub const NPARAMS: usize = 8;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.wave.as_ptr(),
            self.tuning.as_ptr(),
            self.cutoff.as_ptr(),
            self.reso.as_ptr(),
            self.envmod.as_ptr(),
            self.decay.as_ptr(),
            self.accent.as_ptr(),
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

#[inline]
fn blep(t: f64, dt: f64) -> f64 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// Velocity at or above this is an accented step.
const ACCENT_VEL: f32 = 100.0 / 127.0;

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    held: Vec<u8>,
    gate: bool,
    pitch: f64,
    target: f64,
    sliding: bool,
    ph: f64,
    lad: [f64; 4],
    meg: f64,
    veg: f64,
    accented: bool,
    accent_env: f64,
    wow: f64,
    hp: Biquad,
    dc: DcBlock,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        // the 303's coupling: the filter's input and output are AC-coupled, thinning the very bottom
        let mut hp = Biquad::identity();
        hp.set_highpass(fs, 35.0, 0.7);
        Self { fs, v, held: Vec::with_capacity(16), gate: false, pitch: 45.0, target: 45.0, sliding: false, ph: 0.0, lad: [0.0; 4], meg: 0.0, veg: 0.0, accented: false, accent_env: 0.0, wow: 0.0, hp, dc: DcBlock::new(fs) }
    }
    fn reset(&mut self) {
        self.held.clear();
        self.gate = false;
        self.lad = [0.0; 4];
        self.meg = 0.0;
        self.veg = 0.0;
        self.accent_env = 0.0;
        self.wow = 0.0;
        self.hp.reset();
        self.dc.reset();
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn midi(&mut self, ev: Midi) {
        match ev {
            Midi::On { note, vel } => {
                // slide: a new note while the last is still down
                self.sliding = !self.held.is_empty();
                self.held.retain(|n| *n != note);
                self.held.push(note);
                self.target = note as f64;
                if !self.sliding {
                    self.pitch = self.target;
                    self.gate = true;
                    self.meg = 1.0;
                    self.accented = vel >= ACCENT_VEL;
                    if self.accented {
                        self.accent_env = 1.0;
                    }
                }
            }
            Midi::Off { note } => {
                self.held.retain(|n| *n != note);
                if let Some(n) = self.held.last() {
                    self.target = *n as f64;
                    self.sliding = true;
                } else {
                    self.gate = false;
                }
            }
            Midi::AllOff => {
                self.held.clear();
                self.gate = false;
            }
            _ => {}
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let p = self.v;
        let square = p[WAVE] >= 0.5;
        let tune = p[TUNING] as f64 / 100.0;
        let (cut, reso, envmod, decay, accent) = (p[CUTOFF] as f64, p[RESO] as f64, p[ENVMOD] as f64, p[DECAY] as f64, p[ACCENT] as f64);
        let vol = db_to_gain(p[VOLUME] as f64);
        // the filter's own range: about 300 Hz to 2.4 kHz from the knob, the envelope on top
        let base = 300.0 * 8f64.powf(cut);
        let k = 3.95 * reso;
        // the filter envelope: DECAY 200 ms to 2 s; accented notes always take the shortest
        let meg_ms = if self.accented { 200.0 } else { 200.0 * 10f64.powf(decay) };
        let meg_k = (-6.9 / (meg_ms * 0.001 * fs)).exp();
        // the amp envelope: a fast rise, held while the key is down (with a slow sag), fast off
        let veg_up = coeff(fs, 3.0);
        let veg_hold = (-1.0 / (3.0 * fs)).exp();
        let veg_off = coeff(fs, 8.0);
        // the accent sweep: its capacitor charges with each accented note and drains slowly
        let wow_c = coeff(fs, 120.0 - 80.0 * reso);
        let acc_k = (-6.9 / (0.2 * fs)).exp();
        let slide = coeff(fs, 60.0);
        let mut peak = 0.0f64;
        for kk in 0..n {
            self.pitch = if self.sliding { self.target + slide * (self.pitch - self.target) } else { self.target };
            let f = 440.0 * 2f64.powf((self.pitch + tune - 69.0) / 12.0);
            let dt = (f / fs).min(0.45);
            let saw = 2.0 * self.ph - 1.0 - blep(self.ph, dt);
            let osc = if square {
                // the 303's square is shaped from the saw by a transistor: a pulse a little under half
                let w = 0.45;
                let mut q = if self.ph < w { 1.0 } else { -1.0 };
                q += blep(self.ph, dt);
                q -= blep((self.ph - w + 1.0).fract(), dt);
                q
            } else {
                saw
            };
            self.ph = (self.ph + dt).fract();
            // envelopes
            self.meg *= meg_k;
            if self.gate {
                self.veg = if self.veg < 0.98 { self.veg + (1.0 - self.veg) * veg_up } else { self.veg * veg_hold };
            } else {
                self.veg *= 1.0 - veg_off;
            }
            self.accent_env *= acc_k;
            let acc = if self.accented { accent } else { 0.0 };
            self.wow += wow_c * (self.accent_env * acc - self.wow);
            let fc = (base * 2f64.powf(envmod * 4.2 * self.meg + acc * (3.0 * self.wow + 1.2 * self.meg))).clamp(20.0, 0.42 * fs);
            // the diode ladder (run twice a sample)
            let g = 1.0 - (-std::f64::consts::TAU * fc / (2.0 * fs)).exp();
            let x = self.hp.process(osc * 0.8);
            for _ in 0..2 {
                let u = (x - k * self.lad[3]).tanh();
                self.lad[0] += g * (u - self.lad[0].tanh());
                self.lad[1] += g * (self.lad[0].tanh() - self.lad[1].tanh());
                self.lad[2] += g * (self.lad[1].tanh() - self.lad[2].tanh());
                self.lad[3] += g * (self.lad[2].tanh() - self.lad[3].tanh());
            }
            let level = self.veg * (1.0 + 0.8 * acc);
            let y = self.dc.process(self.lad[3] * (1.0 + 0.5 * k)) * level * vol;
            peak = peak.max(y.abs());
            for c in chans.iter_mut() {
                c[kk] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Acid Squelch", values: &[(WAVE, 0.0), (CUTOFF, 0.25), (RESO, 0.85), (ENVMOD, 0.7), (DECAY, 0.35), (ACCENT, 0.8)] },
        Preset { name: "Round Square Bass", values: &[(WAVE, 1.0), (CUTOFF, 0.3), (RESO, 0.3), (ENVMOD, 0.3), (DECAY, 0.5), (ACCENT, 0.4)] },
        Preset { name: "Screaming Resonance", values: &[(WAVE, 0.0), (CUTOFF, 0.45), (RESO, 0.97), (ENVMOD, 0.6), (DECAY, 0.2), (ACCENT, 1.0)] },
        Preset { name: "Dark Rubber", values: &[(WAVE, 0.0), (CUTOFF, 0.1), (RESO, 0.5), (ENVMOD, 0.4), (DECAY, 0.7), (ACCENT, 0.5)] },
        Preset { name: "Open Saw", values: &[(WAVE, 0.0), (CUTOFF, 0.8), (RESO, 0.2), (ENVMOD, 0.2), (DECAY, 0.6), (ACCENT, 0.3)] },
    ]
}

fn panel() -> Panel {
    let k = |p, x, y, r, label, tip| Item::Knob { p, x, y, r, label, tip, active: None };
    Panel {
        title: "BASS LINE",
        subtitle: ("IN THE MANNER OF THE ROLAND TB-303", "SAW / SQUARE · DIODE LADDER · ACCENT · SLIDE"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::Switch { p: WAVE, x: 80.0, y: 160.0, labels: ["SAW", "SQUARE"], tip: "Waveform: the saw, or the square the 303 shapes from it." },
            k(TUNING, 180.0, 160.0, 20.0, "TUNING", "Fine tuning, a semitone either way."),
            k(CUTOFF, 255.0, 160.0, 24.0, "CUT OFF", "The filter's cutoff frequency."),
            k(RESO, 335.0, 160.0, 24.0, "RESONANCE", "Filter resonance: up high, the squelch."),
            k(ENVMOD, 415.0, 160.0, 24.0, "ENV MOD", "How far each note's envelope sweeps the filter."),
            k(DECAY, 495.0, 160.0, 24.0, "DECAY", "How quickly the filter sweep falls (accented notes always fall fast)."),
            k(ACCENT, 575.0, 160.0, 24.0, "ACCENT", "Notes played hard (velocity 100 and up): louder, with a filter sweep that climbs over a run of accents."),
            k(VOLUME, 575.0, 262.0, 18.0, "VOLUME", "Output level."),
            Item::Text { x: 260.0, y: 262.0, text: "SLIDE: PLAY THE NEXT NOTE BEFORE LETTING GO", size: 10.0, align: Align2::CENTER_CENTER },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Bass Line", class_id: *b"PQKeysBassLn303R", vendor: "PEQUOD Keys",
    instrument, categories: [Instrument, Synth]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        with(defaults(), &[(CUTOFF, 1.0), (RESO, 0.0), (ENVMOD, 0.0), (ACCENT, 0.0)])
    }

    include!("../../instrument_tests.rs");

    /// Overlapping notes slide (the pitch moves without the filter envelope starting again).
    #[test]
    fn slides() {
        let p = tune_params();
        let ev = [(0, Midi::On { note: 45, vel: 0.5 }), (24000, Midi::On { note: 57, vel: 0.5 }), (26000, Midi::Off { note: 45 })];
        let (l, _) = render(&p, &ev, 60000);
        let f1 = pitch(&l[8000..]);
        let f2 = pitch(&l[40000..]);
        println!("before {f1:.1} Hz, after the slide {f2:.1} Hz");
        assert!((f1 - 110.0).abs() < 2.0 && (f2 - 220.0).abs() < 3.0);
    }
}
