//! Ring Mod: after the Moog MF-102 Ring Modulator: your signal multiplied by a sine carrier, which replaces each note with the sum and difference of the two: bells, robots and clangs. An LFO can sweep the carrier.

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
pub const H: u32 = 330;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "freq"]
    pub freq: FloatParam,
    #[id = "lforate"]
    pub lforate: FloatParam,
    #[id = "lfoamt"]
    pub lfoamt: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            freq: hz("Frequency", 440.0, 20.0, 4000.0),
            lforate: rate("LFO Rate", 1.0, 0.05, 15.0),
            lfoamt: pct("LFO Amount", 0.0),
            mix: pct("Mix", 1.0),
            drive: pct("Drive", 0.2),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const FREQ: usize = 0;
pub const LFORATE: usize = 1;
pub const LFOAMT: usize = 2;
pub const MIX: usize = 3;
pub const DRIVE: usize = 4;
pub const OUTPUT: usize = 5;
pub const NPARAMS: usize = 6;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.freq.as_ptr(),
            self.lforate.as_ptr(),
            self.lfoamt.as_ptr(),
            self.mix.as_ptr(),
            self.drive.as_ptr(),
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
    ph: f64,
    lfo: Lfo,
    f: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, ph: 0.0, lfo: Lfo::default(), f: Smooth::new(fs, 30.0, 440.0), out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.ph = 0.0;
        self.lfo = Lfo::default();
        self.f.snap(self.v[FREQ] as f64);
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.f.set(v[FREQ] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let amt = self.v[LFOAMT] as f64;
        let rate = self.v[LFORATE] as f64;
        let mix = self.v[MIX] as f64;
        let drive = 1.0 + 8.0 * self.v[DRIVE] as f64;
        for k in 0..n {
            let l = self.lfo.next(self.fs, rate);
            let f = self.f.next() * 2f64.powf(2.0 * amt * l);
            self.ph = (self.ph + f / self.fs).fract();
            let car = (2.0 * std::f64::consts::PI * self.ph).sin();
            let o = self.out.next();
            for c in chans.iter_mut() {
                let x = c[k] as f64;
                let xd = (x * drive).tanh() / drive.tanh().max(0.5) ;
                c[k] = ((xd * car * mix + x * (1.0 - mix)) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Bell Tones", values: &[] },
        Preset { name: "Robot Voice", values: &[(FREQ, 60.0)] },
        Preset { name: "Clang", values: &[(FREQ, 1800.0), (DRIVE, 0.5)] },
        Preset { name: "Swept Sci-Fi", values: &[(FREQ, 600.0), (LFOAMT, 0.6), (LFORATE, 0.4)] },
        Preset { name: "Tremolo-ish (low carrier)", values: &[(FREQ, 20.0), (MIX, 0.7)] },
        Preset { name: "Half Ring", values: &[(FREQ, 330.0), (MIX, 0.5)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "RING MOD",
        subtitle: ("AFTER THE MOOG MF-102", "BELLS · ROBOTS · CLANGS"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![
            Item::Knob { p: FREQ, x: 70.0, y: 190.0, r: 32.0, label: "FREQUENCY", tip: "Carrier frequency.", active: None },
            Item::Knob { p: LFORATE, x: 170.0, y: 190.0, r: 32.0, label: "LFO RATE", tip: "Speed of the carrier sweep.", active: None },
            Item::Knob { p: LFOAMT, x: 270.0, y: 190.0, r: 32.0, label: "LFO AMOUNT", tip: "Depth of the sweep (up to two octaves).", active: None },
            Item::Knob { p: DRIVE, x: 370.0, y: 190.0, r: 32.0, label: "DRIVE", tip: "Input drive.", active: None },
            Item::Knob { p: MIX, x: 470.0, y: 190.0, r: 32.0, label: "MIX", tip: "Ring-modulated against dry.", active: None },
            Item::Knob { p: OUTPUT, x: 570.0, y: 190.0, r: 32.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Ring Mod", class_id: *b"PqdRingModMF102A", vendor: "PEQUOD Pedals",
    categories: [Fx, Modulation]
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

    use std::f64::consts::PI;
    fn goertzel(x: &[f32], f: f64) -> f64 {
        let w = 2.0 * PI * f / 48000.0;
        let (mut s1, mut s2) = (0.0, 0.0);
        for v in x {
            let s = *v as f64 + 2.0 * w.cos() * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        ((s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2).max(0.0)).sqrt() / x.len() as f64 * 2.0
    }
    fn stereo(p: &[f32], x: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        let r = input.pop().unwrap();
        (input.pop().unwrap(), r)
    }
    fn corr(a: &[f32], b: &[f32]) -> f64 {
        let ab: f64 = a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum();
        let aa: f64 = a.iter().map(|x| (*x as f64).powi(2)).sum();
        let bb: f64 = b.iter().map(|x| (*x as f64).powi(2)).sum();
        ab / (aa * bb).sqrt().max(1e-12)
    }
    /// short-window rms envelope
    fn env(x: &[f32], w: usize) -> Vec<f64> {
        x.chunks(w).map(rms).collect()
    }

    #[test]
    fn sum_and_difference() {
        let p = with(defaults(), &[(FREQ, 1000.0), (DRIVE, 0.0)]);
        let y = stereo(&p, &sine(440.0, 0.1, 48000)).0;
        let s = goertzel(&y[24000..], 1440.0);
        let d = goertzel(&y[24000..], 560.0);
        let o = goertzel(&y[24000..], 440.0);
        println!("560 {:.1} / 1440 {:.1} / 440 {:.1} dB", db(d), db(s), db(o));
        assert!(db(s) > db(o) + 30.0 && db(d) > db(o) + 30.0);
    }

}
