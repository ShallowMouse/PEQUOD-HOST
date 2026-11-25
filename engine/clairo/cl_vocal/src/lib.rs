//! Immunity Vocal: the vocal on Immunity - an 1176-style FET compressor (the CLA-76 Rostam used on the vocal
//! stems, per the Mix with the Masters session) and the echo he loaded onto her voice to carry its soft,
//! close delivery (as he described it). INPUT drives the FET; RATIO 4, 8, 12, 20 or all buttons in;
//! ATTACK and RELEASE as on the 1176 (faster clockwise). Estimated.

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
    #[id = "input"]
    pub input: FloatParam,
    #[id = "ratio"]
    pub ratio: FloatParam,
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "release"]
    pub release: FloatParam,
    #[id = "echo"]
    pub echo: FloatParam,
    #[id = "time"]
    pub time: FloatParam,
    #[id = "feedback"]
    pub feedback: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            input: db("Input", 0.0, -12.0, 24.0),
            ratio: FloatParam::new("Ratio", 0.0, FloatRange::Linear { min: 0.0, max: 4.0 }).with_step_size(1.0),
            attack: pct("Attack", 0.5),
            release: pct("Release", 0.5),
            echo: pct("Echo", 0.3),
            time: ms("Echo Time", 240.0, 60.0, 900.0),
            feedback: pct("Repeats", 0.3),
            tone: hz("Echo Tone", 3500.0, 1000.0, 12000.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const INPUT: usize = 0;
pub const RATIO: usize = 1;
pub const ATTACK: usize = 2;
pub const RELEASE: usize = 3;
pub const ECHO: usize = 4;
pub const TIME: usize = 5;
pub const FEEDBACK: usize = 6;
pub const TONE: usize = 7;
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
            self.input.as_ptr(),
            self.ratio.as_ptr(),
            self.attack.as_ptr(),
            self.release.as_ptr(),
            self.echo.as_ptr(),
            self.time.as_ptr(),
            self.feedback.as_ptr(),
            self.tone.as_ptr(),
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
    env: f64,
    echo: Vec<Echo>,
    gr_db: f64,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, env: 0.0, echo: per(ch, Echo::new(fs, 1.0)), gr_db: 0.0 }
    }
    fn reset(&mut self) {
        self.env = 0.0;
        self.echo.iter_mut().for_each(|e| e.reset());
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let p = self.v;
        // the 1176: attack 800 us down to 20 us, release 1.1 s down to 50 ms
        let att = 0.8 - 0.78 * p[ATTACK] as f64;
        let rel = 1100.0 - 1050.0 * p[RELEASE] as f64;
        let (ca, cr) = (coeff(fs, att), coeff(fs, rel));
        let ri = (p[RATIO].round().max(0.0) as usize).min(4);
        let ratio = [4.0, 8.0, 12.0, 20.0, 30.0][ri];
        let knee = if ri == 4 { 2.0 } else { 6.0 };
        let thr = -18.0;
        let ing = db_to_gain(p[INPUT] as f64);
        for e in self.echo.iter_mut() {
            e.lp.set_lowpass(fs, p[TONE] as f64);
            e.hp.set_lowpass(fs, 200.0);
        }
        let d = p[TIME] as f64 * 0.001 * fs;
        let fb = p[FEEDBACK] as f64 * 0.9;
        let ew = p[ECHO] as f64;
        let out = db_to_gain(p[OUTPUT] as f64);
        let mut gmax = 0.0f64;
        for k in 0..n {
            let lvl = chans.iter().map(|c| (c[k] as f64 * ing).abs()).fold(0.0, f64::max);
            self.env = lvl + if lvl > self.env { ca } else { cr } * (self.env - lvl);
            let ldb = gain_to_db(self.env.max(1e-6));
            let over = ldb - thr;
            let gr = if over <= -knee { 0.0 } else if over < knee { (1.0 - 1.0 / ratio) * (over + knee).powi(2) / (4.0 * knee) } else { (1.0 - 1.0 / ratio) * over };
            // all buttons: the ratio goes wild and the attack overshoots
            let gr = if ri == 4 { gr * 1.15 } else { gr };
            self.gr_db = gr;
            gmax = gmax.max(gr);
            let g = db_to_gain(-gr);
            for (c, ch) in chans.iter_mut().enumerate() {
                let x = ch[k] as f64 * ing * g;
                // the FET's and the output transformer's bit of colour
                let x = (x * 1.2).tanh() / 1.2;
                let j = c.min(self.echo.len() - 1);
                let e = self.echo[j].process(x, d, fb);
                ch[k] = ((x + e * ew) * out) as f32;
            }
        }
        if !meters.is_empty() {
            meters[0] = gmax as f32;
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Bags Vocal", values: &[(INPUT, 8.0), (RATIO, 0.0), (ATTACK, 0.4), (RELEASE, 0.6), (ECHO, 0.3), (TIME, 260.0), (FEEDBACK, 0.25)] },
        Preset { name: "Alewife (close and dry)", values: &[(INPUT, 4.0), (RATIO, 0.0), (ECHO, 0.12), (TIME, 120.0), (FEEDBACK, 0.0)] },
        Preset { name: "Loaded Echo", values: &[(INPUT, 10.0), (RATIO, 1.0), (ECHO, 0.5), (TIME, 380.0), (FEEDBACK, 0.45), (TONE, 2800.0)] },
        Preset { name: "All Buttons", values: &[(INPUT, 14.0), (RATIO, 4.0), (ATTACK, 0.8), (RELEASE, 0.7), (ECHO, 0.2), (OUTPUT, -6.0)] },
    ]
}

