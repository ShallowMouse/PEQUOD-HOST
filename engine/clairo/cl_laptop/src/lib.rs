//! Laptop Diary: Clairo's first songs - 4EVER, Flaming Hot Cheetos, the demo of How Did I Ever, and Pretty
//! Girl's webcam video - were made in GarageBand on her Mac (Mixdown). This is that: a laptop's built-in
//! microphone (thin, a bump in the presence range, the fan and room hiss under it, and the noise
//! suppression ducking between phrases), GarageBand's track compressor, and its echo and reverb sends.
//! Estimated.

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
    #[id = "mic"]
    pub mic: FloatParam,
    #[id = "distance"]
    pub distance: FloatParam,
    #[id = "hiss"]
    pub hiss: FloatParam,
    #[id = "suppress"]
    pub suppress: FloatParam,
    #[id = "comp"]
    pub comp: FloatParam,
    #[id = "echo"]
    pub echo: FloatParam,
    #[id = "echotime"]
    pub echotime: FloatParam,
    #[id = "reverb"]
    pub reverb: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            mic: pct("Built-in Mic", 0.8),
            distance: pct("Distance", 0.3),
            hiss: pct("Fan & Hiss", 0.3),
            suppress: pct("Noise Suppression", 0.4),
            comp: pct("Compressor", 0.5),
            echo: pct("Echo", 0.2),
            echotime: ms("Echo Time", 280.0, 60.0, 800.0),
            reverb: pct("Reverb", 0.25),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const MIC: usize = 0;
pub const DISTANCE: usize = 1;
pub const HISS: usize = 2;
pub const SUPPRESS: usize = 3;
pub const COMP: usize = 4;
pub const ECHO: usize = 5;
pub const ECHOTIME: usize = 6;
pub const REVERB: usize = 7;
pub const OUTPUT: usize = 8;
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
            self.mic.as_ptr(),
            self.distance.as_ptr(),
            self.hiss.as_ptr(),
            self.suppress.as_ptr(),
            self.comp.as_ptr(),
            self.echo.as_ptr(),
            self.echotime.as_ptr(),
            self.reverb.as_ptr(),
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

#[allow(unused_imports)]
use vintage::verb::Fdn;
/// Per-channel state, grown to however many channels come.
fn per<T: Clone>(n: usize, t: T) -> Vec<T> {
    vec![t; n.max(2)]
}
/// A simple echo with a darkening repeat (shared by several here).
#[allow(dead_code)]
#[derive(Clone)]
struct Echo {
    line: DelayLine,
    lp: OnePole,
    hp: OnePole,
}
#[allow(dead_code)]
impl Echo {
    fn new(fs: f64, max_s: f64) -> Self {
        Self { line: DelayLine::new((fs * max_s) as usize + 8), lp: OnePole::default(), hp: OnePole::default() }
    }
    fn reset(&mut self) {
        self.line.reset();
        self.lp.reset();
        self.hp.reset();
    }
    #[inline]
    fn process(&mut self, x: f64, d: f64, fb: f64) -> f64 {
        let y = self.line.read(d);
        let r = self.hp.hp(self.lp.lp(y));
        self.line.write(x + (fb * r).tanh());
        y
    }
}

