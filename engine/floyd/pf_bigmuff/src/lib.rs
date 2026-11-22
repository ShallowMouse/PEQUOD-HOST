//! Big Muff: the Electro-Harmonix Big Muff Pi as David Gilmour used it: a Ram's Head (violet, 1973-77) or a Sovtek Civil
//! War (1994). The whole circuit, solved from its part values: the input booster, the two 1N914-clipping stages with their
//! 470k / 560p feedback, the Sustain pot between them, the passive tone stack and the output booster. Values after Kit
//! Rae's measurements of the violet Ram's Head; the Civil War's tone stack after the Sovtek green.

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

use vintage::amp::net::Taper;
use vintage::circuit::{self, Circuit, Solver};

const OS: usize = 4;
const C_SUS: usize = 0;
const C_TONE: usize = 1;
const C_VOL: usize = 2;

/// The whole pedal. `civil`: the Sovtek Civil War's tone stack.
fn big_muff(civil: bool, fs: f64) -> Solver {
    let q = circuit::BC108;
    let d = circuit::D1N914;
    // tone stack: low-pass leg (R, C) and high-pass leg (C, R)
    let (rl, cl, ch, rh) = if civil { (20e3, 10e-9, 10e-9, 22e3) } else { (39e3, 10e-9, 4e-9, 22e3) };
    let mut c = Circuit::new()
        .input(1, 10e3)
        .dc(2, 9.0, 5.0)
        // Q1: input booster
        .c(1, 3, 1e-6)
        .r(3, 4, 39e3)
        .r(5, 4, 470e3)
        .c(5, 4, 560e-12)
        .r(4, 0, 100e3)
        .r(2, 5, 15e3)
        .r(6, 0, 100.0)
        .npn(5, 4, 6, q)
        // sustain: Q1's output across the 100k pot, its wiper through 1k into the first clipper
        .c(5, 7, 1e-6)
        .pot(0, 8, 7, 100e3, Taper::Linear, C_SUS)
        .r(8, 9, 1e3)
        .c(9, 10, 1e-6)
        .r(10, 11, 10e3);
    // the two clipping stages: 470k + 560p feedback, 1N914 pair through 1u, 15k collector, 150R emitter
    for (b, col, e, dm, nxt_c, nxt_r) in [(11usize, 12usize, 13usize, 14usize, 15usize, 16usize), (16, 17, 18, 19, 20, 21)] {
        let _ = nxt_r;
        c = c.r(col, b, 470e3).c(col, b, 560e-12).r(b, 0, 100e3).r(2, col, 15e3).r(e, 0, 150.0).npn(col, b, e, q)
            .c(col, dm, 1e-6).diode(dm, b, d).diode(b, dm, d);
        if col == 12 {
            // coupling into the second clipper
            c = c.c(col, nxt_c, 1e-6).r(nxt_c, 16, 10e3);
        }
    }
    // tone stack from Q3's collector (17)
    c = c
        .c(17, 22, 1e-6)
        .r(22, 23, rl)
        .c(23, 0, cl)
        .c(22, 24, ch)
        .r(24, 0, rh)
        .pot(23, 25, 24, 100e3, Taper::Linear, C_TONE)
        // Q4: output booster
        .c(25, 26, 1e-6)
        .r(2, 26, 470e3)
        .r(26, 0, 100e3)
        .r(2, 27, 15e3)
        .r(28, 0, 3.3e3)
        .npn(27, 26, 28, q)
        .c(27, 29, 1e-6)
        .pot(0, 30, 29, 100e3, Taper::Audio, C_VOL)
        .r(30, 0, 1e6)
        .output(30);
    c.build(fs)
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ckt: Solver,
    os: Oversampler,
    dc: DcBlock,
    version: i32,
    inp: Smooth,
}

