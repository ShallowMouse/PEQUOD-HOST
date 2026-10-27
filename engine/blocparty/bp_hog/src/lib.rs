//! HOG: a harmonic octave generator after the Electro-Harmonix HOG Russell Lissack pairs with distortion (Hunting for Witches): two octaves down, one down, the dry note, a fifth up, an octave up, an octave and a fifth, and two octaves up, mixed like drawbars, with a low-pass and a SWELL that fades the added voices in after each attack. Phase-matched splicing keeps sustained notes steady.

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



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "sub2"]
    pub sub2: FloatParam,
    #[id = "sub"]
    pub sub: FloatParam,
    #[id = "dry"]
    pub dry: FloatParam,
    #[id = "fifth"]
    pub fifth: FloatParam,
    #[id = "oct"]
    pub oct: FloatParam,
    #[id = "oct5"]
    pub oct5: FloatParam,
    #[id = "oct2"]
    pub oct2: FloatParam,
    #[id = "filter"]
    pub filter: FloatParam,
    #[id = "swell"]
    pub swell: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            sub2: pct("-2 Octaves", 0.0),
            sub: pct("-1 Octave", 0.4),
            dry: pct("Dry", 1.0),
            fifth: pct("+5th", 0.0),
            oct: pct("+1 Octave", 0.5),
            oct5: pct("+Octave+5th", 0.0),
            oct2: pct("+2 Octaves", 0.0),
            filter: hz("Low-pass", 9000.0, 800.0, 16000.0),
            swell: ms("Swell", 0.0, 0.0, 2000.0),
            output: db("Output", -3.0, -24.0, 12.0),
        }
    }
}

pub const SUB2: usize = 0;
pub const SUB: usize = 1;
pub const DRY: usize = 2;
pub const FIFTH: usize = 3;
pub const OCT: usize = 4;
pub const OCT5: usize = 5;
pub const OCT2: usize = 6;
pub const FILTER: usize = 7;
pub const SWELL: usize = 8;
pub const OUTPUT: usize = 9;
pub const NPARAMS: usize = 10;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.sub2.as_ptr(),
            self.sub.as_ptr(),
            self.dry.as_ptr(),
            self.fifth.as_ptr(),
            self.oct.as_ptr(),
            self.oct5.as_ptr(),
            self.oct2.as_ptr(),
            self.filter.as_ptr(),
            self.swell.as_ptr(),
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
/// Two-head pitch shifter that splices at the best-matching point (cross-correlation
/// search), so the heads join in phase: no warble or pitch error on sustained notes.
pub struct SyncShift {
    line: DelayLine,
    d: [f64; 2],
    active: usize,
    fading: usize,
    fade: usize,
    lo: f64,
    hi: f64,
    search: i64,
    m: usize,
}

impl SyncShift {
    pub fn new(fs: f64, win_ms: f64) -> Self {
        let m = 96;
        let lo = (m * 4 + 8) as f64;
        let win = win_ms * 0.001 * fs;
        let search = (0.009 * fs) as i64;
        let hi = lo + win;
        let len = (hi + search as f64 + 16.0) as usize;
        Self { line: DelayLine::new(len), d: [hi, hi], active: 0, fading: 0, fade: (0.008 * fs) as usize, lo, hi, search, m }
    }
    pub fn reset(&mut self) {
        self.line.reset();
        self.d = [self.hi, self.hi];
        self.active = 0;
        self.fading = 0;
    }
    fn best(&self, cur: f64, base: f64, r: f64) -> f64 {
        let mut best = (f64::MIN, base);
        let mut k = -self.search;
        while k <= self.search {
            let cand = base + k as f64;
            let (mut ab, mut aa, mut bb) = (0.0, 0.0, 0.0);
            for j in 0..self.m {
                let a = self.line.read_int((cur - j as f64 * r).max(1.0).round() as usize);
                let b = self.line.read_int((cand - j as f64 * r).max(1.0).round() as usize);
                ab += a * b;
                aa += a * a;
                bb += b * b;
            }
            let score = ab / (aa * bb + 1e-12).sqrt();
            if score > best.0 {
                best = (score, cand);
            }
            k += 2;
        }
        best.1
    }
    #[inline]
    pub fn process(&mut self, x: f64, r: f64) -> f64 {
        self.line.write(x);
        let rate = 1.0 - r;
        self.d[0] += rate;
        self.d[1] += rate;
        let a = self.active;
        let o = 1 - a;
        if self.fading == 0 {
            let need = if r > 1.0 { self.d[a] <= self.lo } else { self.d[a] >= self.hi };
            if need {
                let base = if r > 1.0 { self.hi } else { self.lo + self.search as f64 + 2.0 };
                self.d[o] = self.best(self.d[a], base, r);
                self.fading = self.fade;
            }
        }
        if self.fading > 0 {
            let t = 1.0 - self.fading as f64 / self.fade as f64;
            let th = t * std::f64::consts::FRAC_PI_2;
            let y = self.line.read(self.d[a].max(1.0)) * th.cos() + self.line.read(self.d[o].max(1.0)) * th.sin();
            self.fading -= 1;
            if self.fading == 0 {
                self.active = o;
            }
            y
        } else {
            self.line.read(self.d[a].clamp(1.0, self.line.max_delay()))
        }
    }
}


pub const RATIOS: [f64; 6] = [0.25, 0.5, 1.4983070768766815, 2.0, 2.996614153753363, 4.0];

