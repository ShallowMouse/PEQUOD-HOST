//! Tonewheel Organ: Rick Wright's Hammond (an M-102 and later a C3 / RT-3, from Ummagumma on): 91 tonewheels, nine drawbars,
//! percussion, the scanner vibrato / chorus, key click and a driven preamp. No Leslie here: use Rotary Speakers after it.

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
pub const H: u32 = 480;

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum PercHarm {
    #[name = "Second"]
    Second,
    #[name = "Third"]
    Third,
}

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum PercDecay {
    #[name = "Fast"]
    Fast,
    #[name = "Slow"]
    Slow,
}

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum PercVol {
    #[name = "Normal"]
    Normal,
    #[name = "Soft"]
    Soft,
}

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum Vibrato {
    #[name = "Off"]
    Off,
    #[name = "V1"]
    V1,
    #[name = "V2"]
    V2,
    #[name = "V3"]
    V3,
    #[name = "C1"]
    C1,
    #[name = "C2"]
    C2,
    #[name = "C3"]
    C3,
}

/// On/off switches are stored as 0/1 (as the released plugin saved them).
#[derive(Enum, PartialEq, Clone, Copy)]
pub enum OffOn {
    #[name = "Off"]
    Off,
    #[name = "On"]
    On,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "d16"]
    pub d16: FloatParam,
    #[id = "d513"]
    pub d513: FloatParam,
    #[id = "d8"]
    pub d8: FloatParam,
    #[id = "d4"]
    pub d4: FloatParam,
    #[id = "d223"]
    pub d223: FloatParam,
    #[id = "d2"]
    pub d2: FloatParam,
    #[id = "d135"]
    pub d135: FloatParam,
    #[id = "d113"]
    pub d113: FloatParam,
    #[id = "d1"]
    pub d1: FloatParam,
    #[id = "perc"]
    pub perc: EnumParam<OffOn>,
    #[id = "perc_harm"]
    pub perc_harm: EnumParam<PercHarm>,
    #[id = "perc_decay"]
    pub perc_decay: EnumParam<PercDecay>,
    #[id = "perc_vol"]
    pub perc_vol: EnumParam<PercVol>,
    #[id = "vibrato"]
    pub vibrato: EnumParam<Vibrato>,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "click"]
    pub click: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            d16: FloatParam::new("16'", 8.0, FloatRange::Linear { min: 0.0, max: 8.0 }).with_step_size(1.0),  // texts '0' .. '4' .. '8'
            d513: FloatParam::new("5 1/3'", 8.0, FloatRange::Linear { min: 0.0, max: 8.0 }).with_step_size(1.0),  // texts '0' .. '4' .. '8'
            d8: FloatParam::new("8'", 8.0, FloatRange::Linear { min: 0.0, max: 8.0 }).with_step_size(1.0),  // texts '0' .. '4' .. '8'
            d4: FloatParam::new("4'", 0.0, FloatRange::Linear { min: 0.0, max: 8.0 }).with_step_size(1.0),  // texts '0' .. '4' .. '8'
            d223: FloatParam::new("2 2/3'", 0.0, FloatRange::Linear { min: 0.0, max: 8.0 }).with_step_size(1.0),  // texts '0' .. '4' .. '8'
            d2: FloatParam::new("2'", 0.0, FloatRange::Linear { min: 0.0, max: 8.0 }).with_step_size(1.0),  // texts '0' .. '4' .. '8'
            d135: FloatParam::new("1 3/5'", 0.0, FloatRange::Linear { min: 0.0, max: 8.0 }).with_step_size(1.0),  // texts '0' .. '4' .. '8'
            d113: FloatParam::new("1 1/3'", 0.0, FloatRange::Linear { min: 0.0, max: 8.0 }).with_step_size(1.0),  // texts '0' .. '4' .. '8'
            d1: FloatParam::new("1'", 0.0, FloatRange::Linear { min: 0.0, max: 8.0 }).with_step_size(1.0),  // texts '0' .. '4' .. '8'
            perc: EnumParam::new("Percussion", OffOn::Off),
            perc_harm: EnumParam::new("Perc Harmonic", PercHarm::Third),
            perc_decay: EnumParam::new("Perc Decay", PercDecay::Fast),
            perc_vol: EnumParam::new("Perc Volume", PercVol::Normal),
            vibrato: EnumParam::new("Vibrato/Chorus", Vibrato::C3),
            drive: pct("Overdrive", 0.2),
            click: pct("Key Click", 0.4),
            volume: db("Volume", 0.0, -24.0, 6.0),
        }
    }
}

