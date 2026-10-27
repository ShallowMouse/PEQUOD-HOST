//! Bassman 6G6-B.
//!
//! Fender Bassman 6G6-B blonde piggyback, typical values: normal channel
//! with a 7025 input, the deep 1M bass pot, two 6L6s in fixed bias, silicon
//! rectifier and feedback, into the closed 2x12.
use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
use vintage::amp::cab;
use vintage::amp::model::Controls;
use vintage::amp::plugin::{AmpDsp, AmpModel, Settings, CAB_POSITIONS};
use vintage::amp::specs;
use vintage::nih_plug_egui::EguiState;
use vintage::params::*;
use vintage::ui::{self, Item, MeterKind, Panel, Preset};
use vintage::VintageParams;

const W: u32 = 640;
const H: u32 = 432;

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Cab {
    #[name = "Built-in (miked)"]
    BuiltIn,
    #[name = "IR"]
    Ir,
    #[name = "DI (no cab)"]
    Di,
    #[name = "Speaker (no mic)"]
    Speaker,
}
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum BrightSw {
    #[name = "OFF"]
    A,
    #[name = "ON"]
    B,
}
#[derive(Params)]
pub struct AmpParams {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[persist = "ir-path"]
    ir_path: Arc<RwLock<String>>,
    #[id = "input"]
    input: FloatParam,
    #[id = "volume"]
    volume: FloatParam,
    #[id = "treble"]
    treble: FloatParam,
    #[id = "bass"]
    bass: FloatParam,
    #[id = "bright"]
    bright: EnumParam<BrightSw>,
    #[id = "cab"]
    cab: EnumParam<Cab>,
    #[id = "mic"]
    mic: FloatParam,
    #[id = "output"]
    output: FloatParam,
}

fn dial(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 1.0, max: 10.0 })
        .with_step_size(0.1)
        .with_value_to_string(formatters::v2s_f32_rounded(1))
}

impl Default for AmpParams {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            ir_path: Arc::new(RwLock::new(String::new())),
            input: db("Guitar Level", 0.0, -12.0, 12.0),
            volume: dial("Volume", 4.0),
            treble: dial("Treble", 6.0),
            bass: dial("Bass", 4.0),
            bright: EnumParam::new("Bright", BrightSw::A),
            cab: EnumParam::new("Cabinet", Cab::BuiltIn),
            mic: pct("Mic Position", 0.3),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

const INPUT: usize = 0;
const VOLUME: usize = 1;
const TREBLE: usize = 2;
const BASS: usize = 3;
const BRIGHT: usize = 4;
const CAB: usize = 5;
const MIC: usize = 6;
const OUTPUT: usize = 7;

impl VintageParams for AmpParams {
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
            self.bass.as_ptr(),
            self.bright.as_ptr(),
            self.cab.as_ptr(),
            self.mic.as_ptr(),
            self.output.as_ptr(),
        ]
    }
    fn ir_path(&self) -> Option<Arc<RwLock<String>>> {
        Some(self.ir_path.clone())
    }
}

pub struct Model;

fn rot(v: f32) -> f64 {
    ((v as f64 - 1.0) / (10.0 - 1.0)).clamp(0.0, 1.0)
}

impl AmpModel for Model {
    fn spec() -> vintage::amp::model::AmpSpec {
        specs::bassman()
    }
    fn cab() -> cab::CabSpec {
        cab::FENDER_2X12_JENSEN
    }
    fn settings(v: &[f32]) -> Settings {
        Settings {
            controls: Controls {
                volume: rot(v[VOLUME]),
                treble: rot(v[TREBLE]),
                bass: rot(v[BASS]),
                middle: 0.5,
                cut: 0.0,
                bright: v[BRIGHT] > 0.5,
                alt: false,
            },
            input_db: v[INPUT],
            output_db: v[OUTPUT],
            cab: (v[CAB].round() as usize).min(3),
            mic: v[MIC],
        }
    }
}

