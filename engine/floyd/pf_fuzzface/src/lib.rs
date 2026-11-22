//! Fuzz Face: the Dallas Arbiter Fuzz Face as David Gilmour used it from 1970 (two BC108 silicon transistors), or the
//! earlier AC128 germanium one (positive ground). The circuit itself, solved from the schematic's part values: the two
//! transistors with the 100k feedback, the 1k Fuzz pot and its bypass cap, the 470R / 8k2 collector load (BIAS trims the
//! 8k2, as the internal trimmer on later units), the 500k volume pot, and the guitar in front: pickup coil, cable and
//! the guitar's own volume pot, which the Fuzz Face's low input impedance loads (that's why it cleans up).

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
pub enum Transistors {
    #[name = "BC108 silicon"]
    Bc108Silicon,
    #[name = "AC128 germanium"]
    Ac128Germanium,
}

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum Pickup {
    #[name = "Single coil"]
    SingleCoil,
    #[name = "Humbucker"]
    Humbucker,
    #[name = "Buffered (no loading)"]
    BufferedNoLoading,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "volume"]
    pub volume: FloatParam,
    #[id = "fuzz"]
    pub fuzz: FloatParam,
    #[id = "guitar_vol"]
    pub guitar_vol: FloatParam,
    #[id = "bias"]
    pub bias: FloatParam,
    #[id = "transistors"]
    pub transistors: EnumParam<Transistors>,
    #[id = "pickup"]
    pub pickup: EnumParam<Pickup>,
    #[id = "level"]
    pub level: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            volume: FloatParam::new("Volume", 6.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            fuzz: FloatParam::new("Fuzz", 8.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            guitar_vol: FloatParam::new("Guitar Volume", 10.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            bias: FloatParam::new("Bias", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            transistors: EnumParam::new("Transistors", Transistors::Bc108Silicon),
            pickup: EnumParam::new("Pickup", Pickup::SingleCoil),
            level: db("Guitar Level", 0.0, -12.0, 12.0),
        }
    }
}

pub const VOLUME: usize = 0;
pub const FUZZ: usize = 1;
pub const GUITAR_VOL: usize = 2;
pub const BIAS: usize = 3;
pub const TRANSISTORS: usize = 4;
pub const PICKUP: usize = 5;
pub const LEVEL: usize = 6;
pub const NPARAMS: usize = 7;

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
            self.fuzz.as_ptr(),
            self.guitar_vol.as_ptr(),
            self.bias.as_ptr(),
            self.transistors.as_ptr(),
            self.pickup.as_ptr(),
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

use vintage::amp::net::Taper;
use vintage::circuit::{self, BjtModel, Circuit, Solver};

const OS: usize = 4;
const C_VOL: usize = 0;
const C_FUZZ: usize = 1;
const C_GUITAR: usize = 2;
const C_BIAS: usize = 3;

/// The guitar and the pedal, from the pickup to the volume pot's wiper.
fn fuzz_face(ge: bool, pickup: i32, fs: f64) -> Solver {
    // pickup: (DC resistance, inductance); a buffer has neither
    let (rp, lp) = match pickup {
        0 => (6.5e3, 2.6),
        1 => (8.5e3, 4.2),
        _ => (100.0, 0.0),
    };
    let mut c = Circuit::new().input(1, rp);
    c = if lp > 0.0 { c.l(1, 2, lp) } else { c.r(1, 2, 1.0) };
    // the pickup's own capacitance and a 3 m cable
    c = c.c(2, 0, 450e-12);
    if pickup < 2 {
        // the guitar's 250k audio volume pot (wiper to the jack)
        c = c.pot(0, 3, 2, 250e3, Taper::Audio, C_GUITAR);
    } else {
        c = c.r(2, 3, 1.0).r(3, 0, 1e6);
    }
    // selected transistors, as in a good unit: a lower-gain Q1 and a higher-gain Q2
    let (vcc, q1, q2, ce) = if ge {
        (-9.0, BjtModel { bf: 70.0, ..circuit::AC128 }, BjtModel { bf: 110.0, ..circuit::NKT275 }, 22e-6)
    } else {
        (9.0, BjtModel { bf: 100.0, ..circuit::BC108 }, BjtModel { bf: 130.0, ..circuit::BC108 }, 20e-6)
    };
    c = c
        .dc(10, vcc, 2.0)
        // input cap into Q1's base, the 100k feedback from Q2's emitter
        .c(3, 4, 2.2e-6)
        .r(6, 4, 100e3)
        // Q1: collector load 33k, emitter to ground; Q2's base on Q1's collector
        .r(10, 5, 33e3)
        // Q2: emitter through the 1k Fuzz pot to ground, its wiper bypassed by the electrolytic
        .pot(0, 7, 6, 1e3, Taper::Linear, C_FUZZ)
        .c(7, 0, ce)
        // collector load: 470R to the supply, the output tap, then the 8k2 (trimmable) to the collector
        .r(10, 9, 470.0)
        // (noon sets Q2's collector near half the supply for the transistors fitted)
        .rheo(9, 8, if ge { 19e3 } else { 11e3 }, Taper::Linear, C_BIAS, 2.2e3)
        // output: 10n into the 500k audio volume, wiper into the next input (1M)
        .c(9, 11, 10e-9)
        .pot(0, 12, 11, 500e3, Taper::Audio, C_VOL)
        .r(12, 0, 1e6)
        .output(12);
    c = if ge { c.pnp(5, 4, 0, q1).pnp(8, 5, 6, q2) } else { c.npn(5, 4, 0, q1).npn(8, 5, 6, q2) };
    c.build(fs)
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ckt: Solver,
    os: Oversampler,
    dc: DcBlock,
    key: (i32, i32),
    inp: Smooth,
}

impl Dsp {
    fn controls(&mut self) {
        let v = self.v;
        self.ckt.set_ctl(C_VOL, v[VOLUME] as f64 / 10.0);
        self.ckt.set_ctl(C_FUZZ, v[FUZZ] as f64 / 10.0);
        self.ckt.set_ctl(C_GUITAR, v[GUITAR_VOL] as f64 / 10.0);
        self.ckt.set_ctl(C_BIAS, v[BIAS] as f64 / 10.0);
    }
    fn rebuild(&mut self) {
        let k = (self.v[TRANSISTORS].round() as i32, self.v[PICKUP].round() as i32);
        self.ckt = fuzz_face(k.0 == 1, k.1, self.fs * OS as f64);
        self.key = k;
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
        let mut d = Self { fs, v, ckt: fuzz_face(false, 0, fs * OS as f64), os: Oversampler::new(), dc: DcBlock::new(fs), key: (-1, -1), inp: Smooth::new(fs, 20.0, 1.0) };
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
        self.inp.snap(db_to_gain(self.v[LEVEL] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        if (v[TRANSISTORS].round() as i32, v[PICKUP].round() as i32) != self.key {
            self.rebuild();
        } else {
            self.controls();
        }
        self.inp.set(db_to_gain(v[LEVEL] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().max(1);
        let mut peak = 0.0f64;
        for k in 0..n {
            // a guitar pedal is mono: full scale is 1 V from the pickups
            let x = chans.iter().map(|c| c[k] as f64).sum::<f64>() / nch as f64 * self.inp.next();
            let ckt = &mut self.ckt;
            let y = self.os.process(x, |u| ckt.step(&[u]));
            let y = self.dc.process(y);
            peak = peak.max(y.abs());
            for c in chans.iter_mut() {
                c[k] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Time / Money Solo", values: &[(FUZZ, 9.0)] },
        Preset { name: "Full Fuzz", values: &[(VOLUME, 6.5), (FUZZ, 10.0)] },
        Preset { name: "Cleaned Up (guitar on 6)", values: &[(VOLUME, 7.5), (FUZZ, 10.0), (GUITAR_VOL, 6.0)] },
        Preset { name: "Echoes Violin (with Echorec)", values: &[(FUZZ, 7.0), (GUITAR_VOL, 7.5)] },
        Preset { name: "Low Fuzz Crunch", values: &[(VOLUME, 7.0), (FUZZ, 3.0)] },
        Preset { name: "Germanium (pre-1970)", values: &[(VOLUME, 6.5), (FUZZ, 9.0), (TRANSISTORS, 1.0)] },
        Preset { name: "Starved Bias Splat", values: &[(VOLUME, 7.0), (FUZZ, 10.0), (BIAS, 1.5)] },
    ]
}

/*
('switch', 443.0, 124.5, 39, 14)
('knob', 199.5, 159.5, 41.5)
('knob', 439.5, 159.5, 41.5)
('switch', 436.0, 194.5, 39, 14)
('divider', 250)
('knob', 79.5, 319.5, 23.2)
('selector?', 199.5, 319.5, 36, 36)
('knob', 319.5, 319.5, 22.0)
('knob', 559.5, 319.5, 19.5)

crate: ['floyd/pf_fuzzface', 'library/panic_abort']
vintage/src/ui.rs
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
AFTER THE DALLAS ARBITER FUZZ FACE2 x BC108 SILICON  
  1970VOLUMEOutput level (500k).FUZZThe 1k pot on Q2's emitter: how much of it the 20 uF cap bypasses. Up = more gain, harder clipping.OUTPUTGUITAR VOLUMEYour guitar's volume knob. A Fuzz Face loads the pickup heavily, so backing it off cleans the fuzz up rather than just turning it down.SINGLEHUMB.BUFFER
PICKUPWhat the pedal sees at its input: a single-coil or humbucking pickup (the inductance darkens the tone into the low input impedance), or a buffer (a wah, or a buffered pedal before it).BIASQ2's collector load (the internal trimmer). Noon is about right; low gets gated and splatty, high gets smoother.TRANSISTORSBC108AC128BC108 silicon (Gilmour's from 1970: brighter, more gain) or the earlier AC128 germanium (warmer, smoother).GUITAR LEVELHow hot your DI is: 0 dB treats full scale as 1 V from the pickups.FUZZ FACEfloyd/pf_fuzzface/src/lib.rs
Time / Money Solo
Full Fuzz
Cleaned Up (guitar on 6)
Echoes Violin (with Echorec)
Low Fuzz Crunch
Germanium (pre-1970)
Starved Bias Splat

*/
fn panel() -> Panel {
    Panel {
        title: "FUZZ FACE",
        subtitle: ("AFTER THE DALLAS ARBITER FUZZ FACE", "2 x BC108 SILICON  ·  1970"),
        accent: ui::TANGERINE,
        size: (W, H),
        items: vec![
            Item::Knob { p: VOLUME, x: 200.0, y: 160.0, r: 42.0, label: "VOLUME", tip: "Output level (500k).", active: None },
            Item::Knob { p: FUZZ, x: 440.0, y: 160.0, r: 42.0, label: "FUZZ", tip: "The 1k pot on Q2's emitter: how much of it the 20 uF cap bypasses. Up = more gain, harder clipping.", active: None },
            Item::Lamp { m: 0, x: 320.0, y: 160.0, label: "OUTPUT", lo: 0.05, hi: 0.6 },
            Item::Divider { y: 250.0 },
            Item::Knob { p: GUITAR_VOL, x: 80.0, y: 320.0, r: 24.0, label: "GUITAR VOLUME", tip: "Your guitar's volume knob. A Fuzz Face loads the pickup heavily, so backing it off cleans the fuzz up rather than just turning it down.", active: None },
            Item::Selector { p: PICKUP, x: 200.0, y: 320.0, label: "PICKUP", positions: &["SINGLE", "HUMB.", "BUFFER"], tip: "What the pedal sees at its input: a single-coil or humbucking pickup (the inductance darkens the tone into the low input impedance), or a buffer (a wah, or a buffered pedal before it)." },
            Item::Knob { p: BIAS, x: 320.0, y: 320.0, r: 22.0, label: "BIAS", tip: "Q2's collector load (the internal trimmer). Noon is about right; low gets gated and splatty, high gets smoother.", active: None },
            Item::Text { x: 440.0, y: 280.0, text: "TRANSISTORS", size: 13.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: TRANSISTORS, x: 440.0, y: 320.0, labels: ["BC108", "AC128"], tip: "BC108 silicon (Gilmour's from 1970: brighter, more gain) or the earlier AC128 germanium (warmer, smoother)." },
            Item::Knob { p: LEVEL, x: 560.0, y: 320.0, r: 20.0, label: "GUITAR LEVEL", tip: "How hot your DI is: 0 dB treats full scale as 1 V from the pickups.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Fuzz Face", class_id: *b"PFFuzzFacePQD001", vendor: "PEQUOD Pink Floyd",
    categories: [Fx, Distortion]
);

#[cfg(test)]
mod tests {
    use super::*;

    /// The operating point a working Fuzz Face measures: Q2's collector about half the supply.
    #[test]
    fn bias_points() {
        for ge in [false, true] {
            let mut c = fuzz_face(ge, 0, 192000.0);
            for (ctl, pos) in [(C_VOL, 0.7), (C_FUZZ, 1.0), (C_GUITAR, 1.0), (C_BIAS, 0.5)] {
                c.set_ctl(ctl, pos);
            }
            c.reset();
            let (q1c, q2e, q2c) = (c.node(5), c.node(6), c.node(8));
            eprintln!("ge {ge}: Q1 c {q1c:.2} V, Q2 e {q2e:.2} V, Q2 c {q2c:.2} V");
            assert!(q2c.abs() > 3.0 && q2c.abs() < 6.5, "Q2 collector {q2c}");
        }
    }

    #[test]
    fn gain_and_cleanup() {
        let fs = 48000.0;
        let run = |gv: f32, amp: f64| {
            let mut d = Dsp::new(fs as f32, 2);
            let mut v = defaults();
            v[GUITAR_VOL] = gv;
            v[FUZZ] = 10.0;
            v[VOLUME] = 7.0;
            d.set_params(&v);
            d.reset();
            let n = 24000;
            let mut l: Vec<f32> = (0..n).map(|k| (amp * (k as f64 * 2.0 * std::f64::consts::PI * 196.0 / fs).sin()) as f32).collect();
            let mut r = l.clone();
            let mut m = [0.0f32; 1];
            d.process(&mut [&mut l[..], &mut r[..]], &mut m);
            let tail = &l[n / 2..];
            let rms = (tail.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / tail.len() as f64).sqrt();
            // harmonics vs fundamental (Goertzel at 196 Hz and its multiples)
            let bin = |f: f64| {
                let w = 2.0 * std::f64::consts::PI * f / fs;
                let (mut re, mut im) = (0.0, 0.0);
                for (i, x) in tail.iter().enumerate() {
                    re += *x as f64 * (w * i as f64).cos();
                    im += *x as f64 * (w * i as f64).sin();
                }
                (re * re + im * im).sqrt()
            };
            let f1 = bin(196.0);
            let h: f64 = (2..10).map(|k| bin(196.0 * k as f64).powi(2)).sum::<f64>().sqrt();
            (20.0 * rms.log10(), 20.0 * (h / f1).log10())
        };
        for (gv, amp) in [(10.0, 0.1), (10.0, 0.01), (7.0, 0.1), (5.0, 0.1), (4.0, 0.1), (3.0, 0.1), (2.0, 0.1)] {
            let (db, thd) = run(gv, amp);
            eprintln!("guitar vol {gv}, {amp} V in: out {db:.1} dBV, harmonics {thd:.1} dB");
        }
    }
}


#[cfg(test)]
mod ge_debug {
    use super::*;
    #[test]
    fn ge_sweep() {
        let fs = 192000.0;
        for ge in [false, true] {
            let mut c = fuzz_face(ge, 0, fs);
            for (ctl, pos) in [(C_VOL, 0.6), (C_FUZZ, 0.9), (C_GUITAR, 1.0), (C_BIAS, 0.5)] {
                c.set_ctl(ctl, pos);
            }
            c.reset();
            for amp in [0.01, 0.03, 0.1, 0.3, 0.6] {
                let (mut q2, mut y2, mut ymean) = (0.0, 0.0, 0.0);
                let n = 96000;
                for k in 0..n {
                    let y = c.step(&[amp * (k as f64 * 2.0 * std::f64::consts::PI * 110.0 / fs).sin()]);
                    if k >= n / 2 {
                        q2 += c.node(8);
                        y2 += y * y;
                        ymean += y;
                    }
                }
                let m = (n / 2) as f64;
                eprintln!("ge {ge} amp {amp}: Q2 c mean {:.2} V, out rms {:.1} dBV (dc {:.3})", q2 / m, 10.0 * (y2 / m).log10(), ymean / m);
            }
        }
    }
}
