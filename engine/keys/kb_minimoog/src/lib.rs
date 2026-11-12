//! Model 72: a monophonic synthesizer in the manner of the Minimoog Model D.
//!
//! Laid out as the Model D is: the controllers (glide, the modulation mix of oscillator 3 and
//! noise, and where the mod wheel sends it), the oscillator bank (three oscillators with the
//! Model D's ranges from LO to 2' and its six waveforms; oscillator 3 can come off the keyboard
//! to run as a slow modulator), the mixer (which overdrives the filter when it's turned up, as
//! players did on purpose), the 24 dB/octave transistor ladder filter with its emphasis and
//! keyboard control, and the two ADS contour generators with the DECAY switch that makes the
//! release follow the decay time. Low-note priority and single triggering, as on the Model D:
//! play legato and the contours don't restart.

#![allow(clippy::excessive_precision)]
use nih_plug::prelude::*;
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
pub const H: u32 = 690;

fn stepped(name: &str, default: f32, max: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max }).with_step_size(1.0)
}
fn level(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1)
}
fn semis(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: -7.0, max: 7.0 }).with_step_size(0.01).with_unit(" st")
}
fn contour_time(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Skewed { min: 1.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ms")
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "glide"]
    pub glide: FloatParam,
    #[id = "modmix"]
    pub modmix: FloatParam,
    #[id = "oscmod"]
    pub oscmod: FloatParam,
    #[id = "filtmod"]
    pub filtmod: FloatParam,
    #[id = "o3ctrl"]
    pub o3ctrl: FloatParam,
    #[id = "r1"]
    pub r1: FloatParam,
    #[id = "w1"]
    pub w1: FloatParam,
    #[id = "r2"]
    pub r2: FloatParam,
    #[id = "w2"]
    pub w2: FloatParam,
    #[id = "d2"]
    pub d2: FloatParam,
    #[id = "r3"]
    pub r3: FloatParam,
    #[id = "w3"]
    pub w3: FloatParam,
    #[id = "d3"]
    pub d3: FloatParam,
    #[id = "l1"]
    pub l1: FloatParam,
    #[id = "l2"]
    pub l2: FloatParam,
    #[id = "l3"]
    pub l3: FloatParam,
    #[id = "ln"]
    pub ln: FloatParam,
    #[id = "cutoff"]
    pub cutoff: FloatParam,
    #[id = "emph"]
    pub emph: FloatParam,
    #[id = "contour"]
    pub contour: FloatParam,
    #[id = "kbd"]
    pub kbd: FloatParam,
    #[id = "fa"]
    pub fa: FloatParam,
    #[id = "fd"]
    pub fd: FloatParam,
    #[id = "fs"]
    pub fs: FloatParam,
    #[id = "la"]
    pub la: FloatParam,
    #[id = "ld"]
    pub ld: FloatParam,
    #[id = "ls"]
    pub ls: FloatParam,
    #[id = "decaysw"]
    pub decaysw: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            glide: pct("Glide", 0.0),
            modmix: pct("Modulation Mix", 0.0),
            oscmod: stepped("Oscillator Modulation", 0.0, 1.0),
            filtmod: stepped("Filter Modulation", 0.0, 1.0),
            o3ctrl: stepped("Osc 3 Control", 1.0, 1.0),
            r1: stepped("Osc 1 Range", 3.0, 5.0),
            w1: stepped("Osc 1 Waveform", 2.0, 5.0),
            r2: stepped("Osc 2 Range", 3.0, 5.0),
            w2: stepped("Osc 2 Waveform", 2.0, 5.0),
            d2: semis("Osc 2 Frequency", 0.07),
            r3: stepped("Osc 3 Range", 2.0, 5.0),
            w3: stepped("Osc 3 Waveform", 3.0, 5.0),
            d3: semis("Osc 3 Frequency", 0.0),
            l1: level("Osc 1 Volume", 8.0),
            l2: level("Osc 2 Volume", 6.0),
            l3: level("Osc 3 Volume", 0.0),
            ln: level("Noise Volume", 0.0),
            cutoff: hz("Cutoff", 900.0, 30.0, 18000.0),
            emph: level("Emphasis", 2.0),
            contour: level("Amount of Contour", 5.0),
            kbd: stepped("Keyboard Control", 2.0, 3.0),
            fa: contour_time("Filter Attack", 5.0),
            fd: contour_time("Filter Decay", 400.0),
            fs: level("Filter Sustain", 3.0),
            la: contour_time("Loudness Attack", 3.0),
            ld: contour_time("Loudness Decay", 400.0),
            ls: level("Loudness Sustain", 9.0),
            decaysw: stepped("Decay", 1.0, 1.0),
            volume: db("Volume", -6.0, -30.0, 6.0),
        }
    }
}

