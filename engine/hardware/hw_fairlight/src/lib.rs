//! Fairlight CMI Series II (1980-82): the computer musical instrument. Eight voice cards, each with its
//! own waveform memory, its own 8-bit converter and its own clock: a voice plays its wave by running its
//! converter at the note's pitch times the wave's length, so every voice is stepped and grainy in its
//! own way, and the higher it plays the more its images fold back. Waves are drawn as on Page 4 - a
//! harmonic profile - and Page 5's time morph moves the voice from one wave to another as it sounds.
//! Each card has its loudness envelope and a low-pass on its output. Here the waves are profiles to pick
//! (no sample library ships with it); the player, the converter and the cards are the CMI's.

#![allow(clippy::excessive_precision)]
use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
#[allow(unused_imports)]
use vintage::dsp::*;
#[allow(unused_imports)]
use vintage::fx::*;
use vintage::nih_plug_egui::EguiState;
#[allow(unused_imports)]
use vintage::nih_plug_egui::egui::Align2;
#[allow(unused_imports)]
use vintage::params::*;
#[allow(unused_imports)]
use vintage::ui::{self, ActiveWhen, Item, MeterKind, Panel, Preset};
#[allow(unused_imports)]
use vintage::{Midi, VintageDsp, VintageParams};

pub const W: u32 = 620;
pub const H: u32 = 380;

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "wa"]
    pub wa: FloatParam,
    #[id = "wb"]
    pub wb: FloatParam,
    #[id = "morph"]
    pub morph: FloatParam,
    #[id = "len"]
    pub len: FloatParam,
    #[id = "detune"]
    pub detune: FloatParam,
    #[id = "vibrate"]
    pub vibrate: FloatParam,
    #[id = "vibdepth"]
    pub vibdepth: FloatParam,
    #[id = "vibdelay"]
    pub vibdelay: FloatParam,
    #[id = "cutoff"]
    pub cutoff: FloatParam,
    #[id = "a"]
    pub a: FloatParam,
    #[id = "d"]
    pub d: FloatParam,
    #[id = "s"]
    pub s: FloatParam,
    #[id = "r"]
    pub r: FloatParam,
    #[id = "vel"]
    pub vel: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            wa: FloatParam::new("Wave Start", 1.0, FloatRange::Linear { min: 0.0, max: 8.0 }).with_step_size(1.0),
            wb: FloatParam::new("Wave End", 1.0, FloatRange::Linear { min: 0.0, max: 8.0 }).with_step_size(1.0),
            morph: FloatParam::new("Time Morph", 400.0, FloatRange::Skewed { min: 1.0, max: 8000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            len: FloatParam::new("Wave Length", 1.0, FloatRange::Linear { min: 0.0, max: 2.0 }).with_step_size(1.0),
            detune: pct("Card Detune", 0.1),
            vibrate: hz("Vibrato Rate", 5.5, 0.5, 12.0),
            vibdepth: pct("Vibrato Depth", 0.0),
            vibdelay: ms("Vibrato Delay", 300.0, 0.0, 3000.0),
            cutoff: hz("Output Filter", 8000.0, 500.0, 15000.0),
            a: FloatParam::new("Attack", 3.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            d: FloatParam::new("Decay", 800.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            s: pct("Sustain", 0.7),
            r: FloatParam::new("Release", 400.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            vel: pct("Velocity", 0.6),
            volume: db("Volume", -8.0, -30.0, 6.0),
        }
    }
}

pub const WA: usize = 0;
pub const WB: usize = 1;
pub const MORPH: usize = 2;
pub const LEN: usize = 3;
pub const DETUNE: usize = 4;
pub const VIBRATE: usize = 5;
pub const VIBDEPTH: usize = 6;
pub const VIBDELAY: usize = 7;
pub const CUTOFF: usize = 8;
pub const A: usize = 9;
pub const D: usize = 10;
pub const S: usize = 11;
pub const R: usize = 12;
pub const VEL: usize = 13;
pub const VOLUME: usize = 14;
pub const NPARAMS: usize = 15;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.wa.as_ptr(),
            self.wb.as_ptr(),
            self.morph.as_ptr(),
            self.len.as_ptr(),
            self.detune.as_ptr(),
            self.vibrate.as_ptr(),
            self.vibdepth.as_ptr(),
            self.vibdelay.as_ptr(),
            self.cutoff.as_ptr(),
            self.a.as_ptr(),
            self.d.as_ptr(),
            self.s.as_ptr(),
            self.r.as_ptr(),
            self.vel.as_ptr(),
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
#[allow(dead_code)]
#[inline]
fn saw(ph: f64, dt: f64) -> f64 {
    2.0 * ph - 1.0 - blep(ph, dt)
}
#[allow(dead_code)]
#[inline]
fn pulse(ph: f64, dt: f64, w: f64) -> f64 {
    let mut p = if ph < w { 1.0 } else { -1.0 };
    p += blep(ph, dt);
    p -= blep((ph - w + 1.0).fract(), dt);
    p - (2.0 * w - 1.0)
}
#[allow(dead_code)]
#[inline]
fn tri(ph: f64) -> f64 {
    if ph < 0.5 { 4.0 * ph - 1.0 } else { 3.0 - 4.0 * ph }
}
#[allow(dead_code)]
#[inline]
fn mtof(n: f64) -> f64 {
    440.0 * 2f64.powf((n - 69.0) / 12.0)
}
/// A time (ms, 10-90 %) to the per-sample rate of a one-pole segment.
#[allow(dead_code)]
#[inline]
fn rate(fs: f64, ms: f64) -> f64 {
    1.0 - coeff(fs, ms.max(0.3) / 2.2)
}

/// ADSR: attack aims past 1 (as the RC circuits do), decay and release are exponential.
#[allow(dead_code)]
#[derive(Clone, Copy, Default)]
struct Adsr {
    v: f64,
    stage: u8,
}
#[allow(dead_code)]
impl Adsr {
    fn on(&mut self) {
        self.stage = 1;
    }
    fn off(&mut self) {
        if self.stage != 0 {
            self.stage = 3;
        }
    }
    fn idle(&self) -> bool {
        self.stage == 0
    }
    #[inline]
    fn step(&mut self, a: f64, d: f64, s: f64, r: f64) -> f64 {
        match self.stage {
            1 => {
                self.v += (1.3 - self.v) * a;
                if self.v >= 1.0 {
                    self.v = 1.0;
                    self.stage = 2;
                }
            }
            2 => self.v += (s - self.v) * d,
            3 => {
                self.v -= self.v * r;
                if self.v < 1e-5 {
                    self.v = 0.0;
                    self.stage = 0;
                }
            }
            _ => {}
        }
        self.v
    }
}

/// Four one-pole stages with feedback (transistor ladder or OTA cascade). `fb2` takes the
/// feedback from the second stage instead (a 12 dB/octave response). Returns (4-pole, 2-pole).
#[allow(dead_code)]
#[derive(Clone, Copy, Default)]
struct Ladder {
    s: [f64; 4],
}
#[allow(dead_code)]
impl Ladder {
    #[inline]
    fn process(&mut self, x: f64, fc: f64, fs: f64, k: f64, fb2: bool) -> (f64, f64) {
        let g = 1.0 - (-std::f64::consts::TAU * fc.clamp(10.0, 0.42 * fs) / (2.0 * fs)).exp();
        for _ in 0..2 {
            let fb = if fb2 { self.s[1] } else { self.s[3] };
            let u = (x - k * fb).tanh();
            self.s[0] += g * (u - self.s[0].tanh());
            self.s[1] += g * (self.s[0].tanh() - self.s[1].tanh());
            self.s[2] += g * (self.s[1].tanh() - self.s[2].tanh());
            self.s[3] += g * (self.s[2].tanh() - self.s[3].tanh());
        }
        (self.s[3], self.s[1])
    }
}

/// A 12 dB/octave state-variable filter (Oberheim SEM, ARP 4023), trapezoidal; the
/// integrators' OTAs soften at level. Returns (low, band, high).
#[allow(dead_code)]
#[derive(Clone, Copy, Default)]
struct Svf {
    a: f64,
    b: f64,
}
#[allow(dead_code)]
impl Svf {
    #[inline]
    fn process(&mut self, x: f64, fc: f64, fs: f64, res: f64) -> (f64, f64, f64) {
        let g = (std::f64::consts::PI * fc.clamp(10.0, 0.42 * fs) / fs).tan();
        let k = 2.0 - 1.96 * res.clamp(0.0, 1.0);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.b;
        let v1 = a1 * self.a + a2 * v3;
        let v2 = self.b + a2 * self.a + a3 * v3;
        self.a = (2.0 * v1 - self.a).tanh() ;
        self.b = 2.0 * v2 - self.b;
        (v2, v1, x - k * v1 - v2)
    }
}

/// What the voice allocator needs to know of a voice.
#[allow(dead_code)]
trait Slot {
    fn note(&self) -> u8;
    fn held(&self) -> bool;
    fn idle(&self) -> bool;
    fn age(&self) -> u64;
}
/// The same note if it's sounding, else the voice idle longest (round robin, so each voice's
/// own tuning and tolerances come round in turn), else the oldest released, else the oldest.
#[allow(dead_code)]
fn alloc<V: Slot>(vs: &[V], note: u8) -> usize {
    if let Some(i) = vs.iter().position(|v| !v.idle() && v.note() == note) {
        return i;
    }
    let oldest = |f: &dyn Fn(&V) -> bool| vs.iter().enumerate().filter(|(_, v)| f(v)).min_by_key(|(_, v)| v.age()).map(|(i, _)| i);
    oldest(&|v| v.idle()).or_else(|| oldest(&|v| !v.held())).or_else(|| oldest(&|_| true)).unwrap_or(0)
}

// -------------------------------------------------------------------- dsp --

const VOICES: usize = 8;
const WAVES: usize = 9;
const TABLE: usize = 128;

/// Page 4 harmonic profiles (amplitudes of harmonics 1-32).
fn profile(w: usize, h: usize) -> f64 {
    let n = h as f64;
    match w {
        0 => if h == 1 { 1.0 } else { 0.0 },                                  // sine
        1 => 1.0 / n,                                                           // saw
        2 => if h % 2 == 1 { 1.0 / n } else { 0.0 },                            // square
        3 => match h { 1 => 0.8, 2 => 1.0, 3 => 0.6, 4 => 0.5, 6 => 0.35, 8 => 0.3, _ => 0.0 },   // organ
        4 => (-((n - 4.0).powi(2)) / 5.0).exp() + 0.35 / n,                     // choir
        5 => if h == 1 { 0.5 } else { 1.0 / n.powf(0.8) },                       // strings
        6 => n.sqrt() * (-n / 5.0).exp(),                                       // brass
        7 => match h { 1 => 1.0, 4 => 0.6, 7 => 0.45, 11 => 0.35, 17 => 0.2, _ => 0.0 },   // bell
        _ => (-((n - 7.0).powi(2)) / 10.0).exp() + 0.25 / n,                    // breathy vox
    }
}

#[derive(Clone, Copy, Default)]
struct Voice {
    note: u8,
    held: bool,
    sus: bool,
    age: u64,
    vel: f64,
    pitch: f64,
    fenv: Adsr,
    aenv: Adsr,
    pos: f64,
    held_sample: f64,
    since: f64,
    lp: [f64; 2],
    trim: f64,
}

impl Slot for Voice {
    fn note(&self) -> u8 {
        self.note
    }
    fn held(&self) -> bool {
        self.held || self.sus
    }
    fn idle(&self) -> bool {
        self.aenv.idle()
    }
    fn age(&self) -> u64 {
        self.age
    }
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    voices: [Voice; VOICES],
    mono: Vec<u8>,
    clock: u64,
    tables: Vec<[f64; TABLE]>,
    lfo: f64,
    bend: f64,
    modw: f64,
    pedal: bool,
    dc: DcBlock,
}

impl Dsp {
    fn unison(&self) -> bool {
        false
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let tables = (0..WAVES)
            .map(|w| {
                let mut t = [0.0f64; TABLE];
                for (i, x) in t.iter_mut().enumerate() {
                    let ph = i as f64 / TABLE as f64;
                    *x = (1..=32).map(|h| profile(w, h) * (std::f64::consts::TAU * h as f64 * ph).sin()).sum();
                }
                let pk = t.iter().fold(0.0f64, |a, x| a.max(x.abs())).max(1e-9);
                t.iter_mut().for_each(|x| *x /= pk);
                t
            })
            .collect();
        let mut voices = [Voice::default(); VOICES];
        const TRIM: [f64; VOICES] = [0.01, -0.02, 0.015, -0.005, 0.02, -0.015, 0.005, -0.01];
        for (i, vc) in voices.iter_mut().enumerate() {
            vc.trim = TRIM[i];
        }
        Self { fs, v, voices, mono: Vec::new(), clock: 0, tables, lfo: 0.0, bend: 0.0, modw: 0.0, pedal: false, dc: DcBlock::new(fs) }
    }
    fn reset(&mut self) {
        for v in self.voices.iter_mut() {
            v.fenv = Adsr::default();
            v.aenv = Adsr::default();
            v.held = false;
            v.lp = [0.0; 2];
        }
        self.dc.reset();
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }

    fn midi(&mut self, ev: Midi) {
        match ev {
            Midi::On { note, vel } => {
                self.clock += 1;
                let unison = self.unison();
                let idx: Vec<usize> = if unison { (0..VOICES).collect() } else { vec![alloc(&self.voices, note)] };
                for i in idx {
                    let vc = &mut self.voices[i];
                    if vc.idle() || true && !unison {
                        vc.pitch = note as f64;
                    }
                    vc.note = note;
                    vc.held = true;
                    vc.sus = false;
                    vc.vel = vel as f64;
                    vc.age = self.clock;
                    vc.fenv.on();
                    vc.since = 0.0;
                    vc.pos = 0.0;
                    vc.aenv.on();
                }
                if unison {
                    self.mono.retain(|n| *n != note);
                    self.mono.push(note);
                }
            }
            Midi::Off { note } => {
                if self.unison() {
                    self.mono.retain(|n| *n != note);
                    if let Some(last) = self.mono.last().copied() {
                        self.voices.iter_mut().for_each(|v| v.note = last);
                        return;
                    }
                }
                let pedal = self.pedal;
                for v in self.voices.iter_mut().filter(|v| v.held && v.note == note) {
                    v.held = false;
                    if pedal {
                        v.sus = true;
                    } else {
                        v.fenv.off();
                        v.aenv.off();
                    }
                }
                if self.unison() && self.mono.is_empty() {
                    for v in self.voices.iter_mut() {
                        v.held = false;
                        v.fenv.off();
                        v.aenv.off();
                    }
                }
            }
            Midi::Bend { value } => self.bend = value as f64 * 2.0,
            Midi::Cc { cc: 1, value } => self.modw = value as f64,
            Midi::Cc { cc: 64, value } => {
                self.pedal = value >= 0.5;
                if !self.pedal {
                    for v in self.voices.iter_mut().filter(|v| v.sus) {
                        v.sus = false;
                        v.fenv.off();
                        v.aenv.off();
                    }
                }
            }
            Midi::AllOff => {
                self.mono.clear();
                for v in self.voices.iter_mut() {
                    v.held = false;
                    v.sus = false;
                    v.fenv.off();
                    v.aenv.off();
                }
            }
            _ => {}
        }
    }

    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let p = self.v;
        let st = |i: usize, max: usize| (p[i].round().max(0.0) as usize).min(max);
        let (wa, wb) = (st(WA, WAVES - 1), st(WB, WAVES - 1));
        // shorter waves: a coarser grain (the CMI let you draw 32 to 128 points per cycle)
        let len = [32usize, 64, 128][st(LEN, 2)];
        let stride = TABLE / len;
        let (a, d, s, r) = (rate(fs, p[A] as f64), rate(fs, p[D] as f64), p[S] as f64, rate(fs, p[R] as f64));
        let mt = (p[MORPH] as f64 / 1000.0).max(0.001);
        let g = 1.0 - (-std::f64::consts::TAU * p[CUTOFF] as f64 / fs).exp();
        let dly = (p[VIBDELAY] as f64 / 1000.0).max(0.001);
        let vol = db_to_gain(p[VOLUME] as f64);
        let mut peak = 0.0f64;
        for kk in 0..n {
            self.lfo = (self.lfo + p[VIBRATE] as f64 / fs).fract();
            let lv = (std::f64::consts::TAU * self.lfo).sin();
            let mut sum = 0.0;
            for vc in self.voices.iter_mut() {
                if vc.aenv.idle() {
                    continue;
                }
                vc.since += 1.0 / fs;
                let vib = lv * (p[VIBDEPTH] as f64 * (vc.since / dly).min(1.0) + self.modw * 0.5) * 0.5;
                let f = mtof(vc.note as f64 + self.bend + vib + vc.trim * p[DETUNE] as f64 * 10.0);
                // the card's converter clock: the pitch times the wave's length
                vc.pos += f * len as f64 / fs;
                let mut stepped = false;
                while vc.pos >= 1.0 {
                    vc.pos -= 1.0;
                    vc.pitch = (vc.pitch + 1.0) % len as f64;
                    stepped = true;
                }
                if stepped {
                    let i = vc.pitch as usize * stride;
                    let m = (vc.since / mt).min(1.0);
                    let x = self.tables[wa][i] * (1.0 - m) + self.tables[wb][i] * m;
                    // 8 bits
                    vc.held_sample = (x * 127.0).round() / 127.0;
                }
                let ae = vc.aenv.step(a, d, s, r);
                let vs = 1.0 - p[VEL] as f64 * (1.0 - vc.vel);
                // the card's output filter (two poles)
                vc.lp[0] += g * (vc.held_sample - vc.lp[0]);
                vc.lp[1] += g * (vc.lp[0] - vc.lp[1]);
                // the card's VCA is 8-bit too: the envelope steps
                let env = (ae * 255.0).round() / 255.0;
                sum += vc.lp[1] * env * vs;
            }
            let y = self.dc.process(sum) * vol * 0.6;
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
        Preset { name: "Choir Pad", values: &[(WA, 4.0), (WB, 8.0), (MORPH, 1500.0), (A, 200.0), (R, 1500.0), (VIBDEPTH, 0.15)] },
        Preset { name: "Glass Bell", values: &[(WA, 7.0), (WB, 0.0), (MORPH, 1200.0), (A, 1.0), (D, 2500.0), (S, 0.0), (R, 2000.0)] },
        Preset { name: "Brass Section", values: &[(WA, 0.0), (WB, 6.0), (MORPH, 120.0), (A, 40.0), (D, 800.0), (S, 0.8), (CUTOFF, 6000.0)] },
        Preset { name: "Strings", values: &[(WA, 5.0), (WB, 5.0), (DETUNE, 0.5), (A, 350.0), (R, 1200.0), (VIBDEPTH, 0.12), (VIBDELAY, 500.0)] },
        Preset { name: "Organ", values: &[(WA, 3.0), (WB, 3.0), (A, 2.0), (S, 1.0), (R, 40.0), (VEL, 0.0)] },
        Preset { name: "Grainy Lead", values: &[(WA, 1.0), (WB, 2.0), (LEN, 0.0), (MORPH, 600.0), (CUTOFF, 12000.0), (VIBDEPTH, 0.2)] },
        Preset { name: "Breath Vox", values: &[(WA, 8.0), (WB, 4.0), (MORPH, 900.0), (A, 120.0), (R, 900.0), (CUTOFF, 5000.0)] },
    ]
}

const WAVE_NAMES: &[&str] = &["SINE", "SAW", "SQUARE", "ORGAN", "CHOIR", "STRINGS", "BRASS", "BELL", "VOX"];

fn panel() -> Panel {
    let k = |p, x, y, r, label, tip| Item::Knob { p, x, y, r, label, tip, active: None };
    Panel {
        title: "FAIRLIGHT CMI",
        subtitle: ("SERIES II, 1980", "8 VOICE CARDS · 8-BIT · VARIABLE-RATE CONVERTERS · TIME MORPH"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::Selector { p: WA, x: 60.0, y: 130.0, label: "WAVE START", positions: WAVE_NAMES, tip: "The wave each note starts on (Page 4)." },
            Item::Selector { p: WB, x: 170.0, y: 130.0, label: "WAVE END", positions: WAVE_NAMES, tip: "The wave it morphs to (Page 5)." },
            k(MORPH, 270.0, 120.0, 18.0, "TIME MORPH", "How long the morph from start to end takes."),
            Item::Selector { p: LEN, x: 360.0, y: 120.0, label: "POINTS", positions: &["32", "64", "128"], tip: "Points per cycle: fewer, a coarser grain." },
            k(DETUNE, 450.0, 120.0, 16.0, "CARDS", "How far the eight cards' tunings differ."),
            k(VOLUME, 560.0, 120.0, 20.0, "VOLUME", "Output level."),
            Item::Divider { y: 200.0 },
            k(VIBRATE, 60.0, 250.0, 16.0, "VIB RATE", "Vibrato rate."),
            k(VIBDEPTH, 120.0, 250.0, 16.0, "VIB DEPTH", "Vibrato depth (the mod wheel adds more)."),
            k(VIBDELAY, 180.0, 250.0, 16.0, "VIB DELAY", "Vibrato fades in after each key."),
            k(CUTOFF, 270.0, 250.0, 20.0, "FILTER", "Each card's output filter."),
            k(VEL, 350.0, 250.0, 16.0, "VELOCITY", "Velocity to loudness."),
            Item::Divider { y: 295.0 },
            k(A, 60.0, 340.0, 15.0, "ATTACK", "Attack."),
            k(D, 120.0, 340.0, 15.0, "DECAY", "Decay."),
            k(S, 180.0, 340.0, 15.0, "SUSTAIN", "Sustain."),
            k(R, 240.0, 340.0, 15.0, "RELEASE", "Release."),
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Fairlight CMI", class_id: *b"HWFairlightCMIs2", vendor: "PEQUOD Hardware",
    instrument, categories: [Instrument, Synth]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        with(defaults(), &[(WA, 1.0), (WB, 1.0), (S, 1.0), (CUTOFF, 15000.0), (DETUNE, 0.0)])
    }

    /// Eight notes at once, and the ninth takes the oldest voice.
    #[test]
    fn eight_voices() {
        let mut ev = vec![];
        for (i, n) in [48u8, 52, 55, 59, 62, 65, 69, 72, 76].iter().enumerate() {
            ev.push((i * 100, Midi::On { note: *n, vel: 0.8 }));
        }
        let (l, _) = render(&tune_params(), &ev, 24000);
        assert!(l.iter().all(|v| v.is_finite()) && rms(&l[4000..]) > 1e-3);
    }

    include!("../../instrument_tests.rs");
}
