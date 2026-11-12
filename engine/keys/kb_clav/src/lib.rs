//! Clavi D6: in the manner of the Hohner Clavinet D6.
//!
//! Each key has a rubber tip (the tangent) under a steel string. Pressing the key hammers the
//! string against a brass anvil: the strike sets the string ringing, and the anvil fixes its
//! speaking length for as long as the key is held. Let go and the string lifts off the anvil
//! into a band of yarn, which kills it with a little snap. So the D6 is a struck string that
//! sustains under your finger and stops when you release: funky, percussive and bright.
//!
//! Two pickups under the strings, one near the middle (darker) and one near the end (thinner),
//! switched as on the D6: either one, both, or both out of phase (the hollow, nasal one). The
//! four filter tabs (Brilliant, Treble, Medium, Soft) can be combined. The mute slider presses
//! the strings' felt against them for the short, plucky sound.

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
pub const H: u32 = 450;

const VOICES: usize = 12;

fn stepped(name: &str, default: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max }).with_step_size(1.0)
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "pickup"]
    pub pickup: FloatParam,
    #[id = "brilliant"]
    pub brilliant: FloatParam,
    #[id = "treble"]
    pub treble: FloatParam,
    #[id = "medium"]
    pub medium: FloatParam,
    #[id = "soft"]
    pub soft: FloatParam,
    #[id = "mute"]
    pub mute: FloatParam,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            pickup: stepped("Pickups", 2.0, 3.0),
            brilliant: stepped("Brilliant", 0.0, 1.0),
            treble: stepped("Treble", 1.0, 1.0),
            medium: stepped("Medium", 0.0, 1.0),
            soft: stepped("Soft", 0.0, 1.0),
            mute: pct("Mute", 0.0),
            drive: pct("Drive", 0.15),
            volume: db("Volume", -6.0, -30.0, 6.0),
        }
    }
}

pub const PICKUP: usize = 0;
pub const BRILLIANT: usize = 1;
pub const TREBLE: usize = 2;
pub const MEDIUM: usize = 3;
pub const SOFT: usize = 4;
pub const MUTE: usize = 5;
pub const DRIVE: usize = 6;
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
            self.pickup.as_ptr(),
            self.brilliant.as_ptr(),
            self.treble.as_ptr(),
            self.medium.as_ptr(),
            self.soft.as_ptr(),
            self.mute.as_ptr(),
            self.drive.as_ptr(),
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

/// Where the pickups sit along the speaking length (fractions from the anvil end).
const NECK: f64 = 0.19;
const BRIDGE: f64 = 0.075;

#[derive(Clone)]
struct Voice {
    on: bool,
    held: bool,
    note: u8,
    age: usize,
    s: DelayLine,
    out: DelayLine,
    len: f64,
    period: f64,
    prev: f64,
    damp: f64,
    loss: f64,
    release: f64,
    energy: f64,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    voices: Vec<Voice>,
    noise: Noise,
    tabs: [Biquad; 4],
    tabs_set: [bool; 4],
    body: Biquad,
    dc: DcBlock,
}

