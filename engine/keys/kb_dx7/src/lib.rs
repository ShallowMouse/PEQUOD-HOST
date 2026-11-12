//! FM 7: a six-operator FM synthesizer in the manner of the Yamaha DX7.
//!
//! Six sine-wave operators, each with its own frequency ratio, detune, output level, envelope
//! and velocity sensitivity. Some are carriers (you hear them); the rest are modulators, which
//! bend their carriers' phase to make the harmonics. Which is which is set by the algorithm:
//! eight of the DX7's 32 are here, with their own numbers, covering the patches people use
//! most (the electric piano is 5, brass 22, the organ 32). One operator in each feeds back on
//! itself, from a pure tone up to a buzzing saw and noise.
//!
//! As on the DX7, levels run 0-99 with about 0.75 dB a step, so envelopes fall in straight
//! lines of decibels, and velocity can cut a modulator for soft, dull notes and bright hard ones.

#![allow(clippy::excessive_precision)]
use nih_plug::prelude::*;
use std::f64::consts::TAU;
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
pub const H: u32 = 735;

const VOICES: usize = 16;
const OPS: usize = 6;

#[derive(Params)]
pub struct OpParams {
    #[id = "ratio"]
    pub ratio: FloatParam,
    #[id = "detune"]
    pub detune: FloatParam,
    #[id = "level"]
    pub level: FloatParam,
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "sustain"]
    pub sustain: FloatParam,
    #[id = "release"]
    pub release: FloatParam,
    #[id = "velsens"]
    pub velsens: FloatParam,
}

fn time(name: String, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Skewed { min: 1.0, max: 20000.0, factor: FloatRange::skew_factor(-2.4) }).with_step_size(1.0).with_unit(" ms")
}
fn steps(name: String, default: f32, min: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min, max }).with_step_size(1.0)
}

/// The default patch is the tine electric piano (algorithm 5): per operator
/// (ratio, detune, level, attack, decay, sustain, release, velocity).
const INIT: [(f32, f32, f32, f32, f32, f32, f32, f32); OPS] = [
    (1.0, 3.0, 99.0, 1.0, 4000.0, 0.0, 400.0, 2.0),
    (14.0, 0.0, 60.0, 1.0, 350.0, 0.0, 300.0, 7.0),
    (1.0, 0.0, 99.0, 1.0, 5000.0, 0.0, 400.0, 2.0),
    (1.0, 0.0, 85.0, 1.0, 2500.0, 0.0, 400.0, 6.0),
    (1.0, -3.0, 95.0, 1.0, 4000.0, 0.0, 400.0, 2.0),
    (1.0, 0.0, 70.0, 1.0, 1500.0, 0.0, 400.0, 6.0),
];

impl OpParams {
    fn new(op: usize) -> Self {
        let n = |s: &str| format!("Op {op} {s}");
        let d = INIT[op - 1];
        Self {
            ratio: FloatParam::new(n("Ratio"), d.0, FloatRange::Skewed { min: 0.5, max: 31.0, factor: FloatRange::skew_factor(-1.6) }).with_step_size(0.01).with_unit(" x"),
            detune: steps(n("Detune"), d.1, -7.0, 7.0),
            level: steps(n("Level"), d.2, 0.0, 99.0),
            attack: time(n("Attack"), d.3),
            decay: time(n("Decay"), d.4),
            sustain: steps(n("Sustain"), d.5, 0.0, 99.0),
            release: time(n("Release"), d.6),
            velsens: steps(n("Velocity"), d.7, 0.0, 7.0),
        }
    }
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "algorithm"]
    pub algorithm: FloatParam,
    #[id = "feedback"]
    pub feedback: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
    #[nested(array, group = "Operator")]
    pub ops: [OpParams; OPS],
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            algorithm: steps("Algorithm".into(), 2.0, 0.0, 7.0),
            feedback: steps("Feedback".into(), 6.0, 0.0, 7.0),
            volume: db("Volume", -6.0, -30.0, 6.0),
            ops: [OpParams::new(1), OpParams::new(2), OpParams::new(3), OpParams::new(4), OpParams::new(5), OpParams::new(6)],
        }
    }
}

