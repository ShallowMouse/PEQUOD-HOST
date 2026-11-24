//! BBE Sonic Maximizer: BBE's 422/482 (1985), loved and argued over in equal measure. Two ideas, after
//! BBE's description: the lows are delayed a little behind the highs, so the attack of each note comes
//! through first (the way a speaker smears it the other way round), and the highs are lifted in step with
//! the programme. LO CONTOUR adds bass, PROCESS the top. Estimated.

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
    #[id = "lo"]
    pub lo: FloatParam,
    #[id = "process"]
    pub process: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            lo: pct("Lo Contour", 0.3),
            process: pct("Process", 0.4),
        }
    }
}

pub const LO: usize = 0;
pub const PROCESS: usize = 1;
pub const NPARAMS: usize = 2;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.lo.as_ptr(),
            self.process.as_ptr(),
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
    split_lo: Vec<[Biquad; 2]>,
    split_hi: Vec<[Biquad; 2]>,
    delay_lo: Vec<DelayLine>,
    delay_mid: Vec<DelayLine>,
    env: f64,
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let n = ch.max(2);
        let mut lo = Biquad::identity();
        lo.set_lowpass(fs, 150.0, 0.7);
        let mut hi = Biquad::identity();
        hi.set_highpass(fs, 1200.0, 0.7);
        Self { fs, v, split_lo: vec![[lo; 2]; n], split_hi: vec![[hi; 2]; n],
               delay_lo: (0..n).map(|_| DelayLine::new((fs * 0.004) as usize + 8)).collect(),
               delay_mid: (0..n).map(|_| DelayLine::new((fs * 0.004) as usize + 8)).collect(), env: 0.0 }
    }
    fn reset(&mut self) {
        self.split_lo.iter_mut().chain(self.split_hi.iter_mut()).flatten().for_each(|b| b.reset());
        self.delay_lo.iter_mut().chain(self.delay_mid.iter_mut()).for_each(|d| d.reset());
        self.env = 0.0;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let lo_g = db_to_gain(self.v[LO] as f64 * 10.0);
        let hi_g = self.v[PROCESS] as f64 * 3.0;
        let ce = coeff(self.fs, 30.0);
        // the lows 2.5 ms behind, the mids 0.5 ms
        let dl = 0.0025 * self.fs;
        let dm = 0.0005 * self.fs;
        for k in 0..n {
            let lvl = chans.iter().map(|c| (c[k] as f64).abs()).fold(0.0, f64::max);
            self.env = lvl + ce * (self.env - lvl);
            // the top lifted with the programme (more on what's quiet, as BBE described it)
            let dyn_g = hi_g * (1.0 / (1.0 + 4.0 * self.env));
            for (ci, ch) in chans.iter_mut().enumerate() {
                let j = ci.min(self.split_lo.len() - 1);
                let x = ch[k] as f64;
                let l0 = self.split_lo[j][0].process(x);
                let l = self.split_lo[j][1].process(l0);
                let h0 = self.split_hi[j][0].process(x);
                let h = self.split_hi[j][1].process(h0);
                let m = x - l - h;
                self.delay_lo[j].write(l);
                self.delay_mid[j].write(m);
                let y = self.delay_lo[j].read(dl) * lo_g + self.delay_mid[j].read(dm) + h * (1.0 + dyn_g);
                ch[k] = y as f32;
            }
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Clarity", values: &[(LO, 0.2), (PROCESS, 0.5)] },
        Preset { name: "Bass Guitar Definition", values: &[(LO, 0.45), (PROCESS, 0.35)] },
        Preset { name: "Dull Recording Lifted", values: &[(LO, 0.3), (PROCESS, 0.75)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "SONIC MAXIMIZER",
        subtitle: ("BBE 422, 1985", "THE LOWS A LITTLE LATE, THE HIGHS LIFTED WITH THE PROGRAMME"),
        accent: ui::TANGERINE,
        size: (W, H),
        items: vec![
            Item::Knob { p: LO, x: 220.0, y: 200.0, r: 44.0, label: "LO CONTOUR", tip: "Bass lift.", active: None },
            Item::Knob { p: PROCESS, x: 420.0, y: 200.0, r: 44.0, label: "PROCESS", tip: "How much the top is lifted (more on quiet passages).", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Sonic Maximizer", class_id: *b"HWBBESonicMaxPQD", vendor: "PEQUOD Hardware",
    categories: [Fx, Eq]
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
    fn makes_sound() {
        let p = with(defaults(), &[]);
        let y = go(&p, &sine(440.0, 0.2, 48000));
        let l = db(rms(&y[24000..]));
        println!("level {:.1} dB", l);
        assert!(l > -40.0 && y.iter().all(|v| v.is_finite()));
    }

}
