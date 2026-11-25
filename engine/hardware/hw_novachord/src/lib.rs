//! Hammond Novachord (1939): the first polyphonic synthesizer, 163 valves and 72 keys. Twelve top-
//! octave valve oscillators and their dividers give every key, phase-locked to its octaves (the dividers'
//! square waves stacked to make the rich, saw-like tone); each key has its own envelope, so it plays
//! from a slow string swell to a struck, decaying piano or harp tone (the PERCUSSION and SUSTAIN
//! controls). The whole instrument then passes the TONE selector's formant filters and the three
//! RESONATORS, the BALANCER between bass and treble, and the VIBRATO. From the service manual's block
//! diagram; values estimated.

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
pub const H: u32 = 360;

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "perc"]
    pub perc: FloatParam,
    #[id = "sustain"]
    pub sustain: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "res1"]
    pub res1: FloatParam,
    #[id = "res2"]
    pub res2: FloatParam,
    #[id = "res3"]
    pub res3: FloatParam,
    #[id = "bright"]
    pub bright: FloatParam,
    #[id = "deep"]
    pub deep: FloatParam,
    #[id = "balance"]
    pub balance: FloatParam,
    #[id = "vibrato"]
    pub vibrato: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            attack: FloatParam::new("Attack", 1.0, FloatRange::Linear { min: 0.0, max: 4.0 }).with_step_size(1.0),
            perc: FloatParam::new("Percussion", 0.0, FloatRange::Linear { min: 0.0, max: 3.0 }).with_step_size(1.0),
            sustain: FloatParam::new("Sustain", 1.0, FloatRange::Linear { min: 0.0, max: 3.0 }).with_step_size(1.0),
            tone: FloatParam::new("Tone", 2.0, FloatRange::Linear { min: 0.0, max: 4.0 }).with_step_size(1.0),
            res1: FloatParam::new("Resonator 1", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            res2: FloatParam::new("Resonator 2", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            res3: FloatParam::new("Resonator 3", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            bright: FloatParam::new("Brilliant", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            deep: FloatParam::new("Deep Tone", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            balance: pct("Balancer", 0.5),
            vibrato: FloatParam::new("Vibrato", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            volume: db("Volume", -8.0, -30.0, 6.0),
        }
    }
}

pub const ATTACK: usize = 0;
pub const PERC: usize = 1;
pub const SUSTAIN: usize = 2;
pub const TONE: usize = 3;
pub const RES1: usize = 4;
pub const RES2: usize = 5;
pub const RES3: usize = 6;
pub const BRIGHT: usize = 7;
pub const DEEP: usize = 8;
pub const BALANCE: usize = 9;
pub const VIBRATO: usize = 10;
pub const VOLUME: usize = 11;
pub const NPARAMS: usize = 12;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.attack.as_ptr(),
            self.perc.as_ptr(),
            self.sustain.as_ptr(),
            self.tone.as_ptr(),
            self.res1.as_ptr(),
            self.res2.as_ptr(),
            self.res3.as_ptr(),
            self.bright.as_ptr(),
            self.deep.as_ptr(),
            self.balance.as_ptr(),
            self.vibrato.as_ptr(),
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

/// A divide-down bank: twelve top-octave oscillators (C8 to B8), every lower key a binary
/// division of them - so all the C's, all the C#'s... are locked in phase, as on the real thing.
#[derive(Clone)]
struct Bank {
    cycles: [f64; 12],
}
impl Bank {
    fn new() -> Self {
        Self { cycles: std::array::from_fn(|i| 0.37 * i as f64) }
    }
    /// Advance by one sample; `shift` in semitones (bend, vibrato, detune) moves the masters.
    #[inline]
    fn step(&mut self, fs: f64, shift: f64) {
        let r = 2f64.powf(shift / 12.0);
        for (i, c) in self.cycles.iter_mut().enumerate() {
            *c += mtof(108.0 + i as f64) * r / fs;
            if *c >= 1024.0 {
                *c -= 1024.0;
            }
        }
    }
    /// (phase, increment) of a key.
    #[inline]
    fn key(&self, note: u8, fs: f64, shift: f64) -> (f64, f64) {
        let note = note.min(119);
        let pc = (note % 12) as usize;
        let k = ((108 + pc as i32 - note as i32) / 12).max(0);
        let div = (1u32 << k) as f64;
        ((self.cycles[pc] / div).fract(), mtof(note as f64 + shift) / fs)
    }
}

// -------------------------------------------------------------------- dsp --

const VOICES: usize = 36;

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
    bank: Bank,
    vib: f64,
    tone: [Biquad; 3],
    res: [Biquad; 3],
    shelf: [Biquad; 2],
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

/// The dividers' squares at the key and its two octaves up, stacked: a stepped, saw-like wave.
#[inline]
fn divided(bank: &Bank, note: u8, fs: f64, shift: f64) -> f64 {
    let mut x = 0.0;
    let mut g = 1.0;
    for o in 0..3u8 {
        let nn = note.saturating_add(12 * o);
        if nn > 119 {
            break;
        }
        let (ph, dt) = bank.key(nn, fs, shift);
        x += g * pulse(ph, dt.min(0.45), 0.5);
        g *= 0.5;
    }
    x * 0.55
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut res = [Biquad::identity(), Biquad::identity(), Biquad::identity()];
        res[0].set_peak(fs, 530.0, 2.0, 9.0);
        res[1].set_peak(fs, 1200.0, 2.5, 9.0);
        res[2].set_peak(fs, 2700.0, 3.0, 9.0);
        Self { fs, v, voices: [Voice::default(); VOICES], mono: Vec::new(), clock: 0, bank: Bank::new(), vib: 0.0, tone: [Biquad::identity(), Biquad::identity(), Biquad::identity()], res, shelf: [Biquad::identity(), Biquad::identity()], bend: 0.0, modw: 0.0, pedal: false, dc: DcBlock::new(fs) }
    }
    fn reset(&mut self) {
        for v in self.voices.iter_mut() {
            v.fenv = Adsr::default();
            v.aenv = Adsr::default();
            v.held = false;
        }
        self.tone.iter_mut().for_each(|b| b.reset());
        self.res.iter_mut().for_each(|b| b.reset());
        self.shelf.iter_mut().for_each(|b| b.reset());
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
        // ATTACK: sharp to slow swell
        let a = rate(fs, [4.0, 25.0, 120.0, 450.0, 1300.0][st(ATTACK, 4)]);
        // PERCUSSION off: the key holds; small to large: it decays while held, like a struck string
        let perc = st(PERC, 3);
        let (d, s) = match perc {
            0 => (rate(fs, 100.0), 1.0),
            1 => (rate(fs, 400.0), 0.0),
            2 => (rate(fs, 1200.0), 0.0),
            _ => (rate(fs, 3500.0), 0.0),
        };
        // SUSTAIN: how long the tone lasts after the key is let go
        let r = rate(fs, [60.0, 400.0, 1200.0, 3000.0][st(SUSTAIN, 3)]);
        // TONE: formant shaping of the whole instrument (mellow to brilliant)
        let tone = st(TONE, 4);
        let (lpf, mid) = [(1500.0, -3.0), (3000.0, 0.0), (5000.0, 2.0), (7000.0, 4.0), (2200.0, -1.0)][tone];
        self.tone[0].set_lowpass(fs, lpf, 0.6);
        self.tone[1].set_peak(fs, 800.0, 0.8, mid);
        self.tone[2].set_high_shelf(fs, 3000.0, 1.0, if p[BRIGHT] >= 0.5 { 6.0 } else { 0.0 });
        self.shelf[0].set_low_shelf(fs, 180.0, 1.0, if p[DEEP] >= 0.5 { 8.0 } else { 0.0 } + if tone == 4 { 4.0 } else { 0.0 });
        let bal = p[BALANCE] as f64;
        self.shelf[1].set_high_shelf(fs, 700.0, 0.7, (bal - 0.5) * 18.0);
        let rs = [p[RES1] >= 0.5, p[RES2] >= 0.5, p[RES3] >= 0.5];
        let vibon = p[VIBRATO] >= 0.5;
        let vol = db_to_gain(p[VOLUME] as f64);
        let mut peak = 0.0f64;
        for kk in 0..n {
            self.vib = (self.vib + 6.0 / fs).fract();
            let shift = self.bend + if vibon || self.modw > 0.0 { (std::f64::consts::TAU * self.vib).sin() * 0.18 * (if vibon { 1.0 } else { 0.0 } + self.modw) } else { 0.0 };
            self.bank.step(fs, shift);
            let mut sum = 0.0;
            for vc in self.voices.iter_mut() {
                if vc.aenv.idle() {
                    continue;
                }
                let e = vc.aenv.step(a, d, s, r);
                sum += divided(&self.bank, vc.note, fs, shift) * e * (0.5 + 0.5 * vc.vel);
            }
            let mut y = sum;
            for b in self.tone.iter_mut() {
                y = b.process(y);
            }
            for b in self.shelf.iter_mut() {
                y = b.process(y);
            }
            for (b, on) in self.res.iter_mut().zip(rs) {
                if on {
                    y = b.process(y);
                }
            }
            // the output valves
            let out = self.dc.process((y * 0.5).tanh() * 2.0) * vol * 0.6;
            peak = peak.max(out.abs());
            for c in chans.iter_mut() {
                c[kk] = out as f32;
            }
        }
        meters[0] = peak as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Swelling Strings", values: &[(ATTACK, 3.0), (SUSTAIN, 2.0), (TONE, 2.0), (RES2, 1.0), (VIBRATO, 1.0)] },
        Preset { name: "Hawaiian Guitar", values: &[(ATTACK, 0.0), (PERC, 2.0), (SUSTAIN, 2.0), (TONE, 3.0), (RES3, 1.0), (VIBRATO, 1.0)] },
        Preset { name: "Piano", values: &[(ATTACK, 0.0), (PERC, 2.0), (SUSTAIN, 1.0), (TONE, 1.0), (VIBRATO, 0.0)] },
        Preset { name: "Harp", values: &[(ATTACK, 0.0), (PERC, 3.0), (SUSTAIN, 3.0), (TONE, 0.0), (RES1, 1.0), (VIBRATO, 0.0)] },
        Preset { name: "Woodwind", values: &[(ATTACK, 2.0), (SUSTAIN, 0.0), (TONE, 0.0), (RES1, 1.0), (RES2, 1.0), (VIBRATO, 1.0)] },
        Preset { name: "Brilliant Brass", values: &[(ATTACK, 1.0), (SUSTAIN, 1.0), (TONE, 3.0), (BRIGHT, 1.0), (RES3, 1.0)] },
        Preset { name: "Deep Organ", values: &[(ATTACK, 1.0), (SUSTAIN, 0.0), (TONE, 4.0), (DEEP, 1.0), (VIBRATO, 0.0), (BALANCE, 0.35)] },
    ]
}

fn panel() -> Panel {
    let sw = |p, x, y, l: &'static str, tip| Item::Switch { p, x, y, labels: ["", l], tip };
    Panel {
        title: "HAMMOND NOVACHORD",
        subtitle: ("1939 · 163 VALVES", "72-NOTE POLYPHONIC · DIVIDE-DOWN · FORMANT RESONATORS"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::Selector { p: ATTACK, x: 60.0, y: 130.0, label: "ATTACK", positions: &["SHARP", "QUICK", "MEDIUM", "SLOW", "SWELL"], tip: "How each note begins." },
            Item::Selector { p: PERC, x: 170.0, y: 130.0, label: "PERCUSSION", positions: &["OFF", "SMALL", "MEDIUM", "LARGE"], tip: "Off: notes hold. On: they die away while held, like a struck string." },
            Item::Selector { p: SUSTAIN, x: 280.0, y: 130.0, label: "SUSTAIN", positions: &["OFF", "SHORT", "MEDIUM", "LONG"], tip: "How long a note rings on after the key is let go." },
            Item::Selector { p: TONE, x: 390.0, y: 130.0, label: "TONE", positions: &["MELLOW", "NORMAL", "FULL", "BRILLIANT", "DEEP"], tip: "The tone selector's formant filters." },
            Item::Knob { p: VOLUME, x: 540.0, y: 130.0, r: 20.0, label: "SWELL", tip: "Output level (the swell pedal).", active: None },
            Item::Divider { y: 205.0 },
            sw(RES1, 40.0, 250.0, "1ST RES", "The first resonator (around 530 Hz)."),
            sw(RES2, 130.0, 250.0, "2ND RES", "The second resonator (around 1.2 kHz)."),
            sw(RES3, 220.0, 250.0, "3RD RES", "The third resonator (around 2.7 kHz)."),
            sw(BRIGHT, 310.0, 250.0, "BRILLIANT", "Brilliant: more top."),
            sw(DEEP, 400.0, 250.0, "DEEP TONE", "Deep tone: more bass."),
            sw(VIBRATO, 490.0, 250.0, "VIBRATO", "The valve vibrato."),
            Item::Knob { p: BALANCE, x: 575.0, y: 250.0, r: 18.0, label: "BALANCER", tip: "Bass (left) against treble (right).", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Hammond Novachord", class_id: *b"HWHammondNovac39", vendor: "PEQUOD Hardware",
    instrument, categories: [Instrument, Synth]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        with(defaults(), &[(VIBRATO, 0.0), (TONE, 3.0), (SUSTAIN, 0.0)])
    }

    include!("../../instrument_tests.rs");
}
