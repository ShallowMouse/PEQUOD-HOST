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

/// Output scaling from the pedal's volts to the plugin's level: the hardware's audio-taper volume
/// sits about 20 dB down at noon, and this brings the default setting to the expected level.
const MAKEUP: f64 = 11.8850; // +21.5 dB

use vintage::amp::net::Taper;
use vintage::circuit::{self, Circuit, Solver};

const C_SENS: usize = 0;
const C_OUT: usize = 1;
/// The OTA's bias pin and the 27k feeding it from the sensitivity pot, for the meter.
const N_ABC: usize = 12;
const N_IFB: usize = 19;

/// The whole MXR Dyna Comp at the sample rate `fs`.
///
/// 2N3904 emitter follower in; the CA3080 with both inputs biased from 470k off the supply
/// through 1M each, the non-inverting one grounded for audio (15k || 10n, 1u); its output into
/// 150k || 1n to the 3 V bias. Q2 splits the phase (10k / 10k); each side drives an envelope
/// detector (10n, 1N4148 clamp, 1M, a 2N3904) that pulls the 150k / 10u envelope node down. Q5
/// follows that node and feeds the OTA's bias pin through the sensitivity pot and 27k: a feedback
/// compressor. Out through 50n and 10k to the 50k output pot.
fn dyna_comp(fs: f64) -> Solver {
    let q = circuit::N3904;
    let d = circuit::D1N914;
    Circuit::new()
        .input(1, 7e3)
        .dc(2, 9.0, 5.0)
        // 3 V bias
        .r(2, 3, 56e3)
        .r(3, 0, 27e3)
        .c(3, 0, 1e-6)
        // input buffer
        .r(1, 0, 4.7e6)
        .c(1, 4, 10e-9)
        .r(4, 5, 10e3)
        .r(5, 3, 1e6)
        .npn(2, 5, 6, q)
        .r(6, 0, 10e3)
        // the OTA's inputs
        .c(6, 7, 1e-6)
        .r(7, 0, 1e6)
        .r(8, 0, 1e6)
        .r(7, 9, 1e3)
        .r(9, 8, 1e3)
        .r(2, 9, 470e3)
        .r(8, 10, 15e3)
        .c(8, 10, 10e-9)
        .c(10, 0, 1e-6)
        // CA3080: + is the grounded side, - takes the signal; output into 150k || 1n to the bias
        .ota_pin(8, 7, 11, N_ABC, 0)
        .r(3, 11, 150e3)
        .c(3, 11, 1e-9)
        // its output stage runs out of room near the rails
        .diode(11, 2, d)
        .diode(0, 11, d)
        // Q2 phase splitter
        .r(2, 13, 10e3)
        .r(14, 0, 10e3)
        .npn(13, 11, 14, q)
        // envelope detectors
        .c(13, 15, 10e-9)
        .diode(0, 15, d)
        .r(15, 0, 1e6)
        .npn(17, 15, 0, q)
        .c(14, 16, 10e-9)
        .diode(0, 16, d)
        .r(16, 0, 1e6)
        .npn(17, 16, 0, q)
        .r(2, 17, 150e3)
        .c(17, 0, 10e-6)
        // Q5 and the sensitivity pot into the bias pin
        .npn(2, 17, 18, q)
        .rheo(18, N_IFB, 500e3, Taper::Linear, C_SENS, 100.0)
        .r(N_IFB, N_ABC, 27e3)
        // out
        .c(14, 20, 50e-9)
        .r(20, 21, 10e3)
        .pot(0, 22, 21, 50e3, Taper::Audio, C_OUT)
        .r(22, 0, 1e6)
        .output(22)
        .build(fs)
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ckt: [Solver; 2],
    os: [Oversampler2; 2],
    dc: [DcBlock; 2],
    inp: Smooth,
    rest: f64,
}