pub const D16: usize = 0;
pub const D513: usize = 1;
pub const D8: usize = 2;
pub const D4: usize = 3;
pub const D223: usize = 4;
pub const D2: usize = 5;
pub const D135: usize = 6;
pub const D113: usize = 7;
pub const D1: usize = 8;
pub const PERC: usize = 9;
pub const PERC_HARM: usize = 10;
pub const PERC_DECAY: usize = 11;
pub const PERC_VOL: usize = 12;
pub const VIBRATO: usize = 13;
pub const DRIVE: usize = 14;
pub const CLICK: usize = 15;
pub const VOLUME: usize = 16;
pub const NPARAMS: usize = 17;

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
            self.d513.as_ptr(),
            self.d8.as_ptr(),
            self.d4.as_ptr(),
            self.d223.as_ptr(),
            self.d2.as_ptr(),
            self.d135.as_ptr(),
            self.d113.as_ptr(),
            self.d1.as_ptr(),
            self.perc.as_ptr(),
            self.perc_harm.as_ptr(),
            self.perc_decay.as_ptr(),
            self.perc_vol.as_ptr(),
            self.vibrato.as_ptr(),
            self.drive.as_ptr(),
            self.click.as_ptr(),
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

/// The organ itself (keys/kb_hammond), driven through this plugin's own parameters.
#[allow(dead_code, clippy::all)]
mod tpl {
    use super::*;
    use vintage::fx::Leslie;
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
const WHEELS: usize = 91;

    pub fn defaults() -> Vec<f32> {
        vec![0.0; NPARAMS]
    }


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


}

fn map(v: &[f32]) -> [f32; tpl::NPARAMS] {
    let mut t = [0.0f32; tpl::NPARAMS];
    let bars = [D16, D513, D8, D4, D223, D2, D135, D113, D1];
    for (i, &b) in bars.iter().enumerate() {
        t[tpl::DB0 + i] = v[b];
    }
    t[tpl::PERC] = v[PERC];
    // ours: Second, Third; the organ's: 1 = second (4'), 0 = third (2 2/3')
    t[tpl::PERC_HARM] = 1.0 - v[PERC_HARM].round();
    t[tpl::PERC_DECAY] = v[PERC_DECAY];
    t[tpl::PERC_SOFT] = v[PERC_VOL];
    // ours: Off V1 V2 V3 C1 C2 C3; the organ's: Off V1 C1 V2 C2 V3 C3
    t[tpl::VIB] = [0.0, 1.0, 3.0, 5.0, 2.0, 4.0, 6.0][(v[VIBRATO].round() as usize).min(6)];
    t[tpl::CLICK] = v[CLICK];
    t[tpl::DRIVE] = v[DRIVE];
    // no Leslie in the organ itself: Rick Wright's went through the Rotary Speakers
    t[tpl::LESLIE] = 0.0;
    t[tpl::SPEED] = 0.0;
    t[tpl::VOLUME] = v[VOLUME] - 4.5;
    t
}

pub struct Dsp {
    inner: tpl::Dsp,
}

