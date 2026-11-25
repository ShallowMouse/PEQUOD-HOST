//! Oberheim OB-X (1979): Tom Oberheim's first fully programmable polyphonic, eight discrete voices (four
//! and six were sold too). Each voice: two VCOs with saw and pulse (one pulse width for both), VCO 2
//! detune and hard sync to VCO 1, X-MOD (VCO 2 frequency-modulating VCO 1), the 12 dB/octave state-
//! variable filter of the SEM lineage with its bright, open resonance, an ADSR each for filter and VCA,
//! and a shared LFO (sine, square or sample-and-hold) to the VCOs, the pulse width and the filter.
//! Discrete voices drift: each has its own small tuning and filter offsets, as the real cards did.
//! Unison stacks all eight. The OB-Xa (1981) moved to the CEM3320 4-pole; this is the OB-X.

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

pub const W: u32 = 650;
pub const H: u32 = 450;

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
    #[id = "lfow"]
    pub lfow: FloatParam,
    #[id = "lfo1"]
    pub lfo1: FloatParam,
    #[id = "lfo2"]
    pub lfo2: FloatParam,
    #[id = "lfopw"]
    pub lfopw: FloatParam,
    #[id = "lfof"]
    pub lfof: FloatParam,
    #[id = "saw1"]
    pub saw1: FloatParam,
    #[id = "pul1"]
    pub pul1: FloatParam,
    #[id = "saw2"]
    pub saw2: FloatParam,
    #[id = "pul2"]
    pub pul2: FloatParam,
    #[id = "oct2"]
    pub oct2: FloatParam,
    #[id = "detune"]
    pub detune: FloatParam,
    #[id = "pw"]
    pub pw: FloatParam,
    #[id = "sync"]
    pub sync: FloatParam,
    #[id = "xmod"]
    pub xmod: FloatParam,
    #[id = "noise"]
    pub noise: FloatParam,
    #[id = "cutoff"]
    pub cutoff: FloatParam,
    #[id = "res"]
    pub res: FloatParam,
    #[id = "fenvamt"]
    pub fenvamt: FloatParam,
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
            lfo: hz("LFO Rate", 5.0, 0.1, 20.0),
            lfow: FloatParam::new("LFO Wave", 0.0, FloatRange::Linear { min: 0.0, max: 2.0 }).with_step_size(1.0),
            lfo1: pct("LFO to VCO 1", 0.0),
            lfo2: pct("LFO to VCO 2", 0.0),
            lfopw: pct("LFO to PW", 0.0),
            lfof: pct("LFO to Filter", 0.0),
            saw1: FloatParam::new("VCO 1 Saw", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            pul1: FloatParam::new("VCO 1 Pulse", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            saw2: FloatParam::new("VCO 2 Saw", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            pul2: FloatParam::new("VCO 2 Pulse", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            oct2: FloatParam::new("VCO 2 Frequency", 0.0, FloatRange::Linear { min: -24.0, max: 24.0 }).with_step_size(0.01).with_unit(" st"),
            detune: pct("Detune", 0.15),
            pw: pct("Pulse Width", 0.5),
            sync: FloatParam::new("Sync", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            xmod: pct("X-Mod", 0.0),
            noise: pct("Noise", 0.0),
            cutoff: hz("Cutoff", 1500.0, 20.0, 18000.0),
            res: pct("Resonance", 0.25),
            fenvamt: pct("Filter Envelope", 0.4),
            fkbd: FloatParam::new("Filter Keyboard", 2.0, FloatRange::Linear { min: 0.0, max: 2.0 }).with_step_size(1.0),
            fa: FloatParam::new("Filter Attack", 5.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            fd: FloatParam::new("Filter Decay", 600.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            fs: pct("Filter Sustain", 0.4),
            fr: FloatParam::new("Filter Release", 300.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            a: FloatParam::new("Attack", 3.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            d: FloatParam::new("Decay", 600.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            s: pct("Sustain", 0.8),
            r: FloatParam::new("Release", 300.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            volume: db("Volume", -8.0, -30.0, 6.0),
        }
    }
}

pub const PORT: usize = 0;
pub const UNISON: usize = 1;
pub const LFO: usize = 2;
pub const LFOW: usize = 3;
pub const LFO1: usize = 4;
pub const LFO2: usize = 5;
pub const LFOPW: usize = 6;
pub const LFOF: usize = 7;
pub const SAW1: usize = 8;
pub const PUL1: usize = 9;
pub const SAW2: usize = 10;
pub const PUL2: usize = 11;
pub const OCT2: usize = 12;
pub const DETUNE: usize = 13;
pub const PW: usize = 14;
pub const SYNC: usize = 15;
pub const XMOD: usize = 16;
pub const NOISE: usize = 17;
pub const CUTOFF: usize = 18;
pub const RES: usize = 19;
pub const FENVAMT: usize = 20;
pub const FKBD: usize = 21;
pub const FA: usize = 22;
pub const FD: usize = 23;
pub const FS: usize = 24;
pub const FR: usize = 25;
pub const A: usize = 26;
pub const D: usize = 27;
pub const S: usize = 28;
pub const R: usize = 29;
pub const VOLUME: usize = 30;
pub const NPARAMS: usize = 31;

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
            self.lfow.as_ptr(),
            self.lfo1.as_ptr(),
            self.lfo2.as_ptr(),
            self.lfopw.as_ptr(),
            self.lfof.as_ptr(),
            self.saw1.as_ptr(),
            self.pul1.as_ptr(),
            self.saw2.as_ptr(),
            self.pul2.as_ptr(),
            self.oct2.as_ptr(),
            self.detune.as_ptr(),
            self.pw.as_ptr(),
            self.sync.as_ptr(),
            self.xmod.as_ptr(),
            self.noise.as_ptr(),
            self.cutoff.as_ptr(),
            self.res.as_ptr(),
            self.fenvamt.as_ptr(),
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
    filt: Svf,
    trim: f64,
    ftrim: f64,
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
    sh: f64,
    noise: Noise,
    drift: [Wander; VOICES],
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
        // each card's own tolerances: a few cents of tuning, a few percent of cutoff
        const TRIM: [f64; VOICES] = [0.03, -0.05, 0.02, 0.06, -0.02, -0.04, 0.05, -0.01];
        const FTRIM: [f64; VOICES] = [0.04, -0.03, 0.06, -0.05, 0.0, 0.03, -0.06, 0.02];
        for (i, vc) in voices.iter_mut().enumerate() {
            vc.trim = TRIM[i];
            vc.ftrim = FTRIM[i];
            vc.ph = [0.13 * i as f64, 0.71 * i as f64 % 1.0];
        }
        let drift = std::array::from_fn(|i| Wander::new(fs, 0.3, 77 + i as u32));
        Self { fs, v, voices, mono: Vec::new(), clock: 0, lfo: 0.0, sh: 0.0, noise: Noise::new(1979), drift, bend: 0.0, modw: 0.0, pedal: false, dc: [DcBlock::new(fs), DcBlock::new(fs)] }
    }
    fn reset(&mut self) {
        for v in self.voices.iter_mut() {
            v.fenv = Adsr::default();
            v.aenv = Adsr::default();
            v.filt = Svf::default();
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
        let on = |i: usize| p[i] >= 0.5;
        let (s1, p1, s2, p2) = (on(SAW1), on(PUL1), on(SAW2), on(PUL2));
        let sync = on(SYNC);
        let unison = self.unison();
        let gc = if p[PORT] > 0.0 { coeff(fs, 2.0 + 2000.0 * (p[PORT] as f64).powi(2)) } else { 0.0 };
        let (fa, fd, fsus, fr) = (rate(fs, p[FA] as f64), rate(fs, p[FD] as f64), p[FS] as f64, rate(fs, p[FR] as f64));
        let (a, d, s, r) = (rate(fs, p[A] as f64), rate(fs, p[D] as f64), p[S] as f64, rate(fs, p[R] as f64));
        let track = [0.0, 0.5, 1.0][(p[FKBD].round().max(0.0) as usize).min(2)];
        let det = p[DETUNE] as f64 * 0.3;
        let vol = db_to_gain(p[VOLUME] as f64);
        let lw = (p[LFOW].round().max(0.0) as usize).min(2);
        let mut peak = 0.0f64;
        for kk in 0..n {
            self.lfo += p[LFO] as f64 / fs;
            if self.lfo >= 1.0 {
                self.lfo -= 1.0;
                self.sh = self.noise.next().clamp(-1.0, 1.0);
            }
            let lv = match lw {
                0 => (std::f64::consts::TAU * self.lfo).sin(),
                1 => if self.lfo < 0.5 { 1.0 } else { -1.0 },
                _ => self.sh,
            };
            let depth = 1.0 + self.modw;
            let (m1, m2) = (lv * p[LFO1] as f64 * depth * 2.0, lv * p[LFO2] as f64 * depth * 2.0);
            let pw = (0.5 + 0.9 * (p[PW] as f64 - 0.5) + 0.4 * lv * p[LFOPW] as f64).clamp(0.05, 0.95);
            let fl = lv * p[LFOF] as f64 * depth * 3.0;
            let mut out = [0.0f64; 2];
            for (i, vc) in self.voices.iter_mut().enumerate() {
                if vc.aenv.idle() {
                    continue;
                }
                let dr = self.drift[i].next() * 0.04;
                let target = vc.note as f64;
                vc.pitch = target + gc * (vc.pitch - target);
                let spread = if unison { (i as f64 - 3.5) * 0.02 * (0.5 + p[DETUNE] as f64) } else { 0.0 };
                let nb = vc.pitch + self.bend + vc.trim + dr + spread;
                let f2 = mtof(nb + p[OCT2] as f64 + det + m2);
                let dt2 = (f2 / fs).min(0.45);
                let o2 = (if s2 { saw(vc.ph[1], dt2) } else { 0.0 }) + if p2 { pulse(vc.ph[1], dt2, pw) } else { 0.0 };
                let xm = o2 * p[XMOD] as f64 * 24.0;
                let f1 = mtof(nb + m1 + xm);
                let dt1 = (f1 / fs).clamp(0.0, 0.45);
                let o1 = (if s1 { saw(vc.ph[0], dt1) } else { 0.0 }) + if p1 { pulse(vc.ph[0], dt1, pw) } else { 0.0 };
                vc.ph[0] += dt1;
                vc.ph[1] += dt2;
                if vc.ph[0] >= 1.0 {
                    vc.ph[0] -= 1.0;
                    if sync {
                        vc.ph[1] = vc.ph[0] * dt2 / dt1.max(1e-9);
                    }
                }
                vc.ph[1] = vc.ph[1].fract();
                let fe = vc.fenv.step(fa, fd, fsus, fr);
                let ae = vc.aenv.step(a, d, s, r);
                let x = (o1 + o2) * 0.4 + self.noise.next() * p[NOISE] as f64 * 0.5;
                let fc = p[CUTOFF] as f64 * 2f64.powf(vc.ftrim + 6.0 * p[FENVAMT] as f64 * fe + track * (nb - 60.0) / 12.0 + fl);
                let (lp, _, _) = vc.filt.process(x, fc, fs, p[RES] as f64 * 0.97);
                let y = lp * ae * (0.6 + 0.4 * vc.vel);
                // the OB-X's voices pan across the stereo outs
                let pan = [0.3, 0.7, 0.4, 0.6, 0.2, 0.8, 0.45, 0.55][i];
                out[0] += y * (1.0 - pan) * 1.4;
                out[1] += y * pan * 1.4;
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
        Preset { name: "Jump-style Brass", values: &[(SAW1, 1.0), (SAW2, 1.0), (DETUNE, 0.25), (CUTOFF, 900.0), (RES, 0.15), (FENVAMT, 0.45), (FA, 30.0), (FD, 900.0), (FS, 0.55), (A, 10.0), (S, 0.9), (R, 250.0)] },
        Preset { name: "1999 Stabs", values: &[(SAW1, 1.0), (SAW2, 1.0), (OCT2, 12.0), (DETUNE, 0.2), (CUTOFF, 1400.0), (FENVAMT, 0.5), (FD, 350.0), (FS, 0.2), (D, 500.0), (S, 0.3), (R, 150.0)] },
        Preset { name: "Sync Sweep", values: &[(SAW1, 0.0), (SAW2, 1.0), (SYNC, 1.0), (OCT2, 9.0), (LFO, 0.25), (LFO2, 1.0), (CUTOFF, 3000.0), (RES, 0.1)] },
        Preset { name: "Pulse Pad", values: &[(SAW1, 0.0), (PUL1, 1.0), (SAW2, 0.0), (PUL2, 1.0), (PW, 0.6), (LFOPW, 0.5), (LFO, 0.6), (A, 400.0), (R, 1200.0), (CUTOFF, 1200.0), (FENVAMT, 0.2), (FA, 600.0)] },
        Preset { name: "Unison Lead", values: &[(UNISON, 1.0), (DETUNE, 0.5), (PORT, 0.1), (CUTOFF, 2200.0), (RES, 0.35), (FENVAMT, 0.3)] },
        Preset { name: "X-Mod Bell", values: &[(SAW1, 0.0), (PUL1, 1.0), (SAW2, 0.0), (PUL2, 1.0), (OCT2, 7.0), (XMOD, 0.35), (CUTOFF, 5000.0), (FENVAMT, 0.0), (D, 900.0), (S, 0.0), (R, 900.0)] },
        Preset { name: "Resonant Sweep", values: &[(CUTOFF, 300.0), (RES, 0.8), (FENVAMT, 0.6), (FA, 1200.0), (FD, 2500.0), (FS, 0.3), (A, 200.0), (R, 1000.0)] },
    ]
}

fn panel() -> Panel {
    let k = |p, x, y, r, label, tip| Item::Knob { p, x, y, r, label, tip, active: None };
    let sw = |p, x, y, l: &'static str, tip| Item::Switch { p, x, y, labels: ["", l], tip };
    Panel {
        title: "OBERHEIM OB-X",
        subtitle: ("PROGRAMMABLE POLYPHONIC, 1979", "8 DISCRETE VOICES · 2 VCO · 12 dB SVF · X-MOD"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            k(PORT, 50.0, 100.0, 18.0, "PORTAMENTO", "Glide."),
            sw(UNISON, 130.0, 100.0, "UNISON", "All eight voices on one note."),
            k(LFO, 220.0, 100.0, 18.0, "RATE", "LFO rate."),
            Item::Selector { p: LFOW, x: 300.0, y: 100.0, label: "LFO", positions: &["SINE", "SQUARE", "S/H"], tip: "LFO wave." },
            k(LFO1, 380.0, 100.0, 15.0, "VCO 1", "LFO to VCO 1 (the mod wheel adds more)."),
            k(LFO2, 430.0, 100.0, 15.0, "VCO 2", "LFO to VCO 2."),
            k(LFOPW, 480.0, 100.0, 15.0, "PW", "LFO to the pulse width."),
            k(LFOF, 530.0, 100.0, 15.0, "FILTER", "LFO to the filter."),
            k(VOLUME, 600.0, 100.0, 20.0, "VOLUME", "Output level."),
            Item::Divider { y: 138.0 },
            sw(SAW1, 50.0, 168.0, "SAW 1", "VCO 1 sawtooth."),
            sw(PUL1, 50.0, 202.0, "PULSE 1", "VCO 1 pulse."),
            k(XMOD, 140.0, 185.0, 16.0, "X-MOD", "VCO 2 frequency-modulates VCO 1: clangs and growls."),
            k(PW, 210.0, 185.0, 16.0, "PW", "Pulse width, both VCOs."),
            sw(SAW2, 290.0, 168.0, "SAW 2", "VCO 2 sawtooth."),
            sw(PUL2, 290.0, 202.0, "PULSE 2", "VCO 2 pulse."),
            k(OCT2, 380.0, 185.0, 16.0, "VCO 2 FREQ", "VCO 2's pitch against VCO 1, two octaves either way."),
            k(DETUNE, 450.0, 185.0, 16.0, "DETUNE", "VCO 2 fine detune: the OB's width."),
            sw(SYNC, 530.0, 185.0, "SYNC", "VCO 2 synced to VCO 1."),
            k(NOISE, 600.0, 185.0, 16.0, "NOISE", "Noise into the filter."),
            Item::Divider { y: 232.0 },
            k(CUTOFF, 60.0, 282.0, 19.0, "FREQUENCY", "Filter cutoff."),
            k(RES, 140.0, 282.0, 18.0, "RESONANCE", "Filter resonance."),
            k(FENVAMT, 210.0, 282.0, 16.0, "ENV AMT", "How far the filter envelope opens the filter."),
            Item::Selector { p: FKBD, x: 290.0, y: 282.0, label: "KBD", positions: &["OFF", "HALF", "FULL"], tip: "Filter keyboard tracking." },
            k(FA, 380.0, 282.0, 15.0, "A", "Filter attack."),
            k(FD, 440.0, 282.0, 15.0, "D", "Filter decay."),
            k(FS, 500.0, 282.0, 15.0, "S", "Filter sustain."),
            k(FR, 560.0, 282.0, 15.0, "R", "Filter release."),
            Item::Divider { y: 324.0 },
            Item::Text { x: 200.0, y: 362.0, text: "VOLUME ENVELOPE", size: 10.0, align: Align2::CENTER_CENTER },
            k(A, 380.0, 362.0, 15.0, "A", "Attack."),
            k(D, 440.0, 362.0, 15.0, "D", "Decay."),
            k(S, 500.0, 362.0, 15.0, "S", "Sustain."),
            k(R, 560.0, 362.0, 15.0, "R", "Release."),
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Oberheim OB-X", class_id: *b"HWOberheimOBX197", vendor: "PEQUOD Hardware",
    instrument, categories: [Instrument, Synth]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        with(defaults(), &[(SAW2, 0.0), (DETUNE, 0.0), (CUTOFF, 18000.0), (RES, 0.0), (FENVAMT, 0.0), (FKBD, 0.0), (S, 1.0)])
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
