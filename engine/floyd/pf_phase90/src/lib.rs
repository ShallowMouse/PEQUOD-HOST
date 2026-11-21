//! Phase 90: the MXR Phase 90 (script logo), David Gilmour's phaser on Shine On You Crazy Diamond and Have a Cigar: four
//! JFET-swept all-pass stages mixed with the dry signal. Script: no feedback (Gilmour's); Block: the later 24k feedback resistor.

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
pub enum Logo {
    #[name = "Script (no feedback)"]
    ScriptNoFeedback,
    #[name = "Block (feedback)"]
    BlockFeedback,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "speed"]
    pub speed: FloatParam,
    #[id = "logo"]
    pub logo: EnumParam<Logo>,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            speed: rate("Speed", 0.5, 0.08, 9.0),
            logo: EnumParam::new("Version", Logo::ScriptNoFeedback),
            output: db("Output", 0.0, -12.0, 6.0),
        }
    }
}

pub const SPEED: usize = 0;
pub const LOGO: usize = 1;
pub const OUTPUT: usize = 2;
pub const NPARAMS: usize = 3;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.speed.as_ptr(),
            self.logo.as_ptr(),
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

/// First-order all-pass with a moving corner.
#[derive(Clone, Copy, Default)]
struct Ap {
    z: f64,
}
impl Ap {
    #[inline]
    fn process(&mut self, x: f64, a: f64) -> f64 {
        let y = a * x + self.z;
        self.z = x - a * y;
        y
    }
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ap: [[Ap; 4]; 2],
    fb: [f64; 2],
    ph: f64,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, ap: [[Ap::default(); 4]; 2], fb: [0.0; 2], ph: 0.0, out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.ap = [[Ap::default(); 4]; 2];
        self.fb = [0.0; 2];
        self.ph = 0.0;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let rate = self.v[SPEED] as f64;
        let fbk = if self.v[LOGO] > 0.5 { 0.45 } else { 0.0 };
        let nch = chans.len().min(2);
        let mut m = 0.0;
        for k in 0..n {
            self.ph = (self.ph + rate / self.fs).fract();
            // the LFO: a triangle bent by the JFETs' square-law into an exponential sweep, about 200 Hz to 2.5 kHz
            let tri = 1.0 - 2.0 * (self.ph - 0.5).abs();
            m = tri;
            let f = 200.0 * (12.5f64).powf(tri * tri * 0.4 + tri * 0.6);
            let t = (std::f64::consts::PI * f / self.fs).tan();
            let a = (t - 1.0) / (t + 1.0);
            let o = self.out.next();
            for c in 0..nch {
                let x = chans[c][k] as f64;
                // the JFET stages aren't quite linear
                let x = x + 0.015 * x * x;
                let mut w = x + fbk * self.fb[c];
                for s in self.ap[c].iter_mut() {
                    w = s.process(w, a);
                }
                self.fb[c] = w;
                chans[c][k] = ((x + w) * 0.67 * o) as f32;
            }
        }
        meters[0] = m as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Shine On Riff (slow)", values: &[(SPEED, 0.45)] },
        Preset { name: "Have a Cigar", values: &[(SPEED, 1.1)] },
        Preset { name: "Very Slow Sweep", values: &[(SPEED, 0.12)] },
        Preset { name: "Fast Warble", values: &[(SPEED, 5.5)] },
        Preset { name: "Block Logo Swoosh", values: &[(SPEED, 0.6), (LOGO, 1.0)] },
    ]
}

/*
('knob', 319.5, 159.5, 53.7)
('divider', 284)
('knob', 419.5, 335.5, 22.0)

crate: ['floyd/pf_phase90', 'library/panic_abort']
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
/root/.cargo/git/checkouts/nih-plug-d4210f82e920295f/de42101/nih_plug_egui/src/editor.rs
AFTER THE MXR PHASE 90, SCRIPT LOGO4 JFET STAGES  
  ONE KNOBSPEEDSweep rate: about 0.08 to 9 Hz.SWEEPSCRIPTBLOCKSCRIPT: the 1970s circuit, no feedback (Gilmour's). BLOCK: later units with a 24k feedback resistor: more resonant.OUTPUTOutput level (the pedal itself has none).PHASE 90
Shine On Riff (slow)
Have a Cigar
Very Slow Sweep
Fast Warble
Block Logo Swoosh
SpeedVersionOutputassertion failed: unsafe { QueryPerformanceCounter(&mut ticks).as_bool() }/root/.cargo/git/checkouts/nih-plug-d4210f82e920295f/de42101/src/event_loop/windows.rs
nih-event-loop-
NIH-plug event loop

*/
fn panel() -> Panel {
    Panel {
        title: "PHASE 90",
        subtitle: ("AFTER THE MXR PHASE 90, SCRIPT LOGO", "4 JFET STAGES  ·  ONE KNOB"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![
            Item::Knob { p: SPEED, x: 320.0, y: 160.0, r: 54.0, label: "SPEED", tip: "Sweep rate: about 0.08 to 9 Hz.", active: None },
            Item::Lamp { m: 0, x: 460.0, y: 160.0, label: "SWEEP", lo: 0.0, hi: 1.0 },
            Item::Divider { y: 284.0 },
            Item::Switch { p: LOGO, x: 220.0, y: 336.0, labels: ["SCRIPT", "BLOCK"], tip: "SCRIPT: the 1970s circuit, no feedback (Gilmour's). BLOCK: later units with a 24k feedback resistor: more resonant." },
            Item::Knob { p: OUTPUT, x: 420.0, y: 336.0, r: 22.0, label: "OUTPUT", tip: "Output level (the pedal itself has none).", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Phase 90", class_id: *b"PFPhase90PQD0001", vendor: "PEQUOD Pink Floyd",
    categories: [Fx, Modulation]
);
