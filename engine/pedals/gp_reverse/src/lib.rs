//! Reverse Reverb: the reverb's tail is cut into slices of LENGTH and each slice played backwards, so every note swells up out of nothing and stops dead: the backwards-reverb sound of 60s psychedelia and modern ambient, live.

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
use vintage::verb::Fdn;

pub const W: u32 = 640;
pub const H: u32 = 330;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "length"]
    pub length: FloatParam,
    #[id = "decay"]
    pub decay: FloatParam,
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
            length: ms("Length", 600.0, 150.0, 2000.0),
            decay: secs("Decay", 2.0, 0.5, 8.0),
            mix: pct("Mix", 0.5),
            dry: pct("Dry", 1.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const LENGTH: usize = 0;
pub const DECAY: usize = 1;
pub const MIX: usize = 2;
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
            self.length.as_ptr(),
            self.decay.as_ptr(),
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

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    fdn: Fdn,
    rec: [Vec<f64>; 2],
    play: [Vec<f64>; 2],
    pos: usize,
    len: usize,
    key: i32,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let max = (2.05 * fs) as usize;
        let mut d = Self { fs, v, fdn: Fdn::new(fs, 2.5), rec: [vec![0.0; max], vec![0.0; max]], play: [vec![0.0; max], vec![0.0; max]], pos: 0, len: (0.6 * fs) as usize, key: -1, out: Smooth::new(fs, 20.0, 1.0) };
        d.fdn.configure(2.0, [2.0, 2.0, 1.4], 4000.0);
        d
    }
    fn reset(&mut self) {
        self.fdn.reset();
        for b in self.rec.iter_mut().chain(self.play.iter_mut()) {
            b.iter_mut().for_each(|v| *v = 0.0);
        }
        self.pos = 0;
        self.len = (self.v[LENGTH] as f64 * 0.001 * self.fs) as usize;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = (v[DECAY] * 100.0) as i32;
        if k != self.key {
            self.key = k;
            let rt = v[DECAY] as f64;
            self.fdn.configure(2.0, [rt, rt, rt * 0.7], 4000.0);
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let mix = self.v[MIX] as f64 * 1.6;
        let dry = self.v[DRY] as f64;
        let edge = (0.006 * self.fs) as usize;
        let nch = chans.len();
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            let dx = self.fdn.diffuse(x, 4);
            let (l, r) = self.fdn.process(dx);
            // dry signal goes into the slice too, so each note swells into itself
            self.rec[0][self.pos] = l + 0.5 * x;
            self.rec[1][self.pos] = r + 0.5 * x;
            let i = self.len - 1 - self.pos;
            let w = (self.pos.min(self.len - 1 - self.pos) as f64 / edge as f64).min(1.0);
            let (pl, pr) = (self.play[0][i] * w, self.play[1][i] * w);
            self.pos += 1;
            if self.pos >= self.len {
                std::mem::swap(&mut self.rec, &mut self.play);
                self.pos = 0;
                self.len = ((self.v[LENGTH] as f64 * 0.001 * self.fs) as usize).clamp(64, self.rec[0].len());
            }
            let o = self.out.next();
            if nch >= 2 {
                chans[0][k] = ((chans[0][k] as f64 * dry + pl * mix) * o) as f32;
                chans[1][k] = ((chans[1][k] as f64 * dry + pr * mix) * o) as f32;
            } else if nch == 1 {
                chans[0][k] = ((chans[0][k] as f64 * dry + 0.5 * (pl + pr) * mix) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Backwards Swell", values: &[] },
        Preset { name: "Short Reverse", values: &[(LENGTH, 300.0), (DECAY, 1.2)] },
        Preset { name: "Long Psychedelic", values: &[(LENGTH, 1400.0), (DECAY, 4.0), (MIX, 0.6)] },
        Preset { name: "Reverse Only", values: &[(DRY, 0.0), (MIX, 0.7)] },
        Preset { name: "Subtle Pre-Swell", values: &[(MIX, 0.3), (LENGTH, 450.0)] },
        Preset { name: "Ambient Bed", values: &[(LENGTH, 1800.0), (DECAY, 6.0), (MIX, 0.45)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "REVERSE REVERB",
        subtitle: ("BACKWARDS REVERB, LIVE", "EVERY NOTE SWELLS UP AND STOPS DEAD"),
        accent: ui::TEAL,
        size: (W, H),
        items: vec![
            Item::Knob { p: LENGTH, x: 90.0, y: 190.0, r: 36.0, label: "LENGTH", tip: "Length of each reversed slice.", active: None },
            Item::Knob { p: DECAY, x: 205.0, y: 190.0, r: 36.0, label: "DECAY", tip: "Reverb time inside each slice.", active: None },
            Item::Knob { p: MIX, x: 320.0, y: 190.0, r: 36.0, label: "MIX", tip: "Reversed sound level.", active: None },
            Item::Knob { p: DRY, x: 435.0, y: 190.0, r: 36.0, label: "DRY", tip: "Your playing.", active: None },
            Item::Knob { p: OUTPUT, x: 550.0, y: 190.0, r: 36.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Reverse Reverb", class_id: *b"PqdReverseReverb", vendor: "PEQUOD Pedals",
    categories: [Fx, Reverb]
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
    fn swells_up() {
        let p = with(defaults(), &[(LENGTH, 500.0), (DRY, 0.0), (MIX, 1.0)]);
        let (l, _) = impulse(&p, 72000);
        // slice 2 (24000..48000) plays slice 1 backwards: quiet first, loud last
        let early = db(rms(&l[24000..30000]));
        let late = db(rms(&l[42000..47500]));
        println!("reversed slice: start {early:.1} dB, end {late:.1} dB");
        assert!(late > early + 6.0);
    }

}
