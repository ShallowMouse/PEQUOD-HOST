//! SVT Bass: Mark Hoppus' bass sound, a picked bass (his Music Man StingRay on Enema of the State) into an Ampeg SVT-style valve head and 8x10: ULTRA LO and ULTRA HI switches, a five-position mid, valve grit that grows with GAIN and MASTER, and a BLEND to the clean DI as engineers usually record it. Voicing estimated.

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
pub const H: u32 = 400;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum MidF { #[name = "220 Hz"] A, #[name = "450 Hz"] B, #[name = "800 Hz"] C, #[name = "1.6 kHz"] D, #[name = "3 kHz"] E }


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
    #[id = "mid"]
    pub mid: FloatParam,
    #[id = "midf"]
    pub midf: EnumParam<MidF>,
    #[id = "treble"]
    pub treble: FloatParam,
    #[id = "ulo"]
    pub ulo: BoolParam,
    #[id = "uhi"]
    pub uhi: BoolParam,
    #[id = "master"]
    pub master: FloatParam,
    #[id = "blend"]
    pub blend: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            gain: FloatParam::new("Gain", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            bass: db("Bass", 3.0, -15.0, 15.0),
            mid: db("Midrange", 2.0, -12.0, 12.0),
            midf: EnumParam::new("Mid Frequency", MidF::C),
            treble: db("Treble", 3.0, -15.0, 15.0),
            ulo: BoolParam::new("Ultra Lo", false),
            uhi: BoolParam::new("Ultra Hi", true),
            master: FloatParam::new("Master", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            blend: pct("DI Blend", 0.3),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const GAIN: usize = 0;
pub const BASS: usize = 1;
pub const MID: usize = 2;
pub const MIDF: usize = 3;
pub const TREBLE: usize = 4;
pub const ULO: usize = 5;
pub const UHI: usize = 6;
pub const MASTER: usize = 7;
pub const BLEND: usize = 8;
pub const OUTPUT: usize = 9;
pub const NPARAMS: usize = 10;

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
            self.mid.as_ptr(),
            self.midf.as_ptr(),
            self.treble.as_ptr(),
            self.ulo.as_ptr(),
            self.uhi.as_ptr(),
            self.master.as_ptr(),
            self.blend.as_ptr(),
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

#[derive(Clone, Default)]
struct Chan {
    os: Oversampler,
    hp: Biquad,
    eq: [Biquad; 6],
    cab: [Biquad; 6],
    sag: f64,
    dly: Vec<f64>,
    dpos: usize,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ch: Vec<Chan>,
    key: Vec<i32>,
    blend: Smooth,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let ofs = fs * OS as f64;
        let p = |i: usize| self.v[i] as f64;
        let mf = [220.0, 450.0, 800.0, 1600.0, 3000.0][(p(MIDF).round() as usize).min(4)];
        let ulo = p(ULO) > 0.5;
        let uhi = p(UHI) > 0.5;
        for c in self.ch.iter_mut() {
            c.hp.set_highpass(ofs, 30.0, 0.7);
            c.eq[0].set_low_shelf(ofs, 60.0, 0.7, p(BASS));
            c.eq[1].set_peak(ofs, mf, 0.9, p(MID));
            c.eq[2].set_high_shelf(ofs, 4000.0, 0.7, p(TREBLE));
            // ULTRA LO: +2 dB at 40 Hz and a deep cut at 500 Hz; ULTRA HI: +9 dB at 8 kHz
            c.eq[3].set_peak(ofs, 40.0, 1.0, if ulo { 2.0 } else { 0.0 });
            c.eq[4].set_peak(ofs, 500.0, 0.9, if ulo { -10.0 } else { 0.0 });
            c.eq[5].set_high_shelf(ofs, 5000.0, 0.7, if uhi { 9.0 } else { 0.0 });
            // 8x10 closed cabinet
            c.cab[0].set_highpass(fs, 42.0, 1.0);
            c.cab[1].set_peak(fs, 80.0, 1.2, 3.0);
            c.cab[2].set_peak(fs, 400.0, 1.0, -2.0);
            c.cab[3].set_peak(fs, 1600.0, 1.3, 3.5);
            c.cab[4].set_lowpass(fs, 4500.0, 0.7);
            c.cab[5].set_lowpass(fs, 5200.0, 0.6);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mk = || Chan { os: Oversampler::new(), dly: vec![0.0; OS_LATENCY + 1], ..Default::default() };
        let mut d = Self { fs, v, ch: (0..ch.max(2)).map(|_| mk()).collect(), key: vec![], blend: Smooth::new(fs, 20.0, 0.3), out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn reset(&mut self) {
        for c in self.ch.iter_mut() {
            *c = Chan { os: Oversampler::new(), dly: vec![0.0; OS_LATENCY + 1], ..Default::default() };
        }
        self.design();
        self.blend.snap(self.v[BLEND] as f64);
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k: Vec<i32> = v[..NPARAMS].iter().map(|x| (x * 100.0) as i32).collect();
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.blend.set(v[BLEND] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let g = 10f64.powf(-0.3 + 1.6 * self.v[GAIN] as f64 / 10.0);
        let master = 0.3 + 2.0 * (self.v[MASTER] as f64 / 10.0).powi(2);
        let cs = coeff(self.fs * OS as f64, 120.0);
        let last = self.ch.len() - 1;
        let mut most = 0.0f64;
        for k in 0..n {
            let b = self.blend.next();
            let o = self.out.next();
            for (ci, chn) in chans.iter_mut().enumerate() {
                let c = &mut self.ch[ci.min(last)];
                let x = chn[k] as f64;
                let Chan { os, hp, eq, sag, .. } = c;
                let y = os.process(x, |u| {
                    // 12AX7 preamp: lopsided soft clip
                    let s = hp.process(u) * g;
                    let s = (s + 0.2).tanh() - 0.2f64.tanh();
                    let s = cascade(&mut eq[..], s * 2.0);
                    // six 6550s: wide headroom, a gentle squash and sag when pushed
                    let drive = master * (1.0 - 0.2 * *sag);
                    let p = (s * drive * 0.6).tanh() / 0.6;
                    let a = p.abs().min(1.5) / 1.5;
                    *sag = a + cs * (*sag - a);
                    p
                });
                most = most.max(c.sag);
                let amp = cascade(&mut c.cab, y) * 0.45;
                let dl = c.dly.len();
                let clean = c.dly[c.dpos];
                c.dly[c.dpos] = x;
                c.dpos = (c.dpos + 1) % dl;
                chn[k] = ((amp * (1.0 - b) + clean * b) * o) as f32;
            }
        }
        meters[0] = most as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Hoppus Pick Tone", values: &[] },
        Preset { name: "Enema Growl", values: &[(GAIN, 6.5), (MID, 4.0), (MIDF, 2.0), (BLEND, 0.25)] },
        Preset { name: "Dammit Punk Bass", values: &[(GAIN, 7.0), (BASS, 5.0), (TREBLE, 5.0), (UHI, 1.0)] },
        Preset { name: "Clean and Round", values: &[(GAIN, 2.5), (TREBLE, 0.0), (UHI, 0.0), (BLEND, 0.5)] },
        Preset { name: "Scooped (Ultra Lo)", values: &[(ULO, 1.0), (GAIN, 5.0)] },
        Preset { name: "Driven Amp Only", values: &[(GAIN, 8.5), (MASTER, 7.0), (BLEND, 0.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "SVT BASS",
        subtitle: ("HOPPUS' PICKED BASS INTO AN SVT", "ULTRA LO / HI · 5-POSITION MID · 8x10"),
        accent: ui::OXBLOOD,
        size: (W, H),
        items: vec![

            Item::Knob { p: GAIN, x: 70.0, y: 150.0, r: 30.0, label: "GAIN", tip: "Preamp gain: clean to gritty.", active: None },
            Item::Knob { p: BASS, x: 170.0, y: 150.0, r: 30.0, label: "BASS", tip: "Bass (60 Hz).", active: None },
            Item::Knob { p: MID, x: 270.0, y: 150.0, r: 30.0, label: "MIDRANGE", tip: "Midrange boost/cut at the MID FREQUENCY.", active: None },
            Item::Selector { p: MIDF, x: 375.0, y: 150.0, label: "FREQUENCY", positions: &["220", "450", "800", "1.6k", "3k"], tip: "The SVT's five-position mid frequency." },
            Item::Knob { p: TREBLE, x: 480.0, y: 150.0, r: 30.0, label: "TREBLE", tip: "Treble (4 kHz).", active: None },
            Item::Knob { p: MASTER, x: 575.0, y: 150.0, r: 30.0, label: "MASTER", tip: "How hard the power amp is pushed.", active: None },
            Item::Divider { y: 220.0 },
            Item::Switch { p: ULO, x: 80.0, y: 285.0, labels: ["OFF", "U.LO"], tip: "Ultra Lo: deep lows up, a big scoop at 500 Hz." },
            Item::Switch { p: UHI, x: 200.0, y: 285.0, labels: ["OFF", "U.HI"], tip: "Ultra Hi: +9 dB of top, for pick attack." },
            Item::Knob { p: BLEND, x: 330.0, y: 285.0, r: 24.0, label: "DI BLEND", tip: "Clean DI under the amp, as bass is usually recorded.", active: None },
            Item::Lamp { m: 0, x: 440.0, y: 270.0, label: "SAG", lo: 0.2, hi: 0.95 },
            Item::Knob { p: OUTPUT, x: 560.0, y: 285.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "SVT Bass", class_id: *b"B182SVTBassAmpPQ", vendor: "PEQUOD blink-182",
    categories: [Fx, Distortion]
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

    #[test]
    fn grit_and_ultra_hi() {
        let lo = thd(&with(defaults(), &[(GAIN, 0.0), (BLEND, 0.0)]), 82.0, 0.05);
        let hi = thd(&with(defaults(), &[(GAIN, 10.0), (BLEND, 0.0)]), 82.0, 0.05);
        let a = db(rms(&go(&with(defaults(), &[(UHI, 0.0), (BLEND, 0.0), (GAIN, 1.0)]), &sine_amp(6000.0, 0.01, 24000))[12000..]));
        let b = db(rms(&go(&with(defaults(), &[(UHI, 1.0), (BLEND, 0.0), (GAIN, 1.0)]), &sine_amp(6000.0, 0.01, 24000))[12000..]));
        println!("harmonics gain 0 {lo:.1} / 10 {hi:.1} dB; 6 kHz ultra hi off {a:.1} / on {b:.1}");
        assert!(hi > lo + 12.0 && b > a + 4.0);
    }

}
