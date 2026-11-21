//! Dyna Comp: the MXR Dyna Comp (script logo) as David Gilmour used it: a CA3080 OTA gain cell in a feedback loop,
//! Sensitivity setting how hard it squashes, with the slow release that gives the long sustain.

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


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "output"]
    pub output: FloatParam,
    #[id = "sensitivity"]
    pub sensitivity: FloatParam,
    #[id = "level"]
    pub level: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            output: FloatParam::new("Output", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            sensitivity: FloatParam::new("Sensitivity", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            level: db("Guitar Level", 0.0, -12.0, 12.0),
        }
    }
}

pub const OUTPUT: usize = 0;
pub const SENSITIVITY: usize = 1;
pub const LEVEL: usize = 2;
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
            self.output.as_ptr(),
            self.sensitivity.as_ptr(),
            self.level.as_ptr(),
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
    top: Vec<Biquad>,
    hp: Vec<Biquad>,
    out: Smooth,
    inp: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let n = ch.max(2);
        let mut t = Biquad::identity();
        t.set_lowpass(fs, 10000.0, 0.6);
        let mut h = Biquad::identity();
        h.set_highpass(fs, 40.0, 0.7);
        Self { fs, v, env: 0.0, top: vec![t; n], hp: vec![h; n], out: Smooth::new(fs, 20.0, 0.25), inp: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.env = 0.0;
        self.top.iter_mut().chain(self.hp.iter_mut()).for_each(|b| b.reset());
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let o = v[OUTPUT] as f64 / 10.0;
        self.out.set(o * o * 9.5);
        self.inp.set(db_to_gain(v[LEVEL] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let sens = self.v[SENSITIVITY] as f64 / 10.0;
        // sensitivity: the gain in front of the gain cell, so more squash and more noise
        let pre = db_to_gain(36.0 * sens);
        let ca = coeff(self.fs, 4.0);
        let cr = coeff(self.fs, 150.0);
        let last = self.top.len() - 1;
        let mut most = 0.0f64;
        for k in 0..n {
            let o = self.out.next();
            let gi = self.inp.next();
            let peak = chans.iter().map(|c| (c[k] as f64).abs()).fold(0.0, f64::max) * pre * gi;
            let c = if peak > self.env { ca } else { cr };
            self.env = peak + c * (self.env - peak);
            // the OTA's gain falls roughly as 1 / level above a small threshold
            let g = (1.0 + 6.0 * self.env).powf(-0.75);
            most = most.max(-gain_to_db(g));
            for (ci, ch) in chans.iter_mut().enumerate() {
                let j = ci.min(last);
                let x = ch[k] as f64 * gi * pre * g;
                let y = (x * 2.5).tanh() / 2.5 + 0.04 * x * x;
                let y = self.top[j].process(self.hp[j].process(y)) * 0.6;
                ch[k] = (y * o) as f32;
            }
        }
        meters[0] = most as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Shine On Riff (with Phase 90)", values: &[(SENSITIVITY, 6.0)] },
        Preset { name: "Sustaining Clean", values: &[(OUTPUT, 4.5), (SENSITIVITY, 7.5)] },
        Preset { name: "Light Levelling", values: &[(OUTPUT, 6.0), (SENSITIVITY, 3.0)] },
        Preset { name: "Full Squash", values: &[(OUTPUT, 4.0), (SENSITIVITY, 10.0)] },
    ]
}

/*
('meter', 232, 100, 176, 114)
('knob', 129.5, 169.5, 43.9)
('knob', 509.5, 169.5, 43.9)
('divider', 284)
('knob', 319.5, 335.5, 22.0)

crate: ['floyd/pf_dynacomp', 'library/panic_abort']
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
/root/.cargo/git/checkouts/nih-plug-d4210f82e920295f/de42101/nih_plug_egui/src/editor.rs
AFTER THE MXR DYNA COMP, SCRIPT LOGOCA3080 OTA  
  FEEDBACK COMPRESSIONOUTPUTOutput level.SENSITIVITYHow hard the input drives the compressor: more squash and sustain.GAIN REDUCTIONGUITAR LEVELHow hot your DI is (0 dB treats full scale as 1 V from the pickups).DYNA COMP
Shine On Riff (with Phase 90)
Sustaining Clean
Light Levelling
Full Squash
OutputSensitivityGuitar Levelassertion failed: unsafe { QueryPerformanceCounter(&mut ticks).as_bool() }/root/.cargo/git/checkouts/nih-plug-d4210f82e920295f/de42101/src/event_loop/windows.rs
nih-event-loop-
NIH-plug event loop
Could not get the current module's handle

*/
fn panel() -> Panel {
    Panel {
        title: "DYNA COMP",
        subtitle: ("AFTER THE MXR DYNA COMP, SCRIPT LOGO", "CA3080 OTA  ·  FEEDBACK COMPRESSION"),
        accent: ui::MUSTARD,
        size: (W, H),
        items: vec![
            Item::Knob { p: OUTPUT, x: 130.0, y: 170.0, r: 44.0, label: "OUTPUT", tip: "Output level.", active: None },
            Item::Meter { m: 0, x: 232.0, y: 100.0, w: 176.0, h: 124.0, kind: MeterKind::GainReduction, caption: "GAIN REDUCTION" },
            Item::Knob { p: SENSITIVITY, x: 510.0, y: 170.0, r: 44.0, label: "SENSITIVITY", tip: "How hard the input drives the compressor: more squash and sustain.", active: None },
            Item::Divider { y: 284.0 },
            Item::Knob { p: LEVEL, x: 320.0, y: 336.0, r: 22.0, label: "GUITAR LEVEL", tip: "How hot your DI is (0 dB treats full scale as 1 V from the pickups).", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Dyna Comp", class_id: *b"PFDynaCompPQD001", vendor: "PEQUOD Pink Floyd",
    categories: [Fx, Dynamics]
);
