//! Tape 400: in the manner of the Mellotron M400.
//!
//! Under every key of the M400 is a strip of tape, about eight seconds long, with a recording
//! of a real instrument playing that note. Pressing the key pushes the tape against a playback
//! head and a pinch roller pulls it along; let go and a spring pulls it back to the start. So
//! each note starts with a soft, slightly flat swell as the tape gets up to speed, wavers with
//! that key's own wow and flutter, and stops dead after eight seconds when the tape runs out.
//! Each tape has three tracks side by side (the station selector moves the heads across): the
//! classic set of strings, flute and choir. The 35 keys run G2 to F5.
//!
//! The instruments on these tapes are synthesised, not sampled: an ensemble of bowed strings,
//! a breathy flute and an "aah" choir, each with its players' own vibrato. Everything after
//! that (the tape start, the wow, the dulled top, the hiss, the eight-second limit, the pitch
//! knob) is the Mellotron.

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
pub const H: u32 = 400;

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
            station: FloatParam::new("Station", 0.0, FloatRange::Linear { min: 0.0, max: 2.0 }).with_step_size(1.0),
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
fn saw(ph: f64, dt: f64) -> f64 {
    2.0 * ph - 1.0 - blep(ph, dt)
}

/// The players on each track: detune (cents), vibrato rate (Hz), vibrato depth (cents).
const STRINGS: [(f64, f64, f64); 3] = [(-6.0, 5.3, 11.0), (1.0, 5.8, 9.0), (5.0, 6.2, 13.0)];
const CHOIR: [(f64, f64, f64); 4] = [(-9.0, 5.1, 14.0), (-3.0, 5.6, 12.0), (4.0, 5.9, 16.0), (10.0, 5.4, 13.0)];

