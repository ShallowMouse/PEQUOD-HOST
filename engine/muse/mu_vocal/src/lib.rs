//! Bellamy Vocal: Matt Bellamy's vocal chain of the Absolution era: an RCA 77-DX ribbon (Endlessly), a Shure SM7
//! (The Small Print) or a large condenser, a valve preamp, a fast FET-style compressor, presence, a stacked
//! falsetto choir (his layered, Queen-style backing vocals) and a plate.

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
pub const H: u32 = 432;

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum Mic {
    #[name = "RCA 77-DX ribbon"]
    Rca,
    #[name = "Shure SM7"]
    Sm7,
    #[name = "Large condenser"]
    Condenser,
}

fn knob01(name: &str, def: f32) -> FloatParam {
    pct(name, def)
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "mic"]
    pub mic: EnumParam<Mic>,
    #[id = "proximity"]
    pub proximity: FloatParam,
    #[id = "lowcut"]
    pub lowcut: FloatParam,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "comp"]
    pub comp: FloatParam,
    #[id = "presence"]
    pub presence: FloatParam,
    #[id = "choir"]
    pub choir: FloatParam,
    #[id = "plate"]
    pub plate: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            mic: EnumParam::new("Mic", Mic::Sm7),
            proximity: knob01("Proximity", 0.4),
            lowcut: FloatParam::new("Low Cut", 80.0, FloatRange::Skewed { min: 20.0, max: 400.0, factor: FloatRange::skew_factor(-1.0) }).with_unit(" Hz").with_value_to_string(formatters::v2s_f32_rounded(0)),
            drive: knob01("Valve Drive", 0.3),
            comp: knob01("Compression", 0.55),
            presence: FloatParam::new("Presence", 3.0, FloatRange::Linear { min: -6.0, max: 9.0 }).with_unit(" dB").with_value_to_string(formatters::v2s_f32_rounded(1)),
            choir: knob01("Choir", 0.0),
            plate: knob01("Plate", 0.2),
            output: db("Output", 0.0, -12.0, 12.0),
        }
    }
}

pub const MIC: usize = 0;
pub const PROXIMITY: usize = 1;
pub const LOWCUT: usize = 2;
pub const DRIVE: usize = 3;
pub const COMP: usize = 4;
pub const PRESENCE: usize = 5;
pub const CHOIR: usize = 6;
pub const PLATE: usize = 7;
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
            self.mic.as_ptr(),
            self.proximity.as_ptr(),
            self.lowcut.as_ptr(),
            self.drive.as_ptr(),
            self.comp.as_ptr(),
            self.presence.as_ptr(),
            self.choir.as_ptr(),
            self.plate.as_ptr(),
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

use vintage::fx::PitchShift;
use vintage::verb::Fdn;

