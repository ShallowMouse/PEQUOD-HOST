//! API 2500: API's stereo bus compressor (designed by Paul Wolff, 2000s, after API's 525 of 1968), all
//! discrete 2520 op-amps and transformers. THRESHOLD, ATTACK (0.03 to 30 ms), RATIO (1.5:1 to infinity),
//! RELEASE (0.05 to 2 s), KNEE (hard, medium, soft), THRUST (a high-pass tilt in the side chain so the
//! bass pushes it less: NORM, MED, LOUD), TYPE (NEW: feed-forward, OLD: feed-back, like the 525) and
//! OUTPUT, from API's manual. Curves estimated.

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
    #[id = "threshold"]
    pub threshold: FloatParam,
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "ratio"]
    pub ratio: FloatParam,
    #[id = "release"]
    pub release: FloatParam,
    #[id = "knee"]
    pub knee: FloatParam,
    #[id = "thrust"]
    pub thrust: FloatParam,
    #[id = "ctype"]
    pub ctype: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            threshold: db("Threshold", -10.0, -20.0, 10.0),
            attack: FloatParam::new("Attack", 3.0, FloatRange::Linear { min: 0.0, max: 6.0 }).with_step_size(1.0),
            ratio: FloatParam::new("Ratio", 3.0, FloatRange::Linear { min: 0.0, max: 6.0 }).with_step_size(1.0),
            release: FloatParam::new("Release", 2.0, FloatRange::Linear { min: 0.0, max: 5.0 }).with_step_size(1.0),
            knee: FloatParam::new("Knee", 1.0, FloatRange::Linear { min: 0.0, max: 2.0 }).with_step_size(1.0),
            thrust: FloatParam::new("Thrust", 0.0, FloatRange::Linear { min: 0.0, max: 2.0 }).with_step_size(1.0),
            ctype: FloatParam::new("Type", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            output: db("Output", 0.0, -12.0, 12.0),
        }
    }
}

pub const THRESHOLD: usize = 0;
pub const ATTACK: usize = 1;
pub const RATIO: usize = 2;
pub const RELEASE: usize = 3;
pub const KNEE: usize = 4;
pub const THRUST: usize = 5;
pub const CTYPE: usize = 6;
pub const OUTPUT: usize = 7;
pub const NPARAMS: usize = 8;

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
            self.attack.as_ptr(),
            self.ratio.as_ptr(),
            self.release.as_ptr(),
            self.knee.as_ptr(),
            self.thrust.as_ptr(),
            self.ctype.as_ptr(),
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

const ATTACKS: [f64; 7] = [0.03, 0.1, 0.3, 1.0, 3.0, 10.0, 30.0];
const RATIOS: [f64; 7] = [1.5, 2.0, 3.0, 4.0, 6.0, 10.0, 60.0];
const RELEASES: [f64; 6] = [50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0];
const KNEES: [f64; 3] = [0.0, 6.0, 14.0];

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    gr: f64,
    tilt: Vec<Biquad>,
    iron: Vec<Iron>,
    out: Smooth,
}

