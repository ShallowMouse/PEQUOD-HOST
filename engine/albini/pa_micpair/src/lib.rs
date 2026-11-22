//! Mic Pair: two microphones on one source, the way Steve Albini usually miked guitar cabs, bass cabs, drums and vocals:
//! ribbons, dynamics, condensers and a boundary mic, each with its pattern, proximity bass, on-axis curve and off-axis
//! treble loss; distance sets each mic's arrival time, so blending them combs unless time-aligned.

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
pub const H: u32 = 470;

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum MicA {
    #[name = "Coles 4038 (ribbon)"]
    Coles4038Ribbon,
    #[name = "Royer R-121 (ribbon)"]
    RoyerR121Ribbon,
    #[name = "RCA 44 (ribbon)"]
    Rca44Ribbon,
    #[name = "Beyer M160 (ribbon)"]
    BeyerM160Ribbon,
    #[name = "Beyer M380 (dynamic)"]
    BeyerM380Dynamic,
    #[name = "Beyer M201 (dynamic)"]
    BeyerM201Dynamic,
    #[name = "EV RE-20 (dynamic)"]
    EvRe20Dynamic,
    #[name = "Shure SM98 (condenser)"]
    ShureSm98Condenser,
    #[name = "AKG 451 (condenser)"]
    Akg451Condenser,
    #[name = "AKG 414 (condenser)"]
    Akg414Condenser,
    #[name = "FET 47 (condenser)"]
    Fet47Condenser,
    #[name = "Josephson e22S (condenser)"]
    JosephsonE22SCondenser,
    #[name = "PZM (boundary)"]
    PzmBoundary,
}

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum MicB {
    #[name = "Coles 4038 (ribbon)"]
    Coles4038Ribbon,
    #[name = "Royer R-121 (ribbon)"]
    RoyerR121Ribbon,
    #[name = "RCA 44 (ribbon)"]
    Rca44Ribbon,
    #[name = "Beyer M160 (ribbon)"]
    BeyerM160Ribbon,
    #[name = "Beyer M380 (dynamic)"]
    BeyerM380Dynamic,
    #[name = "Beyer M201 (dynamic)"]
    BeyerM201Dynamic,
    #[name = "EV RE-20 (dynamic)"]
    EvRe20Dynamic,
    #[name = "Shure SM98 (condenser)"]
    ShureSm98Condenser,
    #[name = "AKG 451 (condenser)"]
    Akg451Condenser,
    #[name = "AKG 414 (condenser)"]
    Akg414Condenser,
    #[name = "FET 47 (condenser)"]
    Fet47Condenser,
    #[name = "Josephson e22S (condenser)"]
    JosephsonE22SCondenser,
    #[name = "PZM (boundary)"]
    PzmBoundary,
}

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum OutMode {
    #[name = "Mono blend"]
    MonoBlend,
    #[name = "Stereo (A left, B right)"]
    StereoALeftBRight,
}

