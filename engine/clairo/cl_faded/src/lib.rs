//! Faded Tape: Sling was run through tape for its faded, wobbly quality (The Quietus) - Jack Antonoff and
//! Laura Sisk at Allaire, Mellotron, Wurlitzer and flutes going round a machine that's seen some years.
//! WOW and FLUTTER from a tired transport, FADE the oxide's loss of top, SATURATION, HISS, and the odd
//! DROPOUT where the coating has worn. Estimated.

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
    #[id = "wow"]
    pub wow: FloatParam,
    #[id = "flutter"]
    pub flutter: FloatParam,
    #[id = "fade"]
    pub fade: FloatParam,
    #[id = "sat"]
    pub sat: FloatParam,
    #[id = "hiss"]
    pub hiss: FloatParam,
    #[id = "dropout"]
    pub dropout: FloatParam,
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
            wow: pct("Wow", 0.35),
            flutter: pct("Flutter", 0.25),
            fade: pct("Fade", 0.4),
            sat: pct("Saturation", 0.3),
            hiss: pct("Hiss", 0.15),
            dropout: pct("Dropouts", 0.1),
            mix: pct("Mix", 1.0),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const WOW: usize = 0;
pub const FLUTTER: usize = 1;
pub const FADE: usize = 2;
pub const SAT: usize = 3;
pub const HISS: usize = 4;
pub const DROPOUT: usize = 5;
pub const MIX: usize = 6;
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
            self.wow.as_ptr(),
            self.flutter.as_ptr(),
            self.fade.as_ptr(),
            self.sat.as_ptr(),
            self.hiss.as_ptr(),
            self.dropout.as_ptr(),
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
    line: DelayLine,
    lp: [Biquad; 2],
    hp: Biquad,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ch: Vec<Ch>,
    wow: Wander,
    flut: f64,
    noise: Noise,
    drop: f64,
    drop_t: f64,
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, n: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let c = Ch { line: DelayLine::new((fs * 0.05) as usize), lp: [Biquad::identity(); 2], hp: Biquad::identity() };
        Self { fs, v, ch: per(n, c), wow: Wander::new(fs, 0.55, 2021), flut: 0.0, noise: Noise::new(1976), drop: 1.0, drop_t: 1.0 }
    }
    fn reset(&mut self) {
        for c in self.ch.iter_mut() {
            c.line.reset();
            c.lp.iter_mut().for_each(|b| b.reset());
            c.hp.reset();
        }
        self.drop = 1.0;
        self.drop_t = 1.0;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let p = self.v;
        let fade = p[FADE] as f64;
        for c in self.ch.iter_mut() {
            c.lp[0].set_lowpass(fs, 20000.0 * 2f64.powf(-3.2 * fade), 0.6);
            c.lp[1].set_high_shelf(fs, 5000.0, 0.7, -6.0 * fade);
            c.hp.set_peak(fs, 65.0, 0.9, 2.5);
        }
        let wow_ms = 3.5 * p[WOW] as f64;
        let flut_ms = 0.35 * p[FLUTTER] as f64;
        let sat = 1.0 + 3.0 * p[SAT] as f64;
        let hiss = p[HISS] as f64 * 0.01;
        let dp = p[DROPOUT] as f64;
        let mix = p[MIX] as f64;
        let out = db_to_gain(p[OUTPUT] as f64);
        let base = 0.006 * fs;
        let cd = coeff(fs, 8.0);
        for k in 0..n {
            let w = self.wow.next();
            self.flut += 11.3 / fs;
            if self.flut >= 1.0 {
                self.flut -= 1.0;
            }
            let fl = (std::f64::consts::TAU * self.flut).sin() + 0.4 * (std::f64::consts::TAU * self.flut * 2.7).sin();
            let d = base + (wow_ms * w + flut_ms * fl) * 0.001 * fs;
            // dropouts: now and then the level dips for a moment
            if self.noise.next().abs() > 1.0 - dp * 0.0004 {
                self.drop_t = 0.25 + 0.5 * self.noise.next().abs();
            } else if self.drop_t < 1.0 {
                self.drop_t = (self.drop_t + 0.00008).min(1.0);
            }
            self.drop = self.drop_t + cd * (self.drop - self.drop_t);
            let hs = self.noise.next() * hiss;
            for (ci, ch) in chans.iter_mut().enumerate() {
                let last = self.ch.len() - 1;
                let c = &mut self.ch[ci.min(last)];
                let x = ch[k] as f64;
                c.line.write(x);
                let mut y = c.line.read(d.max(1.0));
                y = (y * sat).tanh() / sat.tanh().max(0.3) * 0.9;
                y = c.hp.process(y);
                let y0 = c.lp[0].process(y);
                y = c.lp[1].process(y0) * self.drop + hs;
                ch[k] = ((x * (1.0 - mix) + y * mix) * out) as f32;
            }
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Sling Bus", values: &[(WOW, 0.3), (FLUTTER, 0.2), (FADE, 0.35), (SAT, 0.3), (HISS, 0.1), (DROPOUT, 0.05)] },
        Preset { name: "Wade Mellotron", values: &[(WOW, 0.55), (FLUTTER, 0.35), (FADE, 0.5), (SAT, 0.2), (HISS, 0.15)] },
        Preset { name: "Old Cassette", values: &[(WOW, 0.7), (FLUTTER, 0.6), (FADE, 0.75), (SAT, 0.45), (HISS, 0.4), (DROPOUT, 0.35)] },
        Preset { name: "Just a Touch", values: &[(WOW, 0.12), (FLUTTER, 0.1), (FADE, 0.15), (SAT, 0.15), (HISS, 0.0), (DROPOUT, 0.0)] },
    ]
}

fn panel() -> Panel {
    let k = |p, x, y, r, label, tip| Item::Knob { p, x, y, r, label, tip, active: None };
    Panel {
        title: "FADED TAPE",
        subtitle: ("SLING · ALLAIRE, MOUNT TONCHE", "WOW · FLUTTER · WORN OXIDE"),
        accent: ui::WALNUT,
        size: (W, H),
        items: vec![
            k(WOW, 90.0, 170.0, 28.0, "WOW", "Slow pitch wander."),
            k(FLUTTER, 210.0, 170.0, 24.0, "FLUTTER", "Fast pitch flutter."),
            k(FADE, 330.0, 170.0, 28.0, "FADE", "Worn oxide: the top fades away."),
            k(SAT, 450.0, 170.0, 24.0, "SATURATION", "How hard the tape is hit."),
            k(OUTPUT, 560.0, 170.0, 24.0, "OUTPUT", "Output level."),
            k(HISS, 150.0, 320.0, 22.0, "HISS", "Tape hiss."),
            k(DROPOUT, 290.0, 320.0, 22.0, "DROPOUTS", "Now and then the level dips where the coating's worn."),
            k(MIX, 430.0, 320.0, 22.0, "MIX", "Dry to tape."),
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Faded Tape", class_id: *b"CLClairoFadedTpe", vendor: "PEQUOD Clairo",
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
