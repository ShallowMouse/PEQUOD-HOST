//! Big Muff: the Electro-Harmonix Big Muff Pi as David Gilmour used it: a Ram's Head ('73-'77) or a Sovtek Civil War ('94). Input booster, two diode-clipping gain stages, the passive tone stack and the output booster.

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
pub const H: u32 = 432;

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum Version {
    #[name = "Ram's Head ('73-'77)"]
    RamSHead7377,
    #[name = "Civil War (Sovtek '94)"]
    CivilWarSovtek94,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "volume"]
    pub volume: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "sustain"]
    pub sustain: FloatParam,
    #[id = "version"]
    pub version: EnumParam<Version>,
    #[id = "guitar"]
    pub guitar: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            volume: FloatParam::new("Volume", 4.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            tone: FloatParam::new("Tone", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            sustain: FloatParam::new("Sustain", 6.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            version: EnumParam::new("Version", Version::RamSHead7377),
            guitar: db("Guitar Level", 0.0, -12.0, 12.0),
        }
    }
}

pub const VOLUME: usize = 0;
pub const TONE: usize = 1;
pub const SUSTAIN: usize = 2;
pub const VERSION: usize = 3;
pub const GUITAR: usize = 4;
pub const NPARAMS: usize = 5;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.volume.as_ptr(),
            self.tone.as_ptr(),
            self.sustain.as_ptr(),
            self.version.as_ptr(),
            self.guitar.as_ptr(),
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

const OS_N: usize = 4;
// Q1 (input booster): shunt feedback 470k from collector to base, 39k in series at the input
const RF: f64 = 470e3;
const RS: f64 = 39e3;
// collector load and intrinsic emitter resistance at ~0.2 mA bias (re = 26 mV / Ic)
const RC_EFF: f64 = 9e3;
const RE_INTR: f64 = 130.0;


/// Diode-clipping gain stage (Q2/Q3): high-passed input, gain, diodes in the feedback loop limiting to ~0.6 V, Miller roll-off.
#[derive(Clone, Copy, Default)]
struct ClipStage {
    hp: Biquad,
    lp: Biquad,
    gain: f64,
}

impl ClipStage {
    fn new(fs: f64, gain: f64, f_hp: f64, f_lp: f64) -> Self {
        // coupling cap into the stage: first-order high-pass
        let mut hp = Biquad::identity();
        let t = 1.0 / (2.0 * std::f64::consts::PI * f_hp);
        hp.set_analog(fs, [0.0, t, 0.0], [0.0, t, 1.0]);
        // Miller cap across the feedback: a first-order roll-off
        let mut lp = Biquad::identity();
        lp.set_analog(fs, [0.0, 0.0, 1.0], [0.0, 1.0 / (2.0 * std::f64::consts::PI * f_lp), 1.0]);
        Self { hp, lp, gain }
    }
    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let v = self.hp.process(x) * self.gain;
        // silicon diode pair in the feedback: soft limit around 0.6 V, slightly rounded
        let vd = 0.6;
        let y = vd * (v / vd).tanh() + 0.04 * v / (1.0 + v.abs());
        self.lp.process(y)
    }
    fn reset(&mut self) {
        self.hp.reset();
        self.lp.reset();
    }
}

struct Muff {
    fs_os: f64,
    in_hp: Biquad,
    q1_lp: Biquad,
    q1_av: f64,
    q1_k: f64,
    q2: ClipStage,
    q3: ClipStage,
    lp_tone: Biquad,
    hp_tone: Biquad,
    q4_hp: Biquad,
    sustain: f64,
}

