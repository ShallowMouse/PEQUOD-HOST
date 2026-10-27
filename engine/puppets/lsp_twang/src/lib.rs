//! Western Twang: the spaghetti-western guitar sound of The Age of the Understatement's bigger moments (after Morricone's records): an amp TREMOLO, a drippy SPRING reverb you can push with DWELL, and a SLAP echo, in one box for a clean guitar.

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
pub const H: u32 = 400;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "trem"]
    pub trem: FloatParam,
    #[id = "rate"]
    pub rate: FloatParam,
    #[id = "spring"]
    pub spring: FloatParam,
    #[id = "dwell"]
    pub dwell: FloatParam,
    #[id = "slap"]
    pub slap: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            trem: pct("Tremolo Depth", 0.4),
            rate: rate("Tremolo Rate", 5.0, 0.5, 12.0),
            spring: pct("Spring", 0.45),
            dwell: pct("Dwell", 0.5),
            slap: pct("Slap", 0.2),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const TREM: usize = 0;
pub const RATE: usize = 1;
pub const SPRING: usize = 2;
pub const DWELL: usize = 3;
pub const SLAP: usize = 4;
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
            self.trem.as_ptr(),
            self.rate.as_ptr(),
            self.spring.as_ptr(),
            self.dwell.as_ptr(),
            self.slap.as_ptr(),
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

const STAGES: usize = 32;

struct Spring {
    ap: [f64; STAGES],
    line: DelayLine,
    hp: Biquad,
    lp: Biquad,
    fb: f64,
}

impl Spring {
    fn new(fs: f64) -> Self {
        let mut hp = Biquad::identity();
        hp.set_highpass(fs, 300.0, 0.7);
        let mut lp = Biquad::identity();
        lp.set_lowpass(fs, 4200.0, 0.7);
        Self { ap: [0.0; STAGES], line: DelayLine::new((0.08 * fs) as usize), hp, lp, fb: 0.0 }
    }
    fn reset(&mut self) {
        self.ap = [0.0; STAGES];
        self.line.reset();
        self.hp.reset();
        self.lp.reset();
        self.fb = 0.0;
    }
    /// A chain of first-order allpasses smears the highs later than the lows (the chirp), round a 33 ms loop.
    #[inline]
    fn process(&mut self, x: f64, d: f64) -> f64 {
        let mut y = self.hp.process(x) + 0.78 * self.fb;
        let a = -0.62;
        for z in self.ap.iter_mut() {
            let o = a * y + *z;
            *z = y - a * o;
            y = o;
        }
        self.line.write(y);
        let out = self.lp.process(self.line.read(d));
        self.fb = out;
        out
    }
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    spring: [Spring; 2],
    slap: [DelayLine; 2],
    lfo: Lfo,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, spring: [Spring::new(fs), Spring::new(fs)], slap: [DelayLine::new((0.2 * fs) as usize), DelayLine::new((0.2 * fs) as usize)], lfo: Lfo::default(), out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.spring.iter_mut().for_each(|s| s.reset());
        self.slap.iter_mut().for_each(|s| s.reset());
        self.lfo = Lfo::default();
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let depth = self.v[TREM] as f64;
        let rate = self.v[RATE] as f64;
        let sp = self.v[SPRING] as f64;
        let dwell = 0.5 + 4.0 * self.v[DWELL] as f64;
        let sl = self.v[SLAP] as f64;
        let ds = [0.033 * self.fs, 0.037 * self.fs];
        let st = 0.115 * self.fs;
        let nch = chans.len().min(2);
        for k in 0..n {
            let l = self.lfo.next(self.fs, rate);
            let tg = 1.0 - depth * (0.5 + 0.5 * l);
            let o = self.out.next();
            for ci in 0..nch {
                let x = chans[ci][k] as f64 * tg;
                self.slap[ci].write(x);
                let s = self.slap[ci].read(st);
                let drive = ((x + 0.4 * s * sl) * dwell).tanh() / dwell.max(1.0).sqrt();
                let r = self.spring[ci].process(drive, ds[ci]);
                chans[ci][k] = ((x + sl * 0.5 * s + sp * 1.2 * r) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Spaghetti Western", values: &[] },
        Preset { name: "Surf Drip", values: &[(SPRING, 0.65), (DWELL, 0.75), (TREM, 0.0), (SLAP, 0.1)] },
        Preset { name: "Deep Tremolo", values: &[(TREM, 0.8), (RATE, 4.0), (SPRING, 0.35)] },
        Preset { name: "Fast Trem Shimmer", values: &[(TREM, 0.5), (RATE, 8.5), (SPRING, 0.3)] },
        Preset { name: "Twang Slapback", values: &[(SLAP, 0.55), (SPRING, 0.25), (TREM, 0.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "WESTERN TWANG",
        subtitle: ("TREMOLO · SPRING · SLAP", "THE SPAGHETTI-WESTERN GUITAR"),
        accent: ui::TEAL,
        size: (W, H),
        items: vec![

            Item::Knob { p: TREM, x: 80.0, y: 165.0, r: 32.0, label: "TREMOLO", tip: "Tremolo depth.", active: None },
            Item::Knob { p: RATE, x: 180.0, y: 165.0, r: 26.0, label: "RATE", tip: "Tremolo speed.", active: None },
            Item::Knob { p: SPRING, x: 320.0, y: 165.0, r: 36.0, label: "SPRING", tip: "Spring reverb level.", active: None },
            Item::Knob { p: DWELL, x: 440.0, y: 165.0, r: 26.0, label: "DWELL", tip: "How hard the spring is driven: more drip.", active: None },
            Item::Knob { p: SLAP, x: 560.0, y: 165.0, r: 26.0, label: "SLAP", tip: "A slap echo.", active: None },
            Item::Divider { y: 240.0 },
            Item::Knob { p: OUTPUT, x: 320.0, y: 300.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Western Twang", class_id: *b"LSPWesternTwang1", vendor: "PEQUOD Last Shadow Puppets",
    categories: [Fx, Reverb]
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
    fn tremolo_and_spring_tail() {
        let y = stereo(&with(defaults(), &[(TREM, 0.8), (RATE, 4.0), (SPRING, 0.0), (SLAP, 0.0)]), &sine(330.0, 0.2, 48000)).0;
        let e = env(&y[9600..], 480);
        let (lo, hi) = e.iter().fold((f64::MAX, 0.0f64), |(a, b), v| (a.min(*v), b.max(*v)));
        let mut x = vec![0.0f32; 48000];
        for k in 0..4800 {
            x[k] = (0.3 * (2.0 * PI * 330.0 * k as f64 / 48000.0).sin()) as f32;
        }
        let t = stereo(&with(defaults(), &[(TREM, 0.0), (SPRING, 1.0), (SLAP, 0.0)]), &x).0;
        let tail = db(rms(&t[14400..24000]));
        println!("tremolo swing {:.1} dB; spring tail {tail:.1} dB", db(hi) - db(lo));
        assert!(db(hi) - db(lo) > 10.0 && tail > -45.0 && t.iter().all(|v| v.is_finite()));
    }

}
