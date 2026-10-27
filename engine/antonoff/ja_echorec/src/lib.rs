//! Echorec: magnetic-drum echo after the Binson Echorec that Jack Antonoff chains together and into Copicats. Four playback heads around a spinning drum: no tape wow, a valve preamp, and SWELL that builds into a wash.

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
pub enum OnOff { #[name = "Off"] Off, #[name = "On"] On }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "time"]
    pub time: FloatParam,
    #[id = "h1"]
    pub h1: EnumParam<OnOff>,
    #[id = "h2"]
    pub h2: EnumParam<OnOff>,
    #[id = "h3"]
    pub h3: EnumParam<OnOff>,
    #[id = "h4"]
    pub h4: EnumParam<OnOff>,
    #[id = "swell"]
    pub swell: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "drive"]
    pub drive: FloatParam,
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
            time: ms("Head Spacing", 75.0, 40.0, 150.0),
            h1: EnumParam::new("Head 1", OnOff::On),
            h2: EnumParam::new("Head 2", OnOff::Off),
            h3: EnumParam::new("Head 3", OnOff::On),
            h4: EnumParam::new("Head 4", OnOff::Off),
            swell: pct("Swell", 0.35),
            tone: pct("Tone", 0.5),
            drive: pct("Valve Drive", 0.3),
            mix: pct("Mix", 0.4),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const TIME: usize = 0;
