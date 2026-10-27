//! AMS 15-80S: stereo digital delay after the AMS DMX 15-80S that Martin Hannett was among the first to own. Band-limited, 15-bit converters, long delays, modulation and the optional pitch changer. Hannett liked irregular, unmusical delay times; the presets lean that way.

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
use vintage::fx::{quantise, PitchShift};

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
    #[id = "dlyl"]
    pub delay_l: FloatParam,
    #[id = "dlyr"]
    pub delay_r: FloatParam,
    #[id = "feedback"]
    pub feedback: FloatParam,
    #[id = "cross"]
    pub cross: FloatParam,
    #[id = "modrate"]
    pub mod_rate: FloatParam,
    #[id = "moddepth"]
    pub mod_depth: FloatParam,
    #[id = "pitch"]
    pub pitch: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            input: db("Input", 0.0, -18.0, 12.0),
            delay_l: FloatParam::new("Delay A", 233.0, FloatRange::Skewed { min: 1.0, max: 1600.0, factor: FloatRange::skew_factor(-1.5) }).with_step_size(0.1).with_unit(" ms"),
            delay_r: FloatParam::new("Delay B", 377.0, FloatRange::Skewed { min: 1.0, max: 1600.0, factor: FloatRange::skew_factor(-1.5) }).with_step_size(0.1).with_unit(" ms"),
            feedback: pct("Feedback", 0.3),
            cross: pct("Cross Feed", 0.0),
            mod_rate: rate("Mod Rate", 0.5, 0.05, 10.0),
            mod_depth: pct("Mod Depth", 0.0),
            pitch: FloatParam::new("Pitch", 0.0, FloatRange::Linear { min: -12.0, max: 12.0 }).with_step_size(0.1).with_unit(" st"),
            mix: pct("Mix", 0.35),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const INPUT: usize = 0;
