//! IVP Preamp: the Intersound IVP (1978) as Steve Albini used it (Shellac, Big Black): a six-band EQ in front of the
//! 'Tube Voice' drive channel (op-amp gain into a transistor stage and a transformer), into a power amp.

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
pub enum Channel {
    #[name = "Clean"]
    Clean,
    #[name = "Tube Voice (drive)"]
    TubeVoiceDrive,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "bass"]
    pub bass: FloatParam,
    #[id = "f1"]
    pub f1: FloatParam,
    #[id = "g1"]
    pub g1: FloatParam,
    #[id = "f2"]
    pub f2: FloatParam,
    #[id = "g2"]
    pub g2: FloatParam,
    #[id = "f3"]
    pub f3: FloatParam,
    #[id = "g3"]
    pub g3: FloatParam,
    #[id = "f4"]
    pub f4: FloatParam,
    #[id = "g4"]
    pub g4: FloatParam,
    #[id = "treble"]
    pub treble: FloatParam,
    #[id = "channel"]
    pub channel: EnumParam<Channel>,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            bass: db("Bass", 0.0, -15.0, 15.0),
            f1: hz("Low Mid Freq", 180.0, 60.0, 600.0),
            g1: db("Low Mid", 0.0, -15.0, 15.0),
            f2: hz("Mid Freq", 600.0, 200.0, 2000.0),
            g2: db("Mid", 0.0, -15.0, 15.0),
            f3: hz("High Mid Freq", 1800.0, 600.0, 5000.0),
            g3: db("High Mid", 0.0, -15.0, 15.0),
            f4: hz("Presence Freq", 4500.0, 2000.0, 10000.0),
            g4: db("Presence", 0.0, -15.0, 15.0),
            treble: db("Treble", 0.0, -15.0, 15.0),
            channel: EnumParam::new("Channel", Channel::TubeVoiceDrive),
            drive: pct("Drive", 0.6),
            output: db("Output", -6.0, -30.0, 6.0),
        }
    }
}

pub const BASS: usize = 0;
pub const F1: usize = 1;
pub const G1: usize = 2;
pub const F2: usize = 3;
pub const G2: usize = 4;
pub const F3: usize = 5;
pub const G3: usize = 6;
pub const F4: usize = 7;
pub const G4: usize = 8;
pub const TREBLE: usize = 9;
pub const CHANNEL: usize = 10;
pub const DRIVE: usize = 11;
pub const OUTPUT: usize = 12;
pub const NPARAMS: usize = 13;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.bass.as_ptr(),
            self.f1.as_ptr(),
            self.g1.as_ptr(),
            self.f2.as_ptr(),
            self.g2.as_ptr(),
            self.f3.as_ptr(),
            self.g3.as_ptr(),
            self.f4.as_ptr(),
            self.g4.as_ptr(),
            self.treble.as_ptr(),
            self.channel.as_ptr(),
            self.drive.as_ptr(),
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

const OS: usize = 4;

