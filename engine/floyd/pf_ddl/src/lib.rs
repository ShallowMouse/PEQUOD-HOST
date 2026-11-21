//! Digital Delay: David Gilmour's digital delays from 1977 to 1994: the MXR Digital Delay (narrow bandwidth, companded,
//! a little grainy) and the TC Electronic 2290 (full range, with its modulation section). The repeats sit under the full dry signal.

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
pub enum Machine {
    #[name = "MXR (1977-80)"]
    Mxr197780,
    #[name = "TC 2290 (1987-94)"]
    Tc2290198794,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "time"]
    pub time: FloatParam,
    #[id = "repeats"]
    pub repeats: FloatParam,
    #[id = "level"]
    pub level: FloatParam,
    #[id = "mod_depth"]
    pub mod_depth: FloatParam,
    #[id = "mod_rate"]
    pub mod_rate: FloatParam,
    #[id = "highcut"]
    pub highcut: FloatParam,
    #[id = "machine"]
    pub machine: EnumParam<Machine>,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            time: ms("Delay Time", 440.0, 20.0, 2000.0),
            repeats: pct("Feedback", 0.35),
            level: pct("Delay Level", 0.35),
            mod_depth: pct("Mod Depth", 0.0),
            mod_rate: rate("Mod Rate", 0.6, 0.1, 5.0),
            highcut: hz("High Cut", 9000.0, 1500.0, 20000.0),
            machine: EnumParam::new("Machine", Machine::Mxr197780),
            output: db("Output", 0.0, -12.0, 6.0),
        }
    }
}

pub const TIME: usize = 0;
pub const REPEATS: usize = 1;
pub const LEVEL: usize = 2;
pub const MOD_DEPTH: usize = 3;
pub const MOD_RATE: usize = 4;
pub const HIGHCUT: usize = 5;
pub const MACHINE: usize = 6;
pub const OUTPUT: usize = 7;
pub const NPARAMS: usize = 8;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.time.as_ptr(),
            self.repeats.as_ptr(),
            self.level.as_ptr(),
            self.mod_depth.as_ptr(),
            self.mod_rate.as_ptr(),
            self.highcut.as_ptr(),
            self.machine.as_ptr(),
            self.output.as_ptr(),
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

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    line: [DelayLine; 2],
    fb: [f64; 2],
    hc: [Biquad; 2],
    band: [[Biquad; 2]; 2],
    env: [f64; 2],
    ph: f64,
    time: Smooth,
    key: [i32; 2],
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let hc = (self.v[HIGHCUT] as f64).min(fs * 0.45);
        let mxr = self.v[MACHINE] < 0.5;
        for c in 0..2 {
            self.hc[c].set_lowpass(fs, hc, 0.707);
            // the MXR's converters: band-limited to about 7 kHz (a TC 2290 runs to 16 kHz)
            self.band[c][0].set_lowpass(fs, if mxr { 7000.0 } else { 16000.0f64.min(fs * 0.45) }, 0.707);
            self.band[c][1].set_highpass(fs, if mxr { 60.0 } else { 20.0 }, 0.707);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let n = (fs * 2.1) as usize;
        let mut d = Self {
            fs,
            v,
            line: [DelayLine::new(n), DelayLine::new(n)],
            fb: [0.0; 2],
            hc: [Biquad::identity(); 2],
            band: [[Biquad::identity(); 2]; 2],
            env: [0.0; 2],
            ph: 0.0,
            time: Smooth::new(fs, 60.0, v[TIME] as f64 * 0.001 * fs),
            key: [-1; 2],
            out: Smooth::new(fs, 20.0, 1.0),
        };
        d.design();
        d
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.line[c].reset();
            self.hc[c].reset();
            self.band[c][0].reset();
            self.band[c][1].reset();
            self.fb[c] = 0.0;
            self.env[c] = 0.0;
        }
        self.ph = 0.0;
        self.time.snap(self.v[TIME] as f64 * 0.001 * self.fs);
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = [v[HIGHCUT].round() as i32, v[MACHINE].round() as i32];
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.time.set(v[TIME] as f64 * 0.001 * self.fs);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fbk = self.v[REPEATS] as f64;
        let lvl = self.v[LEVEL] as f64;
        let depth = self.v[MOD_DEPTH] as f64 * 0.004 * self.fs;
        let rate = self.v[MOD_RATE] as f64;
        let mxr = self.v[MACHINE] < 0.5;
        let nch = chans.len().min(2);
        let ce = coeff(self.fs, 10.0);
        let mut peak = 0.0f64;
        for k in 0..n {
            self.ph = (self.ph + rate / self.fs).fract();
            let t = self.time.next() + depth * (0.5 + 0.5 * (2.0 * std::f64::consts::PI * self.ph).sin());
            let o = self.out.next();
            for c in 0..nch {
                let x = chans[c][k] as f64;
                let mut w = self.line[c].read(t.max(1.0));
                w = self.band[c][0].process(w);
                if mxr {
                    // the compander's breathing and a touch of grit at low levels
                    let a = w.abs();
                    self.env[c] = a + ce * (self.env[c] - a);
                    w = w + 0.002 * (w / (self.env[c] + 1e-3)).tanh() * self.env[c].sqrt();
                }
                let rep = self.hc[c].process(w);
                let into = self.band[c][1].process(x + fbk * rep);
                // the converter clips hard at full scale
                self.line[c].write(into.clamp(-1.5, 1.5));
                self.fb[c] = rep;
                peak = peak.max(w.abs());
                chans[c][k] = ((x + lvl * w) * o) as f32;
            }
        }
        meters[0] = peak as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Run Like Hell (380 ms)", values: &[(TIME, 380.0), (REPEATS, 0.3), (LEVEL, 0.7), (HIGHCUT, 8000.0)] },
        Preset { name: "Wall Lead (MXR)", values: &[(REPEATS, 0.3), (LEVEL, 0.3)] },
        Preset { name: "Animals Ambience (MXR)", values: &[(TIME, 500.0), (REPEATS, 0.4), (LEVEL, 0.3), (HIGHCUT, 6000.0)] },
        Preset { name: "Momentary Lapse Lead (TC)", values: &[(TIME, 470.0), (MOD_DEPTH, 0.25), (HIGHCUT, 12000.0), (MACHINE, 1.0)] },
        Preset { name: "Division Bell Lead (TC)", values: &[(TIME, 520.0), (REPEATS, 0.4), (LEVEL, 0.4), (MOD_DEPTH, 0.2), (HIGHCUT, 10000.0), (MACHINE, 1.0)] },
        Preset { name: "Marooned Ambience (TC)", values: &[(TIME, 650.0), (REPEATS, 0.5), (LEVEL, 0.45), (MOD_DEPTH, 0.3), (HIGHCUT, 8000.0), (MACHINE, 1.0)] },
        Preset { name: "Slapback", values: &[(TIME, 110.0), (REPEATS, 0.05), (LEVEL, 0.4)] },
    ]
}

