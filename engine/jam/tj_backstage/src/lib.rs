//! Backstage 30: a solid-state Peavey Backstage combo of the late 70s, the small amp Paul Weller has said he used
//! on All Mod Cons and Setting Sons alongside his Vox AC30s. One channel: PRE gain (pull for BRIGHT), Peavey's
//! SATURATION circuit - transistor clipping that thickens and squashes the sound the way an overdriven valve
//! amp would - the POST gain into the transistor power amp, a three-band EQ, a little spring reverb, and its
//! single 10-inch speaker. Circuit behaviour estimated from period reviews and the controls as described.

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

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "pre"]
    pub pre: FloatParam,
    #[id = "bright"]
    pub bright: FloatParam,
    #[id = "sat"]
    pub sat: FloatParam,
    #[id = "low"]
    pub low: FloatParam,
    #[id = "mid"]
    pub mid: FloatParam,
    #[id = "high"]
    pub high: FloatParam,
    #[id = "reverb"]
    pub reverb: FloatParam,
    #[id = "post"]
    pub post: FloatParam,
    #[id = "speaker"]
    pub speaker: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            pre: FloatParam::new("Pre Gain", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            bright: FloatParam::new("Bright", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            sat: FloatParam::new("Saturation", 0.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            low: FloatParam::new("Low", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            mid: FloatParam::new("Mid", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            high: FloatParam::new("High", 6.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            reverb: FloatParam::new("Reverb", 0.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            post: FloatParam::new("Post Gain", 6.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            speaker: FloatParam::new("Speaker", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
        }
    }
}

pub const PRE: usize = 0;
pub const BRIGHT: usize = 1;
pub const SAT: usize = 2;
pub const LOW: usize = 3;
pub const MID: usize = 4;
pub const HIGH: usize = 5;
pub const REVERB: usize = 6;
pub const POST: usize = 7;
pub const SPEAKER: usize = 8;
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
            self.pre.as_ptr(),
            self.bright.as_ptr(),
            self.sat.as_ptr(),
            self.low.as_ptr(),
            self.mid.as_ptr(),
            self.high.as_ptr(),
            self.reverb.as_ptr(),
            self.post.as_ptr(),
            self.speaker.as_ptr(),
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

/// Peavey's saturation: transistor clipping with a hard-ish knee.
#[inline]
fn saturate(x: f64) -> f64 {
    let x = x + 0.08 * x * x.abs().min(1.0); // (a little asymmetry)
    x / (1.0 + x.abs().powi(4)).powf(0.25)
}

struct Spring {
    ap: [(f64, f64); 6],
    line: DelayLine,
    lp: OnePole,
    hp: Biquad,
    d: f64,
}

impl Spring {
    fn new(fs: f64) -> Self {
        let mut hp = Biquad::identity();
        hp.set_highpass(fs, 250.0, 0.7);
        let mut lp = OnePole::default();
        lp.set_lowpass(fs, 3800.0);
        Self { ap: [(0.0, 0.0); 6], line: DelayLine::new((fs * 0.06) as usize), lp, hp, d: fs * 0.0337 }
    }
    fn reset(&mut self) {
        self.ap = [(0.0, 0.0); 6];
        self.line.reset();
        self.lp.reset();
        self.hp.reset();
    }
    /// One small spring tank: dispersion (the "drip") and a fed-back delay.
    fn process(&mut self, x: f64) -> f64 {
        let fb = self.line.read(self.d);
        let mut y = self.hp.process(x) + 0.62 * self.lp.lp(fb);
        for (z, _) in self.ap.iter_mut() {
            // first-order allpass, g = 0.62
            let g = 0.62;
            let out = -g * y + *z;
            *z = y + g * out;
            y = out;
        }
        self.line.write(y);
        fb
    }
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    bright: Biquad,
    eq: [Biquad; 3],
    spk: [Biquad; 5],
    spring: Spring,
    dc: DcBlock,
    key: [i32; 5],
    pre: Smooth,
    sat: Smooth,
    rev: Smooth,
    post: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let v = self.v;
        let t = |p: usize| (v[p] as f64 - 5.0) / 5.0; // -1..1, 5 = flat
        // the bright pull: a cap across the pre gain, so it bites most at low gain
        let bright = if v[BRIGHT] >= 0.5 { 7.0 * (1.0 - 0.6 * v[PRE] as f64 / 10.0) } else { 0.0 };
        self.bright.set_high_shelf(fs, 1800.0, 0.7, bright);
        self.eq[0].set_low_shelf(fs, 110.0, 0.7, 12.0 * t(LOW));
        self.eq[1].set_peak(fs, 650.0, 0.8, 10.0 * t(MID));
        self.eq[2].set_high_shelf(fs, 3200.0, 0.7, 12.0 * t(HIGH));
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut spk = [Biquad::identity(); 5];
        // one 10-inch speaker in a small open-backed combo
        spk[0].set_highpass(fs, 95.0, 0.8);
        spk[1].set_peak(fs, 130.0, 1.4, 3.0);
        spk[2].set_peak(fs, 2600.0, 1.0, 4.0);
        spk[3].set_lowpass(fs, 4800.0, 0.9);
        spk[4].set_lowpass(fs, 6500.0, 0.7);
        let mut d = Self {
            fs, v, bright: Biquad::identity(), eq: [Biquad::identity(); 3], spk, spring: Spring::new(fs), dc: DcBlock::new(fs),
            key: [-1; 5],
            pre: Smooth::new(fs, 20.0, 1.0), sat: Smooth::new(fs, 20.0, 0.0), rev: Smooth::new(fs, 20.0, 0.0), post: Smooth::new(fs, 20.0, 1.0),
        };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.bright.reset();
        self.eq.iter_mut().chain(self.spk.iter_mut()).for_each(|b| b.reset());
        self.spring.reset();
        self.dc.reset();
        let v = self.v;
        self.pre.snap(Self::pre_gain(v[PRE]));
        self.sat.snap(v[SAT] as f64 / 10.0);
        self.rev.snap(v[REVERB] as f64 / 10.0);
        self.post.snap(Self::post_gain(v[POST]));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = [(v[PRE] * 10.0) as i32, v[BRIGHT].round() as i32, (v[LOW] * 20.0) as i32, (v[MID] * 20.0) as i32, (v[HIGH] * 20.0) as i32];
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.pre.set(Self::pre_gain(v[PRE]));
        self.sat.set(v[SAT] as f64 / 10.0);
        self.rev.set(v[REVERB] as f64 / 10.0);
        self.post.set(Self::post_gain(v[POST]));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nc = chans.len().max(1);
        let spk = self.v[SPEAKER] >= 0.5;
        for k in 0..n {
            let pg = self.pre.next();
            let sat = self.sat.next();
            let rev = self.rev.next();
            let post = self.post.next();
            let x = chans.iter().map(|c| c[k] as f64).sum::<f64>() / nc as f64;
            // the pre gain: transistor stages, clean until pushed
            let p = self.bright.process(x * pg);
            let p = (p * 0.5).tanh() * 2.0;
            // saturation: in, it takes over (the level drops back to where the knob sets it)
            let s = if sat > 1e-4 {
                let drive = 1.0 + 60.0 * sat * sat;
                let q = saturate(p * drive) * (0.5 + 0.5 * sat);
                (1.0 - sat.min(0.25) * 4.0) * p + sat.min(0.25) * 4.0 * q
            } else {
                p
            };
            let y = cascade(&mut self.eq, s);
            let y = y + rev * 0.7 * self.spring.process(y);
            // the transistor power amp: clean, then a hard edge at full power
            let y = (y * post * 0.5).clamp(-1.5, 1.5);
            let y = y / (1.0 + (y * 0.8).powi(8)).powf(0.125);
            let y = if spk { cascade(&mut self.spk, y) } else { y };
            let y = self.dc.process(y) as f32;
            for ch in chans.iter_mut() {
                ch[k] = y;
            }
        }
    }
}

impl Dsp {
    fn pre_gain(v: f32) -> f64 {
        db_to_gain(-12.0 + 3.6 * v as f64)
    }
    fn post_gain(v: f32) -> f64 {
        if v <= 0.0 { 0.0 } else { db_to_gain(-24.0 + 4.0 * v as f64) * 2.0 }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "All Mod Cons Rhythm", values: &[] },
        Preset { name: "Setting Sons Crunch (saturation)", values: &[(PRE, 6.5), (SAT, 4.0), (MID, 6.0), (HIGH, 6.5)] },
        Preset { name: "Bright Clean Chop", values: &[(PRE, 3.5), (BRIGHT, 1.0), (HIGH, 7.0), (LOW, 4.0)] },
        Preset { name: "Full Saturation Lead", values: &[(PRE, 8.0), (SAT, 8.5), (MID, 7.0), (HIGH, 5.5), (POST, 5.0)] },
        Preset { name: "Mr Clean (clean, a little reverb)", values: &[(PRE, 4.0), (REVERB, 2.5), (HIGH, 6.5)] },
        Preset { name: "Into the Desk (no speaker)", values: &[(PRE, 6.0), (SAT, 3.0), (SPEAKER, 0.0), (HIGH, 4.5)] },
        Preset { name: "Spring Drip", values: &[(PRE, 4.5), (REVERB, 6.0), (BRIGHT, 1.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "BACKSTAGE 30",
        subtitle: ("A LATE-70s PEAVEY BACKSTAGE COMBO", "PRE · SATURATION · POST · 1x10"),
        accent: ui::OXBLOOD,
        size: (W, H),
        items: vec![
            Item::Knob { p: PRE, x: 70.0, y: 150.0, r: 24.0, label: "PRE GAIN", tip: "The pre gain: how hard the input stages are driven.", active: None },
            Item::Switch { p: BRIGHT, x: 70.0, y: 210.0, labels: ["", "BRIGHT"], tip: "Pull the pre gain for bright: more top, mostly at low gain." },
            Item::Knob { p: SAT, x: 180.0, y: 150.0, r: 24.0, label: "SATURATION", tip: "Peavey's saturation circuit: transistor clipping that thickens and squashes like an overdriven valve amp. At zero it is out.", active: None },
            Item::Knob { p: POST, x: 290.0, y: 150.0, r: 24.0, label: "POST GAIN", tip: "The master: into the transistor power amp, which edges hard at full power.", active: None },
            Item::Knob { p: REVERB, x: 400.0, y: 150.0, r: 24.0, label: "REVERB", tip: "The little spring reverb.", active: None },
            Item::Switch { p: SPEAKER, x: 540.0, y: 150.0, labels: ["DI", "1x10"], tip: "The combo's own 10-inch speaker, or the preamp output straight to the desk." },
            Item::Divider { y: 250.0 },
            Item::Knob { p: LOW, x: 180.0, y: 320.0, r: 24.0, label: "LOW", tip: "Bass (5 is flat).", active: None },
            Item::Knob { p: MID, x: 320.0, y: 320.0, r: 24.0, label: "MID", tip: "Middle (5 is flat).", active: None },
            Item::Knob { p: HIGH, x: 460.0, y: 320.0, r: 24.0, label: "HIGH", tip: "Treble (5 is flat).", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Backstage 30", class_id: *b"TJamBackstage30Q", vendor: "PEQUOD The Jam",
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
    fn saturation_squashes() {
        let p = with(defaults(), &[(SAT, 8.0)]);
        let a = db(rms(&go(&p, &sine(220.0, 0.03, 48000))[24000..]));
        let b = db(rms(&go(&p, &sine(220.0, 0.3, 48000))[24000..]));
        let pc = with(defaults(), &[(SAT, 0.0), (PRE, 3.0)]);
        let ca = db(rms(&go(&pc, &sine(220.0, 0.03, 48000))[24000..]));
        let cb = db(rms(&go(&pc, &sine(220.0, 0.3, 48000))[24000..]));
        println!("20 dB louder in: saturated +{:.1} dB out, clean +{:.1} dB out", b - a, cb - ca);
        assert!(b - a < 6.0 && cb - ca > 15.0);
    }
    #[test]
    fn bright_bites() {
        let a = level(&[(PRE, 3.0), (BRIGHT, 0.0), (SPEAKER, 0.0)], 4000.0) - level(&[(PRE, 3.0), (BRIGHT, 0.0), (SPEAKER, 0.0)], 300.0);
        let b = level(&[(PRE, 3.0), (BRIGHT, 1.0), (SPEAKER, 0.0)], 4000.0) - level(&[(PRE, 3.0), (BRIGHT, 1.0), (SPEAKER, 0.0)], 300.0);
        println!("4 kHz over 300 Hz: bright off {a:.1}, on {b:.1}");
        assert!(b - a > 3.0);
    }
    #[test]
    fn the_speaker_rolls_off() {
        let a = level(&[(SPEAKER, 0.0), (PRE, 2.0)], 8000.0);
        let b = level(&[(SPEAKER, 1.0), (PRE, 2.0)], 8000.0);
        println!("8 kHz: DI {a:.1}, speaker {b:.1}");
        assert!(a - b > 10.0);
    }
    #[test]
    fn eq_moves() {
        for (p, f) in [(LOW, 80.0), (MID, 650.0), (HIGH, 5000.0)] {
            let lo = level(&[(p, 0.0), (PRE, 2.0), (SPEAKER, 0.0)], f);
            let hi = level(&[(p, 10.0), (PRE, 2.0), (SPEAKER, 0.0)], f);
            println!("{f} Hz: 0 {lo:.1}, 10 {hi:.1}");
            assert!(hi - lo > 12.0);
        }
    }

}
