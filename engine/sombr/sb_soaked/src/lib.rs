//! Soaked: a big, lush algorithmic reverb for the reverb-soaked vocals of sombr's bedroom recordings. Three colours (dark 70s, bright 80s, clean modern), a modulated tail and pre-delay. In the manner of the vintage-style plugin reverbs used on his records.

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
pub const H: u32 = 432;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Mode { #[name = "Hall"] Hall, #[name = "Plate"] Plate, #[name = "Room"] Room, #[name = "Chamber"] Chamber }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Colour { #[name = "1970s"] Seventies, #[name = "1980s"] Eighties, #[name = "Now"] Now }
// (size, diffusion stages)
const MODES: [(f64, usize); 4] = [(1.8, 4), (0.9, 4), (0.6, 2), (1.2, 3)];


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "mode"]
    pub mode: EnumParam<Mode>,
    #[id = "colour"]
    pub colour: EnumParam<Colour>,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "pre"]
    pub pre: FloatParam,
    #[id = "mod"]
    pub modu: FloatParam,
    #[id = "hicut"]
    pub hicut: FloatParam,
    #[id = "lowcut"]
    pub low_cut: FloatParam,
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
            mode: EnumParam::new("Mode", Mode::Hall),
            colour: EnumParam::new("Colour", Colour::Seventies),
            decay: secs("Decay", 3.0, 0.3, 12.0),
            pre: ms("Pre-delay", 20.0, 0.0, 250.0),
            modu: pct("Modulation", 0.4),
            hicut: hz("High Cut", 7000.0, 1500.0, 18000.0),
            low_cut: hz("Low Cut", 150.0, 20.0, 800.0),
            mix: pct("Mix", 0.35),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const MODE: usize = 0;
pub const COLOUR: usize = 1;
pub const DECAY: usize = 2;
pub const PRE: usize = 3;
pub const MOD: usize = 4;
pub const HICUT: usize = 5;
pub const LOWCUT: usize = 6;
pub const MIX: usize = 7;
pub const OUTPUT: usize = 8;
pub const NPARAMS: usize = 9;

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
            self.colour.as_ptr(),
            self.decay.as_ptr(),
            self.pre.as_ptr(),
            self.modu.as_ptr(),
            self.hicut.as_ptr(),
            self.low_cut.as_ptr(),
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
    fdn: Fdn,
    pre: DelayLine,
    key: (usize, usize, f64),
    hc: [Biquad; 2],
    lc: [Biquad; 2],
    filt: (f64, f64),
    pre_ms: Smooth,
    stages: usize,
    sat: f64,
    mix: Smooth,
    out: Smooth,
}