pub const H1: usize = 1;
pub const H2: usize = 2;
pub const H3: usize = 3;
pub const H4: usize = 4;
pub const SWELL: usize = 5;
pub const TONE: usize = 6;
pub const DRIVE: usize = 7;
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
            self.time.as_ptr(),
            self.h1.as_ptr(),
            self.h2.as_ptr(),
            self.h3.as_ptr(),
            self.h4.as_ptr(),
            self.swell.as_ptr(),
            self.tone.as_ptr(),
            self.drive.as_ptr(),
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
    drum: DelayLine,
    fb: f64,
    lp: OnePole,
    hp: OnePole,
    tone: [Biquad; 2],
    tone_key: f64,
    spin: Lfo,
    v: [f32; NPARAMS],
    time: Smooth,
    mix: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut lp = OnePole::default();
        lp.set_lowpass(fs, 6000.0);
        let mut hp = OnePole::default();
        hp.set_lowpass(fs, 90.0);
        Self { fs, drum: DelayLine::new((fs * 0.7) as usize), fb: 0.0, lp, hp, tone: [Biquad::identity(); 2], tone_key: -1.0, spin: Lfo::default(), v, time: Smooth::new(fs, 150.0, 75.0), mix: Smooth::new(fs, 20.0, 0.4), out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.drum.reset();
        self.fb = 0.0;
        self.lp.reset();
        self.hp.reset();
        self.tone.iter_mut().for_each(|b| b.reset());
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let t = v[TONE] as f64;
        if (t - self.tone_key).abs() > 1e-3 {
            for (i, b) in self.tone.iter_mut().enumerate() {
                if i == 0 {
                    b.set_low_shelf(self.fs, 200.0, 0.7, 6.0 * (0.5 - t));
                } else {
                    b.set_high_shelf(self.fs, 2500.0, 0.7, 10.0 * (t - 0.5));
                }
            }
            self.tone_key = t;
        }
        self.time.set(v[TIME] as f64);
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let fs = self.fs;
        let heads = [self.v[H1] > 0.5, self.v[H2] > 0.5, self.v[H3] > 0.5, self.v[H4] > 0.5];
        let nh = heads.iter().filter(|h| **h).count().max(1) as f64;
        let swell = self.v[SWELL] as f64;
        let drive = 1.0 + 6.0 * self.v[DRIVE] as f64;
        let mut peak = 0.0f64;
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            // the drum turns steadily: only a trace of speed ripple
            let d = self.time.next() * 0.001 * fs * (1.0 + 0.0004 * self.spin.next(fs, 2.3));
            // valve record amp
            let rec = tube((x + self.fb * swell * 1.05) * drive * 0.5, 0.15) / (0.5 * drive).max(0.5);
            self.drum.write(rec);
            let mut e = 0.0;
            for (h, on) in heads.iter().enumerate() {
                if *on {
                    e += self.drum.read(d * (h + 1) as f64);
                }
            }
            let e = self.lp.lp(e / nh.sqrt());
            let e = e - self.hp.lp(e);
            self.fb = e;
            let w = cascade(&mut self.tone, e);
            let mx = self.mix.next();
            let o = self.out.next();
            for c in chans.iter_mut() {
                let y = (c[k] as f64 * (1.0 - mx) + w * mx) * o;
                peak = peak.max(y.abs());
                c[k] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Heads 1 + 3 (rhythmic)", values: &[(H1, 1.0), (H2, 0.0), (H3, 1.0), (H4, 0.0), (SWELL, 0.35)] },
        Preset { name: "All Four Heads Swell", values: &[(H1, 1.0), (H2, 1.0), (H3, 1.0), (H4, 1.0), (SWELL, 0.6), (MIX, 0.45)] },
        Preset { name: "Vocal Echolette-ish Slap", values: &[(TIME, 60.0), (H1, 1.0), (H2, 0.0), (H3, 0.0), (H4, 0.0), (SWELL, 0.1), (MIX, 0.3)] },
        Preset { name: "Drums Into the Drum", values: &[(TIME, 90.0), (H1, 0.0), (H2, 1.0), (H3, 0.0), (H4, 1.0), (SWELL, 0.4), (DRIVE, 0.5), (MIX, 0.35)] },
        Preset { name: "Dark Wash", values: &[(H1, 1.0), (H2, 1.0), (H3, 1.0), (H4, 1.0), (SWELL, 0.8), (TONE, 0.25), (MIX, 0.5)] },
        Preset { name: "Chained Binsons (hot)", values: &[(H1, 1.0), (H3, 1.0), (H4, 1.0), (SWELL, 0.55), (DRIVE, 0.75), (OUTPUT, -4.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "ECHOREC",
        subtitle: ("AFTER THE BINSON ECHOREC", "MAGNETIC DRUM · 4 HEADS · SWELL"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![

            Item::Knob { p: TIME, x: 90.0, y: 160.0, r: 40.0, label: "HEAD SPACING", tip: "The gap between heads (the drum's speed). Heads play at 1x, 2x, 3x and 4x.", active: None },
            Item::Switch { p: H1, x: 210.0, y: 135.0, labels: ["", "1"], tip: "Playback head 1." },
            Item::Switch { p: H2, x: 290.0, y: 135.0, labels: ["", "2"], tip: "Playback head 2." },
            Item::Switch { p: H3, x: 370.0, y: 135.0, labels: ["", "3"], tip: "Playback head 3." },
            Item::Switch { p: H4, x: 450.0, y: 135.0, labels: ["", "4"], tip: "Playback head 4." },
            Item::Text { x: 330.0, y: 105.0, text: "HEADS", size: 13.0, align: Align2::CENTER_CENTER },
            Item::Knob { p: SWELL, x: 330.0, y: 210.0, r: 30.0, label: "SWELL", tip: "Feedback: builds the repeats into a wash.", active: None },
            Item::Lamp { m: 0, x: 540.0, y: 160.0, label: "LEVEL", lo: 0.05, hi: 0.9 },
            Item::Divider { y: 264.0 },
            Item::Knob { p: TONE, x: 120.0, y: 330.0, r: 22.0, label: "TONE", tip: "Dark to bright repeats.", active: None },
            Item::Knob { p: DRIVE, x: 240.0, y: 330.0, r: 22.0, label: "VALVE DRIVE", tip: "How hard the valve record amp is pushed.", active: None },
            Item::Knob { p: MIX, x: 400.0, y: 330.0, r: 22.0, label: "MIX", tip: "Dry vs echo.", active: None },
            Item::Knob { p: OUTPUT, x: 540.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Echorec", class_id: *b"JAEchorecDrumPQ1", vendor: "PEQUOD Antonoff",
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
    fn heads_at_spacing() {
        let p = with(defaults(), &[(TIME, 80.0), (H1, 0.0), (H2, 1.0), (H3, 0.0), (H4, 0.0), (SWELL, 0.0), (MIX, 1.0)]);
        let mut x = vec![0.0f32; 96000];
        for v in x.iter_mut().skip(48000).take(48) {
            *v = 0.4;
        }
        let mut input = vec![x];
        run(&p, &mut input);
        let pk = input[0].iter().enumerate().skip(48200).max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap()).unwrap().0 as f64 / 48.0 - 1000.0;
        println!("head 2 at {pk:.1} ms (expect ~160)");
        assert!((pk - 160.0).abs() < 4.0);
    }

}
