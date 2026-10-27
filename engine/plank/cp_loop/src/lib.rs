//! Tape Loop: a loop of tape between record and play heads, long enough to stretch across the studio, as Plank used them. Repeats wear with each pass; with the erase head off, every pass is recorded over the last.

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
pub enum Erase { #[name = "Off"] Off, #[name = "On"] On }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "length"]
    pub length: FloatParam,
    #[id = "speed"]
    pub speed: FloatParam,
    #[id = "repeats"]
    pub repeats: FloatParam,
    #[id = "erase"]
    pub erase: EnumParam<Erase>,
    #[id = "wear"]
    pub wear: FloatParam,
    #[id = "wow"]
    pub wow: FloatParam,
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
            length: secs("Loop Length", 4.0, 0.25, 20.0),
            speed: FloatParam::new("Speed", 1.0, FloatRange::Skewed { min: 0.5, max: 2.0, factor: FloatRange::skew_factor(0.0) }).with_step_size(0.001).with_unit("x"),
            repeats: pct("Repeats", 0.7),
            erase: EnumParam::new("Erase Head", Erase::On),
            wear: pct("Wear", 0.3),
            wow: pct("Wow", 0.2),
            mix: pct("Mix", 0.5),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const LENGTH: usize = 0;
pub const SPEED: usize = 1;
pub const REPEATS: usize = 2;
pub const ERASE: usize = 3;
pub const WEAR: usize = 4;
pub const WOW: usize = 5;
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
            self.length.as_ptr(),
            self.speed.as_ptr(),
            self.repeats.as_ptr(),
            self.erase.as_ptr(),
            self.wear.as_ptr(),
            self.wow.as_ptr(),
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
    line: [DelayLine; 2],
    hf: [OnePole; 2],
    lf: [OnePole; 2],
    wander: Wander,
    lfo: Lfo,
    length: f64,
    speed: Smooth,
    delay: f64,
    repeats: f64,
    erase: bool,
    wear: f64,
    wow: f64,
    mix: Smooth,
    out: Smooth,
    snap: bool,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        Self {
            snap: true,
            fs,
            line: [DelayLine::new((fs * 41.0) as usize), DelayLine::new((fs * 41.0) as usize)],
            hf: [OnePole::default(); 2],
            lf: [OnePole::default(); 2],
            wander: Wander::new(fs, 0.7, 13),
            lfo: Lfo::default(),
            length: 4.0,
            speed: Smooth::new(fs, 250.0, 1.0),
            delay: 4.0 * fs,
            repeats: 0.7,
            erase: true,
            wear: 0.3,
            wow: 0.2,
            mix: Smooth::new(fs, 20.0, 0.5),
            out: Smooth::new(fs, 20.0, 1.0),
        }
    }
    fn reset(&mut self) {
        self.line.iter_mut().for_each(|l| l.reset());
        self.hf.iter_mut().for_each(|p| p.reset());
        self.lf.iter_mut().for_each(|p| p.reset());
        self.snap = true;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.length = v[LENGTH] as f64;
        self.speed.set(v[SPEED] as f64);
        self.repeats = v[REPEATS] as f64;
        self.erase = v[ERASE] > 0.5;
        self.wear = v[WEAR] as f64;
        // each pass loses top end; more wear, more loss
        let hf = 16000.0 * 2f64.powf(-3.0 * self.wear);
        for p in &mut self.hf {
            p.set_lowpass(self.fs, hf);
        }
        for p in &mut self.lf {
            p.set_lowpass(self.fs, 40.0);
        }
        self.wow = v[WOW] as f64;
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let mut peak = 0.0f64;
        // without the erase head every pass is recorded over the last (sound on sound)
        let fb = if self.erase { self.repeats * 0.95 } else { 0.97 + 0.02 * self.repeats };
        let sat = 1.0 + 3.0 * self.wear;
        for k in 0..n {
            let sp = self.speed.next();
            // the loop's delay is its length over the speed; glide like a capstan
            let target = (self.length / sp * self.fs).min(self.line[0].max_delay() - 4.0);
            if self.snap {
                self.delay = target;
                self.snap = false;
            }
            self.delay += (target - self.delay) * 0.00005;
            let w = 1.0 + self.wow * 0.004 * (0.7 * self.lfo.next(self.fs, 0.5 * sp) + 0.3 * self.wander.next());
            let d = self.delay * w;
            let mx = self.mix.next();
            let o = self.out.next();
            for c in 0..nch.min(2) {
                let x = chans[c][k] as f64;
                let y = self.line[c].read(d);
                let rec = x + fb * y;
                let rec = (rec * sat).tanh() / sat;
                let rec = self.hf[c].lp(rec);
                let rec = rec - self.lf[c].lp(rec) * 0.8;
                self.line[c].write(rec);
                let z = (x * (1.0 - mx) + y * mx) * o;
                peak = peak.max(z.abs());
                chans[c][k] = z as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Studio-Length Loop", values: &[(LENGTH, 8.0), (REPEATS, 0.75), (MIX, 0.5)] },
        Preset { name: "Sound on Sound Build", values: &[(LENGTH, 4.0), (ERASE, 0.0), (REPEATS, 0.5), (MIX, 0.6)] },
        Preset { name: "Short Rhythmic Loop", values: &[(LENGTH, 0.5), (REPEATS, 0.6), (WEAR, 0.4), (MIX, 0.5)] },
        Preset { name: "Half-Speed Drone", values: &[(LENGTH, 6.0), (SPEED, 0.5), (REPEATS, 0.85), (MIX, 0.6)] },
        Preset { name: "Worn Loop", values: &[(LENGTH, 3.0), (REPEATS, 0.8), (WEAR, 0.8), (WOW, 0.6), (MIX, 0.5)] },
        Preset { name: "Double-Speed Chirp", values: &[(LENGTH, 2.0), (SPEED, 2.0), (REPEATS, 0.6), (MIX, 0.5)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "TAPE LOOP",
        subtitle: ("A LOOP ACROSS THE STUDIO", "RECORD · PLAY · ERASE"),
        accent: ui::WALNUT,
        size: (W, H),
        items: vec![

            Item::Knob { p: LENGTH, x: 100.0, y: 160.0, r: 46.0, label: "LOOP LENGTH", tip: "Length of the tape loop at normal speed, 0.25 to 20 s.", active: None },
            Item::Knob { p: SPEED, x: 250.0, y: 160.0, r: 40.0, label: "SPEED", tip: "Tape speed. Changing it bends the pitch of what's on the loop, as a real machine does.", active: None },
            Item::Knob { p: REPEATS, x: 390.0, y: 160.0, r: 40.0, label: "REPEATS", tip: "How much of each pass is re-recorded.", active: None },
            Item::Switch { p: ERASE, x: 530.0, y: 160.0, labels: ["OFF", "ERASE"], tip: "Erase head off: every pass is recorded on top of the last (sound on sound) and the loop keeps building." },
            Item::Divider { y: 264.0 },
            Item::Knob { p: WEAR, x: 100.0, y: 330.0, r: 22.0, label: "WEAR", tip: "Tape saturation and treble loss on every pass.", active: None },
            Item::Knob { p: WOW, x: 220.0, y: 330.0, r: 22.0, label: "WOW", tip: "Speed wobble of a long loop.", active: None },
            Item::Knob { p: MIX, x: 380.0, y: 330.0, r: 22.0, label: "MIX", tip: "Input vs the loop.", active: None },
            Item::Knob { p: OUTPUT, x: 540.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Tape Loop", class_id: *b"CPTapeLoopPQD001", vendor: "PEQUOD Plank",
    categories: [Fx, Delay]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
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

    #[test]
    fn loop_returns_after_its_length() {
        let p = with(defaults(), &[(LENGTH, 1.0), (MIX, 1.0), (WOW, 0.0), (WEAR, 0.0), (REPEATS, 0.0)]);
        let mut x = vec![0.0f32; 72000];
        for v in x.iter_mut().take(480) {
            *v = 0.5;
        }
        let mut input = vec![x];
        run(&p, &mut input);
        let first = input[0].iter().enumerate().skip(1000).find(|(_, v)| v.abs() > 0.1).map(|(i, _)| i).unwrap();
        println!("loop back after {:.3} s", first as f64 / 48000.0);
        assert!((first as f64 / 48000.0 - 1.0).abs() < 0.01);
    }
    #[test]
    fn sound_on_sound_builds_but_stays_bounded() {
        let p = with(defaults(), &[(LENGTH, 0.5), (ERASE, 0.0), (MIX, 1.0)]);
        let mut input = vec![sine(220.0, 0.3, 48000 * 10)];
        run(&p, &mut input);
        assert!(input[0].iter().all(|v| v.is_finite() && v.abs() < 4.0));
    }

}
