//! Triple Bass Rig: Chris Wolstenholme's Absolution-era bass rig: a Boss OC-2 octave, then the signal split three ways
//! into three Marshall DBS 7400 heads, one clean, one through a Sovtek "Black Russian" Big Muff (simulated as a
//! circuit) and one through his Human Gear Animato distortion, blended into Marshall DBS cabinets. The octave divider
//! and the Animato are modelled from their behaviour; the Big Muff from its schematic.

#![allow(clippy::excessive_precision)]
use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
#[allow(unused_imports)]
use vintage::dsp::*;
use vintage::nih_plug_egui::EguiState;
#[allow(unused_imports)]
use vintage::params::*;
#[allow(unused_imports)]
use vintage::ui::{self, ActiveWhen, Item, MeterKind, Panel, Preset};
#[allow(unused_imports)]
use vintage::{Midi, VintageDsp, VintageParams};
use vintage::amp::cab::{self, Cabinet};

pub const W: u32 = 640;
pub const H: u32 = 440;

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum Cab {
    #[name = "Built-in (miked)"]
    Mic,
    #[name = "DI (no cab)"]
    Di,
    #[name = "Speaker (no mic)"]
    Speaker,
}

fn k10(name: &str, def: f32) -> FloatParam {
    FloatParam::new(name, def, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1).with_value_to_string(formatters::v2s_f32_rounded(1))
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "octave"]
    pub octave: FloatParam,
    #[id = "clean"]
    pub clean: FloatParam,
    #[id = "muff"]
    pub muff: FloatParam,
    #[id = "dist"]
    pub dist: FloatParam,
    #[id = "sustain"]
    pub sustain: FloatParam,
    #[id = "mufftone"]
    pub mufftone: FloatParam,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "bass"]
    pub bass: FloatParam,
    #[id = "treble"]
    pub treble: FloatParam,
    #[id = "cab"]
    pub cab: EnumParam<Cab>,
    #[id = "level"]
    pub level: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            octave: k10("Octave", 0.0),
            clean: k10("Clean Head", 7.0),
            muff: k10("Muff Head", 5.0),
            dist: k10("Distortion Head", 0.0),
            sustain: k10("Sustain", 7.0),
            mufftone: k10("Muff Tone", 4.5),
            drive: k10("Drive", 6.0),
            bass: k10("Bass", 6.0),
            treble: k10("Treble", 5.0),
            cab: EnumParam::new("Cabinet", Cab::Mic),
            level: db("Bass Level", 0.0, -12.0, 12.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const OCTAVE: usize = 0;
pub const CLEAN: usize = 1;
pub const MUFF: usize = 2;
pub const DIST: usize = 3;
pub const SUSTAIN: usize = 4;
pub const MUFFTONE: usize = 5;
pub const DRIVE: usize = 6;
pub const BASS: usize = 7;
pub const TREBLE: usize = 8;
pub const CAB: usize = 9;
pub const LEVEL: usize = 10;
pub const OUTPUT: usize = 11;
pub const NPARAMS: usize = 12;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.octave.as_ptr(),
            self.clean.as_ptr(),
            self.muff.as_ptr(),
            self.dist.as_ptr(),
            self.sustain.as_ptr(),
            self.mufftone.as_ptr(),
            self.drive.as_ptr(),
            self.bass.as_ptr(),
            self.treble.as_ptr(),
            self.cab.as_ptr(),
            self.level.as_ptr(),
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

use vintage::amp::net::Taper;
use vintage::circuit::{self, Circuit, Solver};

const C_SUS: usize = 0;
const C_TONE: usize = 1;
const C_VOL: usize = 2;

/// The Sovtek Big Muff ("Black Russian"): the four-transistor Big Muff with the Sovtek tone stack
/// (20k / 10n low-pass, 10n / 22k high-pass), volume fixed at noon (the head's gain sets the level).
fn russian_muff(fs: f64) -> Solver {
    let q = circuit::BC108;
    let d = circuit::D1N914;
    let mut c = Circuit::new()
        .input(1, 1e3)
        .dc(2, 9.0, 5.0)
        .c(1, 3, 1e-6)
        .r(3, 4, 39e3)
        .r(5, 4, 470e3)
        .c(5, 4, 560e-12)
        .r(4, 0, 100e3)
        .r(2, 5, 15e3)
        .r(6, 0, 100.0)
        .npn(5, 4, 6, q)
        .c(5, 7, 1e-6)
        .pot(0, 8, 7, 100e3, Taper::Linear, C_SUS)
        .r(8, 9, 1e3)
        .c(9, 10, 1e-6)
        .r(10, 11, 10e3);
    for (b, col, e, dm) in [(11usize, 12usize, 13usize, 14usize), (16, 17, 18, 19)] {
        c = c.r(col, b, 470e3).c(col, b, 560e-12).r(b, 0, 100e3).r(2, col, 15e3).r(e, 0, 150.0).npn(col, b, e, q).c(col, dm, 1e-6).diode(dm, b, d).diode(b, dm, d);
        if col == 12 {
            c = c.c(col, 15, 1e-6).r(15, 16, 10e3);
        }
    }
    c.c(17, 22, 1e-6)
        .r(22, 23, 20e3)
        .c(23, 0, 10e-9)
        .c(22, 24, 10e-9)
        .r(24, 0, 22e3)
        .pot(23, 25, 24, 100e3, Taper::Linear, C_TONE)
        .c(25, 26, 1e-6)
        .r(2, 26, 470e3)
        .r(26, 0, 100e3)
        .r(2, 27, 15e3)
        .r(28, 0, 3.3e3)
        .npn(27, 26, 28, q)
        .c(27, 29, 1e-6)
        .pot(0, 30, 29, 100e3, Taper::Audio, C_VOL)
        .r(30, 0, 1e6)
        .output(30)
        .build(fs)
}

/// One DBS 7400 channel's voicing: its active bass and treble and a solid-state preamp that
/// hardens up when pushed.
#[derive(Clone, Copy, Default)]
struct Head {
    eq: [Biquad; 3],
}

impl Head {
    fn design(&mut self, fs: f64, bass: f64, treble: f64) {
        self.eq[0].set_low_shelf(fs, 100.0, 0.7, (bass - 5.0) * 2.4);
        self.eq[1].set_high_shelf(fs, 2500.0, 0.7, (treble - 5.0) * 2.4);
        self.eq[2].set_highpass(fs, 30.0, 0.7);
    }
    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let y = cascade(&mut self.eq, x);
        // transistor preamp: clean until near its rails, then a quick, hard-ish knee
        y / (1.0 + (y * 0.6).abs().powf(3.0)).powf(1.0 / 3.0)
    }
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    muff: Solver,
    os: Oversampler,
    dist_hp: Biquad,
    dist_lp: Biquad,
    oct_lp: Biquad,
    oct_env: f64,
    flip: f64,
    last: f64,
    heads: [Head; 3],
    cab: Cabinet,
    spk: Cabinet,
    dcb: DcBlock,
    key: Vec<i32>,
    inp: Smooth,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let (b, t) = (self.v[BASS] as f64, self.v[TREBLE] as f64);
        for h in self.heads.iter_mut() {
            h.design(fs, b, t);
        }
        self.muff.set_ctl(C_SUS, self.v[SUSTAIN] as f64 / 10.0);
        self.muff.set_ctl(C_TONE, self.v[MUFFTONE] as f64 / 10.0);
        self.muff.set_ctl(C_VOL, 0.5);
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut dist_hp = Biquad::identity();
        dist_hp.set_highpass(fs, 120.0, 0.7);
        let mut dist_lp = Biquad::identity();
        dist_lp.set_lowpass(fs, 3500.0, 0.7);
        let mut oct_lp = Biquad::identity();
        oct_lp.set_lowpass(fs, 250.0, 0.7);
        let mut d = Self {
            fs,
            v,
            muff: russian_muff(fs * OS as f64),
            os: Oversampler::new(),
            dist_hp,
            dist_lp,
            oct_lp,
            oct_env: 0.0,
            flip: 1.0,
            last: 0.0,
            heads: [Head::default(); 3],
            cab: Cabinet::new(fs, cab::MARSHALL_DBS_4X10),
            spk: Cabinet::new(fs, cab::MARSHALL_DBS_4X10),
            dcb: DcBlock::new(fs),
            key: vec![],
            inp: Smooth::new(fs, 20.0, 1.0),
            out: Smooth::new(fs, 20.0, 1.0),
        };
        d.design();
        d.muff.reset();
        d
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn reset(&mut self) {
        self.design();
        self.muff.reset();
        self.os.reset();
        self.cab.reset();
        self.spk.reset();
        self.dcb.reset();
        self.oct_env = 0.0;
        self.flip = 1.0;
        self.inp.snap(db_to_gain(self.v[LEVEL] as f64));
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k: Vec<i32> = [v[BASS], v[TREBLE], v[SUSTAIN], v[MUFFTONE]].iter().map(|x| (x * 100.0) as i32).collect();
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.inp.set(db_to_gain(v[LEVEL] as f64));
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().max(1) as f64;
        let oct = self.v[OCTAVE] as f64 / 10.0;
        let lv = |p: f32| {
            let g = p as f64 / 10.0;
            g * g
        };
        let (gc, gm, gd) = (lv(self.v[CLEAN]), lv(self.v[MUFF]), lv(self.v[DIST]));
        let drive = 1.0 + 60.0 * (self.v[DRIVE] as f64 / 10.0).powi(2);
        let mut peak = 0.0f64;
        for k in 0..n {
            let x = chans.iter().map(|c| c[k] as f64).sum::<f64>() / nch * self.inp.next();
            // OC-2: a flip-flop clocked by the low-passed bass's zero crossings, gating the signal's envelope
            let lp = self.oct_lp.process(x);
            if (lp >= 0.0) != (self.last >= 0.0) && lp >= 0.0 {
                self.flip = -self.flip;
            }
            self.last = lp;
            let a = lp.abs();
            self.oct_env = a + if a > self.oct_env { 0.9 } else { 0.9995 } * (self.oct_env - a);
            let x = x + oct * 1.4 * self.flip * self.oct_env;
            // the three heads
            let clean = self.heads[0].process(x * 1.5);
            let m = &mut self.muff;
            let muffed = if gm > 0.0 { self.os.process(x, |u| m.step(&[u])) * 4.0 } else { 0.0 };
            let muffed = self.heads[1].process(muffed);
            let dsig = if gd > 0.0 {
                // Animato: an op-amp gain stage into hard silicon clipping, then its tone
                let s = self.dist_hp.process(x) * drive;
                let s = s.clamp(-3.0, 3.0);
                let s = (s * 1.5).tanh() * 0.7;
                self.dist_lp.process(s)
            } else {
                0.0
            };
            let dsig = self.heads[2].process(dsig * 1.5);
            let y = self.dcb.process(gc * clean + gm * muffed + gd * dsig);
            peak = peak.max(y.abs());
            let y = match (self.v[CAB].round() as usize).min(2) {
                0 => self.cab.process(y),
                1 => y,
                _ => self.spk.process_speaker(y),
            };
            let y = (y * self.out.next()) as f32;
            for c in chans.iter_mut() {
                c[k] = y;
            }
        }
        meters[0] = peak as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Clean + Muff Blend", values: &[] },
        Preset { name: "Hysteria Grind", values: &[(CLEAN, 6.0), (MUFF, 6.5), (DIST, 4.0), (SUSTAIN, 8.0)] },
        Preset { name: "Hyper Music Fuzz", values: &[(CLEAN, 5.0), (MUFF, 7.5), (MUFFTONE, 6.0), (TREBLE, 6.0)] },
        Preset { name: "Octave Fuzz Riff", values: &[(OCTAVE, 6.0), (MUFF, 7.0), (CLEAN, 6.0)] },
        Preset { name: "Three Heads Full", values: &[(CLEAN, 6.0), (MUFF, 6.0), (DIST, 6.0), (DRIVE, 8.0)] },
        Preset { name: "Clean DBS", values: &[(MUFF, 0.0), (CLEAN, 8.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "TRIPLE BASS RIG",
        subtitle: ("OC-2 · CLEAN / BIG MUFF / ANIMATO · 3 x DBS 7400", "CHRIS WOLSTENHOLME'S BASS RIG, ABSOLUTION"),
        accent: ui::MUSTARD,
        size: (W, H),
        items: vec![
            Item::Knob { p: OCTAVE, x: 60.0, y: 140.0, r: 24.0, label: "OCTAVE", tip: "The Boss OC-2's octave-down voice, mixed in before the split.", active: None },
            Item::Knob { p: CLEAN, x: 160.0, y: 140.0, r: 28.0, label: "CLEAN HEAD", tip: "Level of the clean DBS 7400.", active: None },
            Item::Knob { p: MUFF, x: 270.0, y: 140.0, r: 28.0, label: "MUFF HEAD", tip: "Level of the head fed by the Sovtek Big Muff.", active: None },
            Item::Knob { p: DIST, x: 380.0, y: 140.0, r: 28.0, label: "DIST HEAD", tip: "Level of the head fed by the Animato distortion.", active: None },
            Item::Knob { p: BASS, x: 480.0, y: 140.0, r: 22.0, label: "BASS", tip: "The heads' bass control.", active: None },
            Item::Knob { p: TREBLE, x: 570.0, y: 140.0, r: 22.0, label: "TREBLE", tip: "The heads' treble control.", active: None },
            Item::Divider { y: 230.0 },
            Item::Knob { p: SUSTAIN, x: 80.0, y: 300.0, r: 22.0, label: "SUSTAIN", tip: "The Big Muff's sustain (gain).", active: None },
            Item::Knob { p: MUFFTONE, x: 180.0, y: 300.0, r: 22.0, label: "MUFF TONE", tip: "The Big Muff's tone: bassier left, brighter right.", active: None },
            Item::Knob { p: DRIVE, x: 280.0, y: 300.0, r: 22.0, label: "DRIVE", tip: "The Animato's gain.", active: None },
            Item::Knob { p: LEVEL, x: 380.0, y: 300.0, r: 20.0, label: "BASS LEVEL", tip: "How hot the bass DI is (0 dB treats full scale as 1 V).", active: None },
            Item::Selector { p: CAB, x: 470.0, y: 300.0, label: "CABINET", positions: &["MIC'D", "DI", "SPEAKER"], tip: "MIC'D: Marshall DBS cabinets, close-miked. DI: no cabinet. SPEAKER: the cabinets with no mic." },
            Item::Knob { p: OUTPUT, x: 570.0, y: 300.0, r: 20.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Triple Bass Rig", class_id: *b"MuseTripleBass01", vendor: "PEQUOD Muse",
    categories: [Fx, Distortion]
);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presets_run_clean() {
        for pr in presets() {
            let mut v = defaults();
            for (i, x) in pr.values {
                v[*i] = *x;
            }
            let mut d = Dsp::new(48000.0, 1);
            d.set_params(&v);
            let mut l: Vec<f32> = (0..48000).map(|k| (0.3 * (k as f64 * 2.0 * std::f64::consts::PI * 55.0 / 48000.0).sin()) as f32).collect();
            let mut m = [0.0f32];
            let t = std::time::Instant::now();
            d.process(&mut [&mut l[..]], &mut m);
            let rms = (l[24000..].iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / 24000.0).sqrt();
            eprintln!("{}: {:.1} dBFS, {:.0} ms", pr.name, 20.0 * rms.log10(), t.elapsed().as_secs_f64() * 1e3);
            assert!(l.iter().all(|x| x.is_finite() && x.abs() < 8.0), "{}", pr.name);
        }
    }
}
