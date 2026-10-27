//! Harmonic Trem: the harmonic tremolo of early-60s brown Fender amps: the signal is split into lows and highs, and the two are pulsed in opposition, so the tremolo throbs and almost phases rather than simply getting louder and quieter.

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
    #[id = "speed"]
    pub speed: FloatParam,
    #[id = "intensity"]
    pub intensity: FloatParam,
    #[id = "xover"]
    pub xover: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            speed: rate("Speed", 4.0, 0.5, 12.0),
            intensity: pct("Intensity", 0.6),
            xover: hz("Crossover", 700.0, 250.0, 2000.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const SPEED: usize = 0;
pub const INTENSITY: usize = 1;
pub const XOVER: usize = 2;
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
            self.speed.as_ptr(),
            self.intensity.as_ptr(),
            self.xover.as_ptr(),
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
    lp: Vec<[Biquad; 2]>,
    hp: Vec<[Biquad; 2]>,
    lfo: Lfo,
    key: i32,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let f = self.v[XOVER] as f64;
        for b in self.lp.iter_mut() {
            b[0].set_lowpass(self.fs, f, 0.7071);
            b[1].set_lowpass(self.fs, f, 0.7071);
        }
        for b in self.hp.iter_mut() {
            b[0].set_highpass(self.fs, f, 0.7071);
            b[1].set_highpass(self.fs, f, 0.7071);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, lp: vec![[Biquad::identity(); 2]; ch.max(2)], hp: vec![[Biquad::identity(); 2]; ch.max(2)], lfo: Lfo::default(), key: -1, out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.lp.iter_mut().chain(self.hp.iter_mut()).for_each(|b| b.iter_mut().for_each(|q| q.reset()));
        self.lfo = Lfo::default();
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = v[XOVER] as i32;
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let i = self.v[INTENSITY] as f64;
        let sp = self.v[SPEED] as f64;
        let last = self.lp.len() - 1;
        for k in 0..n {
            let s = self.lfo.next(self.fs, sp);
            let gl = 1.0 - i * (0.5 + 0.5 * s);
            let gh = 1.0 - i * (0.5 - 0.5 * s);
            let o = self.out.next();
            for (ci, c) in chans.iter_mut().enumerate() {
                let j = ci.min(last);
                let x = c[k] as f64;
                let lo = cascade(&mut self.lp[j], x);
                let hi = cascade(&mut self.hp[j], x);
                c[k] = ((lo * gl + hi * gh) * (1.0 + 0.5 * i) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Brownface Throb", values: &[] },
        Preset { name: "Slow Harmonic Swirl", values: &[(SPEED, 1.5), (INTENSITY, 0.8)] },
        Preset { name: "Fast Shimmer", values: &[(SPEED, 8.0), (INTENSITY, 0.45)] },
        Preset { name: "Deep and Dark", values: &[(INTENSITY, 0.9), (XOVER, 450.0)] },
        Preset { name: "Bright Split", values: &[(XOVER, 1400.0)] },
        Preset { name: "Subtle Pulse", values: &[(INTENSITY, 0.3), (SPEED, 5.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "HARMONIC TREM",
        subtitle: ("THE BROWNFACE FENDER TREMOLO", "LOWS AND HIGHS PULSE IN OPPOSITION"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![
            Item::Knob { p: SPEED, x: 90.0, y: 190.0, r: 40.0, label: "SPEED", tip: "Tremolo rate.", active: None },
            Item::Knob { p: INTENSITY, x: 243.3, y: 190.0, r: 40.0, label: "INTENSITY", tip: "Depth.", active: None },
            Item::Knob { p: XOVER, x: 396.7, y: 190.0, r: 40.0, label: "CROSSOVER", tip: "Where lows and highs are split.", active: None },
            Item::Knob { p: OUTPUT, x: 550.0, y: 190.0, r: 40.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Harmonic Trem", class_id: *b"PqdHarmonicTrem1", vendor: "PEQUOD Pedals",
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
    fn bands_in_opposition() {
        let p = with(defaults(), &[(INTENSITY, 0.9), (SPEED, 3.0)]);
        let lo = stereo(&p, &sine(150.0, 0.2, 48000)).0;
        let hi = stereo(&p, &sine(3000.0, 0.2, 48000)).0;
        let el: Vec<f32> = env(&lo[9600..], 480).iter().map(|v| *v as f32).collect();
        let eh: Vec<f32> = env(&hi[9600..], 480).iter().map(|v| *v as f32).collect();
        let m = |e: &[f32]| e.iter().map(|v| *v as f64).sum::<f64>() / e.len() as f64;
        let (ml, mh) = (m(&el), m(&eh));
        let a: Vec<f32> = el.iter().map(|v| (*v as f64 - ml) as f32).collect();
        let b: Vec<f32> = eh.iter().map(|v| (*v as f64 - mh) as f32).collect();
        let c = corr(&a, &b);
        println!("envelope correlation lows/highs {c:.2}");
        assert!(c < -0.7);
    }

}
