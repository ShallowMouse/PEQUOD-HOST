//! Fuzz Factory: the Z.Vex Fuzz Factory, built into Matt Bellamy's Manson guitars and on his board: a germanium
//! Fuzz Face core whose bias points are all on knobs. Drive is the fuzz, Comp and Gate move the two transistors'
//! operating points (squash, sputter, gating), Stab starves the supply. Simulated as a circuit; the Factory's own
//! resistor values are estimated around the Fuzz Face it grew from.

#![allow(clippy::excessive_precision)]
use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
#[allow(unused_imports)]
use vintage::dsp::*;
use vintage::nih_plug_egui::EguiState;
#[allow(unused_imports)]
use vintage::params::*;
#[allow(unused_imports)]
use vintage::ui::{self, ActiveWhen, Item, MeterKind, Panel, Preset};
#[allow(unused_imports)]
use vintage::{Midi, VintageDsp, VintageParams};

pub const W: u32 = 640;
pub const H: u32 = 400;

fn knob(name: &str, def: f32) -> FloatParam {
    FloatParam::new(name, def, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1).with_value_to_string(formatters::v2s_f32_rounded(1))
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "volume"]
    pub volume: FloatParam,
    #[id = "gate"]
    pub gate: FloatParam,
    #[id = "comp"]
    pub comp: FloatParam,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "stab"]
    pub stab: FloatParam,
    #[id = "level"]
    pub level: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            volume: knob("Volume", 5.0),
            gate: knob("Gate", 10.0),
            comp: knob("Comp", 5.0),
            drive: knob("Drive", 10.0),
            stab: knob("Stab", 10.0),
            level: db("Guitar Level", 0.0, -12.0, 12.0),
        }
    }
}

pub const VOLUME: usize = 0;
pub const GATE: usize = 1;
pub const COMP: usize = 2;
pub const DRIVE: usize = 3;
pub const STAB: usize = 4;
pub const LEVEL: usize = 5;
pub const NPARAMS: usize = 6;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![self.volume.as_ptr(), self.gate.as_ptr(), self.comp.as_ptr(), self.drive.as_ptr(), self.stab.as_ptr(), self.level.as_ptr()]
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
use vintage::circuit::{self, BjtModel, Circuit, Solver};

/// Output scaling from the pedal's volts to the plugin's level (the audio-taper volume sits
/// about 20 dB down at noon).
const MAKEUP: f64 = 5.9566; // +15.5 dB
const C_VOL: usize = 0;
const C_DRIVE: usize = 1;
const C_COMP: usize = 2;
const C_STAB: usize = 3;
/// Gate: Q2's collector bias resistance (a variable slot: its curve isn't a plain taper).
const V_GATE: usize = 0;
const N_Q2C: usize = 8;

fn gate_ohms(g: f64) -> f64 {
    // fully up: Q2's collector near half the supply; down: starved toward cut-off, so it sputters and gates
    4.7e3 + 5e3 * g + 30e3 * (1.0 - g) * (1.0 - g)
}

/// The pedal: two germanium PNPs on -9 V (as the Fuzz Face it's built on), input from the guitar
/// (a 7k source). Stab: 50k rheostat in the supply, decoupled by 100u. Comp: 10k in Q1's collector
/// load. Drive: the 1k pot in Q2's emitter, bypassed by 22u at its wiper. Gate: Q2's collector bias.
fn fuzz_factory(fs: f64) -> Solver {
    let q1 = BjtModel { bf: 80.0, ..circuit::NKT275 };
    let q2 = BjtModel { bf: 110.0, ..circuit::NKT275 };
    Circuit::new()
        .input(1, 7e3)
        .dc(20, -9.0, 2.0)
        // Stab: the supply through up to 50k, decoupled
        .rheo(20, 10, 50e3, Taper::Linear, C_STAB, 100.0)
        .c(10, 0, 100e-6)
        // input
        .c(1, 4, 2.2e-6)
        .r(6, 4, 100e3)
        // Q1 with Comp in its collector load
        .r(13, 5, 22e3)
        .rheo(10, 13, 10e3, Taper::Linear, C_COMP, 10.0)
        .pnp(5, 4, 0, q1)
        // Q2: Drive in its emitter
        .pot(0, 7, 6, 1e3, Taper::Linear, C_DRIVE)
        .c(7, 0, 22e-6)
        .pnp(N_Q2C, 5, 6, q2)
        // Gate in its collector bias, 470R to the rail, the output tap between
        .r(10, 9, 470.0)
        .var(9, N_Q2C)
        // out through 10n to the 100k audio volume
        .c(9, 11, 10e-9)
        .pot(0, 12, 11, 100e3, Taper::Audio, C_VOL)
        .r(12, 0, 1e6)
        .output(12)
        .build(fs)
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ckt: Solver,
    os: Oversampler,
    dc: DcBlock,
    inp: Smooth,
}

