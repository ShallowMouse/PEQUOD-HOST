//! Moog Polymoog 203a (1975): fully polyphonic by dividing down - two banks of top-octave oscillators
//! (OSC 1 and OSC 2, the second detuned against the first by BEAT), every one of the 71 keys a division
//! of them with its own filter and loudness contour, and the keyboard velocity-sensitive. Each bank
//! gives saw or pulse (with PWM); then the RESONATORS - three fixed-band filters, low, mid and high,
//! with their own frequency, width and level - colour the whole instrument. Its eight preset modes
//! (strings, piano, organ, harpsichord, funk, vibes, brass and the vox humana Gary Numan made famous on
//! Cars) are the presets here. Divide-down banks are phase-locked: every C is in step with every other
//! C, the thing that makes a Polymoog chord sound as it does. Details estimated.

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

pub const W: u32 = 640;
pub const H: u32 = 450;

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "w1"]
    pub w1: FloatParam,
    #[id = "w2"]
    pub w2: FloatParam,
    #[id = "l1"]
    pub l1: FloatParam,
    #[id = "l2"]
    pub l2: FloatParam,
    #[id = "oct2"]
    pub oct2: FloatParam,
    #[id = "beat"]
    pub beat: FloatParam,
    #[id = "pw"]
    pub pw: FloatParam,
    #[id = "lfo"]
    pub lfo: FloatParam,
    #[id = "pwm"]
    pub pwm: FloatParam,
    #[id = "vib"]
    pub vib: FloatParam,
    #[id = "cutoff"]
    pub cutoff: FloatParam,
    #[id = "emph"]
    pub emph: FloatParam,
    #[id = "contour"]
    pub contour: FloatParam,
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
    #[id = "rlo"]
    pub rlo: FloatParam,
    #[id = "rmid"]
    pub rmid: FloatParam,
    #[id = "rhi"]
    pub rhi: FloatParam,
    #[id = "rlof"]
    pub rlof: FloatParam,
    #[id = "rmidf"]
    pub rmidf: FloatParam,
    #[id = "rhif"]
    pub rhif: FloatParam,
    #[id = "rq"]
    pub rq: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            w1: FloatParam::new("Osc 1 Wave", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            w2: FloatParam::new("Osc 2 Wave", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            l1: pct("Osc 1 Level", 0.8),
            l2: pct("Osc 2 Level", 0.6),
            oct2: FloatParam::new("Osc 2 Octave", 1.0, FloatRange::Linear { min: 0.0, max: 2.0 }).with_step_size(1.0),
            beat: pct("Beat", 0.2),
            pw: pct("Pulse Width", 0.3),
            lfo: hz("Mod Rate", 5.0, 0.1, 20.0),
            pwm: pct("Mod to PW", 0.0),
            vib: pct("Mod to Pitch", 0.0),
            cutoff: hz("Filter", 2000.0, 40.0, 16000.0),
            emph: pct("Emphasis", 0.1),
            contour: pct("Contour Amount", 0.3),
            a: FloatParam::new("Attack", 10.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            d: FloatParam::new("Decay", 600.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            s: pct("Sustain", 0.8),
            r: FloatParam::new("Release", 400.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            vel: pct("Velocity", 0.5),
            rlo: pct("Resonator Low", 0.0),
            rmid: pct("Resonator Mid", 0.0),
            rhi: pct("Resonator High", 0.0),
            rlof: hz("Low Frequency", 250.0, 80.0, 600.0),
            rmidf: hz("Mid Frequency", 1000.0, 400.0, 2500.0),
            rhif: hz("High Frequency", 3500.0, 1500.0, 8000.0),
            rq: pct("Resonator Width", 0.5),
            volume: db("Volume", -10.0, -30.0, 6.0),
        }
    }
}

pub const W1: usize = 0;
pub const W2: usize = 1;
pub const L1: usize = 2;
pub const L2: usize = 3;
pub const OCT2: usize = 4;
pub const BEAT: usize = 5;
pub const PW: usize = 6;
pub const LFO: usize = 7;
pub const PWM: usize = 8;
pub const VIB: usize = 9;
pub const CUTOFF: usize = 10;
pub const EMPH: usize = 11;
pub const CONTOUR: usize = 12;
pub const A: usize = 13;
pub const D: usize = 14;
pub const S: usize = 15;
pub const R: usize = 16;
pub const VEL: usize = 17;
pub const RLO: usize = 18;
pub const RMID: usize = 19;
pub const RHI: usize = 20;
pub const RLOF: usize = 21;
pub const RMIDF: usize = 22;
pub const RHIF: usize = 23;
pub const RQ: usize = 24;
pub const VOLUME: usize = 25;
pub const NPARAMS: usize = 26;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.w1.as_ptr(),
            self.w2.as_ptr(),
            self.l1.as_ptr(),
            self.l2.as_ptr(),
            self.oct2.as_ptr(),
            self.beat.as_ptr(),
            self.pw.as_ptr(),
            self.lfo.as_ptr(),
            self.pwm.as_ptr(),
            self.vib.as_ptr(),
            self.cutoff.as_ptr(),
            self.emph.as_ptr(),
            self.contour.as_ptr(),
            self.a.as_ptr(),
            self.d.as_ptr(),
            self.s.as_ptr(),
            self.r.as_ptr(),
            self.vel.as_ptr(),
            self.rlo.as_ptr(),
            self.rmid.as_ptr(),
            self.rhi.as_ptr(),
            self.rlof.as_ptr(),
            self.rmidf.as_ptr(),
            self.rhif.as_ptr(),
            self.rq.as_ptr(),
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

const VOICES: usize = 24;

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
    filt: Ladder,
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
    bank: [Bank; 2],
    lfo: f64,
    res: [Biquad; 3],
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
        let mut b2 = Bank::new();
        b2.cycles.iter_mut().for_each(|c| *c += 0.5);
        Self { fs, v, voices: [Voice::default(); VOICES], mono: Vec::new(), clock: 0, bank: [Bank::new(), b2], lfo: 0.0, res: [Biquad::identity(), Biquad::identity(), Biquad::identity()], bend: 0.0, modw: 0.0, pedal: false, dc: DcBlock::new(fs) }
    }
    fn reset(&mut self) {
        for v in self.voices.iter_mut() {
            v.fenv = Adsr::default();
            v.aenv = Adsr::default();
            v.filt = Ladder::default();
            v.held = false;
        }
        self.res.iter_mut().for_each(|b| b.reset());
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
        let (a, d, s, r) = (rate(fs, p[A] as f64), rate(fs, p[D] as f64), p[S] as f64, rate(fs, p[R] as f64));
        let q = 0.7 + 6.0 * (1.0 - p[RQ] as f64);
        self.res[0].set_bandpass(fs, p[RLOF] as f64, q);
        self.res[1].set_bandpass(fs, p[RMIDF] as f64, q);
        self.res[2].set_bandpass(fs, p[RHIF] as f64, q);
        let rl = [p[RLO] as f64, p[RMID] as f64, p[RHI] as f64];
        let rsum: f64 = rl.iter().sum();
        let (sq1, sq2) = (p[W1] >= 0.5, p[W2] >= 0.5);
        let oct2 = [-12.0, 0.0, 12.0][(p[OCT2].round().max(0.0) as usize).min(2)];
        let beat = p[BEAT] as f64 * 0.25;
        let k = 3.5 * p[EMPH] as f64;
        let vol = db_to_gain(p[VOLUME] as f64);
        let mut peak = 0.0f64;
        for kk in 0..n {
            self.lfo = (self.lfo + p[LFO] as f64 / fs).fract();
            let lv = (std::f64::consts::TAU * self.lfo).sin();
            let vib = lv * (p[VIB] as f64 + 0.5 * self.modw) * 0.5;
            let sh1 = self.bend + vib;
            let sh2 = self.bend + vib + beat;
            self.bank[0].step(fs, sh1);
            self.bank[1].step(fs, sh2);
            let pw = (0.5 - 0.45 * p[PW] as f64 + 0.35 * lv * p[PWM] as f64).clamp(0.05, 0.95);
            let mut sum = 0.0;
            for vc in self.voices.iter_mut() {
                if vc.aenv.idle() {
                    continue;
                }
                let (ph1, dt1) = self.bank[0].key(vc.note, fs, sh1);
                let n2 = (vc.note as f64 + oct2).clamp(0.0, 119.0) as u8;
                let (ph2, dt2) = self.bank[1].key(n2, fs, sh2);
                let o1 = if sq1 { pulse(ph1, dt1.min(0.45), pw) } else { saw(ph1, dt1.min(0.45)) };
                let o2 = if sq2 { pulse(ph2, dt2.min(0.45), pw) } else { saw(ph2, dt2.min(0.45)) };
                let ae = vc.aenv.step(a, d, s, r);
                let vs = 1.0 - p[VEL] as f64 * (1.0 - vc.vel);
                let fc = p[CUTOFF] as f64 * 2f64.powf((vc.note as f64 - 60.0) / 12.0 * 0.7 + 4.0 * p[CONTOUR] as f64 * ae * vs);
                let x = (o1 * p[L1] as f64 + o2 * p[L2] as f64) * 0.5;
                let y = vc.filt.process(x, fc, fs, k, false).0;
                sum += y * ae * vs;
            }
            let mut out = sum * (1.0 - 0.6 * rsum.min(1.0));
            for (b, l) in self.res.iter_mut().zip(rl) {
                out += b.process(sum) * l * 2.0;
            }
            let y = self.dc.process(out) * vol * 0.9;
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
        Preset { name: "Vox Humana", values: &[(W1, 1.0), (W2, 1.0), (PW, 0.35), (PWM, 0.6), (LFO, 5.5), (BEAT, 0.35), (L2, 0.8), (CUTOFF, 1800.0), (CONTOUR, 0.15), (A, 60.0), (R, 700.0), (RMID, 0.6), (RLO, 0.3), (RMIDF, 900.0)] },
        Preset { name: "Strings", values: &[(W1, 0.0), (W2, 0.0), (BEAT, 0.4), (CUTOFF, 3500.0), (A, 250.0), (R, 900.0), (VIB, 0.08), (LFO, 5.8), (RHI, 0.3)] },
        Preset { name: "Piano", values: &[(W1, 0.0), (W2, 1.0), (PW, 0.2), (CUTOFF, 1200.0), (CONTOUR, 0.6), (A, 2.0), (D, 1500.0), (S, 0.0), (R, 400.0), (VEL, 0.9)] },
        Preset { name: "Organ", values: &[(W1, 1.0), (W2, 1.0), (PW, 0.0), (OCT2, 2.0), (BEAT, 0.0), (CUTOFF, 5000.0), (CONTOUR, 0.0), (A, 3.0), (S, 1.0), (R, 30.0), (VEL, 0.0)] },
        Preset { name: "Harpsichord", values: &[(W1, 1.0), (W2, 0.0), (PW, 0.75), (OCT2, 2.0), (CUTOFF, 2500.0), (CONTOUR, 0.5), (A, 1.0), (D, 900.0), (S, 0.0), (R, 200.0), (RHI, 0.4)] },
        Preset { name: "Funk", values: &[(W1, 1.0), (W2, 0.0), (PW, 0.5), (CUTOFF, 400.0), (EMPH, 0.55), (CONTOUR, 0.7), (A, 1.0), (D, 250.0), (S, 0.2), (R, 100.0), (VEL, 0.8)] },
        Preset { name: "Vibes", values: &[(W1, 1.0), (W2, 1.0), (PW, 0.0), (OCT2, 2.0), (CUTOFF, 2500.0), (A, 2.0), (D, 2000.0), (S, 0.0), (R, 1500.0), (PWM, 0.0), (VIB, 0.0), (LFO, 6.0), (RMID, 0.3)] },
        Preset { name: "Brass", values: &[(W1, 0.0), (W2, 0.0), (BEAT, 0.2), (CUTOFF, 600.0), (CONTOUR, 0.6), (A, 80.0), (D, 700.0), (S, 0.6), (RLO, 0.3)] },
    ]
}

fn panel() -> Panel {
    let k = |p, x, y, r, label, tip| Item::Knob { p, x, y, r, label, tip, active: None };
    Panel {
        title: "MOOG POLYMOOG",
        subtitle: ("MODEL 203a, 1975", "DIVIDE-DOWN · 71 KEYS · VELOCITY · THREE RESONATORS"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::Switch { p: W1, x: 50.0, y: 120.0, labels: ["SAW", "PULSE"], tip: "Oscillator bank 1 wave." },
            k(L1, 120.0, 120.0, 16.0, "OSC 1", "Bank 1 level."),
            Item::Switch { p: W2, x: 190.0, y: 120.0, labels: ["SAW", "PULSE"], tip: "Oscillator bank 2 wave." },
            Item::Selector { p: OCT2, x: 260.0, y: 120.0, label: "OSC 2", positions: &["16'", "8'", "4'"], tip: "Bank 2's octave." },
            k(L2, 330.0, 120.0, 16.0, "LEVEL", "Bank 2 level."),
            k(BEAT, 390.0, 120.0, 16.0, "BEAT", "Bank 2 detuned against bank 1: the chorus beat."),
            k(PW, 450.0, 120.0, 16.0, "PW", "Pulse width."),
            k(VOLUME, 570.0, 120.0, 20.0, "VOLUME", "Output level."),
            Item::Divider { y: 165.0 },
            k(LFO, 50.0, 215.0, 16.0, "MOD RATE", "Modulation rate."),
            k(PWM, 110.0, 215.0, 16.0, "PWM", "Modulation to the pulse width."),
            k(VIB, 170.0, 215.0, 16.0, "VIBRATO", "Modulation to the pitch (the mod wheel adds more)."),
            k(CUTOFF, 260.0, 215.0, 22.0, "FILTER", "Every key's filter."),
            k(EMPH, 330.0, 215.0, 16.0, "EMPHASIS", "Filter resonance."),
            k(CONTOUR, 390.0, 215.0, 16.0, "CONTOUR", "How far each key's contour opens its filter."),
            k(VEL, 450.0, 215.0, 16.0, "VELOCITY", "How much the keyboard's velocity sets loudness and brightness."),
            Item::Divider { y: 260.0 },
            k(A, 60.0, 305.0, 15.0, "ATTACK", "Each key's attack."),
            k(D, 120.0, 305.0, 15.0, "DECAY", "Decay."),
            k(S, 180.0, 305.0, 15.0, "SUSTAIN", "Sustain."),
            k(R, 240.0, 305.0, 15.0, "RELEASE", "Release."),
            Item::Divider { y: 345.0 },
            Item::Text { x: 60.0, y: 370.0, text: "RESONATORS", size: 10.0, align: Align2::CENTER_CENTER },
            k(RLO, 60.0, 410.0, 15.0, "LOW", "Low resonator level."),
            k(RLOF, 120.0, 410.0, 15.0, "FREQ", "Low resonator frequency."),
            k(RMID, 200.0, 410.0, 15.0, "MID", "Mid resonator level."),
            k(RMIDF, 260.0, 410.0, 15.0, "FREQ", "Mid resonator frequency."),
            k(RHI, 340.0, 410.0, 15.0, "HIGH", "High resonator level."),
            k(RHIF, 400.0, 410.0, 15.0, "FREQ", "High resonator frequency."),
            k(RQ, 480.0, 410.0, 15.0, "WIDTH", "Resonator bandwidth."),
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Moog Polymoog", class_id: *b"HWMoogPolymo203a", vendor: "PEQUOD Hardware",
    instrument, categories: [Instrument, Synth]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        with(defaults(), &[(L2, 0.0), (CUTOFF, 16000.0), (EMPH, 0.0), (CONTOUR, 0.0), (S, 1.0)])
    }
    /// Divide-down: two A's an octave apart are locked in phase.
    #[test]
    fn octaves_locked() {
        let d = Dsp::new(48000.0, 1);
        let (p1, _) = d.bank[0].key(57, 48000.0, 0.0);
        let (p2, _) = d.bank[0].key(69, 48000.0, 0.0);
        assert!(((2.0 * p1).fract() - p2).abs() < 1e-9);
    }

    include!("../../instrument_tests.rs");
}
