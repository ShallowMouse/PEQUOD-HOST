//! Bass Pedals: in the manner of the Moog Taurus I bass pedal synthesizer.
//!
//! Thirteen pedals of enormous bass: two oscillators (the second beats against the first, or sits
//! a fifth or an octave away), Moog's ladder filter with its contour, and a loudness contour, all
//! monophonic with low-note priority and glide. The Taurus I had three preset sounds - Tuba,
//! Taurus and Bass - and one you set yourself; here every control is the "set yourself" one and
//! the three are presets. It plays from any keyboard; the pedals' own octave (C to C) is the
//! OCTAVE switch's home.

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

fn stepped(name: &str, default: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max }).with_step_size(1.0)
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "octave"]
    pub octave: FloatParam,
    #[id = "wave"]
    pub wave: FloatParam,
    #[id = "vco2"]
    pub vco2: FloatParam,
    #[id = "interval"]
    pub interval: FloatParam,
    #[id = "beat"]
    pub beat: FloatParam,
    #[id = "cutoff"]
    pub cutoff: FloatParam,
    #[id = "emph"]
    pub emph: FloatParam,
    #[id = "contour"]
    pub contour: FloatParam,
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "sustain"]
    pub sustain: FloatParam,
    #[id = "glide"]
    pub glide: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            octave: stepped("Octave", 1.0, 2.0),
            wave: stepped("Waveform", 0.0, 1.0),
            vco2: pct("VCO 2 Level", 0.7),
            interval: stepped("VCO 2 Interval", 0.0, 2.0),
            beat: FloatParam::new("VCO 2 Beat", 8.0, FloatRange::Linear { min: 0.0, max: 40.0 }).with_step_size(0.5).with_unit(" ct"),
            cutoff: hz("Filter Cutoff", 350.0, 30.0, 5000.0),
            emph: pct("Emphasis", 0.3),
            contour: pct("Contour Amount", 0.5),
            attack: ms("Attack", 5.0, 1.0, 2000.0),
            decay: ms("Decay", 800.0, 10.0, 6000.0),
            sustain: pct("Sustain", 0.7),
            glide: pct("Glide", 0.0),
            volume: db("Volume", -6.0, -30.0, 6.0),
        }
    }
}

pub const OCTAVE: usize = 0;
pub const WAVE: usize = 1;
pub const VCO2: usize = 2;
pub const INTERVAL: usize = 3;
pub const BEAT: usize = 4;
pub const CUTOFF: usize = 5;
pub const EMPH: usize = 6;
pub const CONTOUR: usize = 7;
pub const ATTACK: usize = 8;
pub const DECAY: usize = 9;
pub const SUSTAIN: usize = 10;
pub const GLIDE: usize = 11;
pub const VOLUME: usize = 12;
pub const NPARAMS: usize = 13;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.octave.as_ptr(),
            self.wave.as_ptr(),
            self.vco2.as_ptr(),
            self.interval.as_ptr(),
            self.beat.as_ptr(),
            self.cutoff.as_ptr(),
            self.emph.as_ptr(),
            self.contour.as_ptr(),
            self.attack.as_ptr(),
            self.decay.as_ptr(),
            self.sustain.as_ptr(),
            self.glide.as_ptr(),
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

#[inline]
fn osc(pulse: bool, ph: f64, dt: f64) -> f64 {
    if pulse {
        // the Taurus's narrow pulse: hollow and reedy
        let w = 0.25;
        let mut q = if ph < w { 1.0 } else { -1.0 };
        q += blep(ph, dt);
        q -= blep((ph - w + 1.0).fract(), dt);
        q - (2.0 * w - 1.0)
    } else {
        2.0 * ph - 1.0 - blep(ph, dt)
    }
}

/// An ADS contour (attack, decay to sustain; release at the decay time, as the Taurus does).
#[derive(Clone, Copy, Default)]
struct Contour {
    v: f64,
    attacking: bool,
}

