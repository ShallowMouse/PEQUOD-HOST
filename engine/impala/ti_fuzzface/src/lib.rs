//! Fuzz Face: the Dunlop Fuzz Face, the chunky fuzz on Elephant (Lonerism) and Disciples (Currents)
//! (Mixdown). Two transistors with feedback, the input loading the guitar so the sound cleans up as the
//! guitar's volume comes down, and a Fuzz control that sets how much gain the second stage keeps. Germanium
//! (softer, rounder, sags as it warms) or silicon (brighter, harder and more gain). Behaviour follows
//! published Fuzz Face circuit analyses; the exact curves are estimated.

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

pub const W: u32 = 640;
pub const H: u32 = 432;

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Kind { #[name = "Germanium"] Ge, #[name = "Silicon"] Si }

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "fuzz"]
    pub fuzz: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
    #[id = "guitar"]
    pub guitar: FloatParam,
    #[id = "kind"]
    pub kind: EnumParam<Kind>,
    #[id = "bias"]
    pub bias: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            fuzz: pct("Fuzz", 0.85),
            volume: db("Volume", -6.0, -30.0, 12.0),
            guitar: pct("Guitar Volume", 1.0),
            kind: EnumParam::new("Transistors", Kind::Ge),
            bias: pct("Bias", 0.5),
        }
    }
}

pub const FUZZ: usize = 0;
pub const VOLUME: usize = 1;
pub const GUITAR: usize = 2;
pub const KIND: usize = 3;
pub const BIAS: usize = 4;
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
            self.fuzz.as_ptr(),
            self.volume.as_ptr(),
            self.guitar.as_ptr(),
            self.kind.as_ptr(),
            self.bias.as_ptr(),
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

#[derive(Clone, Default)]
struct St {
    inp: Biquad,
    mid: Biquad,
    lp: Biquad,
    post: Biquad,
    sag: f64,
}

