//! FET Limiter: feedback FET compressor in the manner of the UREI 1176, the heart of James Ford's vocal chain. Attack and release run backwards (7 = fastest), ratios 4, 8, 12, 20 and all-buttons-in. Modelled from published behaviour.

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
pub const H: u32 = 432;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Ratio { #[name = "4:1"] R4, #[name = "8:1"] R8, #[name = "12:1"] R12, #[name = "20:1"] R20, #[name = "All"] All }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "input"]
    pub input: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "release"]
    pub release: FloatParam,
    #[id = "ratio"]
    pub ratio: EnumParam<Ratio>,
    #[id = "mix"]
    pub mix: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            input: FloatParam::new("Input", 24.0, FloatRange::Linear { min: 0.0, max: 48.0 }).with_step_size(0.1).with_unit(" dB"),
            output: FloatParam::new("Output", -18.0, FloatRange::Linear { min: -48.0, max: 12.0 }).with_step_size(0.1).with_unit(" dB"),
            attack: FloatParam::new("Attack", 3.0, FloatRange::Linear { min: 1.0, max: 7.0 }).with_step_size(0.1),
            release: FloatParam::new("Release", 5.0, FloatRange::Linear { min: 1.0, max: 7.0 }).with_step_size(0.1),
            ratio: EnumParam::new("Ratio", Ratio::R4),
            mix: pct("Mix", 1.0),
        }
    }
}

pub const INPUT: usize = 0;
pub const OUTPUT: usize = 1;
pub const ATTACK: usize = 2;
pub const RELEASE: usize = 3;
pub const RATIO: usize = 4;
pub const MIX: usize = 5;
pub const NPARAMS: usize = 6;

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
            self.output.as_ptr(),
            self.attack.as_ptr(),
            self.release.as_ptr(),
            self.ratio.as_ptr(),
            self.mix.as_ptr(),
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

// (ratio, threshold dB at the detector, knee dB)
const RATIOS: [(f64, f64, f64); 5] = [(4.0, -14.0, 6.0), (8.0, -10.0, 4.0), (12.0, -8.0, 3.0), (20.0, -6.0, 2.0), (16.0, -12.0, 1.0)];