/// On/off switches are stored as 0/1 (as the released plugin saved them).
#[derive(Enum, PartialEq, Clone, Copy)]
pub enum OffOn {
    #[name = "Off"]
    Off,
    #[name = "On"]
    On,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "mic_a"]
    pub mic_a: EnumParam<MicA>,
    #[id = "dist_a"]
    pub dist_a: FloatParam,
    #[id = "angle_a"]
    pub angle_a: FloatParam,
    #[id = "mic_b"]
    pub mic_b: EnumParam<MicB>,
    #[id = "dist_b"]
    pub dist_b: FloatParam,
    #[id = "angle_b"]
    pub angle_b: FloatParam,
    #[id = "blend"]
    pub blend: FloatParam,
    #[id = "out_mode"]
    pub out_mode: EnumParam<OutMode>,
    #[id = "polarity"]
    pub polarity: EnumParam<OffOn>,
    #[id = "align"]
    pub align: EnumParam<OffOn>,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            mic_a: EnumParam::new("Mic A", MicA::Akg451Condenser),
            dist_a: FloatParam::new("Mic A Distance", 25.0, FloatRange::Skewed { min: 1.0, max: 300.0, factor: FloatRange::skew_factor(-2.0) }).with_step_size(0.5).with_unit(" cm"),  // texts '1.0' .. '19.5' .. '300.0'
            angle_a: FloatParam::new("Mic A Angle", 0.0, FloatRange::Linear { min: 0.0, max: 90.0 }).with_step_size(1.0).with_unit("°"),  // texts '0' .. '45' .. '90'
            mic_b: EnumParam::new("Mic B", MicB::Coles4038Ribbon),
            dist_b: FloatParam::new("Mic B Distance", 20.0, FloatRange::Skewed { min: 1.0, max: 300.0, factor: FloatRange::skew_factor(-2.0) }).with_step_size(0.5).with_unit(" cm"),  // texts '1.0' .. '19.5' .. '300.0'
            angle_b: FloatParam::new("Mic B Angle", 0.0, FloatRange::Linear { min: 0.0, max: 90.0 }).with_step_size(1.0).with_unit("°"),  // texts '0' .. '45' .. '90'
            blend: pct("Blend A-B", 0.5),
            out_mode: EnumParam::new("Output", OutMode::MonoBlend),
            polarity: EnumParam::new("Mic B Polarity Flip", OffOn::Off),
            align: EnumParam::new("Time Align", OffOn::Off),
            output: db("Output", 0.0, -18.0, 18.0),
        }
    }
}

pub const MIC_A: usize = 0;
pub const DIST_A: usize = 1;
pub const ANGLE_A: usize = 2;
pub const MIC_B: usize = 3;
pub const DIST_B: usize = 4;
pub const ANGLE_B: usize = 5;
pub const BLEND: usize = 6;
pub const OUT_MODE: usize = 7;
pub const POLARITY: usize = 8;
pub const ALIGN: usize = 9;
pub const OUTPUT: usize = 10;
pub const NPARAMS: usize = 11;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.mic_a.as_ptr(),
            self.dist_a.as_ptr(),
            self.angle_a.as_ptr(),
            self.mic_b.as_ptr(),
            self.dist_b.as_ptr(),
            self.angle_b.as_ptr(),
            self.blend.as_ptr(),
            self.out_mode.as_ptr(),
            self.polarity.as_ptr(),
            self.align.as_ptr(),
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

use std::f64::consts::PI;

const NAMES: &[&str] = &["Coles 4038", "Royer R-121", "RCA 44", "Beyer M160", "Beyer M380", "Beyer M201", "EV RE-20", "Shure SM98", "AKG 451", "AKG 414", "FET 47", "Josephson e22S", "PZM boundary"];

pub struct MicSpec {
    /// gradient share: 0 omni / boundary, 0.5 cardioid, 0.75 hyper, 1 figure-8
    pub k: f64,
    /// how much of the pattern's proximity bass the mic shows (the RE-20's Variable-D has little)
    pub prox: f64,
    pub hp: (f64, f64),
    pub peaks: [(f64, f64, f64); 4],
    pub lp: f64,
    pub diaphragm: f64,
}

