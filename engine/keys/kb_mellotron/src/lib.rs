//! Tape 400: in the manner of the Mellotron M400.
//!
//! Under every key of the M400 is a strip of tape, about eight seconds long, with a recording
//! of a real instrument playing that note. Pressing the key pushes the tape against a playback
//! head and a pinch roller pulls it along; let go and a spring pulls it back to the start. So
//! each note starts with a soft, slightly flat swell as the tape gets up to speed, wavers with
//! that key's own wow and flutter, and stops dead after eight seconds when the tape runs out.
//! Each tape has three tracks side by side, and tape frames were swapped for other sets: here
//! the classic violins, flute and choir, and the cello, string section, brass, trombone, sax,
//! oboe, clarinet and the male, female and 8-voice choirs. The 35 keys run G2 to F5.
//!
//! The instruments on these tapes are synthesised, not sampled (vintage::tapes): small
//! ensembles of players with their own vibrato, their source waveforms and body or vowel
//! formants. Everything after that (the tape start, the wow, the dulled top, the hiss, the
//! eight-second limit, the pitch knob) is the Mellotron.

#![allow(clippy::excessive_precision)]
use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
#[allow(unused_imports)]
use vintage::dsp::*;
use vintage::nih_plug_egui::EguiState;
use vintage::tapes::{sounds, Sound, TapeVoice};
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

const VOICES: usize = 20;
/// Seconds of tape under each key.
pub const TAPE_SECONDS: f64 = 8.0;

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "station"]
    pub station: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "pitch"]
    pub pitch: FloatParam,
    #[id = "wow"]
    pub wow: FloatParam,
    #[id = "hiss"]
    pub hiss: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            station: FloatParam::new("Tape", 0.0, FloatRange::Linear { min: 0.0, max: (TAPES.len() - 1) as f32 }).with_step_size(1.0),
            tone: pct("Tone", 0.5),
            pitch: FloatParam::new("Pitch", 0.0, FloatRange::Linear { min: -100.0, max: 100.0 }).with_step_size(1.0).with_unit(" ct"),
            wow: pct("Tape Wear", 0.35),
            hiss: pct("Hiss", 0.3),
            volume: db("Volume", -6.0, -30.0, 6.0),
        }
    }
}

pub const STATION: usize = 0;
pub const TONE: usize = 1;
pub const PITCH: usize = 2;
pub const WOW: usize = 3;
pub const HISS: usize = 4;
pub const VOLUME: usize = 5;
pub const NPARAMS: usize = 6;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![self.station.as_ptr(), self.tone.as_ptr(), self.pitch.as_ptr(), self.wow.as_ptr(), self.hiss.as_ptr(), self.volume.as_ptr()]
    }
}

/// Default plain values in linked order (for tests).
#[allow(dead_code)]
pub fn defaults() -> Vec<f32> {
    let p = P::default();
    p.linked().iter().map(|q| unsafe { q.preview_plain(q.default_normalized_value()) }).collect()
}

// -------------------------------------------------------------------- dsp --

/// The tapes, in the order the TAPE list shows them (the first three are the M400's classic
/// frame, so older sessions keep their sound).
pub const TAPES: [Sound; 13] = [
    sounds::VIOLINS, sounds::FLUTE, sounds::CHOIR,
    sounds::CELLO, sounds::STRING_SECTION,
    sounds::BRASS, sounds::TROMBONE, sounds::SAX, sounds::OBOE, sounds::CLARINET,
    sounds::MALE_CHOIR, sounds::FEMALE_CHOIR, sounds::EIGHT_CHOIR,
];
const NAMES: &[&str] = &[
    "3 VIOLINS", "FLUTE", "MIXED CHOIR", "CELLO", "STRING SECTION", "3 TRUMPETS", "TROMBONE",
    "ALTO SAX", "OBOE", "CLARINET", "MALE CHOIR", "FEMALE CHOIR", "8 VOICE CHOIR",
];

#[derive(Clone, Copy)]
struct Voice {
    on: bool,
    held: bool,
    note: u8,
    env: f64,
    tape: TapeVoice,
    wow: Wander,
    flutter: f64,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    voices: [Voice; VOICES],
    noise: Noise,
    tape_lp: [Biquad; 2],
    tone_set: f32,
    hiss_bp: Biquad,
    dc: DcBlock,
    clock: u32,
}