impl Muff {
    fn new(fs_os: f64) -> Self {
        let mut in_hp = Biquad::identity();
        in_hp.set_highpass(fs_os, 4.0, 0.5);
        let mut m = Self {
            fs_os,
            in_hp,
            q1_lp: Biquad::identity(),
            q1_av: 9.0,
            q1_k: 0.5,
            // clipping stages: ~26x and ~30x small-signal, 100 nF into ~15k (about 110 Hz), Miller roll-off
            q2: ClipStage::new(fs_os, 26.0, 110.0, 6000.0),
            q3: ClipStage::new(fs_os, 30.0, 110.0, 5000.0),
            lp_tone: Biquad::identity(),
            hp_tone: Biquad::identity(),
            q4_hp: Biquad::identity(),
            sustain: 0.5,
        };
        // tone stack: 39k / 10 nF low-pass (~408 Hz), 4 nF / 22k high-pass (~1.8 kHz)
        m.lp_tone.set_analog(fs_os, [0.0, 0.0, 1.0], [0.0, 39e3 * 10e-9, 1.0]);
        m.hp_tone.set_analog(fs_os, [0.0, 22e3 * 4e-9, 0.0], [0.0, 22e3 * 4e-9, 1.0]);
        m.q4_hp.set_highpass(fs_os, 20.0, 0.5);
        m.set_r22(100.0);
        m
    }
    /// Ram's Head: 39k / 10n low-pass, 4n / 22k high-pass. Civil War: a lower low-pass leg and a lower high-pass corner (more bass, a gentler scoop).
    fn set_version(&mut self, civil: bool) {
        let fs_os = self.fs_os;
        if civil {
            let (a, rl, chp) = (1.5, 39e3, 7e-9);
            self.lp_tone.set_analog(fs_os, [0.0, 0.0, a], [0.0, rl * 10e-9, 1.0]);
            self.hp_tone.set_analog(fs_os, [0.0, 22e3 * chp, 0.0], [0.0, 22e3 * chp, 1.0]);
        } else {
            self.lp_tone.set_analog(fs_os, [0.0, 0.0, 1.0], [0.0, 39e3 * 10e-9, 1.0]);
            self.hp_tone.set_analog(fs_os, [0.0, 22e3 * 4e-9, 0.0], [0.0, 22e3 * 4e-9, 1.0]);
        }
    }
    fn reset(&mut self) {
        for b in [&mut self.in_hp, &mut self.q1_lp, &mut self.lp_tone, &mut self.hp_tone, &mut self.q4_hp] {
            b.reset();
        }
        self.q2.reset();
        self.q3.reset();
    }
    /// R22 (Q1 emitter resistor) -> open-loop gain -> closed-loop gain of the shunt-feedback booster, and its bandwidth.
    fn set_r22(&mut self, r22: f64) {
        let a = RC_EFF / (RE_INTR + r22.max(0.0));
        let ratio = RF / RS;
        self.q1_av = ratio * a / (a + 1.0 + ratio);
        // un-degenerated, the transistor's exponential curve shows through more
        self.q1_k = RE_INTR / (RE_INTR + r22.max(0.0));
        // feedback cap 470 pF across RF: the pole widens as loop gain falls
        let fc = (1.0 / (2.0 * std::f64::consts::PI * RF * 470e-12)) * (1.0 + ratio / (a + 1.0));
        // first order: the gain falls back toward the feedback limit rather than to nothing
        let fc = fc.min(self.fs_os * 0.2);
        let shelf = 1.0 / self.q1_av.max(1.0);
        self.q1_lp.set_analog(self.fs_os, [0.0, shelf / (2.0 * std::f64::consts::PI * fc * shelf.max(1e-3)), 1.0], [0.0, 1.0 / (2.0 * std::f64::consts::PI * fc), 1.0]);
    }
    #[inline]
    fn process(&mut self, x: f64, tone: f64) -> f64 {
        // input booster Q1: collector idles near 7 V on a 9 V supply, so it runs out of room upward first
        let v = self.in_hp.process(x) * self.q1_av;
        let v = v + self.q1_k * 0.08 * v * v.abs().min(4.0);
        let v = if v > 0.0 { 1.9 * (v / 1.9).tanh() } else { -6.0 * (-v / 6.0).tanh() };
        let v = self.q1_lp.process(v);
        // sustain pot (with R23 keeping a little signal at zero)
        let v = v * self.sustain;
        let v = self.q2.process(v);
        let v = self.q3.process(v);
        // passive tone stack: low-pass and high-pass legs mixed by the tone pot (mid scoop near 1 kHz in the middle)
        let lo = self.lp_tone.process(v);
        let hi = self.hp_tone.process(v);
        let t = tone;
        let v = lo * (1.0 - t) + hi * t;
        // output booster Q4 (~4.5x) with a little headroom limit
        let v = self.q4_hp.process(v) * 4.5;
        3.5 * (v / 3.5).tanh()
    }
}

pub struct Dsp {
    fs: f64,
    muff: [Muff; 2],
    os: [Oversampler; 2],
    out_lp: [Biquad; 2],
    v: [f32; NPARAMS],
    version: i32,
    input: Smooth,
    sus: Smooth,
    tone: Smooth,
    vol: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let fs_os = fs * OS_N as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut out_lp = [Biquad::identity(); 2];
        for b in &mut out_lp {
            b.set_lowpass(fs, 12000.0f64.min(fs * 0.45), 0.707);
        }
        Self {
            fs,
            muff: [Muff::new(fs_os), Muff::new(fs_os)],
            os: [Oversampler::new(), Oversampler::new()],
            out_lp,
            v,
            version: -1,
            input: Smooth::new(fs, 20.0, 1.0),
            sus: Smooth::new(fs, 20.0, 0.5),
            tone: Smooth::new(fs, 20.0, 0.5),
            vol: Smooth::new(fs, 20.0, 0.5),
        }
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.muff[c].reset();
            self.os[c].reset();
            self.out_lp[c].reset();
        }
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let ver = v[VERSION].round() as i32;
        if ver != self.version {
            for m in &mut self.muff {
                m.set_version(ver == 1);
            }
            self.version = ver;
        }
        self.input.set(db_to_gain(v[GUITAR] as f64));
        let s = v[SUSTAIN] as f64 / 10.0;
        self.sus.set((s * 100e3 + 1e3) / 101e3);
        self.tone.set(v[TONE] as f64 / 10.0);
        let vol = v[VOLUME] as f64 / 10.0;
        self.vol.set(vol * vol);
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(2);
        let mut peak = 0.0f64;
        for k in 0..n {
            let gi = self.input.next();
            let sus = self.sus.next();
            let tone = self.tone.next();
            let vol = self.vol.next();
            for c in 0..nch {
                let x = chans[c][k] as f64 * gi;
                let m = &mut self.muff[c];
                m.sustain = sus;
                let y = self.os[c].process(x, |s| m.process(s, tone));
                peak = peak.max(y.abs());
                let y = self.out_lp[c].process(y) * vol * 0.6;
                chans[c][k] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Ram's Head Classic", values: &[] },  // OCR "Ram's Head Classic"
        Preset { name: "Comfortably Numb Solo", values: &[(VOLUME, 4.5), (TONE, 5.5), (SUSTAIN, 6.5)] },
        Preset { name: "Dogs (Animals)", values: &[(TONE, 6.0), (SUSTAIN, 7.0)] },
        Preset { name: "Mild Muff into a Hot Hiwatt", values: &[(VOLUME, 5.0), (TONE, 5.5), (SUSTAIN, 4.0)] },
        Preset { name: "Sorrow (Civil War)", values: &[(SUSTAIN, 7.0), (VERSION, 1.0)] },
        Preset { name: "Division Bell Lead", values: &[(VOLUME, 4.5), (TONE, 6.0), (VERSION, 1.0)] },
        Preset { name: "Dark Wall of Fuzz", values: &[(VOLUME, 3.5), (TONE, 3.0), (SUSTAIN, 10.0)] },  // OCR 'Dark Wall of Fuzz'
    ]
}

