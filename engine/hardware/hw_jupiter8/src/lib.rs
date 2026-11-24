//! Roland Jupiter-8 (1981): Roland's eight-voice flagship. Per voice: VCO-1 (triangle, saw, pulse,
//! square) and VCO-2 (sine, saw, pulse, noise) with ranges from 64' to 2', VCO-2 fine tune and a LOW
//! FREQ mode, CROSS MOD (VCO-2 frequency-modulating VCO-1) and SYNC (VCO-1 resetting VCO-2), a
//! SOURCE MIX knob between them, pulse width with PWM by the LFO or ENV-1, the non-resonant HPF, the
//! IR3109 VCF switchable 12 or 24 dB/octave, and two ADSRs - ENV-1 for the filter (and pitch), ENV-2
//! for the VCA. One LFO (sine, saw, square, random) with a delay, to the VCOs and the VCF. Very stable
//! voices, as Roland's were. Panel and ranges from the owner's manual; curves estimated.

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
pub const H: u32 = 515;

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "port"]
    pub port: FloatParam,
    #[id = "unison"]
    pub unison: FloatParam,
    #[id = "lfo"]
    pub lfo: FloatParam,
    #[id = "lfodelay"]
    pub lfodelay: FloatParam,
    #[id = "lfow"]
    pub lfow: FloatParam,
    #[id = "vcomod"]
    pub vcomod: FloatParam,
    #[id = "vcoenv"]
    pub vcoenv: FloatParam,
    #[id = "xmod"]
    pub xmod: FloatParam,
    #[id = "sync"]
    pub sync: FloatParam,
    #[id = "r1"]
    pub r1: FloatParam,
    #[id = "w1"]
    pub w1: FloatParam,
    #[id = "r2"]
    pub r2: FloatParam,
    #[id = "w2"]
    pub w2: FloatParam,
    #[id = "fine2"]
    pub fine2: FloatParam,
    #[id = "lowf"]
    pub lowf: FloatParam,
    #[id = "pw"]
    pub pw: FloatParam,
    #[id = "pwm"]
    pub pwm: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "hpf"]
    pub hpf: FloatParam,
    #[id = "cutoff"]
    pub cutoff: FloatParam,
    #[id = "res"]
    pub res: FloatParam,
    #[id = "slope"]
    pub slope: FloatParam,
    #[id = "fenvamt"]
    pub fenvamt: FloatParam,
    #[id = "flfo"]
    pub flfo: FloatParam,
    #[id = "fkbd"]
    pub fkbd: FloatParam,
    #[id = "fa"]
    pub fa: FloatParam,
    #[id = "fd"]
    pub fd: FloatParam,
    #[id = "fs"]
    pub fs: FloatParam,
    #[id = "fr"]
    pub fr: FloatParam,
    #[id = "a"]
    pub a: FloatParam,
    #[id = "d"]
    pub d: FloatParam,
    #[id = "s"]
    pub s: FloatParam,
    #[id = "r"]
    pub r: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            port: pct("Portamento", 0.0),
            unison: FloatParam::new("Unison", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            lfo: hz("LFO Rate", 5.0, 0.1, 30.0),
            lfodelay: ms("LFO Delay", 0.0, 0.0, 5000.0),
            lfow: FloatParam::new("LFO Wave", 0.0, FloatRange::Linear { min: 0.0, max: 3.0 }).with_step_size(1.0),
            vcomod: pct("VCO Mod LFO", 0.0),
            vcoenv: pct("VCO Mod ENV-1", 0.0),
            xmod: pct("Cross Mod", 0.0),
            sync: FloatParam::new("Sync", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            r1: FloatParam::new("VCO-1 Range", 3.0, FloatRange::Linear { min: 0.0, max: 5.0 }).with_step_size(1.0),
            w1: FloatParam::new("VCO-1 Wave", 1.0, FloatRange::Linear { min: 0.0, max: 3.0 }).with_step_size(1.0),
            r2: FloatParam::new("VCO-2 Range", 3.0, FloatRange::Linear { min: 0.0, max: 5.0 }).with_step_size(1.0),
            w2: FloatParam::new("VCO-2 Wave", 1.0, FloatRange::Linear { min: 0.0, max: 3.0 }).with_step_size(1.0),
            fine2: FloatParam::new("VCO-2 Fine Tune", 0.1, FloatRange::Linear { min: -1.0, max: 1.0 }).with_step_size(0.01).with_unit(" st"),
            lowf: FloatParam::new("VCO-2 Low Freq", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            pw: pct("Pulse Width", 0.0),
            pwm: FloatParam::new("PWM Source", 0.0, FloatRange::Linear { min: 0.0, max: 2.0 }).with_step_size(1.0),
            mix: pct("Source Mix", 0.5),
            hpf: FloatParam::new("HPF", 0.0, FloatRange::Linear { min: 0.0, max: 3.0 }).with_step_size(1.0),
            cutoff: hz("Cutoff", 1500.0, 20.0, 18000.0),
            res: pct("Resonance", 0.2),
            slope: FloatParam::new("Slope", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            fenvamt: pct("ENV Mod", 0.4),
            flfo: pct("LFO Mod", 0.0),
            fkbd: pct("Key Follow", 0.5),
            fa: FloatParam::new("ENV-1 Attack", 5.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            fd: FloatParam::new("ENV-1 Decay", 600.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            fs: pct("ENV-1 Sustain", 0.4),
            fr: FloatParam::new("ENV-1 Release", 300.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            a: FloatParam::new("ENV-2 Attack", 3.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            d: FloatParam::new("ENV-2 Decay", 600.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            s: pct("ENV-2 Sustain", 0.8),
            r: FloatParam::new("ENV-2 Release", 300.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            volume: db("Volume", -8.0, -30.0, 6.0),
        }
    }
}

pub const PORT: usize = 0;
pub const UNISON: usize = 1;
pub const LFO: usize = 2;
pub const LFODELAY: usize = 3;
pub const LFOW: usize = 4;
pub const VCOMOD: usize = 5;
pub const VCOENV: usize = 6;
pub const XMOD: usize = 7;
pub const SYNC: usize = 8;
pub const R1: usize = 9;
pub const W1: usize = 10;
pub const R2: usize = 11;
pub const W2: usize = 12;
pub const FINE2: usize = 13;
pub const LOWF: usize = 14;
pub const PW: usize = 15;
pub const PWM: usize = 16;
pub const MIX: usize = 17;
pub const HPF: usize = 18;
pub const CUTOFF: usize = 19;
pub const RES: usize = 20;
pub const SLOPE: usize = 21;
pub const FENVAMT: usize = 22;
pub const FLFO: usize = 23;
pub const FKBD: usize = 24;
pub const FA: usize = 25;
pub const FD: usize = 26;
pub const FS: usize = 27;
pub const FR: usize = 28;
pub const A: usize = 29;
pub const D: usize = 30;
pub const S: usize = 31;
pub const R: usize = 32;
pub const VOLUME: usize = 33;
pub const NPARAMS: usize = 34;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.port.as_ptr(),
            self.unison.as_ptr(),
            self.lfo.as_ptr(),
            self.lfodelay.as_ptr(),
            self.lfow.as_ptr(),
            self.vcomod.as_ptr(),
            self.vcoenv.as_ptr(),
            self.xmod.as_ptr(),
            self.sync.as_ptr(),
            self.r1.as_ptr(),
            self.w1.as_ptr(),
            self.r2.as_ptr(),
            self.w2.as_ptr(),
            self.fine2.as_ptr(),
            self.lowf.as_ptr(),
            self.pw.as_ptr(),
            self.pwm.as_ptr(),
            self.mix.as_ptr(),
            self.hpf.as_ptr(),
            self.cutoff.as_ptr(),
            self.res.as_ptr(),
            self.slope.as_ptr(),
            self.fenvamt.as_ptr(),
            self.flfo.as_ptr(),
            self.fkbd.as_ptr(),
            self.fa.as_ptr(),
            self.fd.as_ptr(),
            self.fs.as_ptr(),
            self.fr.as_ptr(),
            self.a.as_ptr(),
            self.d.as_ptr(),
            self.s.as_ptr(),
            self.r.as_ptr(),
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
/// Octaves from 8' for 64', 32', 16', 8', 4', 2'.
const RANGE: [f64; 6] = [-3.0, -2.0, -1.0, 0.0, 1.0, 2.0];

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
    ph: [f64; 2],
    filt: Ladder,
    hp: OnePole,
    since: f64,
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
    lfo: f64,
    rnd: f64,
    noise: Noise,
    bend: f64,
    modw: f64,
    pedal: bool,
    dc: [DcBlock; 2],
}

impl Dsp {
    fn unison(&self) -> bool {
        self.v[UNISON] >= 0.5
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
            vc.ph = [0.11 * i as f64, (0.53 * i as f64) % 1.0];
        }
        Self { fs, v, voices, mono: Vec::new(), clock: 0, lfo: 0.0, rnd: 0.0, noise: Noise::new(1981), bend: 0.0, modw: 0.0, pedal: false, dc: [DcBlock::new(fs), DcBlock::new(fs)] }
    }
    fn reset(&mut self) {
        for v in self.voices.iter_mut() {
            v.fenv = Adsr::default();
            v.aenv = Adsr::default();
            v.filt = Ladder::default();
            v.held = false;
        }
        self.mono.clear();
        self.dc.iter_mut().for_each(|d| d.reset());
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
                    if vc.idle() || self.v[PORT] <= 0.0 && !unison {
                        vc.pitch = note as f64;
                    }
                    vc.note = note;
                    vc.held = true;
                    vc.sus = false;
                    vc.vel = vel as f64;
                    vc.age = self.clock;
                    vc.fenv.on();
                    vc.since = 0.0;
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
        let (r1, r2) = (RANGE[st(R1, 5)], RANGE[st(R2, 5)]);
        let (w1, w2) = (st(W1, 3), st(W2, 3));
        let sync = p[SYNC] >= 0.5;
        let lowf = p[LOWF] >= 0.5;
        let unison = self.unison();
        let gc = if p[PORT] > 0.0 { coeff(fs, 2.0 + 2000.0 * (p[PORT] as f64).powi(2)) } else { 0.0 };
        let (fa, fd, fsus, fr) = (rate(fs, p[FA] as f64), rate(fs, p[FD] as f64), p[FS] as f64, rate(fs, p[FR] as f64));
        let (a, d, s, r) = (rate(fs, p[A] as f64), rate(fs, p[D] as f64), p[S] as f64, rate(fs, p[R] as f64));
        let k = 3.9 * p[RES] as f64;
        let two = p[SLOPE] < 0.5;
        let hpf: f64 = [0.0, 120.0, 250.0, 500.0][st(HPF, 3)];
        for vc in self.voices.iter_mut() {
            vc.hp.set_lowpass(fs, hpf.max(1.0));
        }
        let mix = p[MIX] as f64;
        let vol = db_to_gain(p[VOLUME] as f64);
        let lw = st(LFOW, 3);
        let dly = (p[LFODELAY] as f64 / 1000.0).max(0.001);
        let mut peak = 0.0f64;
        for kk in 0..n {
            self.lfo += p[LFO] as f64 / fs;
            if self.lfo >= 1.0 {
                self.lfo -= 1.0;
                self.rnd = self.noise.next().clamp(-1.0, 1.0);
            }
            let lv = match lw {
                0 => (std::f64::consts::TAU * self.lfo).sin(),
                1 => 2.0 * self.lfo - 1.0,
                2 => if self.lfo < 0.5 { 1.0 } else { -1.0 },
                _ => self.rnd,
            };
            let mut out = [0.0f64; 2];
            for (i, vc) in self.voices.iter_mut().enumerate() {
                if vc.aenv.idle() {
                    continue;
                }
                vc.since += 1.0 / fs;
                let fade = (vc.since / dly).min(1.0);
                let l = lv * fade;
                let target = vc.note as f64;
                vc.pitch = target + gc * (vc.pitch - target);
                let fe = vc.fenv.step(fa, fd, fsus, fr);
                let ae = vc.aenv.step(a, d, s, r);
                let spread = if unison { (i as f64 - 3.5) * 0.015 } else { 0.0 };
                let nb = vc.pitch + self.bend + spread + 12.0 * (l * p[VCOMOD] as f64 * (1.0 + self.modw) + fe * p[VCOENV] as f64);
                let f2 = if lowf { 0.3 * 2f64.powf(r2 + 3.0) } else { mtof(nb + p[FINE2] as f64) * 2f64.powf(r2) };
                let dt2 = (f2 / fs).min(0.45);
                let pwv = 0.5 - 0.45 * match st(PWM, 2) { 0 => p[PW] as f64, 1 => p[PW] as f64 * (0.5 + 0.5 * l), _ => p[PW] as f64 * fe };
                let pwv = pwv.clamp(0.05, 0.95);
                let o2 = match w2 {
                    0 => (std::f64::consts::TAU * vc.ph[1]).sin(),
                    1 => saw(vc.ph[1], dt2),
                    2 => pulse(vc.ph[1], dt2, pwv),
                    _ => self.noise.next(),
                };
                let f1 = mtof(nb + o2 * p[XMOD] as f64 * 36.0) * 2f64.powf(r1);
                let dt1 = (f1 / fs).clamp(0.0, 0.45);
                let o1 = match w1 {
                    0 => tri(vc.ph[0]),
                    1 => saw(vc.ph[0], dt1),
                    2 => pulse(vc.ph[0], dt1, pwv),
                    _ => pulse(vc.ph[0], dt1, 0.5),
                };
                vc.ph[0] += dt1;
                vc.ph[1] += dt2;
                if vc.ph[0] >= 1.0 {
                    vc.ph[0] -= 1.0;
                    if sync {
                        vc.ph[1] = vc.ph[0] * dt2 / dt1.max(1e-9);
                    }
                }
                vc.ph[1] = vc.ph[1].fract();
                let x0 = (o1 * (1.0 - mix) + o2 * mix) * 0.7;
                let x = if hpf > 0.0 { vc.hp.hp(x0) } else { x0 };
                let fc = p[CUTOFF] as f64 * 2f64.powf(6.0 * p[FENVAMT] as f64 * fe + 2.0 * p[FKBD] as f64 * (nb - 60.0) / 12.0 * 0.5 + 3.0 * l * p[FLFO] as f64);
                let (lp4, lp2) = vc.filt.process(x, fc, fs, if two { k * 0.7 } else { k }, two);
                let y = (if two { lp2 } else { lp4 * (1.0 + 0.4 * k) }) * ae * (0.6 + 0.4 * vc.vel);
                let pan = 0.5 + (i as f64 - 3.5) * 0.05;
                out[0] += y * (1.0 - pan) * 1.3;
                out[1] += y * pan * 1.3;
            }
            let (l, rr) = (self.dc[0].process(out[0]) * vol, self.dc[1].process(out[1]) * vol);
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
        Preset { name: "Rio Pad", values: &[(W1, 1.0), (W2, 1.0), (FINE2, 0.12), (MIX, 0.5), (CUTOFF, 1800.0), (RES, 0.15), (FENVAMT, 0.25), (FA, 300.0), (A, 250.0), (R, 1500.0), (S, 0.9)] },
        Preset { name: "Sync Brass", values: &[(W1, 1.0), (W2, 1.0), (SYNC, 1.0), (R2, 4.0), (VCOENV, 0.15), (MIX, 0.6), (CUTOFF, 900.0), (FENVAMT, 0.5), (FA, 40.0), (FD, 600.0), (FS, 0.5)] },
        Preset { name: "Cross Mod Bell", values: &[(W1, 0.0), (W2, 0.0), (R2, 5.0), (XMOD, 0.25), (MIX, 0.0), (CUTOFF, 6000.0), (FENVAMT, 0.0), (D, 1200.0), (S, 0.0), (R, 1200.0)] },
        Preset { name: "Square Arp Pluck", values: &[(W1, 3.0), (W2, 2.0), (PW, 0.4), (PWM, 1.0), (LFO, 3.0), (CUTOFF, 600.0), (RES, 0.5), (FENVAMT, 0.55), (FD, 220.0), (FS, 0.0), (D, 400.0), (S, 0.0), (R, 200.0)] },
        Preset { name: "Unison Bass", values: &[(UNISON, 1.0), (R1, 2.0), (R2, 2.0), (W1, 1.0), (W2, 2.0), (CUTOFF, 300.0), (RES, 0.3), (FENVAMT, 0.5), (FD, 300.0), (FS, 0.2)] },
        Preset { name: "12 dB Strings", values: &[(SLOPE, 0.0), (W1, 1.0), (W2, 1.0), (FINE2, 0.15), (HPF, 1.0), (CUTOFF, 3000.0), (A, 450.0), (R, 1300.0), (VCOMOD, 0.03), (LFO, 5.5), (LFODELAY, 800.0)] },
        Preset { name: "Low Freq Wobble", values: &[(LOWF, 1.0), (W2, 0.0), (XMOD, 0.06), (W1, 1.0), (MIX, 0.0), (CUTOFF, 1200.0)] },
    ]
}

const RANGES: &[&str] = &["64'", "32'", "16'", "8'", "4'", "2'"];

fn panel() -> Panel {
    let k = |p, x, y, r, label, tip| Item::Knob { p, x, y, r, label, tip, active: None };
    let sw = |p, x, y, l: &'static str, tip| Item::Switch { p, x, y, labels: ["", l], tip };
    Panel {
        title: "ROLAND JUPITER-8",
        subtitle: ("EIGHT VOICE POLYPHONIC, 1981", "2 VCO · CROSS MOD · SYNC · IR3109 12/24 dB · 2 ADSR"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            k(LFO, 50.0, 110.0, 16.0, "RATE", "LFO rate."),
            k(LFODELAY, 110.0, 110.0, 16.0, "DELAY", "LFO delay: it fades in after each key."),
            Item::Selector { p: LFOW, x: 190.0, y: 110.0, label: "LFO", positions: &["SINE", "SAW", "SQUARE", "RANDOM"], tip: "LFO wave." },
            k(VCOMOD, 280.0, 110.0, 16.0, "VCO LFO", "LFO to both VCOs (the mod wheel adds more)."),
            k(VCOENV, 340.0, 110.0, 16.0, "VCO ENV", "ENV-1 to both VCOs' pitch."),
            k(PORT, 410.0, 110.0, 16.0, "PORTA", "Portamento."),
            sw(UNISON, 480.0, 110.0, "UNISON", "Eight voices on one note."),
            k(VOLUME, 580.0, 110.0, 20.0, "VOLUME", "Output level."),
            Item::Divider { y: 150.0 },
            k(XMOD, 50.0, 210.0, 16.0, "CROSS MOD", "VCO-2 frequency-modulates VCO-1."),
            sw(SYNC, 50.0, 260.0, "SYNC", "VCO-1 resets VCO-2."),
            Item::Selector { p: R1, x: 140.0, y: 210.0, label: "VCO-1", positions: RANGES, tip: "VCO-1 range." },
            Item::Selector { p: W1, x: 230.0, y: 210.0, label: "WAVE", positions: &["TRI", "SAW", "PULSE", "SQUARE"], tip: "VCO-1 waveform." },
            Item::Selector { p: R2, x: 320.0, y: 210.0, label: "VCO-2", positions: RANGES, tip: "VCO-2 range." },
            Item::Selector { p: W2, x: 410.0, y: 210.0, label: "WAVE", positions: &["SINE", "SAW", "PULSE", "NOISE"], tip: "VCO-2 waveform." },
            k(FINE2, 490.0, 210.0, 16.0, "FINE", "VCO-2 fine tune."),
            sw(LOWF, 490.0, 260.0, "LOW FREQ", "VCO-2 drops to a sub-audio rate: a modulator for CROSS MOD."),
            k(MIX, 570.0, 210.0, 18.0, "SOURCE MIX", "VCO-1 (left) to VCO-2 (right)."),
            k(PW, 140.0, 300.0, 16.0, "PW", "Pulse width."),
            Item::Selector { p: PWM, x: 230.0, y: 300.0, label: "PWM", positions: &["MAN", "LFO", "ENV-1"], tip: "What moves the pulse width." },
            Item::Divider { y: 335.0 },
            Item::Selector { p: HPF, x: 50.0, y: 385.0, label: "HPF", positions: &["0", "1", "2", "3"], tip: "The non-resonant high-pass." },
            k(CUTOFF, 130.0, 385.0, 22.0, "CUTOFF", "VCF cutoff."),
            k(RES, 200.0, 385.0, 18.0, "RES", "VCF resonance."),
            Item::Switch { p: SLOPE, x: 270.0, y: 385.0, labels: ["12 dB", "24 dB"], tip: "The VCF slope." },
            k(FENVAMT, 340.0, 385.0, 16.0, "ENV", "ENV-1 to the cutoff."),
            k(FLFO, 400.0, 385.0, 16.0, "LFO", "LFO to the cutoff."),
            k(FKBD, 460.0, 385.0, 16.0, "KEY FLW", "Cutoff key follow."),
            Item::Divider { y: 425.0 },
            k(FA, 60.0, 470.0, 15.0, "A", "ENV-1 attack."),
            k(FD, 115.0, 470.0, 15.0, "D", "ENV-1 decay."),
            k(FS, 170.0, 470.0, 15.0, "S", "ENV-1 sustain."),
            k(FR, 225.0, 470.0, 15.0, "R", "ENV-1 release."),
            Item::Text { x: 310.0, y: 470.0, text: "ENV-1 · ENV-2", size: 10.0, align: Align2::CENTER_CENTER },
            k(A, 400.0, 470.0, 15.0, "A", "ENV-2 attack."),
            k(D, 455.0, 470.0, 15.0, "D", "ENV-2 decay."),
            k(S, 510.0, 470.0, 15.0, "S", "ENV-2 sustain."),
            k(R, 565.0, 470.0, 15.0, "R", "ENV-2 release."),
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Roland Jupiter-8", class_id: *b"HWRolandJupiter8", vendor: "PEQUOD Hardware",
    instrument, categories: [Instrument, Synth]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        with(defaults(), &[(MIX, 0.0), (CUTOFF, 18000.0), (RES, 0.0), (FENVAMT, 0.0), (FKBD, 0.0), (S, 1.0)])
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
