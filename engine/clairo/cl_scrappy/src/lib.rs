//! Scrappy Guitar: the guitar chain Rostam Batmanglij used on Immunity for what he called "a scrappy
//! sound" - Avid's SansAmp (after Tech 21's PSA-1 preamp: BUZZ, PUNCH, CRUNCH and DRIVE before, LOW and
//! HIGH after), then an ATR-102 tape recorder, then a small convolution room (Mixdown). All played by
//! Claire. Curves estimated.

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
    #[id = "buzz"]
    pub buzz: FloatParam,
    #[id = "punch"]
    pub punch: FloatParam,
    #[id = "crunch"]
    pub crunch: FloatParam,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "low"]
    pub low: FloatParam,
    #[id = "high"]
    pub high: FloatParam,
    #[id = "tape"]
    pub tape: FloatParam,
    #[id = "room"]
    pub room: FloatParam,
    #[id = "level"]
    pub level: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            buzz: pct("Buzz", 0.4),
            punch: pct("Punch", 0.5),
            crunch: pct("Crunch", 0.4),
            drive: pct("Drive", 0.4),
            low: db("Low", 0.0, -12.0, 12.0),
            high: db("High", -2.0, -12.0, 12.0),
            tape: pct("Tape", 0.5),
            room: pct("Room", 0.2),
            level: db("Level", -6.0, -24.0, 12.0),
        }
    }
}

pub const BUZZ: usize = 0;
pub const PUNCH: usize = 1;
pub const CRUNCH: usize = 2;
pub const DRIVE: usize = 3;
pub const LOW: usize = 4;
pub const HIGH: usize = 5;
pub const TAPE: usize = 6;
pub const ROOM: usize = 7;
pub const LEVEL: usize = 8;
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
            self.buzz.as_ptr(),
            self.punch.as_ptr(),
            self.crunch.as_ptr(),
            self.drive.as_ptr(),
            self.low.as_ptr(),
            self.high.as_ptr(),
            self.tape.as_ptr(),
            self.room.as_ptr(),
            self.level.as_ptr(),
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

