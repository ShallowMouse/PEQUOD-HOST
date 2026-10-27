//! Gold Star Desk: one input of the 12-input valve desk Bill Putnam built (with Dave Gold) for Gold Star Studio A. EQ steps follow Larry Levine's description: low 60 or 100 Hz, cut 3/6 or boost 3/6/9 dB; high 3, 5 or 10 kHz boosted in 3 dB steps to 15 dB.

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
pub enum LowF { #[name = "60 Hz"] F60, #[name = "100 Hz"] F100 }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum LowStep { #[name = "-6 dB"] M6, #[name = "-3 dB"] M3, #[name = "0 dB"] Z, #[name = "+3 dB"] P3, #[name = "+6 dB"] P6, #[name = "+9 dB"] P9 }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum HiF { #[name = "3 kHz"] F3, #[name = "5 kHz"] F5, #[name = "10 kHz"] F10 }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum HiStep { #[name = "0 dB"] Z, #[name = "+3 dB"] P3, #[name = "+6 dB"] P6, #[name = "+9 dB"] P9, #[name = "+12 dB"] P12, #[name = "+15 dB"] P15 }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "lowf"]
    pub low_f: EnumParam<LowF>,
    #[id = "low"]
    pub low: EnumParam<LowStep>,
    #[id = "hif"]
    pub hi_f: EnumParam<HiF>,
    #[id = "high"]
    pub high: EnumParam<HiStep>,
    #[id = "echo"]
    pub echo: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            gain: db("Mic Gain", 0.0, -12.0, 24.0),
            low_f: EnumParam::new("Low Freq", LowF::F100),
            low: EnumParam::new("Low", LowStep::Z),
            hi_f: EnumParam::new("High Freq", HiF::F5),
            high: EnumParam::new("High", HiStep::Z),
            echo: db("Fader", 0.0, -30.0, 6.0),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const GAIN: usize = 0;
pub const LOWF: usize = 1;
pub const LOW: usize = 2;
pub const HIF: usize = 3;
pub const HIGH: usize = 4;
pub const ECHO: usize = 5;
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
            self.low_f.as_ptr(),
            self.low.as_ptr(),
            self.hi_f.as_ptr(),
            self.high.as_ptr(),
            self.echo.as_ptr(),
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
    lo: [Biquad; 2],
    hi: [Biquad; 2],
    xfmr: [Biquad; 2],
    dc: [DcBlock; 2],
    gain: Smooth,
    fader: Smooth,
    out: Smooth,
    key: (usize, usize, usize, usize),
}

