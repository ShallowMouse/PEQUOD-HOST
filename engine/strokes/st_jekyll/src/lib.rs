//! Jekyll & Hyde: a two-in-one overdrive and distortion after the first-generation Visual Sound Jekyll & Hyde that drove the early Strokes records. JEKYLL: a smooth, mid-pushed soft-clipping overdrive. HYDE: a harder, hotter distortion with a voice switch. Run either, or both (Jekyll into Hyde). Voicing estimated.

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
pub const H: u32 = 460;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "jon"]
    pub jon: BoolParam,
    #[id = "jdrive"]
    pub jdrive: FloatParam,
    #[id = "jtone"]
    pub jtone: FloatParam,
    #[id = "jvol"]
    pub jvol: FloatParam,
    #[id = "jvoice"]
    pub jvoice: BoolParam,
    #[id = "hon"]
    pub hon: BoolParam,
    #[id = "hdrive"]
    pub hdrive: FloatParam,
    #[id = "htone"]
    pub htone: FloatParam,
    #[id = "hvol"]
    pub hvol: FloatParam,
    #[id = "hvoice"]
    pub hvoice: BoolParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            jon: BoolParam::new("Jekyll On", true),
            jdrive: pct("Jekyll Drive", 0.45),
            jtone: pct("Jekyll Tone", 0.55),
            jvol: db("Jekyll Volume", 0.0, -20.0, 12.0),
            jvoice: BoolParam::new("Jekyll Voice", false),
            hon: BoolParam::new("Hyde On", false),
            hdrive: pct("Hyde Drive", 0.6),
            htone: pct("Hyde Tone", 0.5),
            hvol: db("Hyde Volume", -6.0, -24.0, 12.0),
            hvoice: BoolParam::new("Hyde Voice", false),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const JON: usize = 0;