impl Dsp {
    fn controls(&mut self) {
        for c in self.ckt.iter_mut() {
            // more sensitivity, less resistance in the bias feed
            c.set_ctl(C_SENS, 1.0 - self.v[SENSITIVITY] as f64 / 10.0);
            c.set_ctl(C_OUT, self.v[OUTPUT] as f64 / 10.0);
        }
    }
    fn iabc(c: &Solver) -> f64 {
        ((c.node(N_IFB) - c.node(N_ABC)) / 27e3).max(1e-12)
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let ckt = dyna_comp(fs * 2.0);
        let mut d = Self { fs, v, ckt: [ckt.clone(), ckt], os: [Oversampler2::new(), Oversampler2::new()], dc: [DcBlock::new(fs), DcBlock::new(fs)], inp: Smooth::new(fs, 20.0, 1.0), rest: 1.0 };
        d.reset();
        d
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn reset(&mut self) {
        self.controls();
        for c in 0..2 {
            self.ckt[c].reset();
            self.os[c].reset();
            self.dc[c] = DcBlock::new(self.fs);
        }
        self.rest = Self::iabc(&self.ckt[0]);
        self.inp.snap(db_to_gain(self.v[LEVEL] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        let sens_changed = v[SENSITIVITY] != self.v[SENSITIVITY];
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.controls();
        if sens_changed {
            // the meter's reference: the bias current with no signal at this setting
            let mut c = self.ckt[0].clone();
            c.reset();
            self.rest = Self::iabc(&c);
        }
        self.inp.set(db_to_gain(v[LEVEL] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(2);
        for k in 0..n {
            let gi = self.inp.next();
            for c in 0..nch {
                let x = chans[c][k] as f64 * gi;
                let ckt = &mut self.ckt[c];
                let y = self.os[c].process(x, |u| ckt.step(&[u]));
                chans[c][k] = (self.dc[c].process(y) * MAKEUP) as f32;
            }
        }
        // gain reduction: the OTA's gain follows its bias current
        let mut gr = 0.0f64;
        for c in 0..nch {
            gr = gr.max(-gain_to_db(Self::iabc(&self.ckt[c]) / self.rest));
        }
        meters[0] = gr.max(0.0) as f32;
    }
}

#[cfg(test)]
mod circuit_tests {
    use super::*;

    #[test]
    fn bias_and_compression() {
        let fs = 96000.0;
        let mut c = dyna_comp(fs);
        c.set_ctl(C_SENS, 0.5);
        c.set_ctl(C_OUT, 1.0);
        c.reset();
        let names = [(3, "bias"), (6, "Q1 e"), (7, "ota -"), (8, "ota +"), (11, "ota out"), (13, "Q2 c"), (14, "Q2 e"), (17, "env"), (18, "Q5 e"), (N_ABC, "abc")];
        for (n, s) in names {
            eprint!("{s} {:.2}  ", c.node(n));
        }
        eprintln!("iabc {:.1} uA", Dsp::iabc(&c) * 1e6);
        // steady-state output level against input level
        for sens in [0.0, 0.5, 1.0] {
            let mut line = format!("sens {sens}:");
            for a in [0.01, 0.03, 0.1, 0.3, 1.0] {
                let mut c = dyna_comp(fs);
                c.set_ctl(C_SENS, 1.0 - sens);
                c.set_ctl(C_OUT, 1.0);
                c.reset();
                let n = (fs * 1.5) as usize;
                let mut y2 = 0.0;
                for k in 0..n {
                    let y = c.step(&[a * (k as f64 * 2.0 * std::f64::consts::PI * 440.0 / fs).sin()]);
                    if k > n * 2 / 3 {
                        y2 += y * y;
                    }
                }
                let rms = (y2 / (n / 3) as f64).sqrt();
                line += &format!("  {:.0} dB in -> {:.1} dBV (iabc {:.0} uA)", 20.0 * (a / 2f64.sqrt()).log10(), 20.0 * rms.log10(), Dsp::iabc(&c) * 1e6);
            }
            eprintln!("{line}");
        }
        // chord steps against plain Newton, hard-driven at full sensitivity
        {
            let mut a = dyna_comp(fs);
            a.set_ctl(C_SENS, 0.0);
            a.set_ctl(C_OUT, 1.0);
            a.reset();
            let mut b = a.clone();
            b.chord = false;
            let (mut e2, mut s2) = (0.0, 0.0);
            for k in 0..(fs as usize) {
                let x = 1.0 * (k as f64 * 2.0 * std::f64::consts::PI * 440.0 / fs).sin();
                let (ya, yb) = (a.step(&[x]), b.step(&[x]));
                e2 += (ya - yb) * (ya - yb);
                s2 += yb * yb;
            }
            eprintln!("chord vs newton: {:.1} dB", 10.0 * (e2 / s2).log10());
            assert!(e2 / s2 < 1e-4);
        }
        let t = std::time::Instant::now();
        let mut c = dyna_comp(fs);
        c.reset();
        let mut hist = [0usize; 32];
        for k in 0..96000 {
            c.step(&[0.2 * (k as f64 * 2.0 * std::f64::consts::PI * 196.0 / fs).sin()]);
            hist[c.iters.min(31)] += 1;
        }
        eprintln!("iterations {:?}", &hist[..12]);
        eprintln!("1 s at 2x in {:.0} ms, size {:?}", t.elapsed().as_secs_f64() * 1e3, c.size());
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
