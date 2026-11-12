//! B-3 Organ: a tonewheel organ in the manner of the Hammond B-3, through a Leslie 122.
//!
//! 91 free-running tonewheels turned by one synchronous motor, so every wheel runs all the
//! time and a key only connects them: the frequencies come from the B-3's real gear ratios
//! (twelve gear pairs, with 2 to 128 teeth per octave), so the tuning is very slightly off
//! equal temperament, as the real one is. Nine drawbars at their real footages, each step
//! about 3 dB, folding back an octave at the top of the wheels. Percussion as on the B-3:
//! single-triggered, 2nd or 3rd harmonic, fast or slow, normal or soft, it takes the 1'
//! drawbar's busbar (so the 1' goes silent while it's on) and the normal setting drops the
//! drawbars a little. Key click from the busbar contacts. The scanner vibrato and chorus
//! (V1-V3, C1-C3). Then the Leslie's tube amp and its rotating horn and drum.

#![allow(clippy::excessive_precision)]
use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
#[allow(unused_imports)]
use vintage::dsp::*;
use vintage::fx::Leslie;
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
pub const H: u32 = 540;

const WHEELS: usize = 91;

fn stepped(name: &str, default: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max }).with_step_size(1.0)
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "db16"]
    pub db16: FloatParam,
    #[id = "db513"]
    pub db513: FloatParam,
    #[id = "db8"]
    pub db8: FloatParam,
    #[id = "db4"]
    pub db4: FloatParam,
    #[id = "db223"]
    pub db223: FloatParam,
    #[id = "db2"]
    pub db2: FloatParam,
    #[id = "db135"]
    pub db135: FloatParam,
    #[id = "db113"]
    pub db113: FloatParam,
    #[id = "db1"]
    pub db1: FloatParam,
    #[id = "perc"]
    pub perc: FloatParam,
    #[id = "percharm"]
    pub perc_harm: FloatParam,
    #[id = "percdecay"]
    pub perc_decay: FloatParam,
    #[id = "percsoft"]
    pub perc_soft: FloatParam,
    #[id = "vib"]
    pub vib: FloatParam,
    #[id = "click"]
    pub click: FloatParam,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "leslie"]
    pub leslie: FloatParam,
    #[id = "speed"]
    pub speed: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            db16: stepped("16'", 8.0, 8.0),
            db513: stepped("5 1/3'", 8.0, 8.0),
            db8: stepped("8'", 8.0, 8.0),
            db4: stepped("4'", 0.0, 8.0),
            db223: stepped("2 2/3'", 0.0, 8.0),
            db2: stepped("2'", 0.0, 8.0),
            db135: stepped("1 3/5'", 0.0, 8.0),
            db113: stepped("1 1/3'", 0.0, 8.0),
            db1: stepped("1'", 0.0, 8.0),
            perc: stepped("Percussion", 1.0, 1.0),
            perc_harm: stepped("Percussion Harmonic", 1.0, 1.0),
            perc_decay: stepped("Percussion Decay", 0.0, 1.0),
            perc_soft: stepped("Percussion Volume", 0.0, 1.0),
            vib: stepped("Vibrato/Chorus", 6.0, 6.0),
            click: pct("Key Click", 0.5),
            drive: pct("Drive", 0.25),
            leslie: stepped("Leslie", 1.0, 1.0),
            speed: stepped("Leslie Speed", 0.0, 1.0),
            volume: db("Volume", -6.0, -30.0, 6.0),
        }
    }
}

pub const DB0: usize = 0; // 16' .. 1' are DB0 .. DB0 + 8
pub const PERC: usize = 9;
pub const PERC_HARM: usize = 10;
pub const PERC_DECAY: usize = 11;
pub const PERC_SOFT: usize = 12;
pub const VIB: usize = 13;
pub const CLICK: usize = 14;
pub const DRIVE: usize = 15;
pub const LESLIE: usize = 16;
pub const SPEED: usize = 17;
pub const VOLUME: usize = 18;
pub const NPARAMS: usize = 19;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.db16.as_ptr(),
            self.db513.as_ptr(),
            self.db8.as_ptr(),
            self.db4.as_ptr(),
            self.db223.as_ptr(),
            self.db2.as_ptr(),
            self.db135.as_ptr(),
            self.db113.as_ptr(),
            self.db1.as_ptr(),
            self.perc.as_ptr(),
            self.perc_harm.as_ptr(),
            self.perc_decay.as_ptr(),
            self.perc_soft.as_ptr(),
            self.vib.as_ptr(),
            self.click.as_ptr(),
            self.drive.as_ptr(),
            self.leslie.as_ptr(),
            self.speed.as_ptr(),
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