/*
('knob', 149.5, 159.5, 39.0)
('knob', 319.5, 159.5, 39.0)
('knob', 489.5, 159.5, 39.0)
('divider', 284)
('knob', 419.5, 333.5, 22.0)

crate: ['floyd/pf_bigmuff', 'library/panic_abort']
key-value support is experimental and must be enabled using the `kv` feature/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/log-0.4.34/src/__private_api.rs
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
AFTER THE ELECTRO-HARMONIX BIG MUFF PI4 TRANSISTORS  
  2 DIODE CLIPPING STAGESVOLUMEOutput level (100k pot). A Big Muff has plenty of level to spare.TONEBlends the low-pass (left: dark, thick) and high-pass (right: thin, cutting) halves of the tone network. At noon the mids are scooped.SUSTAINLevel into the two clipping stages: how much fuzz and sustain.CLIPPINGVERSIONRAM'S HEADCIVIL WARRam's Head (1970s, Gilmour's Animals / Wall Muffs) or the Sovtek Civil War (his 1994 Division Bell / Pulse Muff): more bass, a gentler scoop.GUITAR LEVELHow hot your guitar hits the pedal (0 dB = full-scale DI as 1 V from the pickups).BIG MUFFRam's Head Classic
Comfortably Numb Solo
Dogs (Animals)
Mild Muff into a Hot Hiwatt
Sorrow (Civil War)
Division Bell Lead
Dark Wall of FuzziBFPMgfufPQD0001Big MuffPEQUOD Pink Floyd1.0.1vintage/src/amp/net.rs
@333333
@floyd/pf_bigmuff/src/lib.rs

*/
fn panel() -> Panel {
    Panel {
        title: "BIG MUFF",
        subtitle: ("AFTER THE ELECTRO-HARMONIX BIG MUFF PI", "4 TRANSISTORS  ·  2 DIODE CLIPPING STAGES"),
        accent: ui::TANGERINE,
        size: (W, H),
        items: vec![
            Item::Knob { p: VOLUME, x: 150.0, y: 160.0, r: 40.0, label: "VOLUME", tip: "Output level (100k pot). A Big Muff has plenty of level to spare.", active: None },
            Item::Knob { p: TONE, x: 320.0, y: 160.0, r: 40.0, label: "TONE", tip: "Blends the low-pass (left: dark, thick) and high-pass (right: thin, cutting) halves of the tone network. At noon the mids are scooped.", active: None },
            Item::Knob { p: SUSTAIN, x: 490.0, y: 160.0, r: 40.0, label: "SUSTAIN", tip: "Level into the two clipping stages: how much fuzz and sustain.", active: None },
            Item::Lamp { m: 0, x: 320.0, y: 248.0, label: "CLIPPING", lo: 0.3, hi: 1.0 },
            Item::Divider { y: 284.0 },
            Item::Text { x: 230.0, y: 302.0, text: "VERSION", size: 13.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: VERSION, x: 230.0, y: 338.0, labels: ["RAM'S HEAD", "CIVIL WAR"], tip: "Ram's Head (1970s, Gilmour's Animals / Wall Muffs) or the Sovtek Civil War (his 1994 Division Bell / Pulse Muff): more bass, a gentler scoop." },
            Item::Knob { p: GUITAR, x: 420.0, y: 334.0, r: 22.0, label: "GUITAR LEVEL", tip: "How hot your guitar hits the pedal (0 dB = full-scale DI as 1 V from the pickups).", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Big Muff", class_id: *b"PFBigMuffPQD0001", vendor: "PEQUOD Pink Floyd",
    categories: [Fx, Distortion]
);