/// Approximations of the published on-axis curves.
pub const MICS: [MicSpec; 13] = [
    MicSpec { k: 1.0, prox: 1.0, hp: (30.0, 0.6), peaks: [(250.0, 0.7, 0.5), (2000.0, 1.0, 0.0), (9000.0, 0.7, -1.5), (12000.0, 1.0, 0.0)], lp: 13000.0, diaphragm: 0.035 },
    MicSpec { k: 1.0, prox: 1.0, hp: (30.0, 0.6), peaks: [(100.0, 0.8, 1.0), (1000.0, 1.0, 0.0), (4000.0, 1.0, -1.0), (10000.0, 1.0, -2.0)], lp: 12000.0, diaphragm: 0.03 },
    MicSpec { k: 1.0, prox: 1.0, hp: (30.0, 0.6), peaks: [(150.0, 0.8, 1.5), (1000.0, 1.0, 0.0), (5000.0, 1.0, -2.0), (8000.0, 1.0, -3.0)], lp: 9000.0, diaphragm: 0.04 },
    MicSpec { k: 0.75, prox: 1.0, hp: (40.0, 0.7), peaks: [(200.0, 1.0, 0.0), (2500.0, 1.0, 1.0), (6000.0, 1.0, -1.0), (10000.0, 1.0, 0.0)], lp: 14000.0, diaphragm: 0.025 },
    MicSpec { k: 1.0, prox: 0.8, hp: (40.0, 0.7), peaks: [(80.0, 1.0, 2.0), (1000.0, 1.0, 0.0), (5000.0, 1.0, 2.0), (8000.0, 1.0, 0.0)], lp: 14000.0, diaphragm: 0.03 },
    MicSpec { k: 0.75, prox: 0.8, hp: (60.0, 0.7), peaks: [(250.0, 1.0, -1.0), (3000.0, 1.0, 1.5), (7000.0, 1.0, 2.0), (10000.0, 1.0, 0.0)], lp: 16000.0, diaphragm: 0.02 },
    MicSpec { k: 0.5, prox: 0.25, hp: (45.0, 0.7), peaks: [(200.0, 1.0, 0.0), (2000.0, 1.0, 0.0), (4500.0, 1.0, 1.0), (9000.0, 1.0, -1.0)], lp: 15000.0, diaphragm: 0.03 },
    MicSpec { k: 0.5, prox: 1.0, hp: (60.0, 0.7), peaks: [(500.0, 1.0, 0.0), (3000.0, 1.0, 1.0), (8000.0, 1.0, 3.0), (12000.0, 1.0, 0.0)], lp: 18000.0, diaphragm: 0.01 },
    MicSpec { k: 0.5, prox: 1.0, hp: (50.0, 0.7), peaks: [(400.0, 1.0, -1.0), (3000.0, 1.0, 1.0), (9000.0, 1.0, 3.0), (12000.0, 1.0, 1.0)], lp: 18000.0, diaphragm: 0.012 },
    MicSpec { k: 0.5, prox: 1.0, hp: (30.0, 0.7), peaks: [(200.0, 1.0, 0.5), (3000.0, 1.0, 0.0), (8000.0, 1.0, 2.0), (12000.0, 1.0, 2.0)], lp: 18000.0, diaphragm: 0.025 },
    MicSpec { k: 0.5, prox: 1.0, hp: (30.0, 0.7), peaks: [(100.0, 0.8, 1.5), (1000.0, 1.0, -0.5), (4000.0, 1.0, 2.0), (8000.0, 1.0, 1.0)], lp: 15000.0, diaphragm: 0.034 },
    MicSpec { k: 0.5, prox: 1.0, hp: (40.0, 0.7), peaks: [(150.0, 1.0, 0.5), (2000.0, 1.0, 0.0), (6000.0, 1.0, 1.0), (11000.0, 1.0, 1.0)], lp: 18000.0, diaphragm: 0.02 },
    MicSpec { k: 0.0, prox: 0.0, hp: (20.0, 0.7), peaks: [(100.0, 1.0, 0.0), (1000.0, 1.0, 0.0), (8000.0, 1.0, 2.0), (12000.0, 1.0, 0.0)], lp: 18000.0, diaphragm: 0.01 },
];

#[derive(Clone, Copy, PartialEq)]
struct Shape {
    mic: usize,
    distance: f64,
    angle: f64,
}

#[derive(Clone, Copy)]
struct Path {
    prox: Biquad,
    peaks: [Biquad; 4],
    lp: Biquad,
    off: Biquad,
}

