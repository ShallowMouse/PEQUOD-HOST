//! Phone Line: a voice sent down a telephone line, as Ian Curtis's vocal on Insight was, to put it at a distance. Carbon microphone, 300-3400 Hz line, earpiece; mains hum and line noise. Estimated from telephone standards of the period.

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
pub enum Kind { #[name = "GPO Carbon"] Carbon, #[name = "Clean Line"] Clean, #[name = "Long Distance"] Trunk }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "kind"]
    pub kind: EnumParam<Kind>,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "noise"]
    pub noise: FloatParam,
    #[id = "hum"]
    pub hum: FloatParam,
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
            kind: EnumParam::new("Line", Kind::Carbon),
            drive: pct("Carbon Grit", 0.4),
            noise: pct("Line Noise", 0.1),
            hum: pct("Hum", 0.05),
            mix: pct("Mix", 1.0),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const KIND: usize = 0;
pub const DRIVE: usize = 1;
pub const NOISE: usize = 2;
pub const HUM: usize = 3;
pub const MIX: usize = 4;
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
            self.kind.as_ptr(),
            self.drive.as_ptr(),
            self.noise.as_ptr(),
            self.hum.as_ptr(),
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

const BANDS: [(f64, f64, f64, f64); 3] = [
    // (high-pass, low-pass, resonance Hz, resonance dB)
    (300.0, 3400.0, 1800.0, 6.0),
    (300.0, 3400.0, 1000.0, 0.0),
    (400.0, 2800.0, 1400.0, 4.0),
];

pub struct Dsp {
    fs: f64,
    kind: usize,
    hp: [Biquad; 2],
    lp: [Biquad; 2],
    pk: Biquad,
    noise: Noise,
    nf: OnePole,
    hum_ph: f64,
    drive: f64,
    nz: f64,
    hum: f64,
    mix: Smooth,
    out: Smooth,
}

impl Dsp {
    fn set_kind(&mut self, k: usize) {
        let (h, l, r, g) = BANDS[k];
        self.hp[0].set_highpass(self.fs, h, 0.54);
        self.hp[1].set_highpass(self.fs, h, 1.31);
        self.lp[0].set_lowpass(self.fs, l, 0.54);
        self.lp[1].set_lowpass(self.fs, l, 1.31);
        self.pk.set_peak(self.fs, r, 1.2, g);
        self.kind = k;
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut nf = OnePole::default();
        nf.set_lowpass(fs, 2500.0);
        let mut d = Self {
            fs,
            kind: 9,
            hp: [Biquad::identity(); 2],
            lp: [Biquad::identity(); 2],
            pk: Biquad::identity(),
            noise: Noise::new(4),
            nf,
            hum_ph: 0.0,
            drive: 0.4,
            nz: 0.1,
            hum: 0.05,
            mix: Smooth::new(fs, 20.0, 1.0),
            out: Smooth::new(fs, 20.0, 1.0),
        };
        d.set_kind(0);
        d
    }
    fn reset(&mut self) {
        self.hp.iter_mut().for_each(|b| b.reset());
        self.lp.iter_mut().for_each(|b| b.reset());
        self.pk.reset();
    }
    fn set_params(&mut self, v: &[f32]) {
        let k = (v[KIND].round() as usize).min(2);
        if k != self.kind {
            self.set_kind(k);
        }
        self.drive = v[DRIVE] as f64;
        self.nz = v[NOISE] as f64;
        self.hum = v[HUM] as f64;
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let mut peak = 0.0f64;
        let carbon = self.kind == 0;
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            // carbon mic: level-dependent, asymmetric, gritty
            let dg = 1.0 + 10.0 * self.drive;
            let y = if carbon { ((x * dg + 0.15).tanh() - 0.15f64.tanh()) / dg * 1.3 } else { (x * dg).tanh() / dg };
            let y = self.pk.process(y);
            let y = cascade(&mut self.lp, cascade(&mut self.hp, y));
            self.hum_ph = (self.hum_ph + 50.0 / self.fs).fract();
            let ph = 2.0 * std::f64::consts::PI * self.hum_ph;
            let hum = (ph.sin() + 0.4 * (3.0 * ph).sin()) * 0.03 * self.hum;
            let nz = self.nf.lp(self.noise.next()) * 0.05 * self.nz;
            let y = y * 1.8 + hum + nz;
            let mx = self.mix.next();
            let o = self.out.next();
            for c in chans.iter_mut() {
                let z = (c[k] as f64 * (1.0 - mx) + y * mx) * o;
                peak = peak.max(z.abs());
                c[k] = z as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Insight Vocal", values: &[(KIND, 0.0), (DRIVE, 0.35), (NOISE, 0.08), (HUM, 0.03)] },
        Preset { name: "Clean Phone", values: &[(KIND, 1.0), (DRIVE, 0.1), (NOISE, 0.02), (HUM, 0.0)] },
        Preset { name: "Long Distance Call", values: &[(KIND, 2.0), (DRIVE, 0.4), (NOISE, 0.4), (HUM, 0.1)] },
        Preset { name: "Blend Under the Voice", values: &[(KIND, 0.0), (MIX, 0.4)] },
        Preset { name: "Broken Line", values: &[(KIND, 0.0), (DRIVE, 0.9), (NOISE, 0.5), (HUM, 0.3)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "PHONE LINE",
        subtitle: ("A VOICE DOWN THE TELEPHONE", "CARBON MIC · 300-3400 HZ"),
        accent: ui::ROSE,
        size: (W, H),
        items: vec![

            Item::Selector { p: KIND, x: 110.0, y: 160.0, label: "LINE", positions: &["CARBON", "CLEAN", "TRUNK"], tip: "The handset and line: a GPO carbon handset, a clean line, or a long-distance trunk call." },
            Item::Knob { p: DRIVE, x: 250.0, y: 160.0, r: 40.0, label: "CARBON GRIT", tip: "How hard the carbon mic is driven.", active: None },
            Item::Meter { m: 0, x: 360.0, y: 100.0, w: 220.0, h: 120.0, kind: MeterKind::Vu, caption: "OUTPUT" },
            Item::Divider { y: 264.0 },
            Item::Knob { p: NOISE, x: 100.0, y: 330.0, r: 22.0, label: "LINE NOISE", tip: "Hiss on the line.", active: None },
            Item::Knob { p: HUM, x: 220.0, y: 330.0, r: 22.0, label: "HUM", tip: "50 Hz mains hum (UK).", active: None },
            Item::Knob { p: MIX, x: 380.0, y: 330.0, r: 22.0, label: "MIX", tip: "Direct voice vs the phone line.", active: None },
            Item::Knob { p: OUTPUT, x: 540.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Phone Line", class_id: *b"MHPhoneLinePQD01", vendor: "PEQUOD Hannett",
    categories: [Fx, Distortion]
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
    fn telephone_band() {
        let lvl = |f: f64| {
            let p = with(defaults(), &[(NOISE, 0.0), (HUM, 0.0), (DRIVE, 0.0)]);
            let mut input = vec![sine(f, 0.05, 24000)];
            run(&p, &mut input);
            rms(&input[0][12000..])
        };
        let (a, b, c) = (lvl(100.0), lvl(1000.0), lvl(8000.0));
        println!("100 Hz {:.1} dB, 8 kHz {:.1} dB rel 1 kHz", db(a / b), db(c / b));
        assert!(db(a / b) < -15.0 && db(c / b) < -30.0);
    }

}
