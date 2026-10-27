//! RAT: a Pro Co RAT 2, Alex Turner's distortion on the first album (he ran two). An op-amp with up to about 67 dB of gain that rises with frequency (two RC legs: 560 ohm + 4.7 uF and 47 ohm + 2.2 uF), slowed by the LM308's limited bandwidth, into two silicon diodes to ground, then the passive FILTER (1.5k + 100k pot, 3.3 nF).

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
    #[id = "dist"]
    pub dist: FloatParam,
    #[id = "filter"]
    pub filter: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            dist: pct("Distortion", 0.6),
            filter: pct("Filter", 0.4),
            output: db("Volume", -6.0, -30.0, 6.0),
        }
    }
}

pub const DIST: usize = 0;
pub const FILTER: usize = 1;
pub const OUTPUT: usize = 2;
pub const NPARAMS: usize = 3;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.dist.as_ptr(),
            self.filter.as_ptr(),
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
    hp_a: Biquad,
    hp_b: Biquad,
    gbw: Biquad,
    filt: Biquad,
    g_a: f64,
    g_b: f64,
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
            // the DISTORTION pot is a 100k audio taper
            let t = v[DIST] as f64;
            let rd = 100e3 * (10f64.powf(2.0 * t) - 1.0) / 99.0 + 50.0;
            st.g_a = rd / 560.0;
            st.g_b = rd / 47.0;
            let w1 = 2.0 * std::f64::consts::PI / (560.0 * 4.7e-6);
            let w2 = 2.0 * std::f64::consts::PI / (47.0 * 2.2e-6);
            let _ = (w1, w2);
            st.hp_a.set_analog(ofs, [0.0, 1.0, 0.0], [0.0, 1.0, 1.0 / (560.0 * 4.7e-6)]);
            st.hp_b.set_analog(ofs, [0.0, 1.0, 0.0], [0.0, 1.0, 1.0 / (47.0 * 2.2e-6)]);
            // LM308 (30 pF compensation): about 300 kHz gain-bandwidth over the closed-loop gain
            let g = 1.0 + st.g_a + st.g_b;
            st.gbw.set_lowpass(ofs, (3.0e5 / g).clamp(2000.0, 0.45 * ofs), 0.707);
            // FILTER: 1.5k + 100k linear pot into 3.3 nF (clockwise = darker)
            let r = 1.5e3 + 100e3 * v[FILTER] as f64;
            st.filt.set_analog(ofs, [0.0, 0.0, 1.0], [0.0, r * 3.3e-9, 1.0]);
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
                    let a = st.hp_a.process(u);
                    let b = st.hp_b.process(u);
                    let w = u + st.g_a * a + st.g_b * b;
                    // op-amp rails (9 V supply)
                    let w = 4.0 * (st.gbw.process(w) / 4.0).tanh();
                    // 1N914 pair to ground
                    let vd = 0.55;
                    let c = w / (1.0 + (w / vd).abs().powf(2.5)).powf(1.0 / 2.5);
                    st.filt.process(c)
                });
                let y = y * 1.4;
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
        Preset { name: "Turner Rat (first album)", values: &[] },
        Preset { name: "Brianstorm Riff", values: &[(DIST, 0.7), (FILTER, 0.3), (OUTPUT, -6.0)] },
        Preset { name: "Two Rats Stacked (max)", values: &[(DIST, 1.0), (FILTER, 0.45), (OUTPUT, -10.0)] },
        Preset { name: "Crunchy Rhythm", values: &[(DIST, 0.4), (FILTER, 0.35), (OUTPUT, -4.0)] },
        Preset { name: "Dark Fuzzy", values: &[(DIST, 0.85), (FILTER, 0.75), (OUTPUT, -6.0)] },
        Preset { name: "Boost-ish", values: &[(DIST, 0.15), (FILTER, 0.2), (OUTPUT, -2.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "RAT",
        subtitle: ("AFTER THE PRO CO RAT 2", "TURNER'S FIRST-ALBUM DISTORTION"),
        accent: ui::TANGERINE,
        size: (W, H),
        items: vec![

            Item::Knob { p: DIST, x: 130.0, y: 190.0, r: 46.0, label: "DISTORTION", tip: "The gain pot: up to about 67 dB, more on the treble than the bass.", active: None },
            Item::Knob { p: FILTER, x: 320.0, y: 190.0, r: 46.0, label: "FILTER", tip: "The RAT's filter: turning it up (clockwise) cuts more top.", active: None },
            Item::Knob { p: OUTPUT, x: 510.0, y: 190.0, r: 46.0, label: "VOLUME", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "RAT", class_id: *b"AMkRatDistortPQ1", vendor: "PEQUOD Arctic Monkeys",
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
    fn gain_and_filter() {
        let lo = thd(&with(defaults(), &[(DIST, 0.0)]), 200.0, 0.02);
        let hi = thd(&with(defaults(), &[(DIST, 1.0)]), 200.0, 0.02);
        let p0 = with(defaults(), &[(FILTER, 0.0), (DIST, 0.3)]);
        let p1 = with(defaults(), &[(FILTER, 1.0), (DIST, 0.3)]);
        let b0 = db(rms(&go(&p0, &sine_amp(3000.0, 0.01, 24000))[12000..]));
        let b1 = db(rms(&go(&p1, &sine_amp(3000.0, 0.01, 24000))[12000..]));
        println!("harmonics: dist 0 {lo:.1} dB, dist max {hi:.1} dB; 3 kHz filter 0 {b0:.1} vs filter max {b1:.1}");
        assert!(hi > lo + 20.0 && b0 - b1 > 10.0);
    }

}
