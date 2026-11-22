//! Percolator: the Interfax Harmonic Percolator, Steve Albini's fuzz (Big Black, Shellac): a germanium and a silicon
//! transistor with germanium diodes, asymmetric and splattery; Stock (the original values) or the later, tamer Albini version.

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
pub enum Version {
    #[name = "Stock (Interfax)"]
    StockInterfax,
    #[name = "Albini version"]
    AlbiniVersion,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "harmonics"]
    pub harmonics: FloatParam,
    #[id = "balance"]
    pub balance: FloatParam,
    #[id = "version"]
    pub version: EnumParam<Version>,
    #[id = "level"]
    pub level: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            harmonics: FloatParam::new("Harmonics", 7.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            balance: FloatParam::new("Balance", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            version: EnumParam::new("Version", Version::StockInterfax),
            level: db("Guitar Level", 0.0, -12.0, 12.0),
        }
    }
}

pub const HARMONICS: usize = 0;
pub const BALANCE: usize = 1;
pub const VERSION: usize = 2;
pub const LEVEL: usize = 3;
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
            self.harmonics.as_ptr(),
            self.balance.as_ptr(),
            self.version.as_ptr(),
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

const OS: usize = 4;

#[derive(Clone, Default)]
struct St {
    hp: Biquad,
    lp: Biquad,
    tone: Biquad,
    dc: Biquad,
    sag: f64,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    st: [St; 2],
    os: [Oversampler; 2],
    ver: i32,
    inp: Smooth,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let ofs = self.fs * OS as f64;
        let albini = self.ver == 1;
        for st in self.st.iter_mut() {
            // the stock coupling caps are small (thin, wiry); Albini's are bigger (fuller)
            st.hp.set_highpass(ofs, if albini { 350.0 } else { 600.0 }, 0.7);
            st.lp.set_lowpass(ofs, if albini { 5000.0 } else { 7000.0 }, 0.7);
            st.tone.set_peak(ofs, 1600.0, 0.8, if albini { 1.0 } else { 4.0 });
            st.dc.set_highpass(ofs, 25.0, 0.7);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, st: Default::default(), os: [Oversampler::new(), Oversampler::new()], ver: v[VERSION].round() as i32, inp: Smooth::new(fs, 20.0, 1.0), out: Smooth::new(fs, 20.0, 0.25) };
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
        let ver = v[VERSION].round() as i32;
        if ver != self.ver {
            self.ver = ver;
            self.design();
        }
        let h = v[HARMONICS] as f64 / 10.0;
        self.inp.set(db_to_gain(v[LEVEL] as f64) * (2.0 + 120.0 * h * h));
        let b = v[BALANCE] as f64 / 10.0;
        self.out.set(b * b);
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(2);
        let stock = self.ver == 0;
        let sag_c = coeff(self.fs * OS as f64, if stock { 60.0 } else { 30.0 });
        let mut peak = 0.0f64;
        for k in 0..n {
            let g = self.inp.next();
            let o = self.out.next();
            for c in 0..nch {
                let x = chans[c][k] as f64;
                let st = &mut self.st[c];
                let y = self.os[c].process(x, |u| {
                    let s = st.hp.process(u) * g;
                    // the germanium stage's bias shifts with the signal: the stock one splatters and gates
                    let a = s.abs().min(4.0);
                    st.sag = a + sag_c * (st.sag - a);
                    let b = (if stock { 0.45 } else { 0.25 }) * (st.sag * 0.5).tanh();
                    let w = if s - b > 0.0 { (s - b).tanh() } else { 0.6 * ((s - b) / 0.6).tanh() };
                    // germanium diodes to ground
                    let w = st.tone.process(w);
                    let w = 0.35 * (w / 0.35).tanh();
                    st.dc.process(st.lp.process(w))
                });
                peak = peak.max(y.abs());
                chans[c][k] = (y * o * 2.0) as f32;
            }
        }
        meters[0] = peak as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Shellac Chords", values: &[(HARMONICS, 6.5)] },
        Preset { name: "Big Black Shriek", values: &[(HARMONICS, 9.0), (BALANCE, 4.5)] },
        Preset { name: "Low Harmonics (edge)", values: &[(HARMONICS, 3.5), (BALANCE, 7.0)] },
        Preset { name: "Albini Version", values: &[(BALANCE, 5.5), (VERSION, 1.0)] },
        Preset { name: "Feedback Squeal (stand close)", values: &[(HARMONICS, 10.0)] },
    ]
}

/*
('knob', 199.5, 159.5, 43.9)
('knob', 439.5, 159.5, 43.9)
('divider', 270)
('knob', 419.5, 333.5, 22.0)

crate: ['albini/pa_percolator', 'library/panic_abort']
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
/root/.cargo/git/checkouts/nih-plug-d4210f82e920295f/de42101/nih_plug_egui/src/editor.rs
AFTER THE INTERFAX HARMONIC PERCOLATORSILICON + GERMANIUM  
  GERMANIUM DIODESHARMONICSInput level into the fuzz: how hard the stages are hit.BALANCEOutput level.OUTPUTVERSIONSTOCKALBINISTOCK: the original Interfax values (wilder, more compressed). ALBINI: the later, tamer and fuller version.GUITAR LEVELHow hot your DI is (0 dB treats full scale as 1 V from the pickups).PERCOLATOR
Shellac Chords
Big Black Shriek
Low Harmonics (edge)
Albini Version
Feedback Squeal (stand close)
?albini/pa_percolator/src/lib.rs
HarmonicsBalanceVersionGuitar Levelassertion failed: unsafe { QueryPerformanceCounter(&mut ticks).as_bool() }/root/.cargo/git/checkouts/nih-plug-d4210f82e920295f/de42101/src/event_loop/windows.rs
nih-event-loop-

*/
fn panel() -> Panel {
    Panel {
        title: "HARMONIC PERCOLATOR",
        subtitle: ("AFTER THE INTERFAX HARMONIC PERCOLATOR", "SILICON + GERMANIUM  ·  GERMANIUM DIODES"),
        accent: ui::TANGERINE,
        size: (W, H),
        items: vec![
            Item::Knob { p: HARMONICS, x: 200.0, y: 160.0, r: 44.0, label: "HARMONICS", tip: "Input level into the fuzz: how hard the stages are hit.", active: None },
            Item::Lamp { m: 0, x: 320.0, y: 160.0, label: "OUTPUT", lo: 0.05, hi: 0.6 },
            Item::Knob { p: BALANCE, x: 440.0, y: 160.0, r: 44.0, label: "BALANCE", tip: "Output level.", active: None },
            Item::Divider { y: 270.0 },
            Item::Text { x: 230.0, y: 302.0, text: "VERSION", size: 13.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: VERSION, x: 230.0, y: 338.0, labels: ["STOCK", "ALBINI"], tip: "STOCK: the original Interfax values (wilder, more compressed). ALBINI: the later, tamer and fuller version." },
            Item::Knob { p: LEVEL, x: 420.0, y: 334.0, r: 22.0, label: "GUITAR LEVEL", tip: "How hot your DI is (0 dB treats full scale as 1 V from the pickups).", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Harmonic Percolator", class_id: *b"PAPercolatorPQD1", vendor: "PEQUOD Albini",
    categories: [Fx, Distortion]
);
