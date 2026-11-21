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

const OS: usize = 4;

#[derive(Clone, Default)]
struct St {
    hp: Biquad,
    bass: Biquad,
    treble: Biquad,
    mid: Biquad,
    lp: Biquad,
    dc: Biquad,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    st: [St; 2],
    os: [Oversampler; 2],
    key: [i32; 2],
    inp: Smooth,
    drive: Smooth,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let ofs = self.fs * OS as f64;
        let b = (self.v[BASS] as f64 - 5.0) / 5.0 * 15.0;
        let t = (self.v[TREBLE] as f64 - 5.0) / 5.0 * 15.0;
        for st in self.st.iter_mut() {
            st.hp.set_highpass(ofs, 35.0, 0.6);
            st.bass.set_low_shelf(ofs, 200.0, 0.6, b);
            st.treble.set_high_shelf(ofs, 2000.0, 0.6, t);
            // the clipping stage's mid hump (the "fat" in the overdrive)
            st.mid.set_peak(ofs, 700.0, 0.6, 2.5);
            st.lp.set_lowpass(ofs, 9000.0, 0.7);
            st.dc.set_highpass(ofs, 20.0, 0.7);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, st: Default::default(), os: [Oversampler::new(), Oversampler::new()], key: [-1; 2], inp: Smooth::new(fs, 20.0, 1.0), drive: Smooth::new(fs, 20.0, 1.0), out: Smooth::new(fs, 20.0, 0.25) };
        d.design();
        d
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.st[c] = St::default();
            self.os[c].reset();
        }
        self.design();
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = [(v[BASS] * 10.0).round() as i32, (v[TREBLE] * 10.0).round() as i32];
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.inp.set(db_to_gain(v[LEVEL] as f64));
        let g = v[GAIN] as f64 / 10.0;
        self.drive.set(7.2 * 10f64.powf(g));
        let vol = v[VOLUME] as f64 / 10.0;
        self.out.set(vol * vol);
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(2);
        let mut peak = 0.0f64;
        for k in 0..n {
            let gi = self.inp.next();
            let dr = self.drive.next();
            let o = self.out.next();
            for c in 0..nch {
                let x = chans[c][k] as f64 * gi;
                let st = &mut self.st[c];
                let mut pk = 0.0f64;
                let y = self.os[c].process(x, |u| {
                    // first stage: about 4x, plenty of headroom on 18 V
                    let s = st.hp.process(u) * 4.0;
                    let s = s + 0.03 * s * s;
                    let s = st.treble.process(st.bass.process(s));
                    // second stage: the gain pot; clips softly, a little lopsided, at about 7 V
                    let d = st.mid.process(s * dr * 0.25);
                    pk = pk.max(d.abs() / 7.0);
                    let w = if d > 0.0 { 7.0 * (d / 7.0).tanh() } else { -6.0 * (-d / 6.0).tanh() };
                    st.dc.process(st.lp.process(w)) * 0.6
                });
                peak = peak.max(pk);
                chans[c][k] = (y * o) as f32;
            }
        }
        meters[0] = peak as f32;
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