impl Dsp {
    fn strike(&mut self, note: u8, vel: f64) {
        let fs = self.fs;
        let i = self.voices.iter().position(|v| v.on && v.note == note).or_else(|| self.voices.iter().position(|v| !v.on)).unwrap_or_else(|| self.voices.iter().enumerate().max_by_key(|(_, v)| v.age).map(|(i, _)| i).unwrap_or(0));
        // 60 keys, F1 to E6
        let mut n = note as i32;
        while n < 29 {
            n += 12;
        }
        while n > 88 {
            n -= 12;
        }
        let f = 440.0 * 2f64.powf((n as f64 - 69.0) / 12.0);
        let mute = self.v[MUTE] as f64;
        let t60 = (9.0 * (110.0 / f).powf(0.5)).clamp(1.2, 14.0) * (1.0 - 0.9 * mute);
        // loop filter weight: the string loses its top faster when muted or struck softly
        let damp = (0.06 + 0.3 * mute + 0.12 * (1.0 - vel)).min(0.48);
        let vc = &mut self.voices[i];
        vc.on = true;
        vc.held = true;
        vc.note = note;
        vc.age = 0;
        vc.damp = damp;
        vc.loss = 10f64.powf(-3.0 / (t60 * f));
        // the loop filter runs once per period, so the yarn's 35 ms is counted in periods too
        vc.release = 10f64.powf(-3.0 / (0.035 * f));
        vc.period = fs / f;
        vc.len = (fs / f - 1.0 - damp).min(vc.s.max_delay() - 2.0);
        vc.prev = 0.0;
        vc.energy = 1.0;
        vc.s.reset();
        vc.out.reset();
        // the tangent strikes close to the anvil: a sharp, lopsided kink, harder = brighter
        let len = vc.len as usize + 2;
        let peak = 0.08;
        let amp = 0.15 + 0.85 * vel;
        let mut lp = 0.0;
        let a = 0.35 + 0.6 * vel;
        for k in 0..len {
            let pos = k as f64 / len as f64;
            let shape = if pos < peak { pos / peak } else { (1.0 - pos) / (1.0 - peak) };
            lp += a * (shape - lp);
            vc.s.write((lp + 0.03 * self.noise.next()) * amp);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let max = (fs / 40.0) as usize + 8;
        let mk = || Voice { on: false, held: false, note: 0, age: 0, s: DelayLine::new(max), out: DelayLine::new(max), len: 100.0, period: 100.0, prev: 0.0, damp: 0.1, loss: 0.99, release: 0.9, energy: 0.0 };
        let mut body = Biquad::identity();
        body.set_highpass(fs, 70.0, 0.7);
        Self { fs, v, voices: (0..VOICES).map(|_| mk()).collect(), noise: Noise::new(6), tabs: [Biquad::identity(); 4], tabs_set: [false; 4], body, dc: DcBlock::new(fs) }
    }
    fn reset(&mut self) {
        for vc in &mut self.voices {
            vc.on = false;
            vc.s.reset();
            vc.out.reset();
        }
        self.tabs.iter_mut().for_each(|b| b.reset());
        self.body.reset();
        self.dc.reset();
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn midi(&mut self, ev: Midi) {
        match ev {
            Midi::On { note, vel } => self.strike(note, vel as f64),
            Midi::Off { note } => self.voices.iter_mut().filter(|v| v.on && v.note == note && v.held).for_each(|v| v.held = false),
            Midi::AllOff => self.voices.iter_mut().for_each(|v| v.on = false),
            _ => {}
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        // the filter tabs, each a rocker that's in or out
        let tabs = [self.v[BRILLIANT] >= 0.5, self.v[TREBLE] >= 0.5, self.v[MEDIUM] >= 0.5, self.v[SOFT] >= 0.5];
        if tabs != self.tabs_set {
            self.tabs_set = tabs;
            let mut b = [Biquad::identity(); 4];
            if tabs[0] {
                b[0].set_highpass(fs, 900.0, 0.6);
            }
            if tabs[1] {
                b[1].set_highpass(fs, 350.0, 0.6);
            }
            if tabs[2] {
                b[2].set_peak(fs, 900.0, 0.7, 6.0);
            }
            if tabs[3] {
                b[3].set_lowpass(fs, 1100.0, 0.7);
            }
            self.tabs = b;
        }
        let (neck, bridge, phase) = match self.v[PICKUP].round() as i32 {
            0 => (1.0, 0.0, 1.0),
            1 => (0.0, 1.0, 1.0),
            2 => (1.0, 1.0, 1.0),
            _ => (1.0, 1.0, -1.0),
        };
        let drive = self.v[DRIVE] as f64;
        let pre = 1.0 + 8.0 * drive * drive;
        let vol = db_to_gain(self.v[VOLUME] as f64);
        let mut peak = 0.0f64;
        for k in 0..n {
            let mut sum = 0.0;
            for vc in self.voices.iter_mut().filter(|v| v.on) {
                let y = vc.s.read(vc.len);
                let f = (1.0 - vc.damp) * y + vc.damp * vc.prev;
                vc.prev = y;
                // let go: the string lifts off the anvil into the yarn and dies
                let loss = if vc.held { vc.loss } else { vc.loss * vc.release };
                vc.s.write(f * loss);
                vc.out.write(y);
                // a pickup at fraction p of the length hears harmonic n as sin(n pi p): a comb
                let pn = y - vc.out.read((NECK * vc.period).max(1.0));
                let pb = y - vc.out.read((BRIDGE * vc.period).max(1.0));
                sum += neck * pn * 1.4 + phase * bridge * pb * 2.6;
                vc.energy = 0.999 * vc.energy + 0.001 * y.abs();
                vc.age += 1;
                if vc.age > 2400 && vc.energy < 1e-6 {
                    vc.on = false;
                }
            }
            let y = self.dc.process(self.body.process(sum * 0.35));
            let y = cascade(&mut self.tabs, y);
            let y = tube(y * pre, 0.1) / pre.powf(0.55) * vol;
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
        Preset { name: "Funk Rhythm (Bridge, Treble)", values: &[(PICKUP, 1.0), (TREBLE, 1.0), (BRILLIANT, 0.0), (MEDIUM, 0.0), (SOFT, 0.0), (MUTE, 0.0)] },
        Preset { name: "Both, Out of Phase", values: &[(PICKUP, 3.0), (TREBLE, 1.0), (MEDIUM, 0.0), (SOFT, 0.0)] },
        Preset { name: "Neck, Soft", values: &[(PICKUP, 0.0), (TREBLE, 0.0), (SOFT, 1.0), (MEDIUM, 0.0)] },
        Preset { name: "Muted Pluck", values: &[(PICKUP, 2.0), (MUTE, 0.7), (TREBLE, 1.0)] },
        Preset { name: "Brilliant Spank", values: &[(PICKUP, 1.0), (BRILLIANT, 1.0), (TREBLE, 1.0), (DRIVE, 0.4)] },
        Preset { name: "Medium Driven", values: &[(PICKUP, 2.0), (MEDIUM, 1.0), (TREBLE, 0.0), (DRIVE, 0.75)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "CLAVI D6",
        subtitle: ("IN THE MANNER OF THE HOHNER CLAVINET D6", "STRUCK STRINGS · ANVILS · TWO PICKUPS · FILTER TABS"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::Text { x: 360.0, y: 100.0, text: "FILTERS", size: 11.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: BRILLIANT, x: 450.0, y: 135.0, labels: ["", "BRILLIANT"], tip: "Brilliant tab: cuts deep into the lows for the thinnest, sharpest sound." },
            Item::Switch { p: TREBLE, x: 270.0, y: 135.0, labels: ["", "TREBLE"], tip: "Treble tab: thins out the lows." },
            Item::Switch { p: MEDIUM, x: 270.0, y: 185.0, labels: ["", "MEDIUM"], tip: "Medium tab: a nasal midrange lift." },
            Item::Switch { p: SOFT, x: 450.0, y: 185.0, labels: ["", "SOFT"], tip: "Soft tab: rolls off the top." },
            Item::Selector { p: PICKUP, x: 90.0, y: 160.0, label: "PICKUPS", positions: &["NECK", "BRIDGE", "BOTH", "OUT OF PHASE"], tip: "The D6's pickup switches: the middle pickup (fuller), the end pickup (thinner), both, or both out of phase (hollow and nasal)." },
            Item::Knob { p: VOLUME, x: 580.0, y: 160.0, r: 28.0, label: "VOLUME", tip: "Output level.", active: None },
            Item::Divider { y: 264.0 },
            Item::Knob { p: MUTE, x: 180.0, y: 345.0, r: 30.0, label: "MUTE", tip: "The mute slider: presses felt against the strings for a short, plucked sound.", active: None },
            Item::Knob { p: DRIVE, x: 420.0, y: 345.0, r: 30.0, label: "DRIVE", tip: "How hard the amp is pushed. Clavinets were usually played through a guitar amp.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Clavi D6", class_id: *b"PQKeysClaviD6Hoh", vendor: "PEQUOD Keys",
    instrument, categories: [Instrument, Piano]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        with(defaults(), &[(PICKUP, 0.0), (TREBLE, 0.0), (DRIVE, 0.0)])
    }

    include!("../../instrument_tests.rs");

    /// Held notes sustain; released ones stop within a fraction of a second.
    #[test]
    fn sustains_under_the_finger() {
        let p = tune_params();
        let (held, _) = render(&p, &[(0, Midi::On { note: 52, vel: 0.8 })], 48000);
        let (rel, _) = render(&p, &[(0, Midi::On { note: 52, vel: 0.8 }), (12000, Midi::Off { note: 52 })], 48000);
        println!("held {:.4} released {:.6}", rms(&held[36000..]), rms(&rel[36000..]));
        assert!(rms(&held[36000..]) > 0.1 * rms(&held[2000..6000]));
        assert!(rms(&rel[36000..]) < 1e-3 * rms(&held[2000..6000]));
    }
}
