//! ARP 2600 (1971): the semi-modular, played here through its normals - the connections made behind the
//! panel, so it sounds with no patch cords. VCO-1 (its square to the VCF), VCO-2 (its saw to the VCF,
//! its sine to VCO-1's and the VCF's FM), VCO-3 (its triangle to the VCF; noise and the ADSR at its
//! FM), the ring modulator (VCO-1 by VCO-2), the noise generator with its colour from white to low
//! rumble, the sample and hold (noise, clocked by its internal clock) at VCO-1's and VCO-2's FM, the
//! filter - the early 4012 four-pole ladder, or the 4072 that replaced it - with the ADSR and the
//! keyboard at its FM, the VCA on the AR (or the ADSR), and the spring reverb. The 3604 keyboard: one
//! voice, low-note priority, a new trigger on every key. Normals after the owner's manual; curves
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

pub const W: u32 = 640;
pub const H: u32 = 470;

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "port"]
    pub port: FloatParam,
    #[id = "r1"]
    pub r1: FloatParam,
    #[id = "fine1"]
    pub fine1: FloatParam,
    #[id = "fm1sh"]
    pub fm1sh: FloatParam,
    #[id = "fm1v2"]
    pub fm1v2: FloatParam,
    #[id = "r2"]
    pub r2: FloatParam,
    #[id = "tune2"]
    pub tune2: FloatParam,
    #[id = "fm2sh"]
    pub fm2sh: FloatParam,
    #[id = "fm2v1"]
    pub fm2v1: FloatParam,
    #[id = "r3"]
    pub r3: FloatParam,
    #[id = "tune3"]
    pub tune3: FloatParam,
    #[id = "kbd3"]
    pub kbd3: FloatParam,
    #[id = "fm3n"]
    pub fm3n: FloatParam,
    #[id = "fm3env"]
    pub fm3env: FloatParam,
    #[id = "mring"]
    pub mring: FloatParam,
    #[id = "m1"]
    pub m1: FloatParam,
    #[id = "m2"]
    pub m2: FloatParam,
    #[id = "m3"]
    pub m3: FloatParam,
    #[id = "mnoise"]
    pub mnoise: FloatParam,
    #[id = "ncolor"]
    pub ncolor: FloatParam,
    #[id = "shrate"]
    pub shrate: FloatParam,
    #[id = "cutoff"]
    pub cutoff: FloatParam,
    #[id = "res"]
    pub res: FloatParam,
    #[id = "fenv"]
    pub fenv: FloatParam,
    #[id = "fv2"]
    pub fv2: FloatParam,
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
    #[id = "reverb"]
    pub reverb: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            port: pct("Portamento", 0.0),
            r1: FloatParam::new("VCO-1 Range", 3.0, FloatRange::Linear { min: 0.0, max: 5.0 }).with_step_size(1.0),
            fine1: FloatParam::new("VCO-1 Fine", 0.0, FloatRange::Linear { min: -1.0, max: 1.0 }).with_step_size(0.01).with_unit(" st"),
            fm1sh: pct("VCO-1 FM S/H", 0.0),
            fm1v2: pct("VCO-1 FM VCO-2 Sine", 0.0),
            r2: FloatParam::new("VCO-2 Range", 3.0, FloatRange::Linear { min: 0.0, max: 5.0 }).with_step_size(1.0),
            tune2: FloatParam::new("VCO-2 Tune", 0.07, FloatRange::Linear { min: -12.0, max: 12.0 }).with_step_size(0.01).with_unit(" st"),
            fm2sh: pct("VCO-2 FM S/H", 0.0),
            fm2v1: pct("VCO-2 FM VCO-1 Square", 0.0),
            r3: FloatParam::new("VCO-3 Range", 2.0, FloatRange::Linear { min: 0.0, max: 5.0 }).with_step_size(1.0),
            tune3: FloatParam::new("VCO-3 Tune", -0.05, FloatRange::Linear { min: -12.0, max: 12.0 }).with_step_size(0.01).with_unit(" st"),
            kbd3: FloatParam::new("VCO-3 Keyboard", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            fm3n: pct("VCO-3 FM Noise", 0.0),
            fm3env: pct("VCO-3 FM ADSR", 0.0),
            mring: pct("VCF Ring Mod", 0.0),
            m1: pct("VCF VCO-1 Square", 0.7),
            m2: pct("VCF VCO-2 Saw", 0.7),
            m3: pct("VCF VCO-3 Triangle", 0.0),
            mnoise: pct("VCF Noise", 0.0),
            ncolor: pct("Noise Colour", 0.0),
            shrate: hz("S/H Clock", 8.0, 0.2, 40.0),
            cutoff: hz("Cutoff", 1000.0, 10.0, 16000.0),
            res: pct("Resonance", 0.2),
            fenv: pct("VCF FM ADSR", 0.4),
            fv2: pct("VCF FM VCO-2 Sine", 0.0),
            ftype: FloatParam::new("Filter", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            a: FloatParam::new("ADSR Attack", 5.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            d: FloatParam::new("ADSR Decay", 500.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            s: pct("ADSR Sustain", 0.5),
            r: FloatParam::new("ADSR Release", 250.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            ara: FloatParam::new("AR Attack", 3.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            arr: FloatParam::new("AR Release", 250.0, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms"),
            vca: FloatParam::new("VCA", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            reverb: pct("Reverb", 0.15),
            volume: db("Volume", -6.0, -30.0, 6.0),
        }
    }
}

pub const PORT: usize = 0;
pub const R1: usize = 1;
pub const FINE1: usize = 2;
pub const FM1SH: usize = 3;
pub const FM1V2: usize = 4;
pub const R2: usize = 5;
pub const TUNE2: usize = 6;
pub const FM2SH: usize = 7;
pub const FM2V1: usize = 8;
pub const R3: usize = 9;
pub const TUNE3: usize = 10;
pub const KBD3: usize = 11;
pub const FM3N: usize = 12;
pub const FM3ENV: usize = 13;
pub const MRING: usize = 14;
pub const M1: usize = 15;
pub const M2: usize = 16;
pub const M3: usize = 17;
pub const MNOISE: usize = 18;
pub const NCOLOR: usize = 19;
pub const SHRATE: usize = 20;
pub const CUTOFF: usize = 21;
pub const RES: usize = 22;
pub const FENV: usize = 23;
pub const FV2: usize = 24;
pub const FTYPE: usize = 25;
pub const A: usize = 26;
pub const D: usize = 27;
pub const S: usize = 28;
pub const R: usize = 29;
pub const ARA: usize = 30;
pub const ARR: usize = 31;
pub const VCA: usize = 32;
pub const REVERB: usize = 33;
pub const VOLUME: usize = 34;
pub const NPARAMS: usize = 35;

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
            self.r1.as_ptr(),
            self.fine1.as_ptr(),
            self.fm1sh.as_ptr(),
            self.fm1v2.as_ptr(),
            self.r2.as_ptr(),
            self.tune2.as_ptr(),
            self.fm2sh.as_ptr(),
            self.fm2v1.as_ptr(),
            self.r3.as_ptr(),
            self.tune3.as_ptr(),
            self.kbd3.as_ptr(),
            self.fm3n.as_ptr(),
            self.fm3env.as_ptr(),
            self.mring.as_ptr(),
            self.m1.as_ptr(),
            self.m2.as_ptr(),
            self.m3.as_ptr(),
            self.mnoise.as_ptr(),
            self.ncolor.as_ptr(),
            self.shrate.as_ptr(),
            self.cutoff.as_ptr(),
            self.res.as_ptr(),
            self.fenv.as_ptr(),
            self.fv2.as_ptr(),
            self.ftype.as_ptr(),
            self.a.as_ptr(),
            self.d.as_ptr(),
            self.s.as_ptr(),
            self.r.as_ptr(),
            self.ara.as_ptr(),
            self.arr.as_ptr(),
            self.vca.as_ptr(),
            self.reverb.as_ptr(),
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

/// Octaves from 8' (LF is sub-audio).
const RANGE: [f64; 6] = [-7.0, -2.0, -1.0, 0.0, 1.0, 2.0];

/// The 2600's spring tank: a dispersive chain of all-passes (the chirp) feeding two springs
/// of different lengths, a little feedback each, dark and boingy.
struct Spring {
    ap: Vec<(DelayLine, usize)>,
    lines: [DelayLine; 2],
    len: [f64; 2],
    lp: [OnePole; 2],
    fb: [f64; 2],
}
impl Spring {
    fn new(fs: f64) -> Self {
        let ap = [0.0021, 0.0027, 0.0033, 0.0041, 0.0053].iter().map(|s| (DelayLine::new((s * fs) as usize + 4), (s * fs) as usize)).collect();
        let mut lp = [OnePole::default(), OnePole::default()];
        lp.iter_mut().for_each(|l| l.set_lowpass(fs, 3800.0));
        Self { ap, lines: [DelayLine::new((0.06 * fs) as usize), DelayLine::new((0.06 * fs) as usize)], len: [0.0331 * fs, 0.0413 * fs], lp, fb: [0.0; 2] }
    }
    fn reset(&mut self) {
        self.ap.iter_mut().for_each(|(d, _)| d.reset());
        self.lines.iter_mut().for_each(|d| d.reset());
        self.lp.iter_mut().for_each(|l| l.reset());
        self.fb = [0.0; 2];
    }
    fn process(&mut self, x: f64) -> (f64, f64) {
        let mut y = x;
        for (d, n) in self.ap.iter_mut() {
            let z = d.read_int(*n);
            let v = y + 0.6 * z;
            d.write(v);
            y = z - 0.6 * v;
        }
        let mut out = [0.0; 2];
        for i in 0..2 {
            let z = self.lines[i].read(self.len[i]);
            let z = self.lp[i].lp(z);
            self.lines[i].write(y + 0.72 * z);
            out[i] = z;
        }
        (out[0], out[1])
    }
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    held: Vec<u8>,
    pitch: f64,
    target: f64,
    ph: [f64; 3],
    clock: f64,
    sh: f64,
    adsr: Adsr,
    ar: Adsr,
    lad: Ladder,
    ota: Ladder,
    noise: Noise,
    nlp: [OnePole; 2],
    spring: Spring,
    bend: f64,
    dc: [DcBlock; 2],
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, held: Vec::new(), pitch: 60.0, target: 60.0, ph: [0.0, 0.31, 0.67], clock: 0.0, sh: 0.0, adsr: Adsr::default(), ar: Adsr::default(), lad: Ladder::default(), ota: Ladder::default(), noise: Noise::new(2600), nlp: [OnePole::default(), OnePole::default()], spring: Spring::new(fs), bend: 0.0, dc: [DcBlock::new(fs), DcBlock::new(fs)] }
    }
    fn reset(&mut self) {
        self.held.clear();
        self.adsr = Adsr::default();
        self.ar = Adsr::default();
        self.lad = Ladder::default();
        self.ota = Ladder::default();
        self.spring.reset();
        self.dc.iter_mut().for_each(|d| d.reset());
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
                self.target = *self.held.iter().min().unwrap() as f64;
                if first && self.v[PORT] <= 0.0 {
                    self.pitch = self.target;
                }
                self.adsr.on();
                self.ar.on();
            }
            Midi::Off { note } => {
                self.held.retain(|n| *n != note);
                match self.held.iter().min() {
                    Some(n) => self.target = *n as f64,
                    None => {
                        self.adsr.off();
                        self.ar.off();
                    }
                }
            }
            Midi::Bend { value } => self.bend = value as f64 * 2.0,
            Midi::AllOff => {
                self.held.clear();
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
        let (r1, r2, r3) = (RANGE[st(R1, 5)], RANGE[st(R2, 5)], RANGE[st(R3, 5)]);
        let kbd3 = p[KBD3] >= 0.5;
        let gc = if p[PORT] > 0.0 { coeff(fs, 2.0 + 2500.0 * (p[PORT] as f64).powi(2)) } else { 0.0 };
        let (a, d, s, r) = (rate(fs, p[A] as f64), rate(fs, p[D] as f64), p[S] as f64, rate(fs, p[R] as f64));
        let (ara, arr) = (rate(fs, p[ARA] as f64), rate(fs, p[ARR] as f64));
        // noise colour: white, through pink-ish, to a low rumble
        let col = p[NCOLOR] as f64;
        self.nlp[0].set_lowpass(fs, 20000.0 * 2f64.powf(-9.0 * col));
        self.nlp[1].set_lowpass(fs, 20000.0 * 2f64.powf(-9.0 * col));
        let k = 4.1 * p[RES] as f64;
        let four072 = p[FTYPE] >= 0.5;
        let vol = db_to_gain(p[VOLUME] as f64);
        let wet = p[REVERB] as f64;
        let mut peak = 0.0f64;
        for kk in 0..n {
            self.pitch = self.target + gc * (self.pitch - self.target);
            let note = self.pitch + self.bend;
            let w = self.noise.next();
            let nz = { let a1 = self.nlp[0].lp(w); self.nlp[1].lp(a1) } * (1.0 + 6.0 * col);
            self.clock += p[SHRATE] as f64 / fs;
            if self.clock >= 1.0 {
                self.clock -= 1.0;
                self.sh = nz.clamp(-1.0, 1.0);
            }
            let env = self.adsr.step(a, d, s, r);
            let arv = self.ar.step(ara, 1.0, 1.0, arr);
            // VCO-2 first: its sine feeds VCO-1's and the VCF's FM
            let sq1_prev = if self.ph[0] < 0.5 { 1.0 } else { -1.0 };
            let f2 = mtof(note + p[TUNE2] as f64 + 12.0 * (p[FM2SH] as f64 * self.sh + p[FM2V1] as f64 * sq1_prev)) * 2f64.powf(r2);
            let dt2 = (f2 / fs).min(0.45);
            let sine2 = (std::f64::consts::TAU * self.ph[1]).sin();
            let saw2 = saw(self.ph[1], dt2);
            let f1 = mtof(note + p[FINE1] as f64 + 12.0 * (p[FM1SH] as f64 * self.sh + p[FM1V2] as f64 * sine2)) * 2f64.powf(r1);
            let dt1 = (f1 / fs).min(0.45);
            let sq1 = pulse(self.ph[0], dt1, 0.5);
            let n3 = if kbd3 { note } else { 60.0 };
            let f3 = mtof(n3 + p[TUNE3] as f64 + 12.0 * (p[FM3N] as f64 * nz + 2.0 * p[FM3ENV] as f64 * env)) * 2f64.powf(r3);
            let dt3 = (f3 / fs).clamp(0.0, 0.45);
            let tri3 = tri(self.ph[2]);
            let ring = sq1 * saw2;
            for (ph, dt) in self.ph.iter_mut().zip([dt1, dt2, dt3]) {
                *ph = (*ph + dt).fract();
            }
            let x = (p[MRING] as f64 * ring + p[M1] as f64 * sq1 + p[M2] as f64 * saw2 + p[M3] as f64 * tri3 + p[MNOISE] as f64 * nz) * 0.5;
            let fc = p[CUTOFF] as f64 * 2f64.powf((note - 60.0) / 12.0 + 6.0 * p[FENV] as f64 * env + 3.0 * p[FV2] as f64 * sine2);
            let y = if four072 {
                self.ota.process(x * 0.8, fc, fs, k * 0.95, false).0 * (1.0 + 0.3 * k) * 1.2
            } else {
                self.lad.process(x, fc, fs, k, false).0 * (1.0 + 0.4 * k)
            };
            let amp = if p[VCA] >= 0.5 { env } else { arv };
            let dry = y * amp;
            let (sl, sr) = self.spring.process(dry);
            let l = self.dc[0].process(dry * (1.0 - 0.5 * wet) + sl * wet * 1.5) * vol * 1.4;
            let rr = self.dc[1].process(dry * (1.0 - 0.5 * wet) + sr * wet * 1.5) * vol * 1.4;
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
        Preset { name: "Three VCO Bass", values: &[(R1, 2.0), (R2, 2.0), (R3, 1.0), (M3, 0.6), (CUTOFF, 250.0), (RES, 0.35), (FENV, 0.5), (D, 250.0), (S, 0.1), (VCA, 1.0), (REVERB, 0.0)] },
        Preset { name: "R2-D2 Burble", values: &[(FM1SH, 0.6), (FM2SH, 0.4), (SHRATE, 14.0), (M1, 0.6), (M2, 0.0), (MRING, 0.5), (CUTOFF, 2500.0), (RES, 0.5), (FENV, 0.2), (REVERB, 0.3)] },
        Preset { name: "Ring Mod Clang", values: &[(MRING, 1.0), (M1, 0.0), (M2, 0.0), (R2, 4.0), (TUNE2, 5.3), (CUTOFF, 6000.0), (FENV, 0.0), (VCA, 1.0), (D, 900.0), (S, 0.0), (R, 900.0), (REVERB, 0.35)] },
        Preset { name: "Sample and Hold Filter", values: &[(FM1SH, 0.0), (SHRATE, 9.0), (M1, 0.5), (M2, 0.6), (CUTOFF, 700.0), (RES, 0.75), (FENV, 0.0), (FV2, 0.0), (MNOISE, 0.0), (REVERB, 0.25)] },
        Preset { name: "Thunder and Wind", values: &[(M1, 0.0), (M2, 0.0), (MNOISE, 1.0), (NCOLOR, 0.6), (CUTOFF, 400.0), (RES, 0.4), (FENV, 0.4), (A, 1200.0), (D, 3000.0), (S, 0.3), (R, 2000.0), (VCA, 1.0), (REVERB, 0.5)] },
        Preset { name: "Octave Lead", values: &[(M1, 0.4), (M2, 0.8), (TUNE2, 12.05), (CUTOFF, 1800.0), (RES, 0.3), (FENV, 0.3), (PORT, 0.05), (REVERB, 0.2)] },
        Preset { name: "VCO-3 Vibrato", values: &[(KBD3, 0.0), (R3, 0.0), (TUNE3, 6.0), (M3, 0.0), (FM1V2, 0.0), (FM3ENV, 0.0)] },
        Preset { name: "4072 Squelch", values: &[(FTYPE, 1.0), (M1, 0.8), (M2, 0.6), (CUTOFF, 300.0), (RES, 0.7), (FENV, 0.6), (D, 300.0), (S, 0.2), (VCA, 1.0)] },
    ]
}

const RANGES: &[&str] = &["LF", "32'", "16'", "8'", "4'", "2'"];

fn panel() -> Panel {
    let k = |p, x, y, r, label, tip| Item::Knob { p, x, y, r, label, tip, active: None };
    Panel {
        title: "ARP 2600",
        subtitle: ("SEMI-MODULAR, 1971 · PLAYED THROUGH ITS NORMALS", "3 VCO · RING MOD · S/H · 4012 / 4072 VCF · SPRING"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::Selector { p: R1, x: 50.0, y: 118.0, label: "VCO-1", positions: RANGES, tip: "VCO-1 range." },
            k(FINE1, 50.0, 202.0, 15.0, "FINE", "VCO-1 fine tune."),
            k(FM1SH, 120.0, 118.0, 15.0, "FM S/H", "VCO-1 FM from the sample and hold."),
            k(FM1V2, 120.0, 202.0, 15.0, "FM VCO-2", "VCO-1 FM from VCO-2's sine."),
            Item::Selector { p: R2, x: 200.0, y: 118.0, label: "VCO-2", positions: RANGES, tip: "VCO-2 range." },
            k(TUNE2, 200.0, 202.0, 15.0, "TUNE", "VCO-2 tuning."),
            k(FM2SH, 270.0, 118.0, 15.0, "FM S/H", "VCO-2 FM from the sample and hold."),
            k(FM2V1, 270.0, 202.0, 15.0, "FM VCO-1", "VCO-2 FM from VCO-1's square."),
            Item::Selector { p: R3, x: 350.0, y: 118.0, label: "VCO-3", positions: RANGES, tip: "VCO-3 range." },
            k(TUNE3, 350.0, 202.0, 15.0, "TUNE", "VCO-3 tuning (off the keyboard: its rate)."),
            Item::Switch { p: KBD3, x: 350.0, y: 243.0, labels: ["KBD OFF", "KBD ON"], tip: "VCO-3 follows the keyboard." },
            k(FM3N, 420.0, 118.0, 15.0, "FM NOISE", "VCO-3 FM from noise."),
            k(FM3ENV, 420.0, 202.0, 15.0, "FM ADSR", "VCO-3 FM from the ADSR."),
            k(NCOLOR, 500.0, 118.0, 15.0, "NOISE COL", "Noise colour: white to low rumble."),
            k(SHRATE, 500.0, 202.0, 15.0, "S/H CLOCK", "The sample and hold's internal clock."),
            k(VOLUME, 580.0, 118.0, 16.0, "VOLUME", "Output level."),
            k(REVERB, 580.0, 202.0, 16.0, "REVERB", "The spring reverb."),
            Item::Divider { y: 260.0 },
            k(MRING, 50.0, 298.0, 15.0, "VCF RING", "The VCF mixer: the ring modulator (VCO-1 by VCO-2)."),
            k(M1, 105.0, 298.0, 15.0, "VCO-1 SQ", "VCO-1's square."),
            k(M2, 160.0, 298.0, 15.0, "VCO-2 SAW", "VCO-2's saw."),
            k(M3, 215.0, 298.0, 15.0, "VCO-3 TRI", "VCO-3's triangle."),
            k(MNOISE, 270.0, 298.0, 15.0, "NOISE", "Noise."),
            k(CUTOFF, 345.0, 298.0, 18.0, "VCF", "Cutoff (the keyboard is normalled to its FM)."),
            k(RES, 410.0, 298.0, 16.0, "RES", "Resonance."),
            k(FENV, 465.0, 298.0, 15.0, "FM ADSR", "The ADSR at the VCF's FM."),
            k(FV2, 520.0, 298.0, 15.0, "FM VCO-2", "VCO-2's sine at the VCF's FM."),
            Item::Switch { p: FTYPE, x: 600.0, y: 298.0, labels: ["4012", "4072"], tip: "The early 4012 ladder or the later 4072." },
            Item::Divider { y: 340.0 },
            k(A, 50.0, 378.0, 15.0, "A", "ADSR attack."),
            k(D, 105.0, 378.0, 15.0, "D", "ADSR decay."),
            k(S, 160.0, 378.0, 15.0, "S", "ADSR sustain."),
            k(R, 215.0, 378.0, 15.0, "R", "ADSR release."),
            k(ARA, 300.0, 378.0, 15.0, "AR A", "AR attack."),
            k(ARR, 355.0, 378.0, 15.0, "AR R", "AR release."),
            k(PORT, 430.0, 378.0, 15.0, "PORTA", "The keyboard's portamento."),
            Item::Switch { p: VCA, x: 530.0, y: 378.0, labels: ["VCA AR", "VCA ADSR"], tip: "The VCA's envelope: the AR (normalled) or the ADSR." },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "ARP 2600", class_id: *b"HWArp2600Normals", vendor: "PEQUOD Hardware",
    instrument, categories: [Instrument, Synth]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        with(defaults(), &[(M1, 0.8), (M2, 0.0), (CUTOFF, 16000.0), (RES, 0.0), (FENV, 0.0), (VCA, 1.0), (S, 1.0), (REVERB, 0.0)])
    }

    include!("../../instrument_tests.rs");
}
