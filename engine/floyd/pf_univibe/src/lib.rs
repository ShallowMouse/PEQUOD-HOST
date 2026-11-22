//! Uni-Vibe: the Shin-ei / Univox Uni-Vibe, David Gilmour's throb on Breathe and Echoes (Pompeii): four photocell
//! phase stages, each a different size, lit by one flashing incandescent lamp. Chorus mixes the shifted and dry signals
//! (moving notches); Vibrato is the shifted signal alone.

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
    #[name = "Chorus"]
    Chorus,
    #[name = "Vibrato"]
    Vibrato,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "speed"]
    pub speed: FloatParam,
    #[id = "intensity"]
    pub intensity: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
    #[id = "mode"]
    pub mode: EnumParam<ModeSel>,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            speed: rate("Speed", 2.0, 0.4, 10.0),
            intensity: pct("Intensity", 0.7),
            volume: db("Volume", 0.0, -12.0, 12.0),
            mode: EnumParam::new("Mode", ModeSel::Chorus),
        }
    }
}

pub const SPEED: usize = 0;
pub const INTENSITY: usize = 1;
pub const VOLUME: usize = 2;
pub const MODE: usize = 3;
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
            self.speed.as_ptr(),
            self.intensity.as_ptr(),
            self.volume.as_ptr(),
            self.mode.as_ptr(),
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

/// The four stages' capacitors (collector side), in signal order.
const CAPS: [f64; 4] = [15e-9, 220e-9, 470e-12, 4.7e-9];
/// Photocell resistance: about 1M dark, a few k fully lit; CdS conductance rises a bit slower than the light.
const R_DARK: f64 = 1.0e6;
const R_LIT: f64 = 4.0e3;
/// How often (samples at the circuit's rate) the photocells' resistances are brought up to date.
const CELL_EVERY: usize = 16;
/// The bulb's glow with the driver at rest.
const LAMP_IDLE: f64 = 0.15;

fn cell_ohms(light: f64) -> f64 {
    1.0 / (1.0 / R_DARK + light.max(0.0).powf(0.85) / R_LIT)
}

/// The Uni-Vibe's four phase stages at `fs`, fed from the preamp (input 0).
///
/// Each is a 2N5133 phase splitter on 15 V: 470k / 180k base bias through 1u, 4k7 collector and
/// 4k7 emitter, so the collector and emitter carry equal and opposite signals; the collector feeds
/// the stage's capacitor and the emitter its photocell, and where they meet is the stage output.
/// The finite source and load impedances make each stage an imperfect all-pass, part of the sound.
fn vibe_stages(fs: f64) -> Solver {
    let q = circuit::N3904;
    let mut c = Circuit::new().input(1, 600.0).dc(2, 15.0, 10.0);
    let mut inp = 1;
    for k in 0..4 {
        let (b, col, e, o) = (3 + 4 * k, 4 + 4 * k, 5 + 4 * k, 6 + 4 * k);
        c = c
            .c(inp, b, 1e-6)
            .r(2, b, 470e3)
            .r(b, 0, 180e3)
            .r(2, col, 4.7e3)
            .r(e, 0, 4.7e3)
            .npn(col, b, e, q)
            .c(col, o, CAPS[k])
            .var(e, o);
        inp = o;
    }
    // into the output amplifier
    c.c(inp, 20, 1e-6).r(20, 0, 47e3).output(20).build(fs)
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ckt: [Solver; 2],
    os: [Oversampler2; 2],
    dc: [DcBlock; 2],
    ph: f64,
    lamp: f64,
    cell: f64,
    tick: usize,
    out: Smooth,
}

