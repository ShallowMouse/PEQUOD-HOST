//! Dream Machine: in the manner of the Yamaha CS-80.
//!
//! Eight voices, each two complete synthesizers stacked - layer I and layer II - set up from
//! their own rows of controls and played together. Each layer: an oscillator (saw, square with
//! its pulse width, sine and noise, mixed), a resonant high-pass then low-pass filter (12 dB an
//! octave each, as the CS-80's), the filter's own envelope (initial level, attack level, attack,
//! decay, release) and an ADSR for the loudness. Over both: layer II's detune against layer I
//! (the CS-80's thickness), BRILLIANCE and RESONANCE for the whole instrument, the sub
//! oscillator (vibrato and filter wobble), and touch: velocity (initial touch) opens the filters
//! and plays louder, and the mod wheel stands in for the CS-80's aftertouch, adding brilliance
//! and vibrato as you lean on it. The ribbon is the pitch bend.

#![allow(clippy::excessive_precision)]
use nih_plug::prelude::*;
use std::f64::consts::{PI, TAU};
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
pub const H: u32 = 640;

const VOICES: usize = 8;
const LAYERS: usize = 2;

fn time(name: String, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms")
}
fn level(name: String, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 }).with_unit(" %").with_value_to_string(formatters::v2s_f32_percentage(0)).with_string_to_value(formatters::s2v_f32_percentage())
}
fn freq(name: String, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(-2.0) }).with_unit(" Hz")
}

/// One layer's defaults: feet, pw, saw, square, sine, noise, hpf, hpf res, lpf, lpf res,
/// IL, AL, filter A, D, R, amp A, D, S, R, level.
type LayerInit = (f32, f32, f32, f32, f32, f32, f32, f32, f32, f32, f32, f32, f32, f32, f32, f32, f32, f32, f32, f32);
const INIT: [LayerInit; LAYERS] = [
    (1.0, 0.35, 1.0, 0.0, 0.0, 0.0, 30.0, 0.0, 1400.0, 0.3, 0.0, 0.7, 120.0, 900.0, 800.0, 60.0, 600.0, 0.8, 900.0, 0.8),
    (1.0, 0.25, 0.0, 0.8, 0.3, 0.0, 60.0, 0.0, 2200.0, 0.2, 0.0, 0.5, 300.0, 1500.0, 1000.0, 120.0, 800.0, 0.8, 1100.0, 0.6),
];

#[derive(Params)]
pub struct LayerParams {
    #[id = "feet"]
    pub feet: FloatParam,
    #[id = "pw"]
    pub pw: FloatParam,
    #[id = "saw"]
    pub saw: FloatParam,
    #[id = "square"]
    pub square: FloatParam,
    #[id = "sine"]
    pub sine: FloatParam,
    #[id = "noise"]
    pub noise: FloatParam,
    #[id = "hpf"]
    pub hpf: FloatParam,
    #[id = "hpres"]
    pub hpres: FloatParam,
    #[id = "lpf"]
    pub lpf: FloatParam,
    #[id = "lpres"]
    pub lpres: FloatParam,
    #[id = "il"]
    pub il: FloatParam,
    #[id = "al"]
    pub al: FloatParam,
    #[id = "fa"]
    pub fa: FloatParam,
    #[id = "fd"]
    pub fd: FloatParam,
    #[id = "fr"]
    pub fr: FloatParam,
    #[id = "va"]
    pub va: FloatParam,
    #[id = "vd"]
    pub vd: FloatParam,
    #[id = "vs"]
    pub vs: FloatParam,
    #[id = "vr"]
    pub vr: FloatParam,
    #[id = "level"]
    pub level: FloatParam,
}

