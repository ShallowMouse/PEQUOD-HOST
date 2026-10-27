//! Studer 169: one channel of the compact Studer 169 desk James Ford uses for stems and mixing. Transformer-balanced transistor channel with three-band EQ and switchable mid frequency. EQ frequencies are estimates.

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


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum MidF { #[name = "350 Hz"] F350, #[name = "700 Hz"] F700, #[name = "1.4 kHz"] F1k4, #[name = "2.8 kHz"] F2k8, #[name = "5.6 kHz"] F5k6 }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Hpf { #[name = "Off"] Off, #[name = "80 Hz"] On }
const MIDS: [f64; 5] = [350.0, 700.0, 1400.0, 2800.0, 5600.0];


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "hpf"]
    pub hpf: EnumParam<Hpf>,
    #[id = "low"]
    pub low: FloatParam,
    #[id = "midf"]
    pub mid_f: EnumParam<MidF>,
    #[id = "mid"]
    pub mid: FloatParam,
    #[id = "high"]
    pub high: FloatParam,
    #[id = "fader"]
    pub fader: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            gain: db("Gain", 0.0, -12.0, 30.0),
            hpf: EnumParam::new("Low Cut", Hpf::Off),
            low: db("Low", 0.0, -15.0, 15.0),
            mid_f: EnumParam::new("Mid Freq", MidF::F1k4),
            mid: db("Mid", 0.0, -15.0, 15.0),
            high: db("High", 0.0, -15.0, 15.0),
            fader: db("Fader", 0.0, -30.0, 10.0),
        }
    }
}

pub const GAIN: usize = 0;
pub const HPF: usize = 1;
pub const LOW: usize = 2;
pub const MIDF: usize = 3;
pub const MID: usize = 4;
pub const HIGH: usize = 5;
pub const FADER: usize = 6;
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
            self.gain.as_ptr(),
            self.hpf.as_ptr(),
            self.low.as_ptr(),
            self.mid_f.as_ptr(),
            self.mid.as_ptr(),
            self.high.as_ptr(),
            self.fader.as_ptr(),
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
    os: [Oversampler; 2],
    hpf: [Biquad; 2],
    eq: [[Biquad; 3]; 2],
    xf: [Biquad; 2],
    key: [f64; 5],
    gain: Smooth,
    fader: Smooth,
}

