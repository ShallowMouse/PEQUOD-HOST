//! Expander: the opposite of a compressor. A compressor makes the loud parts quieter; this makes the dynamic range bigger. UPWARD: everything over the threshold gets louder (puts the punch back into squashed drums, a decompressor). DOWNWARD: everything under the threshold gets quieter (cleans up bleed, hiss and room between notes).

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
pub enum Mode { #[name = "Downward"] Down, #[name = "Upward"] Up }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Detect { #[name = "Peak"] Peak, #[name = "RMS"] Rms }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "mode"]
    pub mode: EnumParam<Mode>,
    #[id = "thresh"]
    pub thresh: FloatParam,
    #[id = "ratio"]
    pub ratio: FloatParam,
    #[id = "range"]
    pub range: FloatParam,
    #[id = "knee"]
    pub knee: FloatParam,
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "release"]
    pub release: FloatParam,
    #[id = "detect"]
    pub detect: EnumParam<Detect>,
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
            mode: EnumParam::new("Mode", Mode::Up),
            thresh: db("Threshold", -24.0, -60.0, 0.0),
            ratio: FloatParam::new("Ratio", 1.5, FloatRange::Skewed { min: 1.0, max: 4.0, factor: FloatRange::skew_factor(-1.0) }).with_step_size(0.01).with_value_to_string(Arc::new(|v| format!("1:{:.2}", v))),
            range: db("Range", 12.0, 1.0, 40.0),
            knee: db("Knee", 6.0, 0.0, 24.0),
            attack: ms("Attack", 5.0, 0.1, 100.0),
            release: ms("Release", 150.0, 5.0, 2000.0),
            detect: EnumParam::new("Detector", Detect::Rms),
            mix: pct("Mix", 1.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const MODE: usize = 0;
pub const THRESH: usize = 1;
pub const RATIO: usize = 2;
pub const RANGE: usize = 3;
pub const KNEE: usize = 4;
pub const ATTACK: usize = 5;
pub const RELEASE: usize = 6;
pub const DETECT: usize = 7;
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
            self.mode.as_ptr(),
            self.thresh.as_ptr(),
            self.ratio.as_ptr(),
            self.range.as_ptr(),
            self.knee.as_ptr(),
            self.attack.as_ptr(),
            self.release.as_ptr(),
            self.detect.as_ptr(),
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
    rms: f64,
    gain_db: f64,
    v: [f32; NPARAMS],
    mix: Smooth,
    out: Smooth,
}