impl LayerParams {
    fn new(l: usize) -> Self {
        let n = |s: &str| format!("{} {s}", if l == 0 { "I" } else { "II" });
        let d = INIT[l];
        Self {
            feet: FloatParam::new(n("Feet"), d.0, FloatRange::Linear { min: 0.0, max: 2.0 }).with_step_size(1.0),
            pw: level(n("Pulse Width"), d.1),
            saw: level(n("Saw"), d.2),
            square: level(n("Square"), d.3),
            sine: level(n("Sine"), d.4),
            noise: level(n("Noise"), d.5),
            hpf: freq(n("High-Pass"), d.6, 20.0, 4000.0),
            hpres: level(n("High-Pass Resonance"), d.7),
            lpf: freq(n("Low-Pass"), d.8, 40.0, 18000.0),
            lpres: level(n("Low-Pass Resonance"), d.9),
            il: level(n("Initial Level"), d.10),
            al: level(n("Attack Level"), d.11),
            fa: time(n("Filter Attack"), d.12),
            fd: time(n("Filter Decay"), d.13),
            fr: time(n("Filter Release"), d.14),
            va: time(n("Attack"), d.15),
            vd: time(n("Decay"), d.16),
            vs: level(n("Sustain"), d.17),
            vr: time(n("Release"), d.18),
            level: level(n("Level"), d.19),
        }
    }
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "detune"]
    pub detune: FloatParam,
    #[id = "brill"]
    pub brill: FloatParam,
    #[id = "reso"]
    pub reso: FloatParam,
    #[id = "subspeed"]
    pub subspeed: FloatParam,
    #[id = "subvco"]
    pub subvco: FloatParam,
    #[id = "subvcf"]
    pub subvcf: FloatParam,
    #[id = "touch"]
    pub touch: FloatParam,
    #[id = "after"]
    pub after: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
    #[nested(array, group = "Layer")]
    pub layers: [LayerParams; LAYERS],
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            detune: FloatParam::new("Detune", 6.0, FloatRange::Linear { min: -50.0, max: 50.0 }).with_step_size(0.5).with_unit(" ct"),
            brill: FloatParam::new("Brilliance", 0.0, FloatRange::Linear { min: -1.0, max: 1.0 }),
            reso: pct("Resonance", 0.0),
            subspeed: rate("Sub Osc Speed", 5.5, 0.1, 20.0),
            subvco: pct("Sub Osc VCO", 0.0),
            subvcf: pct("Sub Osc VCF", 0.0),
            touch: pct("Initial Touch", 0.5),
            after: pct("After Touch", 0.5),
            volume: db("Volume", -6.0, -30.0, 6.0),
            layers: [LayerParams::new(0), LayerParams::new(1)],
        }
    }
}

pub const DETUNE: usize = 0;
pub const BRILL: usize = 1;
pub const RESO: usize = 2;
pub const SUBSPEED: usize = 3;
pub const SUBVCO: usize = 4;
pub const SUBVCF: usize = 5;
pub const TOUCH: usize = 6;
pub const AFTER: usize = 7;
pub const VOLUME: usize = 8;
const G: usize = 9;
const PER: usize = 20;
/// Layer `l` (0 = I, 1 = II), field `f` (FEET .. LEVEL).
pub const fn ly(l: usize, f: usize) -> usize {
    G + l * PER + f
}
pub const FEET: usize = 0;
pub const PW: usize = 1;
pub const SAW: usize = 2;
pub const SQUARE: usize = 3;
pub const SINE: usize = 4;
pub const NOISE: usize = 5;
pub const HPF: usize = 6;
pub const HPRES: usize = 7;
pub const LPF: usize = 8;
pub const LPRES: usize = 9;
pub const IL: usize = 10;
pub const AL: usize = 11;
pub const FA: usize = 12;
pub const FD: usize = 13;
pub const FR: usize = 14;
pub const VA: usize = 15;
pub const VD: usize = 16;
pub const VS: usize = 17;
pub const VR: usize = 18;
pub const LEVEL: usize = 19;
pub const NPARAMS: usize = G + LAYERS * PER;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        let mut v = vec![
            self.detune.as_ptr(),
            self.brill.as_ptr(),
            self.reso.as_ptr(),
            self.subspeed.as_ptr(),
            self.subvco.as_ptr(),
            self.subvcf.as_ptr(),
            self.touch.as_ptr(),
            self.after.as_ptr(),
            self.volume.as_ptr(),
        ];
        for l in &self.layers {
            v.extend([
                l.feet.as_ptr(), l.pw.as_ptr(), l.saw.as_ptr(), l.square.as_ptr(), l.sine.as_ptr(), l.noise.as_ptr(),
                l.hpf.as_ptr(), l.hpres.as_ptr(), l.lpf.as_ptr(), l.lpres.as_ptr(),
                l.il.as_ptr(), l.al.as_ptr(), l.fa.as_ptr(), l.fd.as_ptr(), l.fr.as_ptr(),
                l.va.as_ptr(), l.vd.as_ptr(), l.vs.as_ptr(), l.vr.as_ptr(), l.level.as_ptr(),
            ]);
        }
        v
    }
}

