//! Cheap Speaker: re-amping through a tiny plastic-cone speaker, like the $9.95 Radio Shack speaker Larry Fast used to rough up vocals and synths on Peter Gabriel's third album. The plastic cone breaks up and buzzes when pushed. Response estimated.

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
pub enum Size { #[name = "2 inch"] Two, #[name = "3 inch"] Three, #[name = "4 inch"] Four }
// (resonance Hz, high-pass Hz, top roll-off Hz)
const SIZES: [(f64, f64, f64); 3] = [(700.0, 450.0, 6500.0), (420.0, 280.0, 7500.0), (300.0, 200.0, 8000.0)];


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "size"]
    pub size: EnumParam<Size>,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "buzz"]
    pub buzz: FloatParam,
    #[id = "distance"]
    pub distance: FloatParam,
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
            size: EnumParam::new("Speaker", Size::Three),
            drive: pct("Drive", 0.4),
            buzz: pct("Cone Buzz", 0.4),
            distance: FloatParam::new("Mic Distance", 5.0, FloatRange::Linear { min: 1.0, max: 50.0 }).with_step_size(0.5).with_unit(" cm"),
            mix: pct("Mix", 1.0),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const SIZE: usize = 0;
pub const DRIVE: usize = 1;
pub const BUZZ: usize = 2;
pub const DISTANCE: usize = 3;
pub const MIX: usize = 4;
pub const OUTPUT: usize = 5;
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
            self.size.as_ptr(),
            self.drive.as_ptr(),
            self.buzz.as_ptr(),
            self.distance.as_ptr(),
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
    size: usize,
    hp: Biquad,
    res: Biquad,
    lp: Biquad,
    exc: OnePole,
    noise: Noise,
    buzz_f: Biquad,
    refl: DelayLine,
    os: Oversampler,
    drive: f64,
    buzz: f64,
    dist: f64,
    mix: Smooth,
    out: Smooth,
}

