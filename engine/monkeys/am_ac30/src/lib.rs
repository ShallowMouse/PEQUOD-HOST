//! AC30 (FWN): the Vox AC30 Alex Turner played on Favourite Worst Nightmare, through its Top Boost channel.
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
    #[id = "treble"]
    treble: FloatParam,
    #[id = "bass"]
    bass: FloatParam,
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
            volume: dial("Volume", 3.0),
            treble: dial("Treble", 6.0),
            bass: dial("Bass", 4.0),
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
const TREBLE: usize = 2;
const BASS: usize = 3;
const CUT: usize = 4;
const ALT: usize = 5;
const CAB: usize = 6;
const MIC: usize = 7;
const OUTPUT: usize = 8;

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
        specs::ac30()
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
        Preset { name: "Brianstorm (with the Rat)", values: &[(VOLUME, 5.0), (TREBLE, 7.0), (BASS, 4.0), (CUT, 3.0)] },
        Preset { name: "Fluorescent Adolescent Clean", values: &[(VOLUME, 3.0), (TREBLE, 6.5), (BASS, 4.5), (CUT, 2.0)] },
        Preset { name: "505 Swell", values: &[(VOLUME, 4.0), (TREBLE, 5.5), (BASS, 5.0), (CUT, 4.0), (MIC, 0.45)] },
        Preset { name: "Cranked Rhythm", values: &[(VOLUME, 8.0), (TREBLE, 6.0), (BASS, 4.0), (CUT, 3.0)] },
        Preset { name: "Dark Humbug Room", values: &[(VOLUME, 6.0), (TREBLE, 4.0), (BASS, 6.0), (CUT, 6.0), (MIC, 0.6)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "AC30 (FWN)",
        subtitle: ("THE VOX AC30 OF FAVOURITE WORST NIGHTMARE", "4 x EL84  ·  CLASS A  ·  TOP BOOST"),
        accent: ui::OXBLOOD,
        size: (W, H),
        items: vec![
            Item::Knob { p: VOLUME, x: 80.0, y: 146.0, r: 30.0, label: "VOLUME", tip: "Brilliant channel volume, into the Top Boost stage. The AC30 has no master: this sets both loudness and breakup.", active: None },
            Item::Knob { p: TREBLE, x: 203.3, y: 146.0, r: 30.0, label: "TREBLE", tip: "Top Boost treble (1M log pot). Interacts with Bass, as the real network does.", active: None },
            Item::Knob { p: BASS, x: 326.7, y: 146.0, r: 30.0, label: "BASS", tip: "Top Boost bass (1M log pot).", active: None },
            Item::Knob { p: CUT, x: 450.0, y: 146.0, r: 30.0, label: "CUT", tip: "Cut: a cap and pot across the EL84 grids. Turning it up shaves the top off the power stage.", active: None },
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
    name: "AC30 FWN", class_id: *b"AMkAC30FWNPQD001", vendor: "PEQUOD Arctic Monkeys",
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