impl Dsp {
    fn configure(&mut self, mode: usize, col: usize, decay: f64) {
        let (size, st) = MODES[mode];
        // colour: how fast the top dies and how much the tail is modulated/darkened (estimated)
        let (hi_ratio, split) = [(0.35, 2500.0), (0.7, 5000.0), (0.85, 8000.0)][col];
        self.fdn.configure(size, [decay * 1.15, decay, decay * hi_ratio], split);
        self.stages = st;
        self.sat = [0.15, 0.05, 0.0][col];
        self.key = (mode, col, decay);
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut d = Self {
            fs,
            fdn: Fdn::new(fs, 2.0),
            pre: DelayLine::new((fs * 0.3) as usize),
            key: (9, 9, 0.0),
            hc: [Biquad::identity(); 2],
            lc: [Biquad::identity(); 2],
            filt: (0.0, 0.0),
            pre_ms: Smooth::new(fs, 80.0, 20.0),
            stages: 4,
            sat: 0.15,
            mix: Smooth::new(fs, 20.0, 0.35),
            out: Smooth::new(fs, 20.0, 1.0),
        };
        d.configure(0, 0, 3.0);
        d
    }
    fn reset(&mut self) {
        self.fdn.reset();
        self.pre.reset();
        for c in 0..2 {
            self.hc[c].reset();
            self.lc[c].reset();
        }
    }
    fn set_params(&mut self, v: &[f32]) {
        let k = ((v[MODE].round() as usize).min(3), (v[COLOUR].round() as usize).min(2), v[DECAY] as f64);
        if k.0 != self.key.0 || k.1 != self.key.1 || (k.2 - self.key.2).abs() > 1e-3 {
            self.configure(k.0, k.1, k.2);
        }
        self.fdn.mod_depth = 0.0002 + 0.004 * v[MOD] as f64;
        let f = (v[HICUT] as f64, v[LOWCUT] as f64);
        if (f.0 - self.filt.0).abs() > 0.5 || (f.1 - self.filt.1).abs() > 0.5 {
            for c in 0..2 {
                self.hc[c].set_lowpass(self.fs, f.0, 0.7);
                self.lc[c].set_highpass(self.fs, f.1, 0.7);
            }
            self.filt = f;
        }
        self.pre_ms.set(v[PRE] as f64);
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let mut peak = 0.0f64;
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            self.pre.write(x);
            let p = self.pre.read((self.pre_ms.next() * 0.001 * self.fs).max(1.0));
            // older colours: a little converter/analogue grit going in
            let p = if self.sat > 0.0 { (p * (1.0 + 4.0 * self.sat)).tanh() / (1.0 + 4.0 * self.sat) } else { p };
            let (wl, wr) = {
                let d = self.fdn.diffuse(p, self.stages);
                self.fdn.process(d)
            };
            let w = [self.lc[0].process(self.hc[0].process(wl)) * 1.5, self.lc[1].process(self.hc[1].process(wr)) * 1.5];
            let mx = self.mix.next();
            let o = self.out.next();
            for (ci, c) in chans.iter_mut().enumerate() {
                let wv = if nch == 1 { 0.5 * (w[0] + w[1]) } else { w[ci.min(1)] };
                let y = (c[k] as f64 * (1.0 - mx) + wv * mx) * o;
                peak = peak.max(y.abs());
                c[k] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Back to Friends Vocal Wash", values: &[(MODE, 0.0), (COLOUR, 0.0), (DECAY, 3.5), (PRE, 30.0), (MIX, 0.4)] },
        Preset { name: "Undressed Bedroom Hall", values: &[(MODE, 0.0), (COLOUR, 0.0), (DECAY, 4.5), (PRE, 15.0), (HICUT, 5500.0), (MIX, 0.45)] },
        Preset { name: "Bright 80s Plate", values: &[(MODE, 1.0), (COLOUR, 1.0), (DECAY, 2.2), (PRE, 20.0), (HICUT, 11000.0), (MIX, 0.3)] },
        Preset { name: "Soaked (100% wet send)", values: &[(MODE, 0.0), (COLOUR, 0.0), (DECAY, 5.0), (MIX, 1.0)] },
        Preset { name: "Guitar Room", values: &[(MODE, 2.0), (COLOUR, 2.0), (DECAY, 1.0), (PRE, 5.0), (MIX, 0.25)] },
        Preset { name: "Drum Chamber", values: &[(MODE, 3.0), (COLOUR, 1.0), (DECAY, 1.6), (PRE, 10.0), (LOWCUT, 250.0), (MIX, 0.3)] },
        Preset { name: "Endless Pad", values: &[(MODE, 0.0), (COLOUR, 2.0), (DECAY, 12.0), (MOD, 0.8), (MIX, 0.5)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "SOAKED",
        subtitle: ("LUSH ALGORITHMIC REVERB", "HALL · PLATE · ROOM · CHAMBER"),
        accent: ui::TEAL,
        size: (W, H),
        items: vec![

            Item::Selector { p: MODE, x: 75.0, y: 160.0, label: "MODE", positions: &["HALL", "PLATE", "ROOM", "CHAMBER"], tip: "Reverb type." },
            Item::Selector { p: COLOUR, x: 225.0, y: 160.0, label: "COLOUR", positions: &["70s", "80s", "NOW"], tip: "70s: dark, gritty, fast-dying top. 80s: bright. Now: clean." },
            Item::Knob { p: DECAY, x: 350.0, y: 165.0, r: 44.0, label: "DECAY", tip: "Decay time.", active: None },
            Item::Knob { p: PRE, x: 470.0, y: 165.0, r: 30.0, label: "PRE-DELAY", tip: "Delay before the reverb: keeps the voice clear of the wash.", active: None },
            Item::Knob { p: MOD, x: 570.0, y: 165.0, r: 26.0, label: "MOD", tip: "Tail modulation: lush and chorusy when up.", active: None },
            Item::Divider { y: 264.0 },
            Item::Knob { p: HICUT, x: 100.0, y: 330.0, r: 22.0, label: "HIGH CUT", tip: "Darkens the reverb.", active: None },
            Item::Knob { p: LOWCUT, x: 210.0, y: 330.0, r: 22.0, label: "LOW CUT", tip: "Keeps the wash out of the low end.", active: None },
            Item::Knob { p: MIX, x: 380.0, y: 330.0, r: 22.0, label: "MIX", tip: "Dry vs reverb.", active: None },
            Item::Knob { p: OUTPUT, x: 540.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Soaked", class_id: *b"SBSoakedVerbPQD1", vendor: "PEQUOD sombr",
    categories: [Fx, Reverb]
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
    fn decay_follows_setting() {
        for decay in [1.5f32, 4.0] {
            let p = with(defaults(), &[(DECAY, decay), (COLOUR, 2.0), (MIX, 1.0), (PRE, 0.0), (MOD, 0.0), (LOWCUT, 20.0), (HICUT, 18000.0)]);
            let mut x = vec![0.0f32; (48000.0 * decay * 2.5) as usize];
            x[0] = 0.3;
            let mut input = vec![x];
            run(&p, &mut input);
            let h: Vec<f64> = input[0].iter().map(|v| *v as f64).collect();
            let t = vintage::verb::t30(&h, 48000.0, 800.0);
            println!("decay {decay} -> {t:.2}");
            assert!((t / decay as f64 - 1.0).abs() < 0.3);
        }
    }

}
