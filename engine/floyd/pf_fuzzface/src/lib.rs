//! Fuzz Face: the Dallas Arbiter Fuzz Face as David Gilmour used it from 1970 (2 x BC108 silicon), or the earlier AC128 germanium. Two transistors with feedback; the low input impedance loads the pickup, so the guitar's volume cleans it up.

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
pub enum Transistors {
    #[name = "BC108 silicon"]
    Bc108Silicon,
    #[name = "AC128 germanium"]
    Ac128Germanium,
}

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum Pickup {
    #[name = "Single coil"]
    SingleCoil,
    #[name = "Humbucker"]
    Humbucker,
    #[name = "Buffered (no loading)"]
    BufferedNoLoading,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "volume"]
    pub volume: FloatParam,
    #[id = "fuzz"]
    pub fuzz: FloatParam,
    #[id = "guitar_vol"]
    pub guitar_vol: FloatParam,
    #[id = "bias"]
    pub bias: FloatParam,
    #[id = "transistors"]
    pub transistors: EnumParam<Transistors>,
    #[id = "pickup"]
    pub pickup: EnumParam<Pickup>,
    #[id = "level"]
    pub level: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            volume: FloatParam::new("Volume", 6.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            fuzz: FloatParam::new("Fuzz", 8.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            guitar_vol: FloatParam::new("Guitar Volume", 10.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            bias: FloatParam::new("Bias", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            transistors: EnumParam::new("Transistors", Transistors::Bc108Silicon),
            pickup: EnumParam::new("Pickup", Pickup::SingleCoil),
            level: db("Guitar Level", 0.0, -12.0, 12.0),
        }
    }
}

pub const VOLUME: usize = 0;
pub const FUZZ: usize = 1;
pub const GUITAR_VOL: usize = 2;
pub const BIAS: usize = 3;
pub const TRANSISTORS: usize = 4;
pub const PICKUP: usize = 5;
pub const LEVEL: usize = 6;
pub const NPARAMS: usize = 7;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.volume.as_ptr(),
            self.fuzz.as_ptr(),
            self.guitar_vol.as_ptr(),
            self.bias.as_ptr(),
            self.transistors.as_ptr(),
            self.pickup.as_ptr(),
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
    pick: Biquad,
    inp: Biquad,
    mid: Biquad,
    lp: Biquad,
    post: Biquad,
    sag: f64,
}