pub const GLIDE: usize = 0;
pub const MODMIX: usize = 1;
pub const OSCMOD: usize = 2;
pub const FILTMOD: usize = 3;
pub const O3CTRL: usize = 4;
pub const R1: usize = 5;
pub const W1: usize = 6;
pub const R2: usize = 7;
pub const W2: usize = 8;
pub const D2: usize = 9;
pub const R3: usize = 10;
pub const W3: usize = 11;
pub const D3: usize = 12;
pub const L1: usize = 13;
pub const L2: usize = 14;
pub const L3: usize = 15;
pub const LN: usize = 16;
pub const CUTOFF: usize = 17;
pub const EMPH: usize = 18;
pub const CONTOUR: usize = 19;
pub const KBD: usize = 20;
pub const FA: usize = 21;
pub const FD: usize = 22;
pub const FS: usize = 23;
pub const LA: usize = 24;
pub const LD: usize = 25;
pub const LS: usize = 26;
pub const DECAYSW: usize = 27;
pub const VOLUME: usize = 28;
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
            self.glide.as_ptr(),
            self.modmix.as_ptr(),
            self.oscmod.as_ptr(),
            self.filtmod.as_ptr(),
            self.o3ctrl.as_ptr(),
            self.r1.as_ptr(),
            self.w1.as_ptr(),
            self.r2.as_ptr(),
            self.w2.as_ptr(),
            self.d2.as_ptr(),
            self.r3.as_ptr(),
            self.w3.as_ptr(),
            self.d3.as_ptr(),
            self.l1.as_ptr(),
            self.l2.as_ptr(),
            self.l3.as_ptr(),
            self.ln.as_ptr(),
            self.cutoff.as_ptr(),
            self.emph.as_ptr(),
            self.contour.as_ptr(),
            self.kbd.as_ptr(),
            self.fa.as_ptr(),
            self.fd.as_ptr(),
            self.fs.as_ptr(),
            self.la.as_ptr(),
            self.ld.as_ptr(),
            self.ls.as_ptr(),
            self.decaysw.as_ptr(),
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

/// Octaves from the 8' for each range position: LO, 32', 16', 8', 4', 2'.
const RANGE: [f64; 6] = [-7.0, -2.0, -1.0, 0.0, 1.0, 2.0];

/// One Model D waveform. Oscillators 1 and 2: triangle, "sharktooth" (triangle-saw), saw,
/// square, wide and narrow pulse; oscillator 3 has a reverse saw in place of the sharktooth.
#[inline]
fn wave(shape: usize, osc3: bool, ph: f64, dt: f64) -> f64 {
    let tri = if ph < 0.5 { 4.0 * ph - 1.0 } else { 3.0 - 4.0 * ph };
    let saw = 2.0 * ph - 1.0 - blep(ph, dt);
    let pulse = |w: f64| {
        let mut p = if ph < w { 1.0 } else { -1.0 };
        p += blep(ph, dt);
        let q = (ph - w + 1.0).fract();
        p -= blep(q, dt);
        p - (2.0 * w - 1.0)
    };
    match shape {
        0 => tri,
        1 if osc3 => -saw,
        1 => 0.5 * (tri + saw),
        2 => saw,
        3 => pulse(0.5),
        4 => pulse(0.3),
        _ => pulse(0.12),
    }
}

