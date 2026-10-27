//! Showman Head: the Fender Showman (AB763 circuit, two 6L6s) on its own. Amp only: no speaker or cabinet, so follow it with a PEQUOD Cabs plugin (or an IR).

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
use vintage::amp::cab;
use vintage::amp::model::{AmpSpec, Controls};
#[allow(unused_imports)]
use vintage::amp::plugin::{AmpDsp, AmpModel, Settings, CAB_BUILT_IN, CAB_DI, CAB_SPEAKER};
use vintage::amp::specs;

pub const W: u32 = 640;
pub const H: u32 = 410;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "input"]
    pub input: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
    #[id = "treble"]
    pub treble: FloatParam,
    #[id = "middle"]
    pub middle: FloatParam,
    #[id = "bass"]
    pub bass: FloatParam,
    #[id = "bright"]
    pub bright: BoolParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            input: db("Guitar Level", 0.0, -12.0, 12.0),
            volume: FloatParam::new("Volume", 4.0, FloatRange::Linear { min: 1.0, max: 10.0 }).with_step_size(0.1).with_value_to_string(formatters::v2s_f32_rounded(1)),
            treble: FloatParam::new("Treble", 6.0, FloatRange::Linear { min: 1.0, max: 10.0 }).with_step_size(0.1).with_value_to_string(formatters::v2s_f32_rounded(1)),
            middle: FloatParam::new("Middle", 5.0, FloatRange::Linear { min: 1.0, max: 10.0 }).with_step_size(0.1).with_value_to_string(formatters::v2s_f32_rounded(1)),
            bass: FloatParam::new("Bass", 5.0, FloatRange::Linear { min: 1.0, max: 10.0 }).with_step_size(0.1).with_value_to_string(formatters::v2s_f32_rounded(1)),
            bright: BoolParam::new("Bright", false),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const INPUT: usize = 0;
pub const VOLUME: usize = 1;
pub const TREBLE: usize = 2;
pub const MIDDLE: usize = 3;
pub const BASS: usize = 4;
pub const BRIGHT: usize = 5;
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
            self.input.as_ptr(),
            self.volume.as_ptr(),
            self.treble.as_ptr(),
            self.middle.as_ptr(),
            self.bass.as_ptr(),
            self.bright.as_ptr(),
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

pub struct Model;

fn rot(v: f32) -> f64 {
    ((v as f64 - 1.0) / 9.0).clamp(0.0, 1.0)
}

impl AmpModel for Model {
    fn spec() -> AmpSpec {
        specs::showman()
    }
    fn cab() -> cab::CabSpec {
        cab::VOX_2X12_BLUE
    }
    fn settings(v: &[f32]) -> Settings {
        Settings {
            controls: Controls {
                volume: rot(v[VOLUME]),
                treble: rot(v[TREBLE]),
                bass: rot(v[BASS]),
                middle: rot(v[MIDDLE]),
                cut: 0.0,
                bright: v[BRIGHT] > 0.5,
                alt: false,
            },
            input_db: v[INPUT],
            output_db: v[OUTPUT],
            cab: CAB_DI,
            mic: 0.3,
        }
    }
}

pub type Dsp = AmpDsp<Model>;


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Surf Clean", values: &[(VOLUME, 4.0), (BRIGHT, 1.0)] },
        Preset { name: "Big Clean", values: &[(VOLUME, 6.0)] },
        Preset { name: "Loud", values: &[(VOLUME, 9.0)] },
        Preset { name: "Jazz", values: &[(VOLUME, 3.0), (TREBLE, 3.5)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "SHOWMAN HEAD",
        subtitle: ("FENDER SHOWMAN AB763, AMP ONLY", "2 x 6L6 · SURF CLEAN"),
        accent: ui::OXBLOOD,
        size: (W, H),
        items: vec![
            Item::Knob { p: VOLUME, x: 90.0, y: 150.0, r: 30.0, label: "VOLUME", tip: "Volume.", active: None },
            Item::Knob { p: TREBLE, x: 243.3, y: 150.0, r: 30.0, label: "TREBLE", tip: "Treble.", active: None },
            Item::Knob { p: MIDDLE, x: 396.7, y: 150.0, r: 30.0, label: "MIDDLE", tip: "Middle.", active: None },
            Item::Knob { p: BASS, x: 550.0, y: 150.0, r: 30.0, label: "BASS", tip: "Bass.", active: None },
            Item::Divider { y: 212.0 },
            Item::Knob { p: INPUT, x: 70.0, y: 270.0, r: 22.0, label: "GUITAR LEVEL", tip: "How hot the guitar hits the input (0 dB = 1 V peak at full scale). Turn up for a quiet DI.", active: None },
            Item::Switch { p: BRIGHT, x: 140.0, y: 270.0, labels: ["", "BRIGHT"], tip: "Bright switch." },
            Item::Knob { p: OUTPUT, x: 250.0, y: 270.0, r: 22.0, label: "OUTPUT", tip: "Output level. This is the amp alone, with no speaker: follow it with a PEQUOD Cabs plugin or an IR.", active: None },
            Item::Meter { m: 0, x: 424.0, y: 222.0, w: 186.0, h: 110.0, kind: MeterKind::Vu, caption: "POWER" },
            Item::Lamp { m: 1, x: 496.0, y: 352.0, label: "SAG", lo: 2.0, hi: 25.0 },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Showman Head", class_id: *b"PqAHShowman00000", vendor: "PEQUOD Amps",
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

    fn tone(p: &[f32], amp: f64) -> Vec<f32> {
        let x: Vec<f32> = (0..48000).map(|k| (amp * (2.0 * std::f64::consts::PI * 220.0 * k as f64 / 48000.0).sin()) as f32).collect();
        let mut input = vec![x.clone(), x];
        run(p, &mut input);
        input.remove(0)
    }
    fn goertzel(x: &[f32], f: f64) -> f64 {
        let w = 2.0 * std::f64::consts::PI * f / 48000.0;
        let (mut s1, mut s2) = (0.0, 0.0);
        for v in x {
            let s = *v as f64 + 2.0 * w.cos() * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        (s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2).max(0.0).sqrt()
    }
    fn harm(y: &[f32]) -> f64 {
        let h: f64 = (2..7).map(|k| goertzel(&y[24000..], 220.0 * k as f64).powi(2)).sum::<f64>().sqrt();
        db(h / goertzel(&y[24000..], 220.0).max(1e-12))
    }

    #[test]
    fn breaks_up_when_turned_up() {
        let lo = harm(&tone(&with(defaults(), &[(VOLUME, 2.0)]), 0.2));
        let hi = harm(&tone(&with(defaults(), &[(VOLUME, 10.0)]), 0.2));
        println!("harmonics volume 2 {lo:.1} dB, volume 10 {hi:.1} dB");
        assert!(hi > lo + 6.0);
    }

}
