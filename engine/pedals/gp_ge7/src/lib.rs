//! GE-7 EQ: a seven-band graphic EQ pedal after the Boss GE-7: ±15 dB at 100 Hz to 6.4 kHz and a LEVEL control. Classic uses: a mid boost before an amp, scooping the mids, or as a solo boost with extra cut.

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
pub const H: u32 = 300;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "b0"]
    pub b0: FloatParam,
    #[id = "b1"]
    pub b1: FloatParam,
    #[id = "b2"]
    pub b2: FloatParam,
    #[id = "b3"]
    pub b3: FloatParam,
    #[id = "b4"]
    pub b4: FloatParam,
    #[id = "b5"]
    pub b5: FloatParam,
    #[id = "b6"]
    pub b6: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            b0: db("100 Hz", 0.0, -15.0, 15.0),
            b1: db("200 Hz", 0.0, -15.0, 15.0),
            b2: db("400 Hz", 0.0, -15.0, 15.0),
            b3: db("800 Hz", 0.0, -15.0, 15.0),
            b4: db("1.6k Hz", 0.0, -15.0, 15.0),
            b5: db("3.2k Hz", 0.0, -15.0, 15.0),
            b6: db("6.4k Hz", 0.0, -15.0, 15.0),
            output: db("Level", 0.0, -15.0, 15.0),
        }
    }
}

pub const B0: usize = 0;
pub const B1: usize = 1;
pub const B2: usize = 2;
pub const B3: usize = 3;
pub const B4: usize = 4;
pub const B5: usize = 5;
pub const B6: usize = 6;
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
            self.b0.as_ptr(),
            self.b1.as_ptr(),
            self.b2.as_ptr(),
            self.b3.as_ptr(),
            self.b4.as_ptr(),
            self.b5.as_ptr(),
            self.b6.as_ptr(),
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

const BANDS: [f64; 7] = [100.0, 200.0, 400.0, 800.0, 1600.0, 3200.0, 6400.0];

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    eq: Vec<[Biquad; 7]>,
    key: Vec<i32>,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        for e in self.eq.iter_mut() {
            for (i, b) in e.iter_mut().enumerate() {
                b.set_peak(self.fs, BANDS[i], 1.4, self.v[i] as f64);
            }
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, eq: vec![[Biquad::identity(); 7]; ch.max(2)], key: vec![], out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.eq.iter_mut().for_each(|e| e.iter_mut().for_each(|b| b.reset()));
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k: Vec<i32> = v[..7].iter().map(|x| (x * 10.0) as i32).collect();
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let last = self.eq.len() - 1;
        for k in 0..n {
            let o = self.out.next();
            for (ci, c) in chans.iter_mut().enumerate() {
                c[k] = (cascade(&mut self.eq[ci.min(last)], c[k] as f64) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Flat", values: &[] },
        Preset { name: "Mid Boost into the Amp", values: &[(B3, 8.0), (B4, 6.0), (B2, 3.0), (B0, -3.0)] },
        Preset { name: "Scooped", values: &[(B2, -6.0), (B3, -9.0), (B4, -6.0), (B0, 3.0), (B6, 3.0)] },
        Preset { name: "Solo Boost", values: &[(B3, 4.0), (B4, 5.0), (B5, 2.0), (OUTPUT, 6.0)] },
        Preset { name: "Tighten the Lows", values: &[(B0, -9.0), (B1, -4.0)] },
        Preset { name: "Tame the Fizz", values: &[(B5, -4.0), (B6, -9.0)] },
        Preset { name: "Baritone Clarity", values: &[(B0, -5.0), (B1, -3.0), (B4, 3.0), (B5, 4.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "GE-7 EQ",
        subtitle: ("AFTER THE BOSS GE-7", "SEVEN-BAND GRAPHIC EQ"),
        accent: ui::COBALT,
        size: (W, H),
        items: vec![
            Item::Knob { p: B0, x: 60.0, y: 170.0, r: 26.0, label: "100", tip: "Boost or cut at 100 Hz.", active: None },
            Item::Knob { p: B1, x: 135.0, y: 170.0, r: 26.0, label: "200", tip: "Boost or cut at 200 Hz.", active: None },
            Item::Knob { p: B2, x: 210.0, y: 170.0, r: 26.0, label: "400", tip: "Boost or cut at 400 Hz.", active: None },
            Item::Knob { p: B3, x: 285.0, y: 170.0, r: 26.0, label: "800", tip: "Boost or cut at 800 Hz.", active: None },
            Item::Knob { p: B4, x: 360.0, y: 170.0, r: 26.0, label: "1.6k", tip: "Boost or cut at 1.6k Hz.", active: None },
            Item::Knob { p: B5, x: 435.0, y: 170.0, r: 26.0, label: "3.2k", tip: "Boost or cut at 3.2k Hz.", active: None },
            Item::Knob { p: B6, x: 510.0, y: 170.0, r: 26.0, label: "6.4k", tip: "Boost or cut at 6.4k Hz.", active: None },
            Item::Knob { p: OUTPUT, x: 585.0, y: 170.0, r: 26.0, label: "LEVEL", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "GE-7 EQ", class_id: *b"PqdGE7GraphicEQ1", vendor: "PEQUOD Pedals",
    categories: [Fx, Eq]
);

#[cfg(test)]
#[allow(dead_code)]
pub mod tests {
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
    fn band_boosts() {
        let mut input = vec![sine(800.0, 0.05, 24000)];
        run(&with(defaults(), &[(B3, 12.0)]), &mut input);
        let g = db(rms(&input[0][12000..])) - db(0.05 / 2f64.sqrt());
        println!("800 Hz band +12: {g:.1} dB");
        assert!((g - 12.0).abs() < 1.5);
    }

}
