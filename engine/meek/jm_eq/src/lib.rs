//! Black Box EQ: Joe Meek's program equaliser, after the Pultec EQP-1A: low boost and attenuation on the same frequency
//! (the bump-and-dip trick), a peaking high boost with bandwidth, a high attenuation shelf, and the valve make-up stage.

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
pub enum LowF {
    #[name = "20 Hz"]
    V20Hz,
    #[name = "30 Hz"]
    V30Hz,
    #[name = "60 Hz"]
    V60Hz,
    #[name = "100 Hz"]
    V100Hz,
}

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum HighF {
    #[name = "3 kHz"]
    V3Khz,
    #[name = "4 kHz"]
    V4Khz,
    #[name = "5 kHz"]
    V5Khz,
    #[name = "8 kHz"]
    V8Khz,
    #[name = "10 kHz"]
    V10Khz,
    #[name = "12 kHz"]
    V12Khz,
    #[name = "16 kHz"]
    V16Khz,
}

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum AttenF {
    #[name = "5 kHz"]
    V5Khz,
    #[name = "10 kHz"]
    V10Khz,
    #[name = "20 kHz"]
    V20Khz,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "low_f"]
    pub low_f: EnumParam<LowF>,
    #[id = "low_boost"]
    pub low_boost: FloatParam,
    #[id = "low_atten"]
    pub low_atten: FloatParam,
    #[id = "high_f"]
    pub high_f: EnumParam<HighF>,
    #[id = "bandwidth"]
    pub bandwidth: FloatParam,
    #[id = "high_boost"]
    pub high_boost: FloatParam,
    #[id = "atten_f"]
    pub atten_f: EnumParam<AttenF>,
    #[id = "high_atten"]
    pub high_atten: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            low_f: EnumParam::new("Low Frequency", LowF::V60Hz),
            low_boost: FloatParam::new("Low Boost", 0.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            low_atten: FloatParam::new("Low Atten", 0.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            high_f: EnumParam::new("High Frequency", HighF::V10Khz),
            bandwidth: FloatParam::new("Bandwidth", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            high_boost: FloatParam::new("High Boost", 0.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            atten_f: EnumParam::new("Atten Select", AttenF::V20Khz),
            high_atten: FloatParam::new("High Atten", 0.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const LOW_F: usize = 0;
pub const LOW_BOOST: usize = 1;
pub const LOW_ATTEN: usize = 2;
pub const HIGH_F: usize = 3;
pub const BANDWIDTH: usize = 4;
pub const HIGH_BOOST: usize = 5;
pub const ATTEN_F: usize = 6;
pub const HIGH_ATTEN: usize = 7;
pub const OUTPUT: usize = 8;
pub const NPARAMS: usize = 9;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.low_f.as_ptr(),
            self.low_boost.as_ptr(),
            self.low_atten.as_ptr(),
            self.high_f.as_ptr(),
            self.bandwidth.as_ptr(),
            self.high_boost.as_ptr(),
            self.atten_f.as_ptr(),
            self.high_atten.as_ptr(),
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

const LOWS: [f64; 4] = [20.0, 30.0, 60.0, 100.0];
const HIGHS: [f64; 7] = [3000.0, 4000.0, 5000.0, 8000.0, 10000.0, 12000.0, 16000.0];
const ATTENS: [f64; 3] = [5000.0, 10000.0, 20000.0];

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    eq: [[Biquad; 4]; 2],
    hp: [Biquad; 2],
    key: Vec<i32>,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let v = &self.v;
        let lf = LOWS[(v[LOW_F].round() as usize).min(3)];
        let hf = HIGHS[(v[HIGH_F].round() as usize).min(6)].min(fs * 0.45);
        let af = ATTENS[(v[ATTEN_F].round() as usize).min(2)].min(fs * 0.45);
        let lb = v[LOW_BOOST] as f64 / 10.0 * 13.5;
        let la = v[LOW_ATTEN] as f64 / 10.0 * 17.5;
        let hb = v[HIGH_BOOST] as f64 / 10.0 * 16.0;
        let ha = v[HIGH_ATTEN] as f64 / 10.0 * 16.0;
        let q = 2.5 - 2.0 * v[BANDWIDTH] as f64 / 10.0;
        for e in self.eq.iter_mut() {
            // the boost shelf sits a little lower than the cut, hence the bump below the dip
            e[0].set_low_shelf(fs, lf * 1.6, 0.55, lb);
            e[1].set_low_shelf(fs, lf * 3.0, 0.5, -la);
            e[2].set_peak(fs, hf, q, hb);
            e[3].set_high_shelf(fs, af, 0.6, -ha);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut hp = Biquad::identity();
        hp.set_highpass(fs, 12.0, 0.7);
        let mut d = Self { fs, v, eq: [[Biquad::identity(); 4]; 2], hp: [hp; 2], key: vec![], out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.eq[c].iter_mut().for_each(|b| b.reset());
            self.hp[c].reset();
        }
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k: Vec<i32> = v[..NPARAMS - 1].iter().map(|x| (x * 100.0).round() as i32).collect();
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(2);
        let mut peak = 0.0f64;
        for k in 0..n {
            let o = self.out.next();
            for c in 0..nch {
                let mut x = chans[c][k] as f64;
                for b in self.eq[c].iter_mut() {
                    x = b.process(x);
                }
                // the valve make-up stage: a little second harmonic, more when driven
                let y = self.hp[c].process(tube(x, 0.05));
                peak = peak.max(y.abs());
                chans[c][k] = (y * o) as f32;
            }
        }
        meters[0] = peak as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Flat (through the valves)", values: &[] },
        Preset { name: "The Trick: Boost + Atten at 60", values: &[(LOW_BOOST, 5.0), (LOW_ATTEN, 4.0)] },
        Preset { name: "Kick / Bass Weight", values: &[(LOW_BOOST, 6.0), (LOW_ATTEN, 2.0)] },
        Preset { name: "Vocal Air (12 kHz, broad)", values: &[(HIGH_F, 5.0), (BANDWIDTH, 8.0), (HIGH_BOOST, 4.0), (HIGH_ATTEN, 1.0)] },
        Preset { name: "Telstar Bright (5 kHz)", values: &[(HIGH_F, 2.0), (BANDWIDTH, 4.0), (HIGH_BOOST, 6.0)] },
        Preset { name: "Whole Mix Polish", values: &[(LOW_F, 1.0), (LOW_BOOST, 3.0), (LOW_ATTEN, 2.0), (BANDWIDTH, 7.0), (HIGH_BOOST, 3.0)] },
        Preset { name: "Dark (atten 5 kHz)", values: &[(ATTEN_F, 0.0), (HIGH_ATTEN, 6.0)] },
    ]
}

/*
('knob', 380.5, 157.5, 9.8)
('selector?', 69.5, 159.5, 36, 36)
('knob', 169.5, 159.5, 25.6)
('knob', 259.5, 159.5, 25.6)
('knob', 479.5, 159.5, 22.0)
('knob', 569.5, 159.5, 25.6)
('divider', 250)
('knob', 269.5, 325.5, 25.6)
('knob', 469.5, 325.5, 25.6)
('selector?', 139.5, 329.5, 36, 36)

crate: ['library/panic_abort', 'meek/jm_pultec']
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/std/src/sync/once.rs
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
AFTER THE PULTEC EQP-1AJOE MEEK'S PROGRAM EQUALISER203060100
LOW FREQLow frequency (Hz) for both boost and atten.BOOSTLow boost.ATTENLow attenuation. Turn both up for the bump-and-dip.3458101216
HIGH FREQHigh boost frequency (kHz).BANDWIDTHHigh boost bandwidth: sharp to broad.High boost.
ATTEN SELHigh attenuation frequency (kHz).HIGH ATTENHigh attenuation (shelf).OUTPUTOutput level.BLACK BOX EQFlat (through the valves)
The Trick: Boost + Atten at 60
Kick / Bass Weight
Vocal Air (12 kHz, broad)
Telstar Bright (5 kHz)
Whole Mix Polish
Dark (atten 5 kHz)

*/
fn panel() -> Panel {
    Panel {
        title: "BLACK BOX EQ",
        subtitle: ("AFTER THE PULTEC EQP-1A", "JOE MEEK'S PROGRAM EQUALISER"),
        accent: ui::COBALT,
        size: (W, H),
        items: vec![
            Item::Selector { p: LOW_F, x: 70.0, y: 160.0, label: "LOW FREQ", positions: &["20", "30", "60", "100"], tip: "Low frequency (Hz) for both boost and atten." },
            Item::Knob { p: LOW_BOOST, x: 170.0, y: 160.0, r: 26.0, label: "BOOST", tip: "Low boost.", active: None },
            Item::Knob { p: LOW_ATTEN, x: 260.0, y: 160.0, r: 26.0, label: "ATTEN", tip: "Low attenuation. Turn both up for the bump-and-dip.", active: None },
            Item::Selector { p: HIGH_F, x: 380.0, y: 160.0, label: "HIGH FREQ", positions: &["3", "4", "5", "8", "10", "12", "16"], tip: "High boost frequency (kHz)." },
            Item::Knob { p: BANDWIDTH, x: 480.0, y: 160.0, r: 22.0, label: "BANDWIDTH", tip: "High boost bandwidth: sharp to broad.", active: None },
            Item::Knob { p: HIGH_BOOST, x: 570.0, y: 160.0, r: 26.0, label: "BOOST", tip: "High boost.", active: None },
            Item::Divider { y: 250.0 },
            Item::Selector { p: ATTEN_F, x: 140.0, y: 330.0, label: "ATTEN SEL", positions: &["5", "10", "20"], tip: "High attenuation frequency (kHz)." },
            Item::Knob { p: HIGH_ATTEN, x: 270.0, y: 326.0, r: 26.0, label: "HIGH ATTEN", tip: "High attenuation (shelf).", active: None },
            Item::Knob { p: OUTPUT, x: 470.0, y: 326.0, r: 26.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Black Box EQ", class_id: *b"JMBlackBoxEQPQD1", vendor: "PEQUOD Joe Meek",
    categories: [Fx, Eq]
);