/*
('knob', 89.5, 149.5, 35.4)
('knob', 229.5, 149.5, 29.3)
('knob', 349.5, 149.5, 29.3)
('knob', 469.5, 149.5, 23.2)
('divider', 230)
('knob', 289.5, 319.5, 23.2)
('knob', 399.5, 319.5, 23.2)
('knob', 519.5, 319.5, 22.0)

crate: ['floyd/pf_ddl', 'library/panic_abort']
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/std/src/sync/once.rs
vintage/src/ui.rs
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
AFTER THE MXR DDL AND TC ELECTRONIC 2290GILMOUR'S DELAYS, 1977 - 1994DELAY TIMEDelay time (ms). Gilmour usually set it in time with the song, often dotted eighths.FEEDBACKRepeats. Past 95% it keeps building.DELAY LEVELLevel of the delay under the full dry signal.HIGH CUTTop-end roll-off on the repeats (each repeat gets darker).DELAYMACHINEMXRTC 2290MXR: the late-70s digital delay (narrow bandwidth, companded and slightly grainy). TC 2290: the late-80s/90s full-range digital delay.MOD DEPTHModulation of the delay time (the 2290's modulation section): a chorus-like shimmer on the repeats.MOD RATEModulation rate.OUTPUTOutput level.DIGITAL DELAY
333?
Run Like Hell (380 ms)
Wall Lead (MXR)
Animals Ambience (MXR)
Momentary Lapse Lead (TC)
Division Bell Lead (TC)
Marooned Ambience (TC)
Slapback$min > max, or either was NaN. min = 

*/
fn panel() -> Panel {
    Panel {
        title: "DIGITAL DELAY",
        subtitle: ("AFTER THE MXR DDL AND TC ELECTRONIC 2290", "GILMOUR'S DELAYS, 1977 - 1994"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![
            Item::Knob { p: TIME, x: 90.0, y: 150.0, r: 36.0, label: "DELAY TIME", tip: "Delay time (ms). Gilmour usually set it in time with the song, often dotted eighths.", active: None },
            Item::Knob { p: REPEATS, x: 230.0, y: 150.0, r: 30.0, label: "FEEDBACK", tip: "Repeats. Past 95% it keeps building.", active: None },
            Item::Knob { p: LEVEL, x: 350.0, y: 150.0, r: 30.0, label: "DELAY LEVEL", tip: "Level of the delay under the full dry signal.", active: None },
            Item::Knob { p: HIGHCUT, x: 470.0, y: 150.0, r: 24.0, label: "HIGH CUT", tip: "Top-end roll-off on the repeats (each repeat gets darker).", active: None },
            Item::Lamp { m: 0, x: 570.0, y: 150.0, label: "DELAY", lo: 0.01, hi: 0.5 },
            Item::Divider { y: 230.0 },
            Item::Text { x: 140.0, y: 274.0, text: "MACHINE", size: 13.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: MACHINE, x: 140.0, y: 320.0, labels: ["MXR", "TC 2290"], tip: "MXR: the late-70s digital delay (narrow bandwidth, companded and slightly grainy). TC 2290: the late-80s/90s full-range digital delay." },
            Item::Knob { p: MOD_DEPTH, x: 290.0, y: 320.0, r: 24.0, label: "MOD DEPTH", tip: "Modulation of the delay time (the 2290's modulation section): a chorus-like shimmer on the repeats.", active: None },
            Item::Knob { p: MOD_RATE, x: 400.0, y: 320.0, r: 24.0, label: "MOD RATE", tip: "Modulation rate.", active: None },
            Item::Knob { p: OUTPUT, x: 520.0, y: 320.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Digital Delay", class_id: *b"PFDigiDelayPQD01", vendor: "PEQUOD Pink Floyd",
    categories: [Fx, Delay]
);