impl Dsp {
    fn set_size(&mut self, s: usize) {
        let (r, h, l) = SIZES[s];
        self.hp.set_highpass(self.fs, h, 0.9);
        self.res.set_peak(self.fs, r, 2.0, 8.0);
        self.lp.set_lowpass(self.fs, l, 0.9);
        self.exc.set_lowpass(self.fs, r);
        self.size = s;
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut buzz_f = Biquad::identity();
        buzz_f.set_bandpass(fs, 3500.0, 1.0);
        let mut d = Self {
            fs,
            size: 9,
            hp: Biquad::identity(),
            res: Biquad::identity(),
            lp: Biquad::identity(),
            exc: OnePole::default(),
            noise: Noise::new(17),
            buzz_f,
            refl: DelayLine::new((fs * 0.01) as usize),
            os: Oversampler::new(),
            drive: 0.4,
            buzz: 0.4,
            dist: 5.0,
            mix: Smooth::new(fs, 20.0, 1.0),
            out: Smooth::new(fs, 20.0, 1.0),
        };
        d.set_size(1);
        d
    }
    fn reset(&mut self) {
        for b in [&mut self.hp, &mut self.res, &mut self.lp, &mut self.buzz_f] {
            b.reset();
        }
        self.exc.reset();
        self.refl.reset();
        self.os.reset();
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn set_params(&mut self, v: &[f32]) {
        let s = (v[SIZE].round() as usize).min(2);
        if s != self.size {
            self.set_size(s);
        }
        self.drive = v[DRIVE] as f64;
        self.buzz = v[BUZZ] as f64;
        self.dist = v[DISTANCE] as f64;
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let mut peak = 0.0f64;
        let dg = 1.0 + 12.0 * self.drive;
        // a hard surface behind the speaker: reflection delayed by twice the mic distance
        let refl_d = (2.0 * self.dist * 0.01 / 343.0 * self.fs).max(1.0);
        let refl_g = 0.35 / (1.0 + self.dist / 10.0);
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            // cone excursion follows the low end; past its limit the plastic cone bends and rubs
            let e = self.exc.lp(x * dg);
            let y = self.os.process(x * dg, |s| {
                let a = s.abs();
                (s.signum() * (a / (1.0 + a)) + 0.08 * s * s / (1.0 + s * s)) / 0.8
            });
            let rub = (e.abs() - 0.35).max(0.0);
            let buzz = self.buzz_f.process(self.noise.next() * rub * self.buzz * 2.0 + y * rub * self.buzz);
            let y = self.lp.process(self.res.process(self.hp.process(y + buzz))) * 0.6;
            let r = self.refl.read(refl_d);
            self.refl.write(y);
            let y = y + r * refl_g;
            let mx = self.mix.next();
            let o = self.out.next();
            for c in chans.iter_mut() {
                let z = (c[k] as f64 * (1.0 - mx) + y * mx) * o;
                peak = peak.max(z.abs());
                c[k] = z as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "No Self Control Vocal", values: &[(SIZE, 1.0), (DRIVE, 0.5), (BUZZ, 0.5), (DISTANCE, 5.0)] },
        Preset { name: "Synth Line Grit", values: &[(SIZE, 0.0), (DRIVE, 0.6), (BUZZ, 0.6), (DISTANCE, 3.0)] },
        Preset { name: "Clean Small Speaker", values: &[(SIZE, 2.0), (DRIVE, 0.1), (BUZZ, 0.0), (DISTANCE, 10.0)] },
        Preset { name: "Blown Cone", values: &[(SIZE, 1.0), (DRIVE, 1.0), (BUZZ, 1.0)] },
        Preset { name: "Parallel Grit Under Vocal", values: &[(SIZE, 1.0), (DRIVE, 0.6), (BUZZ, 0.4), (MIX, 0.35)] },
        Preset { name: "Distant Speaker in a Box", values: &[(SIZE, 2.0), (DRIVE, 0.3), (DISTANCE, 40.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "CHEAP SPEAKER",
        subtitle: ("RE-AMP THROUGH A $9.95 SPEAKER", "PLASTIC CONE · BUZZ · GRIT"),
        accent: ui::ROSE,
        size: (W, H),
        items: vec![

            Item::Selector { p: SIZE, x: 90.0, y: 160.0, label: "SPEAKER", positions: &["2 IN", "3 IN", "4 IN"], tip: "Speaker size: smaller is thinner and more nasal." },
            Item::Knob { p: DRIVE, x: 220.0, y: 160.0, r: 42.0, label: "DRIVE", tip: "How hard the tiny amp and speaker are pushed.", active: None },
            Item::Knob { p: BUZZ, x: 360.0, y: 160.0, r: 42.0, label: "CONE BUZZ", tip: "The plastic cone bending and rubbing on loud low notes: where much of the distortion came from.", active: None },
            Item::Lamp { m: 0, x: 490.0, y: 160.0, label: "LEVEL", lo: 0.05, hi: 0.9 },
            Item::Divider { y: 264.0 },
            Item::Knob { p: DISTANCE, x: 120.0, y: 330.0, r: 22.0, label: "MIC DISTANCE", tip: "How far the mic is from the speaker; a hard surface behind adds a short reflection.", active: None },
            Item::Knob { p: MIX, x: 320.0, y: 330.0, r: 22.0, label: "MIX", tip: "Original vs the speaker.", active: None },
            Item::Knob { p: OUTPUT, x: 520.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Cheap Speaker", class_id: *b"TPCheapSpeakerPQ", vendor: "PEQUOD Padgham & Lillywhite",
    categories: [Fx, Distortion]
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
    fn small_speaker_has_no_bass_and_buzzes_when_driven() {
        let lvl = |f: f64, drive: f32| {
            let p = with(defaults(), &[(DRIVE, drive), (BUZZ, 0.0)]);
            let mut input = vec![sine(f, 0.1, 48000)];
            run(&p, &mut input);
            rms(&input[0][24000..])
        };
        println!("80 Hz vs 1 kHz: {:.1} dB", db(lvl(80.0, 0.0) / lvl(1000.0, 0.0)));
        assert!(db(lvl(80.0, 0.0) / lvl(1000.0, 0.0)) < -15.0);
    }

}
