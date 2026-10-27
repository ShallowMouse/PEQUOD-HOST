//! Warmer: a three-band saturating limiter for the bus, in the manner of the vintage-warmth plugins tagged on sombr's bass, guitars and master. Drive pushes each band into soft saturation and limiting; the bands are blended back for a thick, glued, slightly crushed sound.

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
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "lowf"]
    pub low_f: FloatParam,
    #[id = "highf"]
    pub high_f: FloatParam,
    #[id = "low"]
    pub low: FloatParam,
    #[id = "mid"]
    pub mid: FloatParam,
    #[id = "high"]
    pub high: FloatParam,
    #[id = "speed"]
    pub speed: FloatParam,
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
            drive: db("Drive", 3.0, 0.0, 24.0),
            low_f: hz("Low Split", 150.0, 50.0, 500.0),
            high_f: hz("High Split", 4000.0, 1500.0, 10000.0),
            low: db("Low", 0.0, -12.0, 12.0),
            mid: db("Mid", 0.0, -12.0, 12.0),
            high: db("High", 0.0, -12.0, 12.0),
            speed: ms("Release", 120.0, 10.0, 1000.0),
            mix: pct("Mix", 1.0),
            output: db("Ceiling", -0.3, -12.0, 0.0),
        }
    }
}

pub const DRIVE: usize = 0;
pub const LOWF: usize = 1;
pub const HIGHF: usize = 2;
pub const LOW: usize = 3;
pub const MID: usize = 4;
pub const HIGH: usize = 5;
pub const SPEED: usize = 6;
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
            self.drive.as_ptr(),
            self.low_f.as_ptr(),
            self.high_f.as_ptr(),
            self.low.as_ptr(),
            self.mid.as_ptr(),
            self.high.as_ptr(),
            self.speed.as_ptr(),
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
    lo: [[Biquad; 2]; 2],
    hi: [[Biquad; 2]; 2],
    key: (f64, f64),
    env: [f64; 3],
    lim: f64,
    os: [[Oversampler; 3]; 2],
    v: [f32; NPARAMS],
    drive: Smooth,
    mix: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self {
            fs,
            lo: [[Biquad::identity(); 2]; 2],
            hi: [[Biquad::identity(); 2]; 2],
            key: (0.0, 0.0),
            env: [0.0; 3],
            lim: 1.0,
            os: [[Oversampler::new(), Oversampler::new(), Oversampler::new()], [Oversampler::new(), Oversampler::new(), Oversampler::new()]],
            v,
            drive: Smooth::new(fs, 20.0, 1.0),
            mix: Smooth::new(fs, 20.0, 1.0),
        }
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.lo[c].iter_mut().for_each(|b| b.reset());
            self.hi[c].iter_mut().for_each(|b| b.reset());
            self.os[c].iter_mut().for_each(|o| o.reset());
        }
        self.env = [0.0; 3];
        self.lim = 1.0;
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = (v[LOWF] as f64, v[HIGHF] as f64);
        if (k.0 - self.key.0).abs() > 0.5 || (k.1 - self.key.1).abs() > 0.5 {
            for c in 0..2 {
                // Linkwitz-Riley 2nd order (two Butterworth 1st-order-equivalent biquads at Q 0.5)
                self.lo[c][0].set_lowpass(self.fs, k.0, 0.5);
                self.lo[c][1].set_lowpass(self.fs, k.0, 0.5);
                self.hi[c][0].set_highpass(self.fs, k.1, 0.5);
                self.hi[c][1].set_highpass(self.fs, k.1, 0.5);
            }
            self.key = k;
        }
        self.drive.set(db_to_gain(v[DRIVE] as f64));
        self.mix.set(v[MIX] as f64);
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let g = [db_to_gain(self.v[LOW] as f64), db_to_gain(self.v[MID] as f64), db_to_gain(self.v[HIGH] as f64)];
        let ceil = db_to_gain(self.v[OUTPUT] as f64);
        let rel = coeff(self.fs, self.v[SPEED] as f64);
        let mut grmax = 0.0f64;
        for k in 0..n {
            let d = self.drive.next();
            let mx = self.mix.next();
            let mut ys = [0.0f64; 2];
            let mut pk = 0.0f64;
            for c in 0..nch.min(2) {
                let x = chans[c][k] as f64;
                let l = cascade(&mut self.lo[c], x);
                let h = cascade(&mut self.hi[c], x);
                let m = x - l - h;
                let mut y = 0.0;
                for (b, s) in [l, m, h].iter().enumerate() {
                    let s = s * d * g[b];
                    y += self.os[c][b].process(s, |z| z / (1.0 + z.abs().powf(2.5)).powf(0.4)) / d.sqrt();
                }
                ys[c] = x * (1.0 - mx) + y * mx;
                pk = pk.max(ys[c].abs());
            }
            // final brickwall-ish limiter to the ceiling
            let want = if pk * self.lim > ceil { ceil / pk.max(1e-9) } else { 1.0 };
            self.lim = if want < self.lim { want } else { want + rel * (self.lim - want) };
            self.env[0] = self.lim;
            grmax = grmax.max(-gain_to_db(self.lim));
            for c in 0..nch.min(2) {
                chans[c][k] = (ys[c] * self.lim).clamp(-ceil, ceil) as f32;
            }
        }
        meters[0] = grmax as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Master Bus Warmth", values: &[(DRIVE, 4.0), (SPEED, 150.0), (OUTPUT, -0.3)] },
        Preset { name: "Thick Bass", values: &[(DRIVE, 9.0), (LOW, 3.0), (HIGH, -3.0), (SPEED, 100.0)] },
        Preset { name: "Crunchy Guitars", values: &[(DRIVE, 12.0), (MID, 3.0), (SPEED, 80.0)] },
        Preset { name: "Crushed Drums (parallel)", values: &[(DRIVE, 18.0), (SPEED, 40.0), (MIX, 0.4)] },
        Preset { name: "Gentle Glue", values: &[(DRIVE, 2.0), (SPEED, 300.0)] },
        Preset { name: "Dark Tape-ish", values: &[(DRIVE, 8.0), (HIGH, -5.0), (LOW, 2.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "WARMER",
        subtitle: ("THREE-BAND SATURATING LIMITER", "DRIVE · BANDS · CEILING"),
        accent: ui::WALNUT,
        size: (W, H),
        items: vec![

            Item::Knob { p: DRIVE, x: 100.0, y: 165.0, r: 46.0, label: "DRIVE", tip: "How hard the three bands are pushed into saturation.", active: None },
            Item::Meter { m: 0, x: 210.0, y: 96.0, w: 220.0, h: 128.0, kind: MeterKind::GainReduction, caption: "LIMIT" },
            Item::Knob { p: OUTPUT, x: 540.0, y: 165.0, r: 40.0, label: "CEILING", tip: "Output ceiling of the final limiter.", active: None },
            Item::Divider { y: 264.0 },
            Item::Knob { p: LOWF, x: 60.0, y: 330.0, r: 20.0, label: "LOW SPLIT", tip: "Crossover between low and mid bands.", active: None },
            Item::Knob { p: LOW, x: 130.0, y: 330.0, r: 20.0, label: "LOW", tip: "Low band level into its saturator.", active: None },
            Item::Knob { p: MID, x: 200.0, y: 330.0, r: 20.0, label: "MID", tip: "Mid band level.", active: None },
            Item::Knob { p: HIGH, x: 270.0, y: 330.0, r: 20.0, label: "HIGH", tip: "High band level.", active: None },
            Item::Knob { p: HIGHF, x: 340.0, y: 330.0, r: 20.0, label: "HIGH SPLIT", tip: "Crossover between mid and high bands.", active: None },
            Item::Knob { p: SPEED, x: 430.0, y: 330.0, r: 20.0, label: "RELEASE", tip: "Limiter release.", active: None },
            Item::Knob { p: MIX, x: 520.0, y: 330.0, r: 20.0, label: "MIX", tip: "Dry vs processed.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Warmer", class_id: *b"SBWarmerBusPQD01", vendor: "PEQUOD sombr",
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
    fn never_exceeds_ceiling() {
        let p = with(defaults(), &[(DRIVE, 20.0), (OUTPUT, -3.0)]);
        let mut input = vec![sine(100.0, 0.9, 48000), sine(3000.0, 0.9, 48000)];
        run(&p, &mut input);
        let pk = input.iter().flatten().fold(0.0f32, |a, v| a.max(v.abs()));
        println!("peak {:.2} dB", db(pk as f64));
        assert!(db(pk as f64) <= -2.99);
    }

}
