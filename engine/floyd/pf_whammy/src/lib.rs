//! Whammy: the DigiTech Whammy (1990s) as David Gilmour used it on Marooned: a pitch shifter whose interval is swept by
//! the treadle (heel = the guitar's own pitch, toe = the full interval), or harmony and detune modes mixed with the dry signal.

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
    #[name = "+1 octave"]
    Up1,
    #[name = "+2 octaves"]
    Up2,
    #[name = "-1 octave"]
    Down1,
    #[name = "-2 octaves"]
    Down2,
    #[name = "Harmony: 5th up"]
    Harmony5ThUp,
    #[name = "Harmony: octave up"]
    HarmonyOctaveUp,
    #[name = "Detune"]
    Detune,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "treadle"]
    pub treadle: FloatParam,
    #[id = "mode"]
    pub mode: EnumParam<ModeSel>,
    #[id = "glide"]
    pub glide: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            treadle: pct("Treadle", 1.0),
            mode: EnumParam::new("Mode", ModeSel::Up1),
            glide: ms("Treadle Glide", 40.0, 1.0, 500.0),
            output: db("Output", 0.0, -12.0, 6.0),
        }
    }
}

pub const TREADLE: usize = 0;
pub const MODE: usize = 1;
pub const GLIDE: usize = 2;
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
            self.treadle.as_ptr(),
            self.mode.as_ptr(),
            self.glide.as_ptr(),
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

use std::f64::consts::PI;
use vintage::fx::PitchShift;
use vintage::pitch::Pitch;

/// Two-head delay-line shifter whose grain length follows the detected period (a whole number of cycles),
/// so the crossfaded heads stay in phase on a held note instead of combing.
struct SyncShift {
    line: DelayLine,
    phase: f64,
    win: f64,
    want: f64,
}

impl SyncShift {
    fn new(fs: f64) -> Self {
        Self { line: DelayLine::new((fs * 0.2) as usize), phase: 0.0, win: 0.04 * fs, want: 0.04 * fs }
    }
    fn reset(&mut self) {
        self.line.reset();
        self.phase = 0.0;
    }
    #[inline]
    fn process(&mut self, x: f64, ratio: f64) -> f64 {
        self.line.write(x);
        if (ratio - 1.0).abs() < 1e-6 {
            return self.line.read(2.0);
        }
        let prev = self.phase;
        self.phase = (self.phase + (1.0 - ratio) / self.win).rem_euclid(1.0);
        // take a new grain length only where head 1 jumps back
        if (ratio > 1.0 && self.phase > prev) || (ratio < 1.0 && self.phase < prev) {
            self.win = self.want;
        }
        let p2 = (self.phase + 0.5).fract();
        let d1 = 2.0 + self.phase * (self.win - 4.0);
        let d2 = 2.0 + p2 * (self.win - 4.0);
        let g1 = (PI * self.phase).sin();
        let g2 = (PI * p2).sin();
        (self.line.read(d1) * g1 + self.line.read(d2) * g2) * 0.85
    }
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ps: [PitchShift; 2],
    ps2: [PitchShift; 2],
    sync: [SyncShift; 2],
    det: Pitch,
    lp: [Biquad; 2],
    pos: f64,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut lp = Biquad::identity();
        lp.set_lowpass(fs, 9000.0, 0.7);
        Self {
            fs,
            v,
            ps: [PitchShift::new(fs, 40.0), PitchShift::new(fs, 40.0)],
            ps2: [PitchShift::new(fs, 40.0), PitchShift::new(fs, 40.0)],
            sync: [SyncShift::new(fs), SyncShift::new(fs)],
            det: Pitch::new(fs),
            lp: [lp; 2],
            pos: v[TREADLE] as f64,
            out: Smooth::new(fs, 20.0, 1.0),
        }
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.ps[c].reset();
            self.ps2[c].reset();
            self.sync[c].reset();
            self.lp[c].reset();
        }
        self.pos = self.v[TREADLE] as f64;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let mode = self.v[MODE].round() as i32;
        let target = self.v[TREADLE] as f64;
        let cg = coeff(self.fs, self.v[GLIDE] as f64);
        // (semitones at the toe, dry mixed in)
        let (semis, dry) = match mode {
            0 => (12.0, false),
            1 => (24.0, false),
            2 => (-12.0, false),
            3 => (-24.0, false),
            4 => (7.0, true),
            5 => (12.0, true),
            _ => (0.1, true),
        };
        let nch = chans.len().min(2);
        for k in 0..n {
            self.pos = target + cg * (self.pos - target);
            let o = self.out.next();
            let r = 2f64.powf(semis * self.pos / 12.0);
            self.det.push(chans.iter().take(nch).map(|c| c[k] as f64).sum::<f64>());
            if k == 0 {
                // grain = a whole number of periods, about 40 ms
                let want = match self.det.freq {
                    Some(f) if f > 40.0 => {
                        let per = self.fs / f;
                        (0.04 * self.fs / per).round().max(1.0) * per
                    }
                    _ => 0.04 * self.fs,
                };
                for s in self.sync.iter_mut() {
                    s.want = want.max(64.0);
                }
            }
            for c in 0..nch {
                let x = chans[c][k] as f64;
                let y = if mode == 6 {
                    // detune: a few cents either side of the dry note
                    let a = self.ps[c].process(x, 2f64.powf(0.1 * self.pos / 12.0));
                    let b = self.ps2[c].process(x, 2f64.powf(-0.1 * self.pos / 12.0));
                    x + 0.5 * (a + b)
                } else {
                    let w = self.lp[c].process(self.sync[c].process(x, r));
                    if dry { 0.7 * (x + w) } else { w }
                };
                chans[c][k] = (y * o) as f32;
            }
        }
        meters[0] = self.pos as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Marooned (+1 octave)", values: &[(GLIDE, 60.0)] },
        Preset { name: "Treadle Heel (dry pitch)", values: &[(TREADLE, 0.0)] },
        Preset { name: "Two Octaves Up", values: &[(MODE, 1.0)] },
        Preset { name: "Octave Down", values: &[(MODE, 2.0)] },
        Preset { name: "Octave Harmony", values: &[(MODE, 5.0)] },
        Preset { name: "Detune Thickener", values: &[(MODE, 6.0)] },
    ]
}

