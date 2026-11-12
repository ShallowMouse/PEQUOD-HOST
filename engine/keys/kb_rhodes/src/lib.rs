//! Stage 73: an electric piano in the manner of the Fender Rhodes Mark I (Stage and Suitcase).
//!
//! A neoprene-tipped hammer strikes a steel tine coupled to a brass tonebar; the tine swings in
//! front of a magnetic pickup. The pickup is what makes the Rhodes: its output isn't a straight
//! copy of the tine's motion but follows the magnetic field, which falls away either side of the
//! pickup's centre. Played softly the tine stays near the middle and the note is round and
//! bell-like; played hard it swings out of the linear region and the note "barks". The VOICING
//! knob is the technician's tine-to-pickup alignment: further off centre gives more bark.
//!
//! Each note also has the tine's short, inharmonic strike partial (the bell), decays long in the
//! bass and short at the top (fast at first, then slower, as the tine hands its energy to the
//! tonebar), and is stopped by a felt damper when the key comes up. The Suitcase's preamp, its
//! bass and treble, and its stereo "vibrato" (really an auto-pan between the two sides).

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
pub const H: u32 = 432;

const VOICES: usize = 16;

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "voicing"]
    pub voicing: FloatParam,
    #[id = "hammer"]
    pub hammer: FloatParam,
    #[id = "sustain"]
    pub sustain: FloatParam,
    #[id = "bass"]
    pub bass: FloatParam,
    #[id = "treble"]
    pub treble: FloatParam,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "vibrate"]
    pub vib_rate: FloatParam,
    #[id = "vib"]
    pub vib: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            voicing: pct("Voicing", 0.5),
            hammer: pct("Hammer", 0.5),
            sustain: pct("Sustain", 0.6),
            bass: db("Bass", 0.0, -12.0, 12.0),
            treble: db("Treble", 0.0, -12.0, 12.0),
            drive: pct("Preamp Drive", 0.15),
            vib_rate: rate("Vibrato Rate", 4.0, 0.5, 10.0),
            vib: pct("Vibrato Depth", 0.0),
            volume: db("Volume", -6.0, -30.0, 6.0),
        }
    }
}

pub const VOICING: usize = 0;
pub const HAMMER: usize = 1;
pub const SUSTAIN: usize = 2;
pub const BASS: usize = 3;
pub const TREBLE: usize = 4;
pub const DRIVE: usize = 5;
pub const VIBRATE: usize = 6;
pub const VIB: usize = 7;
pub const VOLUME: usize = 8;
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
            self.voicing.as_ptr(),
            self.hammer.as_ptr(),
            self.sustain.as_ptr(),
            self.bass.as_ptr(),
            self.treble.as_ptr(),
            self.drive.as_ptr(),
            self.vib_rate.as_ptr(),
            self.vib.as_ptr(),
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

/// The pickup's flux against tine displacement (in units of the field's width): a bell
/// that falls away either side of the pole piece.
#[inline]
fn flux(x: f64) -> f64 {
    1.0 / (1.0 + x * x)
}

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
    a_long: f64,
    a_short: f64,
    k_long: f64,
    k_short: f64,
    bell: f64,
    k_bell: f64,
    damp: f64,
    thump: f64,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    voices: [Voice; VOICES],
    pedal: bool,
    noise: Noise,
    thump_lp: OnePole,
    dc: DcBlock,
    eq: [Biquad; 2],
    eq_set: (f32, f32),
    lfo: Lfo,
}

