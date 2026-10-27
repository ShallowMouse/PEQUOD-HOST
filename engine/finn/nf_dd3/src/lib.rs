//! DD-3: after the Boss DD-3 Digital Delay, on Neil Finn's board (and Kele Okereke's): three time ranges up to 800 ms, clean 12-bit repeats with a slightly rolled-off top, E.LEVEL for the echo and F.BACK for the repeats.

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
use vintage::fx::quantise;

pub const W: u32 = 640;
pub const H: u32 = 320;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Mode { #[name = "S (12-50 ms)"] S, #[name = "M (50-200 ms)"] M, #[name = "L (200-800 ms)"] L }
pub const RANGES: [(f64, f64); 3] = [(12.5, 50.0), (50.0, 200.0), (200.0, 800.0)];


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "level"]
    pub level: FloatParam,
    #[id = "feedback"]
    pub feedback: FloatParam,
    #[id = "time"]
    pub time: FloatParam,
    #[id = "mode"]
    pub mode: EnumParam<Mode>,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            level: pct("E. Level", 0.4),
            feedback: pct("F. Back", 0.3),
            time: pct("D. Time", 0.5),
            mode: EnumParam::new("Mode", Mode::L),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const LEVEL: usize = 0;
pub const FEEDBACK: usize = 1;
pub const TIME: usize = 2;
pub const MODE: usize = 3;
pub const OUTPUT: usize = 4;
pub const NPARAMS: usize = 5;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.level.as_ptr(),
            self.feedback.as_ptr(),
            self.time.as_ptr(),
            self.mode.as_ptr(),
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

pub fn delay_ms(mode: usize, t: f64) -> f64 {
    let (a, b) = RANGES[mode.min(2)];
    a + (b - a) * t
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    line: Vec<DelayLine>,
    lp: Vec<[Biquad; 2]>,
    t: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut lp = vec![[Biquad::identity(); 2]; ch.max(2)];
        lp.iter_mut().for_each(|b| {
            b[0].set_lowpass(fs, 7000.0, 0.54);
            b[1].set_lowpass(fs, 7000.0, 1.31);
        });
        Self { fs, v, line: (0..ch.max(2)).map(|_| DelayLine::new((0.85 * fs) as usize)).collect(), lp, t: Smooth::new(fs, 80.0, 0.5 * fs), out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.line.iter_mut().for_each(|l| l.reset());
        self.lp.iter_mut().for_each(|b| b.iter_mut().for_each(|q| q.reset()));
        self.t.snap(delay_ms(self.v[MODE].round() as usize, self.v[TIME] as f64) * 0.001 * self.fs);
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.t.set(delay_ms(v[MODE].round() as usize, v[TIME] as f64) * 0.001 * self.fs);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let lev = self.v[LEVEL] as f64 * 1.6;
        let fb = self.v[FEEDBACK] as f64 * 0.95;
        let last = self.line.len() - 1;
        for k in 0..n {
            let d = self.t.next().max(1.0);
            let o = self.out.next();
            for (ci, c) in chans.iter_mut().enumerate() {
                let j = ci.min(last);
                let x = c[k] as f64;
                let y = cascade(&mut self.lp[j], self.line[j].read(d));
                self.line[j].write(quantise((x + fb * y).clamp(-1.0, 1.0), 12));
                c[k] = ((x + lev * y) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Neil's Echo (L)", values: &[] },
        Preset { name: "Slapback (M)", values: &[(MODE, 1.0), (TIME, 0.5), (FEEDBACK, 0.1)] },
        Preset { name: "Doubling (S)", values: &[(MODE, 0.0), (TIME, 0.6), (FEEDBACK, 0.0), (LEVEL, 0.5)] },
        Preset { name: "Long Ambient Repeats", values: &[(TIME, 0.8), (FEEDBACK, 0.6), (LEVEL, 0.35)] },
        Preset { name: "Kele's DD-3", values: &[(TIME, 0.35), (FEEDBACK, 0.4)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "DD-3",
        subtitle: ("AFTER THE BOSS DD-3 DIGITAL DELAY", "S · M · L · UP TO 800 ms"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![

            Item::Knob { p: LEVEL, x: 90.0, y: 180.0, r: 36.0, label: "E.LEVEL", tip: "Echo level.", active: None },
            Item::Knob { p: FEEDBACK, x: 220.0, y: 180.0, r: 36.0, label: "F.BACK", tip: "Repeats.", active: None },
            Item::Knob { p: TIME, x: 350.0, y: 180.0, r: 36.0, label: "D.TIME", tip: "Delay time within the range.", active: None },
            Item::Selector { p: MODE, x: 470.0, y: 180.0, label: "MODE", positions: &["S", "M", "L"], tip: "Time range: S 12-50 ms, M 50-200 ms, L 200-800 ms." },
            Item::Knob { p: OUTPUT, x: 575.0, y: 180.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "DD-3", class_id: *b"NFDD3DigitalDly1", vendor: "PEQUOD Finn Brothers",
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
        let p = with(defaults(), &[(MODE, 2.0), (TIME, 0.0), (FEEDBACK, 0.0), (LEVEL, 0.6)]);
        let (l, _) = impulse(&p, 24000);
        let (k, v) = peak_after(&l, 2000, 20000);
        println!("echo at {k} (expect {}) level {v:.3}", 9600 + 100);
        assert!((k as i64 - 9700).abs() < 30 && v > 0.05);
    }

}
