//! Slow Gear: an auto-swell after the Boss SG-1 Slow Gear: every new note or chord starts silent and fades in, for violin-like swells without riding the volume knob. SENS sets how hard a pick has to be to count as a new note, ATTACK the length of the fade.

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
pub const H: u32 = 340;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "sens"]
    pub sens: FloatParam,
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            sens: pct("Sensitivity", 0.5),
            attack: ms("Attack", 600.0, 50.0, 3000.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const SENS: usize = 0;
pub const ATTACK: usize = 1;
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
            self.sens.as_ptr(),
            self.attack.as_ptr(),
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
    fast: f64,
    slow: f64,
    hold: usize,
    g: f64,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, fast: 0.0, slow: 0.0, hold: 0, g: 1.0, out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.fast = 0.0;
        self.slow = 0.0;
        self.hold = 0;
        self.g = 1.0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let thr = db_to_gain(-20.0 - 40.0 * self.v[SENS] as f64);
        let step = 1.0 / (self.v[ATTACK] as f64 * 0.001 * self.fs);
        let ca = coeff(self.fs, 1.0);
        let cr = coeff(self.fs, 30.0);
        let cs = coeff(self.fs, 150.0);
        for k in 0..n {
            let mut a = 0.0f64;
            for c in chans.iter() {
                a = a.max((c[k] as f64).abs());
            }
            self.fast = a + if a > self.fast { ca } else { cr } * (self.fast - a);
            self.slow = self.fast + cs * (self.slow - self.fast);
            if self.hold > 0 {
                self.hold -= 1;
            } else if self.fast > thr && self.fast > 2.0 * self.slow + thr * 0.5 {
                self.g = 0.0;
                self.hold = (0.08 * self.fs) as usize;
            }
            self.g = (self.g + step).min(1.0);
            let gg = self.g * self.g;
            let o = self.out.next();
            for c in chans.iter_mut() {
                c[k] = (c[k] as f64 * gg * o) as f32;
            }
        }
        meters[0] = self.g as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Violin Swell", values: &[] },
        Preset { name: "Fast Swell", values: &[(ATTACK, 200.0)] },
        Preset { name: "Slow Pad", values: &[(ATTACK, 1800.0), (SENS, 0.6)] },
        Preset { name: "Hard Picking Only", values: &[(SENS, 0.2)] },
        Preset { name: "Ambient (into a delay)", values: &[(ATTACK, 1000.0), (SENS, 0.7)] },
        Preset { name: "Barely There", values: &[(ATTACK, 90.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "SLOW GEAR",
        subtitle: ("AFTER THE BOSS SG-1", "AUTOMATIC VOLUME SWELLS"),
        accent: ui::OLIVE,
        size: (W, H),
        items: vec![
            Item::Knob { p: SENS, x: 90.0, y: 190.0, r: 40.0, label: "SENS", tip: "How hard a note must be picked to restart the swell.", active: None },
            Item::Knob { p: ATTACK, x: 320.0, y: 190.0, r: 40.0, label: "ATTACK", tip: "Length of the fade-in.", active: None },
            Item::Knob { p: OUTPUT, x: 550.0, y: 190.0, r: 40.0, label: "OUTPUT", tip: "Output level.", active: None },
            Item::Lamp { m: 0, x: 430.0, y: 110.0, label: "SWELL", lo: 0.05, hi: 0.95 },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Slow Gear", class_id: *b"PqdSlowGearSwell", vendor: "PEQUOD Pedals",
    categories: [Fx, Dynamics]
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
    fn notes_fade_in() {
        let mut x = vec![0.0f32; 48000];
        for k in 4800..48000 {
            let t = (k - 4800) as f64 / 48000.0;
            x[k] = (0.3 * (-t / 2.0).exp() * (2.0 * std::f64::consts::PI * 220.0 * t).sin()) as f32;
        }
        let mut input = vec![x.clone(), x];
        run(&defaults(), &mut input);
        let start = db(rms(&input[0][4800..6240]));
        let later = db(rms(&input[0][4800 + 28800..4800 + 33600]));
        println!("first 30 ms {start:.1} dB, after 0.6 s {later:.1} dB");
        assert!(later - start > 15.0);
    }

}
