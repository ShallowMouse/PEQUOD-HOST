//! Brighton Rock Delay: Brian May's solo delays. Two separate delays (on stage, two Bel BD-80s at 800 and 1600 ms; earlier, a pair of Echoplexes) feed the left and right amps while the centre amp stays dry, so each phrase comes back twice and plays in canon against itself: harmonies and three-part guitar lines from one player.

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
pub const H: u32 = 490;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Sync { #[name = "Free (ms)"] Free, #[name = "Host tempo"] Host }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Div {
    #[name = "1/8"] E, #[name = "1/4"] Q, #[name = "1/4 dotted"] Qd, #[name = "1/2"] H, #[name = "1/2 dotted"] Hd, #[name = "1 bar"] W, #[name = "2 bars"] W2
}
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Character { #[name = "Echoplex (tape)"] Tape, #[name = "BD-80 (digital)"] Digital }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Layout { #[name = "Three amps (L / dry / R)"] Three, #[name = "Mono"] Mono }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "time1"]
    pub time1: FloatParam,
    #[id = "time2"]
    pub time2: FloatParam,
    #[id = "sync"]
    pub sync: EnumParam<Sync>,
    #[id = "div1"]
    pub div1: EnumParam<Div>,
    #[id = "div2"]
    pub div2: EnumParam<Div>,
    #[id = "fb1"]
    pub fb1: FloatParam,
    #[id = "fb2"]
    pub fb2: FloatParam,
    #[id = "level"]
    pub level: FloatParam,
    #[id = "character"]
    pub character: EnumParam<Character>,
    #[id = "layout"]
    pub layout: EnumParam<Layout>,
    #[id = "dry"]
    pub dry: BoolParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            time1: ms("Delay 1", 800.0, 50.0, 3000.0),
            time2: ms("Delay 2", 1600.0, 50.0, 3000.0),
            sync: EnumParam::new("Sync", Sync::Free),
            div1: EnumParam::new("Delay 1 Note", Div::H),
            div2: EnumParam::new("Delay 2 Note", Div::W),
            fb1: pct("Repeats 1", 0.0),
            fb2: pct("Repeats 2", 0.0),
            level: pct("Delay Level", 0.85),
            character: EnumParam::new("Character", Character::Digital),
            layout: EnumParam::new("Layout", Layout::Three),
            dry: BoolParam::new("Dry", true),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const TIME1: usize = 0;
pub const TIME2: usize = 1;
pub const SYNC: usize = 2;
pub const DIV1: usize = 3;
pub const DIV2: usize = 4;
pub const FB1: usize = 5;
pub const FB2: usize = 6;
pub const LEVEL: usize = 7;
pub const CHAR: usize = 8;
pub const LAYOUT: usize = 9;
pub const DRY: usize = 10;
pub const OUTPUT: usize = 11;
pub const NPARAMS: usize = 12;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.time1.as_ptr(),
            self.time2.as_ptr(),
            self.sync.as_ptr(),
            self.div1.as_ptr(),
            self.div2.as_ptr(),
            self.fb1.as_ptr(),
            self.fb2.as_ptr(),
            self.level.as_ptr(),
            self.character.as_ptr(),
            self.layout.as_ptr(),
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

const MAX_S: f64 = 8.5;

struct Line {
    dl: DelayLine,
    lp: Biquad,
    hp: Biquad,
    wow: f64,
    t: Smooth,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    lines: [Line; 2],
    tempo: f64,
    phase: f64,
    key: i32,
    out: Smooth,
}

pub fn div_beats(d: usize) -> f64 {
    [0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 8.0][d.min(6)]
}

impl Dsp {
    fn times(&self) -> (f64, f64) {
        if self.v[SYNC] > 0.5 {
            let beat = 60.0 / self.tempo.clamp(30.0, 300.0);
            (div_beats(self.v[DIV1].round() as usize) * beat, div_beats(self.v[DIV2].round() as usize) * beat)
        } else {
            (self.v[TIME1] as f64 * 0.001, self.v[TIME2] as f64 * 0.001)
        }
    }
    fn design(&mut self) {
        let fs = self.fs;
        let tape = self.v[CHAR] < 0.5;
        for l in self.lines.iter_mut() {
            l.lp.set_lowpass(fs, if tape { 5500.0 } else { 11000.0 }, 0.7);
            l.hp.set_highpass(fs, if tape { 90.0 } else { 40.0 }, 0.7);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mk = |t: f64| Line { dl: DelayLine::new((MAX_S * fs) as usize), lp: Biquad::identity(), hp: Biquad::identity(), wow: 0.0, t: Smooth::new(fs, 120.0, t * fs) };
        let mut d = Self { fs, v, lines: [mk(0.8), mk(1.6)], tempo: 120.0, phase: 0.0, key: -1, out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        for l in self.lines.iter_mut() {
            l.dl.reset();
            l.lp.reset();
            l.hp.reset();
        }
        let (a, b) = self.times();
        self.lines[0].t.snap(a * self.fs);
        self.lines[1].t.snap(b * self.fs);
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        if v[CHAR] as i32 != self.key {
            self.key = v[CHAR] as i32;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn transport(&mut self, tempo: Option<f64>, _beats: Option<f64>, _playing: bool) {
        if let Some(t) = tempo {
            self.tempo = t;
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().max(1);
        let (ta, tb) = self.times();
        let maxd = MAX_S * self.fs - 8.0;
        self.lines[0].t.set((ta * self.fs).min(maxd));
        self.lines[1].t.set((tb * self.fs).min(maxd));
        let fb = [self.v[FB1] as f64 * 0.95, self.v[FB2] as f64 * 0.95];
        let level = self.v[LEVEL] as f64;
        let tape = self.v[CHAR] < 0.5;
        let three = self.v[LAYOUT] < 0.5;
        let dry = self.v[DRY] > 0.5;
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch as f64;
            self.phase = (self.phase + 0.6 / self.fs).fract();
            let mut w = [0.0; 2];
            for (i, l) in self.lines.iter_mut().enumerate() {
                let mut d = l.t.next();
                if tape {
                    // the two Echoplexes ran at slightly different speeds, and wobble
                    let s = (2.0 * std::f64::consts::PI * (self.phase + 0.37 * i as f64)).sin();
                    l.wow = s * 0.0012 * d;
                    d += l.wow;
                }
                let y = l.dl.read(d);
                let y = l.hp.process(l.lp.process(y));
                let y = if tape { (y * 1.2).tanh() / 1.2 } else { y };
                l.dl.write(x + fb[i] * y);
                w[i] = y;
            }
            let o = self.out.next();
            let dg = if dry { 1.0 } else { 0.0 };
            if three && nch >= 2 {
                // left amp: delay 1; right amp: delay 2; the middle amp (both sides) stays dry
                let l = (chans[0][k] as f64 * dg + w[0] * level) * o;
                let r = (chans[1][k] as f64 * dg + w[1] * level) * o;
                chans[0][k] = l as f32;
                chans[1][k] = r as f32;
                for c in chans.iter_mut().skip(2) {
                    c[k] = (c[k] as f64 * dg * o) as f32;
                }
            } else {
                for c in chans.iter_mut() {
                    c[k] = ((c[k] as f64 * dg + 0.5 * (w[0] + w[1]) * level) * o) as f32;
                }
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Brighton Rock (800 / 1600 ms)", values: &[] },
        Preset { name: "Brighton Rock, Echoplexes (1974)", values: &[(CHAR, 0.0)] },
        Preset { name: "Canon at the Half and Whole Bar (sync)", values: &[(SYNC, 1.0), (DIV1, 3.0), (DIV2, 5.0)] },
        Preset { name: "Fast Canon (1/4 and 1/2)", values: &[(SYNC, 1.0), (DIV1, 1.0), (DIV2, 3.0)] },
        Preset { name: "Three-Part Harmony Builder (with repeats)", values: &[(FB1, 0.35), (FB2, 0.35), (LEVEL, 0.75)] },
        Preset { name: "Wet Amps Only (dry off)", values: &[(DRY, 0.0)] },
        Preset { name: "Mono Double Echo", values: &[(LAYOUT, 1.0), (TIME1, 400.0), (TIME2, 800.0), (LEVEL, 0.6)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "BRIGHTON ROCK",
        subtitle: ("BRIAN MAY'S TWO-DELAY CANON", "800 · DRY · 1600  ·  THREE AMPS"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![

            Item::Knob { p: TIME1, x: 90.0, y: 160.0, r: 38.0, label: "DELAY 1", tip: "Left amp's delay. May's: 800 ms.", active: Some(ui::ActiveWhen { p: SYNC, min: 0.0, max: 0.0 }) },
            Item::Knob { p: TIME2, x: 220.0, y: 160.0, r: 38.0, label: "DELAY 2", tip: "Right amp's delay. May's: 1600 ms, twice delay 1, so a phrase plays in three-part canon.", active: Some(ui::ActiveWhen { p: SYNC, min: 0.0, max: 0.0 }) },
            Item::Switch { p: SYNC, x: 340.0, y: 112.0, labels: ["FREE", "SYNC"], tip: "FREE: times in ms. SYNC: note values at the host tempo." },
            Item::Selector { p: DIV1, x: 340.0, y: 190.0, label: "NOTE 1", positions: &["1/8", "1/4", "1/4.", "1/2", "1/2.", "1", "2"], tip: "Delay 1 as a note value (SYNC). 1 and 2 are whole bars of 4/4." },
            Item::Selector { p: DIV2, x: 450.0, y: 190.0, label: "NOTE 2", positions: &["1/8", "1/4", "1/4.", "1/2", "1/2.", "1", "2"], tip: "Delay 2 as a note value (SYNC)." },
            Item::Knob { p: LEVEL, x: 565.0, y: 160.0, r: 30.0, label: "DELAY LEVEL", tip: "How loud the delayed parts are against the dry one. Near full: each echo is another guitarist.", active: None },
            Item::Divider { y: 262.0 },
            Item::Knob { p: FB1, x: 70.0, y: 375.0, r: 22.0, label: "REPEATS 1", tip: "Repeats on delay 1. May's canon uses single repeats; turn up to stack more voices.", active: None },
            Item::Knob { p: FB2, x: 180.0, y: 375.0, r: 22.0, label: "REPEATS 2", tip: "Repeats on delay 2.", active: None },
            Item::Switch { p: CHAR, x: 110.0, y: 300.0, labels: ["TAPE", "DIGITAL"], tip: "TAPE: a pair of Echoplexes (darker, with wow; how he did it in 1974). DIGITAL: the clean Bel BD-80s he used later." },
            Item::Switch { p: LAYOUT, x: 320.0, y: 300.0, labels: ["L/C/R", "MONO"], tip: "L/C/R: delay 1 left, the dry guitar in both, delay 2 right (his three-amp rig). MONO: everything together." },
            Item::Switch { p: DRY, x: 520.0, y: 300.0, labels: ["OFF", "DRY"], tip: "The middle amp: the dry guitar. Off for wet-only use on a send." },
            Item::Knob { p: OUTPUT, x: 520.0, y: 375.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Brighton Rock Delay", class_id: *b"BMyBrightonDlyPQ", vendor: "PEQUOD Brian May",
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
    fn go2(p: &[f32], x: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        let r = input.pop().unwrap();
        (input.pop().unwrap(), r)
    }

    #[test]
    fn echoes_land_left_and_right() {
        let p = defaults();
        let mut x = vec![0.0f32; 48000 * 2];
        x[100] = 1.0;
        let (l, r) = go2(&p, &x);
        let pk = |y: &[f32]| y.iter().enumerate().skip(1000).max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap()).unwrap().0 as f64 / 48.0;
        let (a, b) = (pk(&l), pk(&r));
        println!("left echo at {a:.1} ms, right echo at {b:.1} ms (impulse at 2.1 ms)");
        assert!((a - 802.1).abs() < 3.0 && (b - 1602.1).abs() < 3.0);
    }
    #[test]
    fn sync_follows_tempo() {
        let mut d = Dsp::new(48000.0, 2);
        d.set_params(&with(defaults(), &[(SYNC, 1.0), (DIV1, 3.0), (DIV2, 5.0)]));
        d.transport(Some(120.0), None, true);
        let (a, b) = d.times();
        println!("120 bpm: 1/2 = {:.0} ms, 1 bar = {:.0} ms", a * 1000.0, b * 1000.0);
        assert!((a - 1.0).abs() < 1e-9 && (b - 2.0).abs() < 1e-9);
    }

}
