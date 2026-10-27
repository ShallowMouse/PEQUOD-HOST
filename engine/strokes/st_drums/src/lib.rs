//! Hard to Explain: a drum-bus processor for the Strokes' small, squashed, almost drum-machine drums (Is This It, Hard to Explain). A fast, hard compressor, a sustain cutter that shortens every hit, a band-limit and optional early-sampler grit, so a real kit sounds tight and mechanical. Settings estimated: no chain of Gordon Raphael's was published.

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
use vintage::fx::{comp_gr, quantise};

pub const W: u32 = 640;
pub const H: u32 = 432;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "squash"]
    pub squash: FloatParam,
    #[id = "shorten"]
    pub shorten: FloatParam,
    #[id = "top"]
    pub top: FloatParam,
    #[id = "lowcut"]
    pub lowcut: FloatParam,
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
            squash: pct("Squash", 0.6),
            shorten: pct("Shorten", 0.5),
            top: hz("Top Cut", 7000.0, 2500.0, 18000.0),
            lowcut: hz("Low Cut", 60.0, 20.0, 300.0),
            grit: pct("Sampler Grit", 0.0),
            mix: pct("Mix", 1.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const SQUASH: usize = 0;
pub const SHORTEN: usize = 1;
pub const TOP: usize = 2;
pub const LOWCUT: usize = 3;
pub const GRIT: usize = 4;
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
            self.squash.as_ptr(),
            self.shorten.as_ptr(),
            self.top.as_ptr(),
            self.lowcut.as_ptr(),
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
    v: [f32; NPARAMS],
    filt: Vec<[Biquad; 3]>,
    rms: f64,
    gr: f64,
    fast: f64,
    slow: f64,
    hold: Vec<f64>,
    hcount: f64,
    key: (i32, i32),
    mix: Smooth,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        for f in self.filt.iter_mut() {
            f[0].set_highpass(fs, self.v[LOWCUT] as f64, 0.7);
            f[1].set_lowpass(fs, self.v[TOP] as f64, 0.7);
            f[2].set_peak(fs, 180.0, 1.0, 2.0);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, filt: vec![[Biquad::identity(); 3]; ch.max(2)], rms: 0.0, gr: 0.0, fast: 0.0, slow: 0.0, hold: vec![0.0; ch.max(2)], hcount: 0.0,
                           key: (-1, -1), mix: Smooth::new(fs, 20.0, 1.0), out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.filt.iter_mut().for_each(|f| f.iter_mut().for_each(|b| b.reset()));
        self.rms = 0.0;
        self.gr = 0.0;
        self.fast = 0.0;
        self.slow = 0.0;
        self.mix.snap(self.v[MIX] as f64);
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = (v[TOP] as i32, v[LOWCUT] as i32);
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().max(1) as f64;
        let sq = self.v[SQUASH] as f64;
        let sh = self.v[SHORTEN] as f64;
        let grit = self.v[GRIT] as f64;
        let thr = -12.0 - 24.0 * sq;
        let ratio = 2.0 + 18.0 * sq;
        let cav = coeff(self.fs, 5.0);
        let ca = coeff(self.fs, 0.5);
        let cr = coeff(self.fs, 60.0);
        let cf = coeff(self.fs, 10.0);
        let cs = coeff(self.fs, 400.0);
        // grit: an early sampler's 12-bit-ish depth and reduced rate
        let bits = (16.0 - 8.0 * grit) as u32;
        let step = 1.0 + grit * (self.fs / 22050.0 * 2.0 - 1.0).max(0.0);
        let makeup = db_to_gain(0.5 * (-thr) * (1.0 - 1.0 / ratio) * 0.6);
        let last = self.filt.len() - 1;
        let mut most = 0.0f64;
        for k in 0..n {
            let mut key = 0.0;
            for c in chans.iter() {
                key += (c[k] as f64).powi(2);
            }
            key /= nch;
            self.rms = key + cav * (self.rms - key);
            let lev = 10.0 * self.rms.max(1e-12).log10() + 3.0;
            let target = comp_gr(lev, thr, ratio, 6.0);
            let c = if target > self.gr { ca } else { cr };
            self.gr = target + c * (self.gr - target);
            most = most.max(self.gr);
            // sustain cutter: how far the slow envelope sits above the fast one = the ringing after the hit
            let a = key.sqrt();
            // both envelopes catch the hit at once; the fast one then follows the ringing down,
            // the slow one remembers how loud the hit was
            self.fast = if a > self.fast { a } else { a + cf * (self.fast - a) };
            self.slow = if a > self.slow { a } else { a + cs * (self.slow - a) };
            let ringing = (self.slow / (self.fast + 1e-9)).clamp(1.0, 30.0);
            let cut = ringing.powf(-3.0 * sh);
            let g = db_to_gain(-self.gr) * makeup * cut;
            self.hcount += 1.0;
            let take = self.hcount >= step;
            if take {
                self.hcount -= step;
            }
            let m = self.mix.next();
            let o = self.out.next();
            for (ci, ch) in chans.iter_mut().enumerate() {
                let cc = ci.min(last);
                let x = ch[k] as f64;
                let mut y = cascade(&mut self.filt[cc], x * g);
                if grit > 0.0 {
                    if take {
                        self.hold[cc] = quantise(y, bits);
                    }
                    y = self.hold[cc];
                }
                let y = 0.95 * (y / 0.95).tanh();
                ch[k] = ((x * (1.0 - m) + y * m) * o) as f32;
            }
        }
        meters[0] = most as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Is This It Kit", values: &[] },
        Preset { name: "Hard to Explain (drum machine)", values: &[(SQUASH, 0.85), (SHORTEN, 0.8), (TOP, 5500.0), (LOWCUT, 90.0), (GRIT, 0.4)] },
        Preset { name: "Room on Fire Tight", values: &[(SQUASH, 0.65), (SHORTEN, 0.6), (TOP, 8000.0)] },
        Preset { name: "Parallel Smash", values: &[(SQUASH, 1.0), (SHORTEN, 0.3), (MIX, 0.45)] },
        Preset { name: "Just Shorter Hits", values: &[(SQUASH, 0.2), (SHORTEN, 0.8), (TOP, 14000.0)] },
        Preset { name: "Lo-Fi Sampler Kit", values: &[(SQUASH, 0.7), (SHORTEN, 0.7), (TOP, 4500.0), (GRIT, 0.8)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "HARD TO EXPLAIN",
        subtitle: ("SMALL, SQUASHED DRUMS", "COMPRESS · SHORTEN · BAND-LIMIT · GRIT"),
        accent: ui::MUSTARD,
        size: (W, H),
        items: vec![

            Item::Knob { p: SQUASH, x: 90.0, y: 175.0, r: 40.0, label: "SQUASH", tip: "A fast, hard compressor: threshold and ratio together.", active: None },
            Item::Knob { p: SHORTEN, x: 230.0, y: 175.0, r: 40.0, label: "SHORTEN", tip: "Cuts the ringing after each hit, so the kit sounds tight and mechanical.", active: None },
            Item::Meter { m: 0, x: 320.0, y: 110.0, w: 160.0, h: 110.0, kind: MeterKind::GainReduction, caption: "SQUASH" },
            Item::Knob { p: GRIT, x: 560.0, y: 175.0, r: 30.0, label: "SAMPLER GRIT", tip: "Early-sampler grit: fewer bits and a lower sample rate.", active: None },
            Item::Divider { y: 262.0 },
            Item::Knob { p: LOWCUT, x: 90.0, y: 330.0, r: 22.0, label: "LOW CUT", tip: "High-pass: a smaller, tighter kit.", active: None },
            Item::Knob { p: TOP, x: 210.0, y: 330.0, r: 22.0, label: "TOP CUT", tip: "Low-pass: takes the cymbals' sizzle down.", active: None },
            Item::Knob { p: MIX, x: 400.0, y: 330.0, r: 22.0, label: "MIX", tip: "Dry vs processed (parallel compression).", active: None },
            Item::Knob { p: OUTPUT, x: 540.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Hard to Explain", class_id: *b"StrHardToExplain", vendor: "PEQUOD The Strokes",
    categories: [Fx, Dynamics]
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

    use std::f64::consts::PI;
    fn sine_amp(f: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n).map(|k| (amp * (2.0 * PI * f * k as f64 / 48000.0).sin()) as f32).collect()
    }
    fn goertzel(x: &[f32], f: f64) -> f64 {
        let w = 2.0 * PI * f / 48000.0;
        let (mut s1, mut s2) = (0.0, 0.0);
        for v in x {
            let s = *v as f64 + 2.0 * w.cos() * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        ((s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2).max(0.0)).sqrt() / x.len() as f64
    }
    fn go(p: &[f32], x: &[f32]) -> Vec<f32> {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        input.remove(0)
    }
    fn thd(p: &[f32], f: f64, amp: f64) -> f64 {
        let y = go(p, &sine_amp(f, amp, 48000));
        let a = goertzel(&y[24000..], f);
        let h: f64 = (2..7).map(|k| goertzel(&y[24000..], f * k as f64).powi(2)).sum::<f64>().sqrt();
        db(h / a.max(1e-12))
    }

    /// decaying "drum hits"
    fn hits() -> Vec<f32> {
        let mut nz = vintage::dsp::Noise::new(4);
        let mut x = vec![0.0f32; 48000];
        for h in 0..4 {
            for k in 0..12000 {
                let t = k as f64 / 48000.0;
                x[h * 12000 + k] = (0.5 * (-t / 0.12).exp() * (0.6 * (2.0 * PI * 120.0 * t).sin() + 0.4 * nz.next())) as f32;
            }
        }
        x
    }
    #[test]
    fn hits_get_shorter() {
        let x = hits();
        let a = go(&with(defaults(), &[(SHORTEN, 0.0), (SQUASH, 0.0)]), &x);
        let b = go(&with(defaults(), &[(SHORTEN, 1.0), (SQUASH, 0.0)]), &x);
        // tail (100-250 ms after the hit) relative to the attack (0-20 ms)
        let r = |y: &[f32]| db(rms(&y[12000 + 4800..12000 + 12000])) - db(rms(&y[12000..12000 + 960]));
        println!("tail vs attack: shorten 0 {:.1} dB, shorten 1 {:.1} dB", r(&a), r(&b));
        assert!(r(&a) - r(&b) > 6.0);
    }

}