/// Default plain values in linked order (for tests).
#[allow(dead_code)]
pub fn defaults() -> Vec<f32> {
    let p = P::default();
    p.linked().iter().map(|q| unsafe { q.preview_plain(q.default_normalized_value()) }).collect()
}

// -------------------------------------------------------------------- dsp --

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

/// A 12 dB/octave state-variable filter (topology-preserving), with a little drive.
#[derive(Clone, Copy, Default)]
struct Svf {
    ic1: f64,
    ic2: f64,
    a1: f64,
    a2: f64,
    a3: f64,
    k: f64,
}

impl Svf {
    fn set(&mut self, fs: f64, fc: f64, res: f64) {
        let g = (PI * fc.clamp(10.0, 0.45 * fs) / fs).tan();
        self.k = 2.0 - 1.95 * res.clamp(0.0, 1.0);
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }
    /// (low, high)
    #[inline]
    fn run(&mut self, x: f64) -> (f64, f64) {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = (2.0 * v1 - self.ic1).clamp(-8.0, 8.0);
        self.ic2 = (2.0 * v2 - self.ic2).clamp(-8.0, 8.0);
        (v2, x - self.k * v1 - v2)
    }
}

#[derive(Clone, Copy, Default)]
struct Layer {
    ph: f64,
    hp: Svf,
    lp: Svf,
    fenv: f64,
    fstage: u8, // 0 attack (IL -> AL), 1 decay (to 0), 2 release
    aenv: f64,
    astage: u8, // 0 attack, 1 decay/sustain, 2 release
}

