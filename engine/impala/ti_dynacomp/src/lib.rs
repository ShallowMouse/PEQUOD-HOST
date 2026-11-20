//! Dyna Comp: the MXR Dyna Comp, on Kevin Parker's pedalboard ("just a compressor, a vibrato and a reverb",
//! Premier Guitar), the squash behind guitars compressed until they sound like synths. Two knobs, Output and
//! Sensitivity, round an OTA (CA3080) gain cell: a fast attack, a slow release that swells notes back up,
//! heavy, quacky compression and a slight loss of top. Curves follow published analyses of the circuit and
//! are estimated.

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
    #[id = "output"]
    pub output: FloatParam,
    #[id = "sens"]
    pub sens: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            output: db("Output", 0.0, -24.0, 18.0),
            sens: pct("Sensitivity", 0.6),
        }
    }
}

pub const OUTPUT: usize = 0;
pub const SENS: usize = 1;
pub const NPARAMS: usize = 2;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.output.as_ptr(),
            self.sens.as_ptr(),
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
    env: f64,
    top: Vec<Biquad>,
    hp: Vec<Biquad>,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let n = ch.max(2);
        let mut t = Biquad::identity();
        t.set_lowpass(fs, 6500.0, 0.6);
        let mut h = Biquad::identity();
        h.set_highpass(fs, 40.0, 0.7);
        Self { fs, v, env: 0.0, top: vec![t; n], hp: vec![h; n], out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.env = 0.0;
        self.top.iter_mut().chain(self.hp.iter_mut()).for_each(|b| b.reset());
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let sens = self.v[SENS] as f64;
        // sensitivity: the gain in front of the gain cell, so more squash and more noise
        let pre = db_to_gain(36.0 * sens);
        let ca = coeff(self.fs, 4.0);
        let cr = coeff(self.fs, 650.0);
        let last = self.top.len() - 1;
        let mut most = 0.0f64;
        for k in 0..n {
            let o = self.out.next();
            let peak = chans.iter().map(|c| (c[k] as f64).abs()).fold(0.0, f64::max) * pre;
            let c = if peak > self.env { ca } else { cr };
            self.env = peak + c * (self.env - peak);
            // the OTA's gain falls roughly as 1 / level above a small threshold
            let g = 1.0 / (1.0 + 6.0 * self.env);
            most = most.max(-gain_to_db(g));
            for (ci, ch) in chans.iter_mut().enumerate() {
                let j = ci.min(last);
                let x = ch[k] as f64 * pre * g;
                // the OTA's own soft limit, then the output op-amp's top roll-off
                let y = (x * 1.2).tanh() / 1.2;
                let y = self.top[j].process(self.hp[j].process(y)) * 0.6;
                ch[k] = (y * o) as f32;
            }
        }
        meters[0] = most as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Synth-Like Guitar", values: &[] },
        Preset { name: "Country Squash", values: &[(SENS, 0.8), (OUTPUT, -2.0)] },
        Preset { name: "Light Sustain", values: &[(SENS, 0.3), (OUTPUT, 2.0)] },
        Preset { name: "Bass Evener", values: &[(SENS, 0.45), (OUTPUT, 1.0)] },
        Preset { name: "Everything Squashed", values: &[(SENS, 1.0), (OUTPUT, -4.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "DYNA COMP",
        subtitle: ("AN OTA COMPRESSOR IN A PEDAL", "GUITARS SQUASHED UNTIL THEY SOUND LIKE SYNTHS"),
        accent: ui::OLIVE,
        size: (W, H),
        items: vec![
            Item::Knob { p: OUTPUT, x: 190.0, y: 170.0, r: 36.0, label: "OUTPUT", tip: "Output level.", active: None },
            Item::Knob { p: SENS, x: 450.0, y: 170.0, r: 36.0, label: "SENSITIVITY", tip: "How hard it squashes (and how long notes sustain).", active: None },
            Item::Meter { m: 0, x: 245.0, y: 270.0, w: 150.0, h: 100.0, kind: MeterKind::GainReduction, caption: "SQUASH" },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Dyna Comp", class_id: *b"TImpDynaCompPQD1", vendor: "PEQUOD Tame Impala",
    categories: [Fx, Dynamics]
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
    fn evens_out_level() {
        let p = defaults();
        let a = db(rms(&go(&p, &sine(330.0, 0.02, 48000))[24000..]));
        let b = db(rms(&go(&p, &sine(330.0, 0.4, 48000))[24000..]));
        println!("input +26 dB -> output {:.1} dB", b - a);
        assert!(b - a < 8.0);
    }
    #[test]
    fn sensitivity_squashes_more() {
        let gap = |s: f32| {
            let p = with(defaults(), &[(SENS, s)]);
            db(rms(&go(&p, &sine(330.0, 0.4, 48000))[24000..])) - db(rms(&go(&p, &sine(330.0, 0.02, 48000))[24000..]))
        };
        println!("spread: sens 0.2 {:.1}, sens 1 {:.1}", gap(0.2), gap(1.0));
        assert!(gap(1.0) < gap(0.2));
    }

}