impl Dsp {
    fn set_eq(&mut self, lf: usize, ls: usize, hf: usize, hs: usize) {
        let fl = [60.0, 100.0][lf];
        let gl = [-6.0, -3.0, 0.0, 3.0, 6.0, 9.0][ls];
        let fh = [3000.0, 5000.0, 10000.0][hf];
        let gh = 3.0 * hs as f64;
        for b in &mut self.lo {
            // gentle passive-style shelf, corner set so the step lands near the marked frequency
            b.set_low_shelf(self.fs, fl * 1.6, 0.6, gl);
        }
        for b in &mut self.hi {
            // broad presence-style bell
            b.set_peak(self.fs, fh, 0.75, gh);
        }
        self.key = (lf, ls, hf, hs);
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut xfmr = [Biquad::identity(); 2];
        for b in &mut xfmr {
            b.set_highpass(fs, 18.0, 0.6);
        }
        let mut d = Self {
            fs,
            os: [Oversampler::new(), Oversampler::new()],
            lo: [Biquad::identity(); 2],
            hi: [Biquad::identity(); 2],
            xfmr,
            dc: [DcBlock::new(fs), DcBlock::new(fs)],
            gain: Smooth::new(fs, 20.0, 1.0),
            fader: Smooth::new(fs, 20.0, 1.0),
            out: Smooth::new(fs, 20.0, 1.0),
            key: (9, 9, 9, 9),
        };
        d.set_eq(1, 2, 1, 0);
        d
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.os[c].reset();
            self.lo[c].reset();
            self.hi[c].reset();
            self.xfmr[c].reset();
            self.dc[c].reset();
        }
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn set_params(&mut self, v: &[f32]) {
        let k = (v[LOWF].round() as usize, v[LOW].round() as usize, v[HIF].round() as usize, v[HIGH].round() as usize);
        if k != self.key {
            self.set_eq(k.0.min(1), k.1.min(5), k.2.min(2), k.3.min(5));
        }
        self.gain.set(db_to_gain(v[GAIN] as f64));
        self.fader.set(db_to_gain(v[ECHO] as f64));
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let mut peak = 0.0f64;
        for k in 0..n {
            let g = self.gain.next();
            let f = self.fader.next();
            let o = self.out.next();
            for (c, ch) in chans.iter_mut().enumerate().take(2) {
                let x = ch[k] as f64 * g;
                // valve mic amp: a triode stage driven harder as the gain rises
                let y = self.os[c].process(x, |s| 0.5 * tube(2.0 * s, 0.25));
                let y = self.dc[c].process(y);
                let y = self.xfmr[c].process(y);
                let y = self.hi[c].process(self.lo[c].process(y));
                let y = y * f * o;
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
        Preset { name: "Flat Valve Channel", values: &[] },
        Preset { name: "Acoustic Guitars (strummed wall)", values: &[(HIF, 1.0), (HIGH, 2.0), (LOWF, 1.0), (LOW, 1.0)] },
        Preset { name: "Pianos Bright", values: &[(HIF, 1.0), (HIGH, 3.0), (LOW, 2.0)] },
        Preset { name: "Bass Amp Weight", values: &[(LOWF, 0.0), (LOW, 4.0), (HIF, 0.0), (HIGH, 0.0)] },
        Preset { name: "Hal Blaine Tom Fill", values: &[(GAIN, 6.0), (LOWF, 1.0), (LOW, 3.0), (HIF, 0.0), (HIGH, 2.0)] },
        Preset { name: "Lead Vocal (U47)", values: &[(LOWF, 1.0), (LOW, 1.0), (HIF, 2.0), (HIGH, 2.0)] },
        Preset { name: "Strings Air", values: &[(HIF, 2.0), (HIGH, 3.0), (LOWF, 1.0), (LOW, 1.0)] },
        Preset { name: "Driven Input", values: &[(GAIN, 15.0), (OUTPUT, -9.0)] },
        Preset { name: "Castanets Bite", values: &[(HIF, 0.0), (HIGH, 4.0), (LOWF, 1.0), (LOW, 0.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "GOLD STAR DESK",
        subtitle: ("AFTER THE PUTNAM / GOLD 12-INPUT DESK", "GOLD STAR STUDIO A · VALVE"),
        accent: ui::STEEL,
        size: (W, H),
        items: vec![

            Item::Knob { p: GAIN, x: 90.0, y: 160.0, r: 40.0, label: "MIC GAIN", tip: "Valve mic amplifier gain. Turned up, the triode starts to round and thicken the sound before the EQ.", active: None },
            Item::Meter { m: 0, x: 210.0, y: 92.0, w: 220.0, h: 140.0, kind: MeterKind::Vu, caption: "OUTPUT" },
            Item::Knob { p: ECHO, x: 550.0, y: 160.0, r: 40.0, label: "FADER", tip: "Channel fader (rotary, as on the desk).", active: None },
            Item::Divider { y: 264.0 },
            Item::Selector { p: LOWF, x: 70.0, y: 330.0, label: "LOW FREQ", positions: &["60", "100"], tip: "Low EQ frequency: 60 or 100 Hz." },
            Item::Selector { p: LOW, x: 180.0, y: 330.0, label: "LOW", positions: &["-6", "-3", "0", "+3", "+6", "+9"], tip: "Low: cut 3 or 6 dB, or boost 3, 6 or 9 dB." },
            Item::Selector { p: HIF, x: 330.0, y: 330.0, label: "HIGH FREQ", positions: &["3k", "5k", "10k"], tip: "High EQ frequency: 3, 5 or 10 kHz." },
            Item::Selector { p: HIGH, x: 445.0, y: 330.0, label: "HIGH", positions: &["0", "+3", "+6", "+9", "+12", "+15"], tip: "High boost in 3 dB steps, up to 15 dB (boost only, as on the desk)." },
            Item::Knob { p: OUTPUT, x: 570.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Gold Star Desk", class_id: *b"PSGoldStarDeskPQ", vendor: "PEQUOD Spector",
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
        let n = 48000;
        let mut input = vec![sine(f, 0.05, n)];
        run(p, &mut input);
        db(rms(&input[0][24000..]) / (0.05 / 2f64.sqrt()))
    }
    #[test]
    fn eq_steps_land_near_marked_values() {
        let base = defaults();
        let flat = gain_at(&base, 1000.0);
        let lo9 = gain_at(&with(base.clone(), &[(LOWF, 0.0), (LOW, 5.0)]), 40.0) - gain_at(&base, 40.0);
        let lo6c = gain_at(&with(base.clone(), &[(LOWF, 1.0), (LOW, 0.0)]), 60.0) - gain_at(&base, 60.0);
        let hi15 = gain_at(&with(base.clone(), &[(HIF, 1.0), (HIGH, 5.0)]), 5000.0) - gain_at(&base, 5000.0);
        let hi3 = gain_at(&with(base.clone(), &[(HIF, 2.0), (HIGH, 1.0)]), 10000.0) - gain_at(&base, 10000.0);
        println!("flat {flat:.2}  lo+9@40 {lo9:.2}  lo-6@60 {lo6c:.2}  hi+15@5k {hi15:.2}  hi+3@10k {hi3:.2}");
        assert!(flat.abs() < 1.0);
        assert!((lo9 - 9.0).abs() < 1.5 && (lo6c + 6.0).abs() < 1.5 && (hi15 - 15.0).abs() < 1.5 && (hi3 - 3.0).abs() < 1.0);
    }

}
