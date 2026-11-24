//! Distressor: Empirical Labs' EL8 (Dave Derr, 1995), a digitally controlled analogue compressor that
//! can be anything from a clean leveller to the "Nuke" brick-wall that crushes room mics. INPUT drives it
//! against a fixed threshold; ATTACK (0 to 10: 50 microseconds to 30 ms) and RELEASE (0 to 10: 50 ms to
//! 3.5 s); OUTPUT; RATIO 1:1 (just the colour), 2, 3, 4, 6, 10 (the opto-like one), 20 and NUKE;
//! DETECTOR (normal, HP: the side-chain high-pass, BAND: the 6 kHz emphasis for de-essing); AUDIO
//! (clean, DIST 2: second harmonic like valves, DIST 3: third like tape). From Empirical Labs' manual;
//! the curves are estimated.

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

/// The level the side chain sees, in dB: peak or RMS of the louder channel.
#[allow(dead_code)]
#[derive(Clone, Copy)]
struct Detector {
    ms: f64,
    rms: f64,
}

#[allow(dead_code)]
impl Detector {
    fn new() -> Self {
        Self { ms: 0.0, rms: 0.0 }
    }
    #[inline]
    fn peak_db(&self, x: f64) -> f64 {
        gain_to_db(x.abs().max(1e-9))
    }
    /// RMS over about `win` (a one-pole coefficient).
    #[inline]
    fn rms_db(&mut self, x: f64, win: f64) -> f64 {
        self.ms = x * x + win * (self.ms - x * x);
        self.rms = self.ms.max(1e-18).sqrt();
        gain_to_db(self.rms)
    }
}

/// Gain reduction (dB) smoothed with attack and release.
#[allow(dead_code)]
#[inline]
fn ballistics(state: &mut f64, target: f64, att: f64, rel: f64) -> f64 {
    let c = if target > *state { att } else { rel };
    *state = target + c * (*state - target);
    *state
}

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
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "release"]
    pub release: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
    #[id = "ratio"]
    pub ratio: FloatParam,
    #[id = "detector"]
    pub detector: FloatParam,
    #[id = "audio"]
    pub audio: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            input: db("Input", 6.0, 0.0, 30.0),
            attack: FloatParam::new("Attack", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            release: FloatParam::new("Release", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            output: db("Output", 0.0, -24.0, 12.0),
            ratio: FloatParam::new("Ratio", 3.0, FloatRange::Linear { min: 0.0, max: 7.0 }).with_step_size(1.0),
            detector: FloatParam::new("Detector", 0.0, FloatRange::Linear { min: 0.0, max: 2.0 }).with_step_size(1.0),
            audio: FloatParam::new("Audio", 0.0, FloatRange::Linear { min: 0.0, max: 2.0 }).with_step_size(1.0),
        }
    }
}

pub const INPUT: usize = 0;
pub const ATTACK: usize = 1;
pub const RELEASE: usize = 2;
pub const OUTPUT: usize = 3;
pub const RATIO: usize = 4;
pub const DETECTOR: usize = 5;
pub const AUDIO: usize = 6;
pub const NPARAMS: usize = 7;

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
            self.attack.as_ptr(),
            self.release.as_ptr(),
            self.output.as_ptr(),
            self.ratio.as_ptr(),
            self.detector.as_ptr(),
            self.audio.as_ptr(),
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