impl Path {
    fn new() -> Self {
        Self { prox: Biquad::identity(), peaks: [Biquad::identity(); 4], lp: Biquad::identity(), off: Biquad::identity() }
    }
    fn design(&mut self, fs: f64, sh: Shape) {
        let m = &MICS[sh.mic];
        let k = m.k;
        let cos = sh.angle.to_radians().cos();
        // pressure + gradient with proximity (c / r), times the membrane high-pass; level held at 1 kHz
        let w0 = 343.0 / (sh.distance * 0.01).max(0.005) * m.prox;
        let pat = ((1.0 - k) + k * cos).max(0.03);
        let wh = 2.0 * PI * m.hp.0;
        let num = [pat, k * cos * w0, 0.0];
        let den = [1.0, wh / m.hp.1, wh * wh];
        let w = 2.0 * PI * 1000.0;
        let mag = |n: [f64; 3], d: [f64; 3]| {
            let (nr, ni) = (n[2] - n[0] * w * w, n[1] * w);
            let (dr, di) = (d[2] - d[0] * w * w, d[1] * w);
            ((nr * nr + ni * ni) / (dr * dr + di * di)).sqrt()
        };
        let norm = 1.0 / mag(num, den).max(1e-6) * pat;
        self.prox.set_analog(fs, [num[0] * norm, num[1] * norm, num[2] * norm], den);
        for (b, &(f, q, g)) in self.peaks.iter_mut().zip(&m.peaks) {
            b.set_peak(fs, f, q, g);
        }
        self.lp.set_lowpass(fs, m.lp.min(fs * 0.45), 0.7);
        let f_beam = (343.0 / (PI * m.diaphragm)).min(fs * 0.4);
        self.off.set_high_shelf(fs, f_beam, 0.7, -12.0 * (1.0 - cos));
    }
    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let mut y = self.prox.process(x);
        for b in self.peaks.iter_mut() {
            y = b.process(y);
        }
        self.off.process(self.lp.process(y))
    }
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    line: [DelayLine; 2],
    paths: [[Path; 2]; 2],
    shapes: [Option<Shape>; 2],
    out: Smooth,
    blend: Smooth,
}

