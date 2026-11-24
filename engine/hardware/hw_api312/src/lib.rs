//! API 312: API's microphone preamp card (1967), a 2520 discrete op-amp between an input transformer and
//! the 2503 output transformer: the punchy, forward front end of the API desks. GAIN drives the op-amp
//! (cleanly, then a quick, slightly lopsided clip and the iron thickening the lows), PAD (-20 dB), PHASE
//! and OUTPUT. Behaviour estimated from the published circuit.

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
    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "pad"]
    pub pad: FloatParam,
    #[id = "phase"]
    pub phase: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            gain: db("Gain", 0.0, -10.0, 40.0),
            pad: FloatParam::new("Pad", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            phase: FloatParam::new("Phase", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            output: db("Output", 0.0, -30.0, 10.0),
        }
    }
}

pub const GAIN: usize = 0;
pub const PAD: usize = 1;
pub const PHASE: usize = 2;
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
            self.gain.as_ptr(),
            self.pad.as_ptr(),
            self.phase.as_ptr(),
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
    bands: Vec<[Biquad; 1]>,
    iron: Vec<Iron>,
    iron2: Vec<Iron>,
    gain: Smooth,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        for b in self.bands.iter_mut() {
            b[0].set_highpass1(fs, 12.0);
        }
        let pad = if self.v[PAD] > 0.5 { -20.0 } else { 0.0 };
        self.gain.set(db_to_gain(self.v[GAIN] as f64 + pad));
        self.out.set(db_to_gain(self.v[OUTPUT] as f64 - self.v[GAIN] as f64 * 0.85));
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let n = ch.max(2);
        let mut d = Self { fs, v, bands: vec![[Biquad::identity(); 1]; n], iron: vec![Iron::new(fs, 50.0, 1.6, 40000.0); n],
                           iron2: vec![Iron::new(fs, 45.0, 2.4, 28000.0); n], gain: Smooth::new(fs, 20.0, 1.0), out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.bands.iter_mut().flatten().for_each(|b| b.reset());
        self.iron.iter_mut().chain(self.iron2.iter_mut()).for_each(|i| i.reset());
        self.design();
        self.gain.snap(self.gain.value.max(0.0));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.design();
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let flip = if self.v[PHASE] > 0.5 { -1.0 } else { 1.0 };
        for k in 0..n {
            let g = self.gain.next();
            let o = self.out.next();
            for (ci, ch) in chans.iter_mut().enumerate() {
                let j = ci.min(self.bands.len() - 1);
                let x = self.iron[j].process(ch[k] as f64 * g);
                // the 2520: clean to near its rails, then a quick, slightly uneven clip
                let y = if x >= 0.0 { (x * 1.05).tanh() } else { (x * 0.95).tanh() * 1.02 };
                let y = self.bands[j][0].process(y);
                ch[k] = (self.iron2[j].process(y) * o * flip) as f32;
            }
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Clean Gain", values: &[(GAIN, 6.0)] },
        Preset { name: "Punchy Drums", values: &[(GAIN, 20.0)] },
        Preset { name: "Driven Hard", values: &[(GAIN, 34.0), (OUTPUT, -4.0)] },
        Preset { name: "Padded", values: &[(PAD, 1.0), (GAIN, 26.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "API 312",
        subtitle: ("MICROPHONE PREAMP CARD, 1967", "2520 OP-AMP, 2503 OUTPUT TRANSFORMER"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![
            Item::Knob { p: GAIN, x: 160.0, y: 200.0, r: 48.0, label: "GAIN", tip: "Into the 2520: past the middle it starts to thicken and clip.", active: None },
            Item::Switch { p: PAD, x: 320.0, y: 150.0, labels: ["", "-20 PAD"], tip: "20 dB pad ahead of the transformer." },
            Item::Switch { p: PHASE, x: 320.0, y: 260.0, labels: ["", "PHASE"], tip: "Polarity flip." },
            Item::Knob { p: OUTPUT, x: 480.0, y: 200.0, r: 40.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "API 312", class_id: *b"HWAPI312PrePQD01", vendor: "PEQUOD Hardware",
    categories: [Fx, Distortion]
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
    fn drive_adds_harmonics() {
        let p = with(defaults(), &[(GAIN, 36.0)]);
        let y = go(&p, &sine(200.0, 0.3, 48000));
        // energy at 600 Hz (third harmonic) appears when driven
        let mut bp = Biquad::identity();
        bp.set_bandpass(48000.0, 600.0, 8.0);
        let h: Vec<f32> = y.iter().map(|v| bp.process(*v as f64) as f32).collect();
        println!("3rd harmonic {:.1} dB under the output", db(rms(&y[24000..])) - db(rms(&h[24000..])));
        assert!(db(rms(&h[24000..])) > db(rms(&y[24000..])) - 50.0);
    }

}
