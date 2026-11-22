//! DR103 Bass: Roger Waters' Hiwatt DR103 Custom 100 for bass (1970s), the same amp as Gilmour's set up for bass, into a Hiwatt 4 x 12.

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
#[allow(unused_imports)]
use vintage::amp::model::*;
use vintage::amp::cab;
#[allow(unused_imports)]
use vintage::amp::tube::*;
use vintage::amp::plugin::{AmpDsp, AmpModel, Settings, CAB_POSITIONS};
#[allow(unused_imports)]
use vintage::amp::specs;

pub const W: u32 = 640;
pub const H: u32 = 432;

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum Channel {
    #[name = "Normal"]
    Normal,
    #[name = "Brilliant"]
    Brilliant,
    #[name = "Linked"]
    Linked,
}

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum CabSel {
    #[name = "Built-in (miked)"]
    BuiltInMiked,
    #[name = "IR"]
    Ir,
    #[name = "DI (no cab)"]
    DiNoCab,
    #[name = "Speaker (no mic)"]
    SpeakerNoMic,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[persist = "ir-path"]
    ir_path: Arc<RwLock<String>>,
    #[id = "input"]
    pub input: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
    #[id = "volume2"]
    pub volume2: FloatParam,
    #[id = "bass"]
    pub bass: FloatParam,
    #[id = "treble"]
    pub treble: FloatParam,
    #[id = "middle"]
    pub middle: FloatParam,
    #[id = "presence"]
    pub presence: FloatParam,
    #[id = "master"]
    pub master: FloatParam,
    #[id = "channel"]
    pub channel: EnumParam<Channel>,
    #[id = "cab"]
    pub cab: EnumParam<CabSel>,
    #[id = "mic"]
    pub mic: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            ir_path: Arc::new(RwLock::new(String::new())),
            input: db("Guitar Level", 0.0, -12.0, 12.0),
            volume: FloatParam::new("Normal Volume", 4.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            volume2: FloatParam::new("Brilliant Volume", 0.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            bass: FloatParam::new("Bass", 6.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            treble: FloatParam::new("Treble", 4.5, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            middle: FloatParam::new("Middle", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            presence: FloatParam::new("Presence", 3.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            master: FloatParam::new("Master", 8.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            channel: EnumParam::new("Input", Channel::Normal),
            cab: EnumParam::new("Cabinet", CabSel::BuiltInMiked),
            mic: pct("Mic Position", 0.3),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const INPUT: usize = 0;
pub const VOLUME: usize = 1;
pub const VOLUME2: usize = 2;
pub const BASS: usize = 3;
pub const TREBLE: usize = 4;
pub const MIDDLE: usize = 5;
pub const PRESENCE: usize = 6;
pub const MASTER: usize = 7;
pub const CHANNEL: usize = 8;
pub const CAB: usize = 9;
pub const MIC: usize = 10;
pub const OUTPUT: usize = 11;
pub const NPARAMS: usize = 12;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn ir_path(&self) -> Option<Arc<RwLock<String>>> {
        Some(self.ir_path.clone())
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.input.as_ptr(),
            self.volume.as_ptr(),
            self.volume2.as_ptr(),
            self.bass.as_ptr(),
            self.treble.as_ptr(),
            self.middle.as_ptr(),
            self.presence.as_ptr(),
            self.master.as_ptr(),
            self.channel.as_ptr(),
            self.cab.as_ptr(),
            self.mic.as_ptr(),
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
    (v as f64 / 10.0).clamp(0.0, 1.0)
}

impl AmpModel for Model {
    fn spec() -> AmpSpec {
        // the circuit that measured closest to the released plugin
        specs::ac50()
    }
    fn cab() -> cab::CabSpec {
        cab::FENDER_4X10_P10R
    }
    fn settings(v: &[f32]) -> Settings {
        // the two channels' volumes: one, the other, or (jumpered) both together
        let (n, b) = (rot(v[VOLUME]), rot(v[VOLUME2]));
        let (vol, bright) = match v[CHANNEL].round() as i32 {
            0 => (n, false),
            1 => (b, true),
            _ => ((n * n + b * b).sqrt().min(1.0), b > n * 0.7),
        };
        // the master after the phase inverter: with it low the preamp sets the drive, high it opens up the power stage
        let m = rot(v[MASTER]);
        Settings {
            controls: Controls { volume: (vol * (0.55 + 0.45 * m)).min(1.0), treble: rot(v[TREBLE]), bass: rot(v[BASS]), middle: rot(v[MIDDLE]), cut: (1.0 - rot(v[PRESENCE])) * 0.6, bright, alt: false },
            input_db: v[INPUT],
            output_db: v[OUTPUT] + 20.0 * (0.3 + 0.7 * m).log10() as f32,
            cab: (v[CAB].round() as usize).min(3),
            mic: v[MIC],
        }
    }
}

pub type Dsp = AmpDsp<Model>;

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Money Bass (pick)", values: &[(TREBLE, 6.0), (MIDDLE, 6.0), (PRESENCE, 4.0)] },
        Preset { name: "One of These Days", values: &[(VOLUME, 5.0), (TREBLE, 6.0), (PRESENCE, 5.0), (MASTER, 9.0)] },
        Preset { name: "Dogs / Animals Bass", values: &[(VOLUME, 6.0), (VOLUME2, 2.0), (TREBLE, 5.0), (MIDDLE, 6.0), (PRESENCE, 4.0), (MASTER, 9.0), (CHANNEL, 2.0)] },
        Preset { name: "Round and Deep", values: &[(BASS, 7.0), (TREBLE, 4.0), (MIDDLE, 4.0), (PRESENCE, 2.0), (MASTER, 7.0), (MIC, 0.6)] },
        Preset { name: "Driven Bass", values: &[(VOLUME, 8.0), (TREBLE, 5.0), (MIDDLE, 6.0), (PRESENCE, 4.0), (MASTER, 10.0)] },
    ]
}

/*
('knob', 51.5, 145.5, 23.2)
('knob', 121.5, 145.5, 23.2)
('knob', 191.5, 145.5, 23.2)
('knob', 261.5, 145.5, 23.2)
('knob', 331.5, 145.5, 23.2)
('knob', 401.5, 145.5, 23.2)
('knob', 471.5, 145.5, 23.2)
('selector?', 565.5, 145.5, 36, 36)
('divider', 206)
('meter', 424, 214, 186, 100)
('knob', 69.5, 255.5, 19.5)
('selector?', 169.5, 255.5, 36, 36)
('knob', 269.5, 255.5, 19.5)
('knob', 365.5, 255.5, 19.5)

crate: ['library/panic_abort', 'sequencefloyd/pf_dr103_bass']
/root/.cargo/git/checkouts/nih-plug-d4210f82e920295f/de42101/nih_plug_egui/src/editor.rs
AFTER THE HIWATT DR103, 1970s4 x EL34  
  HIWATT 4 x 12 FOR BASSNORMALBRILL.LINKED
INPUTWhich input the guitar is in: NORMAL (fuller), BRILLIANT (brighter, thinner coupling cap) or LINKED (jumpered, both channels at once; balance them with the two volumes).Normal channel volume (10 nF coupling: full, round). With INPUT on Linked, both volumes add together.BRILLIANTBrilliant channel volume (1 nF coupling: brighter, less bass). Used alone on INPUT Brilliant, or blended on Linked.BASSBass (470k). A Hiwatt goes deep: turn down to keep big chords tight.TREBLETreble (220k, 100 pF): acts mostly on the top octave.MIDDLEMiddle (22k): the Hiwatt mid control has real range. Up for a vocal lead, down for a scooped clean.PRESENCEPresence: less negative feedback in the treble, for more bite from the power stage.MASTERMaster volume (470k). Gilmour ran the masters high and set the gain with the channel volumes: the power valves add the compression.GUITAR LEVELHow hot the guitar hits the input. 0 dB treats a full-scale signal as 1 V from the pickups; turn up for a quiet DI.CABINETMIC'DIRDISPEAKER
MIC'D: the amp's own cabinet, close-miked. IR: your loaded impulse response. DI: the amp output with no cabinet (for your own cab plugin). SPEAKER: the cabinet with no microphone, to mic it up with the Mic plugin and/or Studio Spaces.MIC POSITIONBuilt-in cabinet: microphone from the cone centre (bright) out toward the edge (darker, rounder).OUTPUTPlugin output level. Doesn't change the amp's tone.POWERSAGCUSTOM 100 BASS
Money Bass (pick)
One of These Days
Dogs / Animals Bass
Round and Deep
Driven Bass
bg-worker
Could not spawn background worker thread/root/.cargo/git/checkouts/nih-plug-d4210f82e920295f/de42101/src/event_loop/background_thread.rs

*/
fn panel() -> Panel {
    Panel {
        title: "CUSTOM 100 BASS",
        subtitle: ("AFTER THE HIWATT DR103, 1970s", "4 x EL34  ·  HIWATT 4 x 12 FOR BASS"),
        accent: ui::OXBLOOD,
        size: (W, H),
        items: vec![
            Item::Knob { p: VOLUME, x: 52.0, y: 146.0, r: 24.0, label: "NORMAL", tip: "Normal channel volume (10 nF coupling: full, round). With INPUT on Linked, both volumes add together.", active: None },
            Item::Knob { p: VOLUME2, x: 122.0, y: 146.0, r: 24.0, label: "BRILLIANT", tip: "Brilliant channel volume (1 nF coupling: brighter, less bass). Used alone on INPUT Brilliant, or blended on Linked.", active: None },
            Item::Knob { p: BASS, x: 192.0, y: 146.0, r: 24.0, label: "BASS", tip: "Bass (470k). A Hiwatt goes deep: turn down to keep big chords tight.", active: None },
            Item::Knob { p: TREBLE, x: 262.0, y: 146.0, r: 24.0, label: "TREBLE", tip: "Treble (220k, 100 pF): acts mostly on the top octave.", active: None },
            Item::Knob { p: MIDDLE, x: 332.0, y: 146.0, r: 24.0, label: "MIDDLE", tip: "Middle (22k): the Hiwatt mid control has real range. Up for a vocal lead, down for a scooped clean.", active: None },
            Item::Knob { p: PRESENCE, x: 402.0, y: 146.0, r: 24.0, label: "PRESENCE", tip: "Presence: less negative feedback in the treble, for more bite from the power stage.", active: None },
            Item::Knob { p: MASTER, x: 472.0, y: 146.0, r: 24.0, label: "MASTER", tip: "Master volume (470k). Gilmour ran the masters high and set the gain with the channel volumes: the power valves add the compression.", active: None },
            Item::Selector { p: CHANNEL, x: 566.0, y: 146.0, label: "INPUT", positions: &["NORMAL", "BRILL.", "LINKED"], tip: "Which input the guitar is in: NORMAL (fuller), BRILLIANT (brighter, thinner coupling cap) or LINKED (jumpered, both channels at once; balance them with the two volumes)." },
            Item::Divider { y: 206.0 },
            Item::Knob { p: INPUT, x: 70.0, y: 256.0, r: 20.0, label: "GUITAR LEVEL", tip: "How hot the guitar hits the input. 0 dB treats a full-scale signal as 1 V from the pickups; turn up for a quiet DI.", active: None },
            Item::Selector { p: CAB, x: 170.0, y: 256.0, label: "CABINET", positions: CAB_POSITIONS, tip: "MIC'D: the amp's own cabinet, close-miked. IR: your loaded impulse response. DI: the amp output with no cabinet (for your own cab plugin). SPEAKER: the cabinet with no microphone, to mic it up with the Mic plugin and/or Studio Spaces." },
            Item::Knob { p: MIC, x: 270.0, y: 256.0, r: 20.0, label: "MIC POSITION", tip: "Built-in cabinet: microphone from the cone centre (bright) out toward the edge (darker, rounder).", active: Some(ui::ActiveWhen { p: CAB, min: 0.0, max: 0.0 }) },
            Item::Knob { p: OUTPUT, x: 366.0, y: 256.0, r: 20.0, label: "OUTPUT", tip: "Plugin output level. Doesn't change the amp's tone.", active: None },
            Item::Meter { m: 0, x: 424.0, y: 214.0, w: 186.0, h: 110.0, kind: MeterKind::Vu, caption: "POWER" },
            Item::IrLoader { x: 46.0, y: 320.0, w: 360.0, cab: CAB, ir_pos: 1 },
            Item::Lamp { m: 1, x: 496.0, y: 344.0, label: "SAG", lo: 2.0, hi: 25.0 },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "DR103 Bass", class_id: *b"PFAmpDR103BsPQD1", vendor: "PEQUOD Pink Floyd",
    categories: [Fx, Distortion]
);
