//! Vocal Chain: the vocal chain Laura Sisk describes for Jack Antonoff's records: a U47-style tube mic with a dip near 8 kHz (Wunder CM7), a 1073-style preamp, an opto compressor at 1 ms attack and 50 ms release doing 3-5 dB, then optional doubling and chorus.

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
    #[id = "mic"]
    pub mic: FloatParam,
    #[id = "pre"]
    pub pre: FloatParam,
    #[id = "hpf"]
    pub hpf: FloatParam,
    #[id = "air"]
    pub air: FloatParam,
    #[id = "thresh"]
    pub thresh: FloatParam,
    #[id = "double"]
    pub double: FloatParam,
    #[id = "chorus"]
    pub chorus: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            mic: pct("Mic Colour", 1.0),
            pre: db("Pre Gain", 0.0, -12.0, 24.0),
            hpf: hz("High Pass", 80.0, 20.0, 300.0),
            air: db("High Shelf", 0.0, -12.0, 12.0),
            thresh: db("Comp Threshold", -18.0, -40.0, 0.0),
            double: pct("Double", 0.0),
            chorus: pct("Chorus", 0.0),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const MIC: usize = 0;
pub const PRE: usize = 1;
pub const HPF: usize = 2;
pub const AIR: usize = 3;
pub const THRESH: usize = 4;
pub const DOUBLE: usize = 5;
pub const CHORUS: usize = 6;
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
            self.mic.as_ptr(),
            self.pre.as_ptr(),
            self.hpf.as_ptr(),
            self.air.as_ptr(),
            self.thresh.as_ptr(),
            self.double.as_ptr(),
            self.chorus.as_ptr(),
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
    mic: [[Biquad; 3]; 2],
    hpf: [Biquad; 2],
    air: [Biquad; 2],
    key: [f64; 3],
    os: [Oversampler; 2],
    env: f64,
    gr: f64,
    dbl: [DelayLine; 2],
    w: [Wander; 2],
    lfo: Lfo,
    v: [f32; NPARAMS],
    pre: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self {
            fs,
            mic: [[Biquad::identity(); 3]; 2],
            hpf: [Biquad::identity(); 2],
            air: [Biquad::identity(); 2],
            key: [-1.0; 3],
            os: [Oversampler::new(), Oversampler::new()],
            env: 0.0,
            gr: 0.0,
            dbl: [DelayLine::new((fs * 0.06) as usize), DelayLine::new((fs * 0.06) as usize)],
            w: [Wander::new(fs, 0.4, 11), Wander::new(fs, 0.5, 23)],
            lfo: Lfo::default(),
            v,
            pre: Smooth::new(fs, 20.0, 1.0),
            out: Smooth::new(fs, 20.0, 1.0),
        }
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.mic[c].iter_mut().for_each(|b| b.reset());
            self.hpf[c].reset();
            self.air[c].reset();
            self.os[c].reset();
            self.dbl[c].reset();
        }
        self.env = 0.0;
        self.gr = 0.0;
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = [v[MIC] as f64, v[HPF] as f64, v[AIR] as f64];
        if k != self.key {
            for c in 0..2 {
                // U47-style: low-mid warmth, presence lift, and the CM7's dip near 8 kHz
                self.mic[c][0].set_peak(self.fs, 200.0, 0.8, 1.5 * k[0]);
                self.mic[c][1].set_peak(self.fs, 4000.0, 1.0, 2.0 * k[0]);
                self.mic[c][2].set_peak(self.fs, 8000.0, 2.0, -4.0 * k[0]);
                if k[1] <= 20.5 { self.hpf[c] = Biquad::identity() } else { self.hpf[c].set_highpass(self.fs, k[1], 0.7) }
                self.air[c].set_high_shelf(self.fs, 12000.0, 0.7, k[2]);
            }
            self.key = k;
        }
        self.pre.set(db_to_gain(v[PRE] as f64));
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let fs = self.fs;
        let thr = self.v[THRESH] as f64;
        let dbl = self.v[DOUBLE] as f64;
        let cho = self.v[CHORUS] as f64;
        // opto: 1 ms attack, 50 ms release (programme-dependent tail), 3:1
        let att = coeff(fs, 1.0);
        let rel = coeff(fs, 50.0);
        let mut grmax = 0.0f64;
        for k in 0..n {
            let pre = self.pre.next();
            let o = self.out.next();
            // mic + preamp first (per channel), then a linked compressor
            let mut s = [0.0f64; 2];
            for c in 0..nch.min(2) {
                let x = cascade(&mut self.mic[c], chans[c][k] as f64);
                let x = self.hpf[c].process(x * pre);
                // class-A transformer preamp: soft, mostly 2nd/3rd harmonic
                s[c] = self.os[c].process(x, |z| tube(z * 0.9, 0.1) / 0.9);
            }
            let lvl = s[0].abs().max(if nch > 1 { s[1].abs() } else { 0.0 });
            let target = (gain_to_db(lvl) - thr).max(0.0) * (1.0 - 1.0 / 3.0);
            self.gr = if target > self.gr { target + att * (self.gr - target) } else { target + rel * (self.gr - target) };
            grmax = grmax.max(self.gr);
            let g = db_to_gain(-self.gr);
            let lf = self.lfo.next(fs, 0.8);
            for c in 0..nch.min(2) {
                let y = self.air[c].process(s[c] * g);
                self.dbl[c].write(y);
                // doubler: a second, slightly late and drifting copy; chorus: a modulated short copy
                let dd = (18.0 + 6.0 * c as f64 + 3.0 * self.w[c].next()) * 0.001 * fs;
                let cd = (7.0 + 2.5 * if c == 0 { lf } else { -lf }) * 0.001 * fs;
                let z = y + dbl * 0.7 * self.dbl[c].read(dd) + cho * 0.6 * self.dbl[c].read(cd);
                chans[c][k] = (z * o / (1.0 + 0.4 * (dbl + cho))) as f32;
            }
        }
        meters[0] = grmax as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Sisk Vocal (3-5 dB)", values: &[(THRESH, -18.0), (HPF, 80.0)] },
        Preset { name: "Lana One-Take", values: &[(THRESH, -16.0), (HPF, 70.0), (AIR, 1.5)] },
        Preset { name: "Pop Vocal + Doubler", values: &[(THRESH, -22.0), (DOUBLE, 0.6), (AIR, 3.0), (HPF, 100.0)] },
        Preset { name: "Chorused Backing Vocals", values: &[(THRESH, -24.0), (CHORUS, 0.7), (DOUBLE, 0.4), (HPF, 150.0)] },
        Preset { name: "Driven Pre", values: &[(PRE, 14.0), (THRESH, -10.0), (OUTPUT, -8.0)] },
        Preset { name: "Flat Mic, Light Comp", values: &[(MIC, 0.0), (THRESH, -12.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "VOCAL CHAIN",
        subtitle: ("TUBE MIC · 1073-STYLE PRE · OPTO", "THE ANTONOFF / SISK VOCAL CHAIN"),
        accent: ui::STEEL,
        size: (W, H),
        items: vec![

            Item::Knob { p: MIC, x: 80.0, y: 160.0, r: 30.0, label: "MIC COLOUR", tip: "How much of the U47-style mic's curve (with the CM7's dip near 8 kHz that tames sibilance).", active: None },
            Item::Knob { p: PRE, x: 180.0, y: 160.0, r: 30.0, label: "PRE GAIN", tip: "1073-style preamp drive.", active: None },
            Item::Meter { m: 0, x: 250.0, y: 96.0, w: 200.0, h: 128.0, kind: MeterKind::GainReduction, caption: "OPTO" },
            Item::Knob { p: THRESH, x: 540.0, y: 160.0, r: 40.0, label: "THRESHOLD", tip: "Opto compressor threshold. Sisk aims for only 3-5 dB at peaks (1 ms attack, 50 ms release).", active: None },
            Item::Divider { y: 264.0 },
            Item::Knob { p: HPF, x: 80.0, y: 330.0, r: 22.0, label: "HIGH PASS", tip: "Preamp high-pass.", active: None },
            Item::Knob { p: AIR, x: 180.0, y: 330.0, r: 22.0, label: "HIGH SHELF", tip: "12 kHz shelf.", active: None },
            Item::Knob { p: DOUBLE, x: 300.0, y: 330.0, r: 22.0, label: "DOUBLE", tip: "A doubler after the compressor.", active: None },
            Item::Knob { p: CHORUS, x: 400.0, y: 330.0, r: 22.0, label: "CHORUS", tip: "Chorus after the doubler.", active: None },
            Item::Knob { p: OUTPUT, x: 540.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Vocal Chain", class_id: *b"JAVocalChainPQD1", vendor: "PEQUOD Antonoff",
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
    fn opto_does_a_few_db() {
        let p = defaults();
        let mut input = vec![sine(300.0, 0.25, 48000)];
        let m = run(&p, &mut input);
        println!("GR at -12 dBFS peaks: {:.1} dB", m[0]);
        assert!(m[0] > 2.0 && m[0] < 8.0);
    }

}
