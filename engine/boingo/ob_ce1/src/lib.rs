//! CE-1 Chorus Ensemble: after the Boss CE-1 (1976), the first chorus pedal, built from the Roland Jazz Chorus's circuit, and part of the Oingo Boingo pedal rig. A bucket-brigade CHORUS with an INTENSITY knob, or true pitch VIBRATO with DEPTH and RATE, and the CE-1's famous preamp, which grits up when the LEVEL is pushed. Stereo: dry and chorus on one side, dry minus chorus on the other.

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
use vintage::fx::Bbd;

pub const W: u32 = 640;
pub const H: u32 = 410;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Mode { #[name = "Chorus"] Chorus, #[name = "Vibrato"] Vibrato }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "mode"]
    pub mode: EnumParam<Mode>,
    #[id = "intensity"]
    pub intensity: FloatParam,
    #[id = "depth"]
    pub depth: FloatParam,
    #[id = "rate"]
    pub rate: FloatParam,
    #[id = "level"]
    pub level: FloatParam,
    #[id = "stereo"]
    pub stereo: BoolParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            mode: EnumParam::new("Mode", Mode::Chorus),
            intensity: pct("Intensity", 0.6),
            depth: pct("Vibrato Depth", 0.5),
            rate: rate("Vibrato Rate", 4.0, 0.3, 10.0),
            level: db("Level", 0.0, -12.0, 18.0),
            stereo: BoolParam::new("Stereo", true),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const MODE: usize = 0;
pub const INTENSITY: usize = 1;
pub const DEPTH: usize = 2;
pub const RATE: usize = 3;
pub const LEVEL: usize = 4;
pub const STEREO: usize = 5;
pub const OUTPUT: usize = 6;
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
            self.mode.as_ptr(),
            self.intensity.as_ptr(),
            self.depth.as_ptr(),
            self.rate.as_ptr(),
            self.level.as_ptr(),
            self.stereo.as_ptr(),
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
    bbd: Bbd,
    ph: f64,
    hp: Biquad,
    out: Smooth,
}

