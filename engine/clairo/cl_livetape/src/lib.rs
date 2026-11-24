//! Live to Tape: Charm was tracked live to analogue tape, the band in one room, at Diamond Mine Recording in
//! Queens and at Allaire, with Leon Michels leading a strictly analogue process (The Line of Best Fit,
//! Mixdown). So: the BLEED of the rest of the band in a small, deadened room, a valve preamp's DRIVE, and
//! a multitrack's tape at 15 or 7.5 ips (a bigger head bump and an earlier top at the slower speed), its
//! saturation and hiss. Estimated.

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
    #[id = "bleed"]
    pub bleed: FloatParam,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "speed"]
    pub speed: FloatParam,
    #[id = "level"]
    pub level: FloatParam,
    #[id = "hiss"]
    pub hiss: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            bleed: pct("Room Bleed", 0.3),
            drive: pct("Preamp Drive", 0.35),
            speed: FloatParam::new("Speed", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0),
            level: db("Record Level", 0.0, -12.0, 12.0),
            hiss: pct("Hiss", 0.12),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const BLEED: usize = 0;
pub const DRIVE: usize = 1;
pub const SPEED: usize = 2;
pub const LEVEL: usize = 3;
pub const HISS: usize = 4;
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
            self.bleed.as_ptr(),
            self.drive.as_ptr(),
            self.speed.as_ptr(),
            self.level.as_ptr(),
            self.hiss.as_ptr(),
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

/// Early reflections of a small, treated room (Diamond Mine's live room, its size estimated).
const TAPS: [(f64, f64); 6] = [(0.0041, 0.55), (0.0067, 0.42), (0.0093, 0.33), (0.0131, 0.24), (0.0178, 0.17), (0.0236, 0.11)];

#[derive(Clone)]
struct Ch {
    line: DelayLine,
    room_lp: OnePole,
    pre: Biquad,
    bump: Biquad,
    lp: [Biquad; 2],
    dc: DcBlock,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ch: Vec<Ch>,
    noise: Noise,
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, n: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let c = Ch { line: DelayLine::new((fs * 0.03) as usize), room_lp: OnePole::default(), pre: Biquad::identity(), bump: Biquad::identity(), lp: [Biquad::identity(); 2], dc: DcBlock::new(fs) };
        Self { fs, v, ch: per(n, c), noise: Noise::new(2024) }
    }
    fn reset(&mut self) {
        for c in self.ch.iter_mut() {
            c.line.reset();
            c.room_lp.reset();
            c.pre.reset();
            c.bump.reset();
            c.lp.iter_mut().for_each(|b| b.reset());
            c.dc.reset();
        }
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let p = self.v;
        let slow = p[SPEED] >= 0.5;
        for c in self.ch.iter_mut() {
            c.room_lp.set_lowpass(fs, 3500.0);
            c.pre.set_peak(fs, 180.0, 0.7, 1.5 * p[DRIVE] as f64);
            if slow {
                c.bump.set_peak(fs, 50.0, 1.0, 3.0);
                c.lp[0].set_lowpass(fs, 11000.0, 0.7);
            } else {
                c.bump.set_peak(fs, 80.0, 1.0, 2.0);
                c.lp[0].set_lowpass(fs, 17000.0, 0.7);
            }
            c.lp[1].set_high_shelf(fs, 6000.0, 0.7, if slow { -2.5 } else { -1.0 });
        }
        let bleed = p[BLEED] as f64;
        let drive = 1.0 + 6.0 * p[DRIVE] as f64;
        let rec = db_to_gain(p[LEVEL] as f64);
        let hiss = p[HISS] as f64 * if slow { 0.012 } else { 0.008 };
        let out = db_to_gain(p[OUTPUT] as f64);
        for k in 0..n {
            let hs = self.noise.next() * hiss;
            for (ci, ch) in chans.iter_mut().enumerate() {
                let last = self.ch.len() - 1;
                let c = &mut self.ch[ci.min(last)];
                let x = ch[k] as f64;
                c.line.write(x);
                let mut er = 0.0;
                for (t, g) in TAPS {
                    er += c.line.read(t * fs) * g;
                }
                let x = x + c.room_lp.lp(er) * bleed;
                // the valve preamp: second harmonic first
                let u = c.pre.process(x) * drive;
                let y = c.dc.process((u + 0.15 * u * u).tanh()) / drive.sqrt();
                // the tape
                let t = (y * rec * 1.3).tanh() / 1.3 / rec.max(0.25);
                let t = c.bump.process(t);
                let t = c.lp[0].process(t);
                let t = c.lp[1].process(t) + hs;
                ch[k] = (t * out) as f32;
            }
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Sexy to Someone (band)", values: &[(BLEED, 0.35), (DRIVE, 0.4), (SPEED, 0.0), (LEVEL, 3.0), (HISS, 0.1)] },
        Preset { name: "Juna Keys", values: &[(BLEED, 0.25), (DRIVE, 0.3), (SPEED, 1.0), (LEVEL, 2.0), (HISS, 0.12)] },
        Preset { name: "Hot to Tape", values: &[(BLEED, 0.2), (DRIVE, 0.6), (SPEED, 1.0), (LEVEL, 9.0), (HISS, 0.15)] },
        Preset { name: "Clean Capture", values: &[(BLEED, 0.1), (DRIVE, 0.1), (SPEED, 0.0), (LEVEL, -3.0), (HISS, 0.05)] },
    ]
}

fn panel() -> Panel {
    let k = |p, x, y, r, label, tip| Item::Knob { p, x, y, r, label, tip, active: None };
    Panel {
        title: "LIVE TO TAPE",
        subtitle: ("CHARM · DIAMOND MINE, QUEENS", "ROOM BLEED · VALVE PREAMP · MULTITRACK TAPE"),
        accent: ui::WALNUT,
        size: (W, H),
        items: vec![
            k(BLEED, 100.0, 200.0, 28.0, "BLEED", "The rest of the band, heard in the room."),
            k(DRIVE, 220.0, 200.0, 28.0, "DRIVE", "The valve preamp."),
            Item::Switch { p: SPEED, x: 330.0, y: 200.0, labels: ["15 IPS", "7.5 IPS"], tip: "Tape speed: 7.5 has more bump and less top." },
            k(LEVEL, 440.0, 200.0, 28.0, "REC LEVEL", "How hot it goes to tape."),
            k(HISS, 260.0, 340.0, 22.0, "HISS", "Tape hiss."),
            k(OUTPUT, 400.0, 340.0, 24.0, "OUTPUT", "Output level."),
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Live to Tape", class_id: *b"CLClairoLiveTape", vendor: "PEQUOD Clairo",
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
