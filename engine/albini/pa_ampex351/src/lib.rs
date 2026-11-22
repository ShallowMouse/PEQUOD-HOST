//! Ampex 351: the record electronics of an Ampex 351 used as a valve mic preamp (Albini's favourite for ribbons): input
//! transformer, two valve stages and the output transformer. Clean at normal levels, thickening when pushed.

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
    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "lowcut"]
    pub lowcut: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            gain: db("Gain", 0.0, -12.0, 24.0),
            lowcut: hz("Low Cut", 20.0, 20.0, 300.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const GAIN: usize = 0;
pub const LOWCUT: usize = 1;
pub const OUTPUT: usize = 2;
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
            self.gain.as_ptr(),
            self.lowcut.as_ptr(),
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

const OS: usize = 2;

#[derive(Clone, Default)]
struct St {
    xin: Biquad,
    lf: f64,
    hf: Biquad,
    cut: Biquad,
    dc: Biquad,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    st: [St; 2],
    os: [Oversampler; 2],
    lc: f32,
    gain: Smooth,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let ofs = self.fs * OS as f64;
        let lc = self.v[LOWCUT] as f64;
        for st in self.st.iter_mut() {
            st.xin.set_highpass(ofs, 12.0, 0.6);
            st.hf.set_lowpass(ofs, 22000.0f64.min(ofs * 0.45), 0.6);
            st.dc.set_highpass(ofs, 8.0, 0.7);
            if lc > 20.5 {
                st.cut.set_highpass(ofs, lc * 0.6, 0.5);
            } else {
                st.cut = Biquad::identity();
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
        let mut d = Self { fs, v, st: Default::default(), os: [Oversampler::new(), Oversampler::new()], lc: v[LOWCUT], gain: Smooth::new(fs, 20.0, 1.0), out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.st[c] = St::default();
            self.os[c].reset();
        }
        self.design();
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        if (v[LOWCUT] - self.lc).abs() > 0.05 {
            self.lc = v[LOWCUT];
            self.design();
        }
        self.gain.set(db_to_gain(v[GAIN] as f64));
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(2);
        let clf = coeff(self.fs * OS as f64, 30.0);
        let mut peak = 0.0f64;
        for k in 0..n {
            let g = self.gain.next();
            let o = self.out.next();
            for c in 0..nch {
                let x = chans[c][k] as f64 * g;
                let st = &mut self.st[c];
                let y = self.os[c].process(x, |u| {
                    // input transformer: the core saturates first in the lows
                    let s = st.xin.process(u);
                    st.lf = s + clf * (st.lf - s);
                    let s = s - 0.08 * (st.lf * 3.0).tanh() * st.lf.abs().min(1.0);
                    // two valve stages (about 1.6 V of headroom referred to full scale), the second a little lopsided
                    let s = tube(s * 0.25, 0.15) / 0.25;
                    let s = tube(s * 0.2, 0.25) / 0.2;
                    st.cut.process(st.dc.process(st.hf.process(s)))
                });
                peak = peak.max(y.abs());
                chans[c][k] = (y * o) as f32;
            }
        }
        meters[0] = peak as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Clean Ribbon Pre", values: &[] },
        Preset { name: "Warm Push (+8 dB)", values: &[(GAIN, 8.0), (OUTPUT, -8.0)] },
        Preset { name: "Driven Valves (+16 dB)", values: &[(GAIN, 16.0), (OUTPUT, -15.0)] },
        Preset { name: "Bass Cab, Low Cut", values: &[(GAIN, 4.0), (LOWCUT, 35.0), (OUTPUT, -4.0)] },
        Preset { name: "Vocal, Low Cut 80", values: &[(GAIN, 6.0), (LOWCUT, 80.0), (OUTPUT, -6.0)] },
    ]
}

/*
('meter', 232, 100, 176, 114)
('knob', 129.5, 169.5, 45.1)
('knob', 509.5, 169.5, 45.1)
('divider', 270)
('knob', 319.5, 333.5, 22.0)

crate: ['albini/pa_ampex351', 'library/panic_abort']
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
/root/.cargo/git/checkouts/nih-plug-d4210f82e920295f/de42101/nih_plug_egui/src/editor.rs
AFTER THE AMPEX 351 RECORD ELECTRONICSVALVE MIC PREAMP  
  TRANSFORMER IN AND OUTGAINHow hard the transformer and valves are driven. Normal levels are clean; push it for valve thickening.RECORD LEVELOUTPUTOutput level.LOW CUTHigh-pass after the preamp. 20 Hz = off.AMPEX 351Clean Ribbon Pre
Warm Push (+8 dB)
Driven Valves (+16 dB)
Bass Cab, Low Cut
Vocal, Low Cut 80
vintage/src/amp/tube.rs
GainLow CutOutputassertion failed: unsafe { QueryPerformanceCounter(&mut ticks).as_bool() }/root/.cargo/git/checkouts/nih-plug-d4210f82e920295f/de42101/src/event_loop/windows.rs
nih-event-loop-
NIH-plug event loop

*/
fn panel() -> Panel {
    Panel {
        title: "AMPEX 351",
        subtitle: ("AFTER THE AMPEX 351 RECORD ELECTRONICS", "VALVE MIC PREAMP  ·  TRANSFORMER IN AND OUT"),
        accent: ui::STEEL,
        size: (W, H),
        items: vec![
            Item::Knob { p: GAIN, x: 130.0, y: 170.0, r: 45.0, label: "GAIN", tip: "How hard the transformer and valves are driven. Normal levels are clean; push it for valve thickening.", active: None },
            Item::Meter { m: 0, x: 232.0, y: 100.0, w: 176.0, h: 124.0, kind: MeterKind::Vu, caption: "RECORD LEVEL" },
            Item::Knob { p: OUTPUT, x: 510.0, y: 170.0, r: 45.0, label: "OUTPUT", tip: "Output level.", active: None },
            Item::Divider { y: 270.0 },
            Item::Knob { p: LOWCUT, x: 320.0, y: 334.0, r: 22.0, label: "LOW CUT", tip: "High-pass after the preamp. 20 Hz = off.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Ampex 351", class_id: *b"PAAmpex351PQD001", vendor: "PEQUOD Albini",
    categories: [Fx, Distortion]
);
