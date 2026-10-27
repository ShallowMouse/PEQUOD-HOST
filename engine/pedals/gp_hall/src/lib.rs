//! Hall: a big, smooth concert-hall reverb for guitar: an 8-line feedback delay network with input diffusion, PRE-DELAY to keep the notes clear, DAMP to darken the tail and MOD for a lush, chorused decay.

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
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "size"]
    pub size: FloatParam,
    #[id = "pre"]
    pub pre: FloatParam,
    #[id = "damp"]
    pub damp: FloatParam,
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
            decay: secs("Decay", 2.5, 0.4, 12.0),
            size: pct("Size", 0.6),
            pre: ms("Pre-Delay", 20.0, 0.0, 200.0),
            damp: pct("Damp", 0.4),
            mod_amt: pct("Mod", 0.3),
            mix: pct("Mix", 0.3),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const DECAY: usize = 0;
pub const SIZE: usize = 1;
pub const PRE: usize = 2;
pub const DAMP: usize = 3;
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
            self.decay.as_ptr(),
            self.size.as_ptr(),
            self.pre.as_ptr(),
            self.damp.as_ptr(),
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
    fdn: Fdn,
    pre: DelayLine,
    key: Vec<i32>,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let rt = self.v[DECAY] as f64;
        let damp = self.v[DAMP] as f64;
        self.fdn.configure(1.5 + 2.5 * self.v[SIZE] as f64, [rt * 1.1, rt, rt * (1.0 - 0.75 * damp)], 3000.0);
        self.fdn.mod_depth = 0.0004 + 0.004 * self.v[MOD] as f64;
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, fdn: Fdn::new(fs, 4.1), pre: DelayLine::new((0.21 * fs) as usize), key: vec![], out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.fdn.reset();
        self.pre.reset();
        self.design();
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k: Vec<i32> = [v[DECAY], v[SIZE], v[DAMP], v[MOD]].iter().map(|x| (x * 100.0) as i32).collect();
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let mix = self.v[MIX] as f64;
        let (dg, wg) = ((1.0 - mix).sqrt(), mix.sqrt() * 1.4);
        let pd = (self.v[PRE] as f64 * 0.001 * self.fs).max(1.0);
        let nch = chans.len();
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            self.pre.write(x);
            let d = self.fdn.diffuse(self.pre.read(pd), 4);
            let (l, r) = self.fdn.process(d);
            let o = self.out.next();
            if nch >= 2 {
                chans[0][k] = ((chans[0][k] as f64 * dg + l * wg) * o) as f32;
                chans[1][k] = ((chans[1][k] as f64 * dg + r * wg) * o) as f32;
            } else if nch == 1 {
                chans[0][k] = ((chans[0][k] as f64 * dg + 0.5 * (l + r) * wg) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Concert Hall", values: &[] },
        Preset { name: "Small Hall", values: &[(DECAY, 1.4), (SIZE, 0.35), (MIX, 0.25)] },
        Preset { name: "Big Lush Hall", values: &[(DECAY, 4.5), (SIZE, 0.9), (MOD, 0.6), (MIX, 0.35)] },
        Preset { name: "Dark Cathedral", values: &[(DECAY, 8.0), (SIZE, 1.0), (DAMP, 0.8), (PRE, 60.0), (MIX, 0.4)] },
        Preset { name: "Clear Lead Hall", values: &[(DECAY, 2.2), (PRE, 90.0), (MIX, 0.28)] },
        Preset { name: "Gilmour-ish Wash", values: &[(DECAY, 3.5), (SIZE, 0.8), (DAMP, 0.5), (MOD, 0.45), (PRE, 40.0), (MIX, 0.32)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "HALL",
        subtitle: ("CONCERT-HALL REVERB", "PRE-DELAY · DAMP · MOD"),
        accent: ui::TEAL,
        size: (W, H),
        items: vec![
            Item::Knob { p: DECAY, x: 60.0, y: 190.0, r: 30.0, label: "DECAY", tip: "Reverb time.", active: None },
            Item::Knob { p: SIZE, x: 146.7, y: 190.0, r: 30.0, label: "SIZE", tip: "Room size.", active: None },
            Item::Knob { p: PRE, x: 233.3, y: 190.0, r: 30.0, label: "PRE-DELAY", tip: "Gap before the reverb: keeps notes clear.", active: None },
            Item::Knob { p: DAMP, x: 320.0, y: 190.0, r: 30.0, label: "DAMP", tip: "Darkens the tail.", active: None },
            Item::Knob { p: MOD, x: 406.7, y: 190.0, r: 30.0, label: "MOD", tip: "Chorused, lush tail.", active: None },
            Item::Knob { p: MIX, x: 493.3, y: 190.0, r: 30.0, label: "MIX", tip: "Reverb level.", active: None },
            Item::Knob { p: OUTPUT, x: 580.0, y: 190.0, r: 30.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Hall", class_id: *b"PqdHallReverbQ01", vendor: "PEQUOD Pedals",
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

    #[test]
    fn decay_follows_the_knob() {
        let mut x = vec![0.0f32; 48000 * 8];
        x[100] = 1.0;
        let mut a = vec![x.clone(), x.clone()];
        run(&with(defaults(), &[(DECAY, 1.5), (MIX, 1.0)]), &mut a);
        let mut b = vec![x.clone(), x];
        run(&with(defaults(), &[(DECAY, 5.0), (MIX, 1.0)]), &mut b);
        let ta = vintage::verb::t30(&a[0].iter().map(|v| *v as f64).collect::<Vec<_>>(), 48000.0, 1000.0);
        let tb = vintage::verb::t30(&b[0].iter().map(|v| *v as f64).collect::<Vec<_>>(), 48000.0, 1000.0);
        println!("T30 at 1 kHz: decay 1.5 -> {ta:.2} s, decay 5 -> {tb:.2} s");
        assert!(tb > ta * 2.0);
    }

}
