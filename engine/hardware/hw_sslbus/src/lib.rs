//! SSL Bus Compressor: the quad VCA compressor in the centre section of the SSL 4000 E and G (1977 on),
//! the "glue" on the mix of a thousand records. THRESHOLD, RATIO (2, 4, 10), ATTACK (0.1 to 30 ms),
//! RELEASE (0.1 to 1.2 s, or AUTO), MAKE-UP, stereo linked, as on the front. A feed-forward VCA (the dbx
//! 202 cell); AUTO release is program dependent. Figures from SSL's documentation; the curve is estimated.

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
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "release"]
    pub release: FloatParam,
    #[id = "makeup"]
    pub makeup: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            threshold: db("Threshold", -10.0, -30.0, 0.0),
            ratio: FloatParam::new("Ratio", 1.0, FloatRange::Linear { min: 0.0, max: 2.0 }).with_step_size(1.0),
            attack: FloatParam::new("Attack", 3.0, FloatRange::Linear { min: 0.0, max: 5.0 }).with_step_size(1.0),
            release: FloatParam::new("Release", 4.0, FloatRange::Linear { min: 0.0, max: 4.0 }).with_step_size(1.0),
            makeup: db("Make-Up", 0.0, -5.0, 15.0),
        }
    }
}

pub const THRESHOLD: usize = 0;
pub const RATIO: usize = 1;
pub const ATTACK: usize = 2;
pub const RELEASE: usize = 3;
pub const MAKEUP: usize = 4;
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
            self.threshold.as_ptr(),
            self.ratio.as_ptr(),
            self.attack.as_ptr(),
            self.release.as_ptr(),
            self.makeup.as_ptr(),
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

const RATIOS: [f64; 3] = [2.0, 4.0, 10.0];
const ATTACKS: [f64; 6] = [0.1, 0.3, 1.0, 3.0, 10.0, 30.0];
const RELEASES: [f64; 4] = [100.0, 300.0, 600.0, 1200.0];

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    fast: f64,
    slow: f64,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, fast: 0.0, slow: 0.0, out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.fast = 0.0;
        self.slow = 0.0;
        self.out.snap(db_to_gain(self.v[MAKEUP] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[MAKEUP] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let thr = self.v[THRESHOLD] as f64;
        let ratio = RATIOS[(self.v[RATIO].round() as usize).min(2)];
        let ca = coeff(self.fs, ATTACKS[(self.v[ATTACK].round() as usize).min(5)]);
        let rel = self.v[RELEASE].round() as usize;
        let auto = rel >= 4;
        let cr = coeff(self.fs, if auto { 150.0 } else { RELEASES[rel] } / 3.0);
        let cslow = coeff(self.fs, 1500.0 / 3.0);
        let mut most = 0.0f64;
        for k in 0..n {
            let o = self.out.next();
            let peak = chans.iter().map(|c| (c[k] as f64).abs()).fold(0.0, f64::max);
            let target = comp_gr(gain_to_db(peak.max(1e-9)), thr, ratio, 3.0);
            let gr = if auto {
                // AUTO: quick release for transients over a slower one for the level
                ballistics(&mut self.fast, target, ca, cr);
                ballistics(&mut self.slow, target * 0.6, ca, cslow);
                self.fast.max(self.slow)
            } else {
                ballistics(&mut self.fast, target, ca, cr)
            };
            most = most.max(gr);
            let g = db_to_gain(-gr) * o;
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
        Preset { name: "Mix Glue (4:1, 10 ms, Auto)", values: &[(THRESHOLD, -12.0), (RATIO, 1.0), (ATTACK, 4.0), (RELEASE, 4.0), (MAKEUP, 2.0)] },
        Preset { name: "Gentle Bus (2:1, 30 ms)", values: &[(THRESHOLD, -10.0), (RATIO, 0.0), (ATTACK, 5.0), (RELEASE, 1.0), (MAKEUP, 1.5)] },
        Preset { name: "Drum Smash (10:1, 0.1 ms)", values: &[(THRESHOLD, -24.0), (RATIO, 2.0), (ATTACK, 0.0), (RELEASE, 0.0), (MAKEUP, 8.0)] },
        Preset { name: "Pumping (4:1, 0.1 s)", values: &[(THRESHOLD, -18.0), (RATIO, 1.0), (ATTACK, 3.0), (RELEASE, 0.0), (MAKEUP, 4.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "SSL BUS COMPRESSOR",
        subtitle: ("THE 4000 E/G CENTRE SECTION, 1977", "QUAD VCA, STEREO LINKED"),
        accent: ui::GRAPHITE,
        size: (W, H),
        items: vec![
            Item::Meter { m: 0, x: 235.0, y: 80.0, w: 170.0, h: 120.0, kind: MeterKind::GainReduction, caption: "COMPRESSION" },
            Item::Knob { p: THRESHOLD, x: 90.0, y: 150.0, r: 36.0, label: "THRESHOLD", tip: "Where it starts.", active: None },
            Item::Knob { p: MAKEUP, x: 550.0, y: 150.0, r: 36.0, label: "MAKE-UP", tip: "Make-up gain.", active: None },
            Item::Divider { y: 250.0 },
            Item::Selector { p: RATIO, x: 140.0, y: 330.0, label: "RATIO", positions: &["2", "4", "10"], tip: "2:1, 4:1 or 10:1." },
            Item::Selector { p: ATTACK, x: 320.0, y: 330.0, label: "ATTACK", positions: &[".1", ".3", "1", "3", "10", "30"], tip: "Attack, in milliseconds." },
            Item::Selector { p: RELEASE, x: 500.0, y: 330.0, label: "RELEASE", positions: &[".1", ".3", ".6", "1.2", "AUTO"], tip: "Release, in seconds; AUTO follows the programme." },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "SSL Bus Compressor", class_id: *b"HWSSLBusCompPQD1", vendor: "PEQUOD Hardware",
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
        let s = spread(&[(THRESHOLD, -24.0), (RATIO, 2.0)]);
        println!("+29.5 dB in -> {:.1} dB out", s);
        assert!(s < 24.0, "{s}");
    }

}
