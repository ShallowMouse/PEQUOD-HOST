//! API 550A: Saul Walker's three-band equaliser (1967), discrete 2520 op-amps and "proportional Q": the
//! more you boost or cut, the narrower the curve. Stepped gain on each band (+/-2, 4, 6, 9, 12 dB) and five
//! frequencies a band (LOW 50 to 400 Hz, MID 400 Hz to 5 kHz, HIGH 5 to 15 kHz); the outer bands switch
//! between shelf and peak; a band-pass filter (50 Hz to 15 kHz). From API's documentation; the Q curve
//! is estimated.

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

pub const W: u32 = 640;
pub const H: u32 = 432;

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "lf"]
    pub lf: FloatParam,
    #[id = "lg"]
    pub lg: FloatParam,
    #[id = "lshelf"]
    pub lshelf: FloatParam,
    #[id = "mf"]
    pub mf: FloatParam,
    #[id = "mg"]
    pub mg: FloatParam,
    #[id = "hf"]
    pub hf: FloatParam,
    #[id = "hg"]
    pub hg: FloatParam,
    #[id = "hshelf"]
    pub hshelf: FloatParam,
    #[id = "filt"]
    pub filt: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            lf: FloatParam::new("Low Freq", 1.0, FloatRange::Linear { min: 0.0, max: 4.0 }).with_step_size(1.0),
            lg: FloatParam::new("Low Gain", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(1.0),
            lshelf: FloatParam::new("Low Shape", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            mf: FloatParam::new("Mid Freq", 2.0, FloatRange::Linear { min: 0.0, max: 4.0 }).with_step_size(1.0),
            mg: FloatParam::new("Mid Gain", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(1.0),
            hf: FloatParam::new("High Freq", 2.0, FloatRange::Linear { min: 0.0, max: 4.0 }).with_step_size(1.0),
            hg: FloatParam::new("High Gain", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(1.0),
            hshelf: FloatParam::new("High Shape", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            filt: FloatParam::new("Filter", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
        }
    }
}

pub const LF: usize = 0;
pub const LG: usize = 1;
pub const LSHELF: usize = 2;
pub const MF: usize = 3;
pub const MG: usize = 4;
pub const HF: usize = 5;
pub const HG: usize = 6;
pub const HSHELF: usize = 7;
pub const FILT: usize = 8;
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
            self.lf.as_ptr(),
            self.lg.as_ptr(),
            self.lshelf.as_ptr(),
            self.mf.as_ptr(),
            self.mg.as_ptr(),
            self.hf.as_ptr(),
            self.hg.as_ptr(),
            self.hshelf.as_ptr(),
            self.filt.as_ptr(),
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

const LF_HZ: [f64; 5] = [50.0, 100.0, 200.0, 300.0, 400.0];
const MF_HZ: [f64; 5] = [400.0, 800.0, 1500.0, 3000.0, 5000.0];
const HF_HZ: [f64; 5] = [5000.0, 7000.0, 10000.0, 12500.0, 15000.0];
const GAIN: [f64; 11] = [-12.0, -9.0, -6.0, -4.0, -2.0, 0.0, 2.0, 4.0, 6.0, 9.0, 12.0];

/// Proportional Q: broad for a little, narrower the more you ask.
fn pq(g: f64) -> f64 {
    0.45 + g.abs() / 12.0 * 1.4
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    bands: Vec<[Biquad; 5]>,
    iron: Vec<Iron>,
    iron2: Vec<Iron>,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let v = self.v;
        let g = |i: usize| GAIN[(v[i].round() as usize).min(10)];
        let (lg, mg, hg) = (g(LG), g(MG), g(HG));
        let lf = LF_HZ[(v[LF].round() as usize).min(4)];
        let mf = MF_HZ[(v[MF].round() as usize).min(4)];
        let hf = HF_HZ[(v[HF].round() as usize).min(4)].min(fs * 0.42);
        let filt = v[FILT] > 0.5;
        for b in self.bands.iter_mut() {
            if v[LSHELF] > 0.5 { b[0].set_low_shelf(fs, lf, 0.8, lg); } else { b[0].set_peak(fs, lf, pq(lg), lg); }
            b[1].set_peak(fs, mf, pq(mg), mg);
            if v[HSHELF] > 0.5 { b[2].set_high_shelf(fs, hf, 0.8, hg); } else { b[2].set_peak(fs, hf, pq(hg), hg); }
            if filt { b[3].set_highpass(fs, 50.0, 0.7); b[4].set_lowpass(fs, 15000.0f64.min(fs * 0.42), 0.7); }
            else { b[3] = Biquad::identity(); b[4] = Biquad::identity(); }
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let n = ch.max(2);
        let mut d = Self { fs, v, bands: vec![[Biquad::identity(); 5]; n], iron: vec![Iron::new(fs, 40.0, 1.3, 40000.0); n],
                           iron2: vec![Iron::new(fs, 35.0, 1.5, 30000.0); n] };
        d.design();
        d
    }

    fn reset(&mut self) {
        self.bands.iter_mut().flatten().for_each(|b| b.reset());
        self.iron.iter_mut().chain(self.iron2.iter_mut()).for_each(|i| i.reset());
        self.design();
    }
    fn set_params(&mut self, v: &[f32]) {
        let changed = self.v[..] != v[..NPARAMS];
        self.v.copy_from_slice(&v[..NPARAMS]);
        if changed {
            self.design();
        }
    }

    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        for (ci, ch) in chans.iter_mut().enumerate() {
            let j = ci.min(self.bands.len() - 1);
            for s in ch.iter_mut() {
                let x = self.iron[j].process(*s as f64);
                let y = cascade(&mut self.bands[j], x);
                let y = y - 0.015 * y * y * y;              // the 2520's slight edge
                *s = self.iron2[j].process(y) as f32;
            }
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Kick (50 +6, 3k +4)", values: &[(LF, 0.0), (LG, 8.0), (MF, 3.0), (MG, 7.0)] },
        Preset { name: "Snare (200 +4, 5k +4)", values: &[(LF, 2.0), (LG, 7.0), (LSHELF, 0.0), (MF, 4.0), (MG, 7.0)] },
        Preset { name: "Guitar (800 -4, 10k +2)", values: &[(MF, 1.0), (MG, 3.0), (HF, 2.0), (HG, 6.0)] },
        Preset { name: "Vocal (3k +2, 12.5k +4 shelf)", values: &[(MF, 3.0), (MG, 6.0), (HF, 3.0), (HG, 7.0)] },
        Preset { name: "Telephone-ish", values: &[(FILT, 1.0), (LF, 4.0), (LG, 0.0), (MF, 2.0), (MG, 9.0), (HF, 0.0), (HG, 0.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "API 550A",
        subtitle: ("THREE-BAND EQUALISER, 1967", "PROPORTIONAL Q, DISCRETE 2520 OP-AMPS"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![
            Item::Selector { p: HF, x: 110.0, y: 140.0, label: "HIGH FREQ", positions: &["5", "7", "10", "12.5", "15"], tip: "kHz." },
            Item::Selector { p: HG, x: 250.0, y: 140.0, label: "HIGH", positions: &["-12", "-9", "-6", "-4", "-2", "0", "+2", "+4", "+6", "+9", "+12"], tip: "dB." },
            Item::Switch { p: HSHELF, x: 360.0, y: 140.0, labels: ["PEAK", "SHELF"], tip: "High band: peak or shelf." },
            Item::Switch { p: FILT, x: 520.0, y: 140.0, labels: ["", "FILTER"], tip: "Band-pass, 50 Hz to 15 kHz." },
            Item::Selector { p: MF, x: 110.0, y: 245.0, label: "MID FREQ", positions: &[".4", ".8", "1.5", "3", "5"], tip: "kHz." },
            Item::Selector { p: MG, x: 250.0, y: 245.0, label: "MID", positions: &["-12", "-9", "-6", "-4", "-2", "0", "+2", "+4", "+6", "+9", "+12"], tip: "dB." },
            Item::Selector { p: LF, x: 110.0, y: 350.0, label: "LOW FREQ", positions: &["50", "100", "200", "300", "400"], tip: "Hz." },
            Item::Selector { p: LG, x: 250.0, y: 350.0, label: "LOW", positions: &["-12", "-9", "-6", "-4", "-2", "0", "+2", "+4", "+6", "+9", "+12"], tip: "dB." },
            Item::Switch { p: LSHELF, x: 360.0, y: 350.0, labels: ["PEAK", "SHELF"], tip: "Low band: peak or shelf." },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "API 550A", class_id: *b"HWAPI550AEqPQD01", vendor: "PEQUOD Hardware",
    categories: [Fx, Eq]
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
    fn shapes_as_set() {
        let flat = level(&[], 1500.0);
        let set = level(&[(MF, 2.0), (MG, 10.0)], 1500.0);
        println!("at 1500.0 Hz: {:.1} dB -> {:.1} dB", flat, set);
        assert!(set - flat > 9.0, "{flat} {set}");
    }

}
