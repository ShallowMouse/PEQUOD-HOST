//! MXR Flanger/Doubler: MXR's rack unit (model 126, 1979), bucket-brigade flanging and doubling in a 1U
//! box, on many late-70s and 80s records. FLANGE or DOUBLE mode (short and long delays), MANUAL (where the
//! sweep sits), SWEEP RANGE, SPEED, REGENERATION (with its polarity switch) and MIX, as on the front. BBD
//! bandwidth and companding noise. After MXR's manual; estimated.

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
    #[id = "mode"]
    pub mode: FloatParam,
    #[id = "manual"]
    pub manual: FloatParam,
    #[id = "range"]
    pub range: FloatParam,
    #[id = "speed"]
    pub speed: FloatParam,
    #[id = "regen"]
    pub regen: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            mode: FloatParam::new("Mode", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            manual: pct("Manual", 0.4),
            range: pct("Sweep Range", 0.6),
            speed: rate("Speed", 0.3, 0.05, 10.0),
            regen: FloatParam::new("Regeneration", 0.5, FloatRange::Linear { min: -0.95, max: 0.95 }).with_step_size(0.01),
            mix: pct("Mix", 0.5),
        }
    }
}

pub const MODE: usize = 0;
pub const MANUAL: usize = 1;
pub const RANGE: usize = 2;
pub const SPEED: usize = 3;
pub const REGEN: usize = 4;
pub const MIX: usize = 5;
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
            self.mode.as_ptr(),
            self.manual.as_ptr(),
            self.range.as_ptr(),
            self.speed.as_ptr(),
            self.regen.as_ptr(),
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
    bbd: [Bbd; 2],
    lfo: Lfo,
    fb: [f64; 2],
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut b = [Bbd::new(fs, 80.0), Bbd::new(fs, 80.0)];
        for x in b.iter_mut() { x.set_bandwidth(9000.0); }
        Self { fs, v, bbd: b, lfo: Lfo::default(), fb: [0.0; 2] }
    }
    fn reset(&mut self) {
        let fs = self.fs;
        self.bbd = [Bbd::new(fs, 80.0), Bbd::new(fs, 80.0)];
        for x in self.bbd.iter_mut() { x.set_bandwidth(9000.0); }
        self.fb = [0.0; 2];
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nc = chans.len();
        let double = self.v[MODE] > 0.5;
        // FLANGE: 0.5 to 10 ms; DOUBLE: 15 to 60 ms
        let (lo, hi) = if double { (15.0, 60.0) } else { (0.5, 10.0) };
        let centre = lo + (hi - lo) * self.v[MANUAL] as f64;
        let depth = (hi - lo) * 0.5 * self.v[RANGE] as f64;
        let regen = self.v[REGEN] as f64;
        let mix = self.v[MIX] as f64;
        for k in 0..n {
            let s = self.lfo.next(self.fs, self.v[SPEED] as f64);
            for c in 0..nc {
                let j = c.min(1);
                let x = chans[c][k] as f64;
                // the two channels sweep in opposite directions, for width
                let ms = (centre + depth * if j == 0 { s } else { -s }).clamp(0.3, 75.0);
                let y = self.bbd[j].process(x + self.fb[j] * regen, ms * 0.001 * self.fs);
                self.fb[j] = y;
                chans[c][k] = (x * (1.0 - mix * 0.5) + y * mix) as f32;
            }
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Jet Flange", values: &[(MODE, 0.0), (MANUAL, 0.3), (RANGE, 0.8), (SPEED, 0.15), (REGEN, 0.75)] },
        Preset { name: "Gentle Flange", values: &[(MODE, 0.0), (MANUAL, 0.4), (RANGE, 0.5), (SPEED, 0.4), (REGEN, 0.3)] },
        Preset { name: "Negative Flange", values: &[(MODE, 0.0), (REGEN, -0.7), (SPEED, 0.2), (RANGE, 0.7)] },
        Preset { name: "Doubler", values: &[(MODE, 1.0), (MANUAL, 0.3), (RANGE, 0.15), (SPEED, 0.5), (REGEN, 0.0), (MIX, 0.5)] },
        Preset { name: "Slapback Double", values: &[(MODE, 1.0), (MANUAL, 0.9), (RANGE, 0.05), (REGEN, 0.0), (MIX, 0.45)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "FLANGER/DOUBLER",
        subtitle: ("MXR MODEL 126, 1979", "BUCKET-BRIGADE FLANGE OR DOUBLE"),
        accent: ui::PINE,
        size: (W, H),
        items: vec![
            Item::Switch { p: MODE, x: 80.0, y: 200.0, labels: ["FLANGE", "DOUBLE"], tip: "Short delays for flanging, long ones for doubling." },
            Item::Knob { p: MANUAL, x: 190.0, y: 200.0, r: 30.0, label: "MANUAL", tip: "Where the sweep sits.", active: None },
            Item::Knob { p: RANGE, x: 290.0, y: 200.0, r: 30.0, label: "SWEEP RANGE", tip: "How far it sweeps.", active: None },
            Item::Knob { p: SPEED, x: 390.0, y: 200.0, r: 30.0, label: "SPEED", tip: "Sweep rate.", active: None },
            Item::Knob { p: REGEN, x: 490.0, y: 200.0, r: 30.0, label: "REGEN", tip: "Feedback; below the middle, inverted.", active: None },
            Item::Knob { p: MIX, x: 580.0, y: 200.0, r: 26.0, label: "MIX", tip: "Dry vs. effect.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Flanger/Doubler", class_id: *b"HWMXRFlangeDblPQ", vendor: "PEQUOD Hardware",
    categories: [Fx, Modulation]
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
        let p = with(defaults(), &[]);
        let y = go(&p, &sine(440.0, 0.2, 48000));
        let l = db(rms(&y[24000..]));
        println!("level {:.1} dB", l);
        assert!(l > -40.0 && y.iter().all(|v| v.is_finite()));
    }

}
