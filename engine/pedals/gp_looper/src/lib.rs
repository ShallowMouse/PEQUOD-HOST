//! Looper: a simple phrase looper after the Boss RC-20XL style of pedal: turn MODE to RECORD to start a loop, to PLAY to close it and play it back, to OVERDUB to layer more on top, and to STOP to silence it. Flip CLEAR to erase. Up to 60 seconds; your playing always passes through.

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
pub const H: u32 = 300;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Mode { #[name = "Stop"] Stop, #[name = "Record"] Rec, #[name = "Play"] Play, #[name = "Overdub"] Dub }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "mode"]
    pub mode: EnumParam<Mode>,
    #[id = "clear"]
    pub clear: BoolParam,
    #[id = "loop_level"]
    pub loop_level: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            mode: EnumParam::new("Mode", Mode::Stop),
            clear: BoolParam::new("Clear", false),
            loop_level: db("Loop Level", 0.0, -24.0, 6.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const MODE: usize = 0;
pub const CLEAR: usize = 1;
pub const LOOP: usize = 2;
pub const OUTPUT: usize = 3;
pub const NPARAMS: usize = 4;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.mode.as_ptr(),
            self.clear.as_ptr(),
            self.loop_level.as_ptr(),
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

const MAX_S: f64 = 60.0;

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    buf: Vec<Vec<f32>>,
    len: usize,
    pos: usize,
    prev_clear: bool,
    lvl: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 2;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let max = (MAX_S * fs) as usize;
        Self { fs, v, buf: (0..ch.clamp(1, 2)).map(|_| vec![0.0; max]).collect(), len: 0, pos: 0, prev_clear: false, lvl: Smooth::new(fs, 20.0, 1.0), out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.pos = 0;
        self.lvl.snap(db_to_gain(self.v[LOOP] as f64));
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let clear = v[CLEAR] > 0.5;
        if clear && !self.prev_clear {
            let used = self.len.max(self.pos);
            for b in self.buf.iter_mut() {
                let u = used.min(b.len());
                b[..u].iter_mut().for_each(|s| *s = 0.0);
            }
            self.len = 0;
            self.pos = 0;
        }
        self.prev_clear = clear;
        self.lvl.set(db_to_gain(v[LOOP] as f64));
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let mode = self.v[MODE].round() as usize;
        let max = self.buf[0].len();
        let last = self.buf.len() - 1;
        let _ = self.fs;
        for k in 0..n {
            let lv = self.lvl.next() as f32;
            let o = self.out.next() as f32;
            let first_take = self.len == 0;
            match mode {
                1 if first_take => {
                    // recording the first pass: the loop grows until MODE leaves RECORD
                    for (ci, c) in chans.iter_mut().enumerate() {
                        self.buf[ci.min(last)][self.pos] = c[k];
                        c[k] *= o;
                    }
                    self.pos += 1;
                    if self.pos >= max {
                        self.len = max;
                        self.pos = 0;
                    }
                }
                1 | 3 if !first_take => {
                    for (ci, c) in chans.iter_mut().enumerate() {
                        let b = &mut self.buf[ci.min(last)];
                        let l = b[self.pos];
                        b[self.pos] = (l + c[k]).clamp(-4.0, 4.0);
                        c[k] = (c[k] + l * lv) * o;
                    }
                    self.pos = (self.pos + 1) % self.len;
                }
                2 if !first_take => {
                    for (ci, c) in chans.iter_mut().enumerate() {
                        c[k] = (c[k] + self.buf[ci.min(last)][self.pos] * lv) * o;
                    }
                    self.pos = (self.pos + 1) % self.len;
                }
                _ => {
                    if first_take && self.pos > 0 && mode != 1 {
                        // closing the first take
                        self.len = self.pos;
                        self.pos = 0;
                        continue;
                    }
                    if mode == 0 {
                        self.pos = 0;
                    }
                    for c in chans.iter_mut() {
                        c[k] *= o;
                    }
                }
            }
        }
        meters[0] = if mode == 1 || (mode == 3 && self.len > 0) { 1.0 } else { 0.0 };
        meters[1] = if self.len > 0 && mode >= 2 { 1.0 } else { 0.0 };
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Ready", values: &[] },
        Preset { name: "Quiet Loop Under a Solo", values: &[(LOOP, -6.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "LOOPER",
        subtitle: ("PHRASE LOOPER", "RECORD · PLAY · OVERDUB · 60 SECONDS"),
        accent: ui::GRAPHITE,
        size: (W, H),
        items: vec![

            Item::Selector { p: MODE, x: 120.0, y: 170.0, label: "MODE", positions: &["STOP", "REC", "PLAY", "DUB"], tip: "RECORD starts a loop; PLAY closes it and plays it; OVERDUB layers more; STOP silences it." },
            Item::Lamp { m: 0, x: 250.0, y: 150.0, label: "RECORDING", lo: 0.2, hi: 0.8 },
            Item::Lamp { m: 1, x: 250.0, y: 190.0, label: "PLAYING", lo: 0.2, hi: 0.8 },
            Item::Switch { p: CLEAR, x: 360.0, y: 170.0, labels: ["", "CLEAR"], tip: "Flip on to erase the loop (then back off)." },
            Item::Knob { p: LOOP, x: 490.0, y: 170.0, r: 30.0, label: "LOOP LEVEL", tip: "Loop playback level.", active: None },
            Item::Knob { p: OUTPUT, x: 580.0, y: 170.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Looper", class_id: *b"PqdLooperPhrase1", vendor: "PEQUOD Pedals",
    categories: [Fx, Tools]
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

    fn block(d: &mut Dsp, p: &[f32], x: &mut Vec<Vec<f32>>) {
        let mut m = vec![0.0f32; 2];
        let n = x[0].len();
        let mut k = 0;
        while k < n {
            let e = (k + 512).min(n);
            d.set_params(p);
            let mut ch: Vec<&mut [f32]> = x.iter_mut().map(|c| &mut c[k..e]).collect();
            d.process(&mut ch, &mut m);
            k = e;
        }
    }
    #[test]
    fn records_and_plays_back() {
        let mut d = Dsp::new(48000.0, 2);
        d.set_params(&defaults());
        d.reset();
        let mut x = vec![sine(300.0, 0.3, 24000), sine(300.0, 0.3, 24000)];
        block(&mut d, &with(defaults(), &[(MODE, 1.0)]), &mut x);
        let mut y = vec![vec![0.0f32; 48000], vec![0.0f32; 48000]];
        block(&mut d, &with(defaults(), &[(MODE, 2.0)]), &mut y);
        let a = db(rms(&y[0][1000..23000]));
        let b = db(rms(&y[0][25000..47000]));
        let mut z = vec![vec![0.0f32; 24000], vec![0.0f32; 24000]];
        block(&mut d, &with(defaults(), &[(MODE, 0.0)]), &mut z);
        println!("loop pass 1 {a:.1} dB, pass 2 {b:.1} dB, stopped {:.1}", db(rms(&z[0])));
        assert!(a > -15.0 && b > -15.0 && rms(&z[0]) < 1e-6);
    }

}