// -------------------------------------------------------------------- dsp --

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    hp: Vec<[Biquad; 2]>,
    peak: Vec<Biquad>,
    lp: Vec<Biquad>,
    env: f64,
    gate: f64,
    gr: f64,
    noise: Noise,
    hum: f64,
    echo: Vec<Echo>,
    verb: Fdn,
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut verb = Fdn::new(fs, 1.0);
        verb.configure(0.7, [0.9, 0.8, 0.5], 4000.0);
        Self { fs, v, hp: per(ch, [Biquad::identity(); 2]), peak: per(ch, Biquad::identity()), lp: per(ch, Biquad::identity()), env: 0.0, gate: 1.0, gr: 1.0, noise: Noise::new(2017), hum: 0.0, echo: per(ch, Echo::new(fs, 0.9)), verb }
    }
    fn reset(&mut self) {
        self.hp.iter_mut().flatten().chain(self.peak.iter_mut()).chain(self.lp.iter_mut()).for_each(|b| b.reset());
        self.echo.iter_mut().for_each(|e| e.reset());
        self.verb.reset();
        self.env = 0.0;
        self.gate = 1.0;
        self.gr = 1.0;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let p = self.v;
        let mic = p[MIC] as f64;
        // the built-in mic: high-passed hard, a presence lift, a top that stops early
        for c in 0..self.hp.len() {
            self.hp[c][0].set_highpass(fs, 60.0 + 180.0 * mic, 0.7);
            self.hp[c][1].set_highpass(fs, 60.0 + 180.0 * mic, 0.7);
            self.peak[c].set_peak(fs, 3200.0, 1.2, 7.0 * mic);
            self.lp[c].set_lowpass(fs, 18000.0 - 9000.0 * mic, 0.7);
        }
        for e in self.echo.iter_mut() {
            e.lp.set_lowpass(fs, 4000.0);
            e.hp.set_lowpass(fs, 150.0);
        }
        let dist = p[DISTANCE] as f64;
        let near = 1.0 - 0.7 * dist;
        let hiss = p[HISS] as f64 * 0.004;
        let sup = p[SUPPRESS] as f64;
        let comp = p[COMP] as f64;
        let thr = db_to_gain(-6.0 - 24.0 * comp);
        let (ca, cr) = (coeff(fs, 5.0), coeff(fs, 120.0));
        let (ga, gr_) = (coeff(fs, 15.0), coeff(fs, 250.0));
        let ed = p[ECHOTIME] as f64 * 0.001 * fs;
        let (ew, rw) = (p[ECHO] as f64, p[REVERB] as f64);
        let out_g = db_to_gain(p[OUTPUT] as f64);
        let nch = chans.len();
        for k in 0..n {
            let x_in = chans.iter().map(|c| c[k] as f64).sum::<f64>() / nch.max(1) as f64;
            // fan: a low, wandering hum with hiss
            self.hum = 0.999 * self.hum + 0.001 * self.noise.next();
            let floor = (self.noise.next() * 0.6 + self.hum * 6.0) * hiss;
            let lvl = x_in.abs();
            self.env = lvl + if lvl > self.env { ca } else { cr } * (self.env - lvl);
            // noise suppression: ducks the floor (and a little of the tail) between phrases
            let open = if self.env > 0.01 { 1.0 } else { 1.0 - sup };
            self.gate = open + if open > self.gate { ga } else { gr_ } * (self.gate - open);
            let target = if self.env > thr { (thr / self.env).powf(0.75) } else { 1.0 };
            self.gr = target + if target < self.gr { ca } else { cr } * (self.gr - target);
            let mut wet = 0.0;
            for (c, ch) in chans.iter_mut().enumerate() {
                let j = c.min(self.hp.len() - 1);
                let mut x = ch[k] as f64 * near + floor;
                x = self.hp[j][0].process(x);
                x = self.hp[j][1].process(x);
                x = self.peak[j].process(x);
                x = self.lp[j].process(x);
                x *= self.gate * self.gr * (1.0 + 2.0 * comp);
                let e = self.echo[j].process(x, ed, 0.35);
                let y = x + e * ew;
                wet += y;
                ch[k] = y as f32;
            }
            let room = self.verb.diffuse(wet / nch.max(1) as f64 * (0.3 + dist), 2);
            let (l, r) = self.verb.process(room);
            for (c, ch) in chans.iter_mut().enumerate() {
                let rv = if c % 2 == 0 { l } else { r };
                ch[k] = ((ch[k] as f64 + rv * rw) * out_g) as f32;
            }
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Pretty Girl (webcam)", values: &[(MIC, 1.0), (DISTANCE, 0.5), (HISS, 0.45), (SUPPRESS, 0.6), (COMP, 0.5), (ECHO, 0.1), (REVERB, 0.2)] },
        Preset { name: "4EVER Vocal", values: &[(MIC, 0.7), (DISTANCE, 0.15), (HISS, 0.2), (COMP, 0.6), (ECHO, 0.3), (ECHOTIME, 320.0), (REVERB, 0.35)] },
        Preset { name: "Bedroom Guitar", values: &[(MIC, 0.9), (DISTANCE, 0.4), (HISS, 0.3), (SUPPRESS, 0.0), (COMP, 0.3), (ECHO, 0.0), (REVERB, 0.2)] },
        Preset { name: "Just the Laptop", values: &[(MIC, 1.0), (DISTANCE, 0.3), (HISS, 0.4), (SUPPRESS, 0.4), (COMP, 0.0), (ECHO, 0.0), (REVERB, 0.0)] },
    ]
}

fn panel() -> Panel {
    let k = |p, x, y, r, label, tip| Item::Knob { p, x, y, r, label, tip, active: None };
    Panel {
        title: "LAPTOP DIARY",
        subtitle: ("THE BUILT-IN MIC AND GARAGEBAND", "4EVER · FLAMING HOT CHEETOS · PRETTY GIRL"),
        accent: ui::ROSE,
        size: (W, H),
        items: vec![
            k(MIC, 90.0, 170.0, 30.0, "BUILT-IN MIC", "How much of the laptop mic's thin, honky colour."),
            k(DISTANCE, 210.0, 170.0, 26.0, "DISTANCE", "How far from the laptop: quieter and roomier."),
            k(HISS, 320.0, 170.0, 24.0, "FAN & HISS", "The fan and the mic's noise floor."),
            k(SUPPRESS, 430.0, 170.0, 24.0, "SUPPRESSION", "The noise suppression ducking between phrases."),
            k(OUTPUT, 550.0, 170.0, 26.0, "OUTPUT", "Output level."),
            k(COMP, 120.0, 320.0, 26.0, "COMPRESSOR", "GarageBand's track compressor."),
            k(ECHO, 240.0, 320.0, 24.0, "ECHO", "The echo send."),
            k(ECHOTIME, 340.0, 320.0, 22.0, "TIME", "Echo time."),
            k(REVERB, 450.0, 320.0, 24.0, "REVERB", "The reverb send."),
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Laptop Diary", class_id: *b"CLClairoLaptopDy", vendor: "PEQUOD Clairo",
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

    /// At its defaults a steady tone comes through at a sensible level.
    #[test]
    fn passes_signal() {
        let y = go(&defaults(), &sine(1000.0, 0.1, 48000));
        let g = db(rms(&y[24000..]) / rms(&sine(1000.0, 0.1, 48000)[24000..]));
        println!("gain at defaults {g:.1} dB");
        assert!(g > -15.0 && g < 12.0, "{g}");
    }

}
