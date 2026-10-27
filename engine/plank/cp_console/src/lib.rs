//! Plank Console: one channel of the desk Conny Plank built himself and kept changing until 1987. Two documented quirks: each EQ band has its own boost and cut frequencies, so the sound changes character as a knob passes centre; and he ran the audio rails at 35 V instead of the usual 48 V, which made the desk run hotter. The frequencies here are estimates.

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


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Rail { #[name = "35 V (Plank)"] Plank, #[name = "48 V (stock)"] Stock }
// (boost Hz, boost Q or shelf, cut Hz, cut Q or shelf) - estimated "preferred settings"
const LOW: (f64, f64) = (70.0, 220.0);
const MID: (f64, f64) = (2800.0, 550.0);
const HIGH: (f64, f64) = (11000.0, 4500.0);


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "lowcut"]
    pub low_cut: FloatParam,
    #[id = "loweq"]
    pub low_eq: FloatParam,
    #[id = "mideq"]
    pub mid_eq: FloatParam,
    #[id = "hieq"]
    pub hi_eq: FloatParam,
    #[id = "rail"]
    pub rail: EnumParam<Rail>,
    #[id = "fader"]
    pub fader: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            gain: db("Gain", 0.0, -12.0, 30.0),
            low_cut: hz("Low Cut", 20.0, 20.0, 400.0),
            low_eq: db("Low", 0.0, -15.0, 15.0),
            mid_eq: db("Mid", 0.0, -15.0, 15.0),
            hi_eq: db("High", 0.0, -15.0, 15.0),
            rail: EnumParam::new("Rails", Rail::Plank),
            fader: db("Fader", 0.0, -30.0, 10.0),
        }
    }
}

pub const GAIN: usize = 0;
pub const LOWCUT: usize = 1;
pub const LOWEQ: usize = 2;
pub const MIDEQ: usize = 3;
pub const HIEQ: usize = 4;
pub const RAIL: usize = 5;
pub const FADER: usize = 6;
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
            self.low_cut.as_ptr(),
            self.low_eq.as_ptr(),
            self.mid_eq.as_ptr(),
            self.hi_eq.as_ptr(),
            self.rail.as_ptr(),
            self.fader.as_ptr(),
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
    hpf: [[Biquad; 2]; 2],
    eq: [[Biquad; 3]; 2],
    dc: [DcBlock; 2],
    key: [f64; 4],
    gain: Smooth,
    fader: Smooth,
    rail: f64,
}