struct Chan {
    os: Oversampler,
    st: St,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ch: Vec<Chan>,
    key: Vec<i32>,
    out: Smooth,
    inp: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let ofs = self.fs * OS as f64;
        let ge = self.v[TRANSISTORS] > 0.5;
        let pickup = self.v[PICKUP].round() as i32;
        let gv = self.v[GUITAR_VOL] as f64 / 10.0;
        for c in self.ch.iter_mut() {
            let st = &mut c.st;
            // the pickup's inductance into the pedal's low input impedance: a resonance pulled down and damped,
            // less so as the guitar's volume pot adds series resistance; a buffer sees none of it
            match pickup {
                0 => st.pick.set_lowpass(ofs, 3200.0 + 1500.0 * (1.0 - gv), 0.8),
                1 => st.pick.set_lowpass(ofs, 2000.0 + 1000.0 * (1.0 - gv), 0.75),
                _ => st.pick = Biquad::identity(),
            }
            st.inp.set_highpass(ofs, 90.0, 0.6);
            st.mid.set_peak(ofs, 900.0, 0.7, if ge { 1.0 } else { 2.0 });
            st.lp.set_lowpass(ofs, if ge { 5000.0 } else { 7500.0 }, 0.7);
            st.post.set_highpass(ofs, 25.0, 0.7);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, ch: (0..ch.max(2)).map(|_| Chan { os: Oversampler::new(), st: St::default() }).collect(), key: vec![], out: Smooth::new(fs, 20.0, 1.0), inp: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn reset(&mut self) {
        for c in self.ch.iter_mut() {
            c.os.reset();
            c.st = St::default();
        }
        self.design();
        let vol = self.v[VOLUME] as f64 / 10.0;
        self.out.snap(vol * vol);
        self.inp.snap(db_to_gain(self.v[LEVEL] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k: Vec<i32> = vec![v[TRANSISTORS].round() as i32, v[PICKUP].round() as i32, (v[GUITAR_VOL] * 10.0).round() as i32];
        if k != self.key {
            self.key = k;
            self.design();
        }
        let vol = v[VOLUME] as f64 / 10.0;
        self.out.set(vol * vol);
        self.inp.set(db_to_gain(v[LEVEL] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let v = self.v;
        let ge = v[TRANSISTORS] > 0.5;
        let fuzz = v[FUZZ] as f64 / 10.0;
        // the guitar's volume pot against the pedal's low input impedance: turning it down cuts
        // the drive far more than the level, which is why a Fuzz Face cleans up
        let gv = v[GUITAR_VOL] as f64 / 10.0;
        let into = gv.powf(2.2);
        let gain = (if ge { 55.0 } else { 120.0 }) * (0.08 + 0.92 * fuzz * fuzz) * into + 1.0;
        // the bias trimmer (Q2's collector load): noon is right, low starves it (gated, splatty), high is smoother
        let bias = (v[BIAS] as f64 / 10.0 - 0.5) * if ge { 0.7 } else { 0.3 };
        let sag_c = coeff(self.fs * OS as f64, 40.0);
        let last = self.ch.len() - 1;
        let mut peak = 0.0f64;
        for k in 0..n {
            let o = self.out.next();
            let gi = self.inp.next();
            for (ci, chn) in chans.iter_mut().enumerate() {
                let c = &mut self.ch[ci.min(last)];
                let x = chn[k] as f64 * gi;
                let st = &mut c.st;
                let y = c.os.process(x, |u| {
                    let s = st.inp.process(st.pick.process(u) * (0.25 + 0.75 * gv));
                    let a = s.abs();
                    st.sag = a + sag_c * (st.sag - a);
                    let b = bias + if ge { 0.35 * st.sag.min(1.0) } else { 0.0 };
                    let d = s * gain;
                    let top = if ge { 0.8 } else { 1.0 };
                    let w = if d + b > 0.0 { ((d + b) / top).tanh() * top } else { ((d + b) * 1.6).tanh() / 1.6 } - (b.max(0.0) / top).tanh() * top;
                    let w = st.mid.process(w);
                    st.post.process(st.lp.process(w)) * 0.45
                });
                peak = peak.max(y.abs());
                chn[k] = (y * o) as f32;
            }
        }
        meters[0] = peak as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Time / Money Solo", values: &[(FUZZ, 9.0)] },
        Preset { name: "Full Fuzz", values: &[(VOLUME, 6.5), (FUZZ, 10.0)] },
        Preset { name: "Cleaned Up (guitar on 6)", values: &[(VOLUME, 7.5), (FUZZ, 10.0), (GUITAR_VOL, 6.0)] },
        Preset { name: "Echoes Violin (with Echorec)", values: &[(FUZZ, 7.0), (GUITAR_VOL, 7.5)] },
        Preset { name: "Low Fuzz Crunch", values: &[(VOLUME, 7.0), (FUZZ, 3.0)] },
        Preset { name: "Germanium (pre-1970)", values: &[(VOLUME, 6.5), (FUZZ, 9.0), (TRANSISTORS, 1.0)] },
        Preset { name: "Starved Bias Splat", values: &[(VOLUME, 7.0), (FUZZ, 10.0), (BIAS, 1.5)] },
    ]
}

/*
('switch', 443.0, 124.5, 39, 14)
('knob', 199.5, 159.5, 41.5)
('knob', 439.5, 159.5, 41.5)
('switch', 436.0, 194.5, 39, 14)
('divider', 250)
('knob', 79.5, 319.5, 23.2)
('selector?', 199.5, 319.5, 36, 36)
('knob', 319.5, 319.5, 22.0)
('knob', 559.5, 319.5, 19.5)

crate: ['floyd/pf_fuzzface', 'library/panic_abort']
vintage/src/ui.rs
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
AFTER THE DALLAS ARBITER FUZZ FACE2 x BC108 SILICON  
  1970VOLUMEOutput level (500k).FUZZThe 1k pot on Q2's emitter: how much of it the 20 uF cap bypasses. Up = more gain, harder clipping.OUTPUTGUITAR VOLUMEYour guitar's volume knob. A Fuzz Face loads the pickup heavily, so backing it off cleans the fuzz up rather than just turning it down.SINGLEHUMB.BUFFER
PICKUPWhat the pedal sees at its input: a single-coil or humbucking pickup (the inductance darkens the tone into the low input impedance), or a buffer (a wah, or a buffered pedal before it).BIASQ2's collector load (the internal trimmer). Noon is about right; low gets gated and splatty, high gets smoother.TRANSISTORSBC108AC128BC108 silicon (Gilmour's from 1970: brighter, more gain) or the earlier AC128 germanium (warmer, smoother).GUITAR LEVELHow hot your DI is: 0 dB treats full scale as 1 V from the pickups.FUZZ FACEfloyd/pf_fuzzface/src/lib.rs
Time / Money Solo
Full Fuzz
Cleaned Up (guitar on 6)
Echoes Violin (with Echorec)
Low Fuzz Crunch
Germanium (pre-1970)
Starved Bias Splat

*/
fn panel() -> Panel {
    Panel {
        title: "FUZZ FACE",
        subtitle: ("AFTER THE DALLAS ARBITER FUZZ FACE", "2 x BC108 SILICON  ·  1970"),
        accent: ui::TANGERINE,
        size: (W, H),
        items: vec![
            Item::Knob { p: VOLUME, x: 200.0, y: 160.0, r: 42.0, label: "VOLUME", tip: "Output level (500k).", active: None },
            Item::Knob { p: FUZZ, x: 440.0, y: 160.0, r: 42.0, label: "FUZZ", tip: "The 1k pot on Q2's emitter: how much of it the 20 uF cap bypasses. Up = more gain, harder clipping.", active: None },
            Item::Lamp { m: 0, x: 320.0, y: 160.0, label: "OUTPUT", lo: 0.05, hi: 0.6 },
            Item::Divider { y: 250.0 },
            Item::Knob { p: GUITAR_VOL, x: 80.0, y: 320.0, r: 24.0, label: "GUITAR VOLUME", tip: "Your guitar's volume knob. A Fuzz Face loads the pickup heavily, so backing it off cleans the fuzz up rather than just turning it down.", active: None },
            Item::Selector { p: PICKUP, x: 200.0, y: 320.0, label: "PICKUP", positions: &["SINGLE", "HUMB.", "BUFFER"], tip: "What the pedal sees at its input: a single-coil or humbucking pickup (the inductance darkens the tone into the low input impedance), or a buffer (a wah, or a buffered pedal before it)." },
            Item::Knob { p: BIAS, x: 320.0, y: 320.0, r: 22.0, label: "BIAS", tip: "Q2's collector load (the internal trimmer). Noon is about right; low gets gated and splatty, high gets smoother.", active: None },
            Item::Text { x: 440.0, y: 280.0, text: "TRANSISTORS", size: 13.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: TRANSISTORS, x: 440.0, y: 320.0, labels: ["BC108", "AC128"], tip: "BC108 silicon (Gilmour's from 1970: brighter, more gain) or the earlier AC128 germanium (warmer, smoother)." },
            Item::Knob { p: LEVEL, x: 560.0, y: 320.0, r: 20.0, label: "GUITAR LEVEL", tip: "How hot your DI is: 0 dB treats full scale as 1 V from the pickups.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Fuzz Face", class_id: *b"PFFuzzFacePQD001", vendor: "PEQUOD Pink Floyd",
    categories: [Fx, Distortion]
);
