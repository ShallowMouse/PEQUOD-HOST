//! AC30 Normal: the Vox AC30/6 played through its Normal channel, as Brian May does: no Top Boost stage and no tone controls, just the
//! volume, driven hard by a treble booster. Four EL84s, cathode bias, no feedback, GZ34 sag, the Cut control.
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
pub enum AltSw {
    #[name = "50R '63"]
    A,
    #[name = "82R '62"]
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
    #[id = "cut"]
    cut: FloatParam,
    #[id = "alt"]
    alt: EnumParam<AltSw>,
    #[id = "cab"]
    cab: EnumParam<Cab>,
    #[id = "mic"]
    mic: FloatParam,
    #[id = "output"]
    output: FloatParam,
}

fn dial(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 10.0 })
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
            volume: dial("Volume", 7.0),
            cut: dial("Cut", 2.0),
            alt: EnumParam::new("Cathode", AltSw::A),
            cab: EnumParam::new("Cabinet", Cab::BuiltIn),
            mic: pct("Mic Position", 0.3),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

const INPUT: usize = 0;
const VOLUME: usize = 1;
const CUT: usize = 2;
const ALT: usize = 3;
const CAB: usize = 4;
const MIC: usize = 5;
const OUTPUT: usize = 6;

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
            self.cut.as_ptr(),
            self.alt.as_ptr(),
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
    ((v as f64 - 0.0) / (10.0 - 0.0)).clamp(0.0, 1.0)
}

impl AmpModel for Model {
    fn spec() -> vintage::amp::model::AmpSpec {
        specs::ac30_normal()
    }
    fn cab() -> cab::CabSpec {
        cab::VOX_2X12_BLUE
    }
    fn settings(v: &[f32]) -> Settings {
        Settings {
            controls: Controls {
                volume: rot(v[VOLUME]),
                treble: 0.5,
                bass: 0.5,
                middle: 0.5,
                cut: rot(v[CUT]),
                bright: false,
                alt: v[ALT] > 0.5,
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
        Preset { name: "May Lead (with the Rangemaster)", values: &[(VOLUME, 8.0), (CUT, 2.0)] },
        Preset { name: "Full Up", values: &[(VOLUME, 10.0), (CUT, 3.0)] },
        Preset { name: "Rhythm Crunch", values: &[(VOLUME, 6.0), (CUT, 2.0)] },
        Preset { name: "Clean Normal Channel", values: &[(VOLUME, 3.0), (CUT, 2.0)] },
        Preset { name: "Warm Lead (More Cut)", values: &[(VOLUME, 8.0), (CUT, 6.0), (MIC, 0.5)] },
        Preset { name: "Early '62, 82 Ohm", values: &[(VOLUME, 7.0), (CUT, 2.0), (ALT, 1.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "AC30 NORMAL",
        subtitle: ("THE AC30/6 NORMAL CHANNEL, MAY'S WAY", "4 x EL84  ·  NO TONE STACK  ·  NO FEEDBACK"),
        accent: ui::OXBLOOD,
        size: (W, H),
        items: vec![
            Item::Knob { p: VOLUME, x: 140.0, y: 140.0, r: 32.0, label: "NORMAL VOLUME", tip: "Normal channel volume: one ECC83 stage, no Top Boost, no tone controls. May runs it high and lets the treble booster do the rest.", active: None },
            Item::Knob { p: CUT, x: 360.0, y: 146.0, r: 30.0, label: "CUT", tip: "Cut: a cap and pot across the EL84 grids. Turning it up shaves the top off the power stage.", active: None },
            Item::Text { x: 560.0, y: 98.0, text: "CATHODE", size: 13.0, align: vintage::nih_plug_egui::egui::Align2::CENTER_CENTER },
            Item::Switch { p: ALT, x: 560.0, y: 146.0, labels: ["50R '63", "82R '62"], tip: "Output cathode resistor: 50 ohm (later 1963) runs the EL84s hotter and louder; 82 ohm (to mid-1963) is a little cooler and breaks up sooner." },
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
    name: "AC30 Normal", class_id: *b"BMyAC30NormalPQ1", vendor: "PEQUOD Brian May",
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