/*
('knob', 159.5, 159.5, 47.6)
('selector?', 359.5, 159.5, 36, 36)
('divider', 284)
('knob', 219.5, 335.5, 22.0)
('knob', 419.5, 335.5, 22.0)

crate: ['floyd/pf_whammy', 'library/panic_abort', 'sourcefloyd/pf_whammy']
/root/.cargo/git/checkouts/nih-plug-d4210f82e920295f/de42101/nih_plug_egui/src/editor.rs
AFTER THE DIGITECH WHAMMY, 1990sPITCH PEDAL  
  TREADLETREADLEThe treadle: heel (0%) = the guitar's own pitch, toe (100%) = the full interval. Automate it (or map it to an expression pedal) for Marooned-style sweeps.+1 OCT+2 OCT-1 OCT-2 OCT5THOCT HARMDETUNE
MODEWhammy intervals (shifted signal only) or harmony / detune modes (shifted + dry).TOEGLIDEHow quickly the pitch follows the treadle (smooths automated sweeps).OUTPUTOutput level.WHAMMY
Marooned (+1 octave)
Treadle Heel (dry pitch)
Two Octaves Up
Octave Down
Octave Harmony
Detune Thickener$min > max, or either was NaN. min = 
, max = 
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/core/src/num/f64.rs

*/
fn panel() -> Panel {
    Panel {
        title: "WHAMMY",
        subtitle: ("AFTER THE DIGITECH WHAMMY, 1990s", "PITCH PEDAL  ·  TREADLE"),
        accent: ui::INDIGO,
        size: (W, H),
        items: vec![
            Item::Knob { p: TREADLE, x: 160.0, y: 160.0, r: 48.0, label: "TREADLE", tip: "The treadle: heel (0%) = the guitar's own pitch, toe (100%) = the full interval. Automate it (or map it to an expression pedal) for Marooned-style sweeps.", active: None },
            Item::Selector { p: MODE, x: 360.0, y: 160.0, label: "MODE", positions: &["+1 OCT", "+2 OCT", "-1 OCT", "-2 OCT", "5TH", "OCT HARM", "DETUNE"], tip: "Whammy intervals (shifted signal only) or harmony / detune modes (shifted + dry)." },
            Item::Lamp { m: 0, x: 520.0, y: 120.0, label: "TOE", lo: 0.0, hi: 1.0 },
            Item::Divider { y: 284.0 },
            Item::Knob { p: GLIDE, x: 220.0, y: 336.0, r: 22.0, label: "GLIDE", tip: "How quickly the pitch follows the treadle (smooths automated sweeps).", active: None },
            Item::Knob { p: OUTPUT, x: 420.0, y: 336.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Whammy", class_id: *b"PFWhammyPQD00001", vendor: "PEQUOD Pink Floyd",
    categories: [Fx, PitchShift]
);
