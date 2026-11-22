//! Neotek Channel: a channel of the Neotek consoles at Electrical Audio: the Series II (Studio B, thicker, a softer edge
//! when pushed) or the Elite (Studio A, straighter): preamp gain, high-pass, a four-band EQ and the fader.

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
pub enum Series {
    #[name = "Series II (Studio B)"]
    SeriesIiStudioB,
    #[name = "Elite (Studio A)"]
    EliteStudioA,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "series"]
    pub series: EnumParam<Series>,
    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "hpf"]
    pub hpf: FloatParam,
    #[id = "low"]
    pub low: FloatParam,
    #[id = "lmf"]
    pub lmf: FloatParam,
    #[id = "lm"]
    pub lm: FloatParam,
    #[id = "hmf"]
    pub hmf: FloatParam,
    #[id = "hm"]
    pub hm: FloatParam,
    #[id = "high"]
    pub high: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            series: EnumParam::new("Series", Series::SeriesIiStudioB),
            gain: db("Gain", 0.0, -12.0, 24.0),
            hpf: hz("High-Pass", 20.0, 20.0, 300.0),
            low: db("Low", 0.0, -15.0, 15.0),
            lmf: hz("Low-Mid Freq", 400.0, 100.0, 2000.0),
            lm: db("Low-Mid", 0.0, -15.0, 15.0),
            hmf: hz("High-Mid Freq", 3000.0, 800.0, 10000.0),
            hm: db("High-Mid", 0.0, -15.0, 15.0),
            high: db("High", 0.0, -15.0, 15.0),
            output: db("Fader", 0.0, -24.0, 12.0),
        }
    }
}