impl Dsp {
    fn shape(&self, m: usize, d: usize, a: usize) -> Shape {
        Shape { mic: (self.v[m].round() as usize).min(12), distance: self.v[d] as f64, angle: self.v[a] as f64 }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 2;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let n = (fs * 0.012) as usize;
        let mut d = Self { fs, v, line: [DelayLine::new(n), DelayLine::new(n)], paths: [[Path::new(); 2]; 2], shapes: [None; 2], out: Smooth::new(fs, 20.0, 1.0), blend: Smooth::new(fs, 20.0, 0.5) };
        let v2 = v;
        d.set_params(&v2);
        d
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.line[c].reset();
        }
        self.shapes = [None; 2];
        let v = self.v;
        self.set_params(&v);
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let sh = [self.shape(MIC_A, DIST_A, ANGLE_A), self.shape(MIC_B, DIST_B, ANGLE_B)];
        for i in 0..2 {
            if self.shapes[i] != Some(sh[i]) {
                for c in 0..2 {
                    self.paths[c][i].design(self.fs, sh[i]);
                }
                self.shapes[i] = Some(sh[i]);
            }
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
        self.blend.set(v[BLEND] as f64);
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(2);
        let sps = self.fs / 343.0 * 0.01;
        let (da, db) = (self.v[DIST_A] as f64 * sps, self.v[DIST_B] as f64 * sps);
        let (ta, tb) = if self.v[ALIGN] > 0.5 { let m = da.max(db); (m, m) } else { (da, db) };
        let flip = if self.v[POLARITY] > 0.5 { -1.0 } else { 1.0 };
        let stereo = self.v[OUT_MODE] > 0.5;
        for k in 0..n {
            let o = self.out.next();
            let b = self.blend.next();
            let mut outs = [0.0f64; 2];
            for c in 0..nch {
                self.line[c].write(chans[c][k] as f64);
                let a = self.paths[c][0].process(self.line[c].read(ta.max(1.0)));
                let bb = flip * self.paths[c][1].process(self.line[c].read(tb.max(1.0)));
                if stereo {
                    outs[c] = if c == 0 || nch == 1 { a * (1.0 - b).min(0.5) * 2.0 } else { bb * b.min(0.5) * 2.0 };
                    if nch == 1 {
                        outs[c] = a * (1.0 - b) + bb * b;
                    }
                } else {
                    outs[c] = a * (1.0 - b) + bb * b;
                }
            }
            for c in 0..nch {
                chans[c][k] = (outs[c] * o) as f32;
            }
        }
        let code = |m: usize| match MICS[(self.v[m].round() as usize).min(12)].k {
            k if k < 0.25 => 1.0,
            k if k > 0.9 => 3.0,
            _ => 2.0,
        };
        meters[0] = code(MIC_A);
        meters[1] = code(MIC_B);
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Guitar Cab: 451 + 4038", values: &[] },
        Preset { name: "Guitar Cab: e22S + R-121", values: &[(MIC_A, 11.0), (DIST_A, 30.0), (MIC_B, 1.0), (DIST_B, 18.0)] },  // OCR 'Guitar Cab: e225 + R121'
        Preset { name: "Guitar Cab, Stereo Pair", values: &[(MIC_A, 9.0), (DIST_A, 30.0), (DIST_B, 25.0), (OUT_MODE, 1.0)] },
        Preset { name: "Bass Cab: M380 + FET 47", values: &[(MIC_A, 10.0), (DIST_A, 30.0), (MIC_B, 4.0), (DIST_B, 8.0), (BLEND, 0.6), (ALIGN, 1.0)] },
        Preset { name: "Snare: M201 + SM98", values: &[(MIC_A, 5.0), (DIST_A, 6.0), (ANGLE_A, 30.0), (MIC_B, 7.0), (DIST_B, 6.0), (ANGLE_B, 30.0), (BLEND, 0.4)] },
        Preset { name: "Kick: SM98 beater + FET 47 front (flip)", values: &[(MIC_A, 7.0), (DIST_A, 5.0), (MIC_B, 10.0), (DIST_B, 40.0), (POLARITY, 1.0), (ALIGN, 1.0)] },
        Preset { name: "Overheads: 4038 far", values: &[(MIC_A, 0.0), (DIST_A, 150.0), (DIST_B, 160.0), (OUT_MODE, 1.0)] },
        Preset { name: "Vocal: RE-20 + FET 47", values: &[(MIC_A, 6.0), (DIST_A, 8.0), (MIC_B, 10.0), (DIST_B, 25.0), (ALIGN, 1.0)] },
        Preset { name: "Room Wall PZM (Surfer Rosa)", values: &[(MIC_A, 12.0), (DIST_A, 300.0), (MIC_B, 12.0), (DIST_B, 300.0), (BLEND, 0.0)] },
    ]
}