impl Dsp {
    fn controls(&mut self) {
        let v = self.v;
        self.ckt.set_ctl(C_VOL, v[VOLUME] as f64 / 10.0);
        self.ckt.set_ctl(C_DRIVE, v[DRIVE] as f64 / 10.0);
        self.ckt.set_ctl(C_COMP, v[COMP] as f64 / 10.0);
        // Stab up: less resistance in the supply
        self.ckt.set_ctl(C_STAB, 1.0 - v[STAB] as f64 / 10.0);
        self.ckt.set_var(V_GATE, gate_ohms(v[GATE] as f64 / 10.0));
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, ckt: fuzz_factory(fs * OS as f64), os: Oversampler::new(), dc: DcBlock::new(fs), inp: Smooth::new(fs, 20.0, 1.0) };
        d.reset();
        d
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn reset(&mut self) {
        self.controls();
        self.ckt.reset();
        self.os.reset();
        self.dc.reset();
        self.inp.snap(db_to_gain(self.v[LEVEL] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        let bias = v[STAB] != self.v[STAB] || v[GATE] != self.v[GATE] || v[COMP] != self.v[COMP];
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.controls();
        let _ = bias;
        self.inp.set(db_to_gain(v[LEVEL] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().max(1);
        let mut peak = 0.0f64;
        for k in 0..n {
            let x = chans.iter().map(|c| c[k] as f64).sum::<f64>() / nch as f64 * self.inp.next();
            let ckt = &mut self.ckt;
            let y = self.os.process(x, |u| ckt.step(&[u]));
            let y = self.dc.process(y) * MAKEUP;
            peak = peak.max(y.abs());
            for c in chans.iter_mut() {
                c[k] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}

#[cfg(test)]
mod circuit_tests {
    use super::*;

    #[test]
    fn bias_and_controls() {
        let fs = 192000.0;
        let mut c = fuzz_factory(fs);
        c.set_ctl(C_VOL, 0.5);
        c.set_ctl(C_DRIVE, 1.0);
        c.set_ctl(C_COMP, 0.5);
        c.set_ctl(C_STAB, 0.0);
        for g in [1.0, 0.5, 0.0] {
            c.set_var(V_GATE, gate_ohms(g));
            c.reset();
            eprintln!("gate {g}: rail {:.2} Q1 c {:.2} Q2 c {:.2} e {:.2}", c.node(10), c.node(5), c.node(N_Q2C), c.node(6));
        }
        c.set_var(V_GATE, gate_ohms(1.0));
        c.reset();
        assert!(c.node(N_Q2C) < -2.0 && c.node(N_Q2C) > -7.5, "Q2 collector {}", c.node(N_Q2C));
        let mut d = Dsp::new(48000.0, 1);
        let mut x: Vec<f32> = (0..48000).map(|k| (0.1 * (k as f64 * 2.0 * std::f64::consts::PI * 196.0 / 48000.0).sin()) as f32).collect();
        let t = std::time::Instant::now();
        let mut m = [0.0f32];
        d.process(&mut [&mut x[..]], &mut m);
        let rms = (x[24000..].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / 24000.0).sqrt();
        eprintln!("default: {:.1} dBFS out, 1 s in {:.0} ms", 20.0 * rms.log10(), t.elapsed().as_secs_f64() * 1e3);
        assert!(x.iter().all(|v| v.is_finite()));
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Classic Fuzz (stable)", values: &[] },
        Preset { name: "Plug In Baby Splat", values: &[(COMP, 8.0), (GATE, 7.0)] },
        Preset { name: "Velcro Gate", values: &[(GATE, 3.0), (COMP, 6.0)] },
        Preset { name: "Starved Sputter", values: &[(STAB, 3.0), (GATE, 6.0)] },
        Preset { name: "Low-Drive Edge", values: &[(DRIVE, 3.0), (VOLUME, 6.5)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "FUZZ FACTORY",
        subtitle: ("AFTER THE Z.VEX FUZZ FACTORY", "2 x GERMANIUM · BIAS ON THE KNOBS"),
        accent: ui::MUSTARD,
        size: (W, H),
        items: vec![
            Item::Knob { p: VOLUME, x: 80.0, y: 160.0, r: 34.0, label: "VOLUME", tip: "Output volume (audio taper).", active: None },
            Item::Knob { p: GATE, x: 200.0, y: 160.0, r: 34.0, label: "GATE", tip: "Q2's bias: fully up is open; turn down and notes cut off as they decay, then sputter and gate.", active: None },
            Item::Knob { p: COMP, x: 320.0, y: 160.0, r: 34.0, label: "COMP", tip: "Q1's collector load: more squash and a thicker, compressed fuzz as it rises.", active: None },
            Item::Knob { p: DRIVE, x: 440.0, y: 160.0, r: 34.0, label: "DRIVE", tip: "The fuzz: Q2's emitter bypass, as on a Fuzz Face.", active: None },
            Item::Knob { p: STAB, x: 560.0, y: 160.0, r: 34.0, label: "STAB", tip: "Supply stability: fully up is a stiff 9 V; turn down to starve the circuit (dying-battery splutter).", active: None },
            Item::Divider { y: 270.0 },
            Item::Knob { p: LEVEL, x: 320.0, y: 330.0, r: 22.0, label: "GUITAR LEVEL", tip: "How hot your DI is (0 dB treats full scale as 1 V from the pickups).", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Fuzz Factory", class_id: *b"MuseFuzzFactory1", vendor: "PEQUOD Muse",
    categories: [Fx, Distortion]
);
