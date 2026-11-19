//! Tretow Varispeed: Michael B. Tretow's overdubbing trick at Polar Music, for anything - the
//! backing vocals, the pianos, the guitars. The part is recorded again with the tape running a
//! little faster or slower. RETUNED: the singer or player matched the slowed-down tape, so on
//! playback the take is in tune but brighter or darker, never quite the same voice - the ABBA
//! choir. AS PLAYED: nobody retuned, so the take comes back at the speed's pitch (a vocal a
//! semitone up is about 6 %).
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

use vintage::fx::{PitchShift, VarispeedTakes as Takes, TAKE, MAX_TAKES as MAXT};

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Mode { #[name = "Retuned"] Retuned, #[name = "As Played"] AsPlayed }

const PAN: [f64; MAXT] = [-1.0, 1.0, -0.55, 0.55];

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "takes"]
    pub takes: IntParam,
    #[id = "speed"]
    pub speed: FloatParam,
    #[id = "mode"]
    pub mode: EnumParam<Mode>,
    #[id = "loose"]
    pub loose: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "width"]
    pub width: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            takes: IntParam::new("Takes", 2, IntRange::Linear { min: 1, max: (MAXT - 1) as i32 }),
            speed: FloatParam::new("Tape Speed", 3.3, FloatRange::Linear { min: 0.0, max: 12.0 }).with_step_size(0.1).with_unit(" %"),
            mode: EnumParam::new("Mode", Mode::Retuned),
            loose: ms("Looseness", 5.0, 0.5, 30.0),
            mix: pct("Takes", 0.5),
            width: pct("Width", 1.0),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const TAKES: usize = 0;