impl Dsp {
    fn cells(&mut self) {
        let r = cell_ohms(self.cell);
        for c in self.ckt.iter_mut() {
            for k in 0..4 {
                c.set_var(k, r);
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
        let c = vibe_stages(fs * 2.0);
        let mut d = Self { fs, v, ckt: [c.clone(), c], os: [Oversampler2::new(), Oversampler2::new()], dc: [DcBlock::new(fs), DcBlock::new(fs)], ph: 0.0, lamp: 0.0, cell: 0.0, tick: 0, out: Smooth::new(fs, 20.0, 1.0) };
        d.reset();
        d
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn reset(&mut self) {
        self.ph = 0.0;
        self.lamp = 0.0;
        self.cell = 0.0;
        self.tick = 0;
        self.cells();
        for c in 0..2 {
            self.ckt[c].reset();
            self.os[c].reset();
            self.dc[c] = DcBlock::new(self.fs);
        }
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[VOLUME] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let rate = self.v[SPEED] as f64;
        let inten = self.v[INTENSITY] as f64;
        let vib = self.v[MODE] > 0.5;
        let nch = chans.len().min(2);
        // the bulb heats fast and cools slowly; the cells follow it with their own lag
        let heat = coeff(self.fs, 12.0);
        let cool = coeff(self.fs, 40.0);
        let lag = coeff(self.fs, 8.0);
        for k in 0..n {
            self.ph = (self.ph + rate / self.fs).fract();
            let drive = (0.5 - 0.5 * (2.0 * std::f64::consts::PI * self.ph).cos()) * inten;
            let c = if drive > self.lamp { heat } else { cool };
            self.lamp = drive + c * (self.lamp - drive);
            // the driver keeps the bulb glowing between flashes
            let light = LAMP_IDLE + (1.0 - LAMP_IDLE) * self.lamp * self.lamp;
            self.cell = light + lag * (self.cell - light);
            self.tick += 2;
            if self.tick >= CELL_EVERY {
                self.tick = 0;
                self.cells();
            }
            let o = self.out.next();
            for ch in 0..nch {
                let x = chans[ch][k] as f64;
                let ckt = &mut self.ckt[ch];
                let w = self.os[ch].process(x, |u| ckt.step(&[u]));
                let w = self.dc[ch].process(w);
                // the mixer: the preamp's signal and the fourth stage's, or the stages alone
                let y = if vib { w * WET } else { 0.65 * (x + w * WET) };
                chans[ch][k] = (y * o) as f32;
            }
        }
        meters[0] = self.lamp as f32;
    }
}

/// Make-up for the stages' loss, so the shifted signal matches the dry one.
const WET: f64 = 1.5;

#[cfg(test)]
mod circuit_tests {
    use super::*;

    fn response(light: f64, f: f64) -> (f64, f64) {
        let fs = 96000.0;
        let mut c = vibe_stages(fs);
        for k in 0..4 {
            c.set_var(k, cell_ohms(light));
        }
        c.reset();
        let n = (fs * 0.3) as usize;
        let (mut a, mut b, mut cr) = (0.0, 0.0, 0.0);
        for k in 0..n {
            let x = 0.05 * (k as f64 * 2.0 * std::f64::consts::PI * f / fs).sin();
            let y = c.step(&[x]);
            if k > n / 2 {
                a += x * x;
                b += y * y;
                cr += x * y;
            }
        }
        (10.0 * (b / a).log10(), cr / (a * b).sqrt())
    }

    #[test]
    fn stages() {
        let fs = 96000.0;
        let c = vibe_stages(fs);
        eprintln!("bias: b {:.2} c {:.2} e {:.2}", c.node(3), c.node(4), c.node(5));
        for light in [0.15, 0.3, 0.6, 1.0] {
            let mut line = format!("light {light} ({:.0} ohm):", cell_ohms(light));
            for f in [60.0, 150.0, 400.0, 1000.0, 2500.0, 6000.0] {
                let (g, corr) = response(light, f);
                line += &format!(" {f}:{g:.1}dB/{corr:.2}");
            }
            eprintln!("{line}");
        }
        let mut c = vibe_stages(fs);
        let t = std::time::Instant::now();
        for k in 0..96000 {
            if k % 16 == 0 {
                for s in 0..4 {
                    c.set_var(s, cell_ohms((k as f64 / 20000.0).sin().abs()));
                }
            }
            c.step(&[0.2 * (k as f64 * 0.03).sin()]);
        }
        eprintln!("1 s at 2x in {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Breathe", values: &[(SPEED, 1.6), (INTENSITY, 0.6)] },
        Preset { name: "Echoes Throb (Pompeii)", values: &[(SPEED, 3.2), (INTENSITY, 0.8)] },
        Preset { name: "Slow Deep Swirl", values: &[(SPEED, 0.6), (INTENSITY, 1.0)] },
        Preset { name: "Fast Chorus", values: &[(SPEED, 6.5)] },
        Preset { name: "Vibrato", values: &[(SPEED, 4.5), (INTENSITY, 0.6), (MODE, 1.0)] },
    ]
}

/*
('switch', 325.0, 126.5, 37, 14)
('knob', 129.5, 159.5, 39.0)
('knob', 319.5, 159.5, 39.0)
('knob', 509.5, 159.5, 39.0)
('switch', 314.0, 192.5, 37, 14)
('divider', 284)

crate: ['floyd/pf_univibe', 'library/panic_abort', 'odefloyd/pf_univibe']
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
/root/.cargo/git/checkouts/nih-plug-d4210f82e920295f/de42101/nih_plug_egui/src/editor.rs
AFTER THE SHIN-EI / UNIVOX UNI-VIBE4 PHOTOCELL STAGES  
  INCANDESCENT LAMPSPEEDLamp rate (the foot controller on the real one).INTENSITYHow hard the lamp is flashed: depth of the sweep.VOLUMEOutput level.LAMPCHORUSVIBRATOCHORUS: shifted + dry (moving notches, the classic sound). VIBRATO: shifted signal only (pitch wobble).UNI-VIBE
Breathe
Echoes Throb (Pompeii)
Slow Deep Swirl
333?
Fast Chorus
Vibrato
j?$Dsp
)"8	

*/
fn panel() -> Panel {
    Panel {
        title: "UNI-VIBE",
        subtitle: ("AFTER THE SHIN-EI / UNIVOX UNI-VIBE", "4 PHOTOCELL STAGES  ·  INCANDESCENT LAMP"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![
            Item::Knob { p: SPEED, x: 130.0, y: 160.0, r: 40.0, label: "SPEED", tip: "Lamp rate (the foot controller on the real one).", active: None },
            Item::Knob { p: INTENSITY, x: 320.0, y: 160.0, r: 40.0, label: "INTENSITY", tip: "How hard the lamp is flashed: depth of the sweep.", active: None },
            Item::Knob { p: VOLUME, x: 510.0, y: 160.0, r: 40.0, label: "VOLUME", tip: "Output level.", active: None },
            Item::Lamp { m: 0, x: 320.0, y: 248.0, label: "LAMP", lo: 0.0, hi: 1.0 },
            Item::Divider { y: 284.0 },
            Item::Switch { p: MODE, x: 320.0, y: 336.0, labels: ["CHORUS", "VIBRATO"], tip: "CHORUS: shifted + dry (moving notches, the classic sound). VIBRATO: shifted signal only (pitch wobble)." },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Uni-Vibe", class_id: *b"PFUniVibePQD0001", vendor: "PEQUOD Pink Floyd",
    categories: [Fx, Modulation]
);
