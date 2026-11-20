//! Octave Bass: the bass on The Less I Know the Better. Demoed late at night in a hotel room with hardly any
//! gear, it was a guitar through the P-Bass preset of a Roland GR-55 guitar synth, pitched down an octave and
//! driven a little (Mixdown). Here: an octave down (a pitch shifter, as the GR-55 does it, with the guitar
//! left in or out), a P-Bass voicing (a low-mid thump, the top rolled off as by a flatwound-strung bass
//! through its tone control), a little drive and a compressor to even the notes. Voicing estimated.

#![allow(clippy::excessive_precision)]
use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
#[allow(unused_imports)]
use vintage::dsp::*;
#[allow(unused_imports)]
use vintage::fx::*;
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

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "octave"]
    pub octave: FloatParam,
    #[id = "guitar"]
    pub guitar: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "thump"]
    pub thump: FloatParam,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "evenness"]
    pub evenness: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            octave: pct("Octave Down", 1.0),
            guitar: pct("Guitar", 0.0),
            tone: hz("Tone", 1400.0, 400.0, 5000.0),
            thump: db("Thump", 4.0, 0.0, 12.0),
            drive: pct("Drive", 0.3),
            evenness: pct("Compression", 0.5),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const OCTAVE: usize = 0;
pub const GUITAR: usize = 1;
pub const TONE: usize = 2;
pub const THUMP: usize = 3;
pub const DRIVE: usize = 4;
pub const EVENNESS: usize = 5;
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
            self.octave.as_ptr(),
            self.guitar.as_ptr(),
            self.tone.as_ptr(),
            self.thump.as_ptr(),
            self.drive.as_ptr(),
            self.evenness.as_ptr(),
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
    shift: Vec<PitchShift>,
    eq: Vec<[Biquad; 4]>,
    env: f64,
    key: Vec<i32>,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let v = self.v;
        for e in self.eq.iter_mut() {
            e[0].set_highpass(fs, 35.0, 0.7);
            e[1].set_peak(fs, 110.0, 1.0, v[THUMP] as f64);
            e[2].set_lowpass(fs, v[TONE] as f64, 0.6);
            e[3].set_peak(fs, 700.0, 1.2, -2.0);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let n = ch.max(2);
        let mut d = Self { fs, v, shift: (0..n).map(|_| PitchShift::new(fs, 45.0)).collect(), eq: vec![[Biquad::identity(); 4]; n], env: 0.0, key: vec![], out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.shift.iter_mut().for_each(|s| s.reset());
        self.eq.iter_mut().flatten().for_each(|b| b.reset());
        self.env = 0.0;
        self.design();
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k: Vec<i32> = vec![v[TONE] as i32, (v[THUMP] * 10.0) as i32];
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let v = self.v;
        let oct = v[OCTAVE] as f64;
        let gtr = v[GUITAR] as f64;
        let drive = v[DRIVE] as f64;
        let dg = 1.0 + 10.0 * drive * drive;
        let even = v[EVENNESS] as f64;
        let (ca, cr) = (coeff(self.fs, 8.0), coeff(self.fs, 200.0));
        let last = self.shift.len() - 1;
        for k in 0..n {
            let o = self.out.next();
            let mut lvl = 0.0f64;
            let mut ys = [0.0f64; 8];
            for (ci, ch) in chans.iter().enumerate().take(8) {
                let j = ci.min(last);
                let x = ch[k] as f64;
                let low = self.shift[j].process(x, 0.5);
                let s = low * oct * 1.4 + x * gtr;
                let s = tube(s * dg, 0.15) / dg.sqrt();
                let s = cascade(&mut self.eq[j], s);
                lvl = lvl.max(s.abs());
                ys[ci] = s;
            }
            let c = if lvl > self.env { ca } else { cr };
            self.env = lvl + c * (self.env - lvl);
            let gr = comp_gr(gain_to_db(self.env.max(1e-9)), -28.0, 1.0 + 7.0 * even, 8.0);
            let g = db_to_gain(-gr + gr * 0.5 * even);
            for (ci, ch) in chans.iter_mut().enumerate().take(8) {
                ch[k] = (ys[ci] * g * o) as f32;
            }
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "The Less I Know the Better", values: &[] },
        Preset { name: "Rounder, Darker", values: &[(TONE, 800.0), (THUMP, 6.0), (DRIVE, 0.2)] },
        Preset { name: "Octave Doubled Riff", values: &[(GUITAR, 0.6), (OCTAVE, 0.8), (TONE, 3000.0), (THUMP, 2.0)] },
        Preset { name: "Driven Synth Bass", values: &[(DRIVE, 0.75), (TONE, 2000.0), (EVENNESS, 0.8)] },
        Preset { name: "Clean Low Octave", values: &[(DRIVE, 0.0), (EVENNESS, 0.3), (TONE, 1800.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "OCTAVE BASS",
        subtitle: ("GUITAR AN OCTAVE DOWN AS A P-BASS", "THE HOTEL-ROOM BASS OF THE LESS I KNOW THE BETTER"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::Knob { p: OCTAVE, x: 100.0, y: 160.0, r: 30.0, label: "OCTAVE DOWN", tip: "The guitar an octave down.", active: None },
            Item::Knob { p: GUITAR, x: 220.0, y: 160.0, r: 24.0, label: "GUITAR", tip: "The guitar itself, mixed back in.", active: None },
            Item::Knob { p: DRIVE, x: 330.0, y: 160.0, r: 24.0, label: "DRIVE", tip: "A little drive, as on the record.", active: None },
            Item::Knob { p: EVENNESS, x: 440.0, y: 160.0, r: 24.0, label: "COMPRESSION", tip: "Evens the notes out.", active: None },
            Item::Knob { p: OUTPUT, x: 550.0, y: 160.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },
            Item::Divider { y: 250.0 },
            Item::Knob { p: THUMP, x: 220.0, y: 330.0, r: 24.0, label: "THUMP", tip: "The P-Bass's low-mid thump around 110 Hz.", active: None },
            Item::Knob { p: TONE, x: 420.0, y: 330.0, r: 24.0, label: "TONE", tip: "Where the top rolls off, as a bass's tone control.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Octave Bass", class_id: *b"TImpOctaveBassG1", vendor: "PEQUOD Tame Impala",
    categories: [Fx, PitchShift]
);

#[cfg(test)]
#[allow(dead_code)]
pub mod tests {
    use super::*;
    use std::f64::consts::PI;

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
    pub fn sine(f: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n).map(|k| (amp * (2.0 * PI * f * k as f64 / 48000.0).sin()) as f32).collect()
    }
    pub fn noise(n: usize, amp: f64) -> Vec<f32> {
        let mut nz = vintage::dsp::Noise::new(11);
        (0..n).map(|_| (nz.next() * amp) as f32).collect()
    }
    /// Both channels out for a mono input.
    pub fn go2(p: &[f32], x: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        let r = input.pop().unwrap();
        (input.pop().unwrap(), r)
    }
    pub fn go(p: &[f32], x: &[f32]) -> Vec<f32> {
        go2(p, x).0
    }
    /// Steady-state level of a sine at f through the settings, in dB.
    pub fn level(set: &[(usize, f32)], f: f64) -> f64 {
        let p = with(defaults(), set);
        let y = go(&p, &sine(f, 0.1, 48000));
        db(rms(&y[24000..]))
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
            let l: Vec<f32> = (0..n).map(|k| if (k / 6000) % 2 == 0 { (nz.next() * 0.5) as f32 } else { 0.0 }).collect();
            let mut input = vec![l.clone(), l];
            run(&p, &mut input);
            assert!(input.iter().flatten().all(|v| v.is_finite() && v.abs() < 8.0), "unbounded output for {:?}", p);
        }
    }

    #[test]
    fn plays_an_octave_down() {
        let p = with(defaults(), &[(DRIVE, 0.0), (EVENNESS, 0.0), (TONE, 5000.0)]);
        let y = go(&p, &sine(220.0, 0.2, 48000));
        let s: Vec<f64> = y[24000..].iter().map(|v| *v as f64).collect();
        let band = |f: f64| {
            let mut b = Biquad::identity();
            b.set_bandpass(48000.0, f, 6.0);
            let o: Vec<f32> = s.iter().map(|v| b.process(*v) as f32).collect();
            rms(&o[4000..])
        };
        println!("110 Hz {:.4}, 220 Hz {:.4}", band(110.0), band(220.0));
        assert!(band(110.0) > 2.0 * band(220.0));
    }

}
