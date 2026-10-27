//! TS-808: the Ibanez Tube Screamer Alex Turner has kept on his board since Favourite Worst Nightmare. The op-amp's gain applies only above about 720 Hz (4.7k + 0.047 uF), with two silicon diodes in the feedback path for soft clipping (the mid hump), the 51 pF cap taming the top, then the tone control and level.

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
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            drive: pct("Overdrive", 0.5),
            tone: pct("Tone", 0.5),
            output: db("Level", 0.0, -24.0, 12.0),
        }
    }
}

pub const DRIVE: usize = 0;
pub const TONE: usize = 1;
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
            self.drive.as_ptr(),
            self.tone.as_ptr(),
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
    hp: Biquad,
    cap: Biquad,
    tone: Biquad,
    lp_in: Biquad,
    g: f64,
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
            // drive pot 500k (audio) + 51k over 4.7k, above ~720 Hz
            let t = v[DRIVE] as f64;
            let rf = 51e3 + 500e3 * (10f64.powf(2.0 * t) - 1.0) / 99.0;
            st.g = rf / 4.7e3;
            st.hp.set_analog(ofs, [0.0, 1.0, 0.0], [0.0, 1.0, 1.0 / (4.7e3 * 0.047e-6)]);
            st.cap.set_lowpass(ofs, (1.0 / (2.0 * std::f64::consts::PI * rf * 51e-12)).min(0.45 * ofs), 0.707);
            // tone: a 1 kHz-ish fixed low-pass blended against the brighter path
            let f = 700.0 * (8.0f64).powf(v[TONE] as f64);
            st.tone.set_lowpass(ofs, f, 0.6);
            st.lp_in.set_lowpass(ofs, 12000.0, 0.707);
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
                    let u = st.lp_in.process(u);
                    let w = st.cap.process(st.hp.process(u) * st.g);
                    // diodes in the feedback loop: the clipped part adds to the clean signal
                    let vd = 0.6;
                    let c = vd * (w / vd).tanh();
                    st.tone.process(u + c)
                });
                let y = y * 0.9;
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
        Preset { name: "Turner TS (mid push)", values: &[] },
        Preset { name: "Boost Into a Driven Amp", values: &[(DRIVE, 0.1), (TONE, 0.55), (OUTPUT, 8.0)] },
        Preset { name: "Humbug Lead", values: &[(DRIVE, 0.75), (TONE, 0.45), (OUTPUT, 2.0)] },
        Preset { name: "Bright Crunch", values: &[(DRIVE, 0.5), (TONE, 0.8)] },
        Preset { name: "Dark Smooth", values: &[(DRIVE, 0.65), (TONE, 0.25), (OUTPUT, 2.0)] },
        Preset { name: "Full Gain", values: &[(DRIVE, 1.0), (TONE, 0.5), (OUTPUT, -2.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "TS-808",
        subtitle: ("AFTER THE IBANEZ TUBE SCREAMER", "SOFT CLIP · MID HUMP"),
        accent: ui::TANGERINE,
        size: (W, H),
        items: vec![

            Item::Knob { p: DRIVE, x: 130.0, y: 190.0, r: 46.0, label: "OVERDRIVE", tip: "Gain into the soft-clipping diodes. Bass passes clean, so it tightens and pushes the mids.", active: None },
            Item::Knob { p: TONE, x: 320.0, y: 190.0, r: 46.0, label: "TONE", tip: "Darker (left) to brighter (right).", active: None },
            Item::Knob { p: OUTPUT, x: 510.0, y: 190.0, r: 46.0, label: "LEVEL", tip: "Output level. Turned up with low OVERDRIVE, it's the classic boost into a hot amp.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "TS-808", class_id: *b"AMkTubeScream808", vendor: "PEQUOD Arctic Monkeys",
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
    fn bass_passes_cleaner_than_mids() {
        let p = with(defaults(), &[(DRIVE, 1.0), (TONE, 1.0)]);
        let b = thd(&p, 100.0, 0.03);
        let m = thd(&p, 600.0, 0.03);
        println!("harmonics at full drive: 100 Hz {b:.1} dB, 600 Hz {m:.1} dB");
        assert!(m > b + 6.0);
    }

}