impl Dsp {
    /// Static curve: gain change in dB (positive = louder) for a level in dB.
    pub fn curve(level: f64, thresh: f64, ratio: f64, range: f64, knee: f64, up: bool) -> f64 {
        let slope = ratio - 1.0;
        // signed distance from threshold, softened through the knee
        let d = level - thresh;
        let soft = if knee > 0.0 && d.abs() < knee * 0.5 {
            let t = d + knee * 0.5;
            if up { t * t / (2.0 * knee) } else { -(knee * 0.5 - d).powi(2) / (2.0 * knee) }
        } else if up {
            d.max(0.0)
        } else {
            d.min(0.0)
        };
        (soft * slope).clamp(-range, range)
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, rms: 0.0, gain_db: 0.0, v, mix: Smooth::new(fs, 20.0, 1.0), out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.rms = 0.0;
        self.gain_db = 0.0;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let p = |i: usize| self.v[i] as f64;
        let (up, thr, ratio, range, knee) = (p(MODE) > 0.5, p(THRESH), p(RATIO), p(RANGE), p(KNEE));
        let rms_mode = p(DETECT) > 0.5;
        let ca = coeff(self.fs, p(ATTACK));
        let cr = coeff(self.fs, p(RELEASE));
        let cav = coeff(self.fs, 10.0);
        let mut most = 0.0f64;
        for k in 0..n {
            let mut key = 0.0f64;
            for c in chans.iter() {
                key = key.max((c[k] as f64).abs());
            }
            let level = if rms_mode {
                self.rms = key * key + cav * (self.rms - key * key);
                10.0 * self.rms.max(1e-12).log10() + 3.01
            } else {
                gain_to_db(key)
            };
            let target = Dsp::curve(level, thr, ratio, range, knee, up);
            // attack = how fast the gain moves toward "louder" (upward) or "open" (downward); release = the way back
            let opening = if up { target > self.gain_db } else { target > self.gain_db };
            let c = if opening { ca } else { cr };
            self.gain_db = target + c * (self.gain_db - target);
            most = if self.gain_db.abs() > most.abs() { self.gain_db } else { most };
            let g = db_to_gain(self.gain_db);
            let mx = self.mix.next();
            let o = self.out.next();
            for ch in chans.iter_mut() {
                let x = ch[k] as f64;
                let y = (x * (1.0 - mx) + x * g * mx) * o;
                // upward expansion can push past full scale: a soft safety ceiling just above 0 dBFS
                let y = if y.abs() > 0.9 { y.signum() * (0.9 + 0.1 * ((y.abs() - 0.9) / 0.1).tanh()) } else { y };
                ch[k] = y as f32;
            }
        }
        meters[0] = most.abs() as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Decompress Squashed Drums", values: &[(MODE, 1.0), (THRESH, -18.0), (RATIO, 1.6), (RANGE, 9.0), (ATTACK, 1.0), (RELEASE, 80.0), (DETECT, 0.0)] },
        Preset { name: "Bring Back the Snare Crack", values: &[(MODE, 1.0), (THRESH, -14.0), (RATIO, 2.2), (RANGE, 8.0), (ATTACK, 0.3), (RELEASE, 60.0), (DETECT, 0.0), (OUTPUT, -4.0)] },
        Preset { name: "Lively Mix Bus", values: &[(MODE, 1.0), (THRESH, -20.0), (RATIO, 1.2), (RANGE, 4.0), (ATTACK, 10.0), (RELEASE, 250.0), (OUTPUT, -2.0)] },
        Preset { name: "Vocal Breath Clean-Up", values: &[(MODE, 0.0), (THRESH, -40.0), (RATIO, 2.0), (RANGE, 12.0), (ATTACK, 2.0), (RELEASE, 150.0)] },
        Preset { name: "Tom Bleed Reducer", values: &[(MODE, 0.0), (THRESH, -30.0), (RATIO, 3.0), (RANGE, 20.0), (ATTACK, 0.5), (RELEASE, 200.0), (DETECT, 0.0)] },
        Preset { name: "Amp Hiss Tamer", values: &[(MODE, 0.0), (THRESH, -50.0), (RATIO, 2.5), (RANGE, 15.0), (ATTACK, 5.0), (RELEASE, 300.0)] },
        Preset { name: "Room Mic Tightener", values: &[(MODE, 0.0), (THRESH, -24.0), (RATIO, 1.8), (RANGE, 10.0), (ATTACK, 1.0), (RELEASE, 120.0)] },
        Preset { name: "Guitar Pick Attack", values: &[(MODE, 1.0), (THRESH, -22.0), (RATIO, 1.8), (RANGE, 6.0), (ATTACK, 0.5), (RELEASE, 40.0), (DETECT, 0.0)] },
        Preset { name: "Parallel Punch", values: &[(MODE, 1.0), (THRESH, -24.0), (RATIO, 2.5), (RANGE, 12.0), (ATTACK, 0.5), (RELEASE, 70.0), (MIX, 0.5)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "EXPANDER",
        subtitle: ("THE OPPOSITE OF A COMPRESSOR", "UPWARD · DOWNWARD · MORE DYNAMICS"),
        accent: ui::OLIVE,
        size: (W, H),
        items: vec![

            Item::Switch { p: MODE, x: 90.0, y: 120.0, labels: ["DOWN", "UP"], tip: "UP: everything over the threshold gets louder (undoes compression, adds punch). DOWN: everything under the threshold gets quieter (cleans up bleed, hiss and room)." },
            Item::Switch { p: DETECT, x: 90.0, y: 205.0, labels: ["PEAK", "RMS"], tip: "Peak follows every transient (sharper, punchier). RMS follows the average level (smoother)." },
            Item::Knob { p: THRESH, x: 215.0, y: 165.0, r: 42.0, label: "THRESHOLD", tip: "Where expansion starts. Upward: above this gets louder. Downward: below this gets quieter.", active: None },
            Item::Meter { m: 0, x: 300.0, y: 96.0, w: 190.0, h: 128.0, kind: MeterKind::GainReduction, caption: "GAIN CHANGE" },
            Item::Knob { p: RATIO, x: 560.0, y: 165.0, r: 42.0, label: "RATIO", tip: "Expansion ratio: 1:2 means every 1 dB past the threshold becomes 2 dB. 1:1 = off. (A compressor's ratio, turned upside down.)", active: None },
            Item::Divider { y: 264.0 },
            Item::Knob { p: RANGE, x: 70.0, y: 330.0, r: 22.0, label: "RANGE", tip: "The most it will ever change the level (dB), up or down.", active: None },
            Item::Knob { p: KNEE, x: 160.0, y: 330.0, r: 22.0, label: "KNEE", tip: "How gradually expansion starts around the threshold.", active: None },
            Item::Knob { p: ATTACK, x: 250.0, y: 330.0, r: 22.0, label: "ATTACK", tip: "How fast it reacts. Short = the transient itself gets expanded.", active: None },
            Item::Knob { p: RELEASE, x: 340.0, y: 330.0, r: 22.0, label: "RELEASE", tip: "How fast it lets go.", active: None },
            Item::Knob { p: MIX, x: 450.0, y: 330.0, r: 22.0, label: "MIX", tip: "Dry vs expanded (parallel expansion).", active: None },
            Item::Knob { p: OUTPUT, x: 560.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level. Upward expansion makes peaks louder, so you may want this down. A soft safety ceiling stops overs.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Expander", class_id: *b"PQExpanderPQD001", vendor: "PEQUOD",
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

    fn out_db(set: &[(usize, f32)], amp: f64) -> f64 {
        let p = with(defaults(), set);
        let mut input = vec![sine(1000.0, amp, 48000)];
        run(&p, &mut input);
        db(rms(&input[0][36000..]))
    }
    #[test]
    fn upward_mode_widens_above_threshold() {
        let s = [(MODE, 1.0), (THRESH, -40.0), (RATIO, 2.0), (RANGE, 40.0), (KNEE, 0.0), (OUTPUT, -24.0)];
        let (a, b) = (out_db(&s, 0.1), out_db(&s, 0.05));
        let slope = (a - b) / 6.02;
        println!("upward 1:2 slope {slope:.2} (expect 2)");
        assert!((slope - 2.0).abs() < 0.1);
    }
    #[test]
    fn downward_mode_widens_below_threshold() {
        let s = [(MODE, 0.0), (THRESH, -20.0), (RATIO, 2.0), (RANGE, 40.0), (KNEE, 0.0)];
        let (a, b) = (out_db(&s, 0.02), out_db(&s, 0.01));
        let slope = (a - b) / 6.02;
        let above = out_db(&s, 0.5) - db(0.5 / 2f64.sqrt());
        println!("downward 1:2 slope {slope:.2} (expect 2), above threshold change {above:.2} dB");
        assert!((slope - 2.0).abs() < 0.1 && above.abs() < 0.2);
    }
    #[test]
    fn range_limits_the_change() {
        let s = [(MODE, 0.0), (THRESH, -10.0), (RATIO, 4.0), (RANGE, 6.0), (KNEE, 0.0)];
        let ch = out_db(&s, 0.01) - db(0.01 / 2f64.sqrt());
        println!("range-limited change {ch:.2} dB");
        assert!((ch + 6.0).abs() < 0.3);
    }

}
