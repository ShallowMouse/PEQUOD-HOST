//! Freeze: after the Electro-Harmonix Freeze: switch FREEZE on and the note or chord you're playing is captured and held forever as a drone, with overlapping grains so it never loops audibly, while you play over it. If nothing is playing when you switch it on, it waits for the next note. FADE sets how quickly the drone swells in and dies away.

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
pub const H: u32 = 320;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "freeze"]
    pub freeze: BoolParam,
    #[id = "fade"]
    pub fade: FloatParam,
    #[id = "level"]
    pub level: FloatParam,
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
            freeze: BoolParam::new("Freeze", false),
            fade: ms("Fade", 300.0, 10.0, 3000.0),
            level: db("Drone Level", 0.0, -24.0, 6.0),
            dry: pct("Dry", 1.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const FREEZE: usize = 0;
pub const FADE: usize = 1;
pub const LEVEL: usize = 2;
pub const DRY: usize = 3;
pub const OUTPUT: usize = 4;
pub const NPARAMS: usize = 5;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.freeze.as_ptr(),
            self.fade.as_ptr(),
            self.level.as_ptr(),
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

const CAP_S: f64 = 0.25;
const GRAIN_S: f64 = 0.12;

struct Chan {
    ring: Vec<f64>,
    w: usize,
    cap: Vec<f64>,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ch: Vec<Chan>,
    captured: bool,
    filled: usize,
    env: f64,
    g: f64,
    grains: [(f64, usize); 4],
    seed: u32,
    out: Smooth,
}

impl Dsp {
    fn rnd(&mut self) -> f64 {
        self.seed = self.seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.seed >> 8) as f64 / 16777216.0
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let len = (CAP_S * fs) as usize;
        let gl = (GRAIN_S * fs) as usize;
        let ch = (0..ch.max(2)).map(|_| Chan { ring: vec![0.0; len], w: 0, cap: vec![0.0; len] }).collect();
        Self { fs, v, ch, captured: false, filled: 0, env: 0.0, g: 0.0, grains: [(0.0, 0), (0.0, gl / 4), (0.0, gl / 2), (0.0, 3 * gl / 4)], seed: 7, out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        for c in self.ch.iter_mut() {
            c.ring.iter_mut().for_each(|v| *v = 0.0);
            c.cap.iter_mut().for_each(|v| *v = 0.0);
            c.w = 0;
        }
        self.captured = false;
        self.filled = 0;
        self.env = 0.0;
        self.g = 0.0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let on = self.v[FREEZE] > 0.5;
        let len = self.ch[0].ring.len();
        let gl = (GRAIN_S * self.fs) as usize;
        let step = 1.0 / (self.v[FADE] as f64 * 0.001 * self.fs);
        let lvl = db_to_gain(self.v[LEVEL] as f64);
        let dry = self.v[DRY] as f64;
        let cr = coeff(self.fs, 30.0);
        let last = self.ch.len() - 1;
        for k in 0..n {
            let mut a = 0.0f64;
            for c in chans.iter() {
                a = a.max((c[k] as f64).abs());
            }
            self.env = a.max(a + cr * (self.env - a));
            self.filled = if self.env > 0.003 { self.filled + 1 } else { 0 };
            for (ci, c) in chans.iter().enumerate() {
                let s = &mut self.ch[ci.min(last)];
                s.ring[s.w] = c[k] as f64;
                s.w = (s.w + 1) % len;
            }
            if !on {
                if self.g <= 0.0 {
                    self.captured = false;
                }
            } else if !self.captured && self.filled >= len {
                for s in self.ch.iter_mut() {
                    for i in 0..len {
                        s.cap[i] = s.ring[(s.w + i) % len];
                    }
                }
                self.captured = true;
                self.g = 0.0;
            }
            self.g = if on && self.captured { (self.g + step).min(1.0) } else { (self.g - step).max(0.0) };
            // advance grains
            let mut starts = [0usize; 4];
            let mut wins = [0.0f64; 4];
            for gi in 0..4 {
                if self.grains[gi].1 >= gl {
                    let r = self.rnd();
                    self.grains[gi] = (r * (len - gl) as f64, 0);
                }
                let t = self.grains[gi].1 as f64 / gl as f64;
                wins[gi] = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * t).cos();
                starts[gi] = self.grains[gi].0 as usize + self.grains[gi].1;
                self.grains[gi].1 += 1;
            }
            let o = self.out.next();
            let gg = self.g * lvl * 0.5;
            for (ci, c) in chans.iter_mut().enumerate() {
                let s = &self.ch[ci.min(last)];
                let mut d = 0.0;
                if gg > 0.0 {
                    for gi in 0..4 {
                        d += s.cap[starts[gi].min(len - 1)] * wins[gi];
                    }
                }
                c[k] = ((c[k] as f64 * dry + d * gg) * o) as f32;
            }
        }
        meters[0] = self.g as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Hold the Chord", values: &[] },
        Preset { name: "Slow Swell Drone", values: &[(FADE, 1500.0)] },
        Preset { name: "Instant Hold", values: &[(FADE, 15.0)] },
        Preset { name: "Pad Under a Solo", values: &[(FADE, 800.0), (LEVEL, -6.0)] },
        Preset { name: "Drone Only", values: &[(DRY, 0.0)] },
        Preset { name: "Quiet Bed", values: &[(LEVEL, -12.0), (FADE, 2000.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "FREEZE",
        subtitle: ("AFTER THE EHX FREEZE", "INFINITE SUSTAIN · PLAY OVER IT"),
        accent: ui::INDIGO,
        size: (W, H),
        items: vec![

            Item::Switch { p: FREEZE, x: 95.0, y: 170.0, labels: ["OFF", "FREEZE"], tip: "Capture what you're playing and hold it. Switch off to let it fade." },
            Item::Lamp { m: 0, x: 215.0, y: 155.0, label: "HOLDING", lo: 0.05, hi: 0.9 },
            Item::Knob { p: FADE, x: 330.0, y: 170.0, r: 28.0, label: "FADE", tip: "How fast the drone swells in and dies away.", active: None },
            Item::Knob { p: LEVEL, x: 411.7, y: 170.0, r: 28.0, label: "DRONE", tip: "Level of the held sound.", active: None },
            Item::Knob { p: DRY, x: 493.3, y: 170.0, r: 28.0, label: "DRY", tip: "Your playing over it.", active: None },
            Item::Knob { p: OUTPUT, x: 575.0, y: 170.0, r: 28.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Freeze", class_id: *b"PqdFreezeSustain", vendor: "PEQUOD Pedals",
    categories: [Fx, Generator]
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

    #[test]
    fn holds_after_input_stops() {
        let mut x = vec![0.0f32; 96000];
        for k in 0..24000 {
            x[k] = (0.3 * (2.0 * std::f64::consts::PI * 330.0 * k as f64 / 48000.0).sin()) as f32;
        }
        let p = with(defaults(), &[(FREEZE, 1.0), (DRY, 0.0), (FADE, 50.0)]);
        let mut input = vec![x.clone(), x];
        run(&p, &mut input);
        let held = db(rms(&input[0][72000..]));
        let mut quiet = vec![vec![0.0f32; 96000]];
        run(&with(defaults(), &[(FREEZE, 0.0), (DRY, 0.0)]), &mut quiet);
        println!("held {held:.1} dB after the note stopped");
        assert!(held > -25.0 && rms(&quiet[0]) < 1e-6);
    }

}
