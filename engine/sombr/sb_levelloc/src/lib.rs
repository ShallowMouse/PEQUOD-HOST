//! Level Loc: a cheap, brutal automatic level limiter after the Shure M62 Level-Loc, tagged on the bass and guitar of sombr's 12 to 12 at Sound City. No threshold and almost no controls: everything gets pinned to one level, pumping and crunching on the way. Behaviour estimated.

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



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "input"]
    pub input: FloatParam,
    #[id = "recovery"]
    pub recovery: FloatParam,
    #[id = "grit"]
    pub grit: FloatParam,
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
            input: db("Input", 0.0, -24.0, 24.0),
            recovery: ms("Recovery", 400.0, 50.0, 3000.0),
            grit: pct("Grit", 0.4),
            mix: pct("Mix", 1.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const INPUT: usize = 0;
pub const RECOVERY: usize = 1;
pub const GRIT: usize = 2;
pub const MIX: usize = 3;
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
            self.input.as_ptr(),
            self.recovery.as_ptr(),
            self.grit.as_ptr(),
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
    gain: f64,
    os: [Oversampler; 2],
    hp: [Biquad; 2],
    lp: [Biquad; 2],
    input: Smooth,
    recovery: f64,
    grit: f64,
    mix: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut hp = [Biquad::identity(); 2];
        let mut lp = [Biquad::identity(); 2];
        for c in 0..2 {
            // a narrow, cheap box: no deep bass, soft top
            hp[c].set_highpass(fs, 70.0, 0.7);
            lp[c].set_lowpass(fs, 11000.0, 0.7);
        }
        Self { fs, gain: 1.0, os: [Oversampler::new(), Oversampler::new()], hp, lp, input: Smooth::new(fs, 20.0, 1.0), recovery: 400.0, grit: 0.4, mix: Smooth::new(fs, 20.0, 1.0), out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.gain = 1.0;
        for c in 0..2 {
            self.os[c].reset();
            self.hp[c].reset();
            self.lp[c].reset();
        }
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn set_params(&mut self, v: &[f32]) {
        self.input.set(db_to_gain(v[INPUT] as f64));
        self.recovery = v[RECOVERY] as f64;
        self.grit = v[GRIT] as f64;
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let rec = 1.0 - coeff(self.fs, self.recovery);
        let att = 1.0 - coeff(self.fs, 0.5);
        // target output level ~ -12 dBFS peak; up to 30 dB of make-up for quiet input
        let target = 0.25;
        let mut grmax = 0.0f64;
        for k in 0..n {
            let gi = self.input.next();
            let mut lvl = 0.0f64;
            for c in chans.iter() {
                lvl = lvl.max((c[k] as f64 * gi).abs());
            }
            let want = (target / lvl.max(1e-6)).min(31.6);
            // clamps down fast, creeps back up slowly: the pump
            self.gain += (want - self.gain) * if want < self.gain { att } else { rec };
            grmax = grmax.max(-gain_to_db(self.gain).min(0.0));
            let g = self.gain;
            let mx = self.mix.next();
            let o = self.out.next();
            let grit = self.grit;
            let dg = 1.0 + 6.0 * grit;
            for c in 0..nch.min(2) {
                let x = chans[c][k] as f64;
                let y = x * gi * g;
                // transistor stage running out of headroom
                let y = self.os[c].process(y, |s| ((s * dg).tanh() + 0.1 * grit * (s * dg).tanh().powi(2)) / dg * 1.6);
                let y = self.lp[c].process(self.hp[c].process(y));
                chans[c][k] = ((x * (1.0 - mx) + y * mx) * o) as f32;
            }
        }
        meters[0] = grmax as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "12 to 12 Bass", values: &[(INPUT, 6.0), (RECOVERY, 300.0), (GRIT, 0.4)] },
        Preset { name: "Squashed Guitar", values: &[(INPUT, 9.0), (RECOVERY, 500.0), (GRIT, 0.5)] },
        Preset { name: "Pumping Drum Room", values: &[(INPUT, 12.0), (RECOVERY, 250.0), (GRIT, 0.6), (MIX, 0.6)] },
        Preset { name: "Parallel Crush", values: &[(INPUT, 12.0), (RECOVERY, 400.0), (GRIT, 0.7), (MIX, 0.35)] },
        Preset { name: "Slow Breathing Pad", values: &[(INPUT, 3.0), (RECOVERY, 2000.0), (GRIT, 0.2)] },
        Preset { name: "Lo-Fi Vocal Pin", values: &[(INPUT, 6.0), (RECOVERY, 350.0), (GRIT, 0.5), (MIX, 0.7)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "LEVEL LOC",
        subtitle: ("AFTER THE SHURE M62 LEVEL-LOC", "AUTOMATIC LIMITER · NO THRESHOLD"),
        accent: ui::MUSTARD,
        size: (W, H),
        items: vec![

            Item::Knob { p: INPUT, x: 100.0, y: 165.0, r: 46.0, label: "INPUT", tip: "Input level. There is no threshold: everything is pulled to one level.", active: None },
            Item::Meter { m: 0, x: 210.0, y: 96.0, w: 220.0, h: 138.0, kind: MeterKind::GainReduction, caption: "LEVEL LOC" },
            Item::Knob { p: RECOVERY, x: 540.0, y: 165.0, r: 40.0, label: "RECOVERY", tip: "How slowly the gain comes back up after a loud hit: the pump. (The original has no control for this; estimated.)", active: None },
            Item::Divider { y: 264.0 },
            Item::Knob { p: GRIT, x: 140.0, y: 330.0, r: 24.0, label: "GRIT", tip: "How far the cheap output stage breaks up.", active: None },
            Item::Knob { p: MIX, x: 320.0, y: 330.0, r: 24.0, label: "MIX", tip: "Dry vs limited (parallel).", active: None },
            Item::Knob { p: OUTPUT, x: 500.0, y: 330.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Level Loc", class_id: *b"SBLevelLocPQD001", vendor: "PEQUOD sombr",
    categories: [Fx, Dynamics]
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
    fn pins_loud_and_quiet_to_one_level() {
        let out = |amp: f64| {
            let p = with(defaults(), &[(GRIT, 0.0)]);
            let mut input = vec![sine(1000.0, amp, 96000)];
            run(&p, &mut input);
            db(rms(&input[0][72000..]))
        };
        let (a, b) = (out(0.5), out(0.05));
        println!("in 20 dB apart -> out {:.1} dB apart", a - b);
        assert!((a - b).abs() < 3.0);
    }

}
