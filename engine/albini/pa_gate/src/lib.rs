//! Smart Gate: the MXR Smart Gate as Steve Albini used it on guitar, set so the slightest touch opens it: a noise gate
//! that listens to the treble (Hiss), the midrange (Mid) or everything (Full).

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
    #[name = "Hiss"]
    Hiss,
    #[name = "Mid"]
    Mid,
    #[name = "Full"]
    Full,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "threshold"]
    pub threshold: FloatParam,
    #[id = "mode"]
    pub mode: EnumParam<ModeSel>,
    #[id = "release"]
    pub release: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            threshold: db("Threshold", -48.0, -80.0, -10.0),
            mode: EnumParam::new("Mode", ModeSel::Mid),
            release: ms("Release", 60.0, 5.0, 500.0),
        }
    }
}

pub const THRESHOLD: usize = 0;
pub const MODE: usize = 1;
pub const RELEASE: usize = 2;
pub const NPARAMS: usize = 3;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.threshold.as_ptr(),
            self.mode.as_ptr(),
            self.release.as_ptr(),
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
    det: Biquad,
    det2: Biquad,
    env: f64,
    gain: f64,
    mode: i32,
}

impl Dsp {
    fn design(&mut self) {
        match self.mode {
            0 => {
                self.det.set_highpass(self.fs, 3000.0, 0.7);
                self.det2 = Biquad::identity();
            }
            1 => {
                self.det.set_highpass(self.fs, 250.0, 0.7);
                self.det2.set_lowpass(self.fs, 3000.0, 0.7);
            }
            _ => {
                self.det = Biquad::identity();
                self.det2 = Biquad::identity();
            }
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, det: Biquad::identity(), det2: Biquad::identity(), env: 0.0, gain: 0.0, mode: v[MODE].round() as i32 };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.det.reset();
        self.det2.reset();
        self.env = 0.0;
        self.gain = 0.0;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let m = v[MODE].round() as i32;
        if m != self.mode {
            self.mode = m;
            self.design();
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(2);
        let thr = db_to_gain(self.v[THRESHOLD] as f64);
        // HISS mode only ever turns the gate down by so much (it's a hiss reducer)
        let floor = if self.mode == 0 { db_to_gain(-36.0) } else { 0.0 };
        let ce = coeff(self.fs, 2.0);
        let ca = coeff(self.fs, 0.5);
        let cr = coeff(self.fs, self.v[RELEASE] as f64);
        for k in 0..n {
            let x = chans.iter().take(nch).map(|c| c[k] as f64).sum::<f64>() / nch.max(1) as f64;
            let d = self.det2.process(self.det.process(x)).abs();
            self.env = d.max(self.env * ce);
            let open = if self.env > thr { 1.0 } else if self.env > thr * 0.7 && self.gain > 0.5 { 1.0 } else { floor };
            self.gain = open + if open > self.gain { ca } else { cr } * (self.gain - open);
            for c in chans.iter_mut().take(nch) {
                c[k] = (c[k] as f64 * self.gain) as f32;
            }
        }
        meters[0] = self.gain as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Albini: Open on Touch", values: &[(THRESHOLD, -52.0), (RELEASE, 40.0)] },
        Preset { name: "Tight Stops", values: &[(THRESHOLD, -45.0), (MODE, 2.0), (RELEASE, 15.0)] },
        Preset { name: "Hiss Only", values: &[(THRESHOLD, -60.0), (MODE, 0.0), (RELEASE, 120.0)] },
        Preset { name: "Gentle", values: &[(THRESHOLD, -65.0), (MODE, 2.0), (RELEASE, 250.0)] },
    ]
}

/*
('knob', 199.5, 159.5, 43.9)
('selector?', 439.5, 160.0, 36, 35)
('divider', 270)
('knob', 319.5, 333.5, 22.0)

crate: ['albini/pa_smartgate', 'library/panic_abort']
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
/root/.cargo/git/checkouts/nih-plug-d4210f82e920295f/de42101/nih_plug_egui/src/editor.rs
AFTER THE MXR SMART GATENOISE GATE  
  HISS / MID / FULLTHRESHOLDThe level that opens the gate. Albini set it so the slightest touch opens it.HISSMIDFULL
MODEWhat the gate listens to: HISS (treble only), MID (the guitar's midrange) or FULL (everything).OPENRELEASEHow quickly it closes after you stop (the pedal's own is fast).SMART GATE
Albini: Open on Touch
Tight Stops
Hiss Only
Gentle
j?$Dsp
)"8	
!(El

*/
fn panel() -> Panel {
    Panel {
        title: "SMART GATE",
        subtitle: ("AFTER THE MXR SMART GATE", "NOISE GATE  ·  HISS / MID / FULL"),
        accent: ui::OLIVE,
        size: (W, H),
        items: vec![
            Item::Knob { p: THRESHOLD, x: 200.0, y: 160.0, r: 44.0, label: "THRESHOLD", tip: "The level that opens the gate. Albini set it so the slightest touch opens it.", active: None },
            Item::Lamp { m: 0, x: 320.0, y: 160.0, label: "OPEN", lo: 0.1, hi: 0.9 },
            Item::Selector { p: MODE, x: 440.0, y: 160.0, label: "MODE", positions: &["HISS", "MID", "FULL"], tip: "What the gate listens to: HISS (treble only), MID (the guitar's midrange) or FULL (everything)." },
            Item::Divider { y: 270.0 },
            Item::Knob { p: RELEASE, x: 320.0, y: 334.0, r: 22.0, label: "RELEASE", tip: "How quickly it closes after you stop (the pedal's own is fast).", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Smart Gate", class_id: *b"PASmartGatePQD01", vendor: "PEQUOD Albini",
    categories: [Fx, Dynamics]
);
