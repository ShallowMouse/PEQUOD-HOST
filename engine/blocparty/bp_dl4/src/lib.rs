//! DL4: a multi-mode delay after the Line 6 DL4 on Kele Okereke's board, with the reversed delays Russell Lissack stacks to open Silent Alarm (Like Eating Glass). Modes: DIGITAL (clean), TAPE (darker, with wow and saturation), ANALOG (bucket-brigade dark and soft), REVERSE (each slice of TIME played backwards, swelling in). TWEAK is the tone in every mode.

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
pub enum Mode { #[name = "Digital"] Digital, #[name = "Tape"] Tape, #[name = "Analog"] Analog, #[name = "Reverse"] Reverse }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "mode"]
    pub mode: EnumParam<Mode>,
    #[id = "time"]
    pub time: FloatParam,
    #[id = "repeats"]
    pub repeats: FloatParam,
    #[id = "tweak"]
    pub tweak: FloatParam,
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
            mode: EnumParam::new("Mode", Mode::Digital),
            time: ms("Time", 400.0, 30.0, 2500.0),
            repeats: pct("Repeats", 0.35),
            tweak: pct("Tweak (Tone)", 0.5),
            mix: pct("Mix", 0.35),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const MODE: usize = 0;
pub const TIME: usize = 1;
pub const REPEATS: usize = 2;
pub const TWEAK: usize = 3;
pub const MIX: usize = 4;
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
            self.mode.as_ptr(),
            self.time.as_ptr(),
            self.repeats.as_ptr(),
            self.tweak.as_ptr(),
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

struct Chan {
    dl: DelayLine,
    lp: Biquad,
    hp: Biquad,
    fb: f64,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ch: Vec<Chan>,
    t: Smooth,
    wow: f64,
    rpos: f64,
    key: (i32, i32),
    mix: Smooth,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let mode = (self.v[MODE].round() as usize).min(3);
        let tw = self.v[TWEAK] as f64;
        let top = match mode {
            0 => 3000.0 * 5f64.powf(tw),
            1 => 1800.0 * 4f64.powf(tw),
            2 => 1200.0 * 4f64.powf(tw),
            _ => 2500.0 * 5f64.powf(tw),
        };
        for c in self.ch.iter_mut() {
            c.lp.set_lowpass(self.fs, top.min(0.45 * self.fs), 0.707);
            c.hp.set_highpass(self.fs, if mode == 2 { 90.0 } else { 50.0 }, 0.7);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mk = || Chan { dl: DelayLine::new((5.2 * fs) as usize), lp: Biquad::identity(), hp: Biquad::identity(), fb: 0.0 };
        let mut d = Self { fs, v, ch: (0..ch.max(2)).map(|_| mk()).collect(), t: Smooth::new(fs, 80.0, 0.4 * fs), wow: 0.0, rpos: 0.0, key: (-1, -1),
                           mix: Smooth::new(fs, 20.0, 0.35), out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        for c in self.ch.iter_mut() {
            c.dl.reset();
            c.lp.reset();
            c.hp.reset();
            c.fb = 0.0;
        }
        self.rpos = 0.0;
        self.t.snap(self.v[TIME] as f64 * 0.001 * self.fs);
        self.mix.snap(self.v[MIX] as f64);
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = (v[MODE] as i32, (v[TWEAK] * 400.0) as i32);
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let mode = (self.v[MODE].round() as usize).min(3);
        let fb = self.v[REPEATS] as f64 * 1.02;
        self.t.set(self.v[TIME] as f64 * 0.001 * self.fs);
        let last = self.ch.len() - 1;
        let pi = std::f64::consts::PI;
        for k in 0..n {
            let t = self.t.next();
            self.wow = (self.wow + 0.7 / self.fs).fract();
            let m = self.mix.next();
            let o = self.out.next();
            // reverse: two heads half a slice apart, each reading its slice backwards
            self.rpos = (self.rpos + 1.0) % t.max(2.0);
            for (ci, chn) in chans.iter_mut().enumerate() {
                let c = &mut self.ch[ci.min(last)];
                let x = chn[k] as f64;
                let y = match mode {
                    3 => {
                        let mut acc = 0.0;
                        for h in 0..2 {
                            let p = (self.rpos + h as f64 * 0.5 * t) % t;
                            // within a slice of length t the read point runs backwards: delay grows by 2 per sample
                            let d = 1.0 + 2.0 * p;
                            let w = (pi * p / t).sin().powi(2);
                            acc += c.dl.read(d) * w;
                        }
                        acc
                    }
                    1 => c.dl.read(t * (1.0 + 0.003 * (2.0 * pi * self.wow).sin())),
                    _ => c.dl.read(t),
                };
                let y = c.hp.process(c.lp.process(y));
                let y = match mode {
                    1 => (y * 1.3).tanh() / 1.3,
                    2 => (y * 1.1).tanh() / 1.1,
                    _ => y,
                };
                c.dl.write(x + c.fb);
                c.fb = y * fb;
                chn[k] = ((x * (1.0 - m) + y * m) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Kele Digital Delay", values: &[] },
        Preset { name: "Like Eating Glass (reverse)", values: &[(MODE, 3.0), (TIME, 900.0), (REPEATS, 0.45), (MIX, 0.5)] },
        Preset { name: "Reverse Swell, Short", values: &[(MODE, 3.0), (TIME, 400.0), (REPEATS, 0.2), (MIX, 0.45)] },
        Preset { name: "Tape Echo", values: &[(MODE, 1.0), (TIME, 350.0), (REPEATS, 0.4), (TWEAK, 0.4)] },
        Preset { name: "Analog Dark", values: &[(MODE, 2.0), (TIME, 300.0), (REPEATS, 0.5), (TWEAK, 0.35)] },
        Preset { name: "Dotted Eighth (120 bpm)", values: &[(TIME, 375.0), (REPEATS, 0.4), (MIX, 0.35)] },
        Preset { name: "Ambient Wash", values: &[(TIME, 1200.0), (REPEATS, 0.7), (MIX, 0.45), (TWEAK, 0.3)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "DL4",
        subtitle: ("AFTER THE LINE 6 DL4", "DIGITAL · TAPE · ANALOG · REVERSE"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![

            Item::Selector { p: MODE, x: 90.0, y: 175.0, label: "MODE", positions: &["DIGITAL", "TAPE", "ANALOG", "REVERSE"], tip: "Delay type. REVERSE plays each slice of TIME backwards (Like Eating Glass)." },
            Item::Knob { p: TIME, x: 230.0, y: 175.0, r: 38.0, label: "TIME", tip: "Delay time (in REVERSE: the length of each reversed slice).", active: None },
            Item::Knob { p: REPEATS, x: 360.0, y: 175.0, r: 38.0, label: "REPEATS", tip: "Feedback.", active: None },
            Item::Knob { p: TWEAK, x: 480.0, y: 175.0, r: 30.0, label: "TWEAK", tip: "Tone of the repeats in every mode.", active: None },
            Item::Knob { p: MIX, x: 580.0, y: 175.0, r: 30.0, label: "MIX", tip: "Dry vs delay.", active: None },
            Item::Divider { y: 265.0 },
            Item::Knob { p: OUTPUT, x: 320.0, y: 330.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "DL4", class_id: *b"BlpDL4DelayPQD01", vendor: "PEQUOD Bloc Party",
    categories: [Fx, Delay]
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
    /// harmonics (2..6) relative to the fundamental, dB
    fn thd(p: &[f32], f: f64, amp: f64) -> f64 {
        let y = go(p, &sine_amp(f, amp, 48000));
        let a = goertzel(&y[24000..], f);
        let h: f64 = (2..7).map(|k| goertzel(&y[24000..], f * k as f64).powi(2)).sum::<f64>().sqrt();
        db(h / a.max(1e-12))
    }

    #[test]
    fn digital_echo_lands() {
        let p = with(defaults(), &[(TIME, 250.0), (REPEATS, 0.0), (MIX, 0.5)]);
        let mut x = vec![0.0f32; 24000];
        x[100] = 1.0;
        let y = go(&p, &x);
        let pk = y.iter().enumerate().skip(1000).max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap()).unwrap().0 as f64 / 48.0;
        println!("echo at {pk:.1} ms");
        assert!((pk - 252.1).abs() < 2.0);
    }
    #[test]
    fn reverse_plays_backwards() {
        // a fast rising sweep comes back falling inside each reversed slice
        let p = with(defaults(), &[(MODE, 3.0), (TIME, 500.0), (REPEATS, 0.0), (MIX, 1.0), (TWEAK, 1.0)]);
        let n = 48000 * 3;
        let x: Vec<f32> = (0..n).map(|k| { let t = k as f64 / 48000.0; (0.3 * (2.0 * PI * (200.0 * t + 700.0 * t * t)).sin()) as f32 }).collect();
        let y = go(&p, &x);
        let zc = |a: &[f32]| a.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count() as i64;
        let t = 24000usize;
        let mut falling = 0;
        for k in 1..5 {
            // where head 0 is loudest (middle of its slice), compare just before with just after
            let c = k * t + t / 2;
            let a = zc(&y[c - 3600..c - 2160]);
            let b = zc(&y[c + 2160..c + 3600]);
            if a > b {
                falling += 1;
            }
            println!("slice {k}: {a} then {b} crossings");
        }
        assert!(falling >= 3);
    }

}