impl Dsp {
    fn press(&mut self, note: u8) {
        let fs = self.fs;
        self.clock = self.clock.wrapping_add(1);
        let i = self.voices.iter().position(|v| v.on && v.note == note).or_else(|| self.voices.iter().position(|v| !v.on)).unwrap_or_else(|| self.voices.iter().enumerate().max_by_key(|(_, v)| v.tape.age).map(|(i, _)| i).unwrap_or(0));
        let mut n = note as i32;
        while n < 43 {
            n += 12;
        }
        while n > 77 {
            n -= 12;
        }
        let f = 440.0 * 2f64.powf((n as f64 - 69.0) / 12.0);
        let tape = self.v[STATION].round().clamp(0.0, (TAPES.len() - 1) as f32) as usize;
        // every key's tape is its own: its own wow, and the players start mid-vibrato
        let seed = (note as u32).wrapping_mul(2654435761).wrapping_add(self.clock);
        let vc = &mut self.voices[i];
        vc.on = true;
        vc.held = true;
        vc.note = note;
        vc.env = 0.0;
        vc.tape.start(&TAPES, tape, f, fs, seed);
        vc.wow = Wander::new(fs, 0.7, seed | 3);
        vc.flutter = Noise::new(seed | 5).next();
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let blank = Voice { on: false, held: false, note: 0, env: 0.0, tape: TapeVoice::default(), wow: Wander::new(fs, 0.7, 1), flutter: 0.0 };
        let mut hiss_bp = Biquad::identity();
        hiss_bp.set_highpass(fs, 1500.0, 0.6);
        Self { fs, v, voices: [blank; VOICES], noise: Noise::new(400), tape_lp: [Biquad::identity(); 2], tone_set: f32::NAN, hiss_bp, dc: DcBlock::new(fs), clock: 0 }
    }
    fn reset(&mut self) {
        self.voices.iter_mut().for_each(|v| v.on = false);
        self.tape_lp.iter_mut().for_each(|b| b.reset());
        self.hiss_bp.reset();
        self.dc.reset();
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn midi(&mut self, ev: Midi) {
        match ev {
            Midi::On { note, .. } => self.press(note), // the M400's keys aren't touch-sensitive
            Midi::Off { note } => self.voices.iter_mut().filter(|v| v.on && v.note == note).for_each(|v| v.held = false),
            Midi::AllOff => self.voices.iter_mut().for_each(|v| v.on = false),
            _ => {}
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        if self.tone_set != self.v[TONE] {
            self.tone_set = self.v[TONE];
            let fc = 2500.0 * 2f64.powf(2.2 * self.v[TONE] as f64);
            self.tape_lp[0].set_lowpass(fs, fc, 0.6);
            self.tape_lp[1].set_high_shelf(fs, 4000.0, 0.7, -3.0 + 3.0 * self.v[TONE] as f64);
        }
        let pitch = 2f64.powf(self.v[PITCH] as f64 / 1200.0);
        let wear = self.v[WOW] as f64;
        let hiss = self.v[HISS] as f64;
        let vol = db_to_gain(self.v[VOLUME] as f64);
        let tape_len = (TAPE_SECONDS * fs) as usize;
        let attack = 1.0 / (0.05 * fs);
        let rel = (-1.0 / (0.03 * fs)).exp();
        let mut peak = 0.0f64;
        for k in 0..n {
            let mut sum = 0.0;
            let mut tapes = 0.0;
            for vc in self.voices.iter_mut().filter(|v| v.on) {
                let t = vc.tape.age as f64 / fs;
                // the pinch roller gets the tape up to speed: it starts a little flat
                let start = 1.0 - 0.012 * (-t / 0.035).exp();
                let wow = 1.0 + wear * (0.0035 * vc.wow.next() + 0.0012 * (std::f64::consts::TAU * (6.5 * t + vc.flutter)).sin());
                // the tape envelope: the pad presses in, and lifts on release or when the tape ends
                let playing = vc.held && vc.tape.age < tape_len;
                if playing {
                    vc.env = (vc.env + attack).min(1.0);
                } else {
                    vc.env *= rel;
                }
                let x = vc.tape.next(&TAPES, pitch * start * wow, fs, &mut self.noise);
                sum += x * vc.env;
                tapes += vc.env;
                if !playing && vc.env < 1e-5 {
                    vc.on = false;
                }
            }
            // each tape on the heads brings its own hiss
            let h = self.hiss_bp.process(self.noise.next()) * hiss * 0.004 * tapes.sqrt();
            let y = self.dc.process(cascade(&mut self.tape_lp, sum * 0.4) + h) * vol;
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
        Preset { name: "Strings (Station A)", values: &[(STATION, 0.0), (TONE, 0.5), (WOW, 0.35)] },
        Preset { name: "Flute (Station B)", values: &[(STATION, 1.0), (TONE, 0.55), (WOW, 0.3)] },
        Preset { name: "Choir (Station C)", values: &[(STATION, 2.0), (TONE, 0.45), (WOW, 0.4)] },
        Preset { name: "Worn Strings", values: &[(STATION, 0.0), (TONE, 0.25), (WOW, 0.9), (HISS, 0.6)] },
        Preset { name: "Clean Flute", values: &[(STATION, 1.0), (TONE, 0.8), (WOW, 0.1), (HISS, 0.1)] },
        Preset { name: "Dark Choir, Flat", values: &[(STATION, 2.0), (TONE, 0.2), (PITCH, -40.0), (WOW, 0.6)] },
        Preset { name: "Cello", values: &[(STATION, 3.0), (TONE, 0.45), (WOW, 0.3)] },
        Preset { name: "String Section", values: &[(STATION, 4.0), (TONE, 0.55), (WOW, 0.35)] },
        Preset { name: "Brass", values: &[(STATION, 5.0), (TONE, 0.6), (WOW, 0.3)] },
        Preset { name: "Trombone", values: &[(STATION, 6.0), (TONE, 0.5), (WOW, 0.3)] },
        Preset { name: "Saxophone", values: &[(STATION, 7.0), (TONE, 0.55), (WOW, 0.3)] },
        Preset { name: "Oboe", values: &[(STATION, 8.0), (TONE, 0.55), (WOW, 0.3)] },
        Preset { name: "Clarinet", values: &[(STATION, 9.0), (TONE, 0.5), (WOW, 0.3)] },
        Preset { name: "Male Choir", values: &[(STATION, 10.0), (TONE, 0.45), (WOW, 0.4)] },
        Preset { name: "Female Choir", values: &[(STATION, 11.0), (TONE, 0.5), (WOW, 0.4)] },
        Preset { name: "8 Voice Choir", values: &[(STATION, 12.0), (TONE, 0.45), (WOW, 0.4)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "TAPE 400",
        subtitle: ("IN THE MANNER OF THE MELLOTRON M400", "35 KEYS · 8 SECONDS OF TAPE EACH · 13 TAPES"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::RadioList { p: STATION, x: 30.0, y: 84.0, w: 190.0, row_h: 24.0, names: NAMES, tip: "The tape the heads play. These instruments are synthesised, not recorded." },
            Item::Knob { p: TONE, x: 300.0, y: 165.0, r: 32.0, label: "TONE", tip: "The tone knob: from dull, played-out tape to fresh and bright.", active: None },
            Item::Knob { p: PITCH, x: 420.0, y: 165.0, r: 32.0, label: "PITCH", tip: "The capstan speed: up to a semitone sharp or flat.", active: None },
            Item::Knob { p: VOLUME, x: 560.0, y: 165.0, r: 30.0, label: "VOLUME", tip: "Output level.", active: None },
            Item::Knob { p: WOW, x: 300.0, y: 310.0, r: 24.0, label: "TAPE WEAR", tip: "Wow and flutter: how much each key's tape wavers. Every key wavers in its own way.", active: None },
            Item::Knob { p: HISS, x: 420.0, y: 310.0, r: 24.0, label: "HISS", tip: "Tape hiss: each tape on the heads adds its own.", active: None },
            Item::Text { x: 430.0, y: 390.0, text: "EACH NOTE STOPS AFTER 8 SECONDS", size: 10.0, align: Align2::CENTER_CENTER },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Tape 400", class_id: *b"PQKeysTape400Mel", vendor: "PEQUOD Keys",
    instrument, categories: [Instrument, Sampler]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        with(defaults(), &[(STATION, 1.0), (WOW, 0.0), (HISS, 0.0)])
    }

    include!("../../instrument_tests.rs");

    /// A held key stops when its eight seconds of tape run out.
    #[test]
    fn tape_runs_out() {
        let (l, _) = render(&with(defaults(), &[(HISS, 0.0)]), &[(0, Midi::On { note: 60, vel: 1.0 })], 9 * 48000);
        let during = rms(&l[7 * 48000..(7 * 48000 + 24000)]);
        let after = rms(&l[(8 * 48000 + 9600)..]);
        println!("at 7 s {during:.4}, after 8 s {after:.2e}");
        assert!(during > 0.01 && after < 1e-4);
    }
}