#[derive(Clone, Copy, Default)]
struct Voice {
    on: bool,
    held: bool,
    note: u8,
    age: u64,
    vel: f64,
    layers: [Layer; LAYERS],
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    voices: [Voice; VOICES],
    clock: u64,
    noise: Noise,
    sub: f64,
    bend: f64,
    after: f64,
    pedal: bool,
    tick: u32,
    dc: [DcBlock; 2],
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, voices: [Voice::default(); VOICES], clock: 0, noise: Noise::new(80), sub: 0.0, bend: 0.0, after: 0.0, pedal: false, tick: 0, dc: [DcBlock::new(fs), DcBlock::new(fs)] }
    }
    fn reset(&mut self) {
        self.voices = [Voice::default(); VOICES];
        self.dc.iter_mut().for_each(|d| d.reset());
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn midi(&mut self, ev: Midi) {
        match ev {
            Midi::On { note, vel } => {
                self.clock += 1;
                let i = self.voices.iter().position(|v| v.on && v.note == note).or_else(|| self.voices.iter().position(|v| !v.on)).unwrap_or_else(|| self.voices.iter().enumerate().min_by_key(|(_, v)| v.age).map(|(i, _)| i).unwrap_or(0));
                let il = [self.v[ly(0, IL)] as f64, self.v[ly(1, IL)] as f64];
                let vc = &mut self.voices[i];
                let fresh = !(vc.on && vc.note == note);
                vc.on = true;
                vc.held = true;
                vc.note = note;
                vc.age = self.clock;
                vc.vel = vel as f64;
                for (l, s) in vc.layers.iter_mut().enumerate() {
                    if fresh {
                        *s = Layer::default();
                    }
                    s.fenv = il[l];
                    s.fstage = 0;
                    s.astage = 0;
                }
            }
            Midi::Off { note } => self.voices.iter_mut().filter(|v| v.on && v.note == note && v.held).for_each(|v| v.held = false),
            Midi::Cc { cc: 1, value } => self.after = value as f64,
            Midi::Cc { cc: 64, value } => self.pedal = value >= 0.5,
            Midi::Bend { value } => self.bend = value as f64 * 2.0,
            Midi::AllOff => self.voices.iter_mut().for_each(|v| v.on = false),
            _ => {}
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let p = self.v;
        let c = |ms: f32| 1.0 - (-1.0 / (ms.max(1.0) as f64 * 0.001 * fs / 2.2)).exp();
        // per layer, the block's constants
        struct L {
            oct: f64,
            pw: f64,
            saw: f64,
            sq: f64,
            sine: f64,
            nz: f64,
            hpf: f64,
            hpres: f64,
            lpf: f64,
            lpres: f64,
            al: f64,
            fa: f64,
            fd: f64,
            fr: f64,
            va: f64,
            vd: f64,
            vs: f64,
            vr: f64,
            level: f64,
            det: f64,
        }
        let lay: Vec<L> = (0..LAYERS)
            .map(|l| L {
                oct: [-1.0, 0.0, 1.0][p[ly(l, FEET)].round().clamp(0.0, 2.0) as usize],
                pw: 0.05 + 0.45 * p[ly(l, PW)] as f64,
                saw: p[ly(l, SAW)] as f64,
                sq: p[ly(l, SQUARE)] as f64,
                sine: p[ly(l, SINE)] as f64,
                nz: p[ly(l, NOISE)] as f64,
                hpf: p[ly(l, HPF)] as f64,
                hpres: (p[ly(l, HPRES)] as f64 + p[RESO] as f64).min(1.0),
                lpf: p[ly(l, LPF)] as f64,
                lpres: (p[ly(l, LPRES)] as f64 + p[RESO] as f64).min(1.0),
                al: p[ly(l, AL)] as f64,
                fa: c(p[ly(l, FA)]),
                fd: c(p[ly(l, FD)]),
                fr: c(p[ly(l, FR)]),
                va: c(p[ly(l, VA)]),
                vd: c(p[ly(l, VD)]),
                vs: p[ly(l, VS)] as f64,
                vr: c(p[ly(l, VR)]),
                level: p[ly(l, LEVEL)] as f64,
                det: if l == 1 { p[DETUNE] as f64 / 100.0 } else { 0.0 },
            })
            .collect();
        let brill = p[BRILL] as f64 * 2.0;
        let (subspeed, subvco, subvcf) = (p[SUBSPEED] as f64, p[SUBVCO] as f64, p[SUBVCF] as f64);
        let (touch, after_amt) = (p[TOUCH] as f64, p[AFTER] as f64);
        let vol = db_to_gain(p[VOLUME] as f64);
        let pedal = self.pedal;
        let mut peak = 0.0f64;
        for k in 0..n {
            self.sub = (self.sub + subspeed / fs).fract();
            let s = (TAU * self.sub).sin();
            let after = self.after * after_amt;
            // vibrato: the sub oscillator, and leaning on the keys (after touch) brings in more
            let vib = s * (subvco * 0.6 + after * 0.25);
            let wob = s * subvcf * 1.5;
            let retune = self.tick % 16 == 0;
            self.tick = self.tick.wrapping_add(1);
            let mut out = 0.0;
            for vc in self.voices.iter_mut().filter(|v| v.on) {
                let gate = vc.held || pedal;
                let touch_oct = touch * (vc.vel - 0.5) * 2.0;
                let mut alive = false;
                for (l, (st, c)) in vc.layers.iter_mut().zip(lay.iter()).enumerate() {
                    if c.level <= 0.0 && l > 0 {
                        continue;
                    }
                    // envelopes
                    if !gate && st.astage < 2 {
                        st.astage = 2;
                        st.fstage = 2;
                    }
                    match st.fstage {
                        0 => {
                            st.fenv += (c.al - st.fenv) * c.fa;
                            if (st.fenv - c.al).abs() < 1e-3 {
                                st.fstage = 1;
                            }
                        }
                        1 => st.fenv -= st.fenv * c.fd,
                        _ => st.fenv -= st.fenv * c.fr,
                    }
                    match st.astage {
                        0 => {
                            st.aenv += (1.3 - st.aenv) * c.va;
                            if st.aenv >= 1.0 {
                                st.aenv = 1.0;
                                st.astage = 1;
                            }
                        }
                        1 => st.aenv += (c.vs - st.aenv) * c.vd,
                        _ => st.aenv -= st.aenv * c.vr,
                    }
                    if st.aenv > 1e-4 || st.astage < 2 {
                        alive = true;
                    }
                    // the oscillator
                    let f = 440.0 * 2f64.powf((vc.note as f64 + self.bend + c.det - 69.0) / 12.0 + c.oct + vib / 12.0);
                    let dt = (f / fs).min(0.45);
                    let ph = st.ph;
                    let mut x = 0.0;
                    if c.saw > 0.0 {
                        x += c.saw * (2.0 * ph - 1.0 - blep(ph, dt));
                    }
                    if c.sq > 0.0 {
                        let mut q = if ph < c.pw { 1.0 } else { -1.0 };
                        q += blep(ph, dt);
                        q -= blep((ph - c.pw + 1.0).fract(), dt);
                        x += c.sq * (q - (2.0 * c.pw - 1.0));
                    }
                    if c.sine > 0.0 {
                        x += c.sine * (TAU * ph).sin();
                    }
                    if c.nz > 0.0 {
                        x += c.nz * self.noise.next();
                    }
                    st.ph = (ph + dt).fract();
                    // the filters: high-pass then low-pass, both opened by the filter envelope,
                    // brilliance, touch, after touch and the sub oscillator's wobble
                    if retune {
                        let open = 2f64.powf(5.0 * st.fenv + brill + touch_oct + 1.5 * after + wob);
                        st.lp.set(fs, c.lpf * open, c.lpres);
                        st.hp.set(fs, c.hpf * open.sqrt(), c.hpres);
                    }
                    let (_, h) = st.hp.run(x);
                    let (lo, _) = st.lp.run(h);
                    let loud = 1.0 + touch * (vc.vel - 0.5);
                    out += lo * st.aenv * c.level * loud;
                }
                if !alive {
                    vc.on = false;
                }
            }
            let y = self.dc[0].process(out * 0.18) * vol;
            peak = peak.max(y.abs());
            for ch in chans.iter_mut() {
                ch[k] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}

// ------------------------------------------------------------------ panel --

macro_rules! layer {
    ($l:expr, [$($f:expr => $v:expr),* $(,)?]) => { [$((ly($l, $f), $v)),*] };
}

fn presets() -> Vec<Preset> {
    // the brass the CS-80 is famous for, its strings, a bell, and a lead
    const BRASS: &[(usize, f32)] = &{
        const A: [(usize, f32); 12] = layer!(0, [SAW => 1.0, SQUARE => 0.0, LPF => 700.0, LPRES => 0.25, IL => 0.0, AL => 0.8, FA => 90.0, FD => 600.0, VA => 40.0, VS => 0.9, VR => 400.0, LEVEL => 0.8]);
        const B: [(usize, f32); 8] = layer!(1, [SAW => 1.0, SQUARE => 0.0, SINE => 0.0, LPF => 900.0, AL => 0.7, FA => 150.0, VA => 60.0, LEVEL => 0.6]);
        let mut out = [(0usize, 0.0f32); 22];
        let mut i = 0;
        while i < 12 {
            out[i] = A[i];
            i += 1;
        }
        while i < 20 {
            out[i] = B[i - 12];
            i += 1;
        }
        out[20] = (DETUNE, 8.0);
        out[21] = (SUBVCO, 0.15);
        out
    };
    const STRINGS: &[(usize, f32)] = &{
        const A: [(usize, f32); 9] = layer!(0, [SAW => 1.0, SQUARE => 0.0, LPF => 2500.0, AL => 0.3, FA => 400.0, VA => 500.0, VS => 0.9, VR => 1800.0, LEVEL => 0.7]);
        const B: [(usize, f32); 9] = layer!(1, [SAW => 0.0, SQUARE => 1.0, PW => 0.3, LPF => 2800.0, VA => 700.0, VS => 0.9, VR => 2000.0, LEVEL => 0.7, FEET => 2.0]);
        let mut out = [(0usize, 0.0f32); 20];
        let mut i = 0;
        while i < 9 {
            out[i] = A[i];
            i += 1;
        }
        while i < 18 {
            out[i] = B[i - 9];
            i += 1;
        }
        out[18] = (DETUNE, 12.0);
        out[19] = (SUBVCO, 0.2);
        out
    };
    const BELL: &[(usize, f32)] = &{
        const A: [(usize, f32); 8] = layer!(0, [SAW => 0.0, SQUARE => 0.0, SINE => 1.0, LPF => 8000.0, VA => 1.0, VD => 2500.0, VS => 0.0, VR => 2500.0]);
        const B: [(usize, f32); 9] = layer!(1, [SQUARE => 0.6, SINE => 0.5, FEET => 2.0, LPF => 5000.0, VA => 1.0, VD => 900.0, VS => 0.0, VR => 900.0, LEVEL => 0.5]);
        let mut out = [(0usize, 0.0f32); 18];
        let mut i = 0;
        while i < 8 {
            out[i] = A[i];
            i += 1;
        }
        while i < 17 {
            out[i] = B[i - 8];
            i += 1;
        }
        out[17] = (DETUNE, 30.0);
        out
    };
    const LEAD: &[(usize, f32)] = &{
        const A: [(usize, f32); 8] = layer!(0, [SAW => 1.0, LPF => 1200.0, LPRES => 0.6, AL => 0.6, FA => 40.0, FD => 400.0, VS => 1.0, LEVEL => 0.9]);
        const B: [(usize, f32); 1] = layer!(1, [LEVEL => 0.0]);
        let mut out = [(0usize, 0.0f32); 11];
        let mut i = 0;
        while i < 8 {
            out[i] = A[i];
            i += 1;
        }
        out[8] = B[0];
        out[9] = (SUBVCO, 0.25);
        out[10] = (AFTER, 1.0);
        out
    };
    vec![
        Preset { name: "Brass (the famous one)", values: BRASS },
        Preset { name: "Slow Strings", values: STRINGS },
        Preset { name: "Bells", values: BELL },
        Preset { name: "Expressive Lead", values: LEAD },
    ]
}

fn panel() -> Panel {
    const LABELS: [&str; PER] = [
        "FEET", "PW", "SAW", "SQUARE", "SINE", "NOISE", "HPF", "HP RES", "LPF", "LP RES",
        "IL", "AL", "F.ATK", "F.DEC", "F.REL", "ATTACK", "DECAY", "SUSTAIN", "RELEASE", "LEVEL",
    ];
    const TIPS: [&str; PER] = [
        "Feet: 16' (an octave down), 8', 4' (an octave up).",
        "Pulse width of the square.",
        "Saw level.",
        "Square level.",
        "Sine level.",
        "Noise level.",
        "High-pass cutoff.",
        "High-pass resonance.",
        "Low-pass cutoff.",
        "Low-pass resonance.",
        "Filter envelope: the initial level, where the filter starts.",
        "Filter envelope: the attack level, where it rises to (then decays back).",
        "Filter envelope attack time.",
        "Filter envelope decay time.",
        "Filter envelope release time.",
        "Loudness attack.",
        "Loudness decay.",
        "Loudness sustain.",
        "Loudness release.",
        "This layer's level in the mix.",
    ];
    let mut items = vec![
        Item::Knob { p: DETUNE, x: 50.0, y: 140.0, r: 18.0, label: "DETUNE", tip: "Layer II's tuning against layer I: a few cents for the CS-80's thickness.", active: None },
        Item::Knob { p: BRILL, x: 115.0, y: 140.0, r: 18.0, label: "BRILLIANCE", tip: "Opens or closes every filter at once.", active: None },
        Item::Knob { p: RESO, x: 185.0, y: 140.0, r: 18.0, label: "RESONANCE", tip: "Adds resonance to every filter.", active: None },
        Item::Knob { p: SUBSPEED, x: 255.0, y: 140.0, r: 18.0, label: "SUB SPEED", tip: "The sub oscillator's speed.", active: None },
        Item::Knob { p: SUBVCO, x: 320.0, y: 140.0, r: 18.0, label: "SUB VCO", tip: "Sub oscillator to pitch: vibrato.", active: None },
        Item::Knob { p: SUBVCF, x: 385.0, y: 140.0, r: 18.0, label: "SUB VCF", tip: "Sub oscillator to the filters: wah.", active: None },
        Item::Knob { p: TOUCH, x: 450.0, y: 140.0, r: 18.0, label: "TOUCH", tip: "Initial touch: how much velocity opens the filters and plays louder.", active: None },
        Item::Knob { p: AFTER, x: 515.0, y: 140.0, r: 18.0, label: "AFTER", tip: "After touch (the mod wheel): how much leaning on it brightens and adds vibrato.", active: None },
        Item::Knob { p: VOLUME, x: 585.0, y: 140.0, r: 18.0, label: "VOLUME", tip: "Output level.", active: None },
        Item::Divider { y: 190.0 },
    ];
    for l in 0..LAYERS {
        for f in 0..PER {
            let row = f / 10;
            let x = 45.0 + (f % 10) as f32 * 61.0;
            let y = 246.0 + l as f32 * 214.0 + row as f32 * 86.0;
            items.push(Item::Knob { p: ly(l, f), x, y, r: 14.0, label: LABELS[f], tip: TIPS[f], active: None });
        }
        if l == 0 {
            items.push(Item::Divider { y: 404.0 });
        }
    }
    items.push(Item::Text { x: 320.0, y: 197.0, text: "LAYER I", size: 10.0, align: Align2::CENTER_CENTER });
    items.push(Item::Text { x: 320.0, y: 411.0, text: "LAYER II", size: 10.0, align: Align2::CENTER_CENTER });
    Panel {
        title: "DREAM MACHINE",
        subtitle: ("IN THE MANNER OF THE YAMAHA CS-80", "8 VOICES · TWO LAYERS · TOUCH · SUB OSCILLATOR"),
        accent: ui::MOSS,
        size: (W, H),
        items,
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Dream Machine", class_id: *b"PQKeysDreamCS80Y", vendor: "PEQUOD Keys",
    instrument, categories: [Instrument, Synth]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        with(
            defaults(),
            &[
                (ly(0, SAW), 1.0), (ly(0, SQUARE), 0.0), (ly(0, LPF), 18000.0), (ly(0, HPF), 20.0), (ly(0, IL), 0.0), (ly(0, AL), 0.0),
                (ly(0, VS), 1.0), (ly(0, VR), 100.0), (ly(0, FEET), 1.0), (ly(1, LEVEL), 0.0), (SUBVCO, 0.0), (SUBVCF, 0.0),
            ],
        )
    }

    include!("../../instrument_tests.rs");
}