impl Dsp {
    fn set_thrust(&mut self) {
        // MED: a gentle shelf down in the lows of the side chain; LOUD: more, up to 15 dB
        let cut = [0.0, -7.0, -15.0][(self.v[THRUST].round() as usize).min(2)];
        for t in self.tilt.iter_mut() {
            t.set_low_shelf(self.fs, 400.0, 0.7, cut);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let n = ch.max(2);
        let mut d = Self { fs, v, gr: 0.0, tilt: vec![Biquad::identity(); n], iron: vec![Iron::new(fs, 40.0, 1.4, 26000.0); n], out: Smooth::new(fs, 20.0, 1.0) };
        d.set_thrust();
        d
    }
    fn reset(&mut self) {
        self.gr = 0.0;
        self.tilt.iter_mut().for_each(|b| b.reset());
        self.iron.iter_mut().for_each(|i| i.reset());
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
        self.set_thrust();
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let thr = self.v[THRESHOLD] as f64;
        let ratio = RATIOS[(self.v[RATIO].round() as usize).min(6)];
        let knee = KNEES[(self.v[KNEE].round() as usize).min(2)];
        let ca = coeff(self.fs, ATTACKS[(self.v[ATTACK].round() as usize).min(6)]);
        let cr = coeff(self.fs, RELEASES[(self.v[RELEASE].round() as usize).min(5)] / 3.0);
        let old = self.v[CTYPE] > 0.5;
        let mut most = 0.0f64;
        for k in 0..n {
            let o = self.out.next();
            let g_now = db_to_gain(-self.gr);
            let mut peak = 0.0f64;
            for (ci, ch) in chans.iter().enumerate() {
                let j = ci.min(self.tilt.len() - 1);
                // OLD (feed-back) listens after the gain cell, so it never quite catches up: softer
                let s = self.tilt[j].process(ch[k] as f64 * if old { g_now } else { 1.0 });
                peak = peak.max(s.abs());
            }
            let r = if old { 1.0 + (ratio - 1.0) * 0.6 } else { ratio };
            let target = comp_gr(gain_to_db(peak.max(1e-9)), thr, r, knee) * if old { 1.7 } else { 1.0 };
            let gr = ballistics(&mut self.gr, target, ca, cr);
            most = most.max(gr);
            let g = db_to_gain(-gr) * o;
            for (ci, ch) in chans.iter_mut().enumerate() {
                let j = ci.min(self.iron.len() - 1);
                let x = ch[k] as f64 * g;
                // the 2520 op-amps: clean until they aren't
                let y = if x.abs() < 1.0 { x - 0.02 * x * x * x } else { x.signum() * (0.98 + 0.02 * (x.abs() - 1.0).tanh()) };
                ch[k] = self.iron[j].process(y) as f32;
            }
        }
        meters[0] = most as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Mix Bus, Med Thrust", values: &[(THRESHOLD, -8.0), (RATIO, 1.0), (ATTACK, 4.0), (RELEASE, 3.0), (THRUST, 1.0), (OUTPUT, 2.0)] },
        Preset { name: "Drum Bus, Loud", values: &[(THRESHOLD, -14.0), (RATIO, 3.0), (ATTACK, 3.0), (RELEASE, 1.0), (THRUST, 2.0), (OUTPUT, 5.0)] },
        Preset { name: "Old Type, Soft", values: &[(CTYPE, 1.0), (KNEE, 2.0), (THRESHOLD, -12.0), (RATIO, 2.0), (OUTPUT, 3.0)] },
        Preset { name: "Limit", values: &[(RATIO, 6.0), (ATTACK, 0.0), (RELEASE, 1.0), (KNEE, 0.0), (THRESHOLD, -4.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "API 2500",
        subtitle: ("STEREO BUS COMPRESSOR", "THRUST, KNEE, NEW OR OLD TYPE"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![
            Item::Knob { p: THRESHOLD, x: 80.0, y: 140.0, r: 34.0, label: "THRESHOLD", tip: "Where it starts.", active: None },
            Item::Selector { p: ATTACK, x: 215.0, y: 140.0, label: "ATTACK", positions: &[".03", ".1", ".3", "1", "3", "10", "30"], tip: "Attack, in milliseconds." },
            Item::Meter { m: 0, x: 300.0, y: 80.0, w: 140.0, h: 100.0, kind: MeterKind::GainReduction, caption: "GR" },
            Item::Selector { p: RATIO, x: 545.0, y: 140.0, label: "RATIO", positions: &["1.5", "2", "3", "4", "6", "10", "INF"], tip: "Ratio." },
            Item::Divider { y: 230.0 },
            Item::Selector { p: RELEASE, x: 80.0, y: 320.0, label: "RELEASE", positions: &[".05", ".1", ".2", ".5", "1", "2"], tip: "Release, in seconds." },
            Item::Selector { p: KNEE, x: 210.0, y: 320.0, label: "KNEE", positions: &["HARD", "MED", "SOFT"], tip: "How gradually it comes in." },
            Item::Selector { p: THRUST, x: 340.0, y: 320.0, label: "THRUST", positions: &["NORM", "MED", "LOUD"], tip: "Less bass in the side chain: the lows push it less, the mix keeps its weight." },
            Item::Switch { p: CTYPE, x: 460.0, y: 320.0, labels: ["NEW", "OLD"], tip: "NEW: feed-forward. OLD: feed-back, as API's 525." },
            Item::Knob { p: OUTPUT, x: 570.0, y: 320.0, r: 30.0, label: "OUTPUT", tip: "Make-up gain.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "API 2500", class_id: *b"HWAPI2500CompPQD", vendor: "PEQUOD Hardware",
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
        let s = spread(&[(THRESHOLD, -18.0), (RATIO, 5.0)]);
        println!("+29.5 dB in -> {:.1} dB out", s);
        assert!(s < 24.0, "{s}");
    }

}
