//! Power Boost: the Colorsound Power Boost (18 V), David Gilmour's clean boost and overdrive on The Dark Side of the Moon
//! and Wish You Were Here. A gain stage, the Baxandall bass and treble, and a second stage whose gain sets how hard it clips.

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


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "volume"]
    pub volume: FloatParam,
    #[id = "bass"]
    pub bass: FloatParam,
    #[id = "treble"]
    pub treble: FloatParam,
    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "level"]
    pub level: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            volume: FloatParam::new("Volume", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            bass: FloatParam::new("Bass", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            treble: FloatParam::new("Treble", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            gain: FloatParam::new("Gain", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            level: db("Guitar Level", 0.0, -12.0, 12.0),
        }
    }
}

pub const VOLUME: usize = 0;
pub const BASS: usize = 1;
pub const TREBLE: usize = 2;
pub const GAIN: usize = 3;
pub const LEVEL: usize = 4;
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
            self.bass.as_ptr(),
            self.treble.as_ptr(),
            self.gain.as_ptr(),
            self.level.as_ptr(),
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

/// Output scaling from the pedal's volts to the plugin's level: the hardware's audio-taper volume
/// sits about 20 dB down at noon, and this brings the default setting to the expected level.
const MAKEUP: f64 = 11.8850; // +21.5 dB

use vintage::amp::net::Taper;
use vintage::circuit::{self, Circuit, Solver};

const OS: usize = 4;
const C_GAIN: usize = 0;
const C_BASS: usize = 1;
const C_TREBLE: usize = 2;
const C_VOL: usize = 3;
/// Q2's collector (the end of the gain pair) and its resting voltage, for the drive lamp.
const N_Q2C: usize = 7;

/// The whole pedal at `supply` volts (two PP3s: 18 V).
///
/// Three BC184Ls. Q1/Q2 are a direct-coupled feedback pair: Q2's collector feeds back to Q1's
/// emitter through 12k, and the gain pot (10k, reverse log, to ground through 22u) sets how much of
/// Q1's emitter is grounded for audio: about 2x fully down, rising to open loop fully up. DC is held by 150k + 120k from
/// Q2's emitter back to Q1's base, decoupled in the middle. Q3 is an inverting stage with the bass
/// and treble network wrapped around it (Colorsound's feedback Baxandall), then the volume pot.
/// Resistor placement follows the published parts list fitted to measured voltages of an 18 V unit
/// (Q1 c/b/e 6.4/3.9/3.5 V, Q2 c/e 11.5/5.8 V, Q3 c/b/e 10.2/2.8/2.2 V at 18.8 V).
fn power_boost(supply: f64, fs: f64) -> Solver {
    let q = circuit::BC184;
    Circuit::new()
        .input(1, 7e3)
        .dc(2, supply, 5.0)
        // input: 1M to ground, 220n into Q1's base
        .r(1, 0, 1e6)
        .c(1, 3, 220e-9)
        // Q1
        .r(2, 4, 5.6e3)
        .c(4, 3, 220e-12)
        .r(5, 0, 1.2e3)
        .npn(4, 3, 5, q)
        // gain pot to ground through 22u
        .rheo(5, 6, 10e3, Taper::Audio, C_GAIN, 10.0)
        .c(6, 0, 22e-6)
        // Q2, direct from Q1's collector; 12k feedback from its collector to Q1's emitter
        .r(2, N_Q2C, 3.9e3)
        .r(8, 0, 4.7e3)
        .npn(N_Q2C, 4, 8, q)
        .r(N_Q2C, 5, 12e3)
        // DC bias: Q2's emitter back to Q1's base, decoupled halfway
        .r(8, 9, 150e3)
        .r(9, 3, 120e3)
        .c(9, 0, 22e-6)
        // into the tone network
        .c(N_Q2C, 10, 22e-6)
        // bass: 4k7, the 100k pot with 100n across it, 4k7 to Q3's collector side; wiper through 39k
        .r(10, 11, 4.7e3)
        .pot(13, 12, 11, 100e3, Taper::Linear, C_BASS)
        .c(11, 13, 100e-9)
        .r(13, 14, 4.7e3)
        .r(12, 15, 39e3)
        // treble: 10n, the 100k pot, 10n; wiper straight to Q3's base
        .c(10, 16, 10e-9)
        .pot(18, 17, 16, 100e3, Taper::Linear, C_TREBLE)
        .c(18, 14, 10e-9)
        .r(17, 15, 1.0)
        // Q3: 180k/33k bias, 4k7 collector, 1k2 emitter with 470R + 10u across it
        .r(2, 15, 180e3)
        .r(15, 0, 33e3)
        .r(2, 19, 4.7e3)
        .r(20, 0, 1.2e3)
        .r(20, 21, 470.0)
        .c(21, 0, 10e-6)
        .npn(19, 15, 20, q)
        // the network's far end is AC-coupled to the collector
        .c(19, 14, 1e-6)
        // out: 22u, the 100k volume pot
        .c(19, 22, 22e-6)
        .pot(0, 23, 22, 100e3, Taper::Audio, C_VOL)
        .r(23, 0, 1e6)
        .output(23)
        .build(fs)
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ckt: [Solver; 2],
    os: [Oversampler; 2],
    dc: [DcBlock; 2],
    rest: f64,
    inp: Smooth,
}

impl Dsp {
    fn controls(&mut self) {
        for c in self.ckt.iter_mut() {
            // reverse-log gain pot: more gain, less resistance
            c.set_ctl(C_GAIN, 1.0 - self.v[GAIN] as f64 / 10.0);
            c.set_ctl(C_BASS, self.v[BASS] as f64 / 10.0);
            c.set_ctl(C_TREBLE, self.v[TREBLE] as f64 / 10.0);
            c.set_ctl(C_VOL, self.v[VOLUME] as f64 / 10.0);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let ckt = power_boost(18.0, fs * OS as f64);
        let rest = ckt.node(N_Q2C);
        let mut d = Self { fs, v, ckt: [ckt.clone(), ckt], os: [Oversampler::new(), Oversampler::new()], dc: [DcBlock::new(fs), DcBlock::new(fs)], rest, inp: Smooth::new(fs, 20.0, 1.0) };
        d.reset();
        d
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn reset(&mut self) {
        self.controls();
        for c in 0..2 {
            self.ckt[c].reset();
            self.os[c].reset();
            self.dc[c] = DcBlock::new(self.fs);
        }
        self.rest = self.ckt[0].node(N_Q2C);
        self.inp.snap(db_to_gain(self.v[LEVEL] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.controls();
        self.inp.set(db_to_gain(v[LEVEL] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(2);
        let mut peak = 0.0f64;
        for k in 0..n {
            let gi = self.inp.next();
            for c in 0..nch {
                let x = chans[c][k] as f64 * gi;
                let ckt = &mut self.ckt[c];
                let (rest, mut pk) = (self.rest, 0.0f64);
                let y = self.os[c].process(x, |u| {
                    let y = ckt.step(&[u]);
                    pk = pk.max((ckt.node(N_Q2C) - rest).abs());
                    y
                });
                peak = peak.max(pk);
                chans[c][k] = (self.dc[c].process(y) * MAKEUP) as f32;
            }
        }
        // the lamp: Q2's collector swinging most of the way to a rail
        meters[0] = (peak / 6.0) as f32;
    }
}

#[cfg(test)]
mod circuit_tests {
    use super::*;

    #[test]
    fn bias_and_response() {
        let mut c = power_boost(18.84, 192000.0);
        c.reset();
        let v = |c: &Solver, n: usize| c.node(n);
        eprintln!(
            "Q1 c {:.2} b {:.2} e {:.2} | Q2 c {:.2} e {:.2} | Q3 c {:.2} b {:.2} e {:.2}",
            v(&c, 4), v(&c, 3), v(&c, 5), v(&c, 7), v(&c, 8), v(&c, 19), v(&c, 15), v(&c, 20)
        );
        for (n, want) in [(4, 6.43), (3, 3.92), (5, 3.54), (7, 11.53), (8, 5.82), (19, 10.19), (15, 2.77), (20, 2.23)] {
            assert!((v(&c, n) - want).abs() < 0.6, "node {n}: {} vs {want}", v(&c, n));
        }
        // small-signal gain against the gain knob, and the tone controls at 100 Hz / 5 kHz
        let fs = 192000.0;
        let gain_at = |gain: f64, bass: f64, treble: f64, f: f64| {
            let mut c = power_boost(18.0, fs);
            c.set_ctl(C_GAIN, 1.0 - gain);
            c.set_ctl(C_BASS, bass);
            c.set_ctl(C_TREBLE, treble);
            c.set_ctl(C_VOL, 1.0);
            c.reset();
            let (mut a, mut b) = (0.0, 0.0);
            let n = (fs * 0.3) as usize;
            for k in 0..n {
                let x = 0.005 * (k as f64 * 2.0 * std::f64::consts::PI * f / fs).sin();
                let y = c.step(&[x]);
                if k > n / 2 {
                    a += x * x;
                    b += y * y;
                }
            }
            10.0 * (b / a).log10()
        };
        {
            let mut c = power_boost(18.0, fs);
            c.set_ctl(C_GAIN, 1.0);
            c.set_ctl(C_VOL, 1.0);
            c.reset();
            let nodes = [1usize, 3, 4, 5, 6, 7, 8, 9, 10, 15];
            let mut mean = [0.0f64; 10];
            let mut sq = [0.0f64; 10];
            let n = 57600;
            for k in 0..n {
                c.step(&[0.005 * (k as f64 * 2.0 * std::f64::consts::PI * 1000.0 / fs).sin()]);
                if k >= n / 2 {
                    for (j, &nd) in nodes.iter().enumerate() {
                        mean[j] += c.node(nd);
                        sq[j] += c.node(nd) * c.node(nd);
                    }
                }
            }
            for (j, &nd) in nodes.iter().enumerate() {
                let m = mean[j] / (n / 2) as f64;
                let rms = (sq[j] / (n / 2) as f64 - m * m).max(0.0).sqrt();
                eprintln!("node {nd}: dc {m:.2} ac {:.1} dBV", 20.0 * rms.log10());
            }
        }
        for g in [0.0, 0.3, 0.5, 0.7, 0.9] {
            eprintln!("gain {g}: {:.1} dB at 1 kHz", gain_at(g, 0.5, 0.5, 1000.0));
        }
        for (b, t) in [(0.0, 0.5), (1.0, 0.5), (0.5, 0.0), (0.5, 1.0), (0.5, 0.5)] {
            eprintln!(
                "bass {b} treble {t}: 100 Hz {:.1}, 1 kHz {:.1}, 5 kHz {:.1} dB",
                gain_at(0.0, b, t, 100.0), gain_at(0.0, b, t, 1000.0), gain_at(0.0, b, t, 5000.0)
            );
        }
        let t = std::time::Instant::now();
        let mut c = power_boost(18.0, fs);
        c.set_ctl(C_GAIN, 0.2);
        c.reset();
        for k in 0..192000 {
            c.step(&[0.3 * (k as f64 * 2.0 * std::f64::consts::PI * 196.0 / fs).sin()]);
        }
        eprintln!("1 s at 4x in {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Clean Push (into a Hiwatt)", values: &[(VOLUME, 6.0), (GAIN, 2.0)] },
        Preset { name: "Edge of Breakup Booster", values: &[(VOLUME, 5.5), (TREBLE, 5.5), (GAIN, 4.5)] },
        Preset { name: "Fat Lead Overdrive", values: &[(VOLUME, 4.5), (BASS, 6.0), (GAIN, 8.0)] },
        Preset { name: "Shine On Lead (with Muff)", values: &[(VOLUME, 5.5), (BASS, 4.5), (TREBLE, 6.0), (GAIN, 3.5)] },
        Preset { name: "Dark Side Lead Boost", values: &[(TREBLE, 6.5), (GAIN, 6.0)] },
        Preset { name: "Full Power", values: &[(VOLUME, 4.0), (BASS, 5.5), (TREBLE, 5.5), (GAIN, 10.0)] },
    ]
}

/*
('knob', 109.5, 159.5, 37.8)
('knob', 249.5, 159.5, 29.3)
('knob', 369.5, 159.5, 29.3)
('knob', 509.5, 159.5, 37.8)
('divider', 284)
('knob', 319.5, 335.5, 22.0)

crate: ['floyd/pf_powerboost', 'library/panic_abort']
vintage/src/ui.rs
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
AFTER THE COLORSOUND POWER BOOST18 VOLTS  
  BAXANDALL BASS & TREBLEVOLUMEOutput level.BASSBaxandall bass: boost or cut below about 200 Hz.TREBLEBaxandall treble: boost or cut above about 2 kHz.GAINSecond-stage gain. Low: a clean boost with a big tone range. High: the stage clips into a fat, mid-heavy overdrive.DRIVEGUITAR LEVELHow hot your DI is (0 dB treats full scale as 1 V from the pickups).POWER BOOST
Clean Push (into a Hiwatt)
Edge of Breakup Booster
Fat Lead Overdrive
Shine On Lead (with Muff)
Dark Side Lead Boost
Full Power
vintage/src/amp/net.rs
j?$Dsp

*/
fn panel() -> Panel {
    Panel {
        title: "POWER BOOST",
        subtitle: ("AFTER THE COLORSOUND POWER BOOST", "18 VOLTS  ·  BAXANDALL BASS & TREBLE"),
        accent: ui::TANGERINE,
        size: (W, H),
        items: vec![
            Item::Knob { p: VOLUME, x: 110.0, y: 160.0, r: 38.0, label: "VOLUME", tip: "Output level.", active: None },
            Item::Knob { p: BASS, x: 250.0, y: 160.0, r: 30.0, label: "BASS", tip: "Baxandall bass: boost or cut below about 200 Hz.", active: None },
            Item::Knob { p: TREBLE, x: 370.0, y: 160.0, r: 30.0, label: "TREBLE", tip: "Baxandall treble: boost or cut above about 2 kHz.", active: None },
            Item::Knob { p: GAIN, x: 510.0, y: 160.0, r: 38.0, label: "GAIN", tip: "Second-stage gain. Low: a clean boost with a big tone range. High: the stage clips into a fat, mid-heavy overdrive.", active: None },
            Item::Lamp { m: 0, x: 320.0, y: 247.0, label: "DRIVE", lo: 0.3, hi: 1.0 },
            Item::Divider { y: 284.0 },
            Item::Knob { p: LEVEL, x: 320.0, y: 336.0, r: 22.0, label: "GUITAR LEVEL", tip: "How hot your DI is (0 dB treats full scale as 1 V from the pickups).", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Power Boost", class_id: *b"PFPowerBoostPQD1", vendor: "PEQUOD Pink Floyd",
    categories: [Fx, Distortion]
);