pub const DLYL: usize = 1;
pub const DLYR: usize = 2;
pub const FEEDBACK: usize = 3;
pub const CROSS: usize = 4;
pub const MODRATE: usize = 5;
pub const MODDEPTH: usize = 6;
pub const PITCH: usize = 7;
pub const MIX: usize = 8;
pub const OUTPUT: usize = 9;
pub const NPARAMS: usize = 10;

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
            self.delay_l.as_ptr(),
            self.delay_r.as_ptr(),
            self.feedback.as_ptr(),
            self.cross.as_ptr(),
            self.mod_rate.as_ptr(),
            self.mod_depth.as_ptr(),
            self.pitch.as_ptr(),
            self.mix.as_ptr(),
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
    lines: [DelayLine; 2],
    aa: [[Biquad; 4]; 2],
    shift: [PitchShift; 2],
    lfo: Lfo,
    fb: [f64; 2],
    dc: [DcBlock; 2],
    input: Smooth,
    dl: [Smooth; 2],
    feedback: f64,
    cross: f64,
    mod_rate: f64,
    mod_depth: f64,
    ratio: f64,
    mix: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut aa = [[Biquad::identity(); 4]; 2];
        for c in &mut aa {
            // ~18 kHz audio bandwidth, steep converter filters (8th order)
            let f = 17500f64.min(fs * 0.42);
            c[0].set_lowpass(fs, f, 0.51);
            c[1].set_lowpass(fs, f, 0.6);
            c[2].set_lowpass(fs, f, 0.9);
            c[3].set_lowpass(fs, f, 2.56);
        }
        Self {
            fs,
            lines: [DelayLine::new((fs * 1.7) as usize), DelayLine::new((fs * 1.7) as usize)],
            aa,
            shift: [PitchShift::new(fs, 45.0), PitchShift::new(fs, 47.0)],
            lfo: Lfo::default(),
            fb: [0.0; 2],
            dc: [DcBlock::new(fs), DcBlock::new(fs)],
            input: Smooth::new(fs, 20.0, 1.0),
            dl: [Smooth::new(fs, 120.0, 233.0), Smooth::new(fs, 120.0, 377.0)],
            feedback: 0.3,
            cross: 0.0,
            mod_rate: 0.5,
            mod_depth: 0.0,
            ratio: 1.0,
            mix: Smooth::new(fs, 20.0, 0.35),
            out: Smooth::new(fs, 20.0, 1.0),
        }
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.lines[c].reset();
            self.aa[c].iter_mut().for_each(|b| b.reset());
            self.shift[c].reset();
            self.dc[c].reset();
        }
        self.fb = [0.0; 2];
    }
    fn set_params(&mut self, v: &[f32]) {
        self.input.set(db_to_gain(v[INPUT] as f64));
        self.dl[0].set(v[DLYL] as f64);
        self.dl[1].set(v[DLYR] as f64);
        self.feedback = v[FEEDBACK] as f64 * 0.98;
        self.cross = v[CROSS] as f64;
        self.mod_rate = v[MODRATE] as f64;
        self.mod_depth = v[MODDEPTH] as f64;
        self.ratio = 2f64.powf(v[PITCH] as f64 / 12.0);
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let mut peak = 0.0f64;
        for k in 0..n {
            let g = self.input.next();
            let xl = chans[0][k] as f64;
            let xr = if nch > 1 { chans[1][k] as f64 } else { xl };
            let m = self.lfo.next(self.fs, self.mod_rate);
            let mut wet = [0.0; 2];
            for c in 0..2 {
                let x = if c == 0 { xl } else { xr } * g;
                let fbin = self.fb[c] * (1.0 - self.cross) + self.fb[1 - c] * self.cross;
                // A/D: band-limit, 15-bit, (headroom ~ +6 dB over full scale here)
                let mut a = x + fbin * self.feedback;
                for b in &mut self.aa[c] {
                    a = b.process(a);
                }
                let a = quantise(a * 0.5, 15) * 2.0;
                self.lines[c].write(a);
                // modulation: up to +/-5 ms, opposite phase on B
                let dm = self.mod_depth * 5.0 * if c == 0 { m } else { -m };
                let d = ((self.dl[c].next() + dm) * 0.001 * self.fs).max(2.0);
                let y = self.lines[c].read(d);
                let y = if (self.ratio - 1.0).abs() > 1e-4 { self.shift[c].process(y, self.ratio) } else { y };
                let y = self.dc[c].process(y);
                self.fb[c] = y;
                wet[c] = y;
            }
            let mx = self.mix.next();
            let o = self.out.next();
            let yl = (xl * (1.0 - mx) + wet[0] * mx) * o;
            let yr = (xr * (1.0 - mx) + wet[1] * mx) * o;
            peak = peak.max(yl.abs()).max(yr.abs());
            if nch > 1 {
                chans[0][k] = yl as f32;
                chans[1][k] = yr as f32;
            } else {
                chans[0][k] = (0.5 * (yl + yr)) as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Disorder Drums (odd times)", values: &[(DLYL, 233.0), (DLYR, 377.0), (FEEDBACK, 0.3), (MIX, 0.35)] },
        Preset { name: "Unknown Pleasures Snare", values: &[(DLYL, 61.0), (DLYR, 89.0), (FEEDBACK, 0.45), (CROSS, 0.4), (MIX, 0.4)] },
        Preset { name: "Vocal Distance", values: &[(DLYL, 113.0), (DLYR, 151.0), (FEEDBACK, 0.25), (MIX, 0.28)] },
        Preset { name: "Long Cold Repeats", values: &[(DLYL, 790.0), (DLYR, 1130.0), (FEEDBACK, 0.55), (CROSS, 0.5), (MIX, 0.35)] },
        Preset { name: "Short Metallic Slap", values: &[(DLYL, 17.0), (DLYR, 23.0), (FEEDBACK, 0.6), (MIX, 0.3)] },
        Preset { name: "Chorused Guitar", values: &[(DLYL, 12.0), (DLYR, 18.0), (MODDEPTH, 0.3), (MODRATE, 0.7), (FEEDBACK, 0.1), (MIX, 0.45)] },
        Preset { name: "Pitch Changer Octave Down", values: &[(DLYL, 40.0), (DLYR, 55.0), (PITCH, -12.0), (FEEDBACK, 0.2), (MIX, 0.4)] },
        Preset { name: "Detuned Doubler", values: &[(DLYL, 22.0), (DLYR, 31.0), (PITCH, 0.2), (FEEDBACK, 0.0), (MIX, 0.5)] },
        Preset { name: "Ping-Pong 15-80S", values: &[(DLYL, 300.0), (DLYR, 600.0), (FEEDBACK, 0.5), (CROSS, 1.0), (MIX, 0.35)] },
        Preset { name: "Rising Pitch Repeats", values: &[(DLYL, 250.0), (DLYR, 350.0), (PITCH, 3.0), (FEEDBACK, 0.6), (MIX, 0.35)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "AMS 15-80S",
        subtitle: ("AFTER THE AMS DMX 15-80S", "STEREO DIGITAL DELAY · 1978"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![

            Item::Knob { p: DLYL, x: 90.0, y: 160.0, r: 40.0, label: "DELAY A", tip: "Left delay, 1 ms to 1.6 s. Hannett set times by ear rather than to the tempo.", active: None },
            Item::Knob { p: DLYR, x: 210.0, y: 160.0, r: 40.0, label: "DELAY B", tip: "Right delay (the S in 15-80S: stereo).", active: None },
            Item::Knob { p: FEEDBACK, x: 330.0, y: 160.0, r: 34.0, label: "FEEDBACK", tip: "Regeneration.", active: None },
            Item::Knob { p: CROSS, x: 430.0, y: 160.0, r: 26.0, label: "CROSS", tip: "Feed each channel's repeats into the other.", active: None },
            Item::Knob { p: PITCH, x: 540.0, y: 160.0, r: 34.0, label: "PITCH", tip: "Pitch changer on the delayed signal, +/- 12 semitones.", active: None },
            Item::Divider { y: 264.0 },
            Item::Knob { p: INPUT, x: 70.0, y: 330.0, r: 22.0, label: "INPUT", tip: "Input level into the converters. Push it to hear the 15-bit edge.", active: None },
            Item::Knob { p: MODRATE, x: 170.0, y: 330.0, r: 22.0, label: "MOD RATE", tip: "Delay modulation speed.", active: None },
            Item::Knob { p: MODDEPTH, x: 270.0, y: 330.0, r: 22.0, label: "MOD DEPTH", tip: "Delay modulation depth, up to +/- 5 ms.", active: None },
            Item::Knob { p: MIX, x: 420.0, y: 330.0, r: 22.0, label: "MIX", tip: "Dry vs delayed.", active: None },
            Item::Knob { p: OUTPUT, x: 550.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "AMS 15-80S", class_id: *b"MHAms1580SPQD001", vendor: "PEQUOD Hannett",
    categories: [Fx, Delay]
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
    fn delays_land_where_set() {
        let p = with(defaults(), &[(DLYL, 100.0), (DLYR, 250.0), (FEEDBACK, 0.0), (MIX, 1.0)]);
        let mut l = vec![0.0f32; 96000];
        l[48000] = 0.5;
        let r = l.clone();
        let mut input = vec![l, r];
        run(&p, &mut input);
        let at = |c: &Vec<f32>| c.iter().enumerate().skip(48200).max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap()).unwrap().0 as f64 / 48.0 - 1000.0;
        let (a, b) = (at(&input[0]), at(&input[1]));
        println!("A {a:.1} ms, B {b:.1} ms");
        assert!((a - 100.0).abs() < 1.5 && (b - 250.0).abs() < 1.5);
    }

}
