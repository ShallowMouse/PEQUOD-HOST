//! Neve 33609: the stereo diode-bridge compressor/limiter (1969 onwards, from the 2254 of the BCM10 and
//! A88 desks), the mix-bus compressor of British rock. A limiter (threshold +4 to +15 dB, release 50 ms to
//! 800 ms or auto) ahead of a compressor (threshold -20 to +10 dB, ratio 1.5:1 to 6:1, release 100 ms to
//! 1.5 s or two auto settings), each switchable, GAIN make-up. The diode bridge colours more the harder it
//! works; Marinair/St Ives transformers in and out. From Neve's manual; the curves are estimated.

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
    #[id = "cthresh"]
    pub cthresh: FloatParam,
    #[id = "ratio"]
    pub ratio: FloatParam,
    #[id = "crel"]
    pub crel: FloatParam,
    #[id = "comp"]
    pub comp: FloatParam,
    #[id = "lthresh"]
    pub lthresh: FloatParam,
    #[id = "lrel"]
    pub lrel: FloatParam,
    #[id = "lim"]
    pub lim: FloatParam,
    #[id = "gain"]
    pub gain: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            cthresh: db("Comp Threshold", -6.0, -20.0, 10.0),
            ratio: FloatParam::new("Ratio", 1.0, FloatRange::Linear { min: 0.0, max: 4.0 }).with_step_size(1.0),
            crel: FloatParam::new("Comp Release", 1.0, FloatRange::Linear { min: 0.0, max: 5.0 }).with_step_size(1.0),
            comp: FloatParam::new("Compressor", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            lthresh: db("Limit Threshold", 10.0, 4.0, 15.0),
            lrel: FloatParam::new("Limit Release", 1.0, FloatRange::Linear { min: 0.0, max: 3.0 }).with_step_size(1.0),
            lim: FloatParam::new("Limiter", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            gain: db("Gain", 0.0, 0.0, 20.0),
        }
    }
}

pub const CTHRESH: usize = 0;
pub const RATIO: usize = 1;
pub const CREL: usize = 2;
pub const COMP: usize = 3;
pub const LTHRESH: usize = 4;
pub const LREL: usize = 5;
pub const LIM: usize = 6;
pub const GAIN: usize = 7;
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
            self.cthresh.as_ptr(),
            self.ratio.as_ptr(),
            self.crel.as_ptr(),
            self.comp.as_ptr(),
            self.lthresh.as_ptr(),
            self.lrel.as_ptr(),
            self.lim.as_ptr(),
            self.gain.as_ptr(),
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