const RATIOS: [f64; 8] = [1.0, 2.0, 3.0, 4.0, 6.0, 10.0, 20.0, 80.0];

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    gr: f64,
    slow: f64,
    hp: Vec<Biquad>,
    band: Vec<Biquad>,
    inp: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let n = ch.max(2);
        let mut hp = Biquad::identity();
        hp.set_highpass1(fs, 100.0);
        let mut band = Biquad::identity();
        band.set_peak(fs, 6000.0, 0.8, 12.0);
        Self { fs, v, gr: 0.0, slow: 0.0, hp: vec![hp; n], band: vec![band; n], inp: Smooth::new(fs, 20.0, 1.0), out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.gr = 0.0;
        self.slow = 0.0;
        self.hp.iter_mut().chain(self.band.iter_mut()).for_each(|b| b.reset());
        self.inp.snap(db_to_gain(self.v[INPUT] as f64));
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.inp.set(db_to_gain(v[INPUT] as f64));
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let ri = (self.v[RATIO].round() as usize).min(7);
        let ratio = RATIOS[ri];
        let nuke = ri == 7;
        let opto = ri == 5;
        // 0..10 on the knobs: attack 0.05 to 30 ms, release 50 ms to 3.5 s (both logarithmic)
        let att = 0.05 * (600.0f64).powf(self.v[ATTACK] as f64 / 10.0);
        let rel = 50.0 * (70.0f64).powf(self.v[RELEASE] as f64 / 10.0);
        let ca = coeff(self.fs, if nuke { 0.05 } else { att });
        let cr = coeff(self.fs, rel / 3.0);
        let cslow = coeff(self.fs, rel * 4.0 / 3.0);
        let det = self.v[DETECTOR].round() as usize;
        let audio = self.v[AUDIO].round() as usize;
        let thr = -18.0;
        let mut most = 0.0f64;
        for k in 0..n {
            let gi = self.inp.next();
            let go = self.out.next();
            let mut peak = 0.0f64;
            for (ci, ch) in chans.iter().enumerate() {
                let j = ci.min(self.hp.len() - 1);
                let mut s = ch[k] as f64 * gi;
                if det == 1 { s = self.hp[j].process(s); }
                if det == 2 { s = self.band[j].process(s); }
                peak = peak.max(s.abs());
            }
            let lvl = gain_to_db(peak.max(1e-9));
            let target = if ri == 0 { 0.0 } else { comp_gr(lvl, thr, ratio, if nuke { 0.0 } else if ratio <= 3.0 { 10.0 } else { 4.0 }) };
            let mut gr = ballistics(&mut self.gr, target, ca, cr);
            if opto {
                // 10:1 behaves like an opto: a slow tail that remembers
                gr = gr.max(ballistics(&mut self.slow, target * 0.7, ca, cslow));
            }
            most = most.max(gr);
            let a = db_to_gain(-gr);
            for ch in chans.iter_mut() {
                let x = ch[k] as f64 * gi * a;
                let y = match audio {
                    1 => x + 0.12 * x * x - 0.04 * x * x * x,         // DIST 2: second harmonic
                    2 => (x * 1.3).tanh() / 1.3,                       // DIST 3: third, as tape
                    _ => x - 0.01 * x * x * x,
                };
                ch[k] = (y * go) as f32;
            }
        }
        meters[0] = most as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Vocal (3:1, Dist 2)", values: &[(INPUT, 8.0), (RATIO, 2.0), (ATTACK, 4.0), (RELEASE, 6.0), (AUDIO, 1.0)] },
        Preset { name: "Nuked Room Mics", values: &[(INPUT, 20.0), (RATIO, 7.0), (ATTACK, 2.0), (RELEASE, 1.0), (OUTPUT, -8.0), (AUDIO, 2.0)] },
        Preset { name: "Bass (4:1, Dist 3)", values: &[(INPUT, 10.0), (RATIO, 3.0), (ATTACK, 5.0), (RELEASE, 5.0), (AUDIO, 2.0)] },
        Preset { name: "Opto 10:1", values: &[(INPUT, 10.0), (RATIO, 5.0), (ATTACK, 6.0), (RELEASE, 7.0)] },
        Preset { name: "De-Ess (Band)", values: &[(INPUT, 6.0), (RATIO, 4.0), (DETECTOR, 2.0), (ATTACK, 1.0), (RELEASE, 2.0)] },
        Preset { name: "Just the Colour (1:1)", values: &[(RATIO, 0.0), (INPUT, 10.0), (AUDIO, 1.0), (OUTPUT, -10.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "DISTRESSOR",
        subtitle: ("EMPIRICAL LABS EL8, 1995", "FROM A CLEAN LEVELLER TO THE NUKE"),
        accent: ui::GRAPHITE,
        size: (W, H),
        items: vec![
            Item::Knob { p: INPUT, x: 80.0, y: 150.0, r: 34.0, label: "INPUT", tip: "Into a fixed threshold: more input, more compression.", active: None },
            Item::Knob { p: ATTACK, x: 190.0, y: 150.0, r: 28.0, label: "ATTACK", tip: "0 to 10: fast to slow.", active: None },
            Item::Knob { p: RELEASE, x: 290.0, y: 150.0, r: 28.0, label: "RELEASE", tip: "0 to 10: fast to slow.", active: None },
            Item::Meter { m: 0, x: 360.0, y: 90.0, w: 140.0, h: 100.0, kind: MeterKind::GainReduction, caption: "GR" },
            Item::Knob { p: OUTPUT, x: 565.0, y: 150.0, r: 34.0, label: "OUTPUT", tip: "Output level.", active: None },
            Item::Divider { y: 245.0 },
            Item::Selector { p: RATIO, x: 160.0, y: 330.0, label: "RATIO", positions: &["1", "2", "3", "4", "6", "10", "20", "NUKE"], tip: "1:1 is just the colour; 10:1 acts like an opto; NUKE is a brick wall." },
            Item::Selector { p: DETECTOR, x: 340.0, y: 330.0, label: "DETECTOR", positions: &["NORM", "HP", "BAND"], tip: "HP: the bass pushes it less. BAND: 6 kHz emphasis, a de-esser." },
            Item::Selector { p: AUDIO, x: 500.0, y: 330.0, label: "AUDIO", positions: &["CLEAN", "DIST 2", "DIST 3"], tip: "DIST 2: valve-like second harmonic. DIST 3: tape-like third." },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Distressor", class_id: *b"HWDistressorEL8Q", vendor: "PEQUOD Hardware",
    categories: [Fx, Dynamics]
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

    /// Louder in comes out much less louder: it compresses.
    fn spread(set: &[(usize, f32)]) -> f64 {
        let p = with(defaults(), set);
        let a = db(rms(&go(&p, &sine(1000.0, 0.03, 48000))[24000..]));
        let b = db(rms(&go(&p, &sine(1000.0, 0.9, 48000))[24000..]));
        b - a
    }
    #[test]
    fn compresses() {
        let s = spread(&[(INPUT, 20.0), (RATIO, 5.0)]);
        println!("+29.5 dB in -> {:.1} dB out", s);
        assert!(s < 24.0, "{s}");
    }

}