pub const SPEED: usize = 1;
pub const MODE: usize = 2;
pub const LOOSE: usize = 3;
pub const MIX: usize = 4;
pub const WIDTH: usize = 5;
pub const OUTPUT: usize = 6;
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
            self.takes.as_ptr(),
            self.speed.as_ptr(),
            self.mode.as_ptr(),
            self.loose.as_ptr(),
            self.mix.as_ptr(),
            self.width.as_ptr(),
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
    takes: Takes,
    shift: Vec<PitchShift>,
    n: usize,
    as_played: bool,
    loose: Smooth,
    mix: Smooth,
    width: Smooth,
    fs: f64,
    speed_pct: f64,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        Self {
            takes: Takes::new(fs),
            shift: (0..MAXT).map(|_| PitchShift::new(fs, 45.0)).collect(),
            n: 2,
            as_played: false,
            loose: Smooth::new(fs, 80.0, 5.0),
            mix: Smooth::new(fs, 20.0, 0.5),
            width: Smooth::new(fs, 20.0, 1.0),
            fs,
            speed_pct: 3.3,
            out: Smooth::new(fs, 20.0, 1.0),
        }
    }
    fn reset(&mut self) {
        self.takes.reset();
        self.shift.iter_mut().for_each(|s| s.reset());
    }
    fn set_params(&mut self, v: &[f32]) {
        self.n = (v[TAKES].round() as usize).clamp(1, MAXT - 1);
        self.takes.set_speed((v[SPEED] as f64).min(6.0));       // (the tone shift saturates: beyond 6 % it's the pitch you hear)
        self.speed_pct = v[SPEED] as f64;
        self.as_played = v[MODE] > 0.5;
        self.loose.set(v[LOOSE] as f64);
        self.mix.set(v[MIX] as f64);
        self.width.set(v[WIDTH] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let nt = self.n;
        let cents = Takes::residual_cents(self.takes.speed);
        let norm = (1.0 / nt as f64).sqrt();
        let mut peak = 0.0f64;
        for k in 0..n {
            let dl = chans[0][k] as f64;
            let dr = if nch >= 2 { chans[1][k] as f64 } else { dl };
            let x = 0.5 * (dl + dr);
            self.takes.write(x);
            let loose = self.loose.next() * 0.001 * self.fs;
            let w = self.width.next();
            let (mut l, mut r) = (0.0, 0.0);
            for t in 1..=nt {
                let mut y = self.takes.take(t, loose, cents * TAKE[t].1.abs().max(0.4));
                if self.as_played {
                    // nobody retuned: the take plays back at the speed it was made at
                    let dir = if TAKE[t].1 >= 0.0 { 1.0 } else { -1.0 };
                    let ratio = 1.0 + dir * self.speed_pct * 0.01 * if t > 2 { 0.5 } else { 1.0 };
                    y = self.shift[t].process(y, ratio);
                }
                let pan = PAN[t % MAXT] * w;
                let a = (pan + 1.0) * std::f64::consts::FRAC_PI_4;
                l += y * a.cos() * std::f64::consts::SQRT_2;
                r += y * a.sin() * std::f64::consts::SQRT_2;
            }
            let m = self.mix.next();
            let o = self.out.next();
            let yl = (dl * (1.0 - m) + l * norm * m) * o;
            let yr = (dr * (1.0 - m) + r * norm * m) * o;
            peak = peak.max(yl.abs()).max(yr.abs());
            if nch >= 2 {
                chans[0][k] = yl as f32;
                chans[1][k] = yr as f32;
            } else if nch == 1 {
                chans[0][k] = (0.5 * (yl + yr)) as f32;
            }
        }
        meters[0] = peak as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "ABBA Backing Vocals (retuned)", values: &[(TAKES, 3.0), (SPEED, 3.3), (MODE, 0.0), (LOOSE, 6.0), (MIX, 0.6), (WIDTH, 1.0)] },
        Preset { name: "30 ips against 31 ips", values: &[(TAKES, 1.0), (SPEED, 3.3), (MODE, 0.0), (LOOSE, 5.0), (MIX, 0.5), (WIDTH, 1.0)] },
        Preset { name: "Lead Vocal Double (tight)", values: &[(TAKES, 1.0), (SPEED, 2.0), (MODE, 0.0), (LOOSE, 2.5), (MIX, 0.4), (WIDTH, 0.3)] },
        Preset { name: "Piano Stack", values: &[(TAKES, 2.0), (SPEED, 3.0), (MODE, 0.0), (LOOSE, 8.0), (MIX, 0.5), (WIDTH, 0.8)] },
        Preset { name: "Choir of One (three takes)", values: &[(TAKES, 3.0), (SPEED, 4.5), (MODE, 0.0), (LOOSE, 9.0), (MIX, 0.7), (WIDTH, 1.0)] },
        Preset { name: "Sped-up Take, Not Retuned (+1 semitone)", values: &[(TAKES, 1.0), (SPEED, 6.0), (MODE, 1.0), (LOOSE, 4.0), (MIX, 0.35), (WIDTH, 0.6)] },
        Preset { name: "Octave Up Novelty (as played)", values: &[(TAKES, 1.0), (SPEED, 12.0), (MODE, 1.0), (LOOSE, 3.0), (MIX, 0.3), (WIDTH, 0.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "TRETOW VARISPEED",
        subtitle: ("OVERDUBS AT ANOTHER TAPE SPEED", "THE POLAR MUSIC DOUBLE"),
        accent: ui::WALNUT,
        size: (W, H),
        items: vec![
            Item::Selector { p: TAKES, x: 80.0, y: 160.0, label: "TAKES", positions: &["1", "2", "3"], tip: "How many extra takes, each made at another tape speed, on top of the part." },
            Item::Knob { p: SPEED, x: 200.0, y: 160.0, r: 34.0, label: "TAPE SPEED", tip: "How far the tape speed was changed for the takes (30 against 31 ips is 3.3 %; a semitone is about 6 %).", active: None },
            Item::Switch { p: MODE, x: 320.0, y: 160.0, labels: ["RETUNED", "AS PLAYED"], tip: "Retuned: the singer or player matched the changed tape, so the takes are in tune but a different colour - the ABBA way. As played: nobody retuned, so they come back at the speed's pitch." },
            Item::Meter { m: 0, x: 410.0, y: 104.0, w: 190.0, h: 112.0, kind: MeterKind::Vu, caption: "OUTPUT" },
            Item::Divider { y: 264.0 },
            Item::Knob { p: LOOSE, x: 110.0, y: 330.0, r: 22.0, label: "LOOSENESS", tip: "How far the takes drift ahead and behind the part.", active: None },
            Item::Knob { p: MIX, x: 230.0, y: 330.0, r: 22.0, label: "TAKES LEVEL", tip: "The part against the takes.", active: None },
            Item::Knob { p: WIDTH, x: 350.0, y: 330.0, r: 22.0, label: "WIDTH", tip: "The takes spread left and right (none: in the middle with the part).", active: None },
            Item::Knob { p: OUTPUT, x: 530.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Tretow Varispeed", class_id: *b"ABTretowVarispPQ", vendor: "PEQUOD ABBA",
    categories: [Fx, Modulation]
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
            let l: Vec<f32> = (0..n).map(|k| if (k / 6000) % 2 == 0 { (nz.next() * 0.5) as f32 } else { 0.0 }).collect();
            let mut input = vec![l.clone(), l];
            run(&p, &mut input);
            assert!(input.iter().flatten().all(|v| v.is_finite() && v.abs() < 8.0), "unbounded output for {:?}", p);
        }
    }


    /// AS PLAYED moves the pitch by the speed; RETUNED keeps it (a few cents at most).
    #[test]
    fn as_played_shifts_retuned_does_not() {
        let peak_hz = |mode: f32| {
            let p = with(defaults(), &[(TAKES, 1.0), (SPEED, 6.0), (MODE, mode), (MIX, 1.0), (WIDTH, 0.0), (LOOSE, 1.0)]);
            let s = sine(440.0, 0.3, 96000);
            let mut input = vec![s.clone(), s];
            run(&p, &mut input);
            // zero crossings over the last second
            let x = &input[0][48000..];
            let zc = x.windows(2).filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0)).count();
            zc as f64 / 2.0
        };
        let (re, ap) = (peak_hz(0.0), peak_hz(1.0));
        println!("retuned {re:.0} Hz, as played {ap:.0} Hz");
        assert!((re - 440.0).abs() < 5.0);
        assert!(ap > 455.0 && ap < 480.0);
    }
}