impl Contour {
    #[inline]
    fn step(&mut self, gate: bool, ca: f64, cd: f64, s: f64) -> f64 {
        if gate {
            if self.attacking {
                self.v += (1.3 - self.v) * ca;
                if self.v >= 1.0 {
                    self.v = 1.0;
                    self.attacking = false;
                }
            } else {
                self.v += (s - self.v) * cd;
            }
        } else {
            self.attacking = false;
            self.v -= self.v * cd;
        }
        self.v
    }
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    held: Vec<u8>,
    gate: bool,
    pitch: f64,
    target: f64,
    ph: [f64; 2],
    lad: [f64; 4],
    fenv: Contour,
    aenv: Contour,
    boost: Biquad,
    dc: DcBlock,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        // the Taurus's famous bottom: its output stage leans on the lows
        let mut boost = Biquad::identity();
        boost.set_low_shelf(fs, 90.0, 0.7, 4.0);
        Self { fs, v, held: Vec::with_capacity(16), gate: false, pitch: 36.0, target: 36.0, ph: [0.0, 0.37], lad: [0.0; 4], fenv: Contour::default(), aenv: Contour::default(), boost, dc: DcBlock::new(fs) }
    }
    fn reset(&mut self) {
        self.held.clear();
        self.gate = false;
        self.lad = [0.0; 4];
        self.fenv = Contour::default();
        self.aenv = Contour::default();
        self.boost.reset();
        self.dc.reset();
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn midi(&mut self, ev: Midi) {
        match ev {
            Midi::On { note, .. } => {
                let first = self.held.is_empty();
                self.held.retain(|n| *n != note);
                self.held.push(note);
                if let Some(n) = self.held.iter().min() {
                    self.target = *n as f64;
                }
                if first {
                    self.gate = true;
                    self.fenv.attacking = true;
                    self.aenv.attacking = true;
                    if self.v[GLIDE] <= 0.0 {
                        self.pitch = self.target;
                    }
                }
            }
            Midi::Off { note } => {
                self.held.retain(|n| *n != note);
                match self.held.iter().min() {
                    Some(n) => self.target = *n as f64,
                    None => self.gate = false,
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
        // OCTAVE: 32' (two down), 16' (one down), 8' (as played)
        let oct = -(2.0 - p[OCTAVE].round().clamp(0.0, 2.0) as f64);
        let pulse = p[WAVE] >= 0.5;
        let v2 = p[VCO2] as f64;
        let interval = [0.0, 7.0, 12.0][p[INTERVAL].round().clamp(0.0, 2.0) as usize];
        let beat = p[BEAT] as f64 / 100.0;
        let cut = p[CUTOFF] as f64;
        let k = 4.0 * p[EMPH] as f64;
        let amount = 5.0 * p[CONTOUR] as f64;
        let ca = coeff(fs, p[ATTACK].max(1.0) as f64 / 2.2);
        let cd = coeff(fs, p[DECAY].max(1.0) as f64 / 2.2);
        let sus = p[SUSTAIN] as f64;
        let glide = p[GLIDE] as f64;
        let gc = if glide > 0.0 { coeff(fs, 5.0 + 2000.0 * glide * glide) } else { 0.0 };
        let vol = db_to_gain(p[VOLUME] as f64);
        let mut peak = 0.0f64;
        for kk in 0..n {
            self.pitch = if gc > 0.0 { self.target + gc * (self.pitch - self.target) } else { self.target };
            let note = self.pitch + 12.0 * oct;
            let f1 = 440.0 * 2f64.powf((note - 69.0) / 12.0);
            let f2 = 440.0 * 2f64.powf((note + interval + beat - 69.0) / 12.0);
            let (d1, d2) = ((f1 / fs).min(0.45), (f2 / fs).min(0.45));
            let x = osc(pulse, self.ph[0], d1) + v2 * osc(pulse, self.ph[1], d2);
            self.ph[0] = (self.ph[0] + d1).fract();
            self.ph[1] = (self.ph[1] + d2).fract();
            let fe = self.fenv.step(self.gate, ca, cd, sus);
            let ae = self.aenv.step(self.gate, ca, cd, sus);
            let fc = (cut * 2f64.powf(amount * fe)).clamp(10.0, 0.42 * fs);
            let g = 1.0 - (-std::f64::consts::TAU * fc / (2.0 * fs)).exp();
            let xin = (x * 0.6).tanh();
            for _ in 0..2 {
                let u = (xin - k * self.lad[3]).tanh();
                self.lad[0] += g * (u - self.lad[0].tanh());
                self.lad[1] += g * (self.lad[0].tanh() - self.lad[1].tanh());
                self.lad[2] += g * (self.lad[1].tanh() - self.lad[2].tanh());
                self.lad[3] += g * (self.lad[2].tanh() - self.lad[3].tanh());
            }
            let y = self.boost.process(self.dc.process(self.lad[3] * (1.0 + 0.35 * k))) * ae * vol * 2.0;
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
        Preset { name: "Taurus", values: &[(OCTAVE, 1.0), (WAVE, 0.0), (VCO2, 0.8), (INTERVAL, 0.0), (BEAT, 10.0), (CUTOFF, 300.0), (EMPH, 0.35), (CONTOUR, 0.55), (ATTACK, 3.0), (DECAY, 1500.0), (SUSTAIN, 0.75)] },
        Preset { name: "Tuba", values: &[(OCTAVE, 1.0), (WAVE, 1.0), (VCO2, 0.0), (CUTOFF, 180.0), (EMPH, 0.15), (CONTOUR, 0.6), (ATTACK, 60.0), (DECAY, 400.0), (SUSTAIN, 0.8)] },
        Preset { name: "Bass", values: &[(OCTAVE, 2.0), (WAVE, 0.0), (VCO2, 0.5), (INTERVAL, 2.0), (BEAT, 4.0), (CUTOFF, 500.0), (EMPH, 0.2), (CONTOUR, 0.4), (ATTACK, 2.0), (DECAY, 600.0), (SUSTAIN, 0.5)] },
        Preset { name: "Earthquake 32'", values: &[(OCTAVE, 0.0), (WAVE, 0.0), (VCO2, 1.0), (BEAT, 20.0), (CUTOFF, 250.0), (EMPH, 0.4), (CONTOUR, 0.5), (DECAY, 3000.0), (SUSTAIN, 0.9)] },
        Preset { name: "Fifth Drone", values: &[(OCTAVE, 1.0), (WAVE, 1.0), (VCO2, 0.7), (INTERVAL, 1.0), (BEAT, 2.0), (CUTOFF, 600.0), (ATTACK, 400.0), (SUSTAIN, 1.0)] },
        Preset { name: "Gliding Bass", values: &[(OCTAVE, 1.0), (GLIDE, 0.3), (CUTOFF, 400.0), (EMPH, 0.5), (CONTOUR, 0.5)] },
    ]
}

fn panel() -> Panel {
    let k = |p, x, y, r, label, tip| Item::Knob { p, x, y, r, label, tip, active: None };
    Panel {
        title: "BASS PEDALS",
        subtitle: ("IN THE MANNER OF THE MOOG TAURUS I", "TWO OSCILLATORS · LADDER FILTER · MONO · GLIDE"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::Selector { p: OCTAVE, x: 70.0, y: 155.0, label: "OCTAVE", positions: &["32'", "16'", "8'"], tip: "How low: 8' as played, 16' an octave down, 32' two." },
            Item::Switch { p: WAVE, x: 200.0, y: 155.0, labels: ["SAW", "PULSE"], tip: "Both oscillators' waveform." },
            k(VCO2, 310.0, 155.0, 22.0, "VCO 2", "The second oscillator's level."),
            Item::Selector { p: INTERVAL, x: 410.0, y: 155.0, label: "INTERVAL", positions: &["UNISON", "FIFTH", "OCTAVE"], tip: "Where the second oscillator sits." },
            k(BEAT, 510.0, 155.0, 22.0, "BEAT", "The second oscillator tuned a little sharp: the slow beating that makes the Taurus so thick."),
            k(VOLUME, 585.0, 155.0, 20.0, "VOLUME", "Output level."),
            Item::Divider { y: 230.0 },
            k(CUTOFF, 60.0, 320.0, 24.0, "CUTOFF", "Filter cutoff."),
            k(EMPH, 140.0, 320.0, 20.0, "EMPHASIS", "Filter resonance."),
            k(CONTOUR, 220.0, 320.0, 20.0, "CONTOUR", "How far the contour opens the filter."),
            k(ATTACK, 310.0, 320.0, 20.0, "ATTACK", "Contour attack."),
            k(DECAY, 390.0, 320.0, 20.0, "DECAY", "Contour decay (and release)."),
            k(SUSTAIN, 470.0, 320.0, 20.0, "SUSTAIN", "Contour sustain."),
            k(GLIDE, 560.0, 320.0, 20.0, "GLIDE", "Slide from note to note."),
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Bass Pedals", class_id: *b"PQKeysTaurusPedl", vendor: "PEQUOD Keys",
    instrument, categories: [Instrument, Synth]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        with(defaults(), &[(OCTAVE, 2.0), (VCO2, 0.0), (CUTOFF, 5000.0), (EMPH, 0.0), (CONTOUR, 0.0), (SUSTAIN, 1.0), (DECAY, 50.0)])
    }

    include!("../../instrument_tests.rs");
}