#[derive(Clone)]
struct Ch {
    pre: [Biquad; 3],
    post: [Biquad; 2],
    bump: Biquad,
    tape_lp: OnePole,
    dc: DcBlock,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ch: Vec<Ch>,
    verb: Fdn,
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, n: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut verb = Fdn::new(fs, 0.8);
        verb.configure(0.45, [0.5, 0.45, 0.3], 5000.0);
        let c = Ch { pre: [Biquad::identity(); 3], post: [Biquad::identity(); 2], bump: Biquad::identity(), tape_lp: OnePole::default(), dc: DcBlock::new(fs) };
        Self { fs, v, ch: per(n, c), verb }
    }
    fn reset(&mut self) {
        for c in self.ch.iter_mut() {
            c.pre.iter_mut().chain(c.post.iter_mut()).for_each(|b| b.reset());
            c.bump.reset();
            c.tape_lp.reset();
            c.dc.reset();
        }
        self.verb.reset();
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let p = self.v;
        for c in self.ch.iter_mut() {
            // before the drive: BUZZ the lows, PUNCH the mids, CRUNCH the top's grit
            c.pre[0].set_low_shelf(fs, 180.0, 0.8, -6.0 + 12.0 * p[BUZZ] as f64);
            c.pre[1].set_peak(fs, 900.0, 0.8, -6.0 + 12.0 * p[PUNCH] as f64);
            c.pre[2].set_high_shelf(fs, 2500.0, 0.8, -6.0 + 12.0 * p[CRUNCH] as f64);
            // after it: the speaker-emulation roll-off, LOW and HIGH
            c.post[0].set_low_shelf(fs, 120.0, 0.8, p[LOW] as f64);
            c.post[1].set_high_shelf(fs, 3300.0, 0.7, p[HIGH] as f64 - 8.0);
            // the ATR-102: a head bump near 50 Hz, the top just softened
            c.bump.set_peak(fs, 50.0, 1.0, 2.0 * p[TAPE] as f64);
            c.tape_lp.set_lowpass(fs, 22000.0 - 6000.0 * p[TAPE] as f64);
        }
        let drive = 1.0 + 40.0 * (p[DRIVE] as f64).powi(2);
        let tape = p[TAPE] as f64;
        let room = p[ROOM] as f64;
        let out = db_to_gain(p[LEVEL] as f64);
        let nch = chans.len();
        for k in 0..n {
            let mut sum = 0.0;
            for (ci, ch) in chans.iter_mut().enumerate() {
                let last = self.ch.len() - 1;
                let c = &mut self.ch[ci.min(last)];
                let mut x = ch[k] as f64;
                for b in c.pre.iter_mut() {
                    x = b.process(x);
                }
                // the tube-like stage: asymmetric, soft
                let u = x * drive;
                let y = (u + 0.2 * u * u.abs().min(1.0)).tanh() / drive.sqrt();
                let mut y = c.dc.process(y);
                for b in c.post.iter_mut() {
                    y = b.process(y);
                }
                let t = c.bump.process(y);
                let t = c.tape_lp.lp((t * (1.0 + tape)).tanh() / (1.0 + tape * 0.6));
                ch[k] = t as f32;
                sum += t;
            }
            let dif = self.verb.diffuse(sum / nch.max(1) as f64, 2);
            let (l, r) = self.verb.process(dif);
            for (ci, ch) in chans.iter_mut().enumerate() {
                let w = if ci % 2 == 0 { l } else { r };
                ch[k] = ((ch[k] as f64 * (1.0 - 0.3 * room) + w * room) * out) as f32;
            }
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Alewife Guitar", values: &[(DRIVE, 0.25), (CRUNCH, 0.3), (TAPE, 0.6), (ROOM, 0.25)] },
        Preset { name: "Scrappy Lead", values: &[(DRIVE, 0.65), (PUNCH, 0.7), (CRUNCH, 0.6), (HIGH, 1.0), (TAPE, 0.5), (ROOM, 0.15)] },
        Preset { name: "Clean-ish Strum", values: &[(DRIVE, 0.1), (BUZZ, 0.3), (TAPE, 0.4), (ROOM, 0.3), (LEVEL, 0.0)] },
        Preset { name: "Fuzzy Bed", values: &[(DRIVE, 0.9), (BUZZ, 0.6), (HIGH, -6.0), (TAPE, 0.8), (ROOM, 0.35)] },
    ]
}

fn panel() -> Panel {
    let k = |p, x, y, r, label, tip| Item::Knob { p, x, y, r, label, tip, active: None };
    Panel {
        title: "SCRAPPY GUITAR",
        subtitle: ("IMMUNITY · ROSTAM'S GUITAR CHAIN", "SANSAMP · ATR-102 TAPE · SMALL ROOM"),
        accent: ui::TANGERINE,
        size: (W, H),
        items: vec![
            k(BUZZ, 80.0, 170.0, 22.0, "BUZZ", "The lows before the drive."),
            k(PUNCH, 170.0, 170.0, 22.0, "PUNCH", "The mids before the drive."),
            k(CRUNCH, 260.0, 170.0, 22.0, "CRUNCH", "The top's grit before the drive."),
            k(DRIVE, 360.0, 170.0, 28.0, "DRIVE", "How hard the preamp is driven."),
            k(LOW, 460.0, 170.0, 22.0, "LOW", "Bass after the drive."),
            k(HIGH, 550.0, 170.0, 22.0, "HIGH", "Treble after the drive."),
            k(TAPE, 180.0, 320.0, 26.0, "TAPE", "How hard the ATR-102 is hit."),
            k(ROOM, 320.0, 320.0, 26.0, "ROOM", "The small room."),
            k(LEVEL, 460.0, 320.0, 26.0, "LEVEL", "Output level."),
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Scrappy Guitar", class_id: *b"CLClairoScrappyG", vendor: "PEQUOD Clairo",
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
