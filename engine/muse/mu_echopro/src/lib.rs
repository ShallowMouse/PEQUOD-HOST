//! Echo Pro: the Line 6 Echo Pro rack delay that ran in parallel with Matt Bellamy's guitar on the Absolution tours,
//! MIDI-switched from his Rocktron controller: its family of echo models (tube and tape echoes, an analog bucket
//! brigade, clean and modulated digital, ping-pong, a ducking "dynamic" delay and reverse).

#![allow(clippy::excessive_precision)]
use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
#[allow(unused_imports)]
use vintage::dsp::*;
use vintage::nih_plug_egui::EguiState;
#[allow(unused_imports)]
use vintage::params::*;
#[allow(unused_imports)]
use vintage::ui::{self, ActiveWhen, Item, MeterKind, Panel, Preset};
#[allow(unused_imports)]
use vintage::{Midi, VintageDsp, VintageParams};

pub const W: u32 = 640;
pub const H: u32 = 420;

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum Model {
    #[name = "Tube Echo"]
    Tube,
    #[name = "Tape Echo"]
    Tape,
    #[name = "Analog Echo"]
    Analog,
    #[name = "Digital"]
    Digital,
    #[name = "Mod Digital"]
    Mod,
    #[name = "Ping Pong"]
    PingPong,
    #[name = "Dynamic"]
    Dynamic,
    #[name = "Reverse"]
    Reverse,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "model"]
    pub model: EnumParam<Model>,
    #[id = "time"]
    pub time: FloatParam,
    #[id = "feedback"]
    pub feedback: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "mod"]
    pub modulation: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            model: EnumParam::new("Model", Model::Digital),
            time: FloatParam::new("Time", 420.0, FloatRange::Skewed { min: 20.0, max: 2000.0, factor: FloatRange::skew_factor(-1.2) }).with_unit(" ms").with_value_to_string(formatters::v2s_f32_rounded(0)),
            feedback: pct("Feedback", 0.35),
            mix: pct("Mix", 0.3),
            tone: pct("Tone", 0.5),
            modulation: pct("Modulation", 0.2),
            output: db("Output", 0.0, -12.0, 6.0),
        }
    }
}

pub const MODEL: usize = 0;
pub const TIME: usize = 1;
pub const FEEDBACK: usize = 2;
pub const MIX: usize = 3;
pub const TONE: usize = 4;
pub const MODULATION: usize = 5;
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
        vec![self.model.as_ptr(), self.time.as_ptr(), self.feedback.as_ptr(), self.mix.as_ptr(), self.tone.as_ptr(), self.modulation.as_ptr(), self.output.as_ptr()]
    }
}

/// Default plain values in linked order (for tests).
#[allow(dead_code)]
pub fn defaults() -> Vec<f32> {
    let p = P::default();
    p.linked().iter().map(|q| unsafe { q.preview_plain(q.default_normalized_value()) }).collect()
}

// -------------------------------------------------------------------- dsp --

struct Side {
    dl: DelayLine,
    lp: Biquad,
    hp: Biquad,
    fb: f64,
    rev: Vec<f64>,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    s: [Side; 2],
    time: Smooth,
    wow: Wander,
    ph: f64,
    env: f64,
    rpos: usize,
    out: Smooth,
    key: Vec<i32>,
}

