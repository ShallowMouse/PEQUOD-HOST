//! Dual Delay: two delays in one, in the spirit of the Eventide TimeFactor Chris Cheney presets per song: A for a rockabilly slapback, B for a longer echo, run side by side or one into the other, each with its own feedback, and a little tape-style modulation.

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
pub const H: u32 = 380;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Route { #[name = "Parallel"] Parallel, #[name = "Series (A into B)"] Series }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "timea"]
    pub time_a: FloatParam,
    #[id = "fba"]
    pub fba: FloatParam,
    #[id = "leva"]
    pub leva: FloatParam,
    #[id = "timeb"]
    pub time_b: FloatParam,
    #[id = "fbb"]
    pub fbb: FloatParam,
    #[id = "levb"]
    pub levb: FloatParam,
    #[id = "route"]
    pub route: EnumParam<Route>,
    #[id = "mod_amt"]
    pub mod_amt: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            time_a: ms("Time A", 110.0, 20.0, 1500.0),
            fba: pct("Feedback A", 0.1),
            leva: pct("Level A", 0.5),
            time_b: ms("Time B", 420.0, 20.0, 1500.0),
            fbb: pct("Feedback B", 0.35),
            levb: pct("Level B", 0.3),
            route: EnumParam::new("Routing", Route::Parallel),
            mod_amt: pct("Mod", 0.15),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const TIMEA: usize = 0;
pub const FBA: usize = 1;
pub const LEVA: usize = 2;
pub const TIMEB: usize = 3;
pub const FBB: usize = 4;
pub const LEVB: usize = 5;
pub const ROUTE: usize = 6;
pub const MOD: usize = 7;
pub const OUTPUT: usize = 8;
pub const NPARAMS: usize = 9;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.time_a.as_ptr(),
            self.fba.as_ptr(),
            self.leva.as_ptr(),
            self.time_b.as_ptr(),
            self.fbb.as_ptr(),
            self.levb.as_ptr(),
            self.route.as_ptr(),
            self.mod_amt.as_ptr(),
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
    a: [DelayLine; 2],
    b: [DelayLine; 2],
    damp: [OnePole; 4],
    w: Wander,
    ta: Smooth,
    tb: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mk = || DelayLine::new((1.6 * fs) as usize);
        let mut damp = [OnePole::default(); 4];
        damp.iter_mut().for_each(|d| d.set_lowpass(fs, 6000.0));
        Self { fs, v, a: [mk(), mk()], b: [mk(), mk()], damp, w: Wander::new(fs, 0.5, 9), ta: Smooth::new(fs, 150.0, 0.11 * fs), tb: Smooth::new(fs, 150.0, 0.42 * fs), out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.a.iter_mut().chain(self.b.iter_mut()).for_each(|l| l.reset());
        self.ta.snap(self.v[TIMEA] as f64 * 0.001 * self.fs);
        self.tb.snap(self.v[TIMEB] as f64 * 0.001 * self.fs);
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.ta.set(v[TIMEA] as f64 * 0.001 * self.fs);
        self.tb.set(v[TIMEB] as f64 * 0.001 * self.fs);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let (fa, fb) = (self.v[FBA] as f64 * 0.95, self.v[FBB] as f64 * 0.95);
        let (la, lb) = (self.v[LEVA] as f64, self.v[LEVB] as f64);
        let series = self.v[ROUTE] > 0.5;
        let md = self.v[MOD] as f64 * 0.003;
        let nch = chans.len().min(2);
        for k in 0..n {
            let m = 1.0 + md * self.w.next();
            let da = (self.ta.next() * m).max(1.0);
            let db_ = (self.tb.next() * m).max(1.0);
            let o = self.out.next();
            for ci in 0..nch {
                let x = chans[ci][k] as f64;
                let ya = self.damp[ci].lp(self.a[ci].read(da));
                let yb = self.damp[2 + ci].lp(self.b[ci].read(db_));
                self.a[ci].write(x + fa * ya);
                let into_b = if series { ya } else { x };
                self.b[ci].write(into_b + fb * yb);
                let wet = if series { yb * lb + ya * la } else { ya * la + yb * lb };
                chans[ci][k] = ((x + wet) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Slap + Echo", values: &[] },
        Preset { name: "Rockabilly Slap Only", values: &[(LEVB, 0.0), (LEVA, 0.6), (TIMEA, 95.0)] },
        Preset { name: "Dotted Lead (series)", values: &[(ROUTE, 1.0), (TIMEA, 90.0), (TIMEB, 375.0), (FBB, 0.45)] },
        Preset { name: "Big Stadium Echo", values: &[(TIMEA, 125.0), (TIMEB, 600.0), (FBB, 0.55), (LEVB, 0.35), (MOD, 0.3)] },
        Preset { name: "Double Slap", values: &[(TIMEA, 80.0), (TIMEB, 160.0), (FBA, 0.0), (FBB, 0.0), (LEVB, 0.35)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "DUAL DELAY",
        subtitle: ("TWO DELAYS · SLAPBACK AND ECHO", "PARALLEL OR SERIES"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![

            Item::Knob { p: TIMEA, x: 70.0, y: 150.0, r: 28.0, label: "TIME A", tip: "Delay A time.", active: None },
            Item::Knob { p: FBA, x: 160.0, y: 150.0, r: 24.0, label: "FB A", tip: "Delay A feedback.", active: None },
            Item::Knob { p: LEVA, x: 240.0, y: 150.0, r: 24.0, label: "LEVEL A", tip: "Delay A level.", active: None },
            Item::Knob { p: TIMEB, x: 400.0, y: 150.0, r: 28.0, label: "TIME B", tip: "Delay B time.", active: None },
            Item::Knob { p: FBB, x: 490.0, y: 150.0, r: 24.0, label: "FB B", tip: "Delay B feedback.", active: None },
            Item::Knob { p: LEVB, x: 570.0, y: 150.0, r: 24.0, label: "LEVEL B", tip: "Delay B level.", active: None },
            Item::Divider { y: 215.0 },
            Item::Selector { p: ROUTE, x: 140.0, y: 280.0, label: "ROUTING", positions: &["PARALLEL", "SERIES"], tip: "Side by side, or A into B." },
            Item::Knob { p: MOD, x: 320.0, y: 280.0, r: 24.0, label: "MOD", tip: "Tape-style wobble on both.", active: None },
            Item::Knob { p: OUTPUT, x: 490.0, y: 280.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Dual Delay", class_id: *b"LEDualDelayTF001", vendor: "PEQUOD The Living End",
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
    fn both_taps() {
        let p = with(defaults(), &[(TIMEA, 100.0), (TIMEB, 300.0), (FBA, 0.0), (FBB, 0.0), (MOD, 0.0)]);
        let (l, _) = impulse(&p, 24000);
        let (ka, _) = peak_after(&l, 2000, 9000);
        let (kb, _) = peak_after(&l, 12000, 20000);
        println!("A at {ka} (4900), B at {kb} (14500)");
        assert!((ka as i64 - 4900).abs() < 30 && (kb as i64 - 14500).abs() < 30);
    }

}
