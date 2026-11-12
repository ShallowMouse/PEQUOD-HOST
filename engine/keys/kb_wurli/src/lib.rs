//! Reed 200: an electric piano in the manner of the Wurlitzer 200A.
//!
//! Felt hammers strike flat steel reeds, each tuned with a blob of solder at its tip. The reeds
//! sit between the teeth of a comb-shaped plate held at a high DC voltage, so each reed is one
//! plate of a capacitor and its swing changes the capacitance. That pickup is the Wurlitzer's
//! sound: capacitance goes as 1/gap, so a reed swinging towards the plate makes far more
//! signal than one swinging away. Soft notes are sweet and nearly pure; dig in and the
//! lopsided pickup gives the reedy, buzzing bark. Shorter sustain than a Rhodes, a felt
//! damper on each reed, the 200A's built-in tremolo (fixed speed, depth on a knob) and its
//! pair of small speakers, or the line out.

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
    #[id = "bark"]
    pub bark: FloatParam,
    #[id = "hammer"]
    pub hammer: FloatParam,
    #[id = "sustain"]
    pub sustain: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "trem"]
    pub trem: FloatParam,
    #[id = "speaker"]
    pub speaker: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            bark: pct("Reed Gap", 0.55),
            hammer: pct("Hammer", 0.5),
            sustain: pct("Sustain", 0.5),
            tone: pct("Tone", 0.5),
            drive: pct("Amp Drive", 0.2),
            trem: pct("Tremolo", 0.0),
            speaker: FloatParam::new("Output", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            volume: db("Volume", -6.0, -30.0, 6.0),
        }
    }
}

pub const BARK: usize = 0;
pub const HAMMER: usize = 1;
pub const SUSTAIN: usize = 2;
pub const TONE: usize = 3;
pub const DRIVE: usize = 4;
pub const TREM: usize = 5;
pub const SPEAKER: usize = 6;
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
            self.bark.as_ptr(),
            self.hammer.as_ptr(),
            self.sustain.as_ptr(),
            self.tone.as_ptr(),
            self.drive.as_ptr(),
            self.trem.as_ptr(),
            self.speaker.as_ptr(),
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
    thump: f64,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    voices: [Voice; VOICES],
    pedal: bool,
    noise: Noise,
    thump_bp: Biquad,
    dc: DcBlock,
    tone: OnePole,
    speaker: [Biquad; 3],
    lfo: Lfo,
}

