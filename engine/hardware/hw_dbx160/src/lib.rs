//! dbx 160: David Blackmer's VCA compressor (1976), true-RMS detection and the punch on a generation of
//! kick drums and bass. THRESHOLD, COMPRESSION (1:1 up to infinity:1) and OUTPUT GAIN, as on the front,
//! plus the 160X's OVER EASY soft knee. Program-dependent timing from dbx's own figures: attack about
//! 15 ms for 10 dB over, 5 ms for 20 dB, 3 ms for 30 dB; release at a constant 120 dB a second. The VCA is
//! clean; the detector does the work.

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

/// The level the side chain sees, in dB: peak or RMS of the louder channel.
#[allow(dead_code)]
#[derive(Clone, Copy)]
struct Detector {
    ms: f64,
    rms: f64,
}

#[allow(dead_code)]
impl Detector {
    fn new() -> Self {
        Self { ms: 0.0, rms: 0.0 }
    }
    #[inline]
    fn peak_db(&self, x: f64) -> f64 {
        gain_to_db(x.abs().max(1e-9))
    }
    /// RMS over about `win` (a one-pole coefficient).
    #[inline]
    fn rms_db(&mut self, x: f64, win: f64) -> f64 {
        self.ms = x * x + win * (self.ms - x * x);
        self.rms = self.ms.max(1e-18).sqrt();
        gain_to_db(self.rms)
    }
}

/// Gain reduction (dB) smoothed with attack and release.
#[allow(dead_code)]
#[inline]
fn ballistics(state: &mut f64, target: f64, att: f64, rel: f64) -> f64 {
    let c = if target > *state { att } else { rel };
    *state = target + c * (*state - target);
    *state
}

pub const W: u32 = 640;
pub const H: u32 = 432;

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "threshold"]
    pub threshold: FloatParam,
    #[id = "ratio"]
    pub ratio: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
    #[id = "overeasy"]
    pub overeasy: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            threshold: db("Threshold", -20.0, -40.0, 10.0),
            ratio: pct("Compression", 0.5),
            output: db("Output Gain", 0.0, -20.0, 20.0),
            overeasy: FloatParam::new("Over Easy", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
        }
    }
}

pub const THRESHOLD: usize = 0;
pub const RATIO: usize = 1;
pub const OUTPUT: usize = 2;
pub const OVEREASY: usize = 3;
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
            self.threshold.as_ptr(),
            self.ratio.as_ptr(),
            self.output.as_ptr(),
            self.overeasy.as_ptr(),
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
    det: Detector,
    gr: f64,
    out: Smooth,
}

/// The COMPRESSION knob: 1:1 at the bottom, infinity:1 at the top.
fn ratio_of(v: f64) -> f64 {
    1.0 / (1.0 - v.clamp(0.0, 0.985))
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, det: Detector::new(), gr: 0.0, out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.det = Detector::new();
        self.gr = 0.0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let thr = self.v[THRESHOLD] as f64;
        let ratio = ratio_of(self.v[RATIO] as f64);
        let knee = if self.v[OVEREASY] > 0.5 { 14.0 } else { 0.0 };
        let win = coeff(self.fs, 8.0);
        let rel_step = 120.0 / self.fs;          // dB per sample
        let mut most = 0.0f64;
        for k in 0..n {
            let o = self.out.next();
            // the true-RMS detector sees both channels together
            let s = chans.iter().map(|c| (c[k] as f64).powi(2)).sum::<f64>() / chans.len().max(1) as f64;
            let lvl = self.det.rms_db(s.sqrt(), win) + 3.0;
            let target = comp_gr(lvl, thr, ratio, knee);
            if target > self.gr {
                // the further over, the faster: 15 ms at 10 dB, 5 ms at 20, 3 ms at 30
                let over = (target - self.gr).max(1.0);
                let t = (150.0 / over).clamp(2.5, 15.0);
                self.gr = target + coeff(self.fs, t) * (self.gr - target);
            } else {
                self.gr = (self.gr - rel_step).max(target);
            }
            most = most.max(self.gr);
            let g = db_to_gain(-self.gr) * o;
            for ch in chans.iter_mut() {
                ch[k] = (ch[k] as f64 * g) as f32;
            }
        }
        meters[0] = most as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Kick Punch", values: &[(THRESHOLD, -22.0), (RATIO, 0.75), (OUTPUT, 4.0)] },
        Preset { name: "Bass, Locked", values: &[(THRESHOLD, -24.0), (RATIO, 0.67), (OUTPUT, 5.0), (OVEREASY, 1.0)] },
        Preset { name: "Snare Smack", values: &[(THRESHOLD, -18.0), (RATIO, 0.8), (OUTPUT, 3.0)] },
        Preset { name: "Vocal, Over Easy", values: &[(THRESHOLD, -26.0), (RATIO, 0.6), (OVEREASY, 1.0), (OUTPUT, 4.0)] },
        Preset { name: "Brick Wall", values: &[(THRESHOLD, -12.0), (RATIO, 1.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "dbx 160",
        subtitle: ("VCA COMPRESSOR/LIMITER, 1976", "TRUE RMS DETECTION, OVER EASY"),
        accent: ui::GRAPHITE,
        size: (W, H),
        items: vec![
            Item::Knob { p: THRESHOLD, x: 110.0, y: 300.0, r: 36.0, label: "THRESHOLD", tip: "Where compression starts.", active: None },
            Item::Knob { p: RATIO, x: 250.0, y: 300.0, r: 36.0, label: "COMPRESSION", tip: "1:1 to infinity:1.", active: None },
            Item::Knob { p: OUTPUT, x: 390.0, y: 300.0, r: 36.0, label: "OUTPUT GAIN", tip: "Make-up gain.", active: None },
            Item::Switch { p: OVEREASY, x: 530.0, y: 300.0, labels: ["HARD", "OVER EASY"], tip: "dbx's soft knee: compression comes in gradually around the threshold." },
            Item::Meter { m: 0, x: 235.0, y: 90.0, w: 170.0, h: 120.0, kind: MeterKind::GainReduction, caption: "GAIN CHANGE" },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "dbx 160", class_id: *b"HWdbx160CompPQD1", vendor: "PEQUOD Hardware",
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

    /// Louder in comes out much less louder: it compresses.
    fn spread(set: &[(usize, f32)]) -> f64 {
        let p = with(defaults(), set);
        let a = db(rms(&go(&p, &sine(1000.0, 0.03, 48000))[24000..]));
        let b = db(rms(&go(&p, &sine(1000.0, 0.9, 48000))[24000..]));
        b - a
    }
    #[test]
    fn compresses() {
        let s = spread(&[(RATIO, 0.8)]);
        println!("+29.5 dB in -> {:.1} dB out", s);
        assert!(s < 24.0, "{s}");
    }

}
