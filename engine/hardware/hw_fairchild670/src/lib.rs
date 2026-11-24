//! Fairchild 670: the stereo variable-mu valve limiter (Rein Narma's design, 1959), on every Abbey Road
//! mix from the Beatles on and still the most sought-after compressor there is. Twenty valves; the
//! 6386 remote-cutoff triodes run push-pull and are biased further down as the level rises, so the ratio
//! steepens the harder it's hit: gentle on quiet passages, a limiter on peaks. Six TIME CONSTANT
//! positions (attack 0.2 or 0.4 ms; release 0.3 s to program-dependent 25 s, from the manual), two
//! channels that work LEFT/RIGHT or LAT/VERT (mid and side, for the cutting lathe), and the big
//! transformers. Curves after the published manual and measurements; the details are estimated.

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

/// Transformer iron: the lows saturate first (a little thickening, then soft limiting),
/// the top rolls gently away.
#[allow(dead_code)]
#[derive(Clone)]
struct Iron {
    lo: OnePole,
    top: OnePole,
    drive: f64,
}

#[allow(dead_code)]
impl Iron {
    fn new(fs: f64, corner: f64, drive: f64, top: f64) -> Self {
        let mut lo = OnePole::default();
        lo.set_lowpass(fs, corner);
        let mut t = OnePole::default();
        t.set_lowpass(fs, top.min(fs * 0.45));
        Self { lo, top: t, drive }
    }
    fn reset(&mut self) {
        self.lo.reset();
        self.top.reset();
    }
    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let l = self.lo.lp(x);
        let d = self.drive;
        let y = x - l + (l * d).tanh() / d;
        self.top.lp(y)
    }
}

/// A valve gain stage: mostly second harmonic, rising with level.
#[allow(dead_code)]
#[inline]
fn valve(x: f64, warmth: f64) -> f64 {
    let y = x + warmth * 0.18 * x * x - warmth * 0.04 * x * x * x;
    (y * (1.0 + warmth * 0.3)).tanh() / (1.0 + warmth * 0.3)
}

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
    #[id = "input"]
    pub input: FloatParam,
    #[id = "threshold"]
    pub threshold: FloatParam,
    #[id = "time"]
    pub time: FloatParam,
    #[id = "mode"]
    pub mode: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            input: db("Input Gain", 0.0, -12.0, 18.0),
            threshold: pct("Threshold", 0.5),
            time: FloatParam::new("Time Constant", 1.0, FloatRange::Linear { min: 0.0, max: 5.0 }).with_step_size(1.0),
            mode: FloatParam::new("AGC", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            output: db("Output", 0.0, -18.0, 18.0),
        }
    }
}

pub const INPUT: usize = 0;
pub const THRESHOLD: usize = 1;
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
            self.input.as_ptr(),
            self.threshold.as_ptr(),
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