/// Semitone offsets of the nine drawbars from the 8' (the key's own pitch).
const FOOTAGE: [i32; 9] = [-12, 7, 0, 12, 19, 24, 28, 31, 36];

/// The B-3's twelve gear pairs (driven/driving teeth), C to B: the motor shaft turns at
/// 20 rev/s and each tonewheel has 2, 4 ... 128 teeth per octave.
const GEARS: [(f64, f64); 12] = [
    (85.0, 104.0),
    (71.0, 82.0),
    (67.0, 73.0),
    (105.0, 108.0),
    (103.0, 100.0),
    (84.0, 77.0),
    (74.0, 64.0),
    (98.0, 80.0),
    (96.0, 74.0),
    (88.0, 64.0),
    (67.0, 46.0),
    (108.0, 70.0),
];

/// Frequency of tonewheel `w` (1..=91). Wheel 1 is the low C (32.7 Hz); the top seven
/// wheels (85-91) are an octave above wheels 73-79.
pub fn wheel_hz(w: usize) -> f64 {
    let w = w.clamp(1, WHEELS);
    if w > 84 {
        return 2.0 * wheel_hz(w - 12);
    }
    let (octave, note) = ((w - 1) / 12, (w - 1) % 12);
    let (a, b) = GEARS[note];
    20.0 * a / b * (2u32 << octave) as f64
}

/// Drawbar position 0..8 to gain: about 3 dB a step, 0 = off.
fn drawbar_gain(pos: f32) -> f64 {
    let p = pos.round();
    if p <= 0.0 { 0.0 } else { db_to_gain(-3.0 * (8.0 - p as f64)) }
}

/// The wheel a drawbar takes for a key (MIDI note), folding back at the top.
fn wheel_for(note: u8, bar: usize) -> usize {
    // the lowest manual key, C (MIDI 36), takes wheel 13 at 8'; notes off the 61-key manual fold in
    let mut n = note as i32;
    while n < 36 {
        n += 12;
    }
    while n > 96 {
        n -= 12;
    }
    let mut w = n - 36 + 13 + FOOTAGE[bar];
    while w > WHEELS as i32 {
        w -= 12;
    }
    while w < 1 {
        w += 12;
    }
    w as usize
}

const TABLE: usize = 4096;

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    table: Vec<f64>,
    inc: [f64; WHEELS],
    phase: [f64; WHEELS],
    gain: [f64; WHEELS],
    target: [f64; WHEELS],
    perc_target: [f64; WHEELS],
    perc_gain: [f64; WHEELS],
    held: [bool; 128],
    nheld: usize,
    dirty: bool,
    perc_env: f64,
    click_env: f64,
    noise: Noise,
    click_bp: Biquad,
    tone: OnePole,
    scanner: DelayLine,
    scan_lfo: Lfo,
    scan_lp: OnePole,
    amp_hp: Biquad,
    leslie: Leslie,
    mod_fast: bool,
}