impl Dsp {
    fn set_eq(&mut self, lc: f64, lo: f64, mid: f64, hi: f64) {
        for c in 0..2 {
            if lc <= 20.5 {
                self.hpf[c] = [Biquad::identity(); 2];
            } else {
                self.hpf[c][0].set_highpass(self.fs, lc, 0.54);
                self.hpf[c][1].set_highpass(self.fs, lc, 1.31);
            }
            // boost and cut use different curves: the character flips through centre
            if lo >= 0.0 { self.eq[c][0].set_low_shelf(self.fs, LOW.0, 0.7, lo) } else { self.eq[c][0].set_peak(self.fs, LOW.1, 1.1, lo) }
            if mid >= 0.0 { self.eq[c][1].set_peak(self.fs, MID.0, 0.7, mid) } else { self.eq[c][1].set_peak(self.fs, MID.1, 1.6, mid) }
            if hi >= 0.0 { self.eq[c][2].set_high_shelf(self.fs, HIGH.0, 0.7, hi) } else { self.eq[c][2].set_high_shelf(self.fs, HIGH.1, 0.6, hi) }
        }
        self.key = [lc, lo, mid, hi];
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut d = Self {
            fs,
            os: [Oversampler::new(), Oversampler::new()],
            hpf: [[Biquad::identity(); 2]; 2],
            eq: [[Biquad::identity(); 3]; 2],
            dc: [DcBlock::new(fs), DcBlock::new(fs)],
            key: [-1.0; 4],
            gain: Smooth::new(fs, 20.0, 1.0),
            fader: Smooth::new(fs, 20.0, 1.0),
            rail: 35.0,
        };
        d.set_eq(20.0, 0.0, 0.0, 0.0);
        d
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.os[c].reset();
            self.hpf[c].iter_mut().for_each(|b| b.reset());
            self.eq[c].iter_mut().for_each(|b| b.reset());
            self.dc[c].reset();
        }
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn set_params(&mut self, v: &[f32]) {
        let k = [v[LOWCUT] as f64, v[LOWEQ] as f64, v[MIDEQ] as f64, v[HIEQ] as f64];
        if k.iter().zip(self.key.iter()).any(|(a, b)| (a - b).abs() > 1e-3) {
            self.set_eq(k[0], k[1], k[2], k[3]);
        }
        self.gain.set(db_to_gain(v[GAIN] as f64));
        self.fader.set(db_to_gain(v[FADER] as f64));
        self.rail = if v[RAIL] > 0.5 { 48.0 } else { 35.0 };
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let mut peak = 0.0f64;
        // headroom scales with the rail voltage; nominal level sits at 0.25 of a 48 V swing
        let head = 4.0 * self.rail / 48.0;
        // lower rails also sit the class-A stages closer to their bend: a little more even-order colour
        let bias = 0.12 * (48.0 - self.rail) / 13.0 + 0.03;
        for k in 0..n {
            let g = self.gain.next();
            let f = self.fader.next();
            for (c, ch) in chans.iter_mut().enumerate().take(2) {
                let x = ch[k] as f64 * g;
                let x = cascade(&mut self.hpf[c], x);
                let y = self.os[c].process(x, |s| head * 0.25 * tube(s / (head * 0.25), bias));
                let y = self.dc[c].process(y);
                let y = cascade(&mut self.eq[c], y) * f;
                peak = peak.max(y.abs());
                ch[k] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Flat Channel (35 V)", values: &[] },
        Preset { name: "Hot Drums Into the Desk", values: &[(GAIN, 14.0), (LOWEQ, 4.0), (HIEQ, 3.0), (FADER, -8.0)] },
        Preset { name: "Motorik Kick", values: &[(LOWEQ, 6.0), (MIDEQ, -6.0), (LOWCUT, 30.0)] },
        Preset { name: "Snare Crack", values: &[(MIDEQ, 5.0), (LOWEQ, -4.0), (LOWCUT, 90.0)] },
        Preset { name: "Synth Bass Weight", values: &[(LOWEQ, 5.0), (HIEQ, -3.0)] },
        Preset { name: "Guitar Bite", values: &[(MIDEQ, 6.0), (LOWCUT, 120.0), (HIEQ, -2.0)] },
        Preset { name: "Vocal Air", values: &[(HIEQ, 6.0), (MIDEQ, -3.0), (LOWCUT, 100.0)] },
        Preset { name: "Dark and Warm", values: &[(HIEQ, -8.0), (LOWEQ, 3.0), (GAIN, 8.0), (FADER, -4.0)] },
        Preset { name: "Stock 48 V Comparison", values: &[(RAIL, 1.0)] },
        Preset { name: "Driven Machine Drums", values: &[(GAIN, 24.0), (FADER, -14.0), (MIDEQ, 4.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "PLANK CONSOLE",
        subtitle: ("AFTER CONNY PLANK'S OWN DESK", "BOOST/CUT SPLIT EQ · 35 V RAILS"),
        accent: ui::STEEL,
        size: (W, H),
        items: vec![

            Item::Knob { p: GAIN, x: 80.0, y: 160.0, r: 36.0, label: "GAIN", tip: "Input gain. Push it to drive the discrete stages.", active: None },
            Item::Knob { p: LOWEQ, x: 200.0, y: 160.0, r: 32.0, label: "LOW", tip: "Boost = a low shelf (about 70 Hz); cut = a dip higher up (about 220 Hz). Frequencies estimated.", active: None },
            Item::Knob { p: MIDEQ, x: 310.0, y: 160.0, r: 32.0, label: "MID", tip: "Boost = a broad presence lift (about 2.8 kHz); cut = a narrower scoop (about 550 Hz). Frequencies estimated.", active: None },
            Item::Knob { p: HIEQ, x: 420.0, y: 160.0, r: 32.0, label: "HIGH", tip: "Boost = air shelf (about 11 kHz); cut = a lower, wider shelf (about 4.5 kHz). Frequencies estimated.", active: None },
            Item::Knob { p: FADER, x: 550.0, y: 160.0, r: 36.0, label: "FADER", tip: "Channel level.", active: None },
            Item::Divider { y: 264.0 },
            Item::Knob { p: LOWCUT, x: 120.0, y: 330.0, r: 22.0, label: "LOW CUT", tip: "High-pass filter.", active: None },
            Item::Switch { p: RAIL, x: 300.0, y: 330.0, labels: ["35 V", "48 V"], tip: "Plank ran the audio at 35 V, with phantom power on its own supply: less headroom, a hotter, warmer channel. 48 V is the stock comparison." },
            Item::Lamp { m: 0, x: 470.0, y: 330.0, label: "LEVEL", lo: 0.1, hi: 1.0 },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Plank Console", class_id: *b"CPPlankConsolePQ", vendor: "PEQUOD Plank",
    categories: [Fx, Eq]
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

    fn gain_at(p: &[f32], f: f64) -> f64 {
        let mut input = vec![sine(f, 0.05, 48000)];
        run(p, &mut input);
        db(rms(&input[0][24000..]) / (0.05 / 2f64.sqrt()))
    }
    #[test]
    fn boost_and_cut_use_different_frequencies() {
        let b = defaults();
        let flat = gain_at(&b, 1000.0);
        let boost = |f| gain_at(&with(b.clone(), &[(MIDEQ, 10.0)]), f) - gain_at(&b, f);
        let cut = |f| gain_at(&with(b.clone(), &[(MIDEQ, -10.0)]), f) - gain_at(&b, f);
        println!("flat {flat:.2}; boost @2.8k {:.1} @550 {:.1}; cut @550 {:.1} @2.8k {:.1}", boost(2800.0), boost(550.0), cut(550.0), cut(2800.0));
        assert!(flat.abs() < 0.5);
        assert!(boost(2800.0) > 8.0 && boost(550.0) < 3.0);
        assert!(cut(550.0) < -8.0 && cut(2800.0) > -3.0);
    }
    #[test]
    fn lower_rails_clip_sooner() {
        let thd = |rail: f32| {
            let p = with(defaults(), &[(RAIL, rail), (GAIN, 9.0), (FADER, -9.0)]);
            let mut input = vec![sine(100.0, 0.25, 48000)];
            run(&p, &mut input);
            let x = &input[0][24000..];
            // 3rd harmonic level relative to fundamental (Goertzel-ish)
            let tone = |f: f64| {
                let (mut s, mut c) = (0.0, 0.0);
                for (k, v) in x.iter().enumerate() {
                    let ph = 2.0 * std::f64::consts::PI * f * k as f64 / 48000.0;
                    s += *v as f64 * ph.sin();
                    c += *v as f64 * ph.cos();
                }
                (s * s + c * c).sqrt()
            };
            db((tone(200.0) + tone(300.0)) / tone(100.0))
        };
        let (a, b) = (thd(0.0), thd(1.0));
        println!("harmonics 35 V {a:.1} dB, 48 V {b:.1} dB");
        assert!(a > b + 2.0);
    }

}