/// (attack ms, fast release s, slow release s, share of the slow one) for positions 1-6
fn time_constant(pos: usize) -> (f64, f64, f64, f64) {
    match pos {
        0 => (0.2, 0.3, 0.3, 0.0),
        1 => (0.2, 0.8, 0.8, 0.0),
        2 => (0.4, 2.0, 2.0, 0.0),
        3 => (0.4, 5.0, 5.0, 0.0),
        4 => (0.2, 2.0, 10.0, 0.7),     // program: 2 s for peaks, 10 s for many
        _ => (0.4, 0.3, 25.0, 0.8),     // program: 0.3 s, 10 s, 25 s for a steady high level
    }
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    fast: [f64; 2],
    slow: [f64; 2],
    iron_in: Vec<Iron>,
    iron_out: Vec<Iron>,
    inp: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 2;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let n = ch.max(2);
        Self {
            fs, v, fast: [0.0; 2], slow: [0.0; 2],
            iron_in: vec![Iron::new(fs, 60.0, 1.6, 22000.0); n],
            iron_out: vec![Iron::new(fs, 50.0, 2.0, 19000.0); n],
            inp: Smooth::new(fs, 20.0, 1.0), out: Smooth::new(fs, 20.0, 1.0),
        }
    }
    fn reset(&mut self) {
        self.fast = [0.0; 2];
        self.slow = [0.0; 2];
        self.iron_in.iter_mut().chain(self.iron_out.iter_mut()).for_each(|i| i.reset());
        self.inp.snap(db_to_gain(self.v[INPUT] as f64));
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.inp.set(db_to_gain(v[INPUT] as f64));
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nc = chans.len();
        let (att, rf, rs, share) = time_constant(self.v[TIME].round() as usize);
        let ca = coeff(self.fs, att);
        let cf = coeff(self.fs, rf * 1000.0 / 3.0);
        let cs = coeff(self.fs, rs * 1000.0 / 3.0);
        // the threshold knob sets the bias: fully up, the 6386s start pulling down early
        let thr = -6.0 - 30.0 * self.v[THRESHOLD] as f64;
        let latvert = self.v[MODE] > 0.5 && nc >= 2;
        let mut most = [0.0f64; 2];
        for k in 0..n {
            let gi = self.inp.next();
            let go = self.out.next();
            let mut x = [0.0f64; 2];
            for c in 0..2 {
                x[c] = self.iron_in[c].process(chans[c.min(nc - 1)][k] as f64 * gi);
            }
            if latvert {
                x = [(x[0] + x[1]) * 0.5, (x[0] - x[1]) * 0.5];
            }
            for c in 0..2 {
                let lvl = gain_to_db(x[c].abs().max(1e-9));
                // variable-mu: the further over, the steeper the slope (about 2:1 gently, 20:1 hard)
                let over = (lvl - thr).max(0.0);
                let ratio = 1.6 + over * 0.9;
                let target = comp_gr(lvl, thr, ratio.min(25.0), 14.0);
                ballistics(&mut self.fast[c], target, ca, cf);
                ballistics(&mut self.slow[c], target * share, ca, cs);
                let gr = self.fast[c].max(self.slow[c]);
                most[c] = most[c].max(gr);
                // the valves colour more the harder they're pulled down
                x[c] = valve(x[c] * db_to_gain(-gr), 0.15 + gr * 0.02);
            }
            if latvert {
                x = [x[0] + x[1], x[0] - x[1]];
            }
            for c in 0..nc {
                let j = c.min(1);
                chans[c][k] = (self.iron_out[j].process(x[j]) * go) as f32;
            }
        }
        meters[0] = most[0] as f32;
        meters[1] = most[1] as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Abbey Road Vocal (pos 2)", values: &[(THRESHOLD, 0.55), (TIME, 1.0)] },
        Preset { name: "Mix Bus, Barely (pos 5)", values: &[(THRESHOLD, 0.3), (TIME, 4.0), (OUTPUT, 1.0)] },
        Preset { name: "Drum Room Pumping (pos 1)", values: &[(INPUT, 8.0), (THRESHOLD, 0.85), (TIME, 0.0)] },
        Preset { name: "Lathe: Lat/Vert (pos 6)", values: &[(MODE, 1.0), (THRESHOLD, 0.45), (TIME, 5.0)] },
        Preset { name: "Bass, Held Down (pos 4)", values: &[(THRESHOLD, 0.7), (TIME, 3.0), (OUTPUT, 3.0)] },
        Preset { name: "Just the Valves", values: &[(THRESHOLD, 0.0), (INPUT, 6.0), (OUTPUT, -6.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "FAIRCHILD 670",
        subtitle: ("STEREO VARIABLE-MU LIMITER, 1959", "TWENTY VALVES, SIX TIME CONSTANTS, LEFT/RIGHT OR LAT/VERT"),
        accent: ui::GRAPHITE,
        size: (W, H),
        items: vec![
            Item::Knob { p: INPUT, x: 80.0, y: 165.0, r: 34.0, label: "INPUT GAIN", tip: "Into the limiter (both channels).", active: None },
            Item::Meter { m: 0, x: 150.0, y: 105.0, w: 150.0, h: 108.0, kind: MeterKind::GainReduction, caption: "CHANNEL 1" },
            Item::Meter { m: 1, x: 340.0, y: 105.0, w: 150.0, h: 108.0, kind: MeterKind::GainReduction, caption: "CHANNEL 2" },
            Item::Knob { p: OUTPUT, x: 560.0, y: 165.0, r: 34.0, label: "OUTPUT", tip: "Output level.", active: None },
            Item::Divider { y: 262.0 },
            Item::Knob { p: THRESHOLD, x: 130.0, y: 335.0, r: 32.0, label: "THRESHOLD", tip: "The bias on the 6386s: further up, they start pulling the level down sooner.", active: None },
            Item::Selector { p: TIME, x: 320.0, y: 335.0, label: "TIME CONSTANT", positions: &["1", "2", "3", "4", "5", "6"],
                tip: "1: 0.2 ms / 0.3 s. 2: 0.2 ms / 0.8 s. 3: 0.4 ms / 2 s. 4: 0.4 ms / 5 s. 5: program, 2 s to 10 s. 6: program, 0.3 s to 25 s." },
            Item::Switch { p: MODE, x: 510.0, y: 335.0, labels: ["LEFT/RIGHT", "LAT/VERT"], tip: "Two channels on left and right, or on mid (lateral) and side (vertical), as for cutting a lacquer." },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Fairchild 670", class_id: *b"HWFairchild670PQ", vendor: "PEQUOD Hardware",
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
        let s = spread(&[(THRESHOLD, 0.8)]);
        println!("+29.5 dB in -> {:.1} dB out", s);
        assert!(s < 24.0, "{s}");
    }

}
