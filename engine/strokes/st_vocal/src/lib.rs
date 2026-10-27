//! Practice Amp Vocal: Julian Casablancas' vocal sound on Is This It, sung through a small Peavey practice amp and re-miked, because he wanted it 'shitty, but not too shitty' (Gordon Raphael). A small solid-state preamp and power amp running out of headroom, a little speaker in a little box, and BLEND to bring some of the clean mic back. Voicing estimated.

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
pub const H: u32 = 440;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Speaker { #[name = "6 inch"] Six, #[name = "8 inch"] Eight, #[name = "10 inch"] Ten }


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
    #[id = "speaker"]
    pub speaker: EnumParam<Speaker>,
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
            drive: db("Drive", 18.0, 0.0, 40.0),
            tone: pct("Tone", 0.55),
            speaker: EnumParam::new("Speaker", Speaker::Eight),
            blend: pct("Clean Blend", 0.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const DRIVE: usize = 0;
pub const TONE: usize = 1;
pub const SPEAKER: usize = 2;
pub const BLEND: usize = 3;
pub const OUTPUT: usize = 4;
pub const NPARAMS: usize = 5;

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
            self.speaker.as_ptr(),
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
    tone: [Biquad; 2],
    spk: [Biquad; 4],
    delay: Vec<f64>,
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
        let t = self.v[TONE] as f64;
        let sp = (self.v[SPEAKER].round() as usize).min(2);
        let (hpf, pk, lpf) = [(220.0, 2200.0, 5200.0), (160.0, 1700.0, 4600.0), (110.0, 1300.0, 4000.0)][sp];
        for c in self.ch.iter_mut() {
            c.hp.set_highpass(ofs, 120.0, 0.7);
            // the practice amp's single tone knob: a tilt around 1 kHz
            c.tone[0].set_low_shelf(ofs, 600.0, 0.7, 6.0 - 12.0 * t);
            c.tone[1].set_high_shelf(ofs, 1600.0, 0.7, -6.0 + 12.0 * t);
            c.spk[0].set_highpass(fs, hpf, 1.1);
            c.spk[1].set_peak(fs, pk, 1.2, 5.0);
            c.spk[2].set_lowpass(fs, lpf, 0.8);
            c.spk[3].set_peak(fs, 450.0, 1.5, 2.5);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mk = || Chan { os: Oversampler::new(), delay: vec![0.0; OS_LATENCY + 1], ..Default::default() };
        let mut d = Self { fs, v, ch: (0..ch.max(2)).map(|_| mk()).collect(), key: vec![], blend: Smooth::new(fs, 20.0, 0.0), out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn reset(&mut self) {
        for c in self.ch.iter_mut() {
            *c = Chan { os: Oversampler::new(), delay: vec![0.0; OS_LATENCY + 1], ..Default::default() };
        }
        self.design();
        self.blend.snap(self.v[BLEND] as f64);
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k: Vec<i32> = vec![(v[TONE] * 400.0) as i32, v[SPEAKER] as i32];
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.blend.set(v[BLEND] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let drive = db_to_gain(self.v[DRIVE] as f64);
        let last = self.ch.len() - 1;
        for k in 0..n {
            let b = self.blend.next();
            let o = self.out.next();
            for (ci, chn) in chans.iter_mut().enumerate() {
                let c = &mut self.ch[ci.min(last)];
                let x = chn[k] as f64;
                let (hp, tone) = (&mut c.hp, &mut c.tone);
                let y = c.os.process(x, |u| {
                    let u = hp.process(u) * drive;
                    // transistor preamp: lopsided soft clip
                    let p = if u >= 0.0 { (u * 1.2).tanh() / 1.2 } else { (u * 0.9).tanh() / 0.9 };
                    let p = cascade(&mut tone[..], p);
                    // the little power amp's rails
                    0.8 * (p * 1.5 / 0.8).tanh()
                });
                let y = cascade(&mut c.spk, y) * 0.8;
                // the clean mic, delayed to line up with the oversampled path
                let dl = c.delay.len();
                let clean = c.delay[c.dpos];
                c.delay[c.dpos] = x;
                c.dpos = (c.dpos + 1) % dl;
                chn[k] = ((y * (1.0 - b) + clean * b) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Is This It Vocal", values: &[] },
        Preset { name: "Shitty, But Not Too Shitty", values: &[(DRIVE, 12.0), (BLEND, 0.25)] },
        Preset { name: "Last Nite Shout", values: &[(DRIVE, 26.0), (TONE, 0.6)] },
        Preset { name: "Tinny 6-inch", values: &[(SPEAKER, 0.0), (DRIVE, 16.0), (TONE, 0.7)] },
        Preset { name: "Warmer 10-inch", values: &[(SPEAKER, 2.0), (DRIVE, 14.0), (TONE, 0.4)] },
        Preset { name: "Subtle Grit on a Clean Vocal", values: &[(DRIVE, 10.0), (BLEND, 0.6)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "PRACTICE AMP VOCAL",
        subtitle: ("JULIAN'S IS THIS IT VOCAL", "THROUGH A SMALL PEAVEY PRACTICE AMP"),
        accent: ui::ROSE,
        size: (W, H),
        items: vec![

            Item::Knob { p: DRIVE, x: 120.0, y: 175.0, r: 46.0, label: "DRIVE", tip: "How hard the voice hits the practice amp's input.", active: None },
            Item::Knob { p: TONE, x: 300.0, y: 175.0, r: 36.0, label: "TONE", tip: "The amp's tone knob: darker to brighter.", active: None },
            Item::Selector { p: SPEAKER, x: 480.0, y: 175.0, label: "SPEAKER", positions: &["6\"", "8\"", "10\""], tip: "Size of the practice amp's speaker: smaller is thinner and more honky." },
            Item::Divider { y: 270.0 },
            Item::Knob { p: BLEND, x: 200.0, y: 335.0, r: 26.0, label: "CLEAN BLEND", tip: "Brings some of the clean vocal mic back under the amp ('not too shitty').", active: None },
            Item::Knob { p: OUTPUT, x: 440.0, y: 335.0, r: 26.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Practice Amp Vocal", class_id: *b"StrPracticeVocal", vendor: "PEQUOD The Strokes",
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
    fn small_speaker_and_grit() {
        let p = defaults();
        let lo = db(rms(&go(&p, &sine_amp(80.0, 0.05, 48000))[24000..]));
        let mid = db(rms(&go(&p, &sine_amp(1500.0, 0.05, 48000))[24000..]));
        let t = thd(&p, 300.0, 0.1);
        println!("80 Hz {lo:.1} dB, 1.5 kHz {mid:.1} dB, harmonics {t:.1} dB");
        assert!(mid - lo > 15.0 && t > -25.0);
    }

}
