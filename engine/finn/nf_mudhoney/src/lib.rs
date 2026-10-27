//! Mudhoney: after the T-Rex Mudhoney II, the dual distortion on Neil Finn's board: two independent distortion channels, each with its own GAIN and TONE. Use either on its own, or both, with A feeding B for a thicker, higher-gain sound. Circuit behaviour estimated.

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
pub const H: u32 = 330;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "aon"]
    pub a_on: BoolParam,
    #[id = "gaina"]
    pub gaina: FloatParam,
    #[id = "tonea"]
    pub tonea: FloatParam,
    #[id = "bon"]
    pub b_on: BoolParam,
    #[id = "gainb"]
    pub gainb: FloatParam,
    #[id = "toneb"]
    pub toneb: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            a_on: BoolParam::new("Channel A", true),
            gaina: pct("Gain A", 0.35),
            tonea: pct("Tone A", 0.5),
            b_on: BoolParam::new("Channel B", false),
            gainb: pct("Gain B", 0.7),
            toneb: pct("Tone B", 0.5),
            output: db("Level", 0.0, -24.0, 12.0),
        }
    }
}

pub const AON: usize = 0;
pub const GAINA: usize = 1;
pub const TONEA: usize = 2;
pub const BON: usize = 3;
pub const GAINB: usize = 4;
pub const TONEB: usize = 5;
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
            self.a_on.as_ptr(),
            self.gaina.as_ptr(),
            self.tonea.as_ptr(),
            self.b_on.as_ptr(),
            self.gainb.as_ptr(),
            self.toneb.as_ptr(),
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
struct St {
    hpa: Biquad,
    hpb: Biquad,
    ta: Biquad,
    tb: Biquad,
    lp: Biquad,
}

struct Chan {
    os: Oversampler,
    st: St,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ch: Vec<Chan>,
    key: Vec<i32>,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let ofs = fs * OS as f64;
        let v = self.v;
        let _ = (fs, ofs, v);
        for c in self.ch.iter_mut() {
            let st = &mut c.st;
            st.hpa.set_highpass(ofs, 120.0, 0.7);
            st.hpb.set_highpass(ofs, 160.0, 0.7);
            st.ta.set_lowpass(ofs, 1500.0 * 5f64.powf(v[TONEA] as f64), 0.7);
            st.tb.set_lowpass(ofs, 1500.0 * 5f64.powf(v[TONEB] as f64), 0.7);
            st.lp.set_lowpass(ofs, 11000.0, 0.7);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, ch: (0..ch.max(2)).map(|_| Chan { os: Oversampler::new(), st: St::default() }).collect(), key: vec![], out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn reset(&mut self) {
        for c in self.ch.iter_mut() {
            c.os.reset();
            c.st = St::default();
        }
        self.design();
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k: Vec<i32> = v[..NPARAMS].iter().map(|x| (x * 1000.0) as i32).collect();
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let v = self.v;
        let ofs = self.fs * OS as f64;
        let _ = (&meters, ofs, &v);
        let last = self.ch.len() - 1;
        let mut most = 0.0f64;
        for k in 0..n {
            let o = self.out.next();
            for (ci, chn) in chans.iter_mut().enumerate() {
                let c = &mut self.ch[ci.min(last)];
                let x = chn[k] as f64;
                let st = &mut c.st;
                let y = c.os.process(x, |u| {
                    let clip = |x: f64| x / (1.0 + (x / 0.5).abs().powf(3.0)).powf(1.0 / 3.0);
                    let (ga, gb) = (v[GAINA] as f64, v[GAINB] as f64);
                    let mut s = st.lp.process(u);
                    if v[AON] > 0.5 {
                        s = st.ta.process(clip(st.hpa.process(s) * (2.0 + 200.0 * ga * ga))) * 1.4;
                    }
                    if v[BON] > 0.5 {
                        s = st.tb.process(clip(st.hpb.process(s) * (2.0 + 300.0 * gb * gb))) * 1.4;
                    }
                    s
                });

                most = most.max(y.abs());
                chn[k] = (y * o) as f32;
            }
        }
        if 0 > 0 {
            meters[0] = most as f32;
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Channel A Crunch", values: &[] },
        Preset { name: "Channel B Lead", values: &[(AON, 0.0), (BON, 1.0)] },
        Preset { name: "A into B (thick)", values: &[(AON, 1.0), (BON, 1.0), (GAINA, 0.3), (GAINB, 0.5), (OUTPUT, -4.0)] },
        Preset { name: "Bright Edge", values: &[(GAINA, 0.2), (TONEA, 0.75)] },
        Preset { name: "Dark Fuzzy B", values: &[(AON, 0.0), (BON, 1.0), (GAINB, 0.9), (TONEB, 0.25)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "MUDHONEY",
        subtitle: ("AFTER THE T-REX MUDHONEY II", "TWO DISTORTION CHANNELS"),
        accent: ui::TANGERINE,
        size: (W, H),
        items: vec![

            Item::Switch { p: AON, x: 70.0, y: 115.0, labels: ["", "A ON"], tip: "Channel A on." },
            Item::Knob { p: GAINA, x: 100.0, y: 205.0, r: 30.0, label: "GAIN A", tip: "Channel A gain.", active: None },
            Item::Knob { p: TONEA, x: 200.0, y: 205.0, r: 24.0, label: "TONE A", tip: "Channel A tone.", active: None },
            Item::Switch { p: BON, x: 320.0, y: 115.0, labels: ["", "B ON"], tip: "Channel B on (fed by A when both are on)." },
            Item::Knob { p: GAINB, x: 350.0, y: 205.0, r: 30.0, label: "GAIN B", tip: "Channel B gain.", active: None },
            Item::Knob { p: TONEB, x: 450.0, y: 205.0, r: 24.0, label: "TONE B", tip: "Channel B tone.", active: None },
            Item::Knob { p: OUTPUT, x: 560.0, y: 205.0, r: 30.0, label: "LEVEL", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Mudhoney", class_id: *b"NFMudhoneyDualDS", vendor: "PEQUOD Finn Brothers",
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
    fn b_after_a_is_heavier() {
        let a = thd(&with(defaults(), &[(AON, 1.0), (BON, 0.0)]), 300.0, 0.03);
        let ab = thd(&with(defaults(), &[(AON, 1.0), (BON, 1.0)]), 300.0, 0.03);
        println!("harmonics A {a:.1} dB, A+B {ab:.1} dB");
        assert!(ab > a + 3.0);
    }

}
