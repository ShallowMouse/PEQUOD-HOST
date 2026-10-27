//! Wah: a Cry Baby-style wah. Brian May uses one half-way down as a tone control (cocked), and to shape the Deacy amp's voices. PEDAL can be automated; AUTO opens it with your playing.

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
pub enum Mode { #[name = "Pedal"] Pedal, #[name = "Auto"] Auto }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Range { #[name = "Cry Baby"] Cry, #[name = "Vintage (narrow)"] Vintage, #[name = "Wide"] Wide }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "pedal"]
    pub pedal: FloatParam,
    #[id = "mode"]
    pub mode: EnumParam<Mode>,
    #[id = "range"]
    pub range: EnumParam<Range>,
    #[id = "q"]
    pub q: FloatParam,
    #[id = "sens"]
    pub sens: FloatParam,
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
            pedal: pct("Pedal", 0.5),
            mode: EnumParam::new("Mode", Mode::Pedal),
            range: EnumParam::new("Range", Range::Cry),
            q: pct("Resonance", 0.55),
            sens: pct("Auto Sensitivity", 0.5),
            mix: pct("Mix", 1.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const PEDAL: usize = 0;
pub const MODE: usize = 1;
pub const RANGE: usize = 2;
pub const Q: usize = 3;
pub const SENS: usize = 4;
pub const MIX: usize = 5;
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
            self.pedal.as_ptr(),
            self.mode.as_ptr(),
            self.range.as_ptr(),
            self.q.as_ptr(),
            self.sens.as_ptr(),
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
    v: [f32; NPARAMS],
    bp: Vec<Biquad>,
    lp: Vec<Biquad>,
    pos: Smooth,
    env: f64,
    k: usize,
    mix: Smooth,
    out: Smooth,
    pub f_now: f64,
}

pub fn range(r: usize) -> (f64, f64) {
    [(380.0, 2200.0), (450.0, 1600.0), (300.0, 3200.0)][r.min(2)]
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut lp = Biquad::identity();
        lp.set_lowpass(fs, 5000.0, 0.7);
        Self { fs, v, bp: vec![Biquad::identity(); ch.max(2)], lp: vec![lp; ch.max(2)], pos: Smooth::new(fs, 15.0, 0.5), env: 0.0, k: 0,
               mix: Smooth::new(fs, 20.0, 1.0), out: Smooth::new(fs, 20.0, 1.0), f_now: 1000.0 }
    }
    fn reset(&mut self) {
        self.bp.iter_mut().chain(self.lp.iter_mut()).for_each(|b| b.reset());
        self.env = 0.0;
        self.k = 0;
        self.pos.snap(self.v[PEDAL] as f64);
        self.mix.snap(self.v[MIX] as f64);
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().max(1) as f64;
        let auto = self.v[MODE] > 0.5;
        let (f0, f1) = range(self.v[RANGE].round() as usize);
        let q = 2.0 + 8.0 * self.v[Q] as f64;
        let sens = self.v[SENS] as f64;
        let ca = coeff(self.fs, 8.0);
        let cr = coeff(self.fs, 180.0);
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch;
            let a = x.abs();
            self.env = a + if a > self.env { ca } else { cr } * (self.env - a);
            let target = if auto {
                (self.env * (2.0 + 40.0 * sens * sens)).min(1.0)
            } else {
                self.v[PEDAL] as f64
            };
            self.pos.set(target);
            let p = self.pos.next();
            if self.k == 0 {
                // the pedal's pot is an audio taper: most of the sweep is near the toe
                self.f_now = f0 * (f1 / f0).powf(p);
                for b in self.bp.iter_mut() {
                    b.set_bandpass(self.fs, self.f_now, q);
                }
            }
            self.k = (self.k + 1) % 16;
            let mx = self.mix.next();
            let o = self.out.next();
            for (ci, ch) in chans.iter_mut().enumerate() {
                let c = ci.min(self.bp.len() - 1);
                let d = ch[k] as f64;
                // a Cry Baby is mostly the resonant peak, with a little of the low end through it
                let w = self.bp[c].process(d) * (0.6 + 0.25 * q) + 0.12 * self.lp[c].process(d);
                ch[k] = ((d * (1.0 - mx) + w * mx) * o) as f32;
            }
        }
        meters[0] = ((self.f_now / f0).ln() / (f1 / f0).ln()) as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Cocked Half-Way (May's tone)", values: &[] },
        Preset { name: "Cocked Toe-Down (nasal lead)", values: &[(PEDAL, 0.75)] },
        Preset { name: "Cocked Heel (dark, vocal)", values: &[(PEDAL, 0.2)] },
        Preset { name: "Auto Wah, Funky", values: &[(MODE, 1.0), (SENS, 0.6), (Q, 0.7)] },
        Preset { name: "Auto Wah, Subtle", values: &[(MODE, 1.0), (SENS, 0.35), (Q, 0.4), (MIX, 0.7)] },
        Preset { name: "Wide Sweep (automate PEDAL)", values: &[(RANGE, 2.0), (Q, 0.6)] },
        Preset { name: "Vintage Narrow", values: &[(RANGE, 1.0), (Q, 0.5)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "WAH",
        subtitle: ("CRY BABY-STYLE WAH", "COCKED AS A TONE CONTROL · PEDAL · AUTO"),
        accent: ui::COBALT,
        size: (W, H),
        items: vec![

            Item::Knob { p: PEDAL, x: 130.0, y: 175.0, r: 50.0, label: "PEDAL", tip: "Pedal position, heel (0) to toe (100). Leave it in one place for a cocked wah, or automate it.", active: Some(ui::ActiveWhen { p: MODE, min: 0.0, max: 0.0 }) },
            Item::Lamp { m: 0, x: 255.0, y: 110.0, label: "SWEEP", lo: 0.0, hi: 1.0 },
            Item::Switch { p: MODE, x: 330.0, y: 175.0, labels: ["PEDAL", "AUTO"], tip: "PEDAL: you (or automation) set it. AUTO: your picking opens it." },
            Item::Selector { p: RANGE, x: 460.0, y: 175.0, label: "RANGE", positions: &["CRY BABY", "VINTAGE", "WIDE"], tip: "Sweep range: Cry Baby (about 380 Hz to 2.2 kHz), vintage narrow, or wide." },
            Item::Knob { p: Q, x: 570.0, y: 175.0, r: 28.0, label: "RESONANCE", tip: "How sharp the peak is (the inductor's Q).", active: None },
            Item::Divider { y: 262.0 },
            Item::Knob { p: SENS, x: 130.0, y: 330.0, r: 22.0, label: "AUTO SENS", tip: "AUTO mode: how far your playing opens it.", active: Some(ui::ActiveWhen { p: MODE, min: 1.0, max: 1.0 }) },
            Item::Knob { p: MIX, x: 320.0, y: 330.0, r: 22.0, label: "MIX", tip: "Dry vs wah.", active: None },
            Item::Knob { p: OUTPUT, x: 510.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Wah", class_id: *b"BMyWahPedalPQD01", vendor: "PEQUOD Brian May",
    categories: [Fx, Filter]
);

#[cfg(test)]
#[allow(dead_code)]
pub mod tests {
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

    use std::f64::consts::PI;
    fn sine_amp(f: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n).map(|k| (amp * (2.0 * PI * f * k as f64 / 48000.0).sin()) as f32).collect()
    }
    fn goertzel(x: &[f32], f: f64) -> f64 {
        let w = 2.0 * PI * f / 48000.0;
        let (mut s1, mut s2) = (0.0, 0.0);
        for v in x {
            let s = *v as f64 + 2.0 * w.cos() * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        ((s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2).max(0.0)).sqrt() / x.len() as f64
    }
    fn go(p: &[f32], x: &[f32]) -> Vec<f32> {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        input.remove(0)
    }
    fn go2(p: &[f32], x: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        let r = input.pop().unwrap();
        (input.pop().unwrap(), r)
    }

    fn peak_for(pedal: f32) -> f64 {
        let p = with(defaults(), &[(PEDAL, pedal), (MIX, 1.0)]);
        let mut best = (0.0, 0.0);
        let mut f = 250.0;
        while f < 4000.0 {
            let y = go(&p, &sine_amp(f, 0.1, 9600));
            let l = rms(&y[4800..]);
            if l > best.0 {
                best = (l, f);
            }
            f *= 1.06;
        }
        best.1
    }
    #[test]
    fn pedal_sweeps_the_peak() {
        let (a, b) = (peak_for(0.0), peak_for(1.0));
        println!("peak: heel {a:.0} Hz, toe {b:.0} Hz");
        assert!(a < 500.0 && b > 1800.0);
    }

}