pub struct Dsp {
    fs: f64,
    gr: f64,
    os: [Oversampler; 2],
    xfmr: [Biquad; 2],
    v: [f32; NPARAMS],
    input: Smooth,
    output: Smooth,
    mix: Smooth,
    last_out: f64,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut xfmr = [Biquad::identity(); 2];
        for b in &mut xfmr {
            b.set_highpass(fs, 15.0, 0.6);
        }
        Self { fs, gr: 0.0, os: [Oversampler::new(), Oversampler::new()], xfmr, v, input: Smooth::new(fs, 20.0, 1.0), output: Smooth::new(fs, 20.0, 1.0), mix: Smooth::new(fs, 20.0, 1.0), last_out: 0.0 }
    }
    fn reset(&mut self) {
        self.gr = 0.0;
        self.last_out = 0.0;
        for c in 0..2 {
            self.os[c].reset();
            self.xfmr[c].reset();
        }
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        // input pot: 0..48 dB, with unity around 24
        self.input.set(db_to_gain(v[INPUT] as f64 - 24.0));
        self.output.set(db_to_gain(v[OUTPUT] as f64 + 18.0));
        self.mix.set(v[MIX] as f64);
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        // knob 1 = slowest; attack 800 us .. 20 us, release 1100 ms .. 50 ms
        let a_knob = (self.v[ATTACK] as f64 - 1.0) / 6.0;
        let r_knob = (self.v[RELEASE] as f64 - 1.0) / 6.0;
        let att_ms = 0.8 * (0.02f64 / 0.8).powf(a_knob);
        let rel_ms = 1100.0 * (50.0f64 / 1100.0).powf(r_knob);
        let ca = coeff(self.fs, att_ms);
        let ri = (self.v[RATIO].round() as usize).min(4);
        let (ratio, thr, knee) = RATIOS[ri];
        let all = ri == 4;
        let mut grmax = 0.0f64;
        for k in 0..n {
            let gin = self.input.next();
            let gout = self.output.next();
            let mx = self.mix.next();
            // feedback detection from the previous output sample
            let lvl = gain_to_db(self.last_out) + 6.0;
            let target = vintage::fx::comp_gr(lvl, thr, ratio, knee) * if all { 1.6 } else { 1.0 };
            // programme-dependent release: faster from shallow reduction, slower from deep (two-stage)
            let rel = rel_ms * (1.0 + 0.04 * self.gr);
            let cr = coeff(self.fs, rel);
            self.gr = if target > self.gr { target + ca * (self.gr - target) } else { target + cr * (self.gr - target) };
            grmax = grmax.max(self.gr);
            let g = db_to_gain(-self.gr);
            let mut lo = 0.0f64;
            for c in 0..nch.min(2) {
                let x = chans[c][k] as f64 * gin;
                // FET + class-A line amp: mostly second harmonic, more when 'all buttons'
                let drive = if all { 1.8 } else { 1.0 };
                let y = self.os[c].process(x * g, |s| tube(s * drive * 0.7, 0.18 + 0.1 * (drive - 1.0)) / (0.7 * drive));
                let y = self.xfmr[c].process(y);
                lo = lo.max(y.abs());
                let z = (chans[c][k] as f64 * (1.0 - mx) + y * gout * mx) as f32;
                chans[c][k] = z;
            }
            self.last_out = lo;
        }
        meters[0] = grmax as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Ford Vocal (4:1, medium)", values: &[(INPUT, 28.0), (OUTPUT, -20.0), (ATTACK, 3.0), (RELEASE, 5.0), (RATIO, 0.0)] },
        Preset { name: "Vocal Ride Gentle", values: &[(INPUT, 25.0), (OUTPUT, -19.0), (ATTACK, 2.0), (RELEASE, 4.0), (RATIO, 0.0)] },
        Preset { name: "Bass Glue (8:1)", values: &[(INPUT, 30.0), (OUTPUT, -22.0), (ATTACK, 2.0), (RELEASE, 6.0), (RATIO, 1.0)] },
        Preset { name: "Drum Room Smash (All)", values: &[(INPUT, 34.0), (OUTPUT, -26.0), (ATTACK, 7.0), (RELEASE, 7.0), (RATIO, 4.0)] },
        Preset { name: "Parallel Drums (All, 50%)", values: &[(INPUT, 36.0), (OUTPUT, -26.0), (ATTACK, 6.0), (RELEASE, 7.0), (RATIO, 4.0), (MIX, 0.5)] },
        Preset { name: "Peak Limiter (20:1)", values: &[(INPUT, 26.0), (OUTPUT, -19.0), (ATTACK, 6.0), (RELEASE, 6.0), (RATIO, 3.0)] },
        Preset { name: "Guitar Sustain (12:1)", values: &[(INPUT, 32.0), (OUTPUT, -24.0), (ATTACK, 2.0), (RELEASE, 3.0), (RATIO, 2.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "FET LIMITER",
        subtitle: ("IN THE MANNER OF THE UREI 1176", "FEEDBACK FET · 4 / 8 / 12 / 20 / ALL"),
        accent: ui::MUSTARD,
        size: (W, H),
        items: vec![

            Item::Knob { p: INPUT, x: 90.0, y: 165.0, r: 46.0, label: "INPUT", tip: "Input level: turn up to compress harder (there is no threshold).", active: None },
            Item::Meter { m: 0, x: 200.0, y: 96.0, w: 240.0, h: 138.0, kind: MeterKind::GainReduction, caption: "GAIN REDUCTION" },
            Item::Knob { p: OUTPUT, x: 550.0, y: 165.0, r: 46.0, label: "OUTPUT", tip: "Output level.", active: None },
            Item::Divider { y: 264.0 },
            Item::Knob { p: ATTACK, x: 80.0, y: 330.0, r: 24.0, label: "ATTACK", tip: "1 = slowest (about 800 us), 7 = fastest (about 20 us).", active: None },
            Item::Knob { p: RELEASE, x: 180.0, y: 330.0, r: 24.0, label: "RELEASE", tip: "1 = slowest (about 1.1 s), 7 = fastest (about 50 ms).", active: None },
            Item::Selector { p: RATIO, x: 320.0, y: 330.0, label: "RATIO", positions: &["4", "8", "12", "20", "ALL"], tip: "Ratio buttons. ALL: every button pressed at once, the famous aggressive, distorted setting." },
            Item::Knob { p: MIX, x: 470.0, y: 330.0, r: 22.0, label: "MIX", tip: "Dry/compressed blend (parallel compression).", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "FET Limiter", class_id: *b"JFFetLimiterPQD1", vendor: "PEQUOD James Ford",
    categories: [Fx, Dynamics]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
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

    #[test]
    fn harder_input_more_reduction_and_all_is_hardest() {
        let gr = |inp: f32, ratio: f32| {
            let p = with(defaults(), &[(INPUT, inp), (RATIO, ratio)]);
            let mut input = vec![sine(1000.0, 0.25, 48000)];
            run(&p, &mut input)[0]
        };
        let (a, b, c) = (gr(20.0, 0.0), gr(34.0, 0.0), gr(34.0, 4.0));
        println!("GR: input 20 {a:.1}, input 34 {b:.1}, all {c:.1}");
        assert!(b > a + 3.0 && c > b);
    }

}
