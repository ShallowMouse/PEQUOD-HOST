//! Harmonizer: an intelligent harmony pedal: it tracks the note you play, finds it in the KEY and SCALE you set, and adds a second voice a diatonic third, fourth, fifth, sixth or octave away, so the harmony stays in key the way a second guitarist's would. For single-note lines (twin-guitar leads in the Thin Lizzy and Iron Maiden manner).

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
use vintage::pitch::Pitch;

pub const W: u32 = 640;
pub const H: u32 = 440;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Key { C, #[name = "C#"] Cs, D, #[name = "Eb"] Eb, E, F, #[name = "F#"] Fs, G, #[name = "Ab"] Ab, A, #[name = "Bb"] Bb, B }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Scale { Major, Minor }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Interval {
    #[name = "3rd up"] ThirdUp, #[name = "4th up"] FourthUp, #[name = "5th up"] FifthUp, #[name = "6th up"] SixthUp,
    #[name = "Octave up"] OctUp, #[name = "3rd down"] ThirdDown, #[name = "4th down"] FourthDown, #[name = "6th down"] SixthDown, #[name = "Octave down"] OctDown,
}


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "key"]
    pub key: EnumParam<Key>,
    #[id = "scale"]
    pub scale: EnumParam<Scale>,
    #[id = "interval"]
    pub interval: EnumParam<Interval>,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "dry"]
    pub dry: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            key: EnumParam::new("Key", Key::E),
            scale: EnumParam::new("Scale", Scale::Minor),
            interval: EnumParam::new("Interval", Interval::ThirdUp),
            mix: pct("Harmony Level", 0.5),
            dry: pct("Dry Level", 1.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const KEY: usize = 0;
pub const SCALE: usize = 1;
pub const INTERVAL: usize = 2;
pub const MIX: usize = 3;
pub const DRY: usize = 4;
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
            self.key.as_ptr(),
            self.scale.as_ptr(),
            self.interval.as_ptr(),
            self.mix.as_ptr(),
            self.dry.as_ptr(),
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


const MAJOR: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
const MINOR: [i32; 7] = [0, 2, 3, 5, 7, 8, 10];
const STEPS: [i32; 9] = [2, 3, 4, 5, 7, -2, -3, -5, -7];

/// semitones from `midi` to the note `steps` scale degrees away, in the given key and scale
pub fn harmony_semis(midi: i32, key: i32, minor: bool, steps: i32) -> i32 {
    let sc = if minor { MINOR } else { MAJOR };
    let rel = (midi - key).rem_euclid(12);
    let oct = (midi - key).div_euclid(12);
    // nearest scale degree to the played note
    let mut deg = 0usize;
    let mut bestd = 99;
    for (i, s) in sc.iter().enumerate() {
        let d = (rel - s).abs();
        if d < bestd {
            bestd = d;
            deg = i;
        }
    }
    let t = deg as i32 + steps;
    let target = key + 12 * (oct + t.div_euclid(7)) + sc[t.rem_euclid(7) as usize];
    let played = key + 12 * oct + sc[deg];
    target - played
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    pitch: Pitch,
    shift: SyncShift,
    ratio: f64,
    target: f64,
    env: f64,
    since: usize,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, pitch: Pitch::new(fs), shift: SyncShift::new(fs, 30.0), ratio: 1.0, target: 1.0, env: 0.0, since: 0, out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.pitch.reset();
        self.shift.reset();
        self.ratio = 1.0;
        self.target = 1.0;
        self.env = 0.0;
        self.since = 0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().max(1) as f64;
        let key = self.v[KEY].round() as i32;
        let minor = self.v[SCALE] > 0.5;
        let steps = STEPS[(self.v[INTERVAL].round() as usize).min(8)];
        let mix = self.v[MIX] as f64;
        let dry = self.v[DRY] as f64;
        let glide = coeff(self.fs, 15.0);
        let cr = coeff(self.fs, 60.0);
        let refresh = (0.15 * self.fs) as usize;
        let mut lock = 0.0f32;
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch;
            let a = x.abs();
            let was = self.env;
            self.env = a.max(a + cr * (self.env - a));
            self.since += 1;
            if (was < 0.004 && self.env >= 0.004) || self.since > refresh {
                self.pitch.new_note();
                self.since = 0;
            }
            self.pitch.push(x);
            if let Some(f) = self.pitch.freq {
                let midi = (69.0 + 12.0 * (f / 440.0).log2()).round() as i32;
                let s = harmony_semis(midi, key, minor, steps);
                self.target = 2f64.powf(s as f64 / 12.0);
                lock = 1.0;
            }
            self.ratio = self.target + glide * (self.ratio - self.target);
            let h = self.shift.process(x, self.ratio);
            let o = self.out.next();
            for c in chans.iter_mut() {
                let d = c[k] as f64;
                c[k] = ((d * dry + h * mix) * o) as f32;
            }
        }
        meters[0] = lock;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Twin Lead: 3rds (E minor)", values: &[] },
        Preset { name: "Maiden 3rds Below (E minor)", values: &[(INTERVAL, 5.0)] },
        Preset { name: "Lizzy Harmony (A major 3rds)", values: &[(KEY, 9.0), (SCALE, 0.0)] },
        Preset { name: "Power 5ths", values: &[(INTERVAL, 2.0)] },
        Preset { name: "Octave Up Lead", values: &[(INTERVAL, 4.0), (MIX, 0.4)] },
        Preset { name: "Sweet 6ths (G major)", values: &[(KEY, 7.0), (SCALE, 0.0), (INTERVAL, 3.0)] },
        Preset { name: "Harmony Only", values: &[(DRY, 0.0), (MIX, 1.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "HARMONIZER",
        subtitle: ("INTELLIGENT HARMONY IN KEY", "3RDS · 4THS · 5THS · 6THS · OCTAVES"),
        accent: ui::INDIGO,
        size: (W, H),
        items: vec![

            Item::Selector { p: KEY, x: 90.0, y: 165.0, label: "KEY", positions: &["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"], tip: "Key of the song." },
            Item::Selector { p: SCALE, x: 220.0, y: 165.0, label: "SCALE", positions: &["MAJ", "MIN"], tip: "Major or natural minor." },
            Item::Selector { p: INTERVAL, x: 360.0, y: 165.0, label: "INTERVAL", positions: &["3+", "4+", "5+", "6+", "8+", "3-", "4-", "6-", "8-"], tip: "Which harmony: diatonic 3rd, 4th, 5th, 6th or octave, up or down." },
            Item::Lamp { m: 0, x: 500.0, y: 150.0, label: "TRACKING", lo: 0.2, hi: 0.8 },
            Item::Divider { y: 240.0 },
            Item::Knob { p: MIX, x: 150.0, y: 305.0, r: 28.0, label: "HARMONY", tip: "Level of the harmony voice.", active: None },
            Item::Knob { p: DRY, x: 320.0, y: 305.0, r: 28.0, label: "DRY", tip: "Level of your own playing.", active: None },
            Item::Knob { p: OUTPUT, x: 490.0, y: 305.0, r: 28.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Harmonizer", class_id: *b"PqdHarmonizerKey", vendor: "PEQUOD Pedals",
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
    fn diatonic_maths() {
        // A (69) in C major, 3rd up -> C (+3); E (64) in C major, 3rd up -> G (+3); C (60) 3rd up -> E (+4)
        assert_eq!(harmony_semis(69, 0, false, 2), 3);
        assert_eq!(harmony_semis(64, 0, false, 2), 3);
        assert_eq!(harmony_semis(60, 0, false, 2), 4);
        assert_eq!(harmony_semis(60, 0, false, -7), -12);
    }
    #[test]
    fn harmony_in_key() {
        let p = with(defaults(), &[(KEY, 0.0), (SCALE, 0.0), (INTERVAL, 0.0), (DRY, 0.0), (MIX, 1.0)]);
        let y = stereo(&p, &sine(440.0, 0.3, 72000)).0;
        let c = goertzel(&y[48000..], 523.25);
        let a = goertzel(&y[48000..], 440.0);
        println!("C {:.1} dB vs A {:.1} dB", db(c), db(a));
        assert!(db(c) > db(a) + 15.0);
    }

}
