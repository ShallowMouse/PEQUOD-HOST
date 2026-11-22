//! Black Box Limiter: after Joe Meek's homemade valve limiter, driven to the maximum for the hottest level on tape: a
//! feedback gain element with a fast or slower attack, a recovery that breathes, and valve drive after it.

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
pub const H: u32 = 432;

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum ModeSel {
    #[name = "Compress"]
    Compress,
    #[name = "Limit"]
    Limit,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "input"]
    pub input: FloatParam,
    #[id = "recovery"]
    pub recovery: FloatParam,
    #[id = "mode"]
    pub mode: EnumParam<ModeSel>,
    #[id = "valve"]
    pub valve: FloatParam,
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
            input: db("Input", 12.0, 0.0, 36.0),
            recovery: ms("Recovery", 250.0, 40.0, 3000.0),
            mode: EnumParam::new("Mode", ModeSel::Limit),
            valve: pct("Valve Drive", 0.4),
            mix: pct("Mix", 1.0),
            output: db("Output", -6.0, -30.0, 12.0),
        }
    }
}

pub const INPUT: usize = 0;
pub const RECOVERY: usize = 1;
pub const MODE: usize = 2;
pub const VALVE: usize = 3;
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
            self.input.as_ptr(),
            self.recovery.as_ptr(),
            self.mode.as_ptr(),
            self.valve.as_ptr(),
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
    env: f64,
    gr: f64,
    last: f64,
    hp: [Biquad; 2],
    inp: Smooth,
    mix: Smooth,
    out: Smooth,
}

/// Where the gain element starts to work (full scale ~ 0 dBFS).
const THRESH: f64 = 0.06;

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut hp = Biquad::identity();
        hp.set_highpass(fs, 20.0, 0.7);
        Self { fs, v, env: 0.0, gr: 1.0, last: 0.0, hp: [hp; 2], inp: Smooth::new(fs, 20.0, 1.0), mix: Smooth::new(fs, 20.0, 1.0), out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.env = 0.0;
        self.gr = 1.0;
        self.last = 0.0;
        self.hp.iter_mut().for_each(|b| b.reset());
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.inp.set(db_to_gain(v[INPUT] as f64));
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let limit = self.v[MODE] > 0.5;
        let ratio = if limit { 5.0 } else { 3.5 };
        let ca = coeff(self.fs, if limit { 0.5 } else { 8.0 });
        let cr = coeff(self.fs, self.v[RECOVERY] as f64);
        let valve = self.v[VALVE] as f64;
        let nch = chans.len().min(2);
        let mut most = 0.0f64;
        for k in 0..n {
            let gi = self.inp.next();
            let m = self.mix.next();
            let o = self.out.next();
            // feedback: the detector listens to the output of the gain element
            let a = self.last;
            self.env = a + if a > self.env { ca } else { cr } * (self.env - a);
            let over = (self.env / if limit { THRESH } else { 2.5 * THRESH }).max(1.0);
            let target = over.powf(1.0 / ratio - 1.0);
            self.gr = target;
            most = most.max(-gain_to_db(self.gr));
            let mut peak = 0.0f64;
            for c in 0..nch {
                let x = chans[c][k] as f64;
                let y = x * gi * self.gr;
                peak = peak.max(y.abs());
                // the valve stage after it
                let d = 1.0 + 4.0 * valve;
                let w = self.hp[c].process(tube(y * d, 0.1 * valve) / d);
                chans[c][k] = ((x * (1.0 - m) + w * m * 0.5) * o) as f32;
            }
            self.last = peak;
        }
        meters[0] = most as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Hottest Level on Tape", values: &[(INPUT, 18.0), (RECOVERY, 300.0)] },
        Preset { name: "Pumping Band (Meek)", values: &[(INPUT, 24.0), (RECOVERY, 180.0), (VALVE, 0.5)] },
        Preset { name: "Bad Penny Whole-Band Squash", values: &[(INPUT, 20.0), (RECOVERY, 400.0), (MODE, 0.0)] },
        Preset { name: "Breathing Vocal", values: &[(INPUT, 16.0), (RECOVERY, 600.0), (MODE, 0.0), (VALVE, 0.3)] },
        Preset { name: "Smash (parallel)", values: &[(INPUT, 30.0), (RECOVERY, 120.0), (MIX, 0.5)] },
        Preset { name: "Gentle Compress", values: &[(INPUT, 8.0), (RECOVERY, 800.0), (MODE, 0.0), (OUTPUT, -3.0)] },
    ]
}

/*
('meter', 232, 100, 176, 114)
('knob', 119.5, 169.5, 43.9)
('knob', 519.5, 169.5, 43.9)
('divider', 270)
('knob', 239.5, 333.5, 22.0)
('knob', 369.5, 333.5, 22.0)
('knob', 499.5, 333.5, 22.0)
('selector?', 109.5, 339.5, 36, 36)

crate: ['library/panic_abort', 'meek/jm_limiter', 'utputmeek/jm_limiter']
/root/.cargo/git/checkouts/nih-plug-d4210f82e920295f/de42101/nih_plug_egui/src/editor.rs
AFTER JOE MEEK'S HOMEMADE LIMITERVALVE  
  FEEDBACK  
  PUMPINGINPUTHow hard it's hit. Meek drove limiters to the maximum.GAIN REDUCTIONRECOVERYHow fast it breathes back up. Short = pumping, long = steady squash.COMP.LIMIT
MODECOMPRESS (about 3.5:1, slower attack) or LIMIT (hard and fast).VALVEValve drive after the gain element.MIXBlend with the dry signal.OUTPUTOutput level.BLACK BOX LIMITER
Hottest Level on Tape
Pumping Band (Meek)
Bad Penny Whole-Band Squash
Breathing Vocal
Smash (parallel)
Gentle Compress
j?$Dsp

*/
fn panel() -> Panel {
    Panel {
        title: "BLACK BOX LIMITER",
        subtitle: ("AFTER JOE MEEK'S HOMEMADE LIMITER", "VALVE  ·  FEEDBACK  ·  PUMPING"),
        accent: ui::MUSTARD,
        size: (W, H),
        items: vec![
            Item::Knob { p: INPUT, x: 120.0, y: 170.0, r: 44.0, label: "INPUT", tip: "How hard it's hit. Meek drove limiters to the maximum.", active: None },
            Item::Meter { m: 0, x: 232.0, y: 100.0, w: 176.0, h: 124.0, kind: MeterKind::GainReduction, caption: "GAIN REDUCTION" },
            Item::Knob { p: RECOVERY, x: 520.0, y: 170.0, r: 44.0, label: "RECOVERY", tip: "How fast it breathes back up. Short = pumping, long = steady squash.", active: None },
            Item::Divider { y: 270.0 },
            Item::Selector { p: MODE, x: 110.0, y: 340.0, label: "MODE", positions: &["COMP.", "LIMIT"], tip: "COMPRESS (about 3.5:1, slower attack) or LIMIT (hard and fast)." },
            Item::Knob { p: VALVE, x: 240.0, y: 334.0, r: 22.0, label: "VALVE", tip: "Valve drive after the gain element.", active: None },
            Item::Knob { p: MIX, x: 370.0, y: 334.0, r: 22.0, label: "MIX", tip: "Blend with the dry signal.", active: None },
            Item::Knob { p: OUTPUT, x: 500.0, y: 334.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Black Box Limiter", class_id: *b"JMBlackBoxLimPQ1", vendor: "PEQUOD Joe Meek",
    categories: [Fx, Dynamics]
);