impl Dsp {
    fn strike(&mut self, note: u8, vel: f64) {
        let fs = self.fs;
        let i = self.voices.iter().position(|v| v.on && v.note == note).or_else(|| self.voices.iter().position(|v| !v.on)).unwrap_or_else(|| self.voices.iter().enumerate().max_by_key(|(_, v)| v.age).map(|(i, _)| i).unwrap_or(0));
        // the 200A's 64 keys run A1 to C7
        let mut n = note as i32;
        while n < 33 {
            n += 12;
        }
        while n > 96 {
            n -= 12;
        }
        let f = 440.0 * 2f64.powf((n as f64 - 69.0) / 12.0);
        let sus = self.v[SUSTAIN] as f64;
        let hammer = self.v[HAMMER] as f64;
        let t60 = (7.0 * (110.0 / f).powf(0.6)).clamp(0.6, 10.0) * (0.4 + 1.2 * sus);
        let k = |t: f64| 10f64.powf(-3.0 / (t * fs));
        let vc = &mut self.voices[i];
        let carry = if vc.on && vc.note == note { vc.ph } else { 0.0 };
        *vc = Voice::default();
        vc.on = true;
        vc.held = true;
        vc.note = note;
        vc.ph = carry;
        vc.inc = f / fs;
        // a loaded cantilever reed's second mode sits well above the 6th harmonic
        let f2 = 6.6 * f;
        vc.inc2 = if f2 < 0.45 * fs { f2 / fs } else { 0.0 };
        vc.amp = 0.06 + 0.94 * vel.powf(1.3);
        vc.k = k(t60);
        vc.over = vc.amp * (0.05 + 0.2 * hammer);
        vc.k_over = k(0.05);
        vc.thump = vel * (0.03 + 0.08 * hammer);
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut thump_bp = Biquad::identity();
        thump_bp.set_bandpass(fs, 600.0, 0.8);
        // the two little oval speakers: no deep bass, a forward midrange, rolled-off top
        let mut speaker = [Biquad::identity(); 3];
        speaker[0].set_highpass(fs, 140.0, 0.8);
        speaker[1].set_peak(fs, 1400.0, 0.9, 3.5);
        speaker[2].set_lowpass(fs, 5500.0, 0.7);
        Self { fs, v, voices: [Voice::default(); VOICES], pedal: false, noise: Noise::new(200), thump_bp, dc: DcBlock::new(fs), tone: OnePole::default(), speaker, lfo: Lfo::default() }
    }
    fn reset(&mut self) {
        self.voices = [Voice::default(); VOICES];
        self.thump_bp.reset();
        self.dc.reset();
        self.tone.reset();
        self.speaker.iter_mut().for_each(|b| b.reset());
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
        // the pickup: signal ~ 1/(gap - x), normalised to unity for small swings.
        // `kp` is the reed's full swing as a fraction of its gap to the plate.
        let kp = 0.25 + 0.6 * self.v[BARK] as f64;
        let damper = 10f64.powf(-3.0 / (0.09 * fs));
        let drive = self.v[DRIVE] as f64;
        let pre = 1.0 + 6.0 * drive * drive;
        let vol = db_to_gain(self.v[VOLUME] as f64);
        let trem = self.v[TREM] as f64;
        let spk = self.v[SPEAKER] >= 0.5;
        self.tone.set_lowpass(fs, 1500.0 * 2f64.powf(4.0 * self.v[TONE] as f64));
        let pedal = self.pedal;
        let mut peak = 0.0f64;
        for k in 0..n {
            let mut sum = 0.0;
            let mut thump = 0.0;
            for vc in self.voices.iter_mut().filter(|v| v.on) {
                let x = vc.amp * (TAU * vc.ph).sin() + vc.over * (TAU * vc.ph2).sin();
                let x = (x * kp).clamp(-0.95, 0.95);
                sum += x / (1.0 - x) / kp;
                thump += vc.thump;
                vc.ph = (vc.ph + vc.inc).fract();
                vc.ph2 = (vc.ph2 + vc.inc2).fract();
                vc.amp *= vc.k * if vc.held || pedal { 1.0 } else { damper };
                vc.over *= vc.k_over;
                vc.thump *= 0.993;
                vc.age += 1;
                if vc.age % 64 == 0 && vc.amp < 1e-5 {
                    vc.on = false;
                }
            }
            let y = self.dc.process(sum * 0.25 + self.thump_bp.process(self.noise.next() * thump));
            let y = self.tone.lp(y);
            // the 200A's tremolo: fixed at about 5.6 Hz, depth on the knob
            let y = y * (1.0 - trem * 0.5 * (1.0 + self.lfo.next(fs, 5.6)));
            let y = tube(y * pre, 0.15) / pre.powf(0.5);
            let y = if spk { cascade(&mut self.speaker, y) * 1.2 } else { y };
            let y = y * vol;
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
        Preset { name: "200A Speakers", values: &[(SPEAKER, 1.0), (TREM, 0.0)] },
        Preset { name: "200A Line Out", values: &[(SPEAKER, 0.0), (TONE, 0.6)] },
        Preset { name: "Tremolo Ballad", values: &[(TREM, 0.55), (BARK, 0.4), (HAMMER, 0.35)] },
        Preset { name: "Buzzy Rock Wurli", values: &[(BARK, 0.95), (DRIVE, 0.75), (HAMMER, 0.8), (TONE, 0.65)] },
        Preset { name: "Soft and Sweet", values: &[(BARK, 0.25), (HAMMER, 0.2), (TONE, 0.35), (TREM, 0.25)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "REED 200",
        subtitle: ("IN THE MANNER OF THE WURLITZER 200A", "STEEL REEDS · ELECTROSTATIC PICKUP · TREMOLO"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::Knob { p: BARK, x: 90.0, y: 160.0, r: 36.0, label: "REED GAP", tip: "How close the reeds swing to the pickup plate. Closer gives more of the buzzing bark when you play hard.", active: None },
            Item::Knob { p: HAMMER, x: 210.0, y: 160.0, r: 36.0, label: "HAMMER", tip: "Felt hardness: harder gives more attack and clank.", active: None },
            Item::Knob { p: SUSTAIN, x: 330.0, y: 160.0, r: 36.0, label: "SUSTAIN", tip: "How long the reeds ring. The sustain pedal (CC64) lifts the dampers.", active: None },
            Item::Knob { p: DRIVE, x: 450.0, y: 160.0, r: 36.0, label: "AMP", tip: "How hard the built-in amp is driven.", active: None },
            Item::Knob { p: VOLUME, x: 565.0, y: 160.0, r: 30.0, label: "VOLUME", tip: "Output level.", active: None },
            Item::Divider { y: 264.0 },
            Item::Knob { p: TONE, x: 120.0, y: 330.0, r: 22.0, label: "TONE", tip: "Darker or brighter.", active: None },
            Item::Knob { p: TREM, x: 260.0, y: 330.0, r: 22.0, label: "VIBRATO", tip: "The 200A's tremolo (labelled vibrato on the real one): fixed speed, this sets the depth.", active: None },
            Item::Switch { p: SPEAKER, x: 440.0, y: 330.0, labels: ["LINE OUT", "SPEAKERS"], tip: "Through the 200A's two small speakers, or straight from its line out." },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Reed 200", class_id: *b"PQKeysReed200Wur", vendor: "PEQUOD Keys",
    instrument, categories: [Instrument, Piano]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        with(defaults(), &[(TREM, 0.0), (DRIVE, 0.0), (HAMMER, 0.0), (SPEAKER, 0.0)])
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

    /// The electrostatic pickup is lopsided: hard notes get a strong 2nd harmonic.
    #[test]
    fn velocity_barks() {
        let p = tune_params();
        let h2 = |vel: f32| {
            let (l, _) = render(&p, &[(0, Midi::On { note: 57, vel })], 24000);
            let x = &l[2400..12000];
            goertzel(x, 440.0) / goertzel(x, 220.0)
        };
        let (soft, hard) = (h2(0.2), h2(1.0));
        println!("2nd harmonic soft {soft:.4} hard {hard:.4}");
        assert!(hard > 3.0 * soft);
    }
}
