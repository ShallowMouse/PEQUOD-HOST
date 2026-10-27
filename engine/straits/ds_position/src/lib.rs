//! Position 4: makes any guitar sound like a Stratocaster's in-between pickup positions, the 'quack' of Sultans of Swing. Two pickups in parallel hear the string at two places, which cancels a band of the lower mids, and the pair's combined resonance sits higher and brighter. Tell it which pickup you played (SOURCE) and pick the position you want; QUACK sets how strong the effect is.

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
pub const H: u32 = 300;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Source { #[name = "Strat neck"] Neck, #[name = "Strat middle"] Middle, #[name = "Strat bridge"] Bridge, #[name = "Humbucker"] Humbucker }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Position { #[name = "1 Bridge"] P1, #[name = "2 Bridge + middle"] P2, #[name = "3 Middle"] P3, #[name = "4 Middle + neck"] P4, #[name = "5 Neck"] P5 }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "source"]
    pub source: EnumParam<Source>,
    #[id = "position"]
    pub position: EnumParam<Position>,
    #[id = "quack"]
    pub quack: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            source: EnumParam::new("Source", Source::Neck),
            position: EnumParam::new("Position", Position::P4),
            quack: FloatParam::new("Quack", 1.0, FloatRange::Linear { min: 0.0, max: 1.5 }).with_step_size(0.01),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const SOURCE: usize = 0;
pub const POSITION: usize = 1;
pub const QUACK: usize = 2;
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
            self.source.as_ptr(),
            self.position.as_ptr(),
            self.quack.as_ptr(),
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

/// A pickup's tone as (low shelf at 200 Hz dB, notch Hz, notch dB, resonance Hz, resonance dB, high shelf at 5 kHz dB).
#[derive(Clone, Copy)]
pub struct Tone(pub f64, pub f64, pub f64, pub f64, pub f64, pub f64);

pub const POSITIONS: [Tone; 5] = [
    Tone(-4.0, 1300.0, 0.0, 3800.0, 5.0, 3.0),
    Tone(-3.0, 1400.0, -6.0, 4500.0, 4.0, 3.0),
    Tone(-1.0, 1100.0, 0.0, 3200.0, 4.0, 1.0),
    Tone(-1.0, 1000.0, -6.0, 4000.0, 4.0, 2.0),
    Tone(2.0, 1000.0, 0.0, 2600.0, 3.0, -1.0),
];
pub const SOURCES: [Tone; 4] = [
    POSITIONS[4],
    POSITIONS[2],
    POSITIONS[0],
    Tone(2.0, 1000.0, 0.0, 2200.0, 2.0, -4.0),
];

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    eq: Vec<[Biquad; 5]>,
    key: Vec<i32>,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let t = POSITIONS[(self.v[POSITION].round() as usize).min(4)];
        let s = SOURCES[(self.v[SOURCE].round() as usize).min(3)];
        let q = self.v[QUACK] as f64;
        for e in self.eq.iter_mut() {
            e[0].set_low_shelf(fs, 200.0, 0.7, (t.0 - s.0) * q);
            e[1].set_peak(fs, t.1, 1.4, t.2 * q);
            e[2].set_peak(fs, s.3, 1.3, -s.4 * q);
            e[3].set_peak(fs, t.3, 1.3, t.4 * q);
            e[4].set_high_shelf(fs, 5000.0, 0.7, (t.5 - s.5) * q);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, eq: vec![[Biquad::identity(); 5]; ch.max(2)], key: vec![], out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.eq.iter_mut().for_each(|e| e.iter_mut().for_each(|b| b.reset()));
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k: Vec<i32> = v[..NPARAMS].iter().map(|x| (x * 100.0) as i32).collect();
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let last = self.eq.len() - 1;
        for k in 0..n {
            let o = self.out.next();
            for (ci, c) in chans.iter_mut().enumerate() {
                c[k] = (cascade(&mut self.eq[ci.min(last)], c[k] as f64) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Sultans (neck to position 4)", values: &[] },
        Preset { name: "Bridge to Position 2", values: &[(SOURCE, 2.0), (POSITION, 1.0)] },
        Preset { name: "Humbucker to Position 4", values: &[(SOURCE, 3.0), (POSITION, 3.0)] },
        Preset { name: "Humbucker to Strat Neck", values: &[(SOURCE, 3.0), (POSITION, 4.0)] },
        Preset { name: "Subtle Quack", values: &[(QUACK, 0.5)] },
        Preset { name: "Extra Quack", values: &[(QUACK, 1.4)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "POSITION 4",
        subtitle: ("STRAT IN-BETWEEN POSITIONS FROM ANY GUITAR", "THE SULTANS OF SWING QUACK"),
        accent: ui::COBALT,
        size: (W, H),
        items: vec![

            Item::Selector { p: SOURCE, x: 110.0, y: 170.0, label: "YOUR PICKUP", positions: &["NECK", "MID", "BRIDGE", "HB"], tip: "The pickup you actually played." },
            Item::Selector { p: POSITION, x: 280.0, y: 170.0, label: "POSITION", positions: &["1", "2", "3", "4", "5"], tip: "The Strat 5-way position you want: 2 and 4 are the in-between 'quack' positions." },
            Item::Knob { p: QUACK, x: 430.0, y: 170.0, r: 36.0, label: "QUACK", tip: "How strong the change is.", active: None },
            Item::Knob { p: OUTPUT, x: 560.0, y: 170.0, r: 26.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Position 4", class_id: *b"DSPosition4Quack", vendor: "PEQUOD Dire Straits",
    categories: [Fx, Eq]
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

    fn lvl(p: &[f32], f: f64) -> f64 {
        let y = stereo(p, &sine(f, 0.1, 24000)).0;
        db(rms(&y[12000..])) - db(0.1 / 2f64.sqrt())
    }
    #[test]
    fn position_4_notches_the_low_mids() {
        let same = with(defaults(), &[(SOURCE, 0.0), (POSITION, 4.0)]);
        let p4 = with(defaults(), &[(SOURCE, 0.0), (POSITION, 3.0)]);
        println!("neck->neck 1 kHz {:.1} dB; neck->4 1 kHz {:.1} dB, 4 kHz {:.1} dB", lvl(&same, 1000.0), lvl(&p4, 1000.0), lvl(&p4, 4000.0));
        assert!(lvl(&same, 1000.0).abs() < 0.5 && lvl(&p4, 1000.0) < -4.0 && lvl(&p4, 4000.0) > 1.0);
    }

}
