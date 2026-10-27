//! Car Radio: hear the mix the way Gold Star's team did, through an AM broadcast chain and a car or transistor speaker (Dave Gold built a small transmitter so mixes could be checked on a car radio). STUDIO/RADIO flips between the two.

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
pub enum Speaker { #[name = "Car Dash"] Car, #[name = "Transistor"] Transistor, #[name = "Kitchen Table"] Kitchen }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Check { #[name = "Studio"] Studio, #[name = "Radio"] Radio }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "check"]
    pub check: EnumParam<Check>,
    #[id = "speaker"]
    pub speaker: EnumParam<Speaker>,
    #[id = "band"]
    pub band: FloatParam,
    #[id = "station"]
    pub station: FloatParam,
    #[id = "static"]
    pub noise: FloatParam,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            check: EnumParam::new("Listen", Check::Radio),
            speaker: EnumParam::new("Speaker", Speaker::Car),
            band: hz("Bandwidth", 4500.0, 2500.0, 8000.0),
            station: pct("Station Limiting", 0.6),
            noise: pct("Static", 0.08),
            drive: pct("Speaker Drive", 0.3),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const CHECK: usize = 0;
pub const SPEAKER: usize = 1;
pub const BAND: usize = 2;
pub const STATION: usize = 3;
pub const STATIC: usize = 4;
pub const DRIVE: usize = 5;
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
            self.check.as_ptr(),
            self.speaker.as_ptr(),
            self.band.as_ptr(),
            self.station.as_ptr(),
            self.noise.as_ptr(),
            self.drive.as_ptr(),
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

const SPK: [(f64, f64, f64, f64); 3] = [
    // (high-pass Hz, resonance Hz, resonance dB, low-pass Hz)
    (140.0, 900.0, 4.0, 5500.0),
    (300.0, 1500.0, 6.0, 4500.0),
    (200.0, 1100.0, 3.0, 6000.0),
];

pub struct Dsp {
    fs: f64,
    radio: bool,
    spk: usize,
    band: f64,
    lp: [Biquad; 2],
    hp: Biquad,
    s_hp: Biquad,
    s_pk: Biquad,
    s_lp: Biquad,
    agc: f64,
    lim: f64,
    station: f64,
    noise: Noise,
    nf: OnePole,
    crackle: f64,
    stat: f64,
    drive: f64,
    out: Smooth,
    fade: f64,
}

impl Dsp {
    fn set_speaker(&mut self, s: usize) {
        let (h, r, g, l) = SPK[s];
        self.s_hp.set_highpass(self.fs, h, 0.9);
        self.s_pk.set_peak(self.fs, r, 1.3, g);
        self.s_lp.set_lowpass(self.fs, l, 0.8);
        self.spk = s;
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut hp = Biquad::identity();
        hp.set_highpass(fs, 50.0, 0.7);
        let mut nf = OnePole::default();
        nf.set_lowpass(fs, 3000.0);
        let mut d = Self {
            fs,
            radio: true,
            spk: 9,
            band: 0.0,
            lp: [Biquad::identity(); 2],
            hp,
            s_hp: Biquad::identity(),
            s_pk: Biquad::identity(),
            s_lp: Biquad::identity(),
            agc: 1.0,
            lim: 1.0,
            station: 0.6,
            noise: Noise::new(91),
            nf,
            crackle: 0.0,
            stat: 0.08,
            drive: 0.3,
            out: Smooth::new(fs, 20.0, 1.0),
            fade: 1.0,
        };
        d.set_speaker(0);
        d
    }
    fn reset(&mut self) {
        self.lp.iter_mut().for_each(|b| b.reset());
        for b in [&mut self.hp, &mut self.s_hp, &mut self.s_pk, &mut self.s_lp] {
            b.reset();
        }
        self.agc = 1.0;
        self.lim = 1.0;
        self.crackle = 0.0;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.radio = v[CHECK] > 0.5;
        let s = (v[SPEAKER].round() as usize).min(2);
        if s != self.spk {
            self.set_speaker(s);
        }
        let b = v[BAND] as f64;
        if (b - self.band).abs() > 1.0 {
            self.band = b;
            // AM receiver IF/audio response: 4th-order low-pass
            self.lp[0].set_lowpass(self.fs, b, 0.54);
            self.lp[1].set_lowpass(self.fs, b, 1.31);
        }
        self.station = v[STATION] as f64;
        self.stat = v[STATIC] as f64;
        self.drive = v[DRIVE] as f64;
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let mut peak = 0.0f64;
        let agc_a = coeff(self.fs, 2000.0);
        let step = 1.0 / (0.01 * self.fs);
        for k in 0..n {
            let target = if self.radio { 1.0 } else { 0.0 };
            self.fade += (target - self.fade).clamp(-step, step);
            let o = self.out.next();
            if self.fade <= 0.0 {
                for c in chans.iter_mut() {
                    let y = c[k] as f64 * o;
                    peak = peak.max(y.abs());
                    c[k] = y as f32;
                }
                continue;
            }
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            // station: slow AGC riding the level up, then a fast peak limiter
            let st = self.station;
            let lvl = x.abs();
            let want = (0.25 / (lvl * self.agc).max(0.02)).clamp(0.3, 4.0);
            let want = 1.0 + st * (want - 1.0);
            self.agc = want + agc_a * (self.agc - want);
            let y = x * self.agc;
            let lim_t = if y.abs() * self.lim > 0.5 { 0.5 / y.abs().max(1e-9) } else { 1.0 };
            self.lim = if lim_t < self.lim { lim_t } else { self.lim + (1.0 - self.lim) * 0.0005 };
            let y = y * (1.0 - st + st * self.lim);
            // AM channel
            let y = cascade(&mut self.lp, self.hp.process(y));
            // static: hiss plus the odd crackle
            let hiss = self.nf.lp(self.noise.next()) * 0.08 * self.stat;
            if self.noise.next() > 1.0 - 0.0004 * self.stat {
                self.crackle = self.noise.next() * 0.6 * self.stat;
            }
            self.crackle *= 0.995;
            let y = y + hiss + self.crackle * self.noise.next().abs();
            // envelope detector: slight asymmetric bend
            let y = y + 0.06 * y * y.abs();
            // speaker
            let dg = 1.0 + 8.0 * self.drive;
            let y = self.s_lp.process(self.s_pk.process(self.s_hp.process(y)));
            let y = (y * dg).tanh() / dg * 1.6;
            let f = self.fade;
            for c in chans.iter_mut() {
                let z = (c[k] as f64 * (1.0 - f) + y * f) * o;
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
        Preset { name: "1963 Car Radio", values: &[] },
        Preset { name: "Pocket Transistor", values: &[(SPEAKER, 1.0), (BAND, 3500.0), (DRIVE, 0.4)] },
        Preset { name: "Kitchen Table Radio", values: &[(SPEAKER, 2.0), (BAND, 5000.0), (STATION, 0.5)] },
        Preset { name: "Strong Local Station", values: &[(STATIC, 0.0), (BAND, 6000.0), (STATION, 0.8)] },
        Preset { name: "Weak Signal at Night", values: &[(STATIC, 0.6), (BAND, 3000.0)] },
        Preset { name: "Studio (bypass)", values: &[(CHECK, 0.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "CAR RADIO",
        subtitle: ("CHECK THE MIX ON AM RADIO", "BROADCAST CHAIN · SMALL SPEAKER"),
        accent: ui::ROSE,
        size: (W, H),
        items: vec![

            Item::Switch { p: CHECK, x: 110.0, y: 160.0, labels: ["STUDIO", "RADIO"], tip: "Flip between the studio mix and the radio. The point is checking: does the record still jump out of a car speaker?" },
            Item::Selector { p: SPEAKER, x: 250.0, y: 160.0, label: "SPEAKER", positions: &["CAR", "POCKET", "KITCHEN"], tip: "Which radio: a car dash speaker, a pocket transistor, a kitchen table set (all estimated)." },
            Item::Meter { m: 0, x: 360.0, y: 100.0, w: 220.0, h: 120.0, kind: MeterKind::Vu, caption: "OUTPUT" },
            Item::Divider { y: 264.0 },
            Item::Knob { p: BAND, x: 80.0, y: 330.0, r: 22.0, label: "BANDWIDTH", tip: "How much top end the AM receiver passes.", active: None },
            Item::Knob { p: STATION, x: 190.0, y: 330.0, r: 22.0, label: "STATION", tip: "The broadcaster's level riding and peak limiting.", active: None },
            Item::Knob { p: STATIC, x: 300.0, y: 330.0, r: 22.0, label: "STATIC", tip: "Hiss and crackle of the AM signal.", active: None },
            Item::Knob { p: DRIVE, x: 410.0, y: 330.0, r: 22.0, label: "SPEAKER DRIVE", tip: "How hard the little speaker is pushed.", active: None },
            Item::Knob { p: OUTPUT, x: 540.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Car Radio", class_id: *b"PSCarRadioPQD001", vendor: "PEQUOD Spector",
    categories: [Fx, Analyzer]
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
    fn radio_cuts_highs_and_studio_is_clean() {
        let lvl = |f: f64, check: f32| {
            let p = with(defaults(), &[(CHECK, check), (STATIC, 0.0), (STATION, 0.0), (DRIVE, 0.0)]);
            let mut input = vec![sine(f, 0.1, 48000)];
            run(&p, &mut input);
            db(rms(&input[0][24000..]) / (0.1 / 2f64.sqrt()))
        };
        let (r1k, r10k, r60) = (lvl(1000.0, 1.0), lvl(10000.0, 1.0), lvl(60.0, 1.0));
        let s10k = lvl(10000.0, 0.0);
        println!("radio 1k {r1k:.1} 10k {r10k:.1} 60 {r60:.1}; studio 10k {s10k:.2}");
        assert!(r10k < r1k - 25.0 && r60 < r1k - 12.0 && s10k.abs() < 0.1);
    }

}
