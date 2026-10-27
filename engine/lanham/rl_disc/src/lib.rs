//! Transcription Disc: the sound of a 1950s radio transcription disc like the ones Roy Lanham's radio work was cut to: narrow bandwidth, surface hiss and crackle, a slow wow from the turntable and a little groove distortion. Turn AGE up for an older, more worn disc.

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
pub const H: u32 = 320;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "age"]
    pub age: FloatParam,
    #[id = "wow"]
    pub wow: FloatParam,
    #[id = "band"]
    pub band: FloatParam,
    #[id = "drive"]
    pub drive: FloatParam,
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
            age: pct("Age", 0.4),
            wow: pct("Wow", 0.2),
            band: pct("Bandwidth", 0.5),
            drive: pct("Groove Distortion", 0.2),
            mix: pct("Mix", 1.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const AGE: usize = 0;
pub const WOW: usize = 1;
pub const BAND: usize = 2;
pub const DRIVE: usize = 3;
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
            self.age.as_ptr(),
            self.wow.as_ptr(),
            self.band.as_ptr(),
            self.drive.as_ptr(),
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

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    bp: Vec<[Biquad; 3]>,
    dl: Vec<DelayLine>,
    hiss: Biquad,
    nz: Noise,
    seed: u32,
    crack: f64,
    lfo: Lfo,
    key: i32,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let f = 3500.0 + 5000.0 * self.v[BAND] as f64;
        for b in self.bp.iter_mut() {
            b[0].set_highpass(fs, 70.0 + 80.0 * (1.0 - self.v[BAND] as f64), 0.7);
            b[1].set_lowpass(fs, f, 0.6);
            b[2].set_lowpass(fs, f, 1.0);
        }
        self.hiss.set_bandpass(fs, 3000.0, 0.5);
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, bp: vec![[Biquad::identity(); 3]; ch.max(2)], dl: (0..ch.max(2)).map(|_| DelayLine::new((0.02 * fs) as usize)).collect(),
            hiss: Biquad::identity(), nz: Noise::new(11), seed: 5, crack: 0.0, lfo: Lfo::default(), key: -1, out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.bp.iter_mut().for_each(|b| b.iter_mut().for_each(|q| q.reset()));
        self.dl.iter_mut().for_each(|l| l.reset());
        self.hiss.reset();
        self.crack = 0.0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = (v[BAND] * 100.0) as i32;
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let age = self.v[AGE] as f64;
        let wow = self.v[WOW] as f64 * 0.0015 * self.fs;
        let drive = 1.0 + 4.0 * self.v[DRIVE] as f64;
        let mix = self.v[MIX] as f64;
        let rate = 1.0 + 25.0 * age * age;
        let cd = coeff(self.fs, 0.3);
        let last = self.bp.len() - 1;
        for k in 0..n {
            // 33 1/3 rpm: the wow repeats every 1.8 s
            let w = self.lfo.next(self.fs, 0.555);
            let d = 2.0 + 0.002 * self.fs + wow * w;
            self.seed = self.seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let r = (self.seed >> 8) as f64 / 16777216.0;
            if r < rate / self.fs {
                self.crack = (r * 1e6 % 1.0 - 0.5) * 0.4 * age;
            }
            self.crack *= cd;
            let noise = self.hiss.process(self.nz.next()) * 0.01 * age + self.crack;
            let o = self.out.next();
            for (ci, c) in chans.iter_mut().enumerate() {
                let j = ci.min(last);
                let x = c[k] as f64;
                self.dl[j].write(x);
                let y = self.dl[j].read(d);
                let y = (y * drive).tanh() / drive.tanh();
                let y = cascade(&mut self.bp[j], y + noise);
                c[k] = ((y * mix + x * (1.0 - mix)) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Radio Transcription", values: &[] },
        Preset { name: "Clean Pressing", values: &[(AGE, 0.1), (WOW, 0.05), (BAND, 0.8)] },
        Preset { name: "Worn Out", values: &[(AGE, 0.9), (WOW, 0.4), (BAND, 0.3), (DRIVE, 0.4)] },
        Preset { name: "AM Broadcast", values: &[(BAND, 0.0), (AGE, 0.2), (WOW, 0.0)] },
        Preset { name: "Half Blend", values: &[(MIX, 0.5)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "TRANSCRIPTION DISC",
        subtitle: ("A 1950s RADIO TRANSCRIPTION DISC", "BANDWIDTH · HISS · CRACKLE · WOW"),
        accent: ui::ROSE,
        size: (W, H),
        items: vec![
            Item::Knob { p: AGE, x: 70.0, y: 180.0, r: 32.0, label: "AGE", tip: "Surface hiss and crackle.", active: None },
            Item::Knob { p: WOW, x: 170.0, y: 180.0, r: 32.0, label: "WOW", tip: "Slow pitch wobble from the turntable.", active: None },
            Item::Knob { p: BAND, x: 270.0, y: 180.0, r: 32.0, label: "BANDWIDTH", tip: "How much top and bottom the disc holds.", active: None },
            Item::Knob { p: DRIVE, x: 370.0, y: 180.0, r: 32.0, label: "GROOVE", tip: "Groove distortion.", active: None },
            Item::Knob { p: MIX, x: 470.0, y: 180.0, r: 32.0, label: "MIX", tip: "Disc against dry.", active: None },
            Item::Knob { p: OUTPUT, x: 570.0, y: 180.0, r: 32.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Transcription Disc", class_id: *b"RLTranscriptDisc", vendor: "PEQUOD Roy Lanham",
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
    fn goertzel(x: &[f32], f: f64) -> f64 {
        let w = 2.0 * PI * f / 48000.0;
        let (mut s1, mut s2) = (0.0, 0.0);
        for v in x {
            let s = *v as f64 + 2.0 * w.cos() * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        ((s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2).max(0.0)).sqrt() / x.len() as f64 * 2.0
    }
    fn stereo(p: &[f32], x: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        let r = input.pop().unwrap();
        (input.pop().unwrap(), r)
    }
    fn corr(a: &[f32], b: &[f32]) -> f64 {
        let ab: f64 = a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum();
        let aa: f64 = a.iter().map(|x| (*x as f64).powi(2)).sum();
        let bb: f64 = b.iter().map(|x| (*x as f64).powi(2)).sum();
        ab / (aa * bb).sqrt().max(1e-12)
    }
    /// short-window rms envelope
    fn env(x: &[f32], w: usize) -> Vec<f64> {
        x.chunks(w).map(rms).collect()
    }

    #[test]
    fn narrow_and_noisy() {
        let p = with(defaults(), &[(BAND, 0.5), (AGE, 0.0), (WOW, 0.0), (DRIVE, 0.0)]);
        let a = db(rms(&stereo(&p, &sine(12000.0, 0.1, 24000)).0[12000..]));
        let b = db(rms(&stereo(&p, &sine(1000.0, 0.1, 24000)).0[12000..]));
        let noise = db(rms(&stereo(&with(defaults(), &[(AGE, 0.8)]), &vec![0.0f32; 48000]).0[24000..]));
        println!("12 kHz {a:.1} dB vs 1 kHz {b:.1} dB; silence with age {noise:.1} dB");
        assert!(b - a > 20.0 && noise > -80.0);
    }

}