struct Chan {
    os: Oversampler,
    st: St,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ch: Vec<Chan>,
    key: Vec<i32>,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let ofs = self.fs * OS as f64;
        let si = self.v[KIND] > 0.5;
        for c in self.ch.iter_mut() {
            let st = &mut c.st;
            // the 2.2 uF input cap into the low input impedance: thins the lows a little
            st.inp.set_highpass(ofs, 90.0, 0.6);
            st.mid.set_peak(ofs, 900.0, 0.7, if si { 2.0 } else { 1.0 });
            st.lp.set_lowpass(ofs, if si { 7500.0 } else { 5000.0 }, 0.7);
            st.post.set_highpass(ofs, 25.0, 0.7);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, ch: (0..ch.max(2)).map(|_| Chan { os: Oversampler::new(), st: St::default() }).collect(), key: vec![], out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn reset(&mut self) {
        for c in self.ch.iter_mut() {
            c.os.reset();
            c.st = St::default();
        }
        self.design();
        self.out.snap(db_to_gain(self.v[VOLUME] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k: Vec<i32> = vec![v[KIND].round() as i32];
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[VOLUME] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let v = self.v;
        let si = v[KIND] > 0.5;
        let fuzz = v[FUZZ] as f64;
        // the guitar's volume pot against the pedal's low input impedance: turning it down cuts
        // the drive far more than the level, which is why a Fuzz Face cleans up
        let gv = v[GUITAR] as f64;
        let into = gv.powf(2.2);
        let gain = (if si { 120.0 } else { 55.0 }) * (0.08 + 0.92 * fuzz * fuzz) * into + 1.0;
        let bias = (v[BIAS] as f64 - 0.5) * if si { 0.3 } else { 0.7 };
        let sag_c = coeff(self.fs * OS as f64, 40.0);
        let last = self.ch.len() - 1;
        for k in 0..n {
            let o = self.out.next();
            for (ci, chn) in chans.iter_mut().enumerate() {
                let c = &mut self.ch[ci.min(last)];
                let x = chn[k] as f64;
                let st = &mut c.st;
                let y = c.os.process(x, |u| {
                    let s = st.inp.process(u * (0.25 + 0.75 * gv));
                    // germanium sags under a big signal: the bias drifts and the clipping goes lopsided and splattery
                    let a = s.abs();
                    st.sag = a + sag_c * (st.sag - a);
                    let b = bias + if si { 0.0 } else { 0.35 * st.sag.min(1.0) };
                    let d = s * gain;
                    let top = if si { 1.0 } else { 0.8 };
                    let w = if d + b > 0.0 { ((d + b) / top).tanh() * top } else { ((d + b) * 1.6).tanh() / 1.6 } - (b.max(0.0) / top).tanh() * top;
                    let w = st.mid.process(w);
                    st.post.process(st.lp.process(w)) * 0.45
                });
                chn[k] = (y * o) as f32;
            }
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Elephant", values: &[] },
        Preset { name: "Disciples (Silicon)", values: &[(KIND, 1.0), (FUZZ, 0.75), (VOLUME, -10.0)] },
        Preset { name: "Cleaned Up on the Guitar", values: &[(GUITAR, 0.55), (VOLUME, 0.0)] },
        Preset { name: "Starved and Splattery", values: &[(BIAS, 0.95), (FUZZ, 1.0), (VOLUME, -4.0)] },
        Preset { name: "Low Fuzz Crunch", values: &[(FUZZ, 0.3), (VOLUME, -2.0)] },
        Preset { name: "Fuzz Bass", values: &[(FUZZ, 0.6), (GUITAR, 0.8), (VOLUME, -6.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "FUZZ FACE",
        subtitle: ("TWO TRANSISTORS AND A SMILE", "ELEPHANT · DISCIPLES"),
        accent: ui::TANGERINE,
        size: (W, H),
        items: vec![
            Item::Knob { p: VOLUME, x: 200.0, y: 150.0, r: 34.0, label: "VOLUME", tip: "Output level.", active: None },
            Item::Knob { p: FUZZ, x: 440.0, y: 150.0, r: 34.0, label: "FUZZ", tip: "How much gain the second transistor keeps.", active: None },
            Item::Divider { y: 250.0 },
            Item::Knob { p: GUITAR, x: 140.0, y: 330.0, r: 24.0, label: "GUITAR VOL", tip: "Your guitar's volume knob: turn it down and the fuzz cleans up, as a Fuzz Face does.", active: None },
            Item::Selector { p: KIND, x: 320.0, y: 330.0, label: "TRANSISTORS", positions: &["GE", "SI"], tip: "Germanium: softer, rounder and saggy. Silicon: brighter, harder, more gain." },
            Item::Knob { p: BIAS, x: 500.0, y: 330.0, r: 22.0, label: "BIAS", tip: "The second transistor's bias: off-centre is gated and splattery.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Fuzz Face", class_id: *b"TImpFuzzFacePQD1", vendor: "PEQUOD Tame Impala",
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

    fn harmonics(p: &[f32]) -> f64 {
        // energy away from the fundamental, relative to all of it
        let x = sine(220.0, 0.3, 48000);
        let y = go(p, &x);
        let s = &y[24000..];
        let mut fund = Biquad::identity();
        fund.set_bandpass(48000.0, 220.0, 8.0);
        let f: Vec<f32> = s.iter().map(|v| fund.process(*v as f64) as f32).collect();
        let (a, b) = (rms(s), rms(&f[4000..]));
        ((a * a - b * b).max(0.0)).sqrt() / a.max(1e-9)
    }
    #[test]
    fn cleans_up_with_the_guitar_volume() {
        let full = harmonics(&with(defaults(), &[(GUITAR, 1.0)]));
        let low = harmonics(&with(defaults(), &[(GUITAR, 0.3)]));
        println!("distortion share: guitar full {full:.2}, at 0.3 {low:.2}");
        assert!(full > 0.4 && low < full * 0.6);
    }
    #[test]
    fn fuzz_adds_grit() {
        let lo = harmonics(&with(defaults(), &[(FUZZ, 0.0)]));
        let hi = harmonics(&with(defaults(), &[(FUZZ, 1.0)]));
        println!("fuzz 0 {lo:.2}, fuzz 1 {hi:.2}");
        assert!(hi > lo);
    }

}