const RATIOS: [f64; 5] = [1.5, 2.0, 3.0, 4.0, 6.0];
const C_REL: [f64; 4] = [100.0, 400.0, 800.0, 1500.0];
const L_REL: [f64; 3] = [50.0, 100.0, 800.0];

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    cgr: f64,
    cslow: f64,
    lgr: f64,
    lslow: f64,
    iron_in: Vec<Iron>,
    iron_out: Vec<Iron>,
    gain: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let n = ch.max(2);
        Self { fs, v, cgr: 0.0, cslow: 0.0, lgr: 0.0, lslow: 0.0, iron_in: vec![Iron::new(fs, 70.0, 1.8, 30000.0); n],
               iron_out: vec![Iron::new(fs, 60.0, 2.2, 22000.0); n], gain: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.cgr = 0.0;
        self.cslow = 0.0;
        self.lgr = 0.0;
        self.lslow = 0.0;
        self.iron_in.iter_mut().chain(self.iron_out.iter_mut()).for_each(|i| i.reset());
        self.gain.snap(db_to_gain(self.v[GAIN] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.gain.set(db_to_gain(v[GAIN] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        // the front panel's levels are dBu; 0 dBu sits at -18 dBFS here
        let cthr = self.v[CTHRESH] as f64 - 18.0;
        let lthr = self.v[LTHRESH] as f64 - 18.0;
        let ratio = RATIOS[(self.v[RATIO].round() as usize).min(4)];
        let cr = self.v[CREL].round() as usize;
        let lr = self.v[LREL].round() as usize;
        let comp_on = self.v[COMP] > 0.5;
        let lim_on = self.v[LIM] > 0.5;
        let ca = coeff(self.fs, 3.0);
        let la = coeff(self.fs, 2.0);
        let c_auto = cr >= 4;
        let crel = coeff(self.fs, if c_auto { 100.0 } else { C_REL[cr] } / 3.0);
        let cslow = coeff(self.fs, if cr == 5 { 5000.0 } else { 1500.0 } / 3.0);
        let l_auto = lr >= 3;
        let lrel = coeff(self.fs, if l_auto { 40.0 } else { L_REL[lr] } / 3.0);
        let lslow = coeff(self.fs, 2000.0 / 3.0);
        let mut most = 0.0f64;
        for k in 0..n {
            let g = self.gain.next();
            let mut x = [0.0f64; 2];
            let nc = chans.len();
            for c in 0..nc.min(2) {
                x[c] = self.iron_in[c].process(chans[c][k] as f64);
            }
            let peak = x.iter().fold(0.0f64, |a, b| a.max(b.abs())).max(1e-9);
            let lvl = gain_to_db(peak);
            let mut gr = 0.0;
            if lim_on {
                let t = comp_gr(lvl, lthr, 40.0, 2.0);
                ballistics(&mut self.lgr, t, la, lrel);
                ballistics(&mut self.lslow, if l_auto { t * 0.5 } else { 0.0 }, la, lslow);
                gr += self.lgr.max(self.lslow);
            }
            if comp_on {
                let t = comp_gr(lvl - gr, cthr, ratio, 6.0);
                ballistics(&mut self.cgr, t, ca, crel);
                ballistics(&mut self.cslow, if c_auto { t * 0.6 } else { 0.0 }, ca, cslow);
                gr += self.cgr.max(self.cslow);
            }
            most = most.max(gr);
            let a = db_to_gain(-gr);
            // the diode bridge: odd harmonics rising with the gain reduction
            let grit = (gr * 0.012).min(0.15);
            for c in 0..nc {
                let j = c.min(1);
                let y = x[j] * a;
                let y = y - grit * y * y * y;
                chans[c][k] = (self.iron_out[j].process(y * g)) as f32;
            }
        }
        meters[0] = most as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Mix Bus 2:1 Auto", values: &[(CTHRESH, -6.0), (RATIO, 1.0), (CREL, 4.0), (GAIN, 3.0)] },
        Preset { name: "Rock Drums 4:1", values: &[(CTHRESH, -12.0), (RATIO, 3.0), (CREL, 0.0), (GAIN, 6.0)] },
        Preset { name: "Limit and Compress", values: &[(LIM, 1.0), (LTHRESH, 8.0), (CTHRESH, -8.0), (RATIO, 2.0), (GAIN, 4.0)] },
        Preset { name: "Vocal 3:1", values: &[(CTHRESH, -14.0), (RATIO, 2.0), (CREL, 1.0), (GAIN, 6.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "NEVE 33609",
        subtitle: ("STEREO DIODE-BRIDGE COMPRESSOR/LIMITER", "LIMITER AHEAD OF COMPRESSOR, AS ON THE FRONT"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![
            Item::Text { x: 160.0, y: 70.0, text: "LIMITER", size: 13.0, align: Align2::CENTER_CENTER },
            Item::Knob { p: LTHRESH, x: 90.0, y: 140.0, r: 30.0, label: "THRESHOLD", tip: "+4 to +15 dB.", active: None },
            Item::Selector { p: LREL, x: 200.0, y: 140.0, label: "RECOVERY", positions: &["50", "100", "800", "A"], tip: "Limiter release, ms; A: auto." },
            Item::Switch { p: LIM, x: 160.0, y: 215.0, labels: ["OUT", "IN"], tip: "Limiter in or out." },
            Item::Meter { m: 0, x: 260.0, y: 75.0, w: 120.0, h: 90.0, kind: MeterKind::GainReduction, caption: "GAIN REDUCTION" },
            Item::Text { x: 480.0, y: 70.0, text: "COMPRESSOR", size: 13.0, align: Align2::CENTER_CENTER },
            Item::Knob { p: CTHRESH, x: 430.0, y: 140.0, r: 30.0, label: "THRESHOLD", tip: "-20 to +10 dB.", active: None },
            Item::Selector { p: RATIO, x: 545.0, y: 140.0, label: "RATIO", positions: &["1.5", "2", "3", "4", "6"], tip: "Ratio." },
            Item::Switch { p: COMP, x: 480.0, y: 215.0, labels: ["OUT", "IN"], tip: "Compressor in or out." },
            Item::Divider { y: 255.0 },
            Item::Selector { p: CREL, x: 200.0, y: 335.0, label: "RECOVERY", positions: &["100", "400", "800", "1500", "A1", "A2"], tip: "Compressor release, ms; A1, A2: auto." },
            Item::Knob { p: GAIN, x: 440.0, y: 335.0, r: 34.0, label: "GAIN", tip: "Make-up gain, 0 to +20 dB.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Neve 33609", class_id: *b"HWNeve33609CompQ", vendor: "PEQUOD Hardware",
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
        let s = spread(&[(CTHRESH, -20.0), (RATIO, 4.0)]);
        println!("+29.5 dB in -> {:.1} dB out", s);
        assert!(s < 24.0, "{s}");
    }

}