pub type Dsp = AmpDsp<Model>;

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Clean", values: &[(VOLUME, 4e+00), (TREBLE, 6e+00), (BASS, 4e+00)] },
        Preset { name: "Edge of Breakup", values: &[(VOLUME, 6e+00), (TREBLE, 6e+00), (BASS, 4e+00)] },
        Preset { name: "Bright Rhythm", values: &[(VOLUME, 4e+00), (TREBLE, 7e+00), (BASS, 3e+00), (BRIGHT, 1e+00)] },
        Preset { name: "Cranked", values: &[(VOLUME, 1e+01), (TREBLE, 6e+00), (BASS, 4e+00)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "BASSMAN",
        subtitle: ("AFTER THE FENDER 6G6-B PIGGYBACK", "2 x 6L6  ·  BLONDE TOLEX, 1964"),
        accent: ui::OXBLOOD,
        size: (W, H),
        items: vec![
            Item::Knob { p: VOLUME, x: 80.0, y: 146.0, r: 30.0, label: "VOLUME", tip: "Normal channel volume.", active: None },
            Item::Knob { p: TREBLE, x: 265.0, y: 146.0, r: 30.0, label: "TREBLE", tip: "Treble (250k).", active: None },
            Item::Knob { p: BASS, x: 450.0, y: 146.0, r: 30.0, label: "BASS", tip: "Bass (1M: goes much deeper than a blackface).", active: None },
            Item::Text { x: 560.0, y: 98.0, text: "BRIGHT", size: 13.0, align: vintage::nih_plug_egui::egui::Align2::CENTER_CENTER },
            Item::Switch { p: BRIGHT, x: 560.0, y: 146.0, labels: ["OFF", "ON"], tip: "Bright switch: a small cap across the volume pot. Most noticeable at low volume settings." },
            Item::Divider { y: 206.0 },
            Item::Knob { p: INPUT, x: 70.0, y: 256.0, r: 20.0, label: "GUITAR LEVEL",
                tip: "How hot the guitar hits the input. 0 dB treats a full-scale signal as 1 V from the pickups; turn up for a quiet DI.", active: None },
            Item::Selector { p: CAB, x: 170.0, y: 256.0, label: "CABINET", positions: CAB_POSITIONS,
                tip: "MIC'D: the amp's own cabinet, close-miked. IR: your loaded impulse response. DI: the amp output with no cabinet (for your own cab plugin). SPEAKER: the cabinet with no microphone, to mic it up with the Mic plugin and/or Studio Spaces." },
            Item::Knob { p: MIC, x: 270.0, y: 256.0, r: 20.0, label: "MIC POSITION",
                tip: "Built-in cabinet: microphone from the cone centre (bright) out toward the edge (darker, rounder).",
                active: Some(ui::ActiveWhen { p: CAB, min: 0.0, max: 0.0 }) },
            Item::Knob { p: OUTPUT, x: 366.0, y: 256.0, r: 20.0, label: "OUTPUT", tip: "Plugin output level. Doesn't change the amp's tone.", active: None },
            Item::Meter { m: 0, x: 424.0, y: 214.0, w: 186.0, h: 110.0, kind: MeterKind::Vu, caption: "POWER" },
            Item::IrLoader { x: 46.0, y: 320.0, w: 360.0, cab: CAB, ir_pos: 1 },
            Item::Lamp { m: 1, x: 496.0, y: 344.0, label: "SAG", lo: 2.0, hi: 25.0 },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, AmpParams, Dsp, panel,
    name: "Bassman 6G6-B", class_id: *b"S2AmpBassmanDIY1", vendor: "PEQUOD Beatles",
    categories: [Fx, Distortion]
);

#[cfg(test)]
mod tests {
    use super::*;
    use vintage::VintageDsp;

    fn defaults() -> Vec<f32> {
        let p = AmpParams::default();
        p.linked().iter().map(|ptr| unsafe { ptr.preview_plain(ptr.default_normalized_value()) }).collect()
    }

    #[test]
    fn makes_sound_and_reports_latency() {
        let mut d = Dsp::new(48000.0, 2);
        d.set_params(&defaults());
        assert_eq!(d.latency(), (vintage::dsp::OS_LATENCY + vintage::amp::ir::BLOCK) as u32);
        let mut l: Vec<f32> = (0..24000).map(|k| (0.2 * (k as f64 * 2.0 * std::f64::consts::PI * 196.0 / 48000.0).sin()) as f32).collect();
        let mut r = l.clone();
        let mut m = [0.0f32; 2];
        d.process(&mut [&mut l[..], &mut r[..]], &mut m);
        let peak = l[12000..].iter().fold(0.0f32, |a, v| a.max(v.abs()));
        assert!(peak > 0.01 && peak < 2.0, "peak {peak}");
        assert!(l.iter().all(|v| v.is_finite()));
        assert_eq!(l, r);
    }
}