pub const ALG: usize = 0;
pub const FB: usize = 1;
pub const VOLUME: usize = 2;
/// Operator `op` (1-6), field `f` (RATIO .. VELSENS).
pub const fn op(op: usize, f: usize) -> usize {
    3 + (op - 1) * 8 + f
}
pub const RATIO: usize = 0;
pub const DETUNE: usize = 1;
pub const LEVEL: usize = 2;
pub const ATTACK: usize = 3;
pub const DECAY: usize = 4;
pub const SUSTAIN: usize = 5;
pub const RELEASE: usize = 6;
pub const VELSENS: usize = 7;
pub const NPARAMS: usize = 3 + 8 * OPS;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        let mut v = vec![self.algorithm.as_ptr(), self.feedback.as_ptr(), self.volume.as_ptr()];
        for o in &self.ops {
            v.extend([o.ratio.as_ptr(), o.detune.as_ptr(), o.level.as_ptr(), o.attack.as_ptr(), o.decay.as_ptr(), o.sustain.as_ptr(), o.release.as_ptr(), o.velsens.as_ptr()]);
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

/// An algorithm: for each operator (index 0 = op 1), a bitmask of the operators that modulate
/// it; which are carriers; and which one feeds back on itself. Modulators always have higher
/// numbers than what they modulate, so computing 6 down to 1 works for all of these.
struct Algorithm {
    #[allow(dead_code)]
    number: u8,
    mods: [u8; OPS],
    carriers: u8,
    feedback: usize,
}

const fn b(ops: &[usize]) -> u8 {
    let mut m = 0u8;
    let mut i = 0;
    while i < ops.len() {
        m |= 1 << (ops[i] - 1);
        i += 1;
    }
    m
}

const ALGORITHMS: [Algorithm; 8] = [
    Algorithm { number: 1, mods: [b(&[2]), 0, b(&[4]), b(&[5]), b(&[6]), 0], carriers: b(&[1, 3]), feedback: 6 },
    Algorithm { number: 2, mods: [b(&[2]), 0, b(&[4]), b(&[5]), b(&[6]), 0], carriers: b(&[1, 3]), feedback: 2 },
    Algorithm { number: 5, mods: [b(&[2]), 0, b(&[4]), 0, b(&[6]), 0], carriers: b(&[1, 3, 5]), feedback: 6 },
    Algorithm { number: 7, mods: [b(&[2]), 0, b(&[4, 5]), 0, b(&[6]), 0], carriers: b(&[1, 3]), feedback: 6 },
    Algorithm { number: 16, mods: [b(&[2, 3, 5]), 0, b(&[4]), 0, b(&[6]), 0], carriers: b(&[1]), feedback: 6 },
    Algorithm { number: 19, mods: [b(&[2]), b(&[3]), 0, b(&[6]), b(&[6]), 0], carriers: b(&[1, 4, 5]), feedback: 6 },
    Algorithm { number: 22, mods: [b(&[2]), 0, b(&[6]), b(&[6]), b(&[6]), 0], carriers: b(&[1, 3, 4, 5]), feedback: 6 },
    Algorithm { number: 32, mods: [0; OPS], carriers: b(&[1, 2, 3, 4, 5, 6]), feedback: 6 },
];

/// DX7 output level (0-99) to amplitude: about 0.75 dB a step, 99 = full.
#[inline]
fn level_amp(l: f64) -> f64 {
    if l < 1.0 { 0.0 } else { 2f64.powf((l - 99.0) / 8.0) }
}

#[derive(Clone, Copy, Default)]
struct Op {
    ph: f64,
    env: f64,
    stage: u8, // 0 attack, 1 decay/sustain, 2 release, 3 done
    out: f64,
    vel_cut: f64,
}

#[derive(Clone, Copy, Default)]
struct Voice {
    on: bool,
    held: bool,
    note: u8,
    age: u64,
    f: f64,
    ops: [Op; OPS],
    fb: [f64; 2],
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    voices: [Voice; VOICES],
    clock: u64,
    pedal: bool,
    bend: f64,
    dc: DcBlock,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, voices: [Voice::default(); VOICES], clock: 0, pedal: false, bend: 0.0, dc: DcBlock::new(fs) }
    }
    fn reset(&mut self) {
        self.voices = [Voice::default(); VOICES];
        self.dc.reset();
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn midi(&mut self, ev: Midi) {
        match ev {
            Midi::On { note, vel } => {
                self.clock += 1;
                let i = self.voices.iter().position(|v| v.on && v.note == note).or_else(|| self.voices.iter().position(|v| !v.on)).unwrap_or_else(|| self.voices.iter().enumerate().min_by_key(|(_, v)| v.age).map(|(i, _)| i).unwrap_or(0));
                let vel = vel as f64;
                let p = self.v;
                let vc = &mut self.voices[i];
                vc.on = true;
                vc.held = true;
                vc.note = note;
                vc.age = self.clock;
                vc.f = 440.0 * 2f64.powf((note as f64 - 69.0) / 12.0);
                vc.fb = [0.0; 2];
                for (o, s) in vc.ops.iter_mut().enumerate() {
                    // DX7 oscillators are key-synced: every note starts at phase zero
                    s.ph = 0.0;
                    s.env = 0.0;
                    s.stage = 0;
                    s.out = 0.0;
                    s.vel_cut = p[op(o + 1, VELSENS)] as f64 / 7.0 * (1.0 - vel) * 32.0;
                }
            }
            Midi::Off { note } => self.voices.iter_mut().filter(|v| v.on && v.note == note && v.held).for_each(|v| v.held = false),
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
        let alg = &ALGORITHMS[(p[ALG].round().clamp(0.0, 7.0)) as usize];
        let fb_n = p[FB].round();
        let beta = if fb_n < 1.0 { 0.0 } else { std::f64::consts::PI * 2f64.powf(fb_n as f64 - 7.0) * 1.6 };
        let ncar = alg.carriers.count_ones() as f64;
        let out_gain = db_to_gain(p[VOLUME] as f64) * 0.3 / ncar.sqrt();
        let bend = 2f64.powf(self.bend / 12.0);
        // per-operator constants for this block
        let mut ratio = [0.0; OPS];
        let mut level = [0.0; OPS];
        let mut att = [0.0; OPS];
        let mut dec = [0.0; OPS];
        let mut sus = [0.0; OPS];
        let mut rel = [0.0; OPS];
        for o in 0..OPS {
            ratio[o] = p[op(o + 1, RATIO)] as f64 * 2f64.powf(p[op(o + 1, DETUNE)] as f64 * 0.5 / 1200.0);
            level[o] = p[op(o + 1, LEVEL)] as f64;
            // envelope steps per sample, in units of the 0..1 envelope
            att[o] = 1.0 / (p[op(o + 1, ATTACK)].max(1.0) as f64 * 0.001 * fs);
            dec[o] = 1.0 / (p[op(o + 1, DECAY)].max(1.0) as f64 * 0.001 * fs);
            sus[o] = p[op(o + 1, SUSTAIN)] as f64 / 99.0;
            rel[o] = 1.0 / (p[op(o + 1, RELEASE)].max(1.0) as f64 * 0.001 * fs);
        }
        let pedal = self.pedal;
        let mut peak = 0.0f64;
        for k in 0..n {
            let mut sum = 0.0;
            for vc in self.voices.iter_mut().filter(|v| v.on) {
                let gate = vc.held || pedal;
                let mut out = 0.0;
                let mut alive = false;
                for o in (0..OPS).rev() {
                    let s = &mut vc.ops[o];
                    // the envelope moves in straight lines of level, so in straight lines of dB
                    if !gate && s.stage < 2 {
                        s.stage = 2;
                    }
                    match s.stage {
                        0 => {
                            s.env += att[o];
                            if s.env >= 1.0 {
                                s.env = 1.0;
                                s.stage = 1;
                            }
                        }
                        1 => {
                            if s.env > sus[o] {
                                s.env = (s.env - dec[o]).max(sus[o]);
                            } else {
                                s.env = (s.env + dec[o]).min(sus[o]);
                            }
                        }
                        2 => {
                            s.env -= rel[o];
                            if s.env <= 0.0 {
                                s.env = 0.0;
                                s.stage = 3;
                            }
                        }
                        _ => {}
                    }
                    // the attack rises in amplitude; decay and release fall in straight lines of dB
                    let amp = if s.stage == 0 { level_amp(level[o] - s.vel_cut) * s.env } else { level_amp(level[o] * s.env - s.vel_cut) };
                    let carrier = alg.carriers & (1 << o) != 0;
                    // a carrier keeps the voice going until its envelope has finished (or it's off)
                    if carrier && s.stage < 3 && level[o] >= 1.0 {
                        alive = true;
                    }
                    // phase modulation from this operator's modulators (already computed)
                    let mut m = 0.0;
                    let mods = alg.mods[o];
                    if mods != 0 {
                        for j in o + 1..OPS {
                            if mods & (1 << j) != 0 {
                                m += vc.ops[j].out;
                            }
                        }
                    }
                    if o + 1 == alg.feedback {
                        m += beta * 0.5 * (vc.fb[0] + vc.fb[1]);
                    }
                    let s = &mut vc.ops[o];
                    let y = amp * (TAU * s.ph + m).sin();
                    s.ph = (s.ph + vc.f * bend * ratio[o] / fs).fract();
                    if o + 1 == alg.feedback {
                        vc.fb = [y, vc.fb[0]];
                    }
                    // a full-level modulator swings its carrier's phase by about 4 pi
                    s.out = if carrier { 0.0 } else { y * 2.0 * TAU };
                    if carrier {
                        out += y;
                    }
                }
                sum += out;
                if !alive {
                    vc.on = false;
                }
            }
            let y = self.dc.process(sum * out_gain);
            peak = peak.max(y.abs());
            for c in chans.iter_mut() {
                c[k] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}

// ------------------------------------------------------------------ panel --

macro_rules! patch {
    ($alg:expr, $fb:expr, $vol:expr, [$($o:expr => ($r:expr, $d:expr, $l:expr, $a:expr, $de:expr, $s:expr, $re:expr, $v:expr)),* $(,)?]) => {{
        const V: &[(usize, f32)] = &[
            (ALG, $alg), (FB, $fb), (VOLUME, $vol),
            $( (op($o, RATIO), $r), (op($o, DETUNE), $d), (op($o, LEVEL), $l), (op($o, ATTACK), $a), (op($o, DECAY), $de), (op($o, SUSTAIN), $s), (op($o, RELEASE), $re), (op($o, VELSENS), $v), )*
        ];
        V
    }};
}

fn presets() -> Vec<Preset> {
    vec![
        // algorithm index: 0=1 1=2 2=5 3=7 4=16 5=19 6=22 7=32
        Preset { name: "Tine E.Piano (alg 5)", values: patch!(2.0, 6.0, -6.0, [
            1 => (1.0, 3.0, 99.0, 1.0, 4000.0, 0.0, 400.0, 2.0),
            2 => (14.0, 0.0, 60.0, 1.0, 350.0, 0.0, 300.0, 7.0),
            3 => (1.0, 0.0, 99.0, 1.0, 5000.0, 0.0, 400.0, 2.0),
            4 => (1.0, 0.0, 85.0, 1.0, 2500.0, 0.0, 400.0, 6.0),
            5 => (1.0, -3.0, 95.0, 1.0, 4000.0, 0.0, 400.0, 2.0),
            6 => (1.0, 0.0, 70.0, 1.0, 1500.0, 0.0, 400.0, 6.0),
        ]) },
        Preset { name: "Solid Bass (alg 16)", values: patch!(4.0, 6.0, -6.0, [
            1 => (0.5, 0.0, 99.0, 1.0, 3000.0, 70.0, 120.0, 2.0),
            2 => (0.5, 0.0, 80.0, 1.0, 400.0, 40.0, 120.0, 5.0),
            3 => (1.0, 0.0, 70.0, 1.0, 250.0, 0.0, 120.0, 5.0),
            4 => (3.0, 0.0, 60.0, 1.0, 200.0, 0.0, 120.0, 4.0),
            5 => (0.5, 1.0, 75.0, 1.0, 600.0, 50.0, 120.0, 4.0),
            6 => (1.0, 0.0, 60.0, 1.0, 300.0, 20.0, 120.0, 3.0),
        ]) },
        Preset { name: "Brass (alg 22)", values: patch!(6.0, 7.0, -6.0, [
            1 => (1.0, 0.0, 99.0, 60.0, 800.0, 85.0, 250.0, 2.0),
            2 => (1.0, 0.0, 80.0, 90.0, 900.0, 70.0, 250.0, 4.0),
            3 => (1.0, 2.0, 95.0, 70.0, 800.0, 85.0, 250.0, 2.0),
            4 => (1.0, -2.0, 95.0, 70.0, 800.0, 85.0, 250.0, 2.0),
            5 => (1.0, 1.0, 93.0, 70.0, 800.0, 85.0, 250.0, 2.0),
            6 => (1.0, 0.0, 70.0, 80.0, 900.0, 70.0, 250.0, 4.0),
        ]) },
        Preset { name: "Tubular Bells (alg 5)", values: patch!(2.0, 0.0, -6.0, [
            1 => (1.0, 0.0, 99.0, 1.0, 9000.0, 0.0, 3000.0, 2.0),
            2 => (3.5, 0.0, 75.0, 1.0, 6000.0, 0.0, 3000.0, 4.0),
            3 => (1.0, 2.0, 95.0, 1.0, 8000.0, 0.0, 3000.0, 2.0),
            4 => (3.5, 0.0, 72.0, 1.0, 5000.0, 0.0, 3000.0, 4.0),
            5 => (2.0, -2.0, 85.0, 1.0, 7000.0, 0.0, 3000.0, 2.0),
            6 => (5.19, 0.0, 65.0, 1.0, 3000.0, 0.0, 3000.0, 4.0),
        ]) },
        Preset { name: "Marimba (alg 7)", values: patch!(3.0, 0.0, -6.0, [
            1 => (1.0, 0.0, 99.0, 1.0, 900.0, 0.0, 400.0, 3.0),
            2 => (3.98, 0.0, 60.0, 1.0, 150.0, 0.0, 200.0, 6.0),
            3 => (4.0, 0.0, 70.0, 1.0, 350.0, 0.0, 200.0, 3.0),
            4 => (10.0, 0.0, 55.0, 1.0, 80.0, 0.0, 100.0, 6.0),
            5 => (1.0, 0.0, 50.0, 1.0, 200.0, 0.0, 100.0, 6.0),
            6 => (1.0, 0.0, 0.0, 1.0, 100.0, 0.0, 100.0, 0.0),
        ]) },
        Preset { name: "Drawbar Organ (alg 32)", values: patch!(7.0, 0.0, -6.0, [
            1 => (0.5, 0.0, 95.0, 5.0, 100.0, 99.0, 60.0, 0.0),
            2 => (1.0, 0.0, 95.0, 5.0, 100.0, 99.0, 60.0, 0.0),
            3 => (1.5, 0.0, 90.0, 5.0, 100.0, 99.0, 60.0, 0.0),
            4 => (2.0, 0.0, 88.0, 5.0, 100.0, 99.0, 60.0, 0.0),
            5 => (3.0, 0.0, 80.0, 5.0, 100.0, 99.0, 60.0, 0.0),
            6 => (4.0, 0.0, 78.0, 5.0, 100.0, 99.0, 60.0, 0.0),
        ]) },
        Preset { name: "Glass Pad (alg 1)", values: patch!(0.0, 4.0, -6.0, [
            1 => (1.0, 2.0, 99.0, 900.0, 3000.0, 85.0, 2500.0, 1.0),
            2 => (2.0, 0.0, 70.0, 1200.0, 3000.0, 60.0, 2500.0, 2.0),
            3 => (1.0, -2.0, 97.0, 1000.0, 3000.0, 85.0, 2500.0, 1.0),
            4 => (7.0, 0.0, 55.0, 1500.0, 4000.0, 40.0, 2500.0, 2.0),
            5 => (3.0, 0.0, 60.0, 1200.0, 4000.0, 50.0, 2500.0, 2.0),
            6 => (1.0, 1.0, 50.0, 2000.0, 4000.0, 50.0, 2500.0, 2.0),
        ]) },
        Preset { name: "Feedback Saw Lead (alg 32)", values: patch!(7.0, 7.0, 2.0, [
            1 => (1.0, 0.0, 0.0, 1.0, 100.0, 0.0, 100.0, 0.0),
            2 => (1.0, 0.0, 0.0, 1.0, 100.0, 0.0, 100.0, 0.0),
            3 => (1.0, 0.0, 0.0, 1.0, 100.0, 0.0, 100.0, 0.0),
            4 => (1.0, 0.0, 0.0, 1.0, 100.0, 0.0, 100.0, 0.0),
            5 => (1.0, 0.0, 0.0, 1.0, 100.0, 0.0, 100.0, 0.0),
            6 => (1.0, 0.0, 99.0, 5.0, 2000.0, 90.0, 200.0, 2.0),
        ]) },
    ]
}

const ROUTES: &[&str] = &[
    "1:  6>5>4>3, 2>1   ·  carriers 1, 3  ·  feedback on 6",
    "2:  6>5>4>3, 2>1   ·  carriers 1, 3  ·  feedback on 2",
    "5:  6>5, 4>3, 2>1   ·  carriers 1, 3, 5  ·  feedback on 6",
    "7:  6>5, 5+4>3, 2>1   ·  carriers 1, 3  ·  feedback on 6",
    "16:  6>5, 4>3, 5+3+2>1   ·  carrier 1  ·  feedback on 6",
    "19:  3>2>1, 6>4, 6>5   ·  carriers 1, 4, 5  ·  feedback on 6",
    "22:  2>1, 6>3, 6>4, 6>5   ·  carriers 1, 3, 4, 5  ·  feedback on 6",
    "32:  all six are carriers  ·  feedback on 6",
];

fn panel() -> Panel {
    const LABELS: [&str; 8] = ["RATIO", "DETUNE", "LEVEL", "ATTACK", "DECAY", "SUSTAIN", "RELEASE", "VELOCITY"];
    const TIPS: [&str; 8] = [
        "Frequency ratio to the key: whole numbers give harmonic tones, others bells and metal.",
        "A touch sharp or flat, for beating and chorus between operators.",
        "Output level 0-99 (about 0.75 dB a step). For a modulator this is how bright; for a carrier, how loud.",
        "Envelope attack time.",
        "Envelope decay time, from full level to the sustain level.",
        "Envelope sustain level (0-99).",
        "Envelope release time after the key comes up.",
        "Velocity sensitivity: how much softer playing turns this operator down.",
    ];
    const OPNAMES: [&str; 6] = ["OP 1", "OP 2", "OP 3", "OP 4", "OP 5", "OP 6"];
    let mut items = vec![
        Item::Selector { p: ALG, x: 90.0, y: 150.0, label: "ALGORITHM", positions: &["1", "2", "5", "7", "16", "19", "22", "32"], tip: "How the six operators connect: which modulate which, which you hear, which feeds back." },
        Item::Knob { p: FB, x: 210.0, y: 150.0, r: 24.0, label: "FEEDBACK", tip: "One operator modulating itself: 0 is a pure tone, 7 a buzzing saw on the edge of noise.", active: None },
        Item::Caption { p: ALG, x: 270.0, y: 130.0, w: 230.0, size: 11.0, texts: ROUTES },
        Item::Knob { p: VOLUME, x: 570.0, y: 150.0, r: 24.0, label: "VOLUME", tip: "Output level.", active: None },
        Item::Divider { y: 205.0 },
    ];
    for o in 0..OPS {
        let y = 256.0 + o as f32 * 86.0;
        items.push(Item::Text { x: 40.0, y, text: OPNAMES[o], size: 12.0, align: Align2::CENTER_CENTER });
        for f in 0..8 {
            items.push(Item::Knob { p: op(o + 1, f), x: 100.0 + f as f32 * 66.0, y, r: 15.0, label: LABELS[f], tip: TIPS[f], active: None });
        }
    }
    Panel {
        title: "FM 7",
        subtitle: ("IN THE MANNER OF THE YAMAHA DX7", "6 OPERATORS · 8 ALGORITHMS · FEEDBACK · 16 VOICES"),
        accent: ui::MOSS,
        size: (W, H),
        items,
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "FM 7", class_id: *b"PQKeysFM7SixOpFM", vendor: "PEQUOD Keys",
    instrument, categories: [Instrument, Synth]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        let mut p = defaults();
        p[ALG] = 7.0;
        p[FB] = 0.0;
        for o in 1..=OPS {
            p[op(o, LEVEL)] = if o == 1 { 99.0 } else { 0.0 };
            p[op(o, RATIO)] = 1.0;
            p[op(o, DETUNE)] = 0.0;
            p[op(o, SUSTAIN)] = 99.0;
        }
        p
    }

    include!("../../instrument_tests.rs");

    #[test]
    fn param_count_matches() {
        assert_eq!(defaults().len(), NPARAMS);
        for a in &ALGORITHMS {
            for (o, m) in a.mods.iter().enumerate() {
                assert!(*m as u32 >> (o + 1) << (o + 1) == *m as u32, "alg {}: op {} modulated by a lower op", a.number, o + 1);
            }
        }
    }

    /// A modulator adds harmonics: the E.Piano's tine operator makes it brighter than a sine.
    #[test]
    fn modulation_adds_harmonics() {
        let sine = tune_params();
        let fm = with(sine.clone(), &[(ALG, 2.0), (op(2, LEVEL), 80.0), (op(2, RATIO), 1.0), (op(2, VELSENS), 0.0)]);
        let bright = |p: &[f32]| {
            let (l, _) = render(p, &[(0, Midi::On { note: 57, vel: 1.0 })], 12000);
            let x = &l[2400..];
            let d: Vec<f32> = x.windows(2).map(|w| w[1] - w[0]).collect();
            rms(&d) / rms(x)
        };
        let (a, b) = (bright(&sine), bright(&fm));
        println!("sine {a:.4} fm {b:.4}");
        assert!(b > 2.0 * a);
    }
}
