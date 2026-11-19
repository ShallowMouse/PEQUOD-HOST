//! Polar Acoustic: the acoustic guitars on ABBA's records, as Michael B. Tretow made them at
//! Polar Music. The strummed part is played again and again, each overdub with the tape
//! machine running a little faster or slower and the guitar retuned to match, so on playback
//! every take is in tune but not quite the same: its body and pick sound shifted by the
//! speed difference, its tuning never perfectly matched, its timing its own. Stacked and
//! spread, the takes shimmer like a chorus no effect box makes. Then the valve limiting
//! amplifiers (UA LN 176s on the instrument tracks) flatten the strum into a rhythm part,
//! and the top end is lifted until it sparkles above the piano.
//!
//! The varispeed takes are modelled, not re-recorded: each is the part delayed by its own
//! lean and drift, left a few cents off pitch (what retuning by ear leaves), and brightened
//! by a shelf that grows with the speed difference (the body resonances moving up).

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
pub enum Image { #[name = "Mono"] Mono, #[name = "Spread"] Spread }

use vintage::fx::{VarispeedTakes as Takes, TAKE, MAX_TAKES as MAXT};
// spread: ABBA's doubled acoustics sit hard left and right; the third and fourth takes inside them
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
    #[id = "loose"]
    pub loose: FloatParam,
    #[id = "limit"]
    pub limit: FloatParam,
    #[id = "sparkle"]
    pub sparkle: FloatParam,
    #[id = "lowcut"]
    pub lowcut: FloatParam,
    #[id = "image"]
    pub image: EnumParam<Image>,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            takes: IntParam::new("Takes", 3, IntRange::Linear { min: 1, max: MAXT as i32 }),
            speed: FloatParam::new("Tape Speed", 3.3, FloatRange::Linear { min: 0.0, max: 6.0 }).with_step_size(0.1).with_unit(" %"),
            loose: ms("Looseness", 7.0, 0.5, 30.0),
            limit: pct("Limiting", 0.55),
            sparkle: db("Sparkle", 4.0, 0.0, 9.0),
            lowcut: hz("Low Cut", 110.0, 30.0, 300.0),
            image: EnumParam::new("Image", Image::Spread),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const TAKES: usize = 0;
pub const SPEED: usize = 1;
pub const LOOSE: usize = 2;
pub const LIMIT: usize = 3;
pub const SPARKLE: usize = 4;
pub const LOWCUT: usize = 5;
pub const IMAGE: usize = 6;
pub const OUTPUT: usize = 7;
pub const NPARAMS: usize = 8;

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
            self.loose.as_ptr(),
            self.limit.as_ptr(),
            self.sparkle.as_ptr(),
            self.lowcut.as_ptr(),
            self.image.as_ptr(),
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
    takes: Takes,
    n: usize,
    loose: Smooth,
    spread: bool,
    limit: f64,
    env: Envelope,
    gr: f64,
    hp1: Biquad,
    hp2: Biquad,
    hpl: Biquad,
    hpr: Biquad,
    lowcut: f64,
    sp_l: Biquad,
    sp_r: Biquad,
    pr_l: Biquad,
    pr_r: Biquad,
    sparkle: f64,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 2;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut d = Self {
            fs,
            takes: Takes::new(fs),
            n: 3,
            loose: Smooth::new(fs, 80.0, 7.0),
            spread: true,
            limit: 0.55,
            env: Envelope::new(fs, 0.3, 60.0),
            gr: 1.0,
            hp1: Biquad::identity(),
            hp2: Biquad::identity(),
            hpl: Biquad::identity(),
            hpr: Biquad::identity(),
            lowcut: -1.0,
            sp_l: Biquad::identity(),
            sp_r: Biquad::identity(),
            pr_l: Biquad::identity(),
            pr_r: Biquad::identity(),
            sparkle: -1.0,
            out: Smooth::new(fs, 20.0, 1.0),
        };
        d.set_eq(110.0, 4.0);
        d
    }
    fn reset(&mut self) {
        self.takes.reset();
        for b in [&mut self.hp1, &mut self.hp2, &mut self.hpl, &mut self.hpr, &mut self.sp_l, &mut self.sp_r, &mut self.pr_l, &mut self.pr_r] {
            b.reset();
        }
        self.env.value = 0.0;
        self.gr = 1.0;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.n = (v[TAKES].round() as usize).clamp(1, MAXT);
        self.takes.set_speed(v[SPEED] as f64);
        self.loose.set(v[LOOSE] as f64);
        self.limit = v[LIMIT] as f64;
        self.spread = v[IMAGE] > 0.5;
        self.set_eq(v[LOWCUT] as f64, v[SPARKLE] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let nt = self.n;
        let cents = Takes::residual_cents(self.takes.speed);
        // equal power, then back to the level of one take on each side
        let norm = if self.spread && nt > 1 { (2.0 / nt as f64).sqrt() } else { 1.0 / (nt as f64).sqrt() };
        // the LN 176s: threshold falls and makeup rises with LIMITING
        let thr = -8.0 - self.limit * 22.0;
        let ratio = 4.0 + self.limit * 16.0;
        let makeup = db_to_gain(self.limit * 12.0);
        let mut peak = 0.0f64;
        let mut grmax = 0.0f64;
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            // the guitar's mic: low end trimmed before it goes to tape
            x = self.hp2.process(self.hp1.process(x));
            self.takes.write(x);
            let loose = self.loose.next() * 0.001 * self.fs;
            let (mut l, mut r) = (0.0, 0.0);
            for i in 0..nt {
                let y = self.takes.take(i, loose, cents * TAKE[i].1.abs().max(0.4));
                if self.spread && nt > 1 {
                    let a = (PAN[i] + 1.0) * std::f64::consts::FRAC_PI_4;
                    l += y * a.cos();
                    r += y * a.sin();
                } else {
                    l += y;
                    r += y;
                }
            }
            l *= norm;
            r *= norm;
            // valve limiting: fast attack, a quick release, the stage's own rounding
            let e = self.env.process(l.abs().max(r.abs()));
            let over = (gain_to_db(e) - thr).max(0.0);
            let target = db_to_gain(-over * (1.0 - 1.0 / ratio));
            let a = if target < self.gr { 0.5 } else { 0.0015 };
            self.gr += (target - self.gr) * a;
            grmax = grmax.max(-gain_to_db(self.gr));
            let g = self.gr * makeup;
            l = tube(l * g * 0.8, 0.08) / 0.8;
            r = tube(r * g * 0.8, 0.08) / 0.8;
            // sparkle: the top lifted, a little presence; the low end tidied once more
            l = self.pr_l.process(self.sp_l.process(self.hpl.process(l)));
            r = self.pr_r.process(self.sp_r.process(self.hpr.process(r)));
            let o = self.out.next();
            let (yl, yr) = (l * o, r * o);
            peak = peak.max(yl.abs()).max(yr.abs());
            if nch >= 2 {
                chans[0][k] = yl as f32;
                chans[1][k] = yr as f32;
                for c in chans.iter_mut().skip(2) {
                    c[k] = 0.0;
                }
            } else if nch == 1 {
                chans[0][k] = (0.5 * (yl + yr)) as f32;
            }
        }
        meters[0] = peak as f32;
        meters[1] = grmax as f32;
    }
}

