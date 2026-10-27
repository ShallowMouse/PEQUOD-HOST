//! Shimmer: a long reverb with a pitch shifter in its feedback loop, so the tail keeps climbing an octave (or a fifth) as it decays: the angelic, organ-like wash of ambient and post-rock guitar. SHIMMER sets how much of the shifted sound is fed back.

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
use vintage::verb::Fdn;
use vintage::fx::PitchShift;

pub const W: u32 = 640;
pub const H: u32 = 330;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Interval { #[name = "Octave up"] Oct, #[name = "Fifth up"] Fifth, #[name = "Octave + fifth"] OctFifth, #[name = "Octave down"] Down }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "shimmer"]
    pub shimmer: FloatParam,
    #[id = "interval"]
    pub interval: EnumParam<Interval>,
    #[id = "tone"]
    pub tone: FloatParam,
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
            decay: secs("Decay", 5.0, 1.0, 20.0),
            shimmer: pct("Shimmer", 0.5),
            interval: EnumParam::new("Interval", Interval::Oct),
            tone: hz("Tone", 6000.0, 1500.0, 14000.0),
            mix: pct("Mix", 0.4),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const DECAY: usize = 0;
pub const SHIMMER: usize = 1;
pub const INTERVAL: usize = 2;
pub const TONE: usize = 3;
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
            self.decay.as_ptr(),
            self.shimmer.as_ptr(),
            self.interval.as_ptr(),
            self.tone.as_ptr(),
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

const RATIOS: [f64; 4] = [2.0, 1.4983, 2.9966, 0.5];

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    fdn: Fdn,
    ps: PitchShift,
    hp: Biquad,
    lp: [Biquad; 2],
    fb: f64,
    key: Vec<i32>,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let rt = self.v[DECAY] as f64;
        self.fdn.configure(3.2, [rt, rt, rt * 0.6], 3500.0);
        self.fdn.mod_depth = 0.002;
        let f = self.v[TONE] as f64;
        let fs = self.fs;
        self.lp.iter_mut().for_each(|b| b.set_lowpass(fs, f, 0.7));
        self.hp.set_highpass(fs, 200.0, 0.7);
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, fdn: Fdn::new(fs, 3.3), ps: PitchShift::new(fs, 80.0), hp: Biquad::identity(), lp: [Biquad::identity(); 2], fb: 0.0, key: vec![], out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.fdn.reset();
        self.ps.reset();
        self.hp.reset();
        self.lp.iter_mut().for_each(|b| b.reset());
        self.fb = 0.0;
        self.design();
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k: Vec<i32> = [v[DECAY], v[TONE] / 100.0].iter().map(|x| (x * 100.0) as i32).collect();
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let mix = self.v[MIX] as f64;
        let (dg, wg) = ((1.0 - mix).sqrt(), mix.sqrt() * 1.4);
        let sh = self.v[SHIMMER] as f64 * 0.55;
        let ratio = RATIOS[(self.v[INTERVAL].round() as usize).min(3)];
        let nch = chans.len();
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            let d = self.fdn.diffuse(x + self.fb, 4);
            let (l, r) = self.fdn.process(d);
            let l = self.lp[0].process(l);
            let r = self.lp[1].process(r);
            let up = self.ps.process(self.hp.process(0.5 * (l + r)), ratio);
            self.fb = (up * sh * 1.5).tanh() / 1.5;
            let o = self.out.next();
            if nch >= 2 {
                chans[0][k] = ((chans[0][k] as f64 * dg + l * wg) * o) as f32;
                chans[1][k] = ((chans[1][k] as f64 * dg + r * wg) * o) as f32;
            } else if nch == 1 {
                chans[0][k] = ((chans[0][k] as f64 * dg + 0.5 * (l + r) * wg) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Angelic Octaves", values: &[] },
        Preset { name: "Ambient Fifths", values: &[(INTERVAL, 1.0), (SHIMMER, 0.45)] },
        Preset { name: "Huge Wash", values: &[(DECAY, 12.0), (SHIMMER, 0.65), (MIX, 0.5)] },
        Preset { name: "Subtle Sparkle", values: &[(SHIMMER, 0.25), (DECAY, 3.5), (MIX, 0.3)] },
        Preset { name: "Organ Pad (octave + fifth)", values: &[(INTERVAL, 2.0), (SHIMMER, 0.5), (TONE, 5000.0)] },
        Preset { name: "Dark Descent (octave down)", values: &[(INTERVAL, 3.0), (SHIMMER, 0.45), (TONE, 3500.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "SHIMMER",
        subtitle: ("PITCH-SHIFTED REVERB", "TAILS THAT CLIMB AS THEY DECAY"),
        accent: ui::TEAL,
        size: (W, H),
        items: vec![
            Item::Knob { p: DECAY, x: 70.0, y: 190.0, r: 32.0, label: "DECAY", tip: "Reverb time.", active: None },
            Item::Knob { p: SHIMMER, x: 170.0, y: 190.0, r: 32.0, label: "SHIMMER", tip: "How much pitch-shifted sound feeds back into the reverb.", active: None },
            Item::Selector { p: INTERVAL, x: 270.0, y: 190.0, label: "INTERVAL", positions: &["8VA", "5TH", "8+5", "8VB"], tip: "The shift in the loop." },
            Item::Knob { p: TONE, x: 370.0, y: 190.0, r: 32.0, label: "TONE", tip: "High cut on the tail.", active: None },
            Item::Knob { p: MIX, x: 470.0, y: 190.0, r: 32.0, label: "MIX", tip: "Reverb level.", active: None },
            Item::Knob { p: OUTPUT, x: 570.0, y: 190.0, r: 32.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Shimmer", class_id: *b"PqdShimmerVerb01", vendor: "PEQUOD Pedals",
    categories: [Fx, Reverb]
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
    fn tail_climbs() {
        let mut x = vec![0.0f32; 48000 * 3];
        for k in 0..12000 {
            x[k] = (0.3 * (2.0 * PI * 220.0 * k as f64 / 48000.0).sin()) as f32;
        }
        let off = stereo(&with(defaults(), &[(SHIMMER, 0.0), (MIX, 1.0)]), &x).0;
        let on = stereo(&with(defaults(), &[(SHIMMER, 0.8), (MIX, 1.0)]), &x).0;
        let r = |y: &[f32]| db(goertzel(&y[72000..], 440.0) / goertzel(&y[72000..], 220.0).max(1e-12));
        println!("440/220 in the tail: off {:.1} dB, on {:.1} dB", r(&off), r(&on));
        assert!(r(&on) > r(&off) + 6.0 && on.iter().all(|v| v.is_finite() && v.abs() < 4.0));
    }

}