impl Dsp {
    fn rebuild(&mut self) {
        let perc_on = self.v[PERC] >= 0.5;
        let mut bars = [0.0f64; 9];
        for (b, g) in bars.iter_mut().enumerate() {
            *g = drawbar_gain(self.v[DB0 + b]);
        }
        if perc_on {
            // percussion takes the 1' busbar; at normal volume the drawbars drop about 3 dB
            bars[8] = 0.0;
            if self.v[PERC_SOFT] < 0.5 {
                bars.iter_mut().for_each(|g| *g *= 0.7);
            }
        }
        self.target = [0.0; WHEELS];
        self.perc_target = [0.0; WHEELS];
        let harm = if self.v[PERC_HARM] >= 0.5 { 4 } else { 3 }; // 2 2/3' (3rd) or 4' (2nd)
        for note in 0..128u8 {
            if !self.held[note as usize] {
                continue;
            }
            for (b, g) in bars.iter().enumerate() {
                if *g > 0.0 {
                    self.target[wheel_for(note, b) - 1] += g;
                }
            }
            self.perc_target[wheel_for(note, harm) - 1] += 1.0;
        }
        self.dirty = false;
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 3;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        // the wheels are nearly sine: a trace of 3rd harmonic from the tooth profile
        let table = (0..=TABLE)
            .map(|i| {
                let t = std::f64::consts::TAU * i as f64 / TABLE as f64;
                t.sin() + 0.012 * (3.0 * t).sin()
            })
            .collect();
        let mut inc = [0.0; WHEELS];
        let mut phase = [0.0; WHEELS];
        let mut nz = Noise::new(91);
        for w in 0..WHEELS {
            inc[w] = wheel_hz(w + 1) / fs;
            phase[w] = 0.5 + 0.5 * nz.next(); // free-running: every wheel somewhere different
        }
        let mut click_bp = Biquad::identity();
        click_bp.set_bandpass(fs, 2500.0, 0.6);
        let mut tone = OnePole::default();
        tone.set_lowpass(fs, 7000.0);
        let mut scan_lp = OnePole::default();
        scan_lp.set_lowpass(fs, 5500.0);
        let mut amp_hp = Biquad::identity();
        amp_hp.set_highpass(fs, 40.0, 0.7);
        Self {
            fs,
            v,
            table,
            inc,
            phase,
            gain: [0.0; WHEELS],
            target: [0.0; WHEELS],
            perc_target: [0.0; WHEELS],
            perc_gain: [0.0; WHEELS],
            held: [false; 128],
            nheld: 0,
            dirty: true,
            perc_env: 0.0,
            click_env: 0.0,
            noise: Noise::new(17),
            click_bp,
            tone,
            scanner: DelayLine::new((fs * 0.006) as usize + 4),
            scan_lfo: Lfo::default(),
            scan_lp,
            amp_hp,
            leslie: Leslie::new(fs),
            mod_fast: false,
        }
    }
    fn reset(&mut self) {
        self.held = [false; 128];
        self.nheld = 0;
        self.gain = [0.0; WHEELS];
        self.perc_gain = [0.0; WHEELS];
        self.perc_env = 0.0;
        self.click_env = 0.0;
        self.dirty = true;
        self.click_bp.reset();
        self.tone.reset();
        self.scanner.reset();
        self.scan_lp.reset();
        self.amp_hp.reset();
        self.leslie.reset();
    }
    fn set_params(&mut self, v: &[f32]) {
        if self.v[..] != v[..NPARAMS] {
            self.dirty = true;
        }
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn midi(&mut self, ev: Midi) {
        match ev {
            Midi::On { note, vel } => {
                let n = note as usize & 127;
                if !self.held[n] {
                    // single trigger: percussion only strikes when every key was up
                    if self.nheld == 0 {
                        self.perc_env = 1.0;
                    }
                    self.held[n] = true;
                    self.nheld += 1;
                    self.click_env = self.click_env.max(0.6 + 0.4 * vel as f64);
                    self.dirty = true;
                }
            }
            Midi::Off { note } => {
                let n = note as usize & 127;
                if self.held[n] {
                    self.held[n] = false;
                    self.nheld -= 1;
                    self.click_env = self.click_env.max(0.4);
                    self.dirty = true;
                }
            }
            Midi::Cc { cc: 1, value } => self.mod_fast = value >= 0.5,
            Midi::AllOff => {
                self.held = [false; 128];
                self.nheld = 0;
                self.dirty = true;
            }
            _ => {}
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        if self.dirty {
            self.rebuild();
        }
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let vol = db_to_gain(self.v[VOLUME] as f64);
        let click = self.v[CLICK] as f64;
        // contacts: the busbars switch almost instantly (that step is part of the click);
        // with less click they close more softly
        let gc = coeff(fs, 0.15 + 4.0 * (1.0 - click));
        let perc_on = self.v[PERC] >= 0.5;
        let perc_t60 = if self.v[PERC_DECAY] >= 0.5 { 2.5 } else { 0.6 };
        let perc_k = 10f64.powf(-3.0 / (perc_t60 * fs));
        let perc_lvl = if self.v[PERC_SOFT] >= 0.5 { 0.5 } else { 1.0 };
        let click_k = (-1.0 / (0.004 * fs)).exp();
        let vib = self.v[VIB].round() as usize; // 0 off, V1 C1 V2 C2 V3 C3
        let (vdepth, chorus) = match vib {
            1 => (0.25, false),
            2 => (0.25, true),
            3 => (0.5, false),
            4 => (0.5, true),
            5 => (0.9, false),
            6 => (0.9, true),
            _ => (0.0, false),
        };
        let drive = self.v[DRIVE] as f64;
        let pre = 1.0 + 9.0 * drive * drive;
        let leslie_on = self.v[LESLIE] >= 0.5;
        let speed = if self.v[SPEED] >= 0.5 || self.mod_fast { 2 } else { 1 };
        let tab = &self.table;
        let mut peak = 0.0f64;
        for k in 0..n {
            let mut s = 0.0;
            let mut ps = 0.0;
            for w in 0..WHEELS {
                let g = self.target[w] + gc * (self.gain[w] - self.target[w]);
                self.gain[w] = g;
                let pg = self.perc_target[w] + gc * (self.perc_gain[w] - self.perc_target[w]);
                self.perc_gain[w] = pg;
                let ph = self.phase[w] + self.inc[w];
                let ph = ph - ph.floor();
                self.phase[w] = ph;
                if g > 1e-6 || pg > 1e-6 {
                    let x = ph * TABLE as f64;
                    let i = x as usize;
                    let f = x - i as f64;
                    let y = tab[i] + f * (tab[i + 1] - tab[i]);
                    s += g * y;
                    ps += pg * y;
                }
            }
            if perc_on {
                s += ps * self.perc_env * perc_lvl * 1.2;
            }
            self.perc_env *= perc_k;
            // key click: the contacts' burst, bright and short
            if self.click_env > 1e-4 {
                s += self.click_bp.process(self.noise.next()) * self.click_env * click * 0.35;
                self.click_env *= click_k;
            }
            let mut x = self.tone.lp(s * 0.15);
            // scanner vibrato: a swept delay line (about 6.9 Hz); chorus adds the dry back
            if vdepth > 0.0 {
                let l = self.scan_lfo.next(fs, 6.87);
                let d = (0.3 + vdepth * 0.55 * (1.0 + l)) * 0.001 * fs;
                self.scanner.write(x);
                let wet = self.scan_lp.lp(self.scanner.read(d.max(1.0)));
                x = if chorus { 0.5 * (x + wet) * 1.3 } else { wet };
            }
            // the Leslie 122's tube amp, then the rotors
            let a = self.amp_hp.process(x);
            let a = tube(a * pre, 0.2) / pre.powf(0.4);
            let (l, r) = if leslie_on { self.leslie.process(a, speed, 1.0, 1.0) } else { (a, a) };
            let (l, r) = (l * vol, r * vol);
            peak = peak.max(l.abs()).max(r.abs());
            if chans.len() >= 2 {
                chans[0][k] = l as f32;
                chans[1][k] = r as f32;
            } else if let Some(c) = chans.get_mut(0) {
                c[k] = (0.5 * (l + r)) as f32;
            }
        }
        meters[0] = peak as f32;
        meters[1] = self.leslie.horn_phase as f32;
        meters[2] = self.leslie.drum_phase as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    // registrations are named by their drawbar numbers, 16' first
    vec![
        Preset { name: "Jazz 888 000 000 Perc", values: &[(0, 8.0), (1, 8.0), (2, 8.0), (3, 0.0), (4, 0.0), (5, 0.0), (6, 0.0), (7, 0.0), (8, 0.0), (PERC, 1.0), (PERC_HARM, 1.0), (PERC_DECAY, 0.0), (VIB, 6.0), (SPEED, 0.0)] },
        Preset { name: "Full Organ 888 888 888", values: &[(0, 8.0), (1, 8.0), (2, 8.0), (3, 8.0), (4, 8.0), (5, 8.0), (6, 8.0), (7, 8.0), (8, 8.0), (PERC, 0.0), (VIB, 6.0), (SPEED, 1.0), (DRIVE, 0.4)] },
        Preset { name: "Gospel 888 800 000", values: &[(0, 8.0), (1, 8.0), (2, 8.0), (3, 8.0), (4, 0.0), (5, 0.0), (6, 0.0), (7, 0.0), (8, 0.0), (PERC, 0.0), (VIB, 6.0), (SPEED, 1.0)] },
        Preset { name: "Rock Grind 888 000 000", values: &[(0, 8.0), (1, 8.0), (2, 8.0), (3, 0.0), (4, 0.0), (5, 0.0), (6, 0.0), (7, 0.0), (8, 0.0), (PERC, 0.0), (VIB, 6.0), (DRIVE, 0.85), (SPEED, 0.0)] },
        Preset { name: "Ballad 688 600 000", values: &[(0, 6.0), (1, 8.0), (2, 8.0), (3, 6.0), (4, 0.0), (5, 0.0), (6, 0.0), (7, 0.0), (8, 0.0), (PERC, 0.0), (VIB, 6.0), (SPEED, 0.0), (DRIVE, 0.1)] },
        Preset { name: "Flutes 004 200 000", values: &[(0, 0.0), (1, 0.0), (2, 4.0), (3, 2.0), (4, 0.0), (5, 0.0), (6, 0.0), (7, 0.0), (8, 0.0), (PERC, 0.0), (VIB, 5.0), (SPEED, 0.0), (DRIVE, 0.0), (VOLUME, 4.0)] },
        Preset { name: "Blues 838 000 000 Perc 2nd", values: &[(0, 8.0), (1, 3.0), (2, 8.0), (3, 0.0), (4, 0.0), (5, 0.0), (6, 0.0), (7, 0.0), (8, 0.0), (PERC, 1.0), (PERC_HARM, 0.0), (PERC_DECAY, 0.0), (VIB, 6.0), (DRIVE, 0.5)] },
        Preset { name: "Squabble 008 000 008", values: &[(0, 0.0), (1, 0.0), (2, 8.0), (3, 0.0), (4, 0.0), (5, 0.0), (6, 0.0), (7, 0.0), (8, 8.0), (PERC, 1.0), (PERC_HARM, 1.0), (VIB, 0.0), (SPEED, 1.0)] },
        Preset { name: "Jazz Soft Slow Perc", values: &[(0, 8.0), (1, 8.0), (2, 8.0), (PERC, 1.0), (PERC_DECAY, 1.0), (PERC_SOFT, 1.0), (VIB, 4.0)] },
        Preset { name: "Dry, No Leslie", values: &[(LESLIE, 0.0), (VIB, 0.0), (DRIVE, 0.0)] },
    ]
}

fn panel() -> Panel {
    const BARS: [&str; 9] = ["16'", "5 1/3'", "8'", "4'", "2 2/3'", "2'", "1 3/5'", "1 1/3'", "1'"];
    const TIPS: [&str; 9] = [
        "16' drawbar: an octave below the key (the sub-octave).",
        "5 1/3' drawbar: the fifth above the 8' (a sub-third harmonic).",
        "8' drawbar: the key's own pitch (the fundamental).",
        "4' drawbar: an octave up (2nd harmonic).",
        "2 2/3' drawbar: an octave and a fifth up (3rd harmonic).",
        "2' drawbar: two octaves up (4th harmonic).",
        "1 3/5' drawbar: two octaves and a major third up (5th harmonic).",
        "1 1/3' drawbar: two octaves and a fifth up (6th harmonic).",
        "1' drawbar: three octaves up (8th harmonic). Silent while percussion is on, as on a B-3.",
    ];
    let mut items = Vec::new();
    for b in 0..9 {
        items.push(Item::Knob { p: DB0 + b, x: 64.0 + b as f32 * 64.0, y: 150.0, r: 22.0, label: BARS[b], tip: TIPS[b], active: if b == 8 { Some(ActiveWhen { p: PERC, min: 0.0, max: 0.5 }) } else { None } });
    }
    items.extend([
        Item::Text { x: 320.0, y: 86.0, text: "DRAWBARS  0 - 8", size: 11.0, align: Align2::CENTER_CENTER },
        Item::Divider { y: 214.0 },
        Item::Text { x: 170.0, y: 240.0, text: "PERCUSSION", size: 11.0, align: Align2::CENTER_CENTER },
        Item::Switch { p: PERC, x: 90.0, y: 272.0, labels: ["OFF", "ON"], tip: "Percussion: a decaying harmonic on the attack. Single-triggered: it only strikes when every key was up, so play detached to hear it." },
        Item::Switch { p: PERC_SOFT, x: 250.0, y: 272.0, labels: ["NORMAL", "SOFT"], tip: "Percussion volume. Normal also drops the drawbars a little, as on the real one." },
        Item::Switch { p: PERC_DECAY, x: 90.0, y: 308.0, labels: ["FAST", "SLOW"], tip: "Percussion decay." },
        Item::Switch { p: PERC_HARM, x: 250.0, y: 308.0, labels: ["2ND", "3RD"], tip: "Percussion harmonic: 2nd (4') or 3rd (2 2/3')." },
        Item::Selector { p: VIB, x: 480.0, y: 285.0, label: "VIBRATO / CHORUS", positions: &["OFF", "V1", "C1", "V2", "C2", "V3", "C3"], tip: "The scanner vibrato: V1-V3 more and more depth, C1-C3 the same mixed with the dry organ (chorus). C3 is the classic." },
        Item::Divider { y: 352.0 },
        Item::Knob { p: CLICK, x: 60.0, y: 432.0, r: 24.0, label: "KEY CLICK", tip: "The busbar contacts closing: the percussive click at the start (and end) of each note.", active: None },
        Item::Knob { p: DRIVE, x: 150.0, y: 432.0, r: 24.0, label: "DRIVE", tip: "How hard the Leslie's tube amp is pushed: from clean to growl.", active: None },
        Item::Switch { p: LESLIE, x: 290.0, y: 414.0, labels: ["BYPASS", "LESLIE"], tip: "Through the Leslie, or straight out of the organ." },
        Item::Switch { p: SPEED, x: 290.0, y: 450.0, labels: ["CHORALE", "TREMOLO"], tip: "Leslie speed: slow (chorale) or fast (tremolo). The mod wheel switches it to fast too. The horn speeds up in about a second, the heavy drum takes longer." },
        Item::Rotor { horn: 1, drum: 2, x: 440.0, y: 430.0, r: 44.0 },
        Item::Knob { p: VOLUME, x: 570.0, y: 432.0, r: 24.0, label: "VOLUME", tip: "Output level.", active: None },
    ]);
    Panel {
        title: "B-3 ORGAN",
        subtitle: ("IN THE MANNER OF THE HAMMOND B-3 AND LESLIE 122", "91 TONEWHEELS · DRAWBARS · PERCUSSION · SCANNER · LESLIE"),
        accent: ui::MOSS,
        size: (W, H),
        items,
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "B-3 Organ", class_id: *b"PQKeysB3OrganLsl", vendor: "PEQUOD Keys",
    instrument, categories: [Instrument, Synth]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        let mut p = defaults();
        for b in 0..9 {
            p[DB0 + b] = 0.0;
        }
        p[DB0 + 2] = 8.0;
        p[PERC] = 0.0;
        p[VIB] = 0.0;
        p[LESLIE] = 0.0;
        p[CLICK] = 0.0;
        p[DRIVE] = 0.0;
        p
    }

    include!("../../instrument_tests.rs");

    #[test]
    fn gears_are_near_equal_temperament() {
        for w in 1..=WHEELS {
            let et = 440.0 * 2f64.powf((w as f64 - 46.0) / 12.0);
            let c = 1200.0 * (wheel_hz(w) / et).log2();
            assert!(c.abs() < 1.5, "wheel {w}: {c:.2} cents");
        }
        assert!((wheel_hz(46) - 440.0).abs() < 1e-9);
    }

    #[test]
    fn percussion_takes_the_1_foot() {
        let p = with(tune_params(), &[(DB0 + 2, 0.0), (DB0 + 8, 8.0)]);
        let ev = [(0, Midi::On { note: 60, vel: 0.8 })];
        let (off, _) = render(&p, &ev, 9600);
        let (on, _) = render(&with(p.clone(), &[(PERC, 1.0), (PERC_DECAY, 1.0)]), &ev, 96000);
        let on_late = &on[86400..];
        println!("1' alone {:.4}, with perc late {:.5}", rms(&off[4800..]), rms(on_late));
        assert!(rms(&off[4800..]) > 0.01);
        assert!(rms(on_late) < 0.1 * rms(&off[4800..]));
    }
}
