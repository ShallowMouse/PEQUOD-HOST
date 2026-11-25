//! Roland Juno-60 (1982): six voices, each one DCO - a digitally clocked oscillator, so it stays in tune -
//! with saw, pulse (width by hand or by the LFO), a square sub-oscillator an octave down and noise; a non-resonant
//! high-pass in four steps; the IR3109 24 dB/octave low-pass with resonance up to
//! self-oscillation, swept by the envelope (either polarity), the LFO and the keys; one ADSR for filter and VCA
//! (or the VCA on a plain gate); a triangle LFO with a delay; and the chorus - one bucket brigade read with its
//! modulation one way for the left and the other for the right: I slow and wide, II faster,
//! hiss and all. The arpeggiator: up, up and down or down over one to three octaves, at its own rate.
//! After the owner's manual and service notes; chorus rates and depths from published measurements; the rest
//! estimated.

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

pub const W: u32 = 660;
pub const H: u32 = 460;

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "lfo"]
    pub lfo: FloatParam,
    #[id = "lfodelay"]
    pub lfodelay: FloatParam,
    #[id = "range"]
    pub range: FloatParam,
    #[id = "dcolfo"]
    pub dcolfo: FloatParam,
    #[id = "pw"]
    pub pw: FloatParam,
    #[id = "pwmode"]
    pub pwmode: FloatParam,
    #[id = "pulse"]
    pub pulse: FloatParam,
    #[id = "saw"]
    pub saw: FloatParam,
    #[id = "sub"]
    pub sub: FloatParam,
    #[id = "noise"]
    pub noise: FloatParam,
    #[id = "hpf"]
    pub hpf: FloatParam,
    #[id = "cutoff"]
    pub cutoff: FloatParam,
    #[id = "res"]
    pub res: FloatParam,
    #[id = "envpol"]
    pub envpol: FloatParam,
    #[id = "env"]
    pub env: FloatParam,
    #[id = "vcflfo"]
    pub vcflfo: FloatParam,
    #[id = "kybd"]
    pub kybd: FloatParam,
    #[id = "vcamode"]
    pub vcamode: FloatParam,
    #[id = "level"]
    pub level: FloatParam,
    #[id = "a"]
    pub a: FloatParam,
    #[id = "d"]
    pub d: FloatParam,
    #[id = "s"]
    pub s: FloatParam,
    #[id = "r"]
    pub r: FloatParam,
    #[id = "chorus"]
    pub chorus: FloatParam,
    #[id = "port"]
    pub port: FloatParam,
    #[id = "arp"]
    pub arp: FloatParam,
    #[id = "arpmode"]
    pub arpmode: FloatParam,
    #[id = "arprange"]
    pub arprange: FloatParam,
    #[id = "arprate"]
    pub arprate: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            lfo: hz("LFO Rate", 3.0, 0.1, 20.0),
            lfodelay: ms("LFO Delay", 0.0, 0.0, 3000.0),
            range: FloatParam::new("Range", 1.0, FloatRange::Linear { min: 0.0, max: 2.0 }).with_step_size(1.0),
            dcolfo: pct("DCO LFO", 0.0),
            pw: pct("PWM", 0.3),
            pwmode: FloatParam::new("PWM Mode", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            pulse: FloatParam::new("Pulse", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            saw: FloatParam::new("Saw", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            sub: pct("Sub Osc", 0.0),
            noise: pct("Noise", 0.0),
            hpf: FloatParam::new("HPF", 0.0, FloatRange::Linear { min: 0.0, max: 3.0 }).with_step_size(1.0),
            cutoff: pct("VCF Freq", 0.6),
            res: pct("Resonance", 0.1),
            envpol: FloatParam::new("Env Polarity", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            env: pct("VCF Env", 0.3),
            vcflfo: pct("VCF LFO", 0.0),
            kybd: pct("VCF Kybd", 0.5),
            vcamode: FloatParam::new("VCA Mode", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            level: pct("VCA Level", 0.7),
            a: FloatParam::new("Attack", 2.0, FloatRange::Skewed { min: 1.0, max: 3000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            d: FloatParam::new("Decay", 600.0, FloatRange::Skewed { min: 2.0, max: 12000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            s: pct("Sustain", 0.7),
            r: FloatParam::new("Release", 300.0, FloatRange::Skewed { min: 2.0, max: 12000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            chorus: FloatParam::new("Chorus", 1.0, FloatRange::Linear { min: 0.0, max: 2.0 }).with_step_size(1.0),
            port: pct("Portamento", 0.0),
            arp: FloatParam::new("Arpeggio", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            arpmode: FloatParam::new("Arp Mode", 0.0, FloatRange::Linear { min: 0.0, max: 2.0 }).with_step_size(1.0),
            arprange: FloatParam::new("Arp Range", 0.0, FloatRange::Linear { min: 0.0, max: 2.0 }).with_step_size(1.0),
            arprate: hz("Arp Rate", 6.0, 1.0, 30.0),
        }
    }
}

pub const LFO: usize = 0;
pub const LFODELAY: usize = 1;
pub const RANGE: usize = 2;
pub const DCOLFO: usize = 3;
pub const PW: usize = 4;
pub const PWMODE: usize = 5;
pub const PULSE: usize = 6;
pub const SAW: usize = 7;
pub const SUB: usize = 8;
pub const NOISE: usize = 9;
pub const HPF: usize = 10;
pub const CUTOFF: usize = 11;
pub const RES: usize = 12;
pub const ENVPOL: usize = 13;
pub const ENV: usize = 14;
pub const VCFLFO: usize = 15;
pub const KYBD: usize = 16;
pub const VCAMODE: usize = 17;
pub const LEVEL: usize = 18;
pub const A: usize = 19;
pub const D: usize = 20;
pub const S: usize = 21;
pub const R: usize = 22;
pub const CHORUS: usize = 23;
pub const PORT: usize = 24;
pub const ARP: usize = 25;
pub const ARPMODE: usize = 26;
pub const ARPRANGE: usize = 27;
pub const ARPRATE: usize = 28;
pub const NPARAMS: usize = 29;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.lfo.as_ptr(),
            self.lfodelay.as_ptr(),
            self.range.as_ptr(),
            self.dcolfo.as_ptr(),
            self.pw.as_ptr(),
            self.pwmode.as_ptr(),
            self.pulse.as_ptr(),
            self.saw.as_ptr(),
            self.sub.as_ptr(),
            self.noise.as_ptr(),
            self.hpf.as_ptr(),
            self.cutoff.as_ptr(),
            self.res.as_ptr(),
            self.envpol.as_ptr(),
            self.env.as_ptr(),
            self.vcflfo.as_ptr(),
            self.kybd.as_ptr(),
            self.vcamode.as_ptr(),
            self.level.as_ptr(),
            self.a.as_ptr(),
            self.d.as_ptr(),
            self.s.as_ptr(),
            self.r.as_ptr(),
            self.chorus.as_ptr(),
            self.port.as_ptr(),
            self.arp.as_ptr(),
            self.arpmode.as_ptr(),
            self.arprange.as_ptr(),
            self.arprate.as_ptr(),
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

const VOICES: usize = 6;
const SIXTY: bool = true;

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
    ph: f64,
    sub: bool,
    filt: Ladder,
    hp: OnePole,
    boost: Biquad,
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

/// The Juno chorus: one bucket brigade, read with the LFO one way (left) and the other (right).
struct Chorus {
    line: DelayLine,
    lp: [OnePole; 2],
    ph: f64,
    noise: Noise,
}
impl Chorus {
    fn new(fs: f64) -> Self {
        let mut lp = [OnePole::default(), OnePole::default()];
        lp.iter_mut().for_each(|l| l.set_lowpass(fs, 9000.0));
        Self { line: DelayLine::new((fs * 0.012) as usize), lp, ph: 0.0, noise: Noise::new(60106) }
    }
    fn reset(&mut self) {
        self.line.reset();
        self.lp.iter_mut().for_each(|l| l.reset());
    }
    /// mode 1: I (0.513 Hz, 1.66-5.35 ms), 2: II (0.863 Hz), 3: I+II (9.75 Hz, shallow)
    fn process(&mut self, x: f64, mode: usize, fs: f64) -> (f64, f64) {
        self.line.write(x);
        if mode == 0 {
            return (x, x);
        }
        let (rate, lo, hi) = match mode { 1 => (0.513, 1.66, 5.35), 2 => (0.863, 1.66, 5.35), _ => (9.75, 3.3, 3.7) };
        self.ph = (self.ph + rate / fs).fract();
        let tri = if self.ph < 0.5 { 4.0 * self.ph - 1.0 } else { 3.0 - 4.0 * self.ph };
        let mid = 0.5 * (lo + hi);
        let half = 0.5 * (hi - lo);
        let dl = (mid + half * tri) * 0.001 * fs;
        let dr = (mid - half * tri) * 0.001 * fs;
        let hiss = self.noise.next() * 0.0015;
        let wl = self.lp[0].lp(self.line.read(dl)) + hiss;
        let wr = self.lp[1].lp(self.line.read(dr)) - hiss;
        (0.5 * (x + wl) * 1.3, 0.5 * (x + wr) * 1.3)
    }
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    voices: [Voice; VOICES],
    mono: Vec<u8>,
    clock: u64,
    lfo: f64,
    lfo_age: f64,
    noise: Noise,
    chorus: Chorus,
    bend: f64,
    modw: f64,
    pedal: bool,
    // the arpeggiator (Juno-60)
    held: Vec<u8>,
    arp_ph: f64,
    arp_i: usize,
    arp_note: Option<u8>,
    dc: [DcBlock; 2],
}

impl Dsp {
    fn unison(&self) -> bool {
        false
    }
    fn arp_on(&self) -> bool {
        SIXTY && self.v[ARP] >= 0.5
    }
    fn key_on(&mut self, note: u8, vel: f32) {
        self.clock += 1;
        let i = alloc(&self.voices, note);
        let port = self.v[PORT] > 0.0;
        let vc = &mut self.voices[i];
        if vc.idle() || !port {
            vc.pitch = note as f64;
        }
        vc.note = note;
        vc.held = true;
        vc.sus = false;
        vc.vel = vel as f64;
        vc.age = self.clock;
        vc.aenv.on();
        if self.voices.iter().all(|v| v.idle() || v.note == note) {
            self.lfo_age = 0.0;
        }
    }
    fn key_off(&mut self, note: u8) {
        let pedal = self.pedal;
        for v in self.voices.iter_mut().filter(|v| v.held && v.note == note) {
            v.held = false;
            if pedal { v.sus = true; } else { v.aenv.off(); }
        }
    }
    /// The arpeggio's notes, in order, over its range.
    fn arp_notes(&self) -> Vec<u8> {
        let mut base = self.held.clone();
        base.sort_unstable();
        let octs = (self.v[ARPRANGE].round().max(0.0) as usize).min(2) + 1;
        let mut up: Vec<u8> = (0..octs).flat_map(|o| base.iter().map(move |n| n.saturating_add(12 * o as u8))).collect();
        match (self.v[ARPMODE].round().max(0.0) as usize).min(2) {
            0 => up,
            1 => {
                let mut down: Vec<u8> = up.iter().rev().skip(1).cloned().collect();
                down.pop();
                up.extend(down);
                up
            }
            _ => { up.reverse(); up }
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut voices = [Voice::default(); VOICES];
        for (i, vc) in voices.iter_mut().enumerate() {
            vc.ph = 0.17 * i as f64;
            vc.boost = Biquad::identity();
        }
        Self { fs, v, voices, mono: Vec::new(), clock: 0, lfo: 0.0, lfo_age: 0.0, noise: Noise::new(1982), chorus: Chorus::new(fs), bend: 0.0, modw: 0.0, pedal: false,
               held: Vec::new(), arp_ph: 0.0, arp_i: 0, arp_note: None, dc: [DcBlock::new(fs), DcBlock::new(fs)] }
    }
    fn reset(&mut self) {
        for v in self.voices.iter_mut() {
            v.aenv = Adsr::default();
            v.filt = Ladder::default();
            v.held = false;
        }
        self.chorus.reset();
        self.held.clear();
        self.arp_note = None;
        self.dc.iter_mut().for_each(|d| d.reset());
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn midi(&mut self, ev: Midi) {
        match ev {
            Midi::On { note, vel } => {
                if self.arp_on() {
                    if self.held.is_empty() {
                        self.arp_ph = 1.0;
                        self.arp_i = 0;
                    }
                    if !self.held.contains(&note) { self.held.push(note); }
                } else {
                    self.key_on(note, vel);
                }
            }
            Midi::Off { note } => {
                self.held.retain(|n| *n != note);
                if !self.arp_on() { self.key_off(note); }
            }
            Midi::Bend { value } => self.bend = value as f64 * 2.0,
            Midi::Cc { cc: 1, value } => self.modw = value as f64,
            Midi::Cc { cc: 64, value } => {
                self.pedal = value >= 0.5;
                if !self.pedal {
                    for v in self.voices.iter_mut().filter(|v| v.sus) {
                        v.sus = false;
                        v.aenv.off();
                    }
                }
            }
            Midi::AllOff => {
                self.held.clear();
                self.arp_note = None;
                for v in self.voices.iter_mut() {
                    v.held = false;
                    v.sus = false;
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
        let range = [-1.0, 0.0, 1.0][st(RANGE, 2)];
        let (a, d, s, r) = (rate(fs, p[A] as f64), rate(fs, p[D] as f64), p[S] as f64, rate(fs, p[R] as f64));
        let gate_vca = p[VCAMODE] >= 0.5;
        let k = 4.2 * p[RES] as f64;
        let hpf_pos = st(HPF, 3);
        let hp_hz: f64 = if SIXTY { [0.0, 240.0, 720.0, 1500.0][hpf_pos] } else { [0.0, 0.0, 240.0, 720.0][hpf_pos] };
        for vc in self.voices.iter_mut() {
            vc.hp.set_lowpass(fs, hp_hz.max(1.0));
            if !SIXTY && hpf_pos == 0 { vc.boost.set_low_shelf(fs, 90.0, 0.7, 4.5); } else { vc.boost = Biquad::identity(); }
        }
        let pol = if p[ENVPOL] >= 0.5 { -1.0 } else { 1.0 };
        let pwmode = st(PWMODE, 2);
        let chorus = st(CHORUS, 3);
        let level = p[LEVEL] as f64;
        let gc = if p[PORT] > 0.0 { coeff(fs, 2.0 + 1500.0 * (p[PORT] as f64).powi(2)) } else { 0.0 };
        let dly = (p[LFODELAY] as f64 / 1000.0).max(0.001);
        let (saw_on, pulse_on) = (p[SAW] >= 0.5, p[PULSE] >= 0.5);
        let mut peak = 0.0f64;
        for kk in 0..n {
            // the arpeggiator
            if self.arp_on() {
                self.arp_ph += p[ARPRATE] as f64 / fs;
                if self.arp_ph >= 1.0 {
                    self.arp_ph -= 1.0;
                    if let Some(old) = self.arp_note.take() { self.key_off(old); }
                    let notes = self.arp_notes();
                    if !notes.is_empty() {
                        let nn = notes[self.arp_i % notes.len()];
                        self.arp_i = (self.arp_i + 1) % notes.len();
                        self.key_on(nn, 0.8);
                        self.arp_note = Some(nn);
                    }
                }
                if self.held.is_empty() {
                    if let Some(old) = self.arp_note.take() { self.key_off(old); }
                }
            } else if let Some(old) = self.arp_note.take() {
                self.key_off(old);
            }
            self.lfo = (self.lfo + p[LFO] as f64 / fs).fract();
            self.lfo_age += 1.0 / fs;
            let tri = if self.lfo < 0.5 { 4.0 * self.lfo - 1.0 } else { 3.0 - 4.0 * self.lfo };
            let lv = tri * (self.lfo_age / dly).min(1.0);
            let mut sum = 0.0;
            for vc in self.voices.iter_mut() {
                if vc.aenv.idle() {
                    continue;
                }
                let target = vc.note as f64;
                vc.pitch = target + gc * (vc.pitch - target);
                let env = vc.aenv.step(a, d, s, r);
                let nb = vc.pitch + self.bend + 12.0 * range + lv * (p[DCOLFO] as f64 + 0.3 * self.modw) * 1.5;
                let f = mtof(nb);
                let dt = (f / fs).min(0.45);
                // the DCO: a clean ramp, the pulse from it, the sub flip-flop
                let pwv = match pwmode {
                    0 => 0.5 - 0.45 * p[PW] as f64 * (0.5 + 0.5 * lv),
                    1 => 0.5 - 0.45 * p[PW] as f64,
                    _ => 0.5 - 0.45 * p[PW] as f64 * env,
                }.clamp(0.05, 0.95);
                let mut x = 0.0;
                if saw_on { x += saw(vc.ph, dt); }
                if pulse_on { x += pulse(vc.ph, dt, pwv); }
                x += p[SUB] as f64 * if vc.sub { 1.0 } else { -1.0 };
                x += p[NOISE] as f64 * self.noise.next() * 0.8;
                vc.ph += dt;
                if vc.ph >= 1.0 {
                    vc.ph -= 1.0;
                    vc.sub = !vc.sub;
                }
                let x = vc.boost.process(if hp_hz > 0.0 { vc.hp.hp(x) } else { x }) * 0.45;
                let fc = 20.0 * 2f64.powf(10.0 * p[CUTOFF] as f64 + 5.0 * pol * p[ENV] as f64 * env + 2.0 * p[VCFLFO] as f64 * lv
                                              + p[KYBD] as f64 * (vc.pitch - 60.0) / 12.0);
                let (lp, _) = vc.filt.process(x, fc, fs, k, false);
                let amp = if gate_vca { if vc.held || vc.sus { 1.0 } else { env.min(1.0) * 0.0 + if env > 0.001 { 1.0 } else { 0.0 } } } else { env };
                sum += lp * (1.0 + 0.4 * k) * amp * level;
            }
            let (l, rr) = self.chorus.process(sum, chorus, fs);
            let (l, rr) = (self.dc[0].process(l) * 0.9, self.dc[1].process(rr) * 0.9);
            peak = peak.max(l.abs()).max(rr.abs());
            let nch = chans.len();
            for (c, ch) in chans.iter_mut().enumerate() {
                ch[kk] = if nch == 1 { (0.5 * (l + rr)) as f32 } else if c == 0 { l as f32 } else { rr as f32 };
            }
        }
        meters[0] = peak as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Brass", values: &[(SAW, 1.0), (PULSE, 0.0), (CUTOFF, 0.35), (ENV, 0.45), (A, 40.0), (D, 700.0), (S, 0.6), (R, 250.0), (CHORUS, 0.0)] },
        Preset { name: "Strings", values: &[(SAW, 1.0), (PULSE, 0.0), (CUTOFF, 0.62), (ENV, 0.1), (A, 450.0), (D, 1200.0), (S, 0.9), (R, 1200.0), (CHORUS, 2.0), (DCOLFO, 0.04), (LFO, 5.5), (LFODELAY, 600.0)] },
        Preset { name: "Juno Bass", values: &[(SAW, 1.0), (PULSE, 1.0), (PW, 0.25), (SUB, 0.8), (RANGE, 0.0), (CUTOFF, 0.38), (RES, 0.25), (ENV, 0.4), (D, 300.0), (S, 0.3), (R, 120.0), (CHORUS, 0.0)] },
        Preset { name: "PWM Pad", values: &[(SAW, 0.0), (PULSE, 1.0), (PWMODE, 0.0), (PW, 0.7), (LFO, 0.8), (CUTOFF, 0.6), (A, 300.0), (R, 1500.0), (CHORUS, 1.0)] },
        Preset { name: "Hoover-ish", values: &[(SAW, 1.0), (PULSE, 1.0), (PWMODE, 0.0), (PW, 0.9), (LFO, 6.0), (SUB, 0.6), (CUTOFF, 0.75), (CHORUS, 2.0), (PORT, 0.15)] },
        Preset { name: "Pluck", values: &[(SAW, 1.0), (PULSE, 0.0), (CUTOFF, 0.3), (RES, 0.35), (ENV, 0.55), (D, 250.0), (S, 0.0), (R, 250.0), (CHORUS, 1.0)] },
        Preset { name: "Resonant Sweep", values: &[(SAW, 1.0), (CUTOFF, 0.2), (RES, 0.75), (ENV, 0.6), (A, 900.0), (D, 2500.0), (S, 0.3), (R, 1200.0), (CHORUS, 1.0)] },
        Preset { name: "Arp Pulses", values: &[(SAW, 0.0), (PULSE, 1.0), (PW, 0.35), (CUTOFF, 0.45), (ENV, 0.4), (D, 180.0), (S, 0.0), (R, 150.0), (ARP, 1.0), (ARPMODE, 1.0), (ARPRANGE, 1.0), (ARPRATE, 8.0), (CHORUS, 1.0)] },
    ]
}

fn panel() -> Panel {
    let k = |p, x, y, r, label, tip| Item::Knob { p, x, y, r, label, tip, active: None };
    let sw = |p, x, y, l: &'static str, tip| Item::Switch { p, x, y, labels: ["", l], tip };
    Panel {
        title: "JUNO-60",
        subtitle: ("SIX VOICE POLYPHONIC, 1982", "DCO · SUB · IR3109 24 dB VCF · ADSR · CHORUS · ARP"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            k(LFO, 45.0, 125.0, 15.0, "LFO RATE", "The LFO (a triangle)."),
            k(LFODELAY, 105.0, 125.0, 15.0, "DELAY", "The LFO fades in after the first key."),
            Item::Selector { p: RANGE, x: 175.0, y: 130.0, label: "RANGE", positions: &["16'", "8'", "4'"], tip: "The DCO's octave." },
            k(DCOLFO, 245.0, 125.0, 15.0, "DCO LFO", "Vibrato (the mod lever adds more)."),
            k(PW, 300.0, 125.0, 15.0, "PWM", "Pulse width."),
            Item::Selector { p: PWMODE, x: 365.0, y: 130.0, label: "PWM MODE", positions: &["LFO", "MAN"], tip: "What moves the pulse width." },
            sw(PULSE, 430.0, 112.0, "PULSE", "The pulse wave."),
            sw(SAW, 430.0, 142.0, "SAW", "The saw wave."),
            k(SUB, 525.0, 125.0, 15.0, "SUB", "The square sub-oscillator, an octave down."),
            k(NOISE, 580.0, 125.0, 15.0, "NOISE", "Noise."),
            Item::Divider { y: 185.0 },
            Item::Selector { p: HPF, x: 45.0, y: 252.0, label: "HPF", positions: &["0", "1", "2", "3"], tip: "The high-pass: 0 flat, 1 to 3 thinner." },
            k(CUTOFF, 125.0, 245.0, 19.0, "FREQ", "The low-pass cutoff."),
            k(RES, 190.0, 245.0, 15.0, "RES", "Resonance: up high, it sings."),
            Item::Switch { p: ENVPOL, x: 255.0, y: 245.0, labels: ["+", "-"], tip: "The envelope's polarity at the filter." },
            k(ENV, 320.0, 245.0, 15.0, "ENV", "How far the envelope sweeps the filter."),
            k(VCFLFO, 375.0, 245.0, 15.0, "LFO", "LFO to the filter."),
            k(KYBD, 430.0, 245.0, 15.0, "KYBD", "Keyboard follow."),
            Item::Switch { p: VCAMODE, x: 500.0, y: 245.0, labels: ["ENV", "GATE"], tip: "The VCA on the envelope or on a plain gate." },
            k(LEVEL, 585.0, 245.0, 15.0, "LEVEL", "The VCA's level."),
            Item::Divider { y: 300.0 },
            k(A, 45.0, 355.0, 15.0, "A", "Attack."),
            k(D, 100.0, 355.0, 15.0, "D", "Decay."),
            k(S, 155.0, 355.0, 15.0, "S", "Sustain."),
            k(R, 210.0, 355.0, 15.0, "R", "Release."),
            k(PORT, 265.0, 355.0, 15.0, "PORTA", "Portamento."),
            Item::Selector { p: CHORUS, x: 335.0, y: 362.0, label: "CHORUS", positions: &["OFF", "I", "II"], tip: "The chorus." },
            Item::Switch { p: ARP, x: 405.0, y: 355.0, labels: ["", "ARP"], tip: "The arpeggiator: hold a chord." },
            Item::Selector { p: ARPMODE, x: 472.0, y: 362.0, label: "MODE", positions: &["UP", "U&D", "DOWN"], tip: "Arpeggio direction." },
            Item::Selector { p: ARPRANGE, x: 566.0, y: 362.0, label: "RANGE", positions: &["1", "2", "3"], tip: "Arpeggio range in octaves." },
            k(ARPRATE, 626.0, 355.0, 15.0, "RATE", "Arpeggio rate."),
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Roland Juno-60", class_id: *b"HWRolandJuno60PQ", vendor: "PEQUOD Hardware",
    instrument, categories: [Instrument, Synth]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        with(defaults(), &[(PULSE, 0.0), (SUB, 0.0), (NOISE, 0.0), (CUTOFF, 1.0), (RES, 0.0), (ENV, 0.0), (KYBD, 0.0), (S, 1.0), (CHORUS, 0.0), (HPF, 0.0)])
    }

    /// Six notes at once, and the ninth takes the oldest voice.
    #[test]
    fn eight_voices() {
        let mut ev = vec![];
        for (i, n) in [48u8, 52, 55, 59, 62, 65, 69].iter().enumerate() {
            ev.push((i * 100, Midi::On { note: *n, vel: 0.8 }));
        }
        let (l, _) = render(&tune_params(), &ev, 24000);
        assert!(l.iter().all(|v| v.is_finite()) && rms(&l[4000..]) > 1e-3);
    }

    include!("../../instrument_tests.rs");
}