impl Dsp {
    fn strike(&mut self, note: u8, vel: f64) {
        let fs = self.fs;
        // one tine per key: a held note is struck again; otherwise a free voice or the oldest
        let i = self.voices.iter().position(|v| v.on && v.note == note).or_else(|| self.voices.iter().position(|v| !v.on)).unwrap_or_else(|| self.voices.iter().enumerate().max_by_key(|(_, v)| v.age).map(|(i, _)| i).unwrap_or(0));
        // the 73 runs E1 to E7; anything outside folds in by octaves
        let mut n = note as i32;
        while n < 28 {
            n += 12;
        }
        while n > 100 {
            n -= 12;
        }
        let f = 440.0 * 2f64.powf((n as f64 - 69.0) / 12.0);
        let sus = self.v[SUSTAIN] as f64;
        let hammer = self.v[HAMMER] as f64;
        // decay: long in the bass, short at the top; a quick early drop then the slow tail
        let t60 = (16.0 * (110.0 / f).powf(0.75)).clamp(0.9, 24.0) * (0.35 + 1.3 * sus);
        let t60_short = (t60 * 0.12).max(0.15);
        let k = |t: f64| 10f64.powf(-3.0 / (t * fs));
        let vc = &mut self.voices[i];
        let carry = if vc.on && vc.note == note { vc.ph } else { 0.0 };
        *vc = Voice::default();
        vc.on = true;
        vc.held = true;
        vc.note = note;
        vc.ph = carry;
        vc.inc = f / fs;
        // the tine's first cantilever overtone: about 6.27 times up (inharmonic: the bell)
        let f2 = 6.267 * f;
        vc.inc2 = if f2 < 0.45 * fs { f2 / fs } else { 0.0 };
        let a = 0.08 + 0.92 * vel.powf(1.4);
        vc.a_long = 0.75 * a;
        vc.a_short = 0.25 * a;
        vc.k_long = k(t60);
        vc.k_short = k(t60_short);
        vc.bell = a * (0.08 + 0.35 * hammer) * (0.5 + 0.5 * vel) * (220.0 / f).powf(0.25).min(1.6);
        vc.k_bell = k((0.03 * (440.0 / f).powf(0.5)).clamp(0.006, 0.12));
        vc.damp = 1.0;
        vc.thump = vel * (0.02 + 0.06 * hammer);
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut thump_lp = OnePole::default();
        thump_lp.set_lowpass(fs, 300.0);
        Self { fs, v, voices: [Voice::default(); VOICES], pedal: false, noise: Noise::new(73), thump_lp, dc: DcBlock::new(fs), eq: [Biquad::identity(); 2], eq_set: (f32::NAN, f32::NAN), lfo: Lfo::default() }
    }
    fn reset(&mut self) {
        self.voices = [Voice::default(); VOICES];
        self.thump_lp.reset();
        self.dc.reset();
        self.eq.iter_mut().for_each(|b| b.reset());
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn midi(&mut self, ev: Midi) {
        match ev {
            Midi::On { note, vel } => self.strike(note, vel as f64),
            Midi::Off { note } => self.voices.iter_mut().filter(|v| v.on && v.note == note).for_each(|v| v.held = false),
            Midi::Cc { cc: 64, value } => self.pedal = value >= 0.5,
            Midi::AllOff => self.voices.iter_mut().for_each(|v| v.on = false),
            _ => {}
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        if self.eq_set != (self.v[BASS], self.v[TREBLE]) {
            self.eq_set = (self.v[BASS], self.v[TREBLE]);
            self.eq[0].set_low_shelf(fs, 200.0, 0.7, self.v[BASS] as f64);
            self.eq[1].set_high_shelf(fs, 2800.0, 0.7, self.v[TREBLE] as f64);
        }
        // tine-to-pickup offset, in field widths: from nearly centred (round) to well off (bark)
        let q = 0.35 + 0.65 * self.v[VOICING] as f64;
        let g0 = 2.0 * q / (1.0 + q * q).powi(2);
        let f0 = flux(q);
        // how far the tine swings at full velocity, in field widths
        let swing = 0.55 + 0.35 * self.v[VOICING] as f64;
        // the felt damper: a fast but not instant stop
        let damper = 10f64.powf(-3.0 / (0.12 * fs));
        let drive = self.v[DRIVE] as f64;
        let pre = 1.0 + 6.0 * drive * drive;
        let vol = db_to_gain(self.v[VOLUME] as f64);
        let (rate, depth) = (self.v[VIBRATE] as f64, self.v[VIB] as f64);
        let pedal = self.pedal;
        let mut peak = 0.0f64;
        for k in 0..n {
            let mut sum = 0.0;
            let mut thump = 0.0;
            for vc in self.voices.iter_mut().filter(|v| v.on) {
                let x = (vc.a_long + vc.a_short) * (TAU * vc.ph).sin() + vc.bell * (TAU * vc.ph2).sin();
                // the pickup: change in flux as the tine moves off its rest point
                sum += -(flux(q + swing * x) - f0) / (g0 * swing);
                thump += vc.thump;
                vc.ph = (vc.ph + vc.inc).fract();
                vc.ph2 = (vc.ph2 + vc.inc2).fract();
                let d = if vc.held || pedal { 1.0 } else { damper };
                vc.a_long *= vc.k_long * d;
                vc.a_short *= vc.k_short * d;
                vc.bell *= vc.k_bell;
                vc.thump *= 0.995;
                vc.age += 1;
                if vc.age % 64 == 0 && vc.a_long + vc.a_short < 1e-5 {
                    vc.on = false;
                }
            }
            let y = self.dc.process(sum * 0.3 + self.thump_lp.lp(self.noise.next() * thump));
            let y = cascade(&mut self.eq, y);
            let y = tube(y * pre, 0.12) / pre.powf(0.5) * vol;
            // the Suitcase's stereo vibrato: the two sides swap level
            let s = self.lfo.next(fs, rate);
            let l = y * (1.0 - depth * 0.5 * (1.0 + s));
            let r = y * (1.0 - depth * 0.5 * (1.0 - s));
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
        Preset { name: "Stage 73, Dry", values: &[(VOICING, 0.5), (HAMMER, 0.5), (VIB, 0.0)] },
        Preset { name: "Suitcase Vibrato", values: &[(VIB, 0.6), (VIBRATE, 4.5), (BASS, 3.0), (TREBLE, 2.0)] },
        Preset { name: "Mellow Ballad", values: &[(VOICING, 0.25), (HAMMER, 0.3), (TREBLE, -4.0), (VIB, 0.35), (VIBRATE, 3.0)] },
        Preset { name: "Barking Funk", values: &[(VOICING, 0.95), (HAMMER, 0.8), (DRIVE, 0.6), (TREBLE, 3.0), (VIB, 0.0)] },
        Preset { name: "Bell Tines", values: &[(VOICING, 0.35), (HAMMER, 1.0), (TREBLE, 4.0), (VIB, 0.2)] },
        Preset { name: "Driven Suitcase", values: &[(VOICING, 0.75), (DRIVE, 0.8), (VIB, 0.5), (VIBRATE, 5.5)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "STAGE 73",
        subtitle: ("IN THE MANNER OF THE FENDER RHODES MARK I", "TINES · TONEBARS · MAGNETIC PICKUPS · SUITCASE"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::Knob { p: VOICING, x: 90.0, y: 160.0, r: 36.0, label: "VOICING", tip: "Where the tines sit against the pickups. Near centre the notes are round and bell-like; further off they bark when you dig in.", active: None },
            Item::Knob { p: HAMMER, x: 210.0, y: 160.0, r: 36.0, label: "HAMMER", tip: "Hammer tip hardness: harder gives more of the bell-like strike and the thump.", active: None },
            Item::Knob { p: SUSTAIN, x: 330.0, y: 160.0, r: 36.0, label: "SUSTAIN", tip: "How long the tines ring. The sustain pedal (CC64) lifts the dampers.", active: None },
            Item::Knob { p: DRIVE, x: 450.0, y: 160.0, r: 36.0, label: "PREAMP", tip: "How hard the Suitcase preamp is pushed.", active: None },
            Item::Knob { p: VOLUME, x: 565.0, y: 160.0, r: 30.0, label: "VOLUME", tip: "Output level.", active: None },
            Item::Divider { y: 264.0 },
            Item::Knob { p: BASS, x: 90.0, y: 330.0, r: 22.0, label: "BASS", tip: "The Suitcase's bass control.", active: None },
            Item::Knob { p: TREBLE, x: 200.0, y: 330.0, r: 22.0, label: "TREBLE", tip: "The Suitcase's treble control.", active: None },
            Item::Knob { p: VIBRATE, x: 380.0, y: 330.0, r: 22.0, label: "VIB RATE", tip: "Speed of the stereo vibrato.", active: None },
            Item::Knob { p: VIB, x: 490.0, y: 330.0, r: 22.0, label: "VIBRATO", tip: "The Suitcase's stereo vibrato: the sound swings between the left and right speakers.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Stage 73", class_id: *b"PQKeysStage73Rhd", vendor: "PEQUOD Keys",
    instrument, categories: [Instrument, Piano]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        with(defaults(), &[(VIB, 0.0), (DRIVE, 0.0), (HAMMER, 0.0)])
    }

    include!("../../instrument_tests.rs");

    fn goertzel(x: &[f32], f: f64) -> f64 {
        let w = TAU * f / 48000.0;
        let (mut re, mut im) = (0.0, 0.0);
        for (i, v) in x.iter().enumerate() {
            re += *v as f64 * (w * i as f64).cos();
            im += *v as f64 * (w * i as f64).sin();
        }
        (re * re + im * im).sqrt() / x.len() as f64
    }

    /// Harder playing barks: more 2nd and 3rd harmonic relative to the fundamental.
    #[test]
    fn velocity_barks() {
        let p = with(tune_params(), &[(VOICING, 0.8)]);
        let bark = |vel: f32| {
            let (l, _) = render(&p, &[(0, Midi::On { note: 57, vel })], 24000);
            let x = &l[4800..24000];
            (goertzel(x, 440.0) + goertzel(x, 660.0)) / goertzel(x, 220.0)
        };
        let (soft, hard) = (bark(0.2), bark(1.0));
        println!("harmonics soft {soft:.4} hard {hard:.4}");
        assert!(hard > 3.0 * soft);
    }
}
