//! Electric Mistress: the Electro-Harmonix Electric Mistress as David Gilmour used it on Animals and The Wall: a
//! bucket-brigade flanger, swept very slowly, with Color as the feedback; Filter Matrix stops the sweep and Range sets the comb.

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
pub enum Matrix {
    #[name = "Flange"]
    Flange,
    #[name = "Filter Matrix"]
    FilterMatrix,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "rate"]
    pub rate: FloatParam,
    #[id = "range"]
    pub range: FloatParam,
    #[id = "color"]
    pub color: FloatParam,
    #[id = "matrix"]
    pub matrix: EnumParam<Matrix>,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            rate: rate("Rate", 0.15, 0.02, 4.0),
            range: pct("Range", 0.7),
            color: pct("Color", 0.45),
            matrix: EnumParam::new("Filter Matrix", Matrix::Flange),
            output: db("Output", 0.0, -12.0, 6.0),
        }
    }
}

pub const RATE: usize = 0;
pub const RANGE: usize = 1;
pub const COLOR: usize = 2;
pub const MATRIX: usize = 3;
pub const OUTPUT: usize = 4;
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
            self.rate.as_ptr(),
            self.range.as_ptr(),
            self.color.as_ptr(),
            self.matrix.as_ptr(),
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

use vintage::fx::Bbd;

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    bbd: [Bbd; 2],
    fb: [f64; 2],
    ph: f64,
    hp: [Biquad; 2],
    out: Smooth,
    pos: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mk = || {
            let mut b = Bbd::new(fs, 20.0);
            b.set_bandwidth(8000.0);
            b
        };
        let mut hp = Biquad::identity();
        hp.set_highpass(fs, 20.0, 0.7);
        Self { fs, v, bbd: [mk(), mk()], fb: [0.0; 2], ph: 0.25, hp: [hp; 2], out: Smooth::new(fs, 20.0, 1.0), pos: Smooth::new(fs, 30.0, 0.5) }
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.bbd[c].reset();
            self.hp[c].reset();
            self.fb[c] = 0.0;
        }
        self.ph = 0.25;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let rate = self.v[RATE] as f64;
        let range = self.v[RANGE] as f64;
        let fbk = 0.97 * self.v[COLOR] as f64;
        let matrix = self.v[MATRIX] > 0.5;
        let nch = chans.len().min(2);
        let ms = 0.001 * self.fs;
        let mut p = 0.0;
        for k in 0..n {
            if !matrix {
                self.ph = (self.ph + rate / self.fs).fract();
            }
            // the sweep: a triangle; the BBD clock makes the delay sweep exponentially-ish
            let target = if matrix { range } else { (1.0 - range) * 0.15 + range * (1.0 - 2.0 * (self.ph - 0.5).abs()) };
            let pos = self.pos.next_to(target);
            p = pos;
            let d = (0.4 * ms) * (9.0f64).powf(pos * 2.0).min(60.0);
            let o = self.out.next();
            for c in 0..nch {
                let x = self.hp[c].process(chans[c][k] as f64);
                let w = self.bbd[c].process(x + fbk * self.fb[c], d.min(19.0 * ms));
                self.fb[c] = w.clamp(-2.0, 2.0);
                chans[c][k] = ((x + w) * 0.85 * o) as f32;
            }
        }
        meters[0] = p as f32;
    }
}

trait NextTo {
    fn next_to(&mut self, t: f64) -> f64;
}
impl NextTo for Smooth {
    fn next_to(&mut self, t: f64) -> f64 {
        self.set(t);
        self.next()
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Animals Shimmer (very slow)", values: &[(RATE, 0.06), (RANGE, 0.75), (COLOR, 0.35)] },
        Preset { name: "The Wall Clean Sweep", values: &[(RATE, 0.12), (RANGE, 0.65)] },
        Preset { name: "Jet Flange", values: &[(RATE, 0.25), (RANGE, 1.0), (COLOR, 0.85)] },
        Preset { name: "Filter Matrix Comb", values: &[(RANGE, 0.55), (COLOR, 0.6), (MATRIX, 1.0)] },
        Preset { name: "Chorus-ish", values: &[(RATE, 0.9), (RANGE, 0.3), (COLOR, 0.1)] },
    ]
}

/*
('knob', 139.5, 159.5, 37.8)
('knob', 319.5, 159.5, 37.8)
('knob', 499.5, 159.5, 37.8)
('divider', 284)
('knob', 419.5, 335.5, 22.0)

crate: ['floyd/pf_mistress', 'library/panic_abort']
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
/root/.cargo/git/checkouts/nih-plug-d4210f82e920295f/de42101/nih_plug_egui/src/editor.rs
AFTER THE ELECTRO-HARMONIX ELECTRIC MISTRESSBUCKET-BRIGADE FLANGER  
  FILTER MATRIXRATESweep rate. Gilmour kept it very slow.RANGESweep width (or, on FILTER MATRIX, the fixed delay: where the comb's notches sit).COLORFeedback: from a gentle shimmer up to a resonant jet sweep.SWEEPFLANGEMATRIXFILTER MATRIX stops the sweep; RANGE then sets a fixed comb filter by hand.OUTPUTOutput level.ELECTRIC MISTRESS
Animals Shimmer (very slow)
ff&?
The Wall Clean Sweep
Jet Flange
Filter Matrix Comb
fff?
Chorus-ish$min > max, or either was NaN. min = 
, max = 

*/
fn panel() -> Panel {
    Panel {
        title: "ELECTRIC MISTRESS",
        subtitle: ("AFTER THE ELECTRO-HARMONIX ELECTRIC MISTRESS", "BUCKET-BRIGADE FLANGER  ·  FILTER MATRIX"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![
            Item::Knob { p: RATE, x: 140.0, y: 160.0, r: 38.0, label: "RATE", tip: "Sweep rate. Gilmour kept it very slow.", active: None },
            Item::Knob { p: RANGE, x: 320.0, y: 160.0, r: 38.0, label: "RANGE", tip: "Sweep width (or, on FILTER MATRIX, the fixed delay: where the comb's notches sit).", active: None },
            Item::Knob { p: COLOR, x: 500.0, y: 160.0, r: 38.0, label: "COLOR", tip: "Feedback: from a gentle shimmer up to a resonant jet sweep.", active: None },
            Item::Lamp { m: 0, x: 320.0, y: 248.0, label: "SWEEP", lo: 0.0, hi: 1.0 },
            Item::Divider { y: 284.0 },
            Item::Switch { p: MATRIX, x: 220.0, y: 336.0, labels: ["FLANGE", "MATRIX"], tip: "FILTER MATRIX stops the sweep; RANGE then sets a fixed comb filter by hand." },
            Item::Knob { p: OUTPUT, x: 420.0, y: 336.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Electric Mistress", class_id: *b"PFMistressPQD001", vendor: "PEQUOD Pink Floyd",
    categories: [Fx, Modulation]
);
