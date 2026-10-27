//! OS-2: overdrive and distortion in one box with a COLOR knob that blends them, after the Boss OS-2 Kele Okereke runs two of. Overdrive: soft clipping in the feedback loop (clean signal plus the clipped mids). Distortion: more gain into hard clipping to ground.

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
    #[id = "color"]
    pub color: FloatParam,
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
            drive: pct("Drive", 0.5),
            color: pct("Color", 0.35),
            tone: pct("Tone", 0.55),
            output: db("Level", 0.0, -24.0, 12.0),
        }
    }
}

pub const DRIVE: usize = 0;
pub const COLOR: usize = 1;
pub const TONE: usize = 2;
pub const OUTPUT: usize = 3;
pub const NPARAMS: usize = 4;

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
            self.color.as_ptr(),
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
    od_hp: Biquad,
    ds_hp: Biquad,
    tone: Biquad,
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
            st.od_hp.set_highpass(ofs, 600.0, 0.6);
            st.ds_hp.set_highpass(ofs, 150.0, 0.7);
            st.tone.set_lowpass(ofs, 900.0 * 7f64.powf(v[TONE] as f64), 0.6);
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
                    let d = v[DRIVE] as f64;
                    let mixc = v[COLOR] as f64;
                    let u = st.lp.process(u);
                    // OD: clean plus soft-clipped mids
                    let w = st.od_hp.process(u) * (2.0 + 100.0 * d * d);
                    let od = u + 0.6 * (w / 0.6).tanh();
                    // DS: gain into two diodes to ground
                    let g = st.ds_hp.process(u) * (8.0 + 500.0 * d * d);
                    let ds = g / (1.0 + (g / 0.5).abs().powf(3.0)).powf(1.0 / 3.0) * 1.4;
                    st.tone.process(od * (1.0 - mixc) + ds * mixc)
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
        Preset { name: "Kele Crunch (more OD)", values: &[] },
        Preset { name: "Helicopter Drive", values: &[(DRIVE, 0.65), (COLOR, 0.55), (TONE, 0.6)] },
        Preset { name: "Hunting for Witches (with the HOG)", values: &[(DRIVE, 0.8), (COLOR, 0.8), (TONE, 0.5), (OUTPUT, -3.0)] },
        Preset { name: "Light Edge", values: &[(DRIVE, 0.25), (COLOR, 0.1), (OUTPUT, 2.0)] },
        Preset { name: "Full Distortion", values: &[(DRIVE, 0.9), (COLOR, 1.0), (OUTPUT, -4.0)] },
        Preset { name: "Two OS-2s Stacked", values: &[(DRIVE, 1.0), (COLOR, 0.6), (TONE, 0.45), (OUTPUT, -6.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "OS-2",
        subtitle: ("AFTER THE BOSS OS-2", "OVERDRIVE / DISTORTION BLEND"),
        accent: ui::TANGERINE,
        size: (W, H),
        items: vec![

            Item::Knob { p: DRIVE, x: 90.0, y: 190.0, r: 40.0, label: "DRIVE", tip: "Gain.", active: None },
            Item::Knob { p: COLOR, x: 245.0, y: 190.0, r: 40.0, label: "COLOR", tip: "Blend from overdrive (left: soft, mid-pushed, clean underneath) to distortion (right: harder, more compressed).", active: None },
            Item::Knob { p: TONE, x: 400.0, y: 190.0, r: 40.0, label: "TONE", tip: "Darker to brighter.", active: None },
            Item::Knob { p: OUTPUT, x: 555.0, y: 190.0, r: 40.0, label: "LEVEL", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "OS-2", class_id: *b"BlpOS2OverDistPQ", vendor: "PEQUOD Bloc Party",
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
    fn color_blends_to_harder() {
        let a = thd(&with(defaults(), &[(COLOR, 0.0), (TONE, 1.0)]), 300.0, 0.05);
        let b = thd(&with(defaults(), &[(COLOR, 1.0), (TONE, 1.0)]), 300.0, 0.05);
        println!("harmonics: OD {a:.1} dB, DS {b:.1} dB");
        assert!(b > a + 3.0);
    }

}