impl Dsp {
    fn set(&mut self, k: [f64; 5]) {
        for c in 0..2 {
            if k[0] > 0.5 {
                self.hpf[c].set_highpass(self.fs, 80.0, 0.7);
            } else {
                self.hpf[c] = Biquad::identity();
            }
            self.eq[c][0].set_low_shelf(self.fs, 100.0, 0.7, k[1]);
            self.eq[c][1].set_peak(self.fs, MIDS[(k[2] as usize).min(4)], 1.0, k[3]);
            self.eq[c][2].set_high_shelf(self.fs, 10000.0, 0.7, k[4]);
        }
        self.key = k;
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut xf = [Biquad::identity(); 2];
        for b in &mut xf {
            // input transformer: a touch of low-end weight and gentle top roll-off
            b.set_low_shelf(fs, 60.0, 0.7, 0.8);
        }
        let mut d = Self {
            fs,
            os: [Oversampler::new(), Oversampler::new()],
            hpf: [Biquad::identity(); 2],
            eq: [[Biquad::identity(); 3]; 2],
            xf,
            key: [-1.0; 5],
            gain: Smooth::new(fs, 20.0, 1.0),
            fader: Smooth::new(fs, 20.0, 1.0),
        };
        d.set([0.0, 0.0, 2.0, 0.0, 0.0]);
        d
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.os[c].reset();
            self.hpf[c].reset();
            self.xf[c].reset();
            self.eq[c].iter_mut().for_each(|b| b.reset());
        }
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn set_params(&mut self, v: &[f32]) {
        let k = [v[HPF] as f64, v[LOW] as f64, v[MIDF].round() as f64, v[MID] as f64, v[HIGH] as f64];
        if k.iter().zip(self.key.iter()).any(|(a, b)| (a - b).abs() > 1e-3) {
            self.set(k);
        }
        self.gain.set(db_to_gain(v[GAIN] as f64));
        self.fader.set(db_to_gain(v[FADER] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let mut peak = 0.0f64;
        for k in 0..n {
            let g = self.gain.next();
            let f = self.fader.next();
            for (c, ch) in chans.iter_mut().enumerate().take(2) {
                let x = self.xf[c].process(ch[k] as f64 * g);
                let x = self.hpf[c].process(x);
                // transistor stage: clean to a point, then a firm, mostly odd-order clip
                let y = self.os[c].process(x, |s| {
                    let a = s * 0.5;
                    (a / (1.0 + a.powi(4)).powf(0.25)) * 2.0 + 0.02 * s * s / (1.0 + s * s)
                });
                let y = cascade(&mut self.eq[c], y) * f;
                peak = peak.max(y.abs());
                ch[k] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Flat Channel", values: &[] },
        Preset { name: "Drum Stem Punch", values: &[(GAIN, 10.0), (LOW, 3.0), (MIDF, 3.0), (MID, 3.0), (FADER, -6.0)] },
        Preset { name: "Driven Drum Bus", values: &[(GAIN, 22.0), (FADER, -16.0), (HIGH, 2.0)] },
        Preset { name: "Vocal Presence", values: &[(HPF, 1.0), (MIDF, 3.0), (MID, 4.0), (HIGH, 2.0)] },
        Preset { name: "Bass Warmth", values: &[(LOW, 4.0), (MIDF, 1.0), (MID, -3.0)] },
        Preset { name: "Guitar Honk Cut", values: &[(HPF, 1.0), (MIDF, 1.0), (MID, -5.0), (HIGH, 2.0)] },
        Preset { name: "Synth Air", values: &[(HIGH, 5.0), (MIDF, 0.0), (MID, -2.0)] },
        Preset { name: "Mix Bus Gentle", values: &[(GAIN, 4.0), (LOW, 1.0), (HIGH, 1.5), (FADER, -3.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "STUDER 169",
        subtitle: ("AFTER THE STUDER 169 DESK", "TRANSISTOR CHANNEL · 3-BAND EQ"),
        accent: ui::WALNUT,
        size: (W, H),
        items: vec![

            Item::Knob { p: GAIN, x: 80.0, y: 160.0, r: 38.0, label: "GAIN", tip: "Input gain. Push it to drive the transistor stages.", active: None },
            Item::Knob { p: LOW, x: 200.0, y: 160.0, r: 30.0, label: "LOW", tip: "Low shelf (about 100 Hz, estimated).", active: None },
            Item::Knob { p: MID, x: 300.0, y: 160.0, r: 30.0, label: "MID", tip: "Mid bell at the selected frequency.", active: None },
            Item::Selector { p: MIDF, x: 400.0, y: 160.0, label: "MID FREQ", positions: &["350", "700", "1.4k", "2.8k", "5.6k"], tip: "Mid frequency (positions estimated)." },
            Item::Knob { p: HIGH, x: 500.0, y: 160.0, r: 30.0, label: "HIGH", tip: "High shelf (about 10 kHz, estimated).", active: None },
            Item::Divider { y: 264.0 },
            Item::Switch { p: HPF, x: 120.0, y: 330.0, labels: ["OFF", "80 HZ"], tip: "Low-cut filter." },
            Item::Lamp { m: 0, x: 280.0, y: 330.0, label: "LEVEL", lo: 0.1, hi: 1.0 },
            Item::Knob { p: FADER, x: 470.0, y: 330.0, r: 26.0, label: "FADER", tip: "Channel level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Studer 169", class_id: *b"JFStuder169PQD01", vendor: "PEQUOD James Ford",
    categories: [Fx, Eq]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    pub fn run(p: &[f32], input: &mut [Vec<f32>]) -> Vec<f32> {
        let mut d = Dsp::new(48000.0, input.len());
        d.set_params(p);
        d.reset();
        let mut m = vec![0.0f32; Dsp::METERS.max(1)];
        let n = input[0].len();
        let mut k = 0;
        while k < n {
            let e = (k + 512).min(n);
            d.set_params(p);
            let mut chans: Vec<&mut [f32]> = input.iter_mut().map(|c| &mut c[k..e]).collect();
            d.process(&mut chans, &mut m);
            k = e;
        }
        m
    }
    pub fn sine(f: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n).map(|k| (amp * (2.0 * std::f64::consts::PI * f * k as f64 / 48000.0).sin()) as f32).collect()
    }
    pub fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
    }
    pub fn db(x: f64) -> f64 {
        20.0 * x.max(1e-12).log10()
    }
    pub fn with(mut p: Vec<f32>, set: &[(usize, f32)]) -> Vec<f32> {
        for (i, v) in set {
            p[*i] = *v;
        }
        p
    }
    /// Every preset runs finite and bounded on noise bursts.
    #[test]
    fn presets_run_clean() {
        let base = defaults();
        let mut all = vec![base.clone()];
        for pr in presets() {
            all.push(with(base.clone(), pr.values));
        }
        for p in all {
            let mut nz = vintage::dsp::Noise::new(7);
            let n = 48000;
            let mut l: Vec<f32> = (0..n).map(|k| if (k / 6000) % 2 == 0 { (nz.next() * 0.5) as f32 } else { 0.0 }).collect();
            let mut r = l.clone();
            {
                let mut input = vec![std::mem::take(&mut l), std::mem::take(&mut r)];
                run(&p, &mut input);
                assert!(input.iter().flatten().all(|v| v.is_finite() && v.abs() < 8.0), "unbounded output for {:?}", p);
            }
        }
    }

    #[test]
    fn eq_and_flat() {
        let g = |p: &[f32], f: f64| {
            let mut input = vec![sine(f, 0.05, 48000)];
            run(p, &mut input);
            db(rms(&input[0][24000..]) / (0.05 / 2f64.sqrt()))
        };
        let b = defaults();
        let flat = g(&b, 1000.0);
        let mid = g(&with(b.clone(), &[(MID, 10.0), (MIDF, 3.0)]), 2800.0) - g(&b, 2800.0);
        println!("flat {flat:.2} dB; mid +10 @2.8k -> {mid:.1}");
        assert!(flat.abs() < 1.0 && (mid - 10.0).abs() < 1.0);
    }

}