/*
('knob', 320.5, 267.5, 19.5)
('divider', 306)
('knob', 39.5, 351.5, 17.1)
('knob', 99.5, 351.5, 17.1)
('knob', 159.5, 351.5, 17.1)
('knob', 219.5, 351.5, 17.1)
('knob', 605.5, 351.5, 17.1)

crate: ['albini/pa_micpair', 'library/panic_abort']
 DYNAMIC 
 CONDENSER 
 BOUNDARYMIC AColes 4038Royer R-121RCA 44Beyer M160Beyer M380Beyer M201EV RE-20Shure SM98AKG 451AKG 414FET 47Josephson e22SPZM boundary
Mic A: usually the bright, detailed one.MIC BMic B: usually the dark, fat one (a ribbon).ABBLEND A - BMono: the mix of A and B. Stereo: the balance between them.A DISTMic A to source. Closer = more proximity bass (pattern mics) and an earlier arrival.A ANGLEMic A off-axis angle (from the cone centre toward the edge).B DISTMic B to source.B ANGLEMic B off-axis angle.OUTPUTMONOST.Mono blend, or A on the left and B on the right.B POLARITYNORMFLIPFlip mic B's polarity (e.g. the two sides of a kick drum).ALIGNOFFONDelay the nearer mic so both arrive together (no comb filtering between them).Output level.MIC PAIRGuitar Cab: 451 + 4038
Guitar Cab: e22S + R-121
Guitar Cab, Stereo Pair
Bass Cab: M380 + FET 47
Snare: M201 + SM98
Kick: SM98 beater + FET 47 front (flip)
Overheads: 4038 far
Vocal: RE-20 + FET 47
Room Wall PZM (Surfer Rosa)$min > max, or either was NaN. min = 

*/
fn panel() -> Panel {
    Panel {
        title: "MIC PAIR",
        subtitle: ("TWO MICS ON ONE SOURCE", "RIBBON · DYNAMIC · CONDENSER · BOUNDARY"),
        accent: ui::SLATE,
        size: (W, H),
        items: vec![
            Item::Text { x: 85.0, y: 76.0, text: "MIC A", size: 12.0, align: Align2::CENTER_CENTER },
            Item::RadioList { p: MIC_A, x: 30.0, y: 88.0, w: 150.0, row_h: 15.5, names: NAMES, tip: "Mic A: usually the bright, detailed one." },
            Item::Text { x: 535.0, y: 76.0, text: "MIC B", size: 12.0, align: Align2::CENTER_CENTER },
            Item::RadioList { p: MIC_B, x: 462.0, y: 88.0, w: 150.0, row_h: 15.5, names: NAMES, tip: "Mic B: usually the dark, fat one (a ribbon)." },
            Item::Polar { x: 262.0, y: 160.0, r: 46.0, pattern: 0, angle: ANGLE_A },
            Item::Text { x: 262.0, y: 228.0, text: "A", size: 13.0, align: Align2::CENTER_CENTER },
            Item::Polar { x: 382.0, y: 160.0, r: 46.0, pattern: 1, angle: ANGLE_B },
            Item::Text { x: 382.0, y: 228.0, text: "B", size: 13.0, align: Align2::CENTER_CENTER },
            Item::Knob { p: BLEND, x: 320.0, y: 268.0, r: 20.0, label: "BLEND A - B", tip: "Mono: the mix of A and B. Stereo: the balance between them.", active: None },
            Item::Divider { y: 306.0 },
            Item::Knob { p: DIST_A, x: 40.0, y: 352.0, r: 18.0, label: "A DIST", tip: "Mic A to source. Closer = more proximity bass (pattern mics) and an earlier arrival.", active: None },
            Item::Knob { p: ANGLE_A, x: 100.0, y: 352.0, r: 18.0, label: "A ANGLE", tip: "Mic A off-axis angle (from the cone centre toward the edge).", active: None },
            Item::Knob { p: DIST_B, x: 160.0, y: 352.0, r: 18.0, label: "B DIST", tip: "Mic B to source.", active: None },
            Item::Knob { p: ANGLE_B, x: 220.0, y: 352.0, r: 18.0, label: "B ANGLE", tip: "Mic B off-axis angle.", active: None },
            Item::Text { x: 312.0, y: 330.0, text: "OUTPUT", size: 12.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: OUT_MODE, x: 312.0, y: 362.0, labels: ["MONO", "ST."], tip: "Mono blend, or A on the left and B on the right." },
            Item::Text { x: 422.0, y: 330.0, text: "B POLARITY", size: 12.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: POLARITY, x: 422.0, y: 362.0, labels: ["NORM", "FLIP"], tip: "Flip mic B's polarity (e.g. the two sides of a kick drum)." },
            Item::Text { x: 530.0, y: 330.0, text: "ALIGN", size: 12.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: ALIGN, x: 530.0, y: 362.0, labels: ["OFF", "ON"], tip: "Delay the nearer mic so both arrive together (no comb filtering between them)." },
            Item::Knob { p: OUTPUT, x: 606.0, y: 352.0, r: 18.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Mic Pair", class_id: *b"PAMicPairPQD0001", vendor: "PEQUOD Albini",
    categories: [Fx, Filter]
);