/// An ADS contour generator. `gate` held: attack then decay to sustain; let go: release at
/// the decay time (DECAY switch on) or quickly (off).
#[derive(Clone, Copy, Default)]
struct Contour {
    v: f64,
    attacking: bool,
}

impl Contour {
    #[inline]
    fn step(&mut self, gate: bool, ca: f64, cd: f64, s: f64, cr: f64) -> f64 {
        if gate {
            if self.attacking {
                self.v += (1.3 - self.v) * ca;
                if self.v >= 1.0 {
                    self.v = 1.0;
                    self.attacking = false;
                }
            } else {
                self.v += (s - self.v) * cd;
            }
        } else {
            self.attacking = false;
            self.v += (0.0 - self.v) * cr;
        }
        self.v
    }
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    held: Vec<u8>,
    gate: bool,
    pitch: f64,
    target: f64,
    ph: [f64; 3],
    lad: [f64; 4],
    fenv: Contour,
    aenv: Contour,
    noise: Noise,
    bend: f64,
    modw: f64,
    dc: DcBlock,
}

impl Dsp {
    fn retarget(&mut self) {
        // low-note priority
        if let Some(n) = self.held.iter().min() {
            self.target = *n as f64;
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, held: Vec::with_capacity(16), gate: false, pitch: 60.0, target: 60.0, ph: [0.0, 0.33, 0.71], lad: [0.0; 4], fenv: Contour::default(), aenv: Contour::default(), noise: Noise::new(72), bend: 0.0, modw: 0.0, dc: DcBlock::new(fs) }
    }
    fn reset(&mut self) {
        self.held.clear();
        self.gate = false;
        self.lad = [0.0; 4];
        self.fenv = Contour::default();
        self.aenv = Contour::default();
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
                if first {
                    // single trigger: the contours only start again from all keys up
                    self.gate = true;
                    self.fenv.attacking = true;
                    self.aenv.attacking = true;
                    if self.v[GLIDE] <= 0.0 {
                        self.pitch = self.target;
                    }
                }
            }
            Midi::Off { note } => {
                self.held.retain(|n| *n != note);
                if self.held.is_empty() {
                    self.gate = false;
                } else {
                    self.retarget();
                }
            }
            Midi::Bend { value } => self.bend = value as f64 * 2.0,
            Midi::Cc { cc: 1, value } => self.modw = value as f64,
            Midi::AllOff => {
                self.held.clear();
                self.gate = false;
            }
            _ => {}
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let p = self.v;
        let st = |i: usize, max: f32| p[i].round().clamp(0.0, max) as usize;
        let (r1, r2, r3) = (RANGE[st(R1, 5.0)], RANGE[st(R2, 5.0)], RANGE[st(R3, 5.0)]);
        let (w1, w2, w3) = (st(W1, 5.0), st(W2, 5.0), st(W3, 5.0));
        let (l1, l2, l3, ln) = (p[L1] as f64 / 10.0, p[L2] as f64 / 10.0, p[L3] as f64 / 10.0, p[LN] as f64 / 10.0);
        let o3_kbd = p[O3CTRL] >= 0.5;
        let (oscmod, filtmod) = (p[OSCMOD] >= 0.5, p[FILTMOD] >= 0.5);
        let modmix = p[MODMIX] as f64;
        // glide: 0 off, up to about 5 s across an octave
        let glide = p[GLIDE] as f64;
        let gc = if glide > 0.0 { coeff(fs, 5.0 + 3000.0 * glide * glide) } else { 0.0 };
        let cut = p[CUTOFF] as f64;
        let k = 4.2 * p[EMPH] as f64 / 10.0;
        let amount = p[CONTOUR] as f64 / 10.0 * 5.0; // octaves at full contour
        let track = [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0][st(KBD, 3.0)];
        let ms = |i: usize| p[i].max(1.0) as f64;
        let (fa, fd, fsus) = (coeff(fs, ms(FA) / 2.2), coeff(fs, ms(FD) / 2.2), p[FS] as f64 / 10.0);
        let (la, ld, lsus) = (coeff(fs, ms(LA) / 2.2), coeff(fs, ms(LD) / 2.2), p[LS] as f64 / 10.0);
        let decay_sw = p[DECAYSW] >= 0.5;
        let (fr, lr) = if decay_sw { (fd, ld) } else { (coeff(fs, 8.0), coeff(fs, 8.0)) };
        let vol = db_to_gain(p[VOLUME] as f64);
        let ref_f = |note: f64| 440.0 * 2f64.powf((note - 69.0) / 12.0);
        let mut peak = 0.0f64;
        for kk in 0..n {
            if gc > 0.0 {
                self.pitch = self.target + gc * (self.pitch - self.target);
            } else {
                self.pitch = self.target;
            }
            let note = self.pitch + self.bend;
            // oscillator 3 first: it's also the modulation source
            let o3_note = if o3_kbd { note } else { 48.0 };
            let f3 = ref_f(o3_note + p[D3] as f64) * 2f64.powf(r3);
            let dt3 = (f3 / fs).min(0.45);
            let o3 = wave(w3, true, self.ph[2], dt3);
            self.ph[2] = (self.ph[2] + dt3).fract();
            let nz = self.noise.next();
            let modsrc = ((1.0 - modmix) * o3 + modmix * nz) * self.modw;
            let pm = if oscmod { modsrc * 5.0 } else { 0.0 };
            let f1 = ref_f(note + pm) * 2f64.powf(r1);
            let f2 = ref_f(note + p[D2] as f64 + pm) * 2f64.powf(r2);
            let (dt1, dt2) = ((f1 / fs).min(0.45), (f2 / fs).min(0.45));
            let o1 = wave(w1, false, self.ph[0], dt1);
            let o2 = wave(w2, false, self.ph[1], dt2);
            self.ph[0] = (self.ph[0] + dt1).fract();
            self.ph[1] = (self.ph[1] + dt2).fract();
            // the mixer: turned past about 4 it pushes the filter's input into overload
            let mix = o1 * l1 + o2 * l2 + o3 * l3 + nz * ln * 0.8;
            let x = (mix * 0.9).tanh();
            let fe = self.fenv.step(self.gate, fa, fd, fsus, fr);
            let ae = self.aenv.step(self.gate, la, ld, lsus, lr);
            let fm = if filtmod { modsrc * 2.0 } else { 0.0 };
            let fc = (cut * 2f64.powf(amount * fe + track * (note - 60.0) / 12.0 + fm)).clamp(10.0, 0.42 * fs);
            // the transistor ladder, twice per sample for stability at high cutoffs
            let g = 1.0 - (-std::f64::consts::TAU * fc / (2.0 * fs)).exp();
            for _ in 0..2 {
                let u = (x - k * self.lad[3]).tanh();
                self.lad[0] += g * (u - self.lad[0].tanh());
                self.lad[1] += g * (self.lad[0].tanh() - self.lad[1].tanh());
                self.lad[2] += g * (self.lad[1].tanh() - self.lad[2].tanh());
                self.lad[3] += g * (self.lad[2].tanh() - self.lad[3].tanh());
            }
            let y = self.dc.process(self.lad[3] * (1.0 + 0.35 * k)) * ae * vol * 1.2;
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
        Preset { name: "Fat Bass", values: &[(R1, 2.0), (R2, 2.0), (R3, 1.0), (W1, 2.0), (W2, 2.0), (W3, 3.0), (L1, 10.0), (L2, 8.0), (L3, 6.0), (D2, 0.06), (D3, -0.08), (CUTOFF, 300.0), (EMPH, 3.0), (CONTOUR, 6.0), (FA, 1.0), (FD, 250.0), (FS, 1.0), (LS, 10.0), (DECAYSW, 0.0)] },
        Preset { name: "Lead with Glide", values: &[(W1, 2.0), (W2, 3.0), (L1, 9.0), (L2, 7.0), (D2, 0.1), (CUTOFF, 1400.0), (EMPH, 4.0), (CONTOUR, 4.0), (GLIDE, 0.25), (FD, 600.0), (FS, 5.0)] },
        Preset { name: "Funk Pluck Bass", values: &[(R1, 2.0), (R2, 1.0), (W1, 5.0), (W2, 2.0), (L1, 9.0), (L2, 6.0), (CUTOFF, 200.0), (EMPH, 5.5), (CONTOUR, 7.0), (FA, 1.0), (FD, 180.0), (FS, 0.0), (LD, 300.0), (LS, 4.0), (DECAYSW, 0.0)] },
        Preset { name: "Wheel Vibrato Lead", values: &[(O3CTRL, 0.0), (R3, 0.0), (W3, 0.0), (D3, 2.0), (L3, 0.0), (OSCMOD, 1.0), (W1, 2.0), (W2, 2.0), (CUTOFF, 2000.0), (EMPH, 3.0)] },
        Preset { name: "Screaming Filter", values: &[(W1, 2.0), (W2, 2.0), (L1, 10.0), (L2, 10.0), (CUTOFF, 700.0), (EMPH, 8.5), (CONTOUR, 6.0), (FD, 900.0), (FS, 4.0)] },
        Preset { name: "Three-Osc Brass", values: &[(W1, 2.0), (W2, 2.0), (W3, 2.0), (R3, 2.0), (L1, 8.0), (L2, 8.0), (L3, 8.0), (D2, 0.08), (D3, -0.1), (CUTOFF, 500.0), (EMPH, 1.5), (CONTOUR, 7.0), (FA, 60.0), (FD, 500.0), (FS, 6.0), (LA, 20.0)] },
        Preset { name: "Wind and Noise", values: &[(L1, 0.0), (L2, 0.0), (LN, 10.0), (CUTOFF, 800.0), (EMPH, 9.0), (CONTOUR, 3.0), (FA, 800.0), (LA, 800.0), (LD, 2000.0)] },
    ]
}

