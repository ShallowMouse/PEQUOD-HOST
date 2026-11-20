//! Holy Grail: the Electro-Harmonix Holy Grail reverb, on Kevin Parker's board for years and on almost every
//! track on Currents (Mixdown). One Reverb knob and a three-way switch: Spring (a short, boingy spring tank
//! with its drip), Hall (a big smooth room) and Flerb (a hall with a flanger swept through its tail). Decay
//! times and the flanger's sweep are estimated from the pedal.

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
use vintage::verb::*;

pub const W: u32 = 640;
pub const H: u32 = 432;

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Mode { #[name = "Spring"] Spring, #[name = "Hall"] Hall, #[name = "Flerb"] Flerb }

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "reverb"]
    pub reverb: FloatParam,
    #[id = "mode"]
    pub mode: EnumParam<Mode>,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            reverb: pct("Reverb", 0.45),
            mode: EnumParam::new("Mode", Mode::Hall),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const REVERB: usize = 0;
pub const MODE: usize = 1;
pub const OUTPUT: usize = 2;
pub const NPARAMS: usize = 3;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.reverb.as_ptr(),
            self.mode.as_ptr(),
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
    fdn: Fdn,
    // the spring: a chain of all-passes that smears the high frequencies late (its chirp)
    disp: [f64; 12],
    tone: [Biquad; 2],
    flange: DelayLine,
    lfo: f64,
    key: i32,
    out: Smooth,
    mix: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        match (self.v[MODE].round() as i32).clamp(0, 2) {
            0 => {
                self.fdn.configure(0.8, [2.2, 2.0, 1.2], 2500.0);
                self.fdn.mod_depth = 0.0008;
                self.tone[0].set_highpass(fs, 250.0, 0.7);
                self.tone[1].set_lowpass(fs, 4500.0, 0.8);
            }
            1 => {
                self.fdn.configure(3.2, [3.0, 2.6, 1.8], 3000.0);
                self.fdn.mod_depth = 0.0015;
                self.tone[0].set_highpass(fs, 120.0, 0.7);
                self.tone[1].set_lowpass(fs, 7000.0, 0.7);
            }
            _ => {
                self.fdn.configure(3.2, [3.2, 2.8, 2.0], 3000.0);
                self.fdn.mod_depth = 0.0015;
                self.tone[0].set_highpass(fs, 150.0, 0.7);
                self.tone[1].set_lowpass(fs, 7500.0, 0.7);
            }
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self {
            fs, v, fdn: Fdn::new(fs, 4.1), disp: [0.0; 12], tone: [Biquad::identity(); 2],
            flange: DelayLine::new((0.02 * fs) as usize), lfo: 0.0, key: -1,
            out: Smooth::new(fs, 20.0, 1.0), mix: Smooth::new(fs, 20.0, 0.45),
        };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.fdn.reset();
        self.disp = [0.0; 12];
        self.tone.iter_mut().for_each(|b| b.reset());
        self.flange.reset();
        self.lfo = 0.0;
        self.design();
        self.mix.snap(self.v[REVERB] as f64);
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = v[MODE].round() as i32;
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.mix.set(v[REVERB] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let mode = (self.v[MODE].round() as i32).clamp(0, 2);
        let nch = chans.len();
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            let mut s = cascade(&mut self.tone, x);
            if mode == 0 {
                // the spring's dispersion: high frequencies arrive after the lows
                let a = 0.62;
                for st in self.disp.iter_mut() {
                    let o = a * s + *st;
                    *st = s - a * o;
                    s = o;
                }
            }
            let d = self.fdn.diffuse(s, if mode == 0 { 2 } else { 4 });
            let (mut l, mut r) = self.fdn.process(d);
            if mode == 2 {
                // the flanger swept through the tail (slow: about 0.3 Hz)
                self.lfo = (self.lfo + 0.3 / self.fs).fract();
                let dl = (0.001 + 0.004 * (0.5 + 0.5 * (std::f64::consts::TAU * self.lfo).sin())) * self.fs;
                let m = 0.5 * (l + r);
                self.flange.write(m);
                let f = self.flange.read(dl);
                l = 0.6 * l + 0.6 * f;
                r = 0.6 * r + 0.6 * f;
            }
            let mix = self.mix.next();
            let (dg, wg) = ((1.0 - 0.5 * mix).sqrt(), mix * 1.3);
            let o = self.out.next();
            if nch >= 2 {
                chans[0][k] = ((chans[0][k] as f64 * dg + l * wg) * o) as f32;
                chans[1][k] = ((chans[1][k] as f64 * dg + r * wg) * o) as f32;
            } else if nch == 1 {
                chans[0][k] = ((chans[0][k] as f64 * dg + 0.5 * (l + r) * wg) * o) as f32;
            }
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Currents Hall", values: &[] },
        Preset { name: "Surf Spring", values: &[(MODE, 0.0), (REVERB, 0.5)] },
        Preset { name: "Flerb Wash", values: &[(MODE, 2.0), (REVERB, 0.65)] },
        Preset { name: "A Touch of Room", values: &[(MODE, 1.0), (REVERB, 0.2)] },
        Preset { name: "All Reverb", values: &[(MODE, 1.0), (REVERB, 1.0), (OUTPUT, -3.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "HOLY GRAIL",
        subtitle: ("SPRING · HALL · FLERB", "THE REVERB ON ALMOST EVERY TRACK OF CURRENTS"),
        accent: ui::PINE,
        size: (W, H),
        items: vec![
            Item::Knob { p: REVERB, x: 220.0, y: 190.0, r: 46.0, label: "REVERB", tip: "How much reverb.", active: None },
            Item::Selector { p: MODE, x: 430.0, y: 190.0, label: "MODE", positions: &["SPRING", "HALL", "FLERB"], tip: "Spring: a short spring tank. Hall: a big smooth room. Flerb: the hall with a flanger swept through it." },
            Item::Knob { p: OUTPUT, x: 560.0, y: 340.0, r: 18.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Holy Grail", class_id: *b"TImpHolyGrailPQ1", vendor: "PEQUOD Tame Impala",
    categories: [Fx, Reverb]
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
    fn hall_rings_longer_than_spring() {
        let tail = |m: f32| {
            let p = with(defaults(), &[(MODE, m), (REVERB, 1.0)]);
            let mut x = vec![0.0f32; 96000];
            for k in 0..480 { x[k] = (0.4 * (k as f64 * 0.3).sin()) as f32; }
            let y = go(&p, &x);
            rms(&y[48000..72000])
        };
        println!("tail at 1-1.5 s: spring {:.5}, hall {:.5}", tail(0.0), tail(1.0));
        assert!(tail(1.0) > 2.0 * tail(0.0));
    }

}