impl Dsp {
    /// the preamp: a lopsided transistor stage that clips softly when pushed
    #[inline]
    pub fn pre(x: f64) -> f64 {
        let y = if x > 0.0 { (x * 1.1).tanh() / 1.1 } else { (x * 1.4).tanh() / 1.4 };
        y * 1.05
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut bbd = Bbd::new(fs, 25.0);
        bbd.set_bandwidth(6500.0);
        let mut hp = Biquad::identity();
        hp.set_highpass(fs, 20.0, 0.7);
        Self { fs, v, bbd, ph: 0.0, hp, out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.bbd.reset();
        self.hp.reset();
        self.ph = 0.0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let vib = self.v[MODE] > 0.5;
        let g = db_to_gain(self.v[LEVEL] as f64);
        let inten = self.v[INTENSITY] as f64;
        let stereo = self.v[STEREO] > 0.5;
        let (rate, depth_ms) = if vib {
            (self.v[RATE] as f64, 0.3 + 3.2 * self.v[DEPTH] as f64)
        } else {
            // the CE-1's chorus speeds up a little as INTENSITY goes up
            (0.55 + 0.6 * inten, 0.4 + 2.4 * inten)
        };
        let base = if vib { 4.5 } else { 5.5 } * 0.001 * self.fs;
        let nch = chans.len();
        let mut most = 0.0f64;
        for k in 0..n {
            self.ph = (self.ph + rate / self.fs).fract();
            let m = if vib { (2.0 * std::f64::consts::PI * self.ph).sin() } else { 1.0 - 4.0 * (self.ph - 0.5).abs() };
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            let pre = self.hp.process(Dsp::pre(x * g)) / g.max(1.0).sqrt();
            most = most.max(pre.abs());
            let w = self.bbd.process(pre, base + depth_ms * 0.001 * self.fs * m);
            let o = self.out.next();
            let (l, r) = if vib {
                (w, w)
            } else if stereo {
                (pre + w, pre - w)
            } else {
                (pre + w, pre + w)
            };
            if nch >= 2 {
                chans[0][k] = (l * 0.7 * o) as f32;
                chans[1][k] = (r * 0.7 * o) as f32;
            } else if nch == 1 {
                chans[0][k] = (0.5 * (l + r) * 0.7 * o) as f32;
            }
        }
        meters[0] = most as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Clean Ensemble", values: &[] },
        Preset { name: "Bartek Chime (wide)", values: &[(INTENSITY, 0.45), (STEREO, 1.0)] },
        Preset { name: "Deep Chorus", values: &[(INTENSITY, 1.0)] },
        Preset { name: "Warbly Vibrato", values: &[(MODE, 1.0), (DEPTH, 0.6), (RATE, 5.5)] },
        Preset { name: "Slow Seasick Vibrato", values: &[(MODE, 1.0), (DEPTH, 0.9), (RATE, 1.2)] },
        Preset { name: "Pushed Preamp Grit", values: &[(LEVEL, 14.0), (INTENSITY, 0.5), (OUTPUT, -4.0)] },
        Preset { name: "Keyboard Ensemble (mono)", values: &[(INTENSITY, 0.7), (STEREO, 0.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "CE-1 CHORUS ENSEMBLE",
        subtitle: ("AFTER THE BOSS CE-1 (1976)", "BBD CHORUS · VIBRATO · PREAMP"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![

            Item::Selector { p: MODE, x: 80.0, y: 170.0, label: "MODE", positions: &["CHORUS", "VIBRATO"], tip: "Chorus (dry plus modulated) or vibrato (modulated only: true pitch wobble)." },
            Item::Knob { p: INTENSITY, x: 210.0, y: 170.0, r: 36.0, label: "INTENSITY", tip: "Chorus depth (and a little speed).", active: Some(ui::ActiveWhen { p: MODE, min: 0.0, max: 0.0 }) },
            Item::Knob { p: DEPTH, x: 330.0, y: 170.0, r: 30.0, label: "DEPTH", tip: "Vibrato depth.", active: Some(ui::ActiveWhen { p: MODE, min: 1.0, max: 1.0 }) },
            Item::Knob { p: RATE, x: 430.0, y: 170.0, r: 30.0, label: "RATE", tip: "Vibrato rate.", active: Some(ui::ActiveWhen { p: MODE, min: 1.0, max: 1.0 }) },
            Item::Lamp { m: 0, x: 540.0, y: 150.0, label: "PEAK", lo: 0.5, hi: 0.95 },
            Item::Divider { y: 245.0 },
            Item::Knob { p: LEVEL, x: 150.0, y: 305.0, r: 26.0, label: "LEVEL", tip: "Preamp input level: push it for the CE-1's grit.", active: None },
            Item::Switch { p: STEREO, x: 280.0, y: 305.0, labels: ["MONO", "STEREO"], tip: "Stereo: dry + chorus one side, dry - chorus the other." },
            Item::Knob { p: OUTPUT, x: 490.0, y: 305.0, r: 26.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "CE-1 Chorus Ensemble", class_id: *b"OBCE1ChorusEnsPQ", vendor: "PEQUOD Oingo Boingo",
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
    fn chorus_vibrato_preamp() {
        let (l, r) = stereo(&defaults(), &sine(700.0, 0.2, 48000));
        let c = corr(&l[24000..], &r[24000..]);
        let v = stereo(&with(defaults(), &[(MODE, 1.0), (DEPTH, 0.8)]), &sine(1000.0, 0.2, 48000)).0;
        let pure = goertzel(&v[24000..], 1000.0) / (rms(&v[24000..]) * 2f64.sqrt());
        let clean = stereo(&with(defaults(), &[(INTENSITY, 0.0), (STEREO, 0.0)]), &sine(300.0, 0.2, 24000)).0;
        let dirty = stereo(&with(defaults(), &[(INTENSITY, 0.0), (STEREO, 0.0), (LEVEL, 18.0)]), &sine(300.0, 0.2, 24000)).0;
        let h = |y: &[f32]| db(goertzel(&y[12000..], 900.0) / goertzel(&y[12000..], 300.0));
        println!("L/R corr {c:.2}; vibrato purity {:.1} dB; 3rd harmonic clean {:.1} / pushed {:.1} dB", db(pure), h(&clean), h(&dirty));
        assert!(c < 0.9 && db(pure) < -6.0 && h(&dirty) > h(&clean) + 15.0);
    }

}
