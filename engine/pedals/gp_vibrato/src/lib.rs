//! Vibrato: true pitch vibrato after the Boss VB-2: a modulated delay with no dry signal, so the pitch itself wavers like a singer's. RISE fades the vibrato in after each new note, as a singer does.

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
    #[id = "rate"]
    pub rate: FloatParam,
    #[id = "depth"]
    pub depth: FloatParam,
    #[id = "rise"]
    pub rise: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            rate: rate("Rate", 5.0, 0.5, 12.0),
            depth: pct("Depth", 0.4),
            rise: ms("Rise Time", 0.0, 0.0, 2000.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const RATE: usize = 0;
pub const DEPTH: usize = 1;
pub const RISE: usize = 2;
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
            self.rate.as_ptr(),
            self.depth.as_ptr(),
            self.rise.as_ptr(),
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
    line: Vec<DelayLine>,
    lfo: Lfo,
    env: f64,
    rise: f64,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, line: (0..ch.max(2)).map(|_| DelayLine::new((0.03 * fs) as usize)).collect(), lfo: Lfo::default(), env: 0.0, rise: 1.0, out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.line.iter_mut().for_each(|l| l.reset());
        self.lfo = Lfo::default();
        self.env = 0.0;
        self.rise = 1.0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let depth = (self.v[DEPTH] as f64) * 0.0045 * self.fs;
        let rt = self.v[RISE] as f64 * 0.001 * self.fs;
        let step = if rt < 1.0 { 1.0 } else { 1.0 / rt };
        let cr = coeff(self.fs, 50.0);
        let base = 0.002 * self.fs;
        let rate = self.v[RATE] as f64;
        let last = self.line.len() - 1;
        for k in 0..n {
            let mut a = 0.0f64;
            for c in chans.iter() {
                a = a.max((c[k] as f64).abs());
            }
            let was = self.env;
            self.env = a.max(a + cr * (self.env - a));
            if was < 0.003 && self.env >= 0.003 {
                self.rise = 0.0;
            }
            self.rise = (self.rise + step).min(1.0);
            let l = self.lfo.next(self.fs, rate);
            let d = base + depth * self.rise * (0.5 + 0.5 * l);
            let o = self.out.next();
            for (ci, c) in chans.iter_mut().enumerate() {
                let j = ci.min(last);
                self.line[j].write(c[k] as f64);
                c[k] = (self.line[j].read(d) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Singer's Vibrato", values: &[] },
        Preset { name: "Deep and Slow", values: &[(RATE, 2.0), (DEPTH, 0.8)] },
        Preset { name: "Warble", values: &[(RATE, 8.0), (DEPTH, 0.3)] },
        Preset { name: "Rising Vibrato", values: &[(RISE, 700.0), (DEPTH, 0.5)] },
        Preset { name: "Seasick", values: &[(RATE, 1.2), (DEPTH, 1.0)] },
        Preset { name: "Subtle Shimmer", values: &[(RATE, 6.0), (DEPTH, 0.15)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "VIBRATO",
        subtitle: ("AFTER THE BOSS VB-2", "TRUE PITCH VIBRATO · RISE TIME"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![
            Item::Knob { p: RATE, x: 90.0, y: 190.0, r: 40.0, label: "RATE", tip: "Vibrato speed.", active: None },
            Item::Knob { p: DEPTH, x: 243.3, y: 190.0, r: 40.0, label: "DEPTH", tip: "How far the pitch moves.", active: None },
            Item::Knob { p: RISE, x: 396.7, y: 190.0, r: 40.0, label: "RISE TIME", tip: "Vibrato fades in after each new note.", active: None },
            Item::Knob { p: OUTPUT, x: 550.0, y: 190.0, r: 40.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Vibrato", class_id: *b"PqdVibratoVB2Q01", vendor: "PEQUOD Pedals",
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
    fn pitch_moves() {
        let still = stereo(&with(defaults(), &[(DEPTH, 0.0)]), &sine(1000.0, 0.2, 48000)).0;
        let moved = stereo(&with(defaults(), &[(DEPTH, 0.6)]), &sine(1000.0, 0.2, 48000)).0;
        let a = goertzel(&still[24000..], 1000.0) / (rms(&still[24000..]) * 2f64.sqrt());
        let b = goertzel(&moved[24000..], 1000.0) / (rms(&moved[24000..]) * 2f64.sqrt());
        println!("1 kHz purity still {:.1} dB, vibrato {:.1} dB", db(a), db(b));
        assert!(db(a) > -1.0 && db(b) < -6.0);
    }

}