impl VintageDsp for Dsp {
    const METERS: usize = <tpl::Dsp as VintageDsp>::METERS;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let mut inner = tpl::Dsp::new(sample_rate, ch);
        inner.set_params(&map(&defaults()));
        Self { inner }
    }
    fn reset(&mut self) {
        self.inner.reset();
    }
    fn latency(&self) -> u32 {
        self.inner.latency()
    }
    fn set_params(&mut self, v: &[f32]) {
        self.inner.set_params(&map(v));
    }
    fn midi(&mut self, ev: Midi) {
        self.inner.midi(ev);
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        self.inner.process(chans, meters);
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Wright Pad (88 8000 000)", values: &[] },
        Preset { name: "Echoes Organ", values: &[(D513, 6.0), (D2, 4.0), (DRIVE, 0.3)] },
        Preset { name: "Mellow Flutes (00 8800 000)", values: &[(D16, 0.0), (D513, 0.0), (VIBRATO, 4.0)] },
        Preset { name: "Full Organ (88 8888 888)", values: &[(D223, 8.0), (D2, 8.0), (D135, 8.0), (D113, 8.0), (D1, 8.0), (DRIVE, 0.4)] },
        Preset { name: "Jazz Percussion (888 000 000)", values: &[(PERC, 1.0)] },
        Preset { name: "Gospel (88 8800 008)", values: &[(D1, 8.0), (DRIVE, 0.35)] },
        Preset { name: "Driven Rock Organ", values: &[(D2, 4.0), (VIBRATO, 0.0), (DRIVE, 0.75)] },
        Preset { name: "Soft Background", values: &[(D16, 4.0), (D513, 0.0), (D8, 6.0), (VIBRATO, 4.0), (DRIVE, 0.0), (CLICK, 0.1)] },
    ]
}

