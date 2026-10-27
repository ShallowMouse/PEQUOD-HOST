//! Elfman Vocal: a vocal chain for Danny Elfman-style lead vocals on Only a Lad: fast compression and presence for a sharp, nervous, theatrical voice, a short SLAP echo, and OCTAVE UP and OCTAVE DOWN voices for the band jumping into the vocal line an octave away, as on On the Outside. Settings estimated.

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
use vintage::fx::comp_gr;

pub const W: u32 = 640;
pub const H: u32 = 410;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "comp"]
    pub comp: FloatParam,
    #[id = "presence"]
    pub presence: FloatParam,
    #[id = "octup"]
    pub octup: FloatParam,
    #[id = "octdown"]
    pub octdown: FloatParam,
    #[id = "slap"]
    pub slap: FloatParam,
    #[id = "slaptime"]
    pub slaptime: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            comp: pct("Compression", 0.5),
            presence: db("Presence", 4.0, -3.0, 10.0),
            octup: pct("Octave Up", 0.0),
            octdown: pct("Octave Down", 0.0),
            slap: pct("Slap", 0.2),
            slaptime: ms("Slap Time", 110.0, 50.0, 200.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const COMP: usize = 0;
pub const PRESENCE: usize = 1;
pub const OCTUP: usize = 2;
pub const OCTDOWN: usize = 3;
pub const SLAP: usize = 4;
pub const SLAPTIME: usize = 5;
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
            self.comp.as_ptr(),
            self.presence.as_ptr(),
            self.octup.as_ptr(),
            self.octdown.as_ptr(),
            self.slap.as_ptr(),
            self.slaptime.as_ptr(),
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


pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    eq: [Biquad; 3],
    rms: f64,
    gr: f64,
    up: SyncShift,
    down: SyncShift,
    slap: DelayLine,
    slap_lp: OnePole,
    key: i32,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        self.eq[0].set_highpass(fs, 100.0, 0.7);
        self.eq[1].set_peak(fs, 4500.0, 0.9, self.v[PRESENCE] as f64);
        self.eq[2].set_peak(fs, 250.0, 1.0, -2.0);
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut slap_lp = OnePole::default();
        slap_lp.set_lowpass(fs, 5000.0);
        let mut d = Self { fs, v, eq: [Biquad::identity(); 3], rms: 0.0, gr: 0.0, up: SyncShift::new(fs, 30.0), down: SyncShift::new(fs, 45.0),
            slap: DelayLine::new((0.21 * fs) as usize), slap_lp, key: -999, out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.eq.iter_mut().for_each(|b| b.reset());
        self.up.reset();
        self.down.reset();
        self.slap.reset();
        self.rms = 0.0;
        self.gr = 0.0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = (v[PRESENCE] * 10.0) as i32;
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let thr = -6.0 - 28.0 * self.v[COMP] as f64;
        let (ou, od, sl) = (self.v[OCTUP] as f64, self.v[OCTDOWN] as f64, self.v[SLAP] as f64);
        let st = self.v[SLAPTIME] as f64 * 0.001 * self.fs;
        let cav = coeff(self.fs, 5.0);
        let ca = coeff(self.fs, 2.0);
        let cr = coeff(self.fs, 90.0);
        let mut most = 0.0f64;
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            let e = cascade(&mut self.eq, x);
            self.rms = e * e + cav * (self.rms - e * e);
            let lev = 10.0 * self.rms.max(1e-12).log10() + 3.0;
            let t = comp_gr(lev, thr, 5.0, 6.0);
            self.gr = t + if t > self.gr { ca } else { cr } * (self.gr - t);
            most = most.max(self.gr);
            let v = e * db_to_gain(-self.gr + 0.5 * self.gr.min(14.0));
            let up = self.up.process(v, 2.0);
            let dn = self.down.process(v, 0.5);
            let s = self.slap.read(st);
            self.slap.write(v);
            let s = self.slap_lp.lp(s);
            let lead = v + sl * 0.6 * s;
            let o = self.out.next();
            if nch >= 2 {
                chans[0][k] = ((lead + ou * 0.7 * up + od * 0.6 * dn) * o) as f32;
                chans[1][k] = ((lead + ou * 0.6 * up + od * 0.7 * dn) * o) as f32;
            } else {
                chans[0][k] = ((lead + ou * 0.65 * up + od * 0.65 * dn) * o) as f32;
            }
        }
        meters[0] = most as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Elfman Lead", values: &[] },
        Preset { name: "Nervous and Upfront", values: &[(COMP, 0.75), (PRESENCE, 7.0), (SLAP, 0.15)] },
        Preset { name: "Octave Leap (band in the line)", values: &[(OCTUP, 0.6), (OCTDOWN, 0.35)] },
        Preset { name: "Gang Octaves Below", values: &[(OCTDOWN, 0.7)] },
        Preset { name: "Slapback Shout", values: &[(SLAP, 0.45), (SLAPTIME, 95.0)] },
        Preset { name: "Theatrical Croon", values: &[(COMP, 0.35), (PRESENCE, 2.0), (SLAP, 0.3), (SLAPTIME, 160.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "ELFMAN VOCAL",
        subtitle: ("DANNY ELFMAN-STYLE LEAD VOCAL", "COMP · PRESENCE · SLAP · OCTAVE VOICES"),
        accent: ui::STEEL,
        size: (W, H),
        items: vec![

            Item::Knob { p: COMP, x: 90.0, y: 160.0, r: 38.0, label: "COMP", tip: "Fast compression for an upfront, even voice.", active: None },
            Item::Knob { p: PRESENCE, x: 220.0, y: 160.0, r: 32.0, label: "PRESENCE", tip: "Bite at 4.5 kHz.", active: None },
            Item::Meter { m: 0, x: 300.0, y: 96.0, w: 160.0, h: 110.0, kind: MeterKind::GainReduction, caption: "COMP" },
            Item::Knob { p: SLAP, x: 550.0, y: 160.0, r: 32.0, label: "SLAP", tip: "A short slap echo.", active: None },
            Item::Divider { y: 240.0 },
            Item::Knob { p: OCTUP, x: 110.0, y: 305.0, r: 26.0, label: "OCTAVE UP", tip: "A voice an octave up, as the band jumps into the line.", active: None },
            Item::Knob { p: OCTDOWN, x: 250.0, y: 305.0, r: 26.0, label: "OCTAVE DOWN", tip: "A voice an octave down.", active: None },
            Item::Knob { p: SLAPTIME, x: 390.0, y: 305.0, r: 26.0, label: "SLAP TIME", tip: "Slap echo time.", active: None },
            Item::Knob { p: OUTPUT, x: 530.0, y: 305.0, r: 26.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Elfman Vocal", class_id: *b"OBElfmanVocalPQ1", vendor: "PEQUOD Oingo Boingo",
    categories: [Fx, Dynamics]
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
    fn octave_voices() {
        let a = stereo(&with(defaults(), &[(SLAP, 0.0)]), &sine(220.0, 0.2, 48000)).0;
        let b = stereo(&with(defaults(), &[(SLAP, 0.0), (OCTUP, 1.0)]), &sine(220.0, 0.2, 48000)).0;
        let r = |y: &[f32]| db(goertzel(&y[24000..], 440.0) / goertzel(&y[24000..], 220.0));
        println!("440/220: off {:.1} dB, octave up {:.1} dB", r(&a), r(&b));
        assert!(r(&b) > r(&a) + 20.0 && r(&b) > -10.0);
    }

}
