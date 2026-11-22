//! Tape Flanger: Joe Meek's flanging (1957) with two tape machines playing the same thing, one slowed by hand on the reel:
//! through-zero flanging, set by hand or swept slowly, with tape saturation on both copies.

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
    #[name = "Hand"]
    Hand,
    #[name = "Sweep"]
    Sweep,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "mode"]
    pub mode: EnumParam<ModeSel>,
    #[id = "hand"]
    pub hand: FloatParam,
    #[id = "rate"]
    pub rate: FloatParam,
    #[id = "depth"]
    pub depth: FloatParam,
    #[id = "blend"]
    pub blend: FloatParam,
    #[id = "tape"]
    pub tape: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            mode: EnumParam::new("Mode", ModeSel::Sweep),
            hand: FloatParam::new("Hand (machine B offset)", 1.0, FloatRange::Linear { min: -2.0, max: 8.0 }).with_unit(" ms").with_value_to_string(formatters::v2s_f32_rounded(2)),
            rate: rate("Sweep Rate", 0.12, 0.02, 2.0),
            depth: pct("Sweep Depth", 0.8),
            blend: pct("Machine Blend", 0.5),
            tape: pct("Tape Drive", 0.3),
            output: db("Output", 0.0, -18.0, 6.0),
        }
    }
}

pub const MODE: usize = 0;
pub const HAND: usize = 1;
pub const RATE: usize = 2;
pub const DEPTH: usize = 3;
pub const BLEND: usize = 4;
pub const TAPE: usize = 5;
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
            self.mode.as_ptr(),
            self.hand.as_ptr(),
            self.rate.as_ptr(),
            self.depth.as_ptr(),
            self.blend.as_ptr(),
            self.tape.as_ptr(),
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
    line: [DelayLine; 2],
    lp: [[Biquad; 2]; 2],
    ph: f64,
    off: Smooth,
    out: Smooth,
}

/// Machine A plays this far behind the input, so machine B can run up to 2 ms ahead of it.
const BASE_MS: f64 = 2.5;

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut lp = Biquad::identity();
        lp.set_lowpass(fs, 14000.0f64.min(fs * 0.45), 0.707);
        Self { fs, v, line: [DelayLine::new((fs * 0.02) as usize), DelayLine::new((fs * 0.02) as usize)], lp: [[lp; 2]; 2], ph: 0.0, off: Smooth::new(fs, 40.0, 1.0), out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn latency(&self) -> u32 {
        (BASE_MS * 0.001 * self.fs) as u32
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.line[c].reset();
            self.lp[c][0].reset();
            self.lp[c][1].reset();
        }
        self.ph = 0.0;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let sweep = self.v[MODE] > 0.5;
        let rate = self.v[RATE] as f64;
        let depth = self.v[DEPTH] as f64;
        let b = self.v[BLEND] as f64;
        let drive = self.v[TAPE] as f64;
        let pre = 1.0 + 3.0 * drive;
        let ms = 0.001 * self.fs;
        let nch = chans.len().min(2);
        let mut o_ms = 0.0;
        for k in 0..n {
            let target = if sweep {
                self.ph = (self.ph + rate / self.fs).fract();
                let tri = 1.0 - 2.0 * (self.ph - 0.5).abs();
                // through zero: from 2 ms ahead out to 8 ms behind, scaled by the depth around 1 ms
                1.0 + (tri * 10.0 - 3.0) * depth
            } else {
                self.v[HAND] as f64
            };
            let off = self.off.next_to(target.clamp(-2.0, 8.0));
            o_ms = off;
            let g = self.out.next();
            for c in 0..nch {
                let x = chans[c][k] as f64;
                self.line[c].write(x);
                let a = self.line[c].read(BASE_MS * ms);
                let bb = self.line[c].read(((BASE_MS + off) * ms).max(1.0));
                // each machine's tape: soft saturation and a little top loss
                let a = self.lp[c][0].process(tape_curve(a * pre) / pre);
                let bb = self.lp[c][1].process(tape_curve(bb * pre) / pre);
                chans[c][k] = (((1.0 - b) * a + b * bb) * 2.0 * g * 0.7) as f32;
            }
        }
        meters[0] = ((o_ms + 2.0) / 10.0) as f32;
    }
}

