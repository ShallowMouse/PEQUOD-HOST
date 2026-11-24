//! Aural Exciter: Aphex's Type B (1975 on), the box behind the "air" on a lot of 70s and 80s records. A
//! side chain high-passes the signal (TUNE), drives it into a harmonics generator, and mixes a little of
//! it back (MIX), phase-shifted, under the original: presence and sparkle without much more level. DRIVE
//! sets how hard the generator works. Estimated from Aphex's documentation.

#![allow(clippy::excessive_precision)]
use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
#[allow(unused_imports)]
use vintage::dsp::*;
#[allow(unused_imports)]
use vintage::fx::*;
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
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "tune"]
    pub tune: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            drive: pct("Drive", 0.5),
            tune: hz("Tune", 3000.0, 700.0, 7000.0),
            mix: pct("Mix", 0.3),
        }
    }
}

pub const DRIVE: usize = 0;
pub const TUNE: usize = 1;
pub const MIX: usize = 2;
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
            self.drive.as_ptr(),
            self.tune.as_ptr(),
            self.mix.as_ptr(),
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
    hp: Vec<[Biquad; 2]>,
    shift: Vec<Biquad>,
    post: Vec<Biquad>,
}

impl Dsp {
    fn design(&mut self) {
        let f = self.v[TUNE] as f64;
        for h in self.hp.iter_mut() {
            h[0].set_highpass(self.fs, f, 0.7);
            h[1].set_highpass1(self.fs, f);
        }
        for s in self.shift.iter_mut() {
            s.set_lowpass1(self.fs, 18000.0f64.min(self.fs * 0.45));
        }
        for p in self.post.iter_mut() {
            p.set_highpass1(self.fs, f * 0.8);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let n = ch.max(2);
        let mut d = Self { fs, v, hp: vec![[Biquad::identity(); 2]; n], shift: vec![Biquad::identity(); n], post: vec![Biquad::identity(); n] };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.hp.iter_mut().flatten().chain(self.shift.iter_mut()).chain(self.post.iter_mut()).for_each(|b| b.reset());
        self.design();
    }
    fn set_params(&mut self, v: &[f32]) {
        let changed = self.v[TUNE] != v[TUNE];
        self.v.copy_from_slice(&v[..NPARAMS]);
        if changed {
            self.design();
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let drive = 1.0 + self.v[DRIVE] as f64 * 20.0;
        let mix = self.v[MIX] as f64 * 0.5;
        for (ci, ch) in chans.iter_mut().enumerate() {
            let j = ci.min(self.hp.len() - 1);
            for s in ch.iter_mut() {
                let x = *s as f64;
                let h0 = self.hp[j][0].process(x);
                let h = self.hp[j][1].process(h0);
                // the generator: odd and even harmonics of the top end only
                let g = (h * drive).tanh() / drive.sqrt() + 0.3 * (h * drive * 0.5).powi(2).min(1.0) / drive.sqrt();
                let g = self.post[j].process(self.shift[j].process(g));
                *s = (x + g * mix) as f32;
            }
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Vocal Air", values: &[(TUNE, 4500.0), (DRIVE, 0.45), (MIX, 0.3)] },
        Preset { name: "Acoustic Sparkle", values: &[(TUNE, 3500.0), (DRIVE, 0.5), (MIX, 0.35)] },
        Preset { name: "Dull Drums Woken Up", values: &[(TUNE, 2500.0), (DRIVE, 0.6), (MIX, 0.35)] },
        Preset { name: "Mix, a Touch", values: &[(TUNE, 5000.0), (DRIVE, 0.3), (MIX, 0.15)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "AURAL EXCITER",
        subtitle: ("APHEX TYPE B", "HARMONICS OF THE TOP END, MIXED UNDER"),
        accent: ui::TANGERINE,
        size: (W, H),
        items: vec![
            Item::Knob { p: DRIVE, x: 160.0, y: 200.0, r: 40.0, label: "DRIVE", tip: "How hard the harmonics generator works.", active: None },
            Item::Knob { p: TUNE, x: 320.0, y: 200.0, r: 40.0, label: "TUNE", tip: "Where the side chain starts: only what's above is excited.", active: None },
            Item::Knob { p: MIX, x: 480.0, y: 200.0, r: 40.0, label: "MIX", tip: "How much of it goes under the original.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Aural Exciter", class_id: *b"HWAphexExciterPQ", vendor: "PEQUOD Hardware",
    categories: [Fx, Distortion]
);

#[cfg(test)]
#[allow(dead_code)]
pub mod tests {
    use super::*;
    use std::f64::consts::PI;

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
    pub fn sine(f: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n).map(|k| (amp * (2.0 * PI * f * k as f64 / 48000.0).sin()) as f32).collect()
    }
    pub fn noise(n: usize, amp: f64) -> Vec<f32> {
        let mut nz = vintage::dsp::Noise::new(11);
        (0..n).map(|_| (nz.next() * amp) as f32).collect()
    }
    /// Both channels out for a mono input.
    pub fn go2(p: &[f32], x: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        let r = input.pop().unwrap();
        (input.pop().unwrap(), r)
    }
    pub fn go(p: &[f32], x: &[f32]) -> Vec<f32> {
        go2(p, x).0
    }
    /// Steady-state level of a sine at f through the settings, in dB.
    pub fn level(set: &[(usize, f32)], f: f64) -> f64 {
        let p = with(defaults(), set);
        let y = go(&p, &sine(f, 0.1, 48000));
        db(rms(&y[24000..]))
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
            let l: Vec<f32> = (0..n).map(|k| if (k / 6000) % 2 == 0 { (nz.next() * 0.5) as f32 } else { 0.0 }).collect();
            let mut input = vec![l.clone(), l];
            run(&p, &mut input);
            assert!(input.iter().flatten().all(|v| v.is_finite() && v.abs() < 8.0), "unbounded output for {:?}", p);
        }
    }

    #[test]
    fn makes_sound() {
        let p = with(defaults(), &[(MIX, 0.8)]);
        let y = go(&p, &sine(440.0, 0.2, 48000));
        let l = db(rms(&y[24000..]));
        println!("level {:.1} dB", l);
        assert!(l > -40.0 && y.iter().all(|v| v.is_finite()));
    }

}
