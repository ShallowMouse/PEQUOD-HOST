//! Opto Comp: an optical compressor pedal: a light-dependent cell that grabs quickly and lets go slowly and smoothly, with a release that lengthens the longer it has been compressing. Even and musical sustain for clean parts, with a BLEND to keep the pick attack.

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
use vintage::fx::comp_gr;

pub const W: u32 = 640;
pub const H: u32 = 440;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "peak"]
    pub peak: FloatParam,
    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "blend"]
    pub blend: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            peak: pct("Peak Reduction", 0.5),
            gain: db("Gain", 4.0, 0.0, 24.0),
            blend: pct("Blend", 1.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const PEAK: usize = 0;
pub const GAIN: usize = 1;
pub const BLEND: usize = 2;
pub const OUTPUT: usize = 3;
pub const NPARAMS: usize = 4;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.peak.as_ptr(),
            self.gain.as_ptr(),
            self.blend.as_ptr(),
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
    rms: f64,
    g1: f64,
    g2: f64,
    gain: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, rms: 0.0, g1: 0.0, g2: 0.0, gain: Smooth::new(fs, 20.0, 1.0), out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.rms = 0.0;
        self.g1 = 0.0;
        self.g2 = 0.0;
        self.gain.snap(db_to_gain(self.v[GAIN] as f64));
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.gain.set(db_to_gain(v[GAIN] as f64));
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().max(1) as f64;
        let thr = -8.0 - 34.0 * self.v[PEAK] as f64;
        let blend = self.v[BLEND] as f64;
        let cav = coeff(self.fs, 5.0);
        let ca = coeff(self.fs, 10.0);
        let r1 = coeff(self.fs, 80.0);
        let a2 = coeff(self.fs, 400.0);
        let r2 = coeff(self.fs, 1800.0);
        let mut most = 0.0f64;
        for k in 0..n {
            let mut key = 0.0;
            for c in chans.iter() {
                key += (c[k] as f64).powi(2);
            }
            key /= nch;
            self.rms = key + cav * (self.rms - key);
            let lev = 10.0 * self.rms.max(1e-12).log10() + 3.0;
            let t = comp_gr(lev, thr, 4.0, 10.0);
            self.g1 = t + if t > self.g1 { ca } else { r1 } * (self.g1 - t);
            self.g2 = t + if t > self.g2 { a2 } else { r2 } * (self.g2 - t);
            let gr = 0.6 * self.g1 + 0.4 * self.g2.min(self.g1 * 1.5 + 1.0);
            most = most.max(gr);
            let g = db_to_gain(-gr) * self.gain.next();
            let o = self.out.next();
            for c in chans.iter_mut() {
                let x = c[k] as f64;
                c[k] = ((x * g * blend + x * (1.0 - blend)) * o) as f32;
            }
        }
        meters[0] = most as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Clean Sustain", values: &[] },
        Preset { name: "Country Squash", values: &[(PEAK, 0.8), (GAIN, 10.0)] },
        Preset { name: "Gentle Leveller", values: &[(PEAK, 0.3), (GAIN, 2.0)] },
        Preset { name: "Parallel (keep the pick)", values: &[(PEAK, 0.85), (GAIN, 12.0), (BLEND, 0.5)] },
        Preset { name: "Funk Rhythm", values: &[(PEAK, 0.6), (GAIN, 6.0), (BLEND, 0.8)] },
        Preset { name: "Before a Drive", values: &[(PEAK, 0.5), (GAIN, 8.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "OPTO COMP",
        subtitle: ("OPTICAL COMPRESSOR PEDAL", "SMOOTH, PROGRAM-DEPENDENT RELEASE"),
        accent: ui::MUSTARD,
        size: (W, H),
        items: vec![

            Item::Knob { p: PEAK, x: 90.0, y: 170.0, r: 38.0, label: "PEAK REDUCTION", tip: "How much it compresses.", active: None },
            Item::Knob { p: GAIN, x: 220.0, y: 170.0, r: 38.0, label: "GAIN", tip: "Make-up gain.", active: None },
            Item::Meter { m: 0, x: 300.0, y: 106.0, w: 160.0, h: 110.0, kind: MeterKind::GainReduction, caption: "GAIN REDUCTION" },
            Item::Knob { p: BLEND, x: 550.0, y: 170.0, r: 38.0, label: "BLEND", tip: "Dry under the compressed sound (fully right = all compressed).", active: None },
            Item::Divider { y: 250.0 },
            Item::Knob { p: OUTPUT, x: 320.0, y: 310.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Opto Comp", class_id: *b"PqdOptoCompPedal", vendor: "PEQUOD Pedals",
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
    fn evens_out_levels() {
        let p = with(defaults(), &[(PEAK, 0.8), (GAIN, 0.0)]);
        let loud = db(rms(&run_mono(&p, &sine(1000.0, 0.5, 48000))[24000..]));
        let quiet = db(rms(&run_mono(&p, &sine(1000.0, 0.05, 48000))[24000..]));
        println!("20 dB in -> {:.1} dB out", loud - quiet);
        assert!(loud - quiet < 12.0);
    }
    fn run_mono(p: &[f32], x: &[f32]) -> Vec<f32> {
        stereo(p, x).0
    }

}
