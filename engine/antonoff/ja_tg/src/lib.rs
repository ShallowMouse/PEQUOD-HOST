//! TG Strip: an EMI TG12345 desk channel of the kind Jack Antonoff runs guitars and bass through (Chandler TG2 strips; the EMI desk at Jungle City), riding the compressor while playing and pushing it into heavy saturation. Laws estimated from published behaviour.

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
    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "bass"]
    pub bass: FloatParam,
    #[id = "treble"]
    pub treble: FloatParam,
    #[id = "presence"]
    pub presence: FloatParam,
    #[id = "comp"]
    pub comp: FloatParam,
    #[id = "recovery"]
    pub recovery: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            gain: db("Input Gain", 10.0, 0.0, 40.0),
            bass: db("Bass", 0.0, -10.0, 10.0),
            treble: db("Treble", 0.0, -10.0, 10.0),
            presence: db("Presence", 0.0, -10.0, 10.0),
            comp: pct("Compression", 0.4),
            recovery: ms("Recovery", 250.0, 50.0, 2000.0),
            output: db("Output", -6.0, -30.0, 12.0),
        }
    }
}

pub const GAIN: usize = 0;
pub const BASS: usize = 1;
pub const TREBLE: usize = 2;
pub const PRESENCE: usize = 3;
pub const COMP: usize = 4;
pub const RECOVERY: usize = 5;
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
            self.gain.as_ptr(),
            self.bass.as_ptr(),
            self.treble.as_ptr(),
            self.presence.as_ptr(),
            self.comp.as_ptr(),
            self.recovery.as_ptr(),
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
    os: [Oversampler; 2],
    eq: [[Biquad; 3]; 2],
    key: [f64; 3],
    env: f64,
    gr: f64,
    v: [f32; NPARAMS],
    gain: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, os: [Oversampler::new(), Oversampler::new()], eq: [[Biquad::identity(); 3]; 2], key: [99.0; 3], env: 0.0, gr: 0.0, v, gain: Smooth::new(fs, 20.0, 1.0), out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.os[c].reset();
            self.eq[c].iter_mut().for_each(|b| b.reset());
        }
        self.env = 0.0;
        self.gr = 0.0;
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = [v[BASS] as f64, v[TREBLE] as f64, v[PRESENCE] as f64];
        if k != self.key {
            for c in &mut self.eq {
                c[0].set_low_shelf(self.fs, 110.0, 0.6, k[0]);
                c[1].set_high_shelf(self.fs, 10000.0, 0.6, k[1]);
                c[2].set_peak(self.fs, 2800.0, 0.9, k[2]);
            }
            self.key = k;
        }
        self.gain.set(db_to_gain(v[GAIN] as f64 - 10.0));
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let comp = self.v[COMP] as f64;
        let rel = coeff(self.fs, self.v[RECOVERY] as f64);
        let att = coeff(self.fs, 2.0);
        let thr = -6.0 - 30.0 * comp;
        let mut grmax = 0.0f64;
        for k in 0..n {
            let g = self.gain.next();
            let o = self.out.next();
            let mut lvl = 0.0f64;
            for c in chans.iter() {
                lvl = lvl.max((c[k] as f64 * g).abs());
            }
            // feed-forward limiter-compressor, ~4:1 rising to hard limiting as COMP goes up
            let over = (gain_to_db(lvl) - thr).max(0.0);
            let ratio = 3.0 + 17.0 * comp * comp;
            let target = over * (1.0 - 1.0 / ratio);
            self.gr = if target > self.gr { target + att * (self.gr - target) } else { target + rel * (self.gr - target) };
            grmax = grmax.max(self.gr);
            let gc = db_to_gain(-self.gr + 0.5 * self.gr.min(24.0));
            for c in 0..nch.min(2) {
                let x = chans[c][k] as f64 * g * gc;
                // transistor channel: hard-ish, bright saturation when pushed
                let y = self.os[c].process(x, |s| {
                    let a = s * 1.4;
                    (a / (1.0 + a.abs().powf(2.2)).powf(1.0 / 2.2)) / 1.4 + 0.03 * s * s / (1.0 + s * s)
                });
                let y = cascade(&mut self.eq[c], y) * o;
                chans[c][k] = y as f32;
            }
        }
        meters[0] = grmax as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Guitar DI Into the Desk", values: &[(GAIN, 26.0), (COMP, 0.5), (TREBLE, 3.0), (OUTPUT, -10.0)] },
        Preset { name: "Bass Through the Strip", values: &[(GAIN, 20.0), (COMP, 0.45), (BASS, 4.0), (PRESENCE, 2.0), (OUTPUT, -8.0)] },
        Preset { name: "Heavy Saturation (ride it)", values: &[(GAIN, 36.0), (COMP, 0.85), (RECOVERY, 150.0), (OUTPUT, -18.0)] },
        Preset { name: "Fuzzy Guitars (LWYMMD)", values: &[(GAIN, 40.0), (COMP, 0.9), (PRESENCE, 5.0), (BASS, -3.0), (OUTPUT, -20.0)] },
        Preset { name: "Drum Crush", values: &[(GAIN, 30.0), (COMP, 1.0), (RECOVERY, 100.0), (TREBLE, 4.0), (OUTPUT, -14.0)] },
        Preset { name: "Clean Channel", values: &[(GAIN, 10.0), (COMP, 0.1), (OUTPUT, -6.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "TG STRIP",
        subtitle: ("IN THE MANNER OF THE EMI TG12345", "DESK CHANNEL · COMPRESSOR · DRIVE"),
        accent: ui::STEEL,
        size: (W, H),
        items: vec![

            Item::Knob { p: GAIN, x: 90.0, y: 160.0, r: 44.0, label: "INPUT GAIN", tip: "How hard the channel is hit. Up high it saturates like an EMI desk pushed too far.", active: None },
            Item::Meter { m: 0, x: 200.0, y: 96.0, w: 220.0, h: 128.0, kind: MeterKind::GainReduction, caption: "COMPRESSION" },
            Item::Knob { p: COMP, x: 545.0, y: 160.0, r: 44.0, label: "COMPRESSION", tip: "Compressor amount: from gentle to slammed. Antonoff rides this while playing.", active: None },
            Item::Divider { y: 264.0 },
            Item::Knob { p: BASS, x: 80.0, y: 330.0, r: 22.0, label: "BASS", tip: "Bass shelf.", active: None },
            Item::Knob { p: PRESENCE, x: 180.0, y: 330.0, r: 22.0, label: "PRESENCE", tip: "Presence at about 2.8 kHz.", active: None },
            Item::Knob { p: TREBLE, x: 280.0, y: 330.0, r: 22.0, label: "TREBLE", tip: "Treble shelf.", active: None },
            Item::Knob { p: RECOVERY, x: 400.0, y: 330.0, r: 22.0, label: "RECOVERY", tip: "Compressor release.", active: None },
            Item::Knob { p: OUTPUT, x: 540.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "TG Strip", class_id: *b"JATGStripPQD0001", vendor: "PEQUOD Antonoff",
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
    fn compresses_more_with_comp() {
        let gr = |c: f32| {
            let p = with(defaults(), &[(COMP, c), (GAIN, 20.0)]);
            let mut input = vec![sine(1000.0, 0.3, 48000)];
            run(&p, &mut input)[0]
        };
        let (a, b) = (gr(0.1), gr(0.9));
        println!("GR comp 0.1 {a:.1} dB, 0.9 {b:.1} dB");
        assert!(b > a + 6.0);
    }

}