trait NextTo {
    fn next_to(&mut self, t: f64) -> f64;
}
impl NextTo for Smooth {
    fn next_to(&mut self, t: f64) -> f64 {
        self.set(t);
        self.next()
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Slow Jet (through zero)", values: &[] },
        Preset { name: "Hand on the Reel", values: &[(MODE, 0.0)] },
        Preset { name: "Fixed Comb (2 ms)", values: &[(MODE, 0.0), (HAND, 2.0)] },
        Preset { name: "Outer Space Sweep", values: &[(RATE, 0.3), (DEPTH, 1.0), (TAPE, 0.5)] },
        Preset { name: "Gentle Phasing", values: &[(RATE, 0.08), (DEPTH, 0.35), (BLEND, 0.4)] },
    ]
}

/*
('selector?', 79.5, 159.5, 36, 36)
('knob', 339.5, 159.5, 29.3)
('knob', 459.5, 159.5, 29.3)
('divider', 270)
('knob', 199.5, 333.5, 22.0)
('knob', 319.5, 333.5, 22.0)
('knob', 439.5, 333.5, 26.8)

crate: ['library/panic_abort', 'meek/jm_flanger']
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
TWO TAPE MACHINES, ONE SLOWED BY HANDTHROUGH-ZERO FLANGING  
  1957HANDSWEEP
MODEHAND: set machine B's offset yourself (automate it). SWEEP: a slow cycle.Machine B's offset from A (ms). Below zero it runs ahead.SWEEP RATEHow fast machine B drifts back and forth.SWEEP DEPTHHow far it drifts.OFFSETMACHINE BLENDBalance of the two machines (50% = deepest notches).TAPE DRIVETape saturation on both copies.OUTPUTOutput level.TAPE FLANGERSlow Jet (through zero)
Hand on the Reel
Fixed Comb (2 ms)
Outer Space Sweep
Gentle Phasing$min > max, or either was NaN. min = 
, max = 
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/core/src/num/f64.rs
j?$Dsp
)"8	

*/
fn panel() -> Panel {
    Panel {
        title: "TAPE FLANGER",
        subtitle: ("TWO TAPE MACHINES, ONE SLOWED BY HAND", "THROUGH-ZERO FLANGING  ·  1957"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![
            Item::Selector { p: MODE, x: 80.0, y: 160.0, label: "MODE", positions: &["HAND", "SWEEP"], tip: "HAND: set machine B's offset yourself (automate it). SWEEP: a slow cycle." },
            Item::Knob { p: HAND, x: 210.0, y: 160.0, r: 36.0, label: "HAND", tip: "Machine B's offset from A (ms). Below zero it runs ahead.", active: Some(ActiveWhen { p: MODE, min: 0.0, max: 0.4 }) },
            Item::Knob { p: RATE, x: 340.0, y: 160.0, r: 30.0, label: "SWEEP RATE", tip: "How fast machine B drifts back and forth.", active: Some(ActiveWhen { p: MODE, min: 0.6, max: 1.0 }) },
            Item::Knob { p: DEPTH, x: 460.0, y: 160.0, r: 30.0, label: "SWEEP DEPTH", tip: "How far it drifts.", active: Some(ActiveWhen { p: MODE, min: 0.6, max: 1.0 }) },
            Item::Lamp { m: 0, x: 570.0, y: 160.0, label: "OFFSET", lo: 0.0, hi: 1.0 },
            Item::Divider { y: 270.0 },
            Item::Knob { p: BLEND, x: 200.0, y: 334.0, r: 22.0, label: "MACHINE BLEND", tip: "Balance of the two machines (50% = deepest notches).", active: None },
            Item::Knob { p: TAPE, x: 320.0, y: 334.0, r: 22.0, label: "TAPE DRIVE", tip: "Tape saturation on both copies.", active: None },
            Item::Knob { p: OUTPUT, x: 440.0, y: 334.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Tape Flanger", class_id: *b"JMTapeFlangerPQ1", vendor: "PEQUOD Joe Meek",
    categories: [Fx, Modulation]
);
