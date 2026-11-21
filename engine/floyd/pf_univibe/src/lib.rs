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

#[derive(Clone, Copy, Default)]
struct Ap {
    z: f64,
}
impl Ap {
    #[inline]
    fn process(&mut self, x: f64, a: f64) -> f64 {
        let y = a * x + self.z;
        self.z = x - a * y;
        y
    }
}

/// The four stages' corners at full light (their capacitors differ: 15n, 220n, 470p, 4n7 in the original).
const CORNERS: [f64; 4] = [1100.0, 120.0, 4200.0, 650.0];

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ap: [[Ap; 4]; 2],
    ph: f64,
    lamp: f64,
    cell: f64,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, ap: [[Ap::default(); 4]; 2], ph: 0.0, lamp: 0.0, cell: 0.0, out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.ap = [[Ap::default(); 4]; 2];
        self.ph = 0.0;
        self.lamp = 0.0;
        self.cell = 0.0;
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
            let light = self.lamp * self.lamp;
            self.cell = light + lag * (self.cell - light);
            // photocell resistance falls with light: the corners rise about 12x from dark to full
            let span = 0.08 + 0.92 * self.cell;
            let mut a = [0.0; 4];
            for (i, f0) in CORNERS.iter().enumerate() {
                let f = (f0 * span).max(20.0);
                let t = (std::f64::consts::PI * f / self.fs).tan();
                a[i] = (t - 1.0) / (t + 1.0);
            }
            let o = self.out.next();
            for ch in 0..nch {
                let x = chans[ch][k] as f64;
                // the transistor stages colour it a little
                let x = x + 0.05 * x * x;
                let mut w = x;
                for (i, s) in self.ap[ch].iter_mut().enumerate() {
                    w = s.process(w, a[i]);
                }
                let y = if vib { w } else { 0.65 * (x + w) };
                chans[ch][k] = (y * o) as f32;
            }
        }
        meters[0] = self.lamp as f32;
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
