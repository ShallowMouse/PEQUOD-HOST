//! Analog Delay: after the MXR M-118 Analog Delay heard at the start of Down to the Waterline on the first Dire Straits album: a bucket-brigade delay of up to 300 ms with dark, softening repeats.

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
pub const H: u32 = 320;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "delay"]
    pub delay: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "regen"]
    pub regen: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            delay: ms("Delay", 280.0, 20.0, 300.0),
            mix: pct("Mix", 0.35),
            regen: pct("Regen", 0.35),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const DELAY: usize = 0;
pub const MIX: usize = 1;
pub const REGEN: usize = 2;
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
            self.delay.as_ptr(),
            self.mix.as_ptr(),
            self.regen.as_ptr(),
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
    bbd: Vec<Bbd>,
    fb: Vec<f64>,
    w: Wander,
    t: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let bbd = (0..ch.max(2)).map(|_| {
            let mut b = Bbd::new(fs, 320.0);
            b.set_bandwidth(3200.0);
            b
        }).collect();
        Self { fs, v, bbd, fb: vec![0.0; ch.max(2)], w: Wander::new(fs, 0.5, 3), t: Smooth::new(fs, 150.0, 0.28 * fs), out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.bbd.iter_mut().for_each(|b| b.reset());
        self.fb.iter_mut().for_each(|f| *f = 0.0);
        self.t.snap(self.v[DELAY] as f64 * 0.001 * self.fs);
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.t.set(v[DELAY] as f64 * 0.001 * self.fs);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let mix = self.v[MIX] as f64;
        let rg = self.v[REGEN] as f64 * 0.95;
        let last = self.bbd.len() - 1;
        for k in 0..n {
            let d = self.t.next() * (1.0 + 0.0015 * self.w.next());
            let o = self.out.next();
            for (ci, c) in chans.iter_mut().enumerate() {
                let j = ci.min(last);
                let x = c[k] as f64;
                let y = self.bbd[j].process(x + self.fb[j] * rg, d);
                self.fb[j] = y;
                c[k] = ((x + y * mix * 1.4) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Down to the Waterline", values: &[] },
        Preset { name: "Slapback", values: &[(DELAY, 90.0), (REGEN, 0.1), (MIX, 0.3)] },
        Preset { name: "Thickening Double", values: &[(DELAY, 35.0), (REGEN, 0.0), (MIX, 0.4)] },
        Preset { name: "Long Dark Trails", values: &[(DELAY, 300.0), (REGEN, 0.6), (MIX, 0.4)] },
        Preset { name: "Oscillation Edge", values: &[(DELAY, 220.0), (REGEN, 1.0), (MIX, 0.35)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "ANALOG DELAY",
        subtitle: ("AFTER THE MXR M-118", "BUCKET-BRIGADE · 300 ms"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![

            Item::Knob { p: DELAY, x: 110.0, y: 180.0, r: 40.0, label: "DELAY", tip: "Delay time, up to 300 ms.", active: None },
            Item::Knob { p: MIX, x: 260.0, y: 180.0, r: 40.0, label: "MIX", tip: "Echo level.", active: None },
            Item::Knob { p: REGEN, x: 410.0, y: 180.0, r: 40.0, label: "REGEN", tip: "Repeats.", active: None },
            Item::Knob { p: OUTPUT, x: 550.0, y: 180.0, r: 26.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Analog Delay", class_id: *b"DSMXRAnalogDly01", vendor: "PEQUOD Dire Straits",
    categories: [Fx, Delay]
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

    fn impulse(p: &[f32], n: usize) -> (Vec<f32>, Vec<f32>) {
        let mut x = vec![0.0f32; n];
        x[100] = 0.5;
        let mut input = vec![x.clone(), x];
        run(p, &mut input);
        let r = input.pop().unwrap();
        (input.pop().unwrap(), r)
    }
    fn peak_after(y: &[f32], from: usize, to: usize) -> (usize, f32) {
        let mut best = (from, 0.0f32);
        for k in from..to.min(y.len()) {
            if y[k].abs() > best.1 {
                best = (k, y[k].abs());
            }
        }
        best
    }

    #[test]
    fn echo_at_time() {
        let p = with(defaults(), &[(DELAY, 200.0), (REGEN, 0.0), (MIX, 1.0)]);
        let (l, _) = impulse(&p, 24000);
        let (k, v) = peak_after(&l, 2000, 20000);
        println!("echo at {k} (expect {}) level {v:.3}", 9600 + 100);
        assert!((k as i64 - 9700).abs() < 80 && v > 0.02);
    }

}
