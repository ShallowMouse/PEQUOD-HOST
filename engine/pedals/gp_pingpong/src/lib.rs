//! Ping-Pong Delay: a stereo delay whose repeats bounce between left and right: the first echo on the left, the next on the right, and so on. Free time or locked to the host tempo, with a darkening TONE in the loop and a little tape-style MOD.

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


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Sync { #[name = "Free"] Free, #[name = "1/4"] Q, #[name = "Dotted 1/8"] De, #[name = "1/8"] E, #[name = "1/4 triplet"] Qt, #[name = "1/16"] S }
pub const SYNC_BEATS: [f64; 6] = [0.0, 1.0, 0.75, 0.5, 2.0 / 3.0, 0.25];


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "time"]
    pub time: FloatParam,
    #[id = "sync"]
    pub sync: EnumParam<Sync>,
    #[id = "feedback"]
    pub feedback: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "mod_amt"]
    pub mod_amt: FloatParam,
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
            time: ms("Time", 375.0, 20.0, 1500.0),
            sync: EnumParam::new("Sync", Sync::Free),
            feedback: pct("Feedback", 0.45),
            tone: hz("Tone", 4500.0, 800.0, 12000.0),
            mod_amt: pct("Mod", 0.15),
            mix: pct("Mix", 0.35),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const TIME: usize = 0;
pub const SYNC: usize = 1;
pub const FEEDBACK: usize = 2;
pub const TONE: usize = 3;
pub const MOD: usize = 4;
pub const MIX: usize = 5;
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
            self.time.as_ptr(),
            self.sync.as_ptr(),
            self.feedback.as_ptr(),
            self.tone.as_ptr(),
            self.mod_amt.as_ptr(),
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

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    line: [DelayLine; 2],
    lp: [OnePole; 2],
    w: Wander,
    tempo: Option<f64>,
    time: Smooth,
    out: Smooth,
}

impl Dsp {
    fn target(&self) -> f64 {
        let s = (self.v[SYNC].round() as usize).min(5);
        match (s, self.tempo) {
            (s, Some(t)) if s > 0 => SYNC_BEATS[s] * 60.0 / t * self.fs,
            _ => self.v[TIME] as f64 * 0.001 * self.fs,
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mk = || DelayLine::new((3.1 * fs) as usize);
        let mut d = Self { fs, v, line: [mk(), mk()], lp: [OnePole::default(); 2], w: Wander::new(fs, 0.6, 21), tempo: None, time: Smooth::new(fs, 120.0, 0.375 * fs), out: Smooth::new(fs, 20.0, 1.0) };
        let t = d.target();
        d.time.snap(t);
        d
    }
    fn reset(&mut self) {
        self.line.iter_mut().for_each(|l| l.reset());
        self.lp = [OnePole::default(); 2];
        let t = self.target();
        self.time.snap(t);
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let fs = self.fs;
        let f = v[TONE] as f64;
        self.lp.iter_mut().for_each(|p| p.set_lowpass(fs, f));
        let t = self.target();
        self.time.set(t);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn transport(&mut self, tempo: Option<f64>, _beats: Option<f64>, _playing: bool) {
        self.tempo = tempo;
        let t = self.target();
        self.time.set(t);
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fb = self.v[FEEDBACK] as f64 * 1.05;
        let md = self.v[MOD] as f64 * 0.003;
        let mix = self.v[MIX] as f64;
        let nch = chans.len();
        for k in 0..n {
            let d = self.time.next() * (1.0 + md * self.w.next());
            let ol = self.lp[0].lp(self.line[0].read(d));
            let or = self.lp[1].lp(self.line[1].read(d));
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            self.line[0].write(x + (fb * or).tanh());
            self.line[1].write(fb * ol);
            let o = self.out.next();
            if nch >= 2 {
                chans[0][k] = ((chans[0][k] as f64 + mix * ol * 2.0) * o) as f32;
                chans[1][k] = ((chans[1][k] as f64 + mix * or * 2.0) * o) as f32;
            } else if nch == 1 {
                chans[0][k] = ((chans[0][k] as f64 + mix * (ol + or)) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Wide Bounce", values: &[] },
        Preset { name: "Dotted Eighth (synced)", values: &[(SYNC, 2.0), (FEEDBACK, 0.4)] },
        Preset { name: "Quarter Notes (synced)", values: &[(SYNC, 1.0)] },
        Preset { name: "Short Stereo Slap", values: &[(TIME, 110.0), (FEEDBACK, 0.15), (MIX, 0.3)] },
        Preset { name: "Dark Long Trails", values: &[(TIME, 650.0), (FEEDBACK, 0.65), (TONE, 1800.0), (MOD, 0.3)] },
        Preset { name: "Ambient Wash", values: &[(TIME, 900.0), (FEEDBACK, 0.75), (TONE, 3000.0), (MIX, 0.45), (MOD, 0.4)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "PING-PONG DELAY",
        subtitle: ("STEREO BOUNCING ECHOES", "FREE OR TEMPO-SYNCED"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![
            Item::Knob { p: TIME, x: 60.0, y: 190.0, r: 30.0, label: "TIME", tip: "Delay time (when not synced).", active: None },
            Item::Selector { p: SYNC, x: 146.7, y: 190.0, label: "SYNC", positions: &["FREE", "1/4", "1/8.", "1/8", "1/4T", "1/16"], tip: "Lock to the host tempo." },
            Item::Knob { p: FEEDBACK, x: 233.3, y: 190.0, r: 30.0, label: "FEEDBACK", tip: "Number of repeats.", active: None },
            Item::Knob { p: TONE, x: 320.0, y: 190.0, r: 30.0, label: "TONE", tip: "Each repeat gets darker.", active: None },
            Item::Knob { p: MOD, x: 406.7, y: 190.0, r: 30.0, label: "MOD", tip: "Tape-style wobble.", active: None },
            Item::Knob { p: MIX, x: 493.3, y: 190.0, r: 30.0, label: "MIX", tip: "Echo level.", active: None },
            Item::Knob { p: OUTPUT, x: 580.0, y: 190.0, r: 30.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Ping-Pong Delay", class_id: *b"PqdPingPongDelay", vendor: "PEQUOD Pedals",
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
    fn bounces() {
        let p = with(defaults(), &[(TIME, 200.0), (MOD, 0.0), (MIX, 1.0), (TONE, 12000.0)]);
        let (l, r) = impulse(&p, 48000);
        let t = 9600 + 100;
        let (kl, _) = peak_after(&l, 1000, 14000);
        let (kr, _) = peak_after(&r, 1000, 24000);
        println!("first echo L at {kl}, first echo R at {kr} (expect {t} / {})", t + 9600);
        assert!((kl as i64 - t as i64).abs() < 30 && (kr as i64 - (t + 9600) as i64).abs() < 30);
    }

}