impl Dsp {
    fn controls(&mut self) {
        self.ckt.set_ctl(C_SUS, self.v[SUSTAIN] as f64 / 10.0);
        self.ckt.set_ctl(C_TONE, self.v[TONE] as f64 / 10.0);
        self.ckt.set_ctl(C_VOL, self.v[VOLUME] as f64 / 10.0);
    }
    fn rebuild(&mut self) {
        self.version = self.v[VERSION].round() as i32;
        self.ckt = big_muff(self.version == 1, self.fs * OS as f64);
        self.controls();
        self.ckt.reset();
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, ckt: big_muff(false, fs * OS as f64), os: Oversampler::new(), dc: DcBlock::new(fs), version: -1, inp: Smooth::new(fs, 20.0, 1.0) };
        d.rebuild();
        d
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn reset(&mut self) {
        self.os.reset();
        self.dc.reset();
        self.rebuild();
        self.inp.snap(db_to_gain(self.v[GUITAR] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        if v[VERSION].round() as i32 != self.version {
            self.rebuild();
        } else {
            self.controls();
        }
        self.inp.set(db_to_gain(v[GUITAR] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().max(1);
        let mut peak = 0.0f64;
        for k in 0..n {
            let x = chans.iter().map(|c| c[k] as f64).sum::<f64>() / nch as f64 * self.inp.next();
            let ckt = &mut self.ckt;
            let y = self.os.process(x, |u| ckt.step(&[u]));
            // the CLIPPING lamp follows the first clipper's diodes
            peak = peak.max((ckt.node(12) - ckt.node(14)).abs());
            let y = self.dc.process(y);
            for c in chans.iter_mut() {
                c[k] = y as f32;
            }
        }
        meters[0] = (peak / 0.6) as f32;
    }
}

#[cfg(test)]
mod circuit_tests {
    use super::*;

    #[test]
    fn bias_and_speed() {
        let mut c = big_muff(false, 192000.0);
        c.set_ctl(C_SUS, 0.6);
        c.set_ctl(C_TONE, 0.5);
        c.set_ctl(C_VOL, 0.4);
        c.reset();
        eprintln!("collectors: Q1 {:.2} Q2 {:.2} Q3 {:.2} Q4 {:.2} V", c.node(5), c.node(12), c.node(17), c.node(27));
        for col in [5, 12, 17, 27] {
            assert!(c.node(col) > 2.5 && c.node(col) < 7.5, "collector {col}: {}", c.node(col));
        }
        // accuracy of a loose Newton tolerance against a tight one
        for tol in if std::env::var("NO_TOL").is_ok() { vec![] } else { vec![1e-4, 1e-3, 3e-3] } {
            let mut a = c.clone();
            let mut b = c.clone();
            a.tol = 1e-8;
            b.tol = tol;
            let (mut e2, mut s2, mut it) = (0.0, 0.0, 0usize);
            for k in 0..48000 {
                let x = 0.3 * (k as f64 * 2.0 * std::f64::consts::PI * 196.0 / 192000.0).sin();
                let (ya, yb) = (a.step(&[x]), b.step(&[x]));
                it += b.iters;
                e2 += (ya - yb) * (ya - yb);
                s2 += ya * ya;
            }
            eprintln!("tol {tol}: error {:.1} dB, {:.2} iterations", 10.0 * (e2 / s2).log10(), it as f64 / 48000.0);
        }
        let t = std::time::Instant::now();
        let mut y2 = 0.0;
        let mut hist = [0usize; 32];
        for k in 0..192000 {
            let y = c.step(&[0.1 * (k as f64 * 2.0 * std::f64::consts::PI * 196.0 / 192000.0).sin()]);
            hist[c.iters.min(31)] += 1;
            y2 += y * y;
        }
        eprintln!("newton iterations: {:?}, refactors {}, size {:?}", hist, c.refactors, c.size());
        eprintln!("1 s at 4x in {:.0} ms, out {:.1} dBV", t.elapsed().as_secs_f64() * 1e3, 10.0 * (y2 / 192000.0).log10());
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