/*
('knob', 51.5, 139.5, 22.0)
('knob', 118.5, 139.5, 22.0)
('knob', 185.5, 139.5, 22.0)
('knob', 252.5, 139.5, 22.0)
('knob', 319.5, 139.5, 22.0)
('knob', 386.5, 139.5, 22.0)
('knob', 453.5, 139.5, 22.0)
('knob', 520.5, 139.5, 22.0)
('knob', 587.5, 139.5, 22.0)
('divider', 206)
('selector?', 539.5, 277.5, 36, 36)
('divider', 330)
('knob', 179.5, 383.5, 22.0)
('knob', 319.5, 383.5, 22.0)
('knob', 459.5, 383.5, 22.0)

crate: ['floyd/pf_organ', 'library/panic_abort']
central_panel
vintage/src/ui.rs
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
PERCUSSIONOFFONPercussion: a decaying 2nd or 3rd harmonic on the first note of a phrase (single trigger). Takes over the 1' drawbar.HARMONIC2ND3RDPercussion harmonic: 2nd (4') or 3rd (2 2/3').DECAYFASTSLOWPercussion decay: fast (~0.2 s) or slow (~1 s).VOLUMENORMSOFTPercussion level. NORMAL also drops the drawbars a little, as the organ does.V1V2V3C1C2C3
VIBRATO / CHORUSThe scanner: V1-V3 vibrato (pitch), C1-C3 chorus (vibrato mixed with the dry organ).OVERDRIVEThe organ preamp driven hard (play into Rotary Speakers for the classic growl).KEY CLICKKey contact click and bounce.Output level (the swell pedal).AFTER THE HAMMOND TONEWHEEL ORGANS91 WHEELS  
  9 DRAWBARS  
  MIDITONEWHEEL ORGAN16'5 1/3'8'4'2 2/3'2'1 3/5'1 1/3'1'floyd/pf_organ/src/lib.rs
Drawbar (0-8): how much of this footage. Brown: 16' and 5 1/3' (sub); white: 8', 4', 2', 1' (octaves); black: 2 2/3', 1 3/5', 1 1/3'.Wright Pad (88 8000 000)
Echoes Organ
Mellow Flutes (00 8800 000)
Full Organ (88 8888 888)
Jazz Percussion (888 000 000)

*/
fn panel() -> Panel {
    Panel {
        title: "TONEWHEEL ORGAN",
        subtitle: ("AFTER THE HAMMOND TONEWHEEL ORGANS", "91 WHEELS  ·  9 DRAWBARS  ·  MIDI"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::Knob { p: D16, x: 51.5, y: 140.0, r: 22.0, label: "16'", tip: "Drawbar 16' (0-8).", active: None },
            Item::Knob { p: D513, x: 118.5, y: 140.0, r: 22.0, label: "5 1/3'", tip: "Drawbar 5 1/3' (0-8).", active: None },
            Item::Knob { p: D8, x: 185.5, y: 140.0, r: 22.0, label: "8'", tip: "Drawbar 8' (0-8).", active: None },
            Item::Knob { p: D4, x: 252.5, y: 140.0, r: 22.0, label: "4'", tip: "Drawbar 4' (0-8).", active: None },
            Item::Knob { p: D223, x: 319.5, y: 140.0, r: 22.0, label: "2 2/3'", tip: "Drawbar 2 2/3' (0-8).", active: None },
            Item::Knob { p: D2, x: 386.5, y: 140.0, r: 22.0, label: "2'", tip: "Drawbar 2' (0-8).", active: None },
            Item::Knob { p: D135, x: 453.5, y: 140.0, r: 22.0, label: "1 3/5'", tip: "Drawbar 1 3/5' (0-8).", active: None },
            Item::Knob { p: D113, x: 520.5, y: 140.0, r: 22.0, label: "1 1/3'", tip: "Drawbar 1 1/3' (0-8).", active: None },
            Item::Knob { p: D1, x: 587.5, y: 140.0, r: 22.0, label: "1'", tip: "Drawbar 1' (0-8).", active: None },
            Item::Divider { y: 206.0 },
            Item::Text { x: 70.5, y: 236.0, text: "PERCUSSION", size: 13.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: PERC, x: 70.5, y: 278.0, labels: ["OFF", "ON"], tip: "Percussion: a decaying 2nd or 3rd harmonic on the first note of a phrase (single trigger). Takes over the 1' drawbar." },
            Item::Text { x: 180.5, y: 236.0, text: "HARMONIC", size: 13.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: PERC_HARM, x: 180.5, y: 278.0, labels: ["2ND", "3RD"], tip: "Percussion harmonic: 2nd (4') or 3rd (2 2/3')." },
            Item::Text { x: 290.5, y: 236.0, text: "DECAY", size: 13.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: PERC_DECAY, x: 290.5, y: 278.0, labels: ["FAST", "SLOW"], tip: "Percussion decay: fast (~0.2 s) or slow (~1 s)." },
            Item::Text { x: 399.5, y: 236.0, text: "VOLUME", size: 13.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: PERC_VOL, x: 399.5, y: 278.0, labels: ["NORM", "SOFT"], tip: "Percussion level. NORMAL also drops the drawbars a little, as the organ does." },
            Item::Selector { p: VIBRATO, x: 540.0, y: 278.0, label: "VIBRATO / CHORUS", positions: &["OFF", "V1", "V2", "V3", "C1", "C2", "C3"], tip: "The scanner: V1-V3 vibrato (pitch), C1-C3 chorus (vibrato mixed with the dry organ)." },
            Item::Divider { y: 330.0 },
            Item::Knob { p: DRIVE, x: 180.0, y: 384.0, r: 22.0, label: "OVERDRIVE", tip: "The organ preamp driven hard (play into Rotary Speakers for the classic growl).", active: None },
            Item::Knob { p: CLICK, x: 320.0, y: 384.0, r: 22.0, label: "KEY CLICK", tip: "Key contact click and bounce.", active: None },
            Item::Knob { p: VOLUME, x: 460.0, y: 384.0, r: 22.0, label: "VOLUME", tip: "Output level (the swell pedal).", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Tonewheel Organ", class_id: *b"PFOrganTWPQD0001", vendor: "PEQUOD Pink Floyd",
    instrument, categories: [Instrument, Synth]
);
