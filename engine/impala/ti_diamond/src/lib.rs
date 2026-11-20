//! Diamond Vibrato: the Canadian-made Diamond Vibrato pedal, which Kevin Parker said was used with pretty
//! much all of the guitars on Lonerism (Red Bull interview). A true pitch vibrato: the signal runs through a
//! short bucket-brigade delay whose time is swept, so the note wavers in pitch with no dry signal mixed in.
//! Rate, depth, a tone trim and volume, with a sine or triangle sweep. The delay range and filtering follow
//! bucket-brigade vibratos of its kind and are estimated.

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
pub enum Wave { #[name = "Sine"] Sine, #[name = "Triangle"] Triangle }

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "rate"]
    pub rate: FloatParam,
    #[id = "depth"]
    pub depth: FloatParam,
    #[id = "wave"]
    pub wave: EnumParam<Wave>,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            rate: hz("Rate", 5.5, 0.5, 12.0),
            depth: pct("Depth", 0.45),
            wave: EnumParam::new("Wave", Wave::Sine),
            tone: pct("Tone", 0.5),
            volume: db("Volume", 0.0, -24.0, 12.0),
        }
    }
}

pub const RATE: usize = 0;
pub const DEPTH: usize = 1;
pub const WAVE: usize = 2;
pub const TONE: usize = 3;
pub const VOLUME: usize = 4;
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
            self.rate.as_ptr(),
            self.depth.as_ptr(),
            self.wave.as_ptr(),
            self.tone.as_ptr(),
            self.volume.as_ptr(),
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
    bbd: Vec<Bbd>,
    tone: Vec<Biquad>,
    phase: f64,
    key: i32,
    depth: Smooth,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        // the tone trim: a treble shelf either way around 2.5 kHz
        let t = (self.v[TONE] as f64 - 0.5) * 16.0;
        for b in self.tone.iter_mut() {
            b.set_high_shelf(self.fs, 2500.0, 0.7, t);
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
        let mut bbd: Vec<Bbd> = (0..n).map(|_| Bbd::new(fs, 20.0)).collect();
        bbd.iter_mut().for_each(|b| b.set_bandwidth(7000.0));
        let mut d = Self { fs, v, bbd, tone: vec![Biquad::identity(); n], phase: 0.0, key: -1, depth: Smooth::new(fs, 30.0, 0.45), out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.bbd.iter_mut().for_each(|b| b.reset());
        self.tone.iter_mut().for_each(|b| b.reset());
        self.phase = 0.0;
        self.depth.snap(self.v[DEPTH] as f64);
        self.out.snap(db_to_gain(self.v[VOLUME] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = (v[TONE] * 100.0) as i32;
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.depth.set(v[DEPTH] as f64);
        self.out.set(db_to_gain(v[VOLUME] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let rate = self.v[RATE] as f64;
        let tri = self.v[WAVE] > 0.5;
        let last = self.bbd.len() - 1;
        for k in 0..n {
            self.phase = (self.phase + rate / self.fs).fract();
            let lfo = if tri { 1.0 - 4.0 * (self.phase - 0.5).abs() } else { (std::f64::consts::TAU * self.phase).sin() };
            // the sweep: 1.5 ms at rest, up to 5 ms more at full depth
            let dep = self.depth.next();
            let d = (0.0015 + 0.0025 * dep * (1.0 + lfo)) * self.fs;
            let o = self.out.next();
            for (ci, c) in chans.iter_mut().enumerate() {
                let j = ci.min(last);
                let y = self.bbd[j].process(c[k] as f64, d);
                c[k] = (self.tone[j].process(y) * o) as f32;
            }
        }
        meters[0] = (0.5 + 0.5 * (std::f64::consts::TAU * self.phase).sin()) as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Lonerism Guitars", values: &[] },
        Preset { name: "Seasick Warble", values: &[(RATE, 3.0), (DEPTH, 0.8), (TONE, 0.4)] },
        Preset { name: "Fast Shimmer", values: &[(RATE, 9.0), (DEPTH, 0.25), (TONE, 0.6)] },
        Preset { name: "Slow Detune", values: &[(RATE, 0.8), (DEPTH, 0.6), (WAVE, 1.0)] },
        Preset { name: "Wobbly Synth Line", values: &[(RATE, 6.5), (DEPTH, 0.35), (WAVE, 1.0), (TONE, 0.35)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "DIAMOND VIBRATO",
        subtitle: ("A BUCKET-BRIGADE PITCH VIBRATO", "ON PRETTY MUCH ALL OF LONERISM'S GUITARS"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![
            Item::Knob { p: RATE, x: 120.0, y: 170.0, r: 32.0, label: "RATE", tip: "How fast the pitch wavers.", active: None },
            Item::Knob { p: DEPTH, x: 260.0, y: 170.0, r: 32.0, label: "DEPTH", tip: "How far the pitch wavers.", active: None },
            Item::Knob { p: TONE, x: 400.0, y: 170.0, r: 24.0, label: "TONE", tip: "Treble trim, darker or brighter.", active: None },
            Item::Knob { p: VOLUME, x: 520.0, y: 170.0, r: 24.0, label: "VOLUME", tip: "Output level.", active: None },
            Item::Selector { p: WAVE, x: 260.0, y: 290.0, label: "WAVE", positions: &["SINE", "TRI"], tip: "The sweep's shape: smooth, or a steadier triangle." },
            Item::Lamp { m: 0, x: 400.0, y: 290.0, label: "RATE", lo: 0.1, hi: 0.9 },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Diamond Vibrato", class_id: *b"TImpDiamondVibr1", vendor: "PEQUOD Tame Impala",
    categories: [Fx, Modulation]
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

    /// The frequency of a sine at the output, from zero crossings over a short window.
    fn freq(y: &[f32]) -> f64 {
        let mut c = 0;
        for w in y.windows(2) {
            if w[0] <= 0.0 && w[1] > 0.0 {
                c += 1;
            }
        }
        c as f64 * 48000.0 / y.len() as f64
    }
    #[test]
    fn pitch_wavers_with_depth_and_not_without() {
        let x = sine(1000.0, 0.2, 48000);
        let span = |p: &[f32]| {
            let y = go(p, &x);
            let fr: Vec<f64> = (0..20).map(|i| freq(&y[9600 + i * 1600..9600 + (i + 1) * 1600])).collect();
            fr.iter().cloned().fold(0.0, f64::max) - fr.iter().cloned().fold(1e9, f64::min)
        };
        let a = span(&with(defaults(), &[(DEPTH, 0.0)]));
        let b = span(&with(defaults(), &[(DEPTH, 1.0), (RATE, 3.0)]));
        println!("pitch swing: depth 0 {a:.1} Hz, full {b:.1} Hz");
        assert!(a < 35.0 && b > 50.0);
    }

}