const RANGES: &[&str] = &["LO", "32'", "16'", "8'", "4'", "2'"];
const WAVES12: &[&str] = &["TRI", "SHARK", "SAW", "SQUARE", "WIDE", "NARROW"];
const WAVES3: &[&str] = &["TRI", "REV SAW", "SAW", "SQUARE", "WIDE", "NARROW"];

fn panel() -> Panel {
    let k = |p, x, y, r, label, tip| Item::Knob { p, x, y, r, label, tip, active: None };
    Panel {
        title: "MODEL 72",
        subtitle: ("IN THE MANNER OF THE MINIMOOG MODEL D", "3 OSCILLATORS · LADDER FILTER · 2 CONTOURS · MONO"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            // controllers
            k(GLIDE, 55.0, 128.0, 20.0, "GLIDE", "Portamento: how long the pitch takes to slide to a new note."),
            k(MODMIX, 135.0, 128.0, 20.0, "MOD MIX", "The modulation source: oscillator 3 (left) to noise (right). The mod wheel sets how much."),
            Item::Switch { p: OSCMOD, x: 300.0, y: 100.0, labels: ["", "OSC MOD"], tip: "The mod wheel modulates the oscillators' pitch (vibrato and trills)." },
            Item::Switch { p: FILTMOD, x: 300.0, y: 128.0, labels: ["", "FILTER MOD"], tip: "The mod wheel modulates the filter cutoff (wah and growl)." },
            Item::Switch { p: O3CTRL, x: 300.0, y: 156.0, labels: ["", "OSC 3 KBD"], tip: "Oscillator 3 follows the keyboard. Off, it runs at its own fixed rate: in LO range it's a slow modulator." },
            k(VOLUME, 570.0, 128.0, 24.0, "VOLUME", "Output level."),
            Item::Divider { y: 186.0 },
            // oscillator bank and mixer
            Item::Selector { p: R1, x: 60.0, y: 242.0, label: "RANGE 1", positions: RANGES, tip: "Oscillator 1 range: 8' is the key's pitch; LO is sub-audio." },
            Item::Selector { p: W1, x: 180.0, y: 242.0, label: "WAVE 1", positions: WAVES12, tip: "Oscillator 1 waveform." },
            k(L1, 390.0, 242.0, 20.0, "OSC 1", "Oscillator 1 in the mixer."),
            Item::Selector { p: R2, x: 60.0, y: 337.0, label: "RANGE 2", positions: RANGES, tip: "Oscillator 2 range." },
            Item::Selector { p: W2, x: 180.0, y: 337.0, label: "WAVE 2", positions: WAVES12, tip: "Oscillator 2 waveform." },
            k(D2, 290.0, 337.0, 20.0, "FREQ 2", "Oscillator 2 tuning against oscillator 1: a touch for the classic fat beating, or up to a fifth either way."),
            k(L2, 390.0, 337.0, 20.0, "OSC 2", "Oscillator 2 in the mixer."),
            k(LN, 490.0, 337.0, 20.0, "NOISE", "White noise in the mixer."),
            Item::Selector { p: R3, x: 60.0, y: 432.0, label: "RANGE 3", positions: RANGES, tip: "Oscillator 3 range." },
            Item::Selector { p: W3, x: 180.0, y: 432.0, label: "WAVE 3", positions: WAVES3, tip: "Oscillator 3 waveform (it has a reverse saw for modulation)." },
            k(D3, 290.0, 432.0, 20.0, "FREQ 3", "Oscillator 3 tuning (or, off the keyboard, its rate)."),
            k(L3, 390.0, 432.0, 20.0, "OSC 3", "Oscillator 3 in the mixer."),
            Item::Divider { y: 482.0 },
            // filter
            k(CUTOFF, 55.0, 540.0, 24.0, "CUTOFF", "Filter cutoff frequency."),
            k(EMPH, 135.0, 540.0, 20.0, "EMPHASIS", "Resonance. Up past about 9 the filter whistles on its own."),
            k(CONTOUR, 215.0, 540.0, 20.0, "CONTOUR", "How far the filter contour opens the cutoff."),
            Item::Selector { p: KBD, x: 320.0, y: 540.0, label: "KEYBOARD", positions: &["OFF", "1", "2", "1+2"], tip: "Keyboard control of the filter: how much the cutoff follows the notes (1 = a third, 2 = two thirds, both = all)." },
            k(FA, 430.0, 540.0, 18.0, "ATTACK", "Filter contour attack."),
            k(FD, 500.0, 540.0, 18.0, "DECAY", "Filter contour decay."),
            k(FS, 570.0, 540.0, 18.0, "SUSTAIN", "Filter contour sustain."),
            // loudness
            Item::Text { x: 130.0, y: 636.0, text: "FILTER ABOVE · LOUDNESS BELOW", size: 10.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: DECAYSW, x: 320.0, y: 636.0, labels: ["", "DECAY"], tip: "On: when you let go, both contours fall at their decay times. Off: notes stop short." },
            k(LA, 430.0, 636.0, 18.0, "ATTACK", "Loudness contour attack."),
            k(LD, 500.0, 636.0, 18.0, "DECAY", "Loudness contour decay."),
            k(LS, 570.0, 636.0, 18.0, "SUSTAIN", "Loudness contour sustain."),
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Model 72", class_id: *b"PQKeysModel72Mog", vendor: "PEQUOD Keys",
    instrument, categories: [Instrument, Synth]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        with(defaults(), &[(L2, 0.0), (L3, 0.0), (LN, 0.0), (CUTOFF, 18000.0), (EMPH, 0.0), (CONTOUR, 0.0), (LS, 10.0), (KBD, 0.0)])
    }

    include!("../../instrument_tests.rs");

    /// Low-note priority, and legato doesn't retrigger the contours.
    #[test]
    fn low_note_priority_single_trigger() {
        let p = tune_params();
        let ev = [(0, Midi::On { note: 57, vel: 1.0 }), (24000, Midi::On { note: 69, vel: 1.0 }), (48000, Midi::Off { note: 57 })];
        let (l, _) = render(&p, &ev, 96000);
        let f_low = pitch(&l[30000..]);
        let f_high = pitch(&l[60000..]);
        println!("both held {f_low:.1} Hz, after letting the low one go {f_high:.1} Hz");
        assert!((f_low - 220.0).abs() < 3.0 && (f_high - 440.0).abs() < 5.0);
    }
}