pub const SERIES: usize = 0;
pub const GAIN: usize = 1;
pub const HPF: usize = 2;
pub const LOW: usize = 3;
pub const LMF: usize = 4;
pub const LM: usize = 5;
pub const HMF: usize = 6;
pub const HM: usize = 7;
pub const HIGH: usize = 8;
pub const OUTPUT: usize = 9;
pub const NPARAMS: usize = 10;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.series.as_ptr(),
            self.gain.as_ptr(),
            self.hpf.as_ptr(),
            self.low.as_ptr(),
            self.lmf.as_ptr(),
            self.lm.as_ptr(),
            self.hmf.as_ptr(),
            self.hm.as_ptr(),
            self.high.as_ptr(),
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
    eq: [[Biquad; 6]; 2],
    dc: [Biquad; 2],
    key: Vec<i32>,
    gain: Smooth,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let v = self.v;
        let ii = v[SERIES] < 0.5;
        for e in self.eq.iter_mut() {
            if v[HPF] > 20.5 {
                e[0].set_highpass(fs, v[HPF] as f64, 0.707);
            } else {
                e[0] = Biquad::identity();
            }
            e[1].set_low_shelf(fs, 100.0, 0.7, v[LOW] as f64);
            e[2].set_peak(fs, v[LMF] as f64, 0.8, v[LM] as f64);
            e[3].set_peak(fs, v[HMF] as f64, 0.8, v[HM] as f64);
            e[4].set_high_shelf(fs, 10000.0f64.min(fs * 0.4), 0.7, v[HIGH] as f64);
            // the Series II's transformers and slower op-amps: a touch of low-end weight and a softer top
            if ii {
                e[5].set_low_shelf(fs, 60.0, 0.7, 0.6);
            } else {
                e[5] = Biquad::identity();
            }
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut dc = Biquad::identity();
        dc.set_highpass(fs, 8.0, 0.7);
        let mut d = Self { fs, v, eq: [[Biquad::identity(); 6]; 2], dc: [dc; 2], key: vec![], gain: Smooth::new(fs, 20.0, 1.0), out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.eq[c].iter_mut().for_each(|b| b.reset());
            self.dc[c].reset();
        }
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k: Vec<i32> = v.iter().map(|x| (x * 10.0).round() as i32).collect();
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.gain.set(db_to_gain(v[GAIN] as f64));
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(2);
        let ii = self.v[SERIES] < 0.5;
        // headroom of the preamp, referred to full scale
        let head = if ii { 3.0 } else { 5.0 };
        let mut peak = 0.0f64;
        for k in 0..n {
            let g = self.gain.next();
            let o = self.out.next();
            for c in 0..nch {
                let x = chans[c][k] as f64 * g;
                let s = if ii { head * tube(x / head, 0.1) } else { head * (x / head).tanh() };
                let mut y = self.dc[c].process(s);
                for b in self.eq[c].iter_mut() {
                    y = b.process(y);
                }
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
        Preset { name: "Flat (just the channel)", values: &[] },
        Preset { name: "Elite, Flat", values: &[(SERIES, 1.0)] },
        Preset { name: "Guitar: Tame the Fizz", values: &[(HPF, 70.0), (HMF, 4500.0), (HM, -3.0)] },
        Preset { name: "Bass: Clean Lows", values: &[(HPF, 30.0), (LMF, 300.0), (LM, -2.0), (HMF, 1500.0), (HM, 2.0)] },
        Preset { name: "Snare Crack", values: &[(LMF, 500.0), (LM, -2.0), (HMF, 5000.0), (HM, 3.0)] },
        Preset { name: "Hot Series II Gain", values: &[(GAIN, 18.0), (OUTPUT, -16.0)] },
    ]
}

/*
('knob', 63.5, 149.5, 28.0)
('knob', 149.5, 149.5, 19.5)
('knob', 219.5, 149.5, 19.5)
('knob', 289.5, 149.5, 17.1)
('knob', 349.5, 149.5, 19.5)
('knob', 419.5, 149.5, 17.1)
('knob', 479.5, 149.5, 19.5)
('knob', 559.5, 149.5, 19.5)
('divider', 230)
('meter', 232, 252, 176, 100)
('knob', 519.5, 315.5, 25.6)
('selector?', 119.5, 319.5, 36, 36)

crate: ['albini/pa_neotek', 'library/panic_abort']
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/std/src/sync/once.rs
vintage/src/ui.rs
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
AFTER THE NEOTEK SERIES II AND ELITEELECTRICAL AUDIO'S CONSOLESGAINPreamp gain. Push the Series II for its softer, thicker edge.HIGH-PASSHigh-pass filter. 20 Hz = off.LOWLow shelf at 100 Hz.LM FREQLow-mid frequency.LOW-MIDLow-mid boost/cut.HM FREQHigh-mid frequency.HIGH-MIDHigh-mid boost/cut.HIGHHigh shelf at 10 kHz.IIELITE
SERIESSeries II (Electrical Audio's Studio B desk: thicker) or Elite (Studio A: straighter).CHANNELFADERChannel level.NEOTEK CHANNELFlat (just the channel)
Elite, Flat
Guitar: Tame the Fizz
Bass: Clean Lows
Snare Crack
Hot Series II Gain
albini/pa_neotek/src/lib.rs
bg-worker

*/
fn panel() -> Panel {
    Panel {
        title: "NEOTEK CHANNEL",
        subtitle: ("AFTER THE NEOTEK SERIES II AND ELITE", "ELECTRICAL AUDIO'S CONSOLES"),
        accent: ui::STEEL,
        size: (W, H),
        items: vec![
            Item::Knob { p: GAIN, x: 64.0, y: 150.0, r: 28.0, label: "GAIN", tip: "Preamp gain. Push the Series II for its softer, thicker edge.", active: None },
            Item::Knob { p: HPF, x: 150.0, y: 150.0, r: 20.0, label: "HIGH-PASS", tip: "High-pass filter. 20 Hz = off.", active: None },
            Item::Knob { p: LOW, x: 220.0, y: 150.0, r: 20.0, label: "LOW", tip: "Low shelf at 100 Hz.", active: None },
            Item::Knob { p: LMF, x: 290.0, y: 150.0, r: 18.0, label: "LM FREQ", tip: "Low-mid frequency.", active: None },
            Item::Knob { p: LM, x: 350.0, y: 150.0, r: 20.0, label: "LOW-MID", tip: "Low-mid boost/cut.", active: None },
            Item::Knob { p: HMF, x: 420.0, y: 150.0, r: 18.0, label: "HM FREQ", tip: "High-mid frequency.", active: None },
            Item::Knob { p: HM, x: 480.0, y: 150.0, r: 20.0, label: "HIGH-MID", tip: "High-mid boost/cut.", active: None },
            Item::Knob { p: HIGH, x: 560.0, y: 150.0, r: 20.0, label: "HIGH", tip: "High shelf at 10 kHz.", active: None },
            Item::Divider { y: 230.0 },
            Item::Selector { p: SERIES, x: 120.0, y: 320.0, label: "SERIES", positions: &["II", "ELITE"], tip: "Series II (Electrical Audio's Studio B desk: thicker) or Elite (Studio A: straighter)." },
            Item::Meter { m: 0, x: 232.0, y: 252.0, w: 176.0, h: 110.0, kind: MeterKind::Vu, caption: "CHANNEL" },
            Item::Knob { p: OUTPUT, x: 520.0, y: 316.0, r: 26.0, label: "FADER", tip: "Channel level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Neotek Channel", class_id: *b"PANeotekChPQD001", vendor: "PEQUOD Albini",
    categories: [Fx, Eq]
);
