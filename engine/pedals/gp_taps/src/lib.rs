//! Rhythm Taps: an original PEQUOD multi-tap delay: four echoes placed in a rhythmic PATTERN across one BEAT (straight, dotted, triplet, gallop, swing and more), panned alternately left and right, so a single note becomes a groove. Free time or locked to the host tempo.

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
pub const H: u32 = 440;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Pattern {
    #[name = "Straight 16ths"] Straight, #[name = "Dotted"] Dotted, #[name = "Triplets"] Triplets, #[name = "Gallop"] Gallop,
    #[name = "Swing"] Swing, #[name = "Reverse Gallop"] RevGallop, #[name = "Clave"] Clave, #[name = "Accelerate"] Accel,
}
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Sync { #[name = "Free"] Free, #[name = "Synced (1 beat)"] Beat, #[name = "Synced (2 beats)"] Two }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "beat"]
    pub beat: FloatParam,
    #[id = "sync"]
    pub sync: EnumParam<Sync>,
    #[id = "pattern"]
    pub pattern: EnumParam<Pattern>,
    #[id = "feedback"]
    pub feedback: FloatParam,
    #[id = "spread"]
    pub spread: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
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
            beat: ms("Beat", 500.0, 100.0, 2000.0),
            sync: EnumParam::new("Sync", Sync::Free),
            pattern: EnumParam::new("Pattern", Pattern::Straight),
            feedback: pct("Feedback", 0.25),
            spread: pct("Spread", 0.7),
            tone: hz("Tone", 5000.0, 800.0, 12000.0),
            mix: pct("Mix", 0.4),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const BEAT: usize = 0;
pub const SYNC: usize = 1;
pub const PATTERN: usize = 2;
pub const FEEDBACK: usize = 3;
pub const SPREAD: usize = 4;
pub const TONE: usize = 5;
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
            self.beat.as_ptr(),
            self.sync.as_ptr(),
            self.pattern.as_ptr(),
            self.feedback.as_ptr(),
            self.spread.as_ptr(),
            self.tone.as_ptr(),
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

/// tap positions (fraction of a beat) and levels
pub const TAPS: [[(f64, f64); 4]; 8] = [
    [(0.25, 0.8), (0.5, 0.65), (0.75, 0.5), (1.0, 0.4)],
    [(0.375, 0.8), (0.75, 0.6), (1.125, 0.45), (1.5, 0.35)],
    [(1.0 / 3.0, 0.8), (2.0 / 3.0, 0.65), (1.0, 0.5), (4.0 / 3.0, 0.4)],
    [(0.5, 0.8), (0.625, 0.6), (0.75, 0.6), (1.0, 0.45)],
    [(2.0 / 3.0, 0.8), (1.0, 0.6), (5.0 / 3.0, 0.5), (2.0, 0.4)],
    [(0.25, 0.7), (0.375, 0.7), (0.5, 0.55), (1.0, 0.45)],
    [(0.375, 0.8), (0.75, 0.7), (1.25, 0.55), (1.5, 0.45)],
    [(0.5, 0.8), (0.75, 0.6), (0.875, 0.5), (0.9375, 0.4)],
];

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    line: DelayLine,
    lp: OnePole,
    tempo: Option<f64>,
    beat: Smooth,
    out: Smooth,
}

