//! Hand Phaser: two tape machines running the same signal, one slowed by hand, which Plank played like instruments. Machine B's delay passes through machine A's, so the sweep goes through zero: real tape flanging.

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


const BASE_MS: f64 = 11.0;


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "hand"]
    pub hand: FloatParam,
    #[id = "auto"]
    pub auto: FloatParam,
    #[id = "speed"]
    pub speed: FloatParam,
    #[id = "range"]
    pub range: FloatParam,
    #[id = "blend"]
    pub blend: FloatParam,
    #[id = "tape"]
    pub tape: FloatParam,
    #[id = "spread"]
    pub spread: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            hand: FloatParam::new("Hand", 0.0, FloatRange::Linear { min: -1.0, max: 1.0 }).with_step_size(0.001),
            auto: pct("Auto Hand", 0.5),
            speed: rate("Hand Speed", 0.15, 0.02, 2.0),
            range: ms("Range", 2.5, 0.3, 10.0),
            blend: pct("Machine Blend", 0.5),
            tape: pct("Tape Drive", 0.3),
            spread: pct("Stereo Spread", 0.0),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const HAND: usize = 0;
pub const AUTO: usize = 1;
pub const SPEED: usize = 2;
pub const RANGE: usize = 3;
pub const BLEND: usize = 4;
pub const TAPE: usize = 5;
pub const SPREAD: usize = 6;
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
            self.hand.as_ptr(),
            self.auto.as_ptr(),
            self.speed.as_ptr(),
            self.range.as_ptr(),
            self.blend.as_ptr(),
            self.tape.as_ptr(),
            self.spread.as_ptr(),
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
    a: [DelayLine; 2],
    b: [DelayLine; 2],
    hf: [[OnePole; 2]; 2],
    wander: Wander,
    lfo: Lfo,
    hand: Smooth,
    auto: f64,
    speed: f64,
    range: Smooth,
    blend: Smooth,
    tape: f64,
    spread: f64,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mk = || DelayLine::new((fs * 0.04) as usize);
        let mut hf = [[OnePole::default(); 2]; 2];
        for c in &mut hf {
            for p in c.iter_mut() {
                p.set_lowpass(fs, 14000.0);
            }
        }
        Self {
            fs,
            a: [mk(), mk()],
            b: [mk(), mk()],
            hf,
            wander: Wander::new(fs, 0.15, 21),
            lfo: Lfo::default(),
            hand: Smooth::new(fs, 30.0, 0.0),
            auto: 0.5,
            speed: 0.15,
            range: Smooth::new(fs, 50.0, 2.5),
            blend: Smooth::new(fs, 20.0, 0.5),
            tape: 0.3,
            spread: 0.0,
            out: Smooth::new(fs, 20.0, 1.0),
        }
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.a[c].reset();
            self.b[c].reset();
            self.hf[c].iter_mut().for_each(|p| p.reset());
        }
    }
    fn latency(&self) -> u32 {
        (BASE_MS * 0.001 * self.fs).round() as u32
    }
    fn set_params(&mut self, v: &[f32]) {
        self.hand.set(v[HAND] as f64);
        self.auto = v[AUTO] as f64;
        self.speed = v[SPEED] as f64;
        self.wander.set_rate(self.fs, self.speed);
        self.range.set(v[RANGE] as f64);
        self.blend.set(v[BLEND] as f64);
        self.tape = v[TAPE] as f64;
        self.spread = v[SPREAD] as f64;
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let mut peak = 0.0f64;
        let base = BASE_MS * 0.001 * self.fs;
        let dg = 1.0 + 4.0 * self.tape;
        for k in 0..n {
            // a hand on the flange: mostly smooth, never quite regular
            let auto = self.auto * (0.6 * self.lfo.next(self.fs, self.speed) + 0.4 * self.wander.next()).clamp(-1.0, 1.0);
            let h = (self.hand.next() + auto).clamp(-1.0, 1.0);
            let off = h * self.range.next() * 0.001 * self.fs;
            let bl = self.blend.next();
            let o = self.out.next();
            for c in 0..nch.min(2) {
                let x = chans[c][k] as f64;
                let t = (x * dg).tanh() / dg;
                self.a[c].write(t);
                self.b[c].write(t);
                // in stereo spread, the right channel's hand moves the other way
                let sgn = if c == 1 { 1.0 - 2.0 * self.spread } else { 1.0 };
                let ya = self.hf[c][0].lp(self.a[c].read(base));
                let yb = self.hf[c][1].lp(self.b[c].read((base + off * sgn).max(1.0)));
                let y = ((1.0 - bl) * ya + bl * yb) * o / (1.0 + 0.6 * self.tape);
                peak = peak.max(y.abs());
                chans[c][k] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Slow Hand Through Zero", values: &[(AUTO, 0.6), (SPEED, 0.1), (RANGE, 3.0)] },
        Preset { name: "Jet Plane", values: &[(AUTO, 0.9), (SPEED, 0.25), (RANGE, 6.0), (TAPE, 0.5)] },
        Preset { name: "Subtle Phasing", values: &[(AUTO, 0.4), (SPEED, 0.08), (RANGE, 0.8), (BLEND, 0.4)] },
        Preset { name: "Manual (automate HAND)", values: &[(AUTO, 0.0), (RANGE, 3.0)] },
        Preset { name: "Drum Bus Swirl", values: &[(AUTO, 0.7), (SPEED, 0.2), (RANGE, 2.0), (TAPE, 0.5)] },
        Preset { name: "Wide Stereo Phase", values: &[(AUTO, 0.6), (SPEED, 0.12), (RANGE, 2.5), (SPREAD, 1.0)] },
        Preset { name: "Synth Pad Drift", values: &[(AUTO, 0.5), (SPEED, 0.05), (RANGE, 5.0), (BLEND, 0.45)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "HAND PHASER",
        subtitle: ("TWO TAPE MACHINES PLAYED BY HAND", "THROUGH-ZERO FLANGING"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![

            Item::Knob { p: HAND, x: 120.0, y: 160.0, r: 52.0, label: "HAND", tip: "Your hand on machine B's reel: centre = in sync (cancels at full depth), left = B ahead, right = B behind. Automate it to play the phasing.", active: None },
            Item::Knob { p: AUTO, x: 270.0, y: 160.0, r: 34.0, label: "AUTO HAND", tip: "An automatic hand: slow, slightly irregular movement around the HAND position.", active: None },
            Item::Knob { p: SPEED, x: 390.0, y: 160.0, r: 34.0, label: "HAND SPEED", tip: "How fast the automatic hand moves.", active: None },
            Item::Knob { p: RANGE, x: 520.0, y: 160.0, r: 34.0, label: "RANGE", tip: "How far the hand can push machine B ahead or behind (ms).", active: None },
            Item::Divider { y: 264.0 },
            Item::Knob { p: BLEND, x: 100.0, y: 330.0, r: 22.0, label: "MACHINE BLEND", tip: "Balance of machine A and B. 50/50 gives the deepest notches.", active: None },
            Item::Knob { p: TAPE, x: 230.0, y: 330.0, r: 22.0, label: "TAPE DRIVE", tip: "Saturation of both machines.", active: None },
            Item::Knob { p: SPREAD, x: 360.0, y: 330.0, r: 22.0, label: "SPREAD", tip: "Stereo: the right channel's hand moves the opposite way.", active: None },
            Item::Knob { p: OUTPUT, x: 530.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Hand Phaser", class_id: *b"CPHandPhaserPQD1", vendor: "PEQUOD Plank",
    categories: [Fx, Modulation]
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
    fn in_sync_cancels_nothing_and_offset_notches() {
        let lvl = |hand: f32, f: f64| {
            let p = with(defaults(), &[(HAND, hand), (AUTO, 0.0), (TAPE, 0.0), (RANGE, 1.0)]);
            let mut input = vec![sine(f, 0.1, 48000)];
            run(&p, &mut input);
            db(rms(&input[0][24000..]) / (0.1 / 2f64.sqrt()))
        };
        // B 1 ms behind: first notch at 500 Hz
        let (z, notch, peakf) = (lvl(0.0, 500.0), lvl(1.0, 500.0), lvl(1.0, 1000.0));
        println!("in sync {z:.1} dB; offset 1 ms: 500 Hz {notch:.1} dB, 1 kHz {peakf:.1} dB");
        assert!(z.abs() < 1.0 && notch < -20.0 && peakf.abs() < 1.5);
    }

}
