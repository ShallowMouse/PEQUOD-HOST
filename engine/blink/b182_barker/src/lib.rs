//! Barker Bus: a drum-bus processor for Travis Barker-style drums: tight, punchy and loud. PUNCH sharpens every hit's front, TIGHT cuts the ring after it, SMASH blends in a heavily compressed copy (parallel compression), and THUMP and CRACK set the kick's weight and the snare's bite. Settings estimated.

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
use vintage::fx::comp_gr;

pub const W: u32 = 640;
pub const H: u32 = 410;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "punch"]
    pub punch: FloatParam,
    #[id = "tight"]
    pub tight: FloatParam,
    #[id = "smash"]
    pub smash: FloatParam,
    #[id = "thump"]
    pub thump: FloatParam,
    #[id = "crack"]
    pub crack: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            punch: pct("Punch", 0.5),
            tight: pct("Tight", 0.3),
            smash: pct("Smash", 0.35),
            thump: db("Thump", 2.0, -6.0, 9.0),
            crack: db("Crack", 3.0, -6.0, 9.0),
            output: db("Output", -2.0, -24.0, 12.0),
        }
    }
}

pub const PUNCH: usize = 0;
pub const TIGHT: usize = 1;
pub const SMASH: usize = 2;
pub const THUMP: usize = 3;
pub const CRACK: usize = 4;
pub const OUTPUT: usize = 5;
pub const NPARAMS: usize = 6;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.punch.as_ptr(),
            self.tight.as_ptr(),
            self.smash.as_ptr(),
            self.thump.as_ptr(),
            self.crack.as_ptr(),
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
    eq: Vec<[Biquad; 3]>,
    fast: f64,
    slow: f64,
    hold: f64,
    rms: f64,
    gr: f64,
    key: (i32, i32),
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        for e in self.eq.iter_mut() {
            e[0].set_low_shelf(fs, 70.0, 0.7, self.v[THUMP] as f64);
            e[1].set_peak(fs, 200.0, 1.2, 0.5 * self.v[CRACK] as f64);
            e[2].set_peak(fs, 4500.0, 0.9, self.v[CRACK] as f64);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, eq: vec![[Biquad::identity(); 3]; ch.max(2)], fast: 0.0, slow: 0.0, hold: 0.0, rms: 0.0, gr: 0.0, key: (-99, -99), out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.eq.iter_mut().for_each(|e| e.iter_mut().for_each(|b| b.reset()));
        self.fast = 0.0;
        self.slow = 0.0;
        self.hold = 0.0;
        self.rms = 0.0;
        self.gr = 0.0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = ((v[THUMP] * 10.0) as i32, (v[CRACK] * 10.0) as i32);
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().max(1) as f64;
        let punch = self.v[PUNCH] as f64;
        let tight = self.v[TIGHT] as f64;
        let smash = self.v[SMASH] as f64;
        let cf = coeff(self.fs, 1.0);
        let cs = coeff(self.fs, 25.0);
        let ch_ = coeff(self.fs, 300.0);
        let cav = coeff(self.fs, 3.0);
        let ca = coeff(self.fs, 0.3);
        let cr = coeff(self.fs, 50.0);
        let last = self.eq.len() - 1;
        let mut most = 0.0f64;
        for k in 0..n {
            let mut key = 0.0;
            for c in chans.iter() {
                key += (c[k] as f64).powi(2);
            }
            key /= nch;
            let a = key.sqrt();
            // transient: fast envelope racing ahead of the slow one at each hit
            self.fast = a + cf * (self.fast - a);
            self.slow = a + cs * (self.slow - a);
            let front = (self.fast / (self.slow + 1e-9)).clamp(1.0, 4.0);
            // ring: how far the hit has fallen from its peak
            self.hold = if a > self.hold { a } else { a + ch_ * (self.hold - a) };
            let ring = (self.hold / (self.fast + 1e-9)).clamp(1.0, 30.0);
            let shape = front.powf(1.2 * punch) * ring.powf(-2.0 * tight);
            // parallel smash: 20:1 at -30 dB, fast
            self.rms = key + cav * (self.rms - key);
            let lev = 10.0 * self.rms.max(1e-12).log10() + 3.0;
            let t = comp_gr(lev, -30.0, 20.0, 3.0);
            let c = if t > self.gr { ca } else { cr };
            self.gr = t + c * (self.gr - t);
            most = most.max(self.gr * smash);
            let sm = db_to_gain(-self.gr + 0.5 * 30.0 * 0.95);
            let o = self.out.next();
            for (ci, chn) in chans.iter_mut().enumerate() {
                let x = chn[k] as f64;
                let e = cascade(&mut self.eq[ci.min(last)], x);
                let y = e * shape;
                let y = y + smash * (e * sm * 0.5);
                let y = 0.98 * (y / 0.98).tanh();
                chn[k] = (y * o) as f32;
            }
        }
        meters[0] = most as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Barker Punch", values: &[] },
        Preset { name: "Enema Drums (bright and tight)", values: &[(PUNCH, 0.6), (TIGHT, 0.4), (SMASH, 0.3), (CRACK, 5.0)] },
        Preset { name: "Smashed Room", values: &[(SMASH, 0.8), (PUNCH, 0.3), (TIGHT, 0.1), (OUTPUT, -6.0)] },
        Preset { name: "Untitled Album Kit (fuller)", values: &[(PUNCH, 0.4), (TIGHT, 0.2), (SMASH, 0.45), (THUMP, 4.0), (CRACK, 2.0)] },
        Preset { name: "Snare Crack", values: &[(PUNCH, 0.7), (CRACK, 7.0), (THUMP, 0.0)] },
        Preset { name: "Gentle Glue", values: &[(PUNCH, 0.2), (TIGHT, 0.0), (SMASH, 0.2), (THUMP, 0.0), (CRACK, 0.0), (OUTPUT, 0.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "BARKER BUS",
        subtitle: ("TIGHT, PUNCHY, LOUD DRUMS", "PUNCH · TIGHT · SMASH · THUMP · CRACK"),
        accent: ui::MUSTARD,
        size: (W, H),
        items: vec![

            Item::Knob { p: PUNCH, x: 90.0, y: 160.0, r: 38.0, label: "PUNCH", tip: "Sharpens the front of every hit.", active: None },
            Item::Knob { p: TIGHT, x: 220.0, y: 160.0, r: 38.0, label: "TIGHT", tip: "Cuts the ringing after each hit.", active: None },
            Item::Meter { m: 0, x: 300.0, y: 96.0, w: 160.0, h: 110.0, kind: MeterKind::GainReduction, caption: "SMASH" },
            Item::Knob { p: SMASH, x: 550.0, y: 160.0, r: 38.0, label: "SMASH", tip: "How much of a heavily compressed copy is blended in (parallel compression).", active: None },
            Item::Divider { y: 240.0 },
            Item::Knob { p: THUMP, x: 150.0, y: 305.0, r: 24.0, label: "THUMP", tip: "Kick weight (70 Hz shelf).", active: None },
            Item::Knob { p: CRACK, x: 320.0, y: 305.0, r: 24.0, label: "CRACK", tip: "Snare body (200 Hz) and bite (4.5 kHz).", active: None },
            Item::Knob { p: OUTPUT, x: 490.0, y: 305.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Barker Bus", class_id: *b"B182BarkerBusPQ1", vendor: "PEQUOD blink-182",
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
    /// harmonics (2..6) relative to the fundamental, dB
    fn thd(p: &[f32], f: f64, amp: f64) -> f64 {
        let y = go(p, &sine_amp(f, amp, 48000));
        let a = goertzel(&y[24000..], f);
        let h: f64 = (2..7).map(|k| goertzel(&y[24000..], f * k as f64).powi(2)).sum::<f64>().sqrt();
        db(h / a.max(1e-12))
    }

    fn hits() -> Vec<f32> {
        let mut nz = vintage::dsp::Noise::new(4);
        let mut x = vec![0.0f32; 48000];
        for h in 0..4 {
            for k in 0..12000 {
                let t = k as f64 / 48000.0;
                x[h * 12000 + k] = (0.4 * (1.0 - (-t / 0.002).exp()) * (-t / 0.15).exp() * (0.6 * (2.0 * PI * 150.0 * t).sin() + 0.4 * nz.next())) as f32;
            }
        }
        x
    }
    #[test]
    fn punch_and_tight() {
        let x = hits();
        let base = with(defaults(), &[(PUNCH, 0.0), (TIGHT, 0.0), (SMASH, 0.0), (THUMP, 0.0), (CRACK, 0.0), (OUTPUT, 0.0)]);
        let a = go(&base, &x);
        let mut p2 = base.clone();
        p2[TIGHT] = 1.0;
        let b = go(&p2, &x);
        let r = |y: &[f32]| db(rms(&y[12000 + 4800..12000 + 9600])) - db(rms(&y[12000..12000 + 960]));
        println!("tail vs attack: tight 0 {:.1} dB, tight 1 {:.1} dB", r(&a), r(&b));
        assert!(r(&a) - r(&b) > 8.0);
    }

}