struct Chan {
    ps: [SyncShift; 6],
    lp: Biquad,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ch: Vec<Chan>,
    env: f64,
    swell: f64,
    lp_key: i32,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mk = || Chan { ps: [SyncShift::new(fs, 60.0), SyncShift::new(fs, 40.0), SyncShift::new(fs, 30.0), SyncShift::new(fs, 30.0), SyncShift::new(fs, 30.0), SyncShift::new(fs, 30.0)], lp: Biquad::identity() };
        Self { fs, v, ch: (0..ch.max(2)).map(|_| mk()).collect(), env: 0.0, swell: 1.0, lp_key: -1, out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        for c in self.ch.iter_mut() {
            c.ps.iter_mut().for_each(|p| p.reset());
            c.lp.reset();
        }
        self.env = 0.0;
        self.swell = 1.0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        if v[FILTER] as i32 != self.lp_key {
            self.lp_key = v[FILTER] as i32;
            for c in self.ch.iter_mut() {
                c.lp.set_lowpass(self.fs, v[FILTER] as f64, 0.707);
            }
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let lv = [self.v[SUB2] as f64, self.v[SUB] as f64, self.v[FIFTH] as f64, self.v[OCT] as f64, self.v[OCT5] as f64, self.v[OCT2] as f64];
        let dry = self.v[DRY] as f64;
        let sw_ms = self.v[SWELL] as f64;
        let rise = if sw_ms > 1.0 { 1.0 / (sw_ms * 0.001 * self.fs) } else { 1.0 };
        let ce = coeff(self.fs, 30.0);
        let last = self.ch.len() - 1;
        let nch = chans.len().max(1) as f64;
        for k in 0..n {
            // SWELL: a new attack (level jumping well above its recent average) restarts the fade-in
            let mut x = 0.0;
            for c in chans.iter() {
                x += (c[k] as f64).abs();
            }
            x /= nch;
            if x > 3.0 * self.env + 0.01 {
                self.swell = 0.0;
            }
            self.env = x + ce * (self.env - x);
            self.swell = (self.swell + rise).min(1.0);
            let o = self.out.next();
            for (ci, chn) in chans.iter_mut().enumerate() {
                let c = &mut self.ch[ci.min(last)];
                let x = chn[k] as f64;
                let mut add = 0.0;
                for i in 0..6 {
                    if lv[i] > 0.0 {
                        add += c.ps[i].process(x, RATIOS[i]) * lv[i];
                    }
                }
                let y = c.lp.process(x * dry + add * self.swell);
                chn[k] = (y * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Hunting for Witches (octaves)", values: &[] },
        Preset { name: "Organ Stack", values: &[(SUB, 0.5), (FIFTH, 0.3), (OCT, 0.6), (OCT2, 0.3), (FILTER, 6000.0)] },
        Preset { name: "Big Sub", values: &[(SUB2, 0.4), (SUB, 0.8), (OCT, 0.0)] },
        Preset { name: "Shimmer Swell", values: &[(SUB, 0.0), (OCT, 0.6), (OCT5, 0.4), (OCT2, 0.5), (SWELL, 900.0)] },
        Preset { name: "Power Fifths", values: &[(SUB, 0.5), (FIFTH, 0.7), (OCT, 0.3)] },
        Preset { name: "Octave Up Only", values: &[(DRY, 0.0), (SUB, 0.0), (OCT, 1.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "HOG",
        subtitle: ("HARMONIC OCTAVE GENERATOR", "SEVEN VOICES · SWELL"),
        accent: ui::INDIGO,
        size: (W, H),
        items: vec![

            Item::Knob { p: SUB2, x: 60.0, y: 175.0, r: 26.0, label: "-2 OCT", tip: "Two octaves down.", active: None },
            Item::Knob { p: SUB, x: 145.0, y: 175.0, r: 26.0, label: "-1 OCT", tip: "One octave down.", active: None },
            Item::Knob { p: DRY, x: 230.0, y: 175.0, r: 26.0, label: "DRY", tip: "Your own note.", active: None },
            Item::Knob { p: FIFTH, x: 315.0, y: 175.0, r: 26.0, label: "+5TH", tip: "A fifth up.", active: None },
            Item::Knob { p: OCT, x: 400.0, y: 175.0, r: 26.0, label: "+1 OCT", tip: "One octave up.", active: None },
            Item::Knob { p: OCT5, x: 485.0, y: 175.0, r: 26.0, label: "+OCT+5", tip: "An octave and a fifth up.", active: None },
            Item::Knob { p: OCT2, x: 570.0, y: 175.0, r: 26.0, label: "+2 OCT", tip: "Two octaves up.", active: None },
            Item::Divider { y: 265.0 },
            Item::Knob { p: FILTER, x: 130.0, y: 330.0, r: 24.0, label: "LOW-PASS", tip: "Takes the fizz off.", active: None },
            Item::Knob { p: SWELL, x: 320.0, y: 330.0, r: 24.0, label: "SWELL", tip: "The added voices fade in over this time after each attack (0 = instant).", active: None },
            Item::Knob { p: OUTPUT, x: 510.0, y: 330.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "HOG", class_id: *b"BlpHOGOctavePQD1", vendor: "PEQUOD Bloc Party",
    categories: [Fx, PitchShift]
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
    fn fifth_and_octaves() {
        let p = with(defaults(), &[(DRY, 0.0), (SUB, 0.0), (OCT, 0.0), (FIFTH, 1.0)]);
        let y = go(&p, &sine_amp(220.0, 0.3, 48000));
        let a = goertzel(&y[24000..], 329.6);
        let b = goertzel(&y[24000..], 220.0);
        println!("+5th: 330 Hz {:.1} dB vs 220 Hz {:.1} dB", db(a), db(b));
        assert!(db(a) > db(b) + 15.0);
    }

}
