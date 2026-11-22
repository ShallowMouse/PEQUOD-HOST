//! Phase 90: the MXR Phase 90 (script logo), David Gilmour's phaser on Shine On You Crazy Diamond and Have a Cigar: four
//! JFET-swept all-pass stages mixed with the dry signal. Script: no feedback (Gilmour's); Block: the later 24k feedback resistor.

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
pub enum Logo {
    #[name = "Script (no feedback)"]
    ScriptNoFeedback,
    #[name = "Block (feedback)"]
    BlockFeedback,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "speed"]
    pub speed: FloatParam,
    #[id = "logo"]
    pub logo: EnumParam<Logo>,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            speed: rate("Speed", 0.5, 0.08, 9.0),
            logo: EnumParam::new("Version", Logo::ScriptNoFeedback),
            output: db("Output", 0.0, -12.0, 6.0),
        }
    }
}

pub const SPEED: usize = 0;
pub const LOGO: usize = 1;
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
            self.speed.as_ptr(),
            self.logo.as_ptr(),
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

use vintage::circuit::{self, Circuit, Solver};

const VREF: f64 = 5.1;
/// The JFETs' gate-source voltage across the sweep: near pinch-off (slow end) to the fast end,
/// as the bias trimmer sets it for 2N5952s.
const VGS_LO: f64 = -1.58;
const VGS_HI: f64 = -1.12;
const N_SUM: usize = 21;
const MIX_GAIN: f64 = 1.34;

/// The audio path of the Phase 90 at `fs`.
///
/// 10n in, 470k to the 5.1 V reference, a unity-gain buffer; four TL06x all-pass stages (10k in, 10k feedback, 47n to
/// the + input, which goes to Vref through 24k in parallel with a 2N5952's channel); the dry
/// signal and the fourth stage's output mixed through 150k each into the output transistor.
/// `block`: the block-logo's 24k from the last stage back into the second stage.
/// Input 0 is the guitar, input 1 the gate voltage from the LFO.
fn phase90(block: bool, fs: f64) -> Solver {
    let j = circuit::J2N5952;
    let mut c = Circuit::new()
        .input(1, 7e3)
        .dc(2, VREF, 10.0)
        .input(30, 1.0)
        .c(1, 31, 10e-9)
        .r(31, 2, 470e3)
        // unity-gain input buffer
        .linamp(31, 3, 3, 2e5);
    let mut inp = 3;
    let mut outs = [0usize; 4];
    for k in 0..4 {
        let (m, p, o) = (4 + 4 * k, 5 + 4 * k, 6 + 4 * k);
        c = c.r(inp, m, 10e3).r(o, m, 10e3).c(inp, p, 47e-9).r(p, 2, 24e3).jfet(p, 30, 2, j).linamp(p, m, o, 2e5);
        outs[k] = o;
        inp = o;
    }
    if block {
        c = c.r(outs[3], 9, 24e3);
    }
    c.r(outs[3], N_SUM, 150e3).r(3, N_SUM, 150e3).r(N_SUM, 2, 470e3).output(N_SUM).build(fs)
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ckt: [Solver; 2],
    os: [Oversampler2; 2],
    dc: [DcBlock; 2],
    block: bool,
    ph: f64,
    out: Smooth,
}