pub const JDRIVE: usize = 1;
pub const JTONE: usize = 2;
pub const JVOL: usize = 3;
pub const JVOICE: usize = 4;
pub const HON: usize = 5;
pub const HDRIVE: usize = 6;
pub const HTONE: usize = 7;
pub const HVOL: usize = 8;
pub const HVOICE: usize = 9;
pub const OUTPUT: usize = 10;
pub const NPARAMS: usize = 11;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.jon.as_ptr(),
            self.jdrive.as_ptr(),
            self.jtone.as_ptr(),
            self.jvol.as_ptr(),
            self.jvoice.as_ptr(),
            self.hon.as_ptr(),
            self.hdrive.as_ptr(),
            self.htone.as_ptr(),
            self.hvol.as_ptr(),
            self.hvoice.as_ptr(),
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
    j_hp: Biquad,
    j_cap: Biquad,
    j_tone: Biquad,
    j_mid: Biquad,
    h_hp: Biquad,
    h_pre: Biquad,
    h_tone: Biquad,
    h_scoop: Biquad,
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
        let ofs = self.fs * OS as f64;
        let v = self.v;
        for c in self.ch.iter_mut() {
            c.j_hp.set_highpass(ofs, if v[JVOICE] > 0.5 { 300.0 } else { 600.0 }, 0.6);
            c.j_cap.set_lowpass(ofs, 9000.0, 0.7);
            c.j_tone.set_lowpass(ofs, 900.0 * 6f64.powf(v[JTONE] as f64), 0.6);
            c.j_mid.set_peak(ofs, 800.0, 0.8, if v[JVOICE] > 0.5 { 0.0 } else { 3.0 });
            c.h_hp.set_highpass(ofs, 120.0, 0.7);
            c.h_pre.set_peak(ofs, 1400.0, 0.7, 4.0);
            c.h_tone.set_lowpass(ofs, 1200.0 * 5f64.powf(v[HTONE] as f64), 0.6);
            c.h_scoop.set_peak(ofs, 650.0, 0.8, if v[HVOICE] > 0.5 { -7.0 } else { 0.0 });
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, ch: (0..ch.max(2)).map(|_| Chan { os: Oversampler::new(), ..Default::default() }).collect(), key: vec![], out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn reset(&mut self) {
        for c in self.ch.iter_mut() {
            *c = Chan { os: Oversampler::new(), ..Default::default() };
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
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let v = self.v;
        let jon = v[JON] > 0.5;
        let hon = v[HON] > 0.5;
        let jg = 1.0 + 120.0 * (v[JDRIVE] as f64).powi(2);
        let hg = 4.0 + 600.0 * (v[HDRIVE] as f64).powi(2);
        let jv = db_to_gain(v[JVOL] as f64);
        let hv = db_to_gain(v[HVOL] as f64);
        let last = self.ch.len() - 1;
        for k in 0..n {
            let o = self.out.next();
            for (ci, chn) in chans.iter_mut().enumerate() {
                let c = &mut self.ch[ci.min(last)];
                let x = chn[k] as f64;
                let Chan { os, j_hp, j_cap, j_tone, j_mid, h_hp, h_pre, h_tone, h_scoop } = c;
                let y = os.process(x, |u| {
                    let mut s = u;
                    if jon {
                        // soft clip in the feedback loop: the clipped treble/mids add to the clean signal
                        let w = j_cap.process(j_hp.process(s) * jg);
                        let c1 = 0.6 * (w / 0.6).tanh();
                        s = j_mid.process(j_tone.process(s + c1)) * jv;
                    }
                    if hon {
                        // hard clip to ground after more gain
                        let w = h_pre.process(h_hp.process(s)) * hg;
                        let c2 = w / (1.0 + (w / 0.5).abs().powf(3.0)).powf(1.0 / 3.0);
                        s = h_scoop.process(h_tone.process(c2)) * hv * 1.6;
                    }
                    s
                });
                chn[k] = (y * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Is This It Crunch (Jekyll)", values: &[] },
        Preset { name: "Last Nite Rhythm", values: &[(JDRIVE, 0.4), (JTONE, 0.6), (JVOL, 2.0)] },
        Preset { name: "Reptilia Lead (both)", values: &[(JON, 1.0), (JDRIVE, 0.3), (HON, 1.0), (HDRIVE, 0.55), (HTONE, 0.55)] },
        Preset { name: "Hyde Only", values: &[(JON, 0.0), (HON, 1.0)] },
        Preset { name: "Hyde Scooped", values: &[(JON, 0.0), (HON, 1.0), (HVOICE, 1.0), (HDRIVE, 0.7)] },
        Preset { name: "Jekyll Boost (low drive, loud)", values: &[(JDRIVE, 0.15), (JVOL, 8.0)] },
        Preset { name: "Thin Fuzzy Lead", values: &[(JON, 1.0), (JDRIVE, 0.7), (JVOICE, 1.0), (JTONE, 0.75)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "JEKYLL & HYDE",
        subtitle: ("AFTER THE VISUAL SOUND JEKYLL & HYDE", "OVERDRIVE + DISTORTION · THE EARLY STROKES"),
        accent: ui::TANGERINE,
        size: (W, H),
        items: vec![

            Item::Text { x: 160.0, y: 90.0, text: "JEKYLL  (OVERDRIVE)", size: 13.0, align: Align2::CENTER_CENTER },
            Item::Text { x: 480.0, y: 90.0, text: "HYDE  (DISTORTION)", size: 13.0, align: Align2::CENTER_CENTER },
            Item::Knob { p: JDRIVE, x: 70.0, y: 165.0, r: 26.0, label: "DRIVE", tip: "Jekyll gain.", active: None },
            Item::Knob { p: JTONE, x: 160.0, y: 165.0, r: 26.0, label: "TONE", tip: "Jekyll tone.", active: None },
            Item::Knob { p: JVOL, x: 250.0, y: 165.0, r: 26.0, label: "VOLUME", tip: "Jekyll level.", active: None },
            Item::Knob { p: HDRIVE, x: 390.0, y: 165.0, r: 26.0, label: "DRIVE", tip: "Hyde gain.", active: None },
            Item::Knob { p: HTONE, x: 480.0, y: 165.0, r: 26.0, label: "TONE", tip: "Hyde tone.", active: None },
            Item::Knob { p: HVOL, x: 570.0, y: 165.0, r: 26.0, label: "VOLUME", tip: "Hyde level.", active: None },
            Item::Switch { p: JVOICE, x: 100.0, y: 245.0, labels: ["MID", "FULL"], tip: "Jekyll voice: MID pushes the midrange (the classic sound); FULL lets more bass in." },
            Item::Switch { p: JON, x: 220.0, y: 245.0, labels: ["OFF", "ON"], tip: "Jekyll on/off." },
            Item::Switch { p: HVOICE, x: 420.0, y: 245.0, labels: ["FLAT", "SCOOP"], tip: "Hyde voice: FLAT, or SCOOP (mids cut)." },
            Item::Switch { p: HON, x: 540.0, y: 245.0, labels: ["OFF", "ON"], tip: "Hyde on/off. With both on, Jekyll drives Hyde." },
            Item::Divider { y: 290.0 },
            Item::Knob { p: OUTPUT, x: 320.0, y: 345.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Jekyll & Hyde", class_id: *b"StrJekyllHydePQ1", vendor: "PEQUOD The Strokes",
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
    fn thd(p: &[f32], f: f64, amp: f64) -> f64 {
        let y = go(p, &sine_amp(f, amp, 48000));
        let a = goertzel(&y[24000..], f);
        let h: f64 = (2..7).map(|k| goertzel(&y[24000..], f * k as f64).powi(2)).sum::<f64>().sqrt();
        db(h / a.max(1e-12))
    }

    #[test]
    fn hyde_is_harder_than_jekyll() {
        let j = thd(&with(defaults(), &[(JON, 1.0), (HON, 0.0)]), 300.0, 0.1);
        let h = thd(&with(defaults(), &[(JON, 0.0), (HON, 1.0)]), 300.0, 0.1);
        let off = thd(&with(defaults(), &[(JON, 0.0), (HON, 0.0)]), 300.0, 0.1);
        println!("harmonics: bypass {off:.1}, Jekyll {j:.1}, Hyde {h:.1} dB");
        assert!(j > off + 20.0 && h > j);
    }

}
