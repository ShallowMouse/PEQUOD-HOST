//! Flanger: after the MXR M117R Flanger: a very short sweeping delay mixed with the dry to make a moving comb filter, from a gentle jet-plane whoosh to metallic, ringing sweeps with REGEN (feedback, positive or negative). MANUAL sets the delay, WIDTH and SPEED the sweep.

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
pub const H: u32 = 330;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "manual"]
    pub manual: FloatParam,
    #[id = "width"]
    pub width: FloatParam,
    #[id = "speed"]
    pub speed: FloatParam,
    #[id = "regen"]
    pub regen: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            manual: pct("Manual", 0.5),
            width: pct("Width", 0.5),
            speed: rate("Speed", 0.3, 0.03, 6.0),
            regen: FloatParam::new("Regen", 0.45, FloatRange::Linear { min: -0.9, max: 0.9 }).with_step_size(0.01),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const MANUAL: usize = 0;
pub const WIDTH: usize = 1;
pub const SPEED: usize = 2;
pub const REGEN: usize = 3;
pub const OUTPUT: usize = 4;
pub const NPARAMS: usize = 5;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.manual.as_ptr(),
            self.width.as_ptr(),
            self.speed.as_ptr(),
            self.regen.as_ptr(),
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

/// the delay (ms) for a manual setting, LFO value (-1..1) and width
pub fn delay_ms(manual: f64, lfo: f64, width: f64) -> f64 {
    0.25 * 2f64.powf(manual * 4.8 + width * 2.2 * lfo)
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    line: Vec<DelayLine>,
    damp: Vec<OnePole>,
    lfo: Lfo,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut damp = vec![OnePole::default(); ch.max(2)];
        damp.iter_mut().for_each(|d| d.set_lowpass(fs, 9000.0));
        Self { fs, v, line: (0..ch.max(2)).map(|_| DelayLine::new((0.04 * fs) as usize)).collect(), damp, lfo: Lfo::default(), out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.line.iter_mut().for_each(|l| l.reset());
        self.lfo = Lfo::default();
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let (man, wid, sp, rg) = (self.v[MANUAL] as f64, self.v[WIDTH] as f64, self.v[SPEED] as f64, self.v[REGEN] as f64);
        let last = self.line.len() - 1;
        for k in 0..n {
            let l = self.lfo.next(self.fs, sp);
            let o = self.out.next();
            for (ci, c) in chans.iter_mut().enumerate() {
                let i = ci.min(last);
                let lv = if ci == 1 { (l * 0.6 + 0.8 * (1.0 - l * l).max(0.0).sqrt()).clamp(-1.0, 1.0) } else { l };
                let d = delay_ms(man, lv, wid) * 0.001 * self.fs;
                let x = c[k] as f64;
                let w = self.line[i].read(d.max(1.0));
                let fb = self.damp[i].lp(w) * rg;
                self.line[i].write((x + fb).clamp(-4.0, 4.0));
                c[k] = (0.5 * (x + w) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Jet Sweep", values: &[] },
        Preset { name: "Slow Metallic", values: &[(REGEN, 0.8), (SPEED, 0.12), (WIDTH, 0.7)] },
        Preset { name: "Negative Regen Hollow", values: &[(REGEN, -0.7)] },
        Preset { name: "Static Comb (manual)", values: &[(WIDTH, 0.0), (MANUAL, 0.6), (REGEN, 0.6)] },
        Preset { name: "Subtle Swirl", values: &[(REGEN, 0.15), (WIDTH, 0.35), (SPEED, 0.5)] },
        Preset { name: "Fast Warble", values: &[(SPEED, 3.0), (WIDTH, 0.25), (REGEN, 0.3)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "FLANGER",
        subtitle: ("AFTER THE MXR M117R", "MANUAL · WIDTH · SPEED · REGEN"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![
            Item::Knob { p: MANUAL, x: 90.0, y: 190.0, r: 36.0, label: "MANUAL", tip: "Delay time: where the sweep sits.", active: None },
            Item::Knob { p: WIDTH, x: 205.0, y: 190.0, r: 36.0, label: "WIDTH", tip: "How far it sweeps.", active: None },
            Item::Knob { p: SPEED, x: 320.0, y: 190.0, r: 36.0, label: "SPEED", tip: "Sweep rate.", active: None },
            Item::Knob { p: REGEN, x: 435.0, y: 190.0, r: 36.0, label: "REGEN", tip: "Feedback: right = ringing, left = hollow.", active: None },
            Item::Knob { p: OUTPUT, x: 550.0, y: 190.0, r: 36.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Flanger", class_id: *b"PqdFlangerM117Q1", vendor: "PEQUOD Pedals",
    categories: [Fx, Modulation]
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
    fn goertzel(x: &[f32], f: f64) -> f64 {
        let w = 2.0 * PI * f / 48000.0;
        let (mut s1, mut s2) = (0.0, 0.0);
        for v in x {
            let s = *v as f64 + 2.0 * w.cos() * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        ((s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2).max(0.0)).sqrt() / x.len() as f64 * 2.0
    }
    fn stereo(p: &[f32], x: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        let r = input.pop().unwrap();
        (input.pop().unwrap(), r)
    }
    fn corr(a: &[f32], b: &[f32]) -> f64 {
        let ab: f64 = a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum();
        let aa: f64 = a.iter().map(|x| (*x as f64).powi(2)).sum();
        let bb: f64 = b.iter().map(|x| (*x as f64).powi(2)).sum();
        ab / (aa * bb).sqrt().max(1e-12)
    }
    /// short-window rms envelope
    fn env(x: &[f32], w: usize) -> Vec<f64> {
        x.chunks(w).map(rms).collect()
    }

    #[test]
    fn comb_notch() {
        let p = with(defaults(), &[(WIDTH, 0.0), (REGEN, 0.0), (MANUAL, 0.5)]);
        let d = delay_ms(0.5, 0.0, 0.0) * 0.001;
        let notch = 1.0 / (2.0 * d);
        let peak = 1.0 / d;
        let a = rms(&stereo(&p, &sine(notch, 0.2, 24000)).0[12000..]);
        let b = rms(&stereo(&p, &sine(peak, 0.2, 24000)).0[12000..]);
        println!("notch {notch:.0} Hz {:.1} dB vs peak {peak:.0} Hz {:.1} dB", db(a), db(b));
        assert!(db(b) - db(a) > 20.0);
    }

}