impl Dsp {
    fn rebuild(&mut self) {
        self.block = self.v[LOGO] > 0.5;
        let c = phase90(self.block, self.fs * 2.0);
        self.ckt = [c.clone(), c];
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let c = phase90(false, fs * 2.0);
        let mut d = Self { fs, v, ckt: [c.clone(), c], os: [Oversampler2::new(), Oversampler2::new()], dc: [DcBlock::new(fs), DcBlock::new(fs)], block: false, ph: 0.0, out: Smooth::new(fs, 20.0, 1.0) };
        d.rebuild();
        d
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn reset(&mut self) {
        self.ph = 0.0;
        self.rebuild();
        for c in 0..2 {
            self.os[c].reset();
            self.dc[c] = DcBlock::new(self.fs);
        }
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        if (v[LOGO] > 0.5) != self.block {
            self.rebuild();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let rate = self.v[SPEED] as f64;
        let nch = chans.len().min(2);
        let mut m = 0.0;
        for k in 0..n {
            // the LFO: an integrator and Schmitt trigger, so a triangle on the gates
            self.ph = (self.ph + rate / self.fs).fract();
            let tri = 1.0 - 2.0 * (self.ph - 0.5).abs();
            m = tri;
            let vg = VREF + VGS_LO + (VGS_HI - VGS_LO) * tri;
            let o = self.out.next();
            for c in 0..nch {
                let x = chans[c][k] as f64;
                let ckt = &mut self.ckt[c];
                // the output transistor: about 2.5 dB over the mixer's halving
                let y = self.os[c].process(x, |u| (ckt.step(&[u, vg]) - VREF) * MIX_GAIN);
                chans[c][k] = (self.dc[c].process(y) * o) as f32;
            }
        }
        meters[0] = m as f32;
    }
}

#[cfg(test)]
mod circuit_tests {
    use super::*;

    /// Level through the pedal at `f` Hz with the gates held at sweep position `tri`.
    fn gain_at(block: bool, tri: f64, f: f64) -> f64 {
        let fs = 96000.0;
        let mut c = phase90(block, fs);
        let vg = VREF + VGS_LO + (VGS_HI - VGS_LO) * tri;
        let (mut a, mut b) = (0.0, 0.0);
        let n = (fs * 0.25) as usize;
        for k in 0..n {
            let x = 0.05 * (k as f64 * 2.0 * std::f64::consts::PI * f / fs).sin();
            let y = (c.step(&[x, vg]) - VREF) * MIX_GAIN;
            if k > n / 2 {
                a += x * x;
                b += y * y;
            }
        }
        10.0 * (b / a).log10()
    }

    #[test]
    fn notches_sweep() {
        for block in [false, true] {
            for tri in [0.0, 0.5, 1.0] {
                // local minima on a log grid: the notches
                let mut g = vec![];
                let mut f = 60.0;
                while f < 16000.0 {
                    g.push((f, gain_at(block, tri, f)));
                    f *= 2f64.powf(1.0 / 12.0);
                }
                let mut line = String::new();
                for i in 1..g.len() - 1 {
                    if g[i].1 < g[i - 1].1 && g[i].1 < g[i + 1].1 && g[i].1 < -6.0 {
                        line += &format!(" notch {:.0} Hz ({:.0} dB)", g[i].0, g[i].1);
                    }
                }
                let peak = g.iter().map(|x| x.1).fold(f64::MIN, f64::max);
                let (best, fbest) = (peak, 0.0);
                let _ = fbest;
                eprintln!("block {block} sweep {tri}: peak {best:.1} dB |{line}");
            }
        }
        let fs = 96000.0;
        let mut c = phase90(false, fs);
        let t = std::time::Instant::now();
        for k in 0..96000 {
            let tri = (k as f64 / 96000.0 * 2.0).fract();
            c.step(&[0.2 * (k as f64 * 0.03).sin(), VREF + VGS_LO + (VGS_HI - VGS_LO) * tri]);
        }
        eprintln!("1 s at 2x in {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Shine On Riff (slow)", values: &[(SPEED, 0.45)] },
        Preset { name: "Have a Cigar", values: &[(SPEED, 1.1)] },
        Preset { name: "Very Slow Sweep", values: &[(SPEED, 0.12)] },
        Preset { name: "Fast Warble", values: &[(SPEED, 5.5)] },
        Preset { name: "Block Logo Swoosh", values: &[(SPEED, 0.6), (LOGO, 1.0)] },
    ]
}

/*
('knob', 319.5, 159.5, 53.7)
('divider', 284)
('knob', 419.5, 335.5, 22.0)

crate: ['floyd/pf_phase90', 'library/panic_abort']
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
/root/.cargo/git/checkouts/nih-plug-d4210f82e920295f/de42101/nih_plug_egui/src/editor.rs
AFTER THE MXR PHASE 90, SCRIPT LOGO4 JFET STAGES  
  ONE KNOBSPEEDSweep rate: about 0.08 to 9 Hz.SWEEPSCRIPTBLOCKSCRIPT: the 1970s circuit, no feedback (Gilmour's). BLOCK: later units with a 24k feedback resistor: more resonant.OUTPUTOutput level (the pedal itself has none).PHASE 90
Shine On Riff (slow)
Have a Cigar
Very Slow Sweep
Fast Warble
Block Logo Swoosh
SpeedVersionOutputassertion failed: unsafe { QueryPerformanceCounter(&mut ticks).as_bool() }/root/.cargo/git/checkouts/nih-plug-d4210f82e920295f/de42101/src/event_loop/windows.rs
nih-event-loop-
NIH-plug event loop

*/
fn panel() -> Panel {
    Panel {
        title: "PHASE 90",
        subtitle: ("AFTER THE MXR PHASE 90, SCRIPT LOGO", "4 JFET STAGES  ·  ONE KNOB"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![
            Item::Knob { p: SPEED, x: 320.0, y: 160.0, r: 54.0, label: "SPEED", tip: "Sweep rate: about 0.08 to 9 Hz.", active: None },
            Item::Lamp { m: 0, x: 460.0, y: 160.0, label: "SWEEP", lo: 0.0, hi: 1.0 },
            Item::Divider { y: 284.0 },
            Item::Switch { p: LOGO, x: 220.0, y: 336.0, labels: ["SCRIPT", "BLOCK"], tip: "SCRIPT: the 1970s circuit, no feedback (Gilmour's). BLOCK: later units with a 24k feedback resistor: more resonant." },
            Item::Knob { p: OUTPUT, x: 420.0, y: 336.0, r: 22.0, label: "OUTPUT", tip: "Output level (the pedal itself has none).", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Phase 90", class_id: *b"PFPhase90PQD0001", vendor: "PEQUOD Pink Floyd",
    categories: [Fx, Modulation]
);