fn panel() -> Panel {
    let k = |p, x, y, r, label, tip| Item::Knob { p, x, y, r, label, tip, active: None };
    Panel {
        title: "IMMUNITY VOCAL",
        subtitle: ("ROSTAM'S VOCAL CHAIN", "1176-STYLE FET COMPRESSOR · LOADED ECHO"),
        accent: ui::OLIVE,
        size: (W, H),
        items: vec![
            k(INPUT, 80.0, 170.0, 28.0, "INPUT", "Drives the FET: more gain reduction."),
            Item::Selector { p: RATIO, x: 190.0, y: 170.0, label: "RATIO", positions: &["4", "8", "12", "20", "ALL"], tip: "The ratio buttons (ALL: every one pushed in)." },
            k(ATTACK, 290.0, 170.0, 22.0, "ATTACK", "Faster clockwise."),
            k(RELEASE, 380.0, 170.0, 22.0, "RELEASE", "Faster clockwise."),
            Item::Meter { m: 0, x: 445.0, y: 120.0, w: 160.0, h: 100.0, kind: MeterKind::GainReduction, caption: "GR" },
            k(ECHO, 90.0, 330.0, 24.0, "ECHO", "How much echo on the voice."),
            k(TIME, 200.0, 330.0, 22.0, "TIME", "Echo time."),
            k(FEEDBACK, 300.0, 330.0, 22.0, "REPEATS", "Echo repeats."),
            k(TONE, 400.0, 330.0, 22.0, "TONE", "How dark the repeats are."),
            k(OUTPUT, 520.0, 330.0, 26.0, "OUTPUT", "Output level."),
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Immunity Vocal", class_id: *b"CLClairoImmuniVx", vendor: "PEQUOD Clairo",
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

    /// At its defaults a steady tone comes through at a sensible level.
    #[test]
    fn passes_signal() {
        let y = go(&defaults(), &sine(1000.0, 0.1, 48000));
        let g = db(rms(&y[24000..]) / rms(&sine(1000.0, 0.1, 48000)[24000..]));
        println!("gain at defaults {g:.1} dB");
        assert!(g > -15.0 && g < 12.0, "{g}");
    }

}