#[derive(Clone, Default)]
struct St {
    eq: [Biquad; 6],
    hp: Biquad,
    lp: Biquad,
    xf: f64,
    dc: Biquad,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    st: [St; 2],
    os: [Oversampler; 2],
    key: Vec<i32>,
    drive: Smooth,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let ofs = self.fs * OS as f64;
        let v = self.v;
        for st in self.st.iter_mut() {
            st.eq[0].set_low_shelf(ofs, 100.0, 0.6, v[BASS] as f64);
            st.eq[1].set_peak(ofs, v[F1] as f64, 1.0, v[G1] as f64);
            st.eq[2].set_peak(ofs, v[F2] as f64, 1.0, v[G2] as f64);
            st.eq[3].set_peak(ofs, v[F3] as f64, 1.0, v[G3] as f64);
            st.eq[4].set_peak(ofs, v[F4] as f64, 1.0, v[G4] as f64);
            st.eq[5].set_high_shelf(ofs, 6000.0, 0.6, v[TREBLE] as f64);
            st.hp.set_highpass(ofs, 40.0, 0.7);
            st.lp.set_lowpass(ofs, 9000.0, 0.7);
            st.dc.set_highpass(ofs, 20.0, 0.7);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, st: Default::default(), os: [Oversampler::new(), Oversampler::new()], key: vec![], drive: Smooth::new(fs, 20.0, 1.0), out: Smooth::new(fs, 20.0, 1.0) };
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
        let k: Vec<i32> = v[..10].iter().map(|x| (x * 10.0).round() as i32).collect();
        if k != self.key {
            self.key = k;
            self.design();
        }
        let d = v[DRIVE] as f64;
        self.drive.set(1.0 + 50.0 * d * d);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(2);
        let drive_ch = self.v[CHANNEL] > 0.5;
        let cxf = coeff(self.fs * OS as f64, 20.0);
        let mut peak = 0.0f64;
        for k in 0..n {
            let g = self.drive.next();
            let o = self.out.next();
            for c in 0..nch {
                let x = chans[c][k] as f64;
                let st = &mut self.st[c];
                let y = self.os[c].process(x, |u| {
                    let mut s = u;
                    for b in st.eq.iter_mut() {
                        s = b.process(s);
                    }
                    if !drive_ch {
                        return s;
                    }
                    // op-amp gain, clipping at the +-15 V rails, into the transistor stage (lopsided)
                    let s = st.hp.process(s) * g;
                    let s = 3.0 * (s / 3.0).tanh();
                    let s = if s > 0.0 { (s * 0.5).tanh() * 2.0 } else { (s * 0.7).tanh() / 0.7 * 0.9 };
                    // the transformer: the core saturates in the lows
                    st.xf = s + cxf * (st.xf - s);
                    let s = s - 0.1 * st.xf * st.xf.abs().min(1.0);
                    st.dc.process(st.lp.process(s)) * 0.23
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
        Preset { name: "Shellac (drive into Bassman)", values: &[(BASS, -3.0), (G1, 2.0), (G3, 4.0), (TREBLE, 3.0), (DRIVE, 0.65)] },
        Preset { name: "Clang", values: &[(BASS, -5.0), (F3, 2500.0), (G3, 6.0), (TREBLE, 5.0), (DRIVE, 0.5)] },
        Preset { name: "Thick Grind", values: &[(G1, 4.0), (G2, 3.0), (TREBLE, -2.0), (DRIVE, 0.8)] },
        Preset { name: "Scooped Drive", values: &[(BASS, 3.0), (F2, 650.0), (G2, -8.0), (TREBLE, 3.0), (DRIVE, 0.7)] },
        Preset { name: "Clean Channel", values: &[(CHANNEL, 0.0), (OUTPUT, 0.0)] },
    ]
}

/*
('knob', 47.5, 149.5, 17.1)
('knob', 108.0, 149.5, 17.7)
('knob', 168.5, 149.5, 17.1)
('knob', 229.0, 149.5, 20.1)
('knob', 289.5, 149.5, 17.1)
('knob', 350.0, 149.5, 17.7)
('knob', 410.5, 149.5, 17.1)
('knob', 471.0, 149.5, 17.7)
('knob', 531.5, 149.5, 17.1)
('knob', 592.0, 149.5, 17.7)
('divider', 230)
('knob', 279.5, 315.5, 29.3)
('knob', 419.5, 315.5, 25.6)
('selector?', 129.5, 319.5, 36, 36)

crate: ['hannelalbini/pa_ivp', 'library/panic_abort']
vintage/src/ui.rs
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
AFTER THE INTERSOUND IVP, 19786-BAND EQ BEFORE THE DRIVE  
  +-15 VBASSBaxandall bass, before the drive.LOW FREQFirst semi-parametric band: frequency.LOWFirst band: boost/cut.MID FREQSecond band: frequency.MIDSecond band: boost/cut.HI-MID FREQThird band: frequency.HI-MIDThird band: boost/cut.PRES. FREQFourth band: frequency.PRESENCEFourth band: boost/cut.TREBLEBaxandall treble, before the drive.CLEANDRIVE
CHANNELCLEAN: the EQ only. DRIVE: the 'Tube Voice' overdrive channel, the only one Albini used.Op-amp gain into the transistor stage and transformer.OUTPUTOutput level (into the power amp).LEVELIVP PREAMP
ff&?
Shellac (drive into Bassman)
Clang
Thick Grind
333?
Scooped Drive
Clean Channelalbini/pa_ivp/src/lib.rs

*/
fn panel() -> Panel {
    Panel {
        title: "IVP PREAMP",
        subtitle: ("AFTER THE INTERSOUND IVP, 1978", "6-BAND EQ BEFORE THE DRIVE  ·  +-15 V"),
        accent: ui::OXBLOOD,
        size: (W, H),
        items: vec![
            Item::Knob { p: BASS, x: 47.5, y: 150.0, r: 18.0, label: "BASS", tip: "Baxandall bass, before the drive.", active: None },
            Item::Knob { p: F1, x: 108.0, y: 150.0, r: 18.0, label: "LOW FREQ", tip: "First semi-parametric band: frequency.", active: None },
            Item::Knob { p: G1, x: 168.5, y: 150.0, r: 18.0, label: "LOW", tip: "First band: boost/cut.", active: None },
            Item::Knob { p: F2, x: 229.0, y: 150.0, r: 18.0, label: "MID FREQ", tip: "Second band: frequency.", active: None },
            Item::Knob { p: G2, x: 289.5, y: 150.0, r: 18.0, label: "MID", tip: "Second band: boost/cut.", active: None },
            Item::Knob { p: F3, x: 350.0, y: 150.0, r: 18.0, label: "HI-MID FREQ", tip: "Third band: frequency.", active: None },
            Item::Knob { p: G3, x: 410.5, y: 150.0, r: 18.0, label: "HI-MID", tip: "Third band: boost/cut.", active: None },
            Item::Knob { p: F4, x: 471.0, y: 150.0, r: 18.0, label: "PRES. FREQ", tip: "Fourth band: frequency.", active: None },
            Item::Knob { p: G4, x: 531.5, y: 150.0, r: 18.0, label: "PRESENCE", tip: "Fourth band: boost/cut.", active: None },
            Item::Knob { p: TREBLE, x: 592.0, y: 150.0, r: 18.0, label: "TREBLE", tip: "Baxandall treble, before the drive.", active: None },
            Item::Divider { y: 230.0 },
            Item::Selector { p: CHANNEL, x: 130.0, y: 320.0, label: "CHANNEL", positions: &["CLEAN", "DRIVE"], tip: "CLEAN: the EQ only. DRIVE: the 'Tube Voice' overdrive channel, the only one Albini used." },
            Item::Knob { p: DRIVE, x: 280.0, y: 316.0, r: 30.0, label: "DRIVE", tip: "Op-amp gain into the transistor stage and transformer.", active: None },
            Item::Knob { p: OUTPUT, x: 420.0, y: 316.0, r: 26.0, label: "OUTPUT", tip: "Output level (into the power amp).", active: None },
            Item::Lamp { m: 0, x: 540.0, y: 316.0, label: "LEVEL", lo: 0.05, hi: 0.8 },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "IVP Preamp", class_id: *b"PAIVPPreampPQD01", vendor: "PEQUOD Albini",
    categories: [Fx, Distortion]
);