impl Dsp {
    fn set_eq(&mut self, lowcut: f64, sparkle: f64) {
        if (lowcut - self.lowcut).abs() > 1e-3 {
            self.lowcut = lowcut;
            self.hp1.set_highpass(self.fs, lowcut * 0.7, 0.5);
            self.hp2.set_highpass(self.fs, lowcut * 0.7, 0.5);
            self.hpl.set_highpass(self.fs, lowcut, 0.6);
            self.hpr.set_highpass(self.fs, lowcut, 0.6);
        }
        if (sparkle - self.sparkle).abs() > 1e-3 {
            self.sparkle = sparkle;
            for b in [&mut self.sp_l, &mut self.sp_r] {
                b.set_high_shelf(self.fs, 7000.0, 0.8, sparkle);
            }
            for b in [&mut self.pr_l, &mut self.pr_r] {
                b.set_peak(self.fs, 3200.0, 0.9, sparkle * 0.3);
            }
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Strummed Wall (Dancing Queen era)", values: &[(TAKES, 3.0), (SPEED, 3.3), (LOOSE, 7.0), (LIMIT, 0.6), (SPARKLE, 4.5), (IMAGE, 1.0)] },
        Preset { name: "Two Takes, Hard Left and Right", values: &[(TAKES, 2.0), (SPEED, 3.3), (LOOSE, 5.0), (LIMIT, 0.5), (SPARKLE, 4.0), (IMAGE, 1.0)] },
        Preset { name: "Four-Take Shimmer", values: &[(TAKES, 4.0), (SPEED, 4.5), (LOOSE, 9.0), (LIMIT, 0.55), (SPARKLE, 5.0), (IMAGE, 1.0)] },
        Preset { name: "Percussive Rhythm Guitar", values: &[(TAKES, 2.0), (SPEED, 2.5), (LOOSE, 4.0), (LIMIT, 0.9), (SPARKLE, 6.0), (LOWCUT, 180.0), (IMAGE, 1.0)] },
        Preset { name: "12-String Sparkle", values: &[(TAKES, 3.0), (SPEED, 3.3), (LOOSE, 6.0), (LIMIT, 0.45), (SPARKLE, 7.5), (LOWCUT, 150.0), (IMAGE, 1.0)] },
        Preset { name: "Doubled in Mono (early Polar)", values: &[(TAKES, 2.0), (SPEED, 3.0), (LOOSE, 6.0), (LIMIT, 0.5), (SPARKLE, 3.0), (IMAGE, 0.0)] },
        Preset { name: "Gentle Ballad Strum", values: &[(TAKES, 2.0), (SPEED, 2.0), (LOOSE, 8.0), (LIMIT, 0.3), (SPARKLE, 3.0), (LOWCUT, 90.0), (IMAGE, 1.0)] },
        Preset { name: "One Take, Limited and Bright", values: &[(TAKES, 1.0), (LIMIT, 0.7), (SPARKLE, 5.0), (IMAGE, 0.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "POLAR ACOUSTIC",
        subtitle: ("THE ABBA ACOUSTIC GUITARS", "VARISPEED TAKES · VALVE LIMITING · SPARKLE"),
        accent: ui::STEEL,
        size: (W, H),
        items: vec![
            Item::Selector { p: TAKES, x: 80.0, y: 160.0, label: "TAKES", positions: &["1", "2", "3", "4"], tip: "How many times the part was played. Each extra take was recorded with the tape running at another speed and the guitar retuned to match." },
            Item::Knob { p: SPEED, x: 200.0, y: 160.0, r: 34.0, label: "TAPE SPEED", tip: "How far apart the takes' tape speeds were (30 against 31 ips is 3.3 %). More: each take's timbre further from the others, and a little more out of tune.", active: None },
            Item::Knob { p: LOOSE, x: 320.0, y: 160.0, r: 34.0, label: "LOOSENESS", tip: "How far the takes drift ahead and behind each other.", active: None },
            Item::Meter { m: 1, x: 410.0, y: 104.0, w: 190.0, h: 112.0, kind: MeterKind::GainReduction, caption: "LN 176 LIMITING" },
            Item::Divider { y: 264.0 },
            Item::Knob { p: LIMIT, x: 80.0, y: 330.0, r: 22.0, label: "LIMITING", tip: "The valve limiting amplifiers on the guitar tracks: more turns the strum into an even, percussive rhythm part.", active: None },
            Item::Knob { p: SPARKLE, x: 190.0, y: 330.0, r: 22.0, label: "SPARKLE", tip: "The top end lifted (and a touch of presence), so the strings ring above the piano.", active: None },
            Item::Knob { p: LOWCUT, x: 300.0, y: 330.0, r: 22.0, label: "LOW CUT", tip: "The body boom trimmed, leaving the bass and piano room underneath.", active: None },
            Item::Switch { p: IMAGE, x: 420.0, y: 330.0, labels: ["MONO", "SPREAD"], tip: "Spread: the takes hard left and right (the third and fourth inside them), as on the records. Mono: all in the middle." },
            Item::Knob { p: OUTPUT, x: 550.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Polar Acoustic", class_id: *b"ABPolarAcoustPQD", vendor: "PEQUOD ABBA",
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

    /// Spread takes: the two sides differ (each its own take), and the takes beat against each other.
    #[test]
    fn takes_spread_and_shimmer() {
        let p = with(defaults(), &[(TAKES, 2.0), (LIMIT, 0.0), (SPARKLE, 0.0), (SPEED, 4.0)]);
        let s = sine(330.0, 0.3, 96000);
        let mut input = vec![s.clone(), s];
        run(&p, &mut input);
        let (l, r) = (&input[0][24000..], &input[1][24000..]);
        let diff: Vec<f32> = l.iter().zip(r).map(|(a, b)| a - b).collect();
        let ratio = rms(&diff) / rms(l).max(1e-9);
        println!("two takes: L-R {:.2} of L", ratio);
        assert!(ratio > 0.1, "the two sides should be different takes");
        // one take in mono: both sides the same
        let p1 = with(defaults(), &[(TAKES, 1.0), (IMAGE, 0.0)]);
        let s = sine(330.0, 0.3, 48000);
        let mut input = vec![s.clone(), s];
        run(&p1, &mut input);
        assert!(input[0].iter().zip(&input[1]).all(|(a, b)| (a - b).abs() < 1e-6));
    }

    /// LIMITING evens out a strum: a loud and a quiet burst come out much closer together.
    #[test]
    fn limiting_evens_the_strum() {
        let level = |lim: f32, amp: f64| {
            let p = with(defaults(), &[(TAKES, 1.0), (IMAGE, 0.0), (LIMIT, lim), (SPARKLE, 0.0)]);
            let s = sine(500.0, amp, 48000);
            let mut input = vec![s.clone(), s];
            run(&p, &mut input);
            20.0 * rms(&input[0][24000..]).log10()
        };
        let spread_off = level(0.0, 0.5) - level(0.0, 0.05);
        let spread_on = level(0.9, 0.5) - level(0.9, 0.05);
        println!("20 dB in: {spread_off:.1} dB out without limiting, {spread_on:.1} dB with");
        assert!(spread_on < spread_off - 8.0);
    }
}