impl Dsp {
    fn side(fs: f64) -> Side {
        Side { dl: DelayLine::new((fs * 2.2) as usize), lp: Biquad::identity(), hp: Biquad::identity(), fb: 0.0, rev: vec![0.0; (fs * 2.1) as usize] }
    }
    fn design(&mut self) {
        let fs = self.fs;
        let t = self.v[TONE] as f64;
        // each model's repeat bandwidth: tone moves it around the model's own centre
        let (f, hp) = match self.v[MODEL].round() as i32 {
            0 => (3500.0, 90.0),
            1 => (4500.0, 120.0),
            2 => (2400.0, 80.0),
            6 | 5 | 3 | 4 => (11000.0, 40.0),
            _ => (9000.0, 60.0),
        };
        let f = (f * 2f64.powf((t - 0.5) * 2.5)).min(fs * 0.45);
        for s in self.s.iter_mut() {
            s.lp.set_lowpass(fs, f, 0.6);
            s.hp.set_highpass(fs, hp, 0.6);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, s: [Self::side(fs), Self::side(fs)], time: Smooth::new(fs, 120.0, 420.0), wow: Wander::new(fs, 0.6, 11), ph: 0.0, env: 0.0, rpos: 0, out: Smooth::new(fs, 20.0, 1.0), key: vec![] };
        d.design();
        d
    }
    fn reset(&mut self) {
        let fs = self.fs;
        self.s = [Self::side(fs), Self::side(fs)];
        self.design();
        self.time.snap(self.v[TIME] as f64);
        self.env = 0.0;
        self.rpos = 0;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = vec![v[MODEL] as i32, (v[TONE] * 100.0) as i32];
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.time.set(v[TIME] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(2);
        let fs = self.fs;
        let model = self.v[MODEL].round() as i32;
        let fbk = self.v[FEEDBACK] as f64 * 0.98;
        let mix = self.v[MIX] as f64;
        let md = self.v[MODULATION] as f64;
        let mut lvl = 0.0f64;
        for k in 0..n {
            let t_ms = self.time.next();
            let mut d = t_ms * 0.001 * fs;
            // modulation: tape wow and flutter, a BBD's clock wobble, the mod-digital chorus
            let w = self.wow.next();
            self.ph = (self.ph + 0.9 / fs).fract();
            let lfo = (2.0 * std::f64::consts::PI * self.ph).sin();
            d *= match model {
                1 => 1.0 + md * (0.004 * w + 0.0015 * (self.ph * 9.0 * 2.0 * std::f64::consts::PI).sin()),
                2 => 1.0 + md * 0.006 * lfo,
                4 => 1.0 + md * 0.012 * lfo,
                0 => 1.0 + md * 0.002 * w,
                _ => 1.0,
            };
            let xin: [f64; 2] = [chans[0][k] as f64, chans[nch - 1][k] as f64];
            let mono = 0.5 * (xin[0] + xin[1]);
            // the dynamic model ducks its repeats while you play
            let a = mono.abs();
            self.env = a + if a > self.env { 0.99 } else { 0.9995 } * (self.env - a);
            let duck = if model == 6 { 1.0 / (1.0 + 12.0 * self.env) } else { 1.0 };
            let reverse_len = (t_ms * 0.001 * fs) as usize;
            let mut wet = [0.0f64; 2];
            for c in 0..2 {
                let s = &mut self.s[c];
                let r = s.dl.read(d.max(1.0));
                let r = s.hp.process(s.lp.process(r));
                let r = match model {
                    0 => tube(r * 1.4, 0.15) / 1.4,
                    1 => tape_curve(r * 1.3) / 1.3,
                    2 => (r * 1.2).tanh() / 1.2,
                    _ => r,
                };
                s.fb = r;
                wet[c] = r;
            }
            // feed the lines: ping-pong crosses the channels, everything else stays on its side
            for c in 0..2 {
                let into = if model == 5 { if c == 0 { mono } else { 0.0 } } else { xin[c] };
                let fbs = if model == 5 { self.s[1 - c].fb } else { self.s[c].fb };
                self.s[c].dl.write(into + fbk * fbs);
            }
            if model == 7 && reverse_len > 8 {
                // reverse: each window of input played backwards
                for c in 0..2 {
                    let s = &mut self.s[c];
                    let m = reverse_len.min(s.rev.len());
                    let i = self.rpos % m;
                    let back = s.rev[m - 1 - i];
                    s.rev[i] = xin[c] + fbk * back * 0.6;
                    let g = (std::f64::consts::PI * i as f64 / m as f64).sin();
                    wet[c] = back * g;
                }
                self.rpos = (self.rpos + 1) % reverse_len.max(1);
            }
            let o = self.out.next();
            for c in 0..nch {
                let y = xin[c] + mix * duck * wet[c];
                lvl = lvl.max(wet[c].abs());
                chans[c][k] = (y * o) as f32;
            }
        }
        meters[0] = lvl as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Parallel Lead Echo", values: &[] },
        Preset { name: "Tape Slapback", values: &[(MODEL, 1.0), (TIME, 120.0), (FEEDBACK, 0.1), (MIX, 0.35)] },
        Preset { name: "Analog Dotted Eighths", values: &[(MODEL, 2.0), (TIME, 375.0), (FEEDBACK, 0.45), (MIX, 0.3)] },
        Preset { name: "Wide Ping Pong", values: &[(MODEL, 5.0), (TIME, 330.0), (FEEDBACK, 0.5), (MIX, 0.35)] },
        Preset { name: "Ducking Solo Delay", values: &[(MODEL, 6.0), (TIME, 480.0), (FEEDBACK, 0.4), (MIX, 0.5)] },
        Preset { name: "Reverse Swell", values: &[(MODEL, 7.0), (TIME, 600.0), (FEEDBACK, 0.2), (MIX, 0.6)] },
        Preset { name: "Tube Echo Wash", values: &[(MODEL, 0.0), (TIME, 520.0), (FEEDBACK, 0.7), (MIX, 0.35), (MODULATION, 0.4)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "ECHO PRO",
        subtitle: ("AFTER THE LINE 6 ECHO PRO", "RACK DELAY · 8 ECHO MODELS"),
        accent: ui::PINE,
        size: (W, H),
        items: vec![
            Item::Selector { p: MODEL, x: 100.0, y: 160.0, label: "MODEL", positions: &["TUBE", "TAPE", "ANALOG", "DIGITAL", "MOD", "PING", "DYNAMIC", "REVERSE"], tip: "The echo model." },
            Item::Knob { p: TIME, x: 250.0, y: 160.0, r: 40.0, label: "TIME", tip: "Delay time.", active: None },
            Item::Knob { p: FEEDBACK, x: 380.0, y: 160.0, r: 32.0, label: "FEEDBACK", tip: "How many repeats.", active: None },
            Item::Knob { p: MIX, x: 500.0, y: 160.0, r: 32.0, label: "MIX", tip: "Repeats against the dry guitar (it ran in parallel, so the dry is always full level).", active: None },
            Item::Divider { y: 270.0 },
            Item::Knob { p: TONE, x: 180.0, y: 340.0, r: 24.0, label: "TONE", tip: "Darker or brighter repeats around each model's own character.", active: None },
            Item::Knob { p: MODULATION, x: 320.0, y: 340.0, r: 24.0, label: "MODULATION", tip: "Wow and flutter (tape), clock wobble (analog), chorus (mod digital).", active: None },
            Item::Knob { p: OUTPUT, x: 460.0, y: 340.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Echo Pro", class_id: *b"MuseEchoProPQD01", vendor: "PEQUOD Muse",
    categories: [Fx, Delay]
);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_model_runs() {
        for m in 0..8 {
            let mut d = Dsp::new(48000.0, 2);
            let mut v = defaults();
            v[MODEL] = m as f32;
            v[FEEDBACK] = 0.95;
            d.set_params(&v);
            let mut l: Vec<f32> = (0..96000).map(|k| if k < 4800 { (0.5 * (k as f64 * 0.05).sin()) as f32 } else { 0.0 }).collect();
            let mut r = l.clone();
            let mut mt = [0.0f32];
            d.process(&mut [&mut l[..], &mut r[..]], &mut mt);
            assert!(l.iter().chain(r.iter()).all(|x| x.is_finite() && x.abs() < 8.0), "model {m}");
            assert!(l[60000..].iter().any(|x| x.abs() > 1e-4), "model {m} repeats");
        }
    }
}