/// The choir: four voices, each a few cents off, a little late, panned across.
const VOICES: [(f64, f64, f64); 4] = [(-9.0, 0.019, -0.8), (7.0, 0.027, 0.8), (-4.0, 0.035, -0.4), (12.0, 0.042, 0.4)];

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    lc: [Biquad; 2],
    mic: [Biquad; 3],
    pres: Biquad,
    env: f64,
    gr: f64,
    ps: Vec<PitchShift>,
    dl: Vec<DelayLine>,
    fdn: Fdn,
    pre_hp: Biquad,
    key: Vec<i32>,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let lc = self.v[LOWCUT] as f64;
        self.lc[0].set_highpass(fs, lc, 0.54);
        self.lc[1].set_highpass(fs, lc, 1.31);
        let prox = self.v[PROXIMITY] as f64;
        match self.v[MIC].round() as i32 {
            0 => {
                // ribbon: figure-8, big proximity bass, soft top that falls from about 8 kHz
                self.mic[0].set_low_shelf(fs, 180.0, 0.7, 2.0 + 10.0 * prox);
                self.mic[1].set_lowpass(fs, 9000.0, 0.6);
                self.mic[2].set_peak(fs, 2500.0, 0.8, -1.5);
            }
            1 => {
                // SM7: cardioid dynamic, flat-ish with its presence lift and a gentle top
                self.mic[0].set_low_shelf(fs, 150.0, 0.7, 1.0 + 6.0 * prox);
                self.mic[1].set_lowpass(fs, 14000.0, 0.7);
                self.mic[2].set_peak(fs, 4500.0, 0.9, 3.0);
            }
            _ => {
                // large condenser: cardioid, an airy lift around 10-12 kHz
                self.mic[0].set_low_shelf(fs, 140.0, 0.7, 0.5 + 5.0 * prox);
                self.mic[1].set_high_shelf(fs, 9000.0, 0.7, 3.0);
                self.mic[2].set_peak(fs, 5000.0, 1.0, 1.0);
            }
        }
        self.pres.set_peak(fs, 3500.0, 0.8, self.v[PRESENCE] as f64);
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut fdn = Fdn::new(fs, 1.6);
        // a plate: dense, bright, about 2.2 s
        fdn.configure(1.2, [2.0, 2.3, 1.6], 6000.0);
        let mut pre_hp = Biquad::identity();
        pre_hp.set_highpass(fs, 250.0, 0.7);
        let mut d = Self {
            fs,
            v,
            lc: [Biquad::identity(); 2],
            mic: [Biquad::identity(); 3],
            pres: Biquad::identity(),
            env: 0.0,
            gr: 1.0,
            ps: (0..4).map(|_| PitchShift::new(fs, 35.0)).collect(),
            dl: (0..4).map(|_| DelayLine::new((fs * 0.06) as usize)).collect(),
            fdn,
            pre_hp,
            key: vec![],
            out: Smooth::new(fs, 20.0, 1.0),
        };
        d.design();
        d
    }
    fn reset(&mut self) {
        for b in self.lc.iter_mut().chain(self.mic.iter_mut()) {
            b.reset();
        }
        self.pres.reset();
        self.env = 0.0;
        self.gr = 1.0;
        self.ps.iter_mut().for_each(|p| p.reset());
        self.dl.iter_mut().for_each(|p| p.reset());
        self.fdn.reset();
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k: Vec<i32> = [v[MIC], v[PROXIMITY], v[LOWCUT], v[PRESENCE]].iter().map(|x| (x * 100.0) as i32).collect();
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().max(1);
        let fs = self.fs;
        let drive = self.v[DRIVE] as f64;
        let comp = self.v[COMP] as f64;
        let choir = self.v[CHOIR] as f64;
        let plate = self.v[PLATE] as f64;
        // FET-style: fast attack, quick release, ratio and threshold from one knob
        let thr = db_to_gain(-6.0 - 24.0 * comp);
        let ratio = 2.0 + 10.0 * comp;
        let att = coeff(fs, 0.3);
        let rel = coeff(fs, 60.0);
        let mut most = 0.0f64;
        for k in 0..n {
            let x = chans.iter().map(|c| c[k] as f64).sum::<f64>() / nch as f64;
            let mut s = cascade(&mut self.lc, x);
            s = cascade(&mut self.mic, s);
            // valve preamp
            let g = 1.0 + 6.0 * drive;
            s = tube(s * g, 0.15 + 0.2 * drive) / g.sqrt();
            // compressor
            let a = s.abs();
            let c = if a > self.env { att } else { rel };
            self.env = a + c * (self.env - a);
            let want = if self.env > thr { (thr / self.env).powf(1.0 - 1.0 / ratio) } else { 1.0 };
            self.gr = want + 0.3 * (self.gr - want);
            s *= self.gr * (1.0 + 2.0 * comp);
            most = most.max(-gain_to_db(self.gr));
            s = self.pres.process(s);
            // choir
            let (mut l, mut r) = (s, s);
            if choir > 0.0 {
                for (i, &(cents, dly, pan)) in VOICES.iter().enumerate() {
                    self.dl[i].write(s);
                    let d = self.dl[i].read(dly * fs);
                    let w = self.ps[i].process(d, 2f64.powf(cents / 1200.0)) * choir * 0.5;
                    l += w * (1.0 - pan) * 0.5;
                    r += w * (1.0 + pan) * 0.5;
                }
            }
            // plate
            if plate > 0.0 {
                let p = self.pre_hp.process(s);
                let dif = self.fdn.diffuse(p, 3);
                let (pl, pr) = self.fdn.process(dif);
                l += pl * plate;
                r += pr * plate;
            }
            let o = self.out.next();
            if chans.len() >= 2 {
                chans[0][k] = (l * o) as f32;
                chans[1][k] = (r * o) as f32;
            } else if let Some(c) = chans.first_mut() {
                c[k] = (0.5 * (l + r) * o) as f32;
            }
        }
        meters[0] = most as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Absolution Lead (SM7)", values: &[] },
        Preset { name: "Endlessly (77-DX)", values: &[(MIC, 0.0), (PROXIMITY, 0.6), (COMP, 0.4), (PLATE, 0.3)] },
        Preset { name: "Falsetto Choir", values: &[(MIC, 2.0), (CHOIR, 0.8), (PLATE, 0.35), (PRESENCE, 4.0)] },
        Preset { name: "Big Chorus Stack", values: &[(CHOIR, 0.55), (COMP, 0.7), (DRIVE, 0.45)] },
        Preset { name: "Close and Dry", values: &[(PLATE, 0.0), (PROXIMITY, 0.7), (COMP, 0.5)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "BELLAMY VOCAL",
        subtitle: ("MIC · VALVE PRE · FET COMPRESSOR · CHOIR · PLATE", "MATT BELLAMY'S VOCAL CHAIN, ABSOLUTION ERA"),
        accent: ui::OXBLOOD,
        size: (W, H),
        items: vec![
            Item::Selector { p: MIC, x: 80.0, y: 150.0, label: "MIC", positions: &["77-DX", "SM7", "COND."], tip: "RCA 77-DX ribbon (used on Endlessly), Shure SM7 (The Small Print) or a large condenser." },
            Item::Knob { p: PROXIMITY, x: 200.0, y: 150.0, r: 24.0, label: "PROXIMITY", tip: "How close to the mic: more bass build-up close in (strongest on the ribbon).", active: None },
            Item::Knob { p: DRIVE, x: 310.0, y: 150.0, r: 24.0, label: "VALVE DRIVE", tip: "How hard the valve preamp is pushed.", active: None },
            Item::Knob { p: COMP, x: 420.0, y: 150.0, r: 24.0, label: "COMPRESSION", tip: "A fast FET-style compressor: threshold and ratio together.", active: None },
            Item::Meter { m: 0, x: 480.0, y: 100.0, w: 140.0, h: 100.0, kind: MeterKind::GainReduction, caption: "GAIN REDUCTION" },
            Item::Divider { y: 250.0 },
            Item::Knob { p: LOWCUT, x: 80.0, y: 330.0, r: 20.0, label: "LOW CUT", tip: "High-pass before the mic voicing.", active: None },
            Item::Knob { p: PRESENCE, x: 190.0, y: 330.0, r: 22.0, label: "PRESENCE", tip: "Presence lift around 3.5 kHz.", active: None },
            Item::Knob { p: CHOIR, x: 310.0, y: 330.0, r: 26.0, label: "CHOIR", tip: "Stacked falsetto layers: four detuned, slightly late voices spread across the stereo field.", active: None },
            Item::Knob { p: PLATE, x: 430.0, y: 330.0, r: 24.0, label: "PLATE", tip: "Plate reverb level.", active: None },
            Item::Knob { p: OUTPUT, x: 550.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Bellamy Vocal", class_id: *b"MuseBellamyVox01", vendor: "PEQUOD Muse",
    categories: [Fx, Dynamics]
);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presets_run_clean() {
        for pr in presets() {
            let mut v = defaults();
            for (i, x) in pr.values {
                v[*i] = *x;
            }
            let mut d = Dsp::new(48000.0, 2);
            d.set_params(&v);
            let mut l: Vec<f32> = (0..48000).map(|k| (0.3 * (k as f64 * 0.03).sin()) as f32).collect();
            let mut r = l.clone();
            let mut m = [0.0f32];
            d.process(&mut [&mut l[..], &mut r[..]], &mut m);
            assert!(l.iter().chain(r.iter()).all(|x| x.is_finite() && x.abs() < 8.0), "{}", pr.name);
        }
    }
}