#[derive(Clone, Copy)]
struct Voice {
    on: bool,
    held: bool,
    note: u8,
    age: usize,
    f: f64,
    ph: [f64; 4],
    vph: [f64; 4],
    env: f64,
    form: [Biquad; 3],
    lp: OnePole,
    breath: Biquad,
    wow: Wander,
    flutter: f64,
    station: usize,
    #[allow(dead_code)]
    seed: u32,
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
        let i = self.voices.iter().position(|v| v.on && v.note == note).or_else(|| self.voices.iter().position(|v| !v.on)).unwrap_or_else(|| self.voices.iter().enumerate().max_by_key(|(_, v)| v.age).map(|(i, _)| i).unwrap_or(0));
        let mut n = note as i32;
        while n < 43 {
            n += 12;
        }
        while n > 77 {
            n -= 12;
        }
        let f = 440.0 * 2f64.powf((n as f64 - 69.0) / 12.0);
        let station = self.v[STATION].round().clamp(0.0, 2.0) as usize;
        let mut form = [Biquad::identity(); 3];
        let mut lp = OnePole::default();
        let mut breath = Biquad::identity();
        match station {
            0 => {
                // violin bodies: the main air and wood resonances and the bright "bridge hill"
                form[0].set_peak(fs, 480.0, 1.2, 5.0);
                form[1].set_peak(fs, 1300.0, 1.4, 3.0);
                form[2].set_peak(fs, 2900.0, 1.6, 4.0);
                lp.set_lowpass(fs, 5000.0);
            }
            1 => {
                breath.set_bandpass(fs, (2.0 * f).min(0.4 * fs), 3.0);
                lp.set_lowpass(fs, 6000.0);
            }
            _ => {
                // "aah": the open vowel's formants
                form[0].set_peak(fs, 720.0, 4.0, 12.0);
                form[1].set_peak(fs, 1150.0, 5.0, 9.0);
                form[2].set_peak(fs, 2600.0, 6.0, 7.0);
                lp.set_lowpass(fs, 3500.0);
            }
        }
        // every key's tape is its own: its own wow, and the players start mid-vibrato
        let seed = (note as u32).wrapping_mul(2654435761).wrapping_add(self.clock);
        let mut nz = Noise::new(seed | 1);
        let vc = &mut self.voices[i];
        *vc = Voice {
            on: true,
            held: true,
            note,
            age: 0,
            f,
            ph: [0.5 + 0.5 * nz.next(), 0.5 + 0.5 * nz.next(), 0.5 + 0.5 * nz.next(), 0.5 + 0.5 * nz.next()],
            vph: [0.5 + 0.5 * nz.next(), 0.5 + 0.5 * nz.next(), 0.5 + 0.5 * nz.next(), 0.5 + 0.5 * nz.next()],
            env: 0.0,
            form,
            lp,
            breath,
            wow: Wander::new(fs, 0.7, seed | 3),
            flutter: nz.next(),
            station,
            seed,
        };
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let blank = Voice { on: false, held: false, note: 0, age: 0, f: 440.0, ph: [0.0; 4], vph: [0.0; 4], env: 0.0, form: [Biquad::identity(); 3], lp: OnePole::default(), breath: Biquad::identity(), wow: Wander::new(fs, 0.7, 1), flutter: 0.0, station: 0, seed: 0 };
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
                let t = vc.age as f64 / fs;
                // the pinch roller gets the tape up to speed: it starts a little flat
                let start = 1.0 - 0.012 * (-t / 0.035).exp();
                let wow = 1.0 + wear * (0.0035 * vc.wow.next() + 0.0012 * (TAU * (6.5 * t + vc.flutter)).sin());
                let speed = pitch * start * wow;
                let f = vc.f * speed;
                // the tape envelope: the pad presses in, and lifts on release or when the tape ends
                let playing = vc.held && vc.age < tape_len;
                if playing {
                    vc.env = (vc.env + attack).min(1.0);
                } else {
                    vc.env *= rel;
                }
                let x = match vc.station {
                    0 => {
                        let mut s = 0.0;
                        let bow = (t / 0.12).min(1.0);
                        for (j, (det, vr, vd)) in STRINGS.iter().enumerate() {
                            let vib = (vd / 1200.0) * (TAU * vc.vph[j]).sin() * (t / 0.4).min(1.0);
                            let dt = f * 2f64.powf(det / 1200.0 + vib) / fs;
                            s += saw(vc.ph[j], dt);
                            vc.ph[j] = (vc.ph[j] + dt).fract();
                            vc.vph[j] = (vc.vph[j] + vr / fs).fract();
                        }
                        cascade(&mut vc.form, vc.lp.lp(s * 0.3 * bow))
                    }
                    1 => {
                        let vib = (8.0 / 1200.0) * (TAU * vc.vph[0]).sin() * ((t - 0.5) / 0.4).clamp(0.0, 1.0);
                        let dt = f * 2f64.powf(vib) / fs;
                        let p = TAU * vc.ph[0];
                        vc.ph[0] = (vc.ph[0] + dt).fract();
                        vc.vph[0] = (vc.vph[0] + 4.8 / fs).fract();
                        let tone = p.sin() + 0.22 * (2.0 * p).sin() + 0.07 * (3.0 * p).sin();
                        // breath on the tone, plus the chiff as the note speaks
                        let chiff = 1.0 + 6.0 * (-t / 0.03).exp();
                        let air = vc.breath.process(self.noise.next()) * 0.25 * chiff;
                        vc.lp.lp(0.55 * tone + air) * (t / 0.06).min(1.0)
                    }
                    _ => {
                        let mut s = 0.0;
                        for (j, (det, vr, vd)) in CHOIR.iter().enumerate() {
                            let vib = (vd / 1200.0) * (TAU * vc.vph[j]).sin();
                            let dt = f * 2f64.powf(det / 1200.0 + vib) / fs;
                            s += saw(vc.ph[j], dt);
                            vc.ph[j] = (vc.ph[j] + dt).fract();
                            vc.vph[j] = (vc.vph[j] + vr / fs).fract();
                        }
                        let breath = 0.15 * self.noise.next();
                        cascade(&mut vc.form, vc.lp.lp((s * 0.2 + breath * 0.3) * (t / 0.15).min(1.0)))
                    }
                };
                sum += x * vc.env;
                tapes += vc.env;
                vc.age += 1;
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
    ]
}

fn panel() -> Panel {
    Panel {
        title: "TAPE 400",
        subtitle: ("IN THE MANNER OF THE MELLOTRON M400", "35 KEYS · 8 SECONDS OF TAPE EACH · STRINGS, FLUTE, CHOIR"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::Selector { p: STATION, x: 100.0, y: 165.0, label: "STATION", positions: &["A  STRINGS", "B  FLUTE", "C  CHOIR"], tip: "Which of the tape's three tracks the heads play. These instruments are synthesised, not recorded." },
            Item::Knob { p: TONE, x: 250.0, y: 165.0, r: 34.0, label: "TONE", tip: "The tone knob: from dull, played-out tape to fresh and bright.", active: None },
            Item::Knob { p: PITCH, x: 370.0, y: 165.0, r: 34.0, label: "PITCH", tip: "The capstan speed: up to a semitone sharp or flat.", active: None },
            Item::Knob { p: VOLUME, x: 560.0, y: 165.0, r: 30.0, label: "VOLUME", tip: "Output level.", active: None },
            Item::Divider { y: 262.0 },
            Item::Knob { p: WOW, x: 200.0, y: 325.0, r: 24.0, label: "TAPE WEAR", tip: "Wow and flutter: how much each key's tape wavers. Every key wavers in its own way.", active: None },
            Item::Knob { p: HISS, x: 330.0, y: 325.0, r: 24.0, label: "HISS", tip: "Tape hiss: each tape on the heads adds its own.", active: None },
            Item::Text { x: 500.0, y: 325.0, text: "EACH NOTE STOPS AFTER 8 SECONDS", size: 10.0, align: Align2::CENTER_CENTER },
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