impl Dsp {
    fn target(&self) -> f64 {
        let s = self.v[SYNC].round() as usize;
        match (s, self.tempo) {
            (s, Some(t)) if s > 0 => s as f64 * 60.0 / t * self.fs,
            _ => self.v[BEAT] as f64 * 0.001 * self.fs,
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, line: DelayLine::new((8.5 * fs) as usize), lp: OnePole::default(), tempo: None, beat: Smooth::new(fs, 150.0, 0.5 * fs), out: Smooth::new(fs, 20.0, 1.0) };
        let t = d.target();
        d.beat.snap(t);
        d
    }
    fn reset(&mut self) {
        self.line.reset();
        self.lp = OnePole::default();
        let fs = self.fs;
        self.lp.set_lowpass(fs, self.v[TONE] as f64);
        let t = self.target();
        self.beat.snap(t);
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let fs = self.fs;
        self.lp.set_lowpass(fs, v[TONE] as f64);
        let t = self.target();
        self.beat.set(t);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn transport(&mut self, tempo: Option<f64>, _beats: Option<f64>, _playing: bool) {
        self.tempo = tempo;
        let t = self.target();
        self.beat.set(t);
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let pat = TAPS[(self.v[PATTERN].round() as usize).min(7)];
        let fb = self.v[FEEDBACK] as f64 * 0.95;
        let spread = self.v[SPREAD] as f64;
        let mix = self.v[MIX] as f64;
        let nch = chans.len();
        let maxd = self.line.max_delay() - 2.0;
        for k in 0..n {
            let b = self.beat.next();
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            let (mut l, mut r) = (0.0, 0.0);
            let mut lastv = 0.0;
            for (i, (pos, lv)) in pat.iter().enumerate() {
                let y = self.line.read((pos * b).clamp(1.0, maxd)) * lv;
                let p = if i % 2 == 0 { -spread } else { spread };
                let th = (p + 1.0) * std::f64::consts::FRAC_PI_4;
                l += y * th.cos() * std::f64::consts::SQRT_2;
                r += y * th.sin() * std::f64::consts::SQRT_2;
                lastv = y / lv;
            }
            self.line.write(x + self.lp.lp(lastv * fb));
            let o = self.out.next();
            if nch >= 2 {
                chans[0][k] = ((chans[0][k] as f64 + mix * l) * o) as f32;
                chans[1][k] = ((chans[1][k] as f64 + mix * r) * o) as f32;
            } else if nch == 1 {
                chans[0][k] = ((chans[0][k] as f64 + mix * 0.5 * (l + r)) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Straight Sixteenths", values: &[] },
        Preset { name: "Dotted Groove (synced)", values: &[(PATTERN, 1.0), (SYNC, 1.0)] },
        Preset { name: "Triplet Cascade", values: &[(PATTERN, 2.0), (FEEDBACK, 0.35)] },
        Preset { name: "Gallop", values: &[(PATTERN, 3.0), (BEAT, 450.0)] },
        Preset { name: "Swing Echo", values: &[(PATTERN, 4.0), (SYNC, 1.0), (MIX, 0.35)] },
        Preset { name: "Clave Pulse", values: &[(PATTERN, 6.0), (SYNC, 2.0)] },
        Preset { name: "Accelerating Taps", values: &[(PATTERN, 7.0), (BEAT, 800.0), (FEEDBACK, 0.4)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "RHYTHM TAPS",
        subtitle: ("PEQUOD ORIGINAL · MULTI-TAP DELAY", "FOUR TAPS IN A RHYTHM · STEREO"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![

            Item::Selector { p: PATTERN, x: 100.0, y: 165.0, label: "PATTERN", positions: &["16", "DOT", "TRP", "GAL", "SW", "RG", "CLV", "ACC"], tip: "Where the four taps fall across the beat." },
            Item::Knob { p: BEAT, x: 260.0, y: 165.0, r: 36.0, label: "BEAT", tip: "Length of one beat (when not synced).", active: None },
            Item::Selector { p: SYNC, x: 400.0, y: 165.0, label: "SYNC", positions: &["FREE", "1 BEAT", "2 BEATS"], tip: "Lock the beat to the host tempo." },
            Item::Knob { p: MIX, x: 540.0, y: 165.0, r: 36.0, label: "MIX", tip: "Echo level.", active: None },
            Item::Divider { y: 240.0 },
            Item::Knob { p: FEEDBACK, x: 90.0, y: 305.0, r: 26.0, label: "FEEDBACK", tip: "Repeats the whole pattern.", active: None },
            Item::Knob { p: SPREAD, x: 243.3, y: 305.0, r: 26.0, label: "SPREAD", tip: "Taps alternate left and right.", active: None },
            Item::Knob { p: TONE, x: 396.7, y: 305.0, r: 26.0, label: "TONE", tip: "Darkens the repeats.", active: None },
            Item::Knob { p: OUTPUT, x: 550.0, y: 305.0, r: 26.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Rhythm Taps", class_id: *b"PqdRhythmTapsDly", vendor: "PEQUOD Pedals",
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

    fn impulse(p: &[f32], n: usize) -> (Vec<f32>, Vec<f32>) {
        let mut x = vec![0.0f32; n];
        x[100] = 0.5;
        let mut input = vec![x.clone(), x];
        run(p, &mut input);
        let r = input.pop().unwrap();
        (input.pop().unwrap(), r)
    }
    fn peak_after(y: &[f32], from: usize, to: usize) -> (usize, f32) {
        let mut best = (from, 0.0f32);
        for k in from..to.min(y.len()) {
            if y[k].abs() > best.1 {
                best = (k, y[k].abs());
            }
        }
        best
    }

    #[test]
    fn taps_fall_on_the_pattern() {
        let p = with(defaults(), &[(BEAT, 400.0), (FEEDBACK, 0.0), (MIX, 1.0), (SPREAD, 0.0)]);
        let (l, _) = impulse(&p, 48000);
        for (pos, _) in TAPS[0].iter() {
            let t = (pos * 0.4 * 48000.0) as usize + 100;
            let (k, v) = peak_after(&l, t - 200, t + 200);
            println!("tap at {k} (expect {t}) level {v:.3}");
            assert!((k as i64 - t as i64).abs() < 10 && v > 0.05);
        }
    }

}
