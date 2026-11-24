//! ARP Odyssey (model 2800, 1972-81): ARP's duophonic answer to the Minimoog. Two VCOs - the lowest key
//! held plays VCO 1, the highest VCO 2 (one key: both) - with saw and square, pulse width and PWM, hard
//! sync of VCO 2 to VCO 1, the ring modulator (the two squares multiplied), white or pink noise, the
//! sample and hold (mixing VCO 1's saw and noise, clocked by the LFO), the non-resonant high-pass, and
//! the three filters it had in turn: the 4023 12 dB/octave of the white Mk I, the 4035 24 dB ladder of
//! the Mk II, and the 2140 24 dB of the orange-and-black Mk III. An ADSR and an AR; the VCA on either;
//! ARP keyboards retrigger on every new key. Ranges and sliders after the owner's manual; the rest is
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

pub const W: u32 = 620;
pub const H: u32 = 560;

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "port"]
    pub port: FloatParam,
    #[id = "lfo"]
    pub lfo: FloatParam,
    #[id = "shmix"]
    pub shmix: FloatParam,
    #[id = "r1"]
    pub r1: FloatParam,
    #[id = "kbd1"]
    pub kbd1: FloatParam,
    #[id = "fm1"]
    pub fm1: FloatParam,
    #[id = "sh1"]
    pub sh1: FloatParam,
    #[id = "pw1"]
    pub pw1: FloatParam,
    #[id = "pwm1"]
    pub pwm1: FloatParam,
    #[id = "r2"]
    pub r2: FloatParam,
    #[id = "tune2"]
    pub tune2: FloatParam,
    #[id = "fm2"]
    pub fm2: FloatParam,
    #[id = "env2"]
    pub env2: FloatParam,
    #[id = "pw2"]
    pub pw2: FloatParam,
    #[id = "sync"]
    pub sync: FloatParam,
    #[id = "w1"]
    pub w1: FloatParam,
    #[id = "w2"]
    pub w2: FloatParam,
    #[id = "l1"]
    pub l1: FloatParam,
    #[id = "l2"]
    pub l2: FloatParam,
    #[id = "ring"]
    pub ring: FloatParam,
    #[id = "noise"]
    pub noise: FloatParam,
    #[id = "pink"]
    pub pink: FloatParam,
    #[id = "hpf"]
    pub hpf: FloatParam,
    #[id = "cutoff"]
    pub cutoff: FloatParam,
    #[id = "res"]
    pub res: FloatParam,
    #[id = "fkbd"]
    pub fkbd: FloatParam,
    #[id = "fsh"]
    pub fsh: FloatParam,
    #[id = "fenv"]
    pub fenv: FloatParam,
    #[id = "ftype"]
    pub ftype: FloatParam,
    #[id = "a"]
    pub a: FloatParam,
    #[id = "d"]
    pub d: FloatParam,
    #[id = "s"]
    pub s: FloatParam,
    #[id = "r"]
    pub r: FloatParam,
    #[id = "ara"]
    pub ara: FloatParam,
    #[id = "arr"]
    pub arr: FloatParam,
    #[id = "vca"]
    pub vca: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            port: pct("Portamento", 0.0),
            lfo: hz("LFO Speed", 5.0, 0.2, 20.0),
            shmix: pct("S/H Mixer", 1.0),
            r1: FloatParam::new("VCO 1 Range", 2.0, FloatRange::Linear { min: 0.0, max: 4.0 }).with_step_size(1.0),
            kbd1: FloatParam::new("VCO 1 Keyboard", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            fm1: pct("VCO 1 FM (LFO Sine)", 0.0),
            sh1: pct("VCO 1 FM (S/H)", 0.0),
            pw1: pct("VCO 1 Pulse Width", 0.5),
            pwm1: pct("VCO 1 PWM (LFO)", 0.0),
            r2: FloatParam::new("VCO 2 Range", 2.0, FloatRange::Linear { min: 0.0, max: 4.0 }).with_step_size(1.0),
            tune2: FloatParam::new("VCO 2 Tune", 0.08, FloatRange::Linear { min: -12.0, max: 12.0 }).with_step_size(0.01).with_unit(" st"),
            fm2: pct("VCO 2 FM (LFO Sine)", 0.0),
            env2: pct("VCO 2 FM (ADSR)", 0.0),
            pw2: pct("VCO 2 Pulse Width", 0.5),
            sync: FloatParam::new("Sync", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            w1: FloatParam::new("VCO 1 Wave", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            w2: FloatParam::new("VCO 2 Wave", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            l1: pct("Mixer VCO 1", 0.8),
            l2: pct("Mixer VCO 2", 0.6),
            ring: pct("Mixer Ring Mod", 0.0),
            noise: pct("Mixer Noise", 0.0),
            pink: FloatParam::new("Noise", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            hpf: hz("High-Pass", 16.0, 16.0, 16000.0),
            cutoff: hz("Cutoff", 1200.0, 16.0, 16000.0),
            res: pct("Resonance", 0.2),
            fkbd: pct("VCF Keyboard", 0.5),
            fsh: pct("VCF S/H", 0.0),
            fenv: pct("VCF ADSR", 0.4),
            ftype: FloatParam::new("Filter", 2.0, FloatRange::Linear { min: 0.0, max: 2.0 }).with_step_size(1.0),
            a: FloatParam::new("Attack", 3.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            d: FloatParam::new("Decay", 400.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            s: pct("Sustain", 0.5),
            r: FloatParam::new("Release", 200.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            ara: FloatParam::new("AR Attack", 3.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            arr: FloatParam::new("AR Release", 200.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            vca: FloatParam::new("VCA", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            volume: db("Volume", -6.0, -30.0, 6.0),
        }
    }
}

pub const PORT: usize = 0;
pub const LFO: usize = 1;
pub const SHMIX: usize = 2;
pub const R1: usize = 3;
pub const KBD1: usize = 4;
pub const FM1: usize = 5;
pub const SH1: usize = 6;
pub const PW1: usize = 7;
pub const PWM1: usize = 8;
pub const R2: usize = 9;
pub const TUNE2: usize = 10;
pub const FM2: usize = 11;
pub const ENV2: usize = 12;
pub const PW2: usize = 13;
pub const SYNC: usize = 14;
pub const W1: usize = 15;
pub const W2: usize = 16;
pub const L1: usize = 17;
pub const L2: usize = 18;
pub const RING: usize = 19;
pub const NOISE: usize = 20;
pub const PINK: usize = 21;
pub const HPF: usize = 22;
pub const CUTOFF: usize = 23;
pub const RES: usize = 24;
pub const FKBD: usize = 25;
pub const FSH: usize = 26;
pub const FENV: usize = 27;
pub const FTYPE: usize = 28;
pub const A: usize = 29;
pub const D: usize = 30;
pub const S: usize = 31;
pub const R: usize = 32;
pub const ARA: usize = 33;
pub const ARR: usize = 34;
pub const VCA: usize = 35;
pub const VOLUME: usize = 36;
pub const NPARAMS: usize = 37;

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
            self.lfo.as_ptr(),
            self.shmix.as_ptr(),
            self.r1.as_ptr(),
            self.kbd1.as_ptr(),
            self.fm1.as_ptr(),
            self.sh1.as_ptr(),
            self.pw1.as_ptr(),
            self.pwm1.as_ptr(),
            self.r2.as_ptr(),
            self.tune2.as_ptr(),
            self.fm2.as_ptr(),
            self.env2.as_ptr(),
            self.pw2.as_ptr(),
            self.sync.as_ptr(),
            self.w1.as_ptr(),
            self.w2.as_ptr(),
            self.l1.as_ptr(),
            self.l2.as_ptr(),
            self.ring.as_ptr(),
            self.noise.as_ptr(),
            self.pink.as_ptr(),
            self.hpf.as_ptr(),
            self.cutoff.as_ptr(),
            self.res.as_ptr(),
            self.fkbd.as_ptr(),
            self.fsh.as_ptr(),
            self.fenv.as_ptr(),
            self.ftype.as_ptr(),
            self.a.as_ptr(),
            self.d.as_ptr(),
            self.s.as_ptr(),
            self.r.as_ptr(),
            self.ara.as_ptr(),
            self.arr.as_ptr(),
            self.vca.as_ptr(),
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

/// Octaves from the 8' (VCO 1's LO is sub-audio when it's off the keyboard).
const RANGE: [f64; 5] = [-6.0, -1.0, 0.0, 1.0, 2.0];

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    held: Vec<u8>,
    gate: bool,
    n1: f64,
    n2: f64,
    t1: f64,
    t2: f64,
    ph: [f64; 2],
    lfo: f64,
    sh: f64,
    adsr: Adsr,
    ar: Adsr,
    svf: Svf,
    lad: Ladder,
    hp: OnePole,
    noise: Noise,
    pk: [f64; 3],
    bend: f64,
    dc: DcBlock,
}

impl Dsp {
    fn retarget(&mut self) {
        if let (Some(lo), Some(hi)) = (self.held.iter().min(), self.held.iter().max()) {
            self.t1 = *lo as f64;
            self.t2 = *hi as f64;
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, held: Vec::with_capacity(16), gate: false, n1: 60.0, n2: 60.0, t1: 60.0, t2: 60.0, ph: [0.0, 0.37], lfo: 0.0, sh: 0.0, adsr: Adsr::default(), ar: Adsr::default(), svf: Svf::default(), lad: Ladder::default(), hp: OnePole::default(), noise: Noise::new(2800), pk: [0.0; 3], bend: 0.0, dc: DcBlock::new(fs) }
    }
    fn reset(&mut self) {
        self.held.clear();
        self.gate = false;
        self.adsr = Adsr::default();
        self.ar = Adsr::default();
        self.svf = Svf::default();
        self.lad = Ladder::default();
        self.hp.reset();
        self.dc.reset();
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn midi(&mut self, ev: Midi) {
        match ev {
            Midi::On { note, .. } => {
                let first = self.held.is_empty();
                self.held.retain(|n| *n != note);
                self.held.push(note);
                self.retarget();
                if first && self.v[PORT] <= 0.0 {
                    self.n1 = self.t1;
                    self.n2 = self.t2;
                }
                // ARP keyboards retrigger on every new key
                self.gate = true;
                self.adsr.on();
                self.ar.on();
            }
            Midi::Off { note } => {
                self.held.retain(|n| *n != note);
                if self.held.is_empty() {
                    self.gate = false;
                    self.adsr.off();
                    self.ar.off();
                } else {
                    self.retarget();
                }
            }
            Midi::Bend { value } => self.bend = value as f64 * 2.0,
            Midi::AllOff => {
                self.held.clear();
                self.gate = false;
                self.adsr.off();
                self.ar.off();
            }
            _ => {}
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let p = self.v;
        let st = |i: usize, max: usize| (p[i].round().max(0.0) as usize).min(max);
        let (r1, r2) = (RANGE[st(R1, 4)], RANGE[st(R2, 4)]);
        let kbd1 = p[KBD1] >= 0.5;
        let sync = p[SYNC] >= 0.5;
        let (sq1, sq2) = (p[W1] >= 0.5, p[W2] >= 0.5);
        let gc = if p[PORT] > 0.0 { coeff(fs, 2.0 + 2500.0 * (p[PORT] as f64).powi(2)) } else { 0.0 };
        let (a, d, s, r) = (rate(fs, p[A] as f64), rate(fs, p[D] as f64), p[S] as f64, rate(fs, p[R] as f64));
        let (ara, arr) = (rate(fs, p[ARA] as f64), rate(fs, p[ARR] as f64));
        self.hp.set_lowpass(fs, p[HPF] as f64);
        let k_ladder = 4.1 * p[RES] as f64;
        let ftype = st(FTYPE, 2);
        let vol = db_to_gain(p[VOLUME] as f64);
        let lfo_inc = p[LFO] as f64 / fs;
        let mut peak = 0.0f64;
        for kk in 0..n {
            self.n1 = self.t1 + gc * (self.n1 - self.t1);
            self.n2 = self.t2 + gc * (self.n2 - self.t2);
            // LFO (sine and square) and the S/H, clocked by it
            self.lfo += lfo_inc;
            let wrapped = self.lfo >= 1.0;
            self.lfo = self.lfo.fract();
            let sine = (std::f64::consts::TAU * self.lfo).sin();
            let white = self.noise.next();
            // pink: Paul Kellet's three-pole economy filter
            self.pk[0] = 0.99765 * self.pk[0] + white * 0.0990460;
            self.pk[1] = 0.96300 * self.pk[1] + white * 0.2965164;
            self.pk[2] = 0.57000 * self.pk[2] + white * 1.0526913;
            let pinkv = (self.pk[0] + self.pk[1] + self.pk[2] + white * 0.1848) * 0.25;
            let nz = if p[PINK] >= 0.5 { pinkv } else { white };
            let saw1_raw = 2.0 * self.ph[0] - 1.0;
            if wrapped {
                self.sh = (1.0 - p[SHMIX] as f64) * saw1_raw + p[SHMIX] as f64 * white.clamp(-1.0, 1.0);
            }
            let env = self.adsr.step(a, d, s, r);
            let arv = self.ar.step(ara, 1.0, 1.0, arr);
            let note1 = if kbd1 { self.n1 + self.bend } else { 60.0 };
            let f1 = mtof(note1 + 12.0 * (p[FM1] as f64 * sine + p[SH1] as f64 * self.sh)) * 2f64.powf(r1);
            let f2 = mtof(self.n2 + self.bend + p[TUNE2] as f64 + 12.0 * p[FM2] as f64 * sine + 24.0 * p[ENV2] as f64 * env) * 2f64.powf(r2);
            let (dt1, dt2) = ((f1 / fs).min(0.45), (f2 / fs).min(0.45));
            let w1 = (0.05 + 0.45 * p[PW1] as f64 + 0.4 * p[PWM1] as f64 * sine).clamp(0.05, 0.95);
            let w2 = (0.05 + 0.45 * p[PW2] as f64).clamp(0.05, 0.95);
            let o1 = if sq1 { pulse(self.ph[0], dt1, w1) } else { saw(self.ph[0], dt1) };
            let o2 = if sq2 { pulse(self.ph[1], dt2, w2) } else { saw(self.ph[1], dt2) };
            let ring = if (self.ph[0] < w1) == (self.ph[1] < w2) { 1.0 } else { -1.0 };
            self.ph[0] += dt1;
            self.ph[1] += dt2;
            if self.ph[0] >= 1.0 {
                self.ph[0] -= 1.0;
                if sync {
                    self.ph[1] = self.ph[0] * dt2 / dt1;
                }
            }
            self.ph[1] = self.ph[1].fract();
            let mix = o1 * p[L1] as f64 + o2 * p[L2] as f64 + ring * p[RING] as f64 * 0.8 + nz * p[NOISE] as f64;
            let x = self.hp.hp(mix * 0.6);
            let track = p[FKBD] as f64 * (0.5 * (self.n1 + self.n2) - 60.0) / 12.0;
            let fc = p[CUTOFF] as f64 * 2f64.powf(track + 6.0 * p[FENV] as f64 * env + 3.0 * p[FSH] as f64 * self.sh);
            let y = match ftype {
                0 => self.svf.process(x, fc, fs, p[RES] as f64 * 0.98).0,
                1 => self.lad.process(x, fc, fs, k_ladder, false).0 * (1.0 + 0.4 * k_ladder),
                _ => self.lad.process(x * 0.8, fc, fs, k_ladder * 0.95, false).0 * (1.0 + 0.3 * k_ladder) * 1.2,
            };
            let amp = if p[VCA] >= 0.5 { env } else { arv };
            let out = self.dc.process(y) * amp * vol * 1.4;
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
        Preset { name: "Chameleon Bass", values: &[(R1, 1.0), (R2, 1.0), (W1, 1.0), (W2, 0.0), (TUNE2, 0.1), (L1, 0.9), (L2, 0.7), (CUTOFF, 250.0), (RES, 0.45), (FENV, 0.55), (D, 220.0), (S, 0.1), (R, 80.0), (VCA, 1.0), (FTYPE, 2.0)] },
        Preset { name: "Sync Lead", values: &[(SYNC, 1.0), (ENV2, 0.35), (L1, 0.0), (L2, 0.9), (TUNE2, 7.0), (CUTOFF, 2500.0), (RES, 0.2), (FENV, 0.2), (D, 700.0), (S, 0.3), (PORT, 0.15)] },
        Preset { name: "Ring Mod Bells", values: &[(RING, 0.9), (L1, 0.0), (L2, 0.0), (R2, 3.0), (TUNE2, 3.86), (W1, 1.0), (W2, 1.0), (CUTOFF, 6000.0), (FENV, 0.0), (VCA, 1.0), (D, 900.0), (S, 0.0), (R, 900.0)] },
        Preset { name: "S/H Bloops", values: &[(SH1, 0.5), (LFO, 7.0), (SHMIX, 1.0), (FSH, 0.6), (CUTOFF, 900.0), (RES, 0.7), (FENV, 0.1), (VCA, 0.0)] },
        Preset { name: "Duo Fifths", values: &[(TUNE2, 0.0), (L1, 0.7), (L2, 0.7), (W1, 1.0), (PWM1, 0.4), (LFO, 0.8), (CUTOFF, 1500.0), (RES, 0.25), (FENV, 0.3), (S, 0.6)] },
        Preset { name: "Wind and Surf", values: &[(L1, 0.0), (L2, 0.0), (NOISE, 1.0), (PINK, 1.0), (CUTOFF, 600.0), (RES, 0.8), (FENV, 0.3), (A, 1500.0), (D, 3000.0), (S, 0.3), (R, 1500.0), (VCA, 1.0), (ARA, 1500.0), (ARR, 2000.0)] },
        Preset { name: "Mk I Brass", values: &[(FTYPE, 0.0), (W1, 0.0), (W2, 0.0), (TUNE2, 0.12), (CUTOFF, 500.0), (RES, 0.1), (FENV, 0.6), (A, 70.0), (D, 600.0), (S, 0.6), (VCA, 1.0)] },
    ]
}

const RANGES: &[&str] = &["LO", "16'", "8'", "4'", "2'"];
const WAVES: [&str; 2] = ["SAW", "SQUARE"];

fn panel() -> Panel {
    let k = |p, x, y, r, label, tip| Item::Knob { p, x, y, r, label, tip, active: None };
    Panel {
        title: "ARP ODYSSEY",
        subtitle: ("MODEL 2800, 1972", "DUOPHONIC · 2 VCO · RING MOD · S/H · 3 FILTER REVISIONS"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            k(PORT, 50.0, 110.0, 18.0, "PORTAMENTO", "How long the pitch slides to a new note."),
            k(LFO, 120.0, 110.0, 18.0, "LFO SPEED", "The LFO, which also clocks the sample and hold."),
            k(SHMIX, 190.0, 110.0, 18.0, "S/H MIX", "What the sample and hold samples: VCO 1's saw (left) to noise (right)."),
            Item::Switch { p: PINK, x: 280.0, y: 110.0, labels: ["WHITE", "PINK"], tip: "The noise generator's colour." },
            Item::Selector { p: FTYPE, x: 400.0, y: 110.0, label: "FILTER", positions: &["4023 12dB", "4035 24dB", "2140 24dB"], tip: "Which Odyssey: the Mk I's 4023 (12 dB/oct), the Mk II's 4035 ladder, or the Mk III's 2140." },
            k(VOLUME, 560.0, 110.0, 22.0, "VOLUME", "Output level."),
            Item::Divider { y: 150.0 },
            Item::Selector { p: R1, x: 50.0, y: 200.0, label: "VCO 1", positions: RANGES, tip: "VCO 1 range; off the keyboard, LO makes it a sub-audio modulator." },
            Item::Switch { p: KBD1, x: 50.0, y: 260.0, labels: ["KBD OFF", "KBD ON"], tip: "VCO 1 follows the keyboard (the lowest key)." },
            k(FM1, 140.0, 210.0, 16.0, "FM SINE", "VCO 1 frequency modulation by the LFO's sine: vibrato."),
            k(SH1, 200.0, 210.0, 16.0, "FM S/H", "VCO 1 frequency modulation by the sample and hold: the random burble."),
            k(PW1, 260.0, 210.0, 16.0, "PW", "VCO 1 pulse width."),
            k(PWM1, 320.0, 210.0, 16.0, "PWM", "VCO 1 pulse width modulation by the LFO."),
            Item::Switch { p: W1, x: 390.0, y: 210.0, labels: ["SAW", "SQUARE"], tip: "VCO 1 wave into the mixer." },
            k(L1, 470.0, 210.0, 16.0, "VCO 1", "VCO 1 in the mixer."),
            k(RING, 540.0, 210.0, 16.0, "RING", "The ring modulator in the mixer: the two squares multiplied - clangs and bells."),
            Item::Selector { p: R2, x: 50.0, y: 320.0, label: "VCO 2", positions: RANGES, tip: "VCO 2 range. It plays the highest key." },
            k(TUNE2, 140.0, 320.0, 16.0, "TUNE", "VCO 2 tuning against VCO 1."),
            k(FM2, 200.0, 320.0, 16.0, "FM SINE", "VCO 2 frequency modulation by the LFO's sine."),
            k(ENV2, 260.0, 320.0, 16.0, "FM ADSR", "VCO 2 swept by the ADSR: with sync, the Odyssey's tearing lead."),
            k(PW2, 320.0, 320.0, 16.0, "PW", "VCO 2 pulse width."),
            Item::Switch { p: W2, x: 390.0, y: 300.0, labels: ["SAW", "SQUARE"], tip: "VCO 2 wave into the mixer." },
            Item::Switch { p: SYNC, x: 390.0, y: 340.0, labels: ["", "SYNC"], tip: "VCO 2 hard-synced to VCO 1." },
            k(L2, 470.0, 320.0, 16.0, "VCO 2", "VCO 2 in the mixer."),
            k(NOISE, 540.0, 320.0, 16.0, "NOISE", "Noise in the mixer."),
            Item::Divider { y: 372.0 },
            k(HPF, 50.0, 420.0, 18.0, "HPF", "The non-resonant high-pass filter before the VCF."),
            k(CUTOFF, 120.0, 420.0, 22.0, "VCF", "Low-pass cutoff."),
            k(RES, 190.0, 420.0, 18.0, "RESONANCE", "Resonance: the 24 dB filters whistle at the top."),
            k(FKBD, 250.0, 420.0, 16.0, "KBD", "How much the cutoff follows the keys."),
            k(FSH, 310.0, 420.0, 16.0, "S/H", "Cutoff modulated by the sample and hold."),
            k(FENV, 370.0, 420.0, 16.0, "ADSR", "Cutoff swept by the ADSR."),
            Item::Switch { p: VCA, x: 470.0, y: 420.0, labels: ["AR", "ADSR"], tip: "Which envelope opens the VCA." },
            Item::Divider { y: 462.0 },
            k(A, 50.0, 510.0, 16.0, "ATTACK", "ADSR attack."),
            k(D, 110.0, 510.0, 16.0, "DECAY", "ADSR decay."),
            k(S, 170.0, 510.0, 16.0, "SUSTAIN", "ADSR sustain."),
            k(R, 230.0, 510.0, 16.0, "RELEASE", "ADSR release."),
            k(ARA, 350.0, 510.0, 16.0, "AR ATTACK", "AR attack."),
            k(ARR, 410.0, 510.0, 16.0, "AR RELEASE", "AR release."),
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "ARP Odyssey", class_id: *b"HWArpOdyssey2800", vendor: "PEQUOD Hardware",
    instrument, categories: [Instrument, Synth]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        with(defaults(), &[(L2, 0.0), (CUTOFF, 16000.0), (RES, 0.0), (FENV, 0.0), (FKBD, 0.0), (VCA, 1.0), (S, 1.0)])
    }
    /// Two keys: VCO 1 takes the lower, VCO 2 the upper.
    #[test]
    fn duophonic() {
        let p = with(tune_params(), &[(L1, 0.0), (L2, 0.8), (TUNE2, 0.0)]);
        let (l, _) = render(&p, &[(0, Midi::On { note: 45, vel: 1.0 }), (0, Midi::On { note: 69, vel: 1.0 })], 48000);
        let f = pitch(&l[9600..]);
        println!("VCO 2 alone with A2 and A4 held: {f:.1} Hz");
        assert!((f - 440.0).abs() < 6.0);
    }

    include!("../../instrument_tests.rs");
}
