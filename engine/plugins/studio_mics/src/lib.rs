//! Studio Mics — the microphones of 1960s Abbey Road.
//!
//! Each mic is its on-axis frequency response (approximated from the
//! makers' published curves), its polar pattern(s), and the physics that
//! move with placement:
//!
//! * Proximity effect. A pressure-gradient mic (figure-8, and the
//!   gradient half of a cardioid) responds to a spherical wave at distance
//!   r as 1 + c/(j w r): bass rises 6 dB/octave below c/(2 pi r). That's
//!   modelled exactly, so a ribbon an inch from a guitar amp booms the way
//!   it should and an omni doesn't.
//! * Off-axis. The pattern sets the level ((1-k) + k cos angle) and the
//!   gradient's share of the bass; the diaphragm size sets where the
//!   treble starts to fall away off-axis.
//! * Valve mics (U47, U48, U67, KM54, KM56) have a gentle valve stage.
//!
//! Put it after an amp with CABINET set to SPEAKER, or on any dry source,
//! then Studio Spaces for the room.

use nih_plug::prelude::*;
use std::f64::consts::PI;
use std::sync::{Arc, RwLock};
use vintage::dsp::*;
use vintage::nih_plug_egui::EguiState;
use vintage::params::*;
use vintage::ui::{self, Item, Panel, Preset};
use vintage::{VintageDsp, VintageParams};

const W: u32 = 640;
const H: u32 = 432;

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum MicModel {
    #[name = "U47 (valve)"]
    U47,
    #[name = "U48 (valve)"]
    U48,
    #[name = "U67 (valve)"]
    U67,
    #[name = "KM54 (valve)"]
    Km54,
    #[name = "KM56 (valve)"]
    Km56,
    #[name = "D19 (dynamic)"]
    D19,
    #[name = "D20 (dynamic)"]
    D20,
    #[name = "4038 (ribbon)"]
    R4038,
    #[name = "4033 (ribbon)"]
    R4033,
}

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum PatternSel {
    #[name = "Omni"]
    Omni,
    #[name = "Cardioid"]
    Cardioid,
    #[name = "Figure-8"]
    Figure8,
}

#[derive(Params)]
pub struct MicParams {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "mic"]
    mic: EnumParam<MicModel>,
    #[id = "pattern"]
    pattern: EnumParam<PatternSel>,
    #[id = "distance"]
    distance: FloatParam,
    #[id = "angle"]
    angle: FloatParam,
    #[id = "valve"]
    valve: FloatParam,
    #[id = "lowcut"]
    low_cut: FloatParam,
    #[id = "output"]
    output: FloatParam,
}

impl Default for MicParams {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            mic: EnumParam::new("Microphone", MicModel::U47),
            pattern: EnumParam::new("Pattern", PatternSel::Cardioid),
            distance: FloatParam::new("Distance", 20.0, FloatRange::Skewed { min: 1.0, max: 300.0, factor: FloatRange::skew_factor(-2.0) })
                .with_step_size(0.5)
                .with_unit(" cm"),
            angle: FloatParam::new("Angle", 0.0, FloatRange::Linear { min: 0.0, max: 90.0 })
                .with_step_size(1.0)
                .with_unit("°"),
            valve: pct("Valve Drive", 0.3),
            low_cut: hz("Low Cut", 20.0, 20.0, 300.0),
            output: db("Output", 0.0, -18.0, 18.0),
        }
    }
}

const MIC: usize = 0;
const PATTERN: usize = 1;
const DISTANCE: usize = 2;
const ANGLE: usize = 3;
const VALVE: usize = 4;
const LOWCUT: usize = 5;
const OUTPUT: usize = 6;

impl VintageParams for MicParams {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.mic.as_ptr(),
            self.pattern.as_ptr(),
            self.distance.as_ptr(),
            self.angle.as_ptr(),
            self.valve.as_ptr(),
            self.low_cut.as_ptr(),
            self.output.as_ptr(),
        ]
    }
}

// ------------------------------------------------------------------- mics --

#[derive(Clone, Copy, Debug)]
pub struct MicSpec {
    /// Omni, cardioid, figure-8 available?
    pub patterns: [bool; 3],
    /// Membrane/low-frequency limit: high-pass (Hz, Q).
    pub hp: (f64, f64),
    /// On-axis response far from the source: (Hz, Q, dB).
    pub peaks: [(f64, f64, f64); 4],
    pub lp: (f64, f64),
    /// Diaphragm (or ribbon) size, m: sets where off-axis treble falls.
    pub diaphragm: f64,
    pub valve: bool,
}

/// Approximations of the published on-axis curves.
pub const MICS: [MicSpec; 9] = [
    // U47: warm low mids, broad presence rise 5-10 kHz, top rolls off ~15 kHz
    MicSpec { patterns: [true, true, false], hp: (30.0, 0.7), peaks: [(180.0, 0.8, 1.0), (1500.0, 1.0, -0.8), (5500.0, 0.9, 3.0), (10500.0, 1.5, 2.0)], lp: (15500.0, 0.7), diaphragm: 0.034, valve: true },
    // U48: the U47's capsule with cardioid and figure-8
    MicSpec { patterns: [false, true, true], hp: (30.0, 0.7), peaks: [(180.0, 0.8, 1.0), (1500.0, 1.0, -0.8), (5000.0, 0.9, 2.5), (10000.0, 1.5, 1.5)], lp: (15000.0, 0.7), diaphragm: 0.034, valve: true },
    // U67: flatter mids, presence/air lift around 9 kHz
    MicSpec { patterns: [true, true, true], hp: (35.0, 0.7), peaks: [(120.0, 0.8, 0.8), (3000.0, 1.0, 0.0), (9000.0, 0.8, 2.5), (12500.0, 2.0, 1.0)], lp: (17000.0, 0.7), diaphragm: 0.034, valve: true },
    // KM54: small valve cardioid, bright 10 kHz lift
    MicSpec { patterns: [false, true, false], hp: (45.0, 0.7), peaks: [(400.0, 1.0, -0.8), (4000.0, 1.2, 1.0), (10000.0, 1.0, 3.5), (14000.0, 2.0, 1.0)], lp: (18000.0, 0.7), diaphragm: 0.014, valve: true },
    // KM56: small valve multi-pattern
    MicSpec { patterns: [true, true, true], hp: (45.0, 0.7), peaks: [(200.0, 1.0, 0.5), (2500.0, 1.0, 0.0), (8000.0, 0.9, 2.5), (13000.0, 2.0, 1.0)], lp: (18000.0, 0.7), diaphragm: 0.014, valve: true },
    // D19: dynamic cardioid, lean bass, forward 4 kHz, top off by 11-12 kHz
    MicSpec { patterns: [false, true, false], hp: (100.0, 0.7), peaks: [(250.0, 1.0, -1.0), (4000.0, 1.0, 4.0), (7500.0, 2.0, -2.0), (9000.0, 1.0, 0.0)], lp: (11500.0, 0.7), diaphragm: 0.025, valve: false },
    // D20: dynamic cardioid with extended bass and a mid scoop
    MicSpec { patterns: [false, true, false], hp: (40.0, 0.7), peaks: [(90.0, 1.0, 2.0), (650.0, 1.0, -2.0), (5000.0, 1.0, 4.0), (8000.0, 2.0, 0.0)], lp: (12000.0, 0.7), diaphragm: 0.025, valve: false },
    // 4038: ribbon figure-8, smooth, gently dark top
    MicSpec { patterns: [false, false, true], hp: (30.0, 0.6), peaks: [(250.0, 0.7, 0.5), (2000.0, 1.0, 0.0), (9000.0, 0.7, -1.5), (12000.0, 1.0, 0.0)], lp: (13000.0, 0.6), diaphragm: 0.035, valve: false },
    // 4033: cardioid ribbon/pressure hybrid, warm and dark
    MicSpec { patterns: [false, true, false], hp: (40.0, 0.7), peaks: [(150.0, 0.8, 1.0), (1000.0, 1.0, 0.0), (5000.0, 1.0, -1.0), (8000.0, 1.0, 0.0)], lp: (9000.0, 0.6), diaphragm: 0.03, valve: false },
];

const NAMES: &[&str] = &["U47 (valve)", "U48 (valve)", "U67 (valve)", "KM54 (valve)", "KM56 (valve)", "D19 (dynamic)", "D20 (dynamic)", "4038 (ribbon)", "4033 (ribbon)"];

const ABOUT: &[&str] = &[
    "Neumann U47: Abbey Road's main vocal mic of the early 60s. Cardioid or omni. Warm, with a broad presence lift.",
    "Neumann U48: the U47 with cardioid and figure-8. A figure-8 lets two singers share one mic face to face, as the Beatles often did for harmonies.",
    "Neumann U67 (from the mid-60s): vocals and guitar amps. Flatter mids with an airy lift. Omni, cardioid or figure-8.",
    "Neumann KM54: small valve cardioid, bright and detailed. Overheads, acoustic guitar, piano.",
    "Neumann KM56: small valve mic with three patterns. Overheads, strings, acoustic instruments.",
    "AKG D19: Geoff Emerick's drum mic on Revolver-era sessions, close on the kit. Lean bass and a forward upper midrange.",
    "AKG D20: dynamic cardioid with an extended low end. Bass drum and bass amp.",
    "Coles/STC 4038 ribbon: figure-8, smooth and slightly dark. Drum overheads, brass, amps. Strong proximity bass up close.",
    "STC 4033: cardioid ribbon/pressure hybrid: warm, dark and rounded. Guitar amps and voices.",
];

/// Gradient share of a pattern: 0 omni, 0.5 cardioid, 1 figure-8.
fn grad_share(p: usize) -> f64 {
    [0.0, 0.5, 1.0][p.min(2)]
}

/// Pick the requested pattern, or the nearest one this mic has.
pub fn pattern_for(mic: &MicSpec, want: usize) -> usize {
    if mic.patterns[want.min(2)] {
        return want.min(2);
    }
    for d in 1..3 {
        for c in [want as i32 - d, want as i32 + d] {
            if (0..3).contains(&c) && mic.patterns[c as usize] {
                return c as usize;
            }
        }
    }
    1
}

// -------------------------------------------------------------------- dsp --

#[derive(Clone, Copy, PartialEq)]
struct Shape {
    mic: usize,
    pattern: usize,
    distance: f64,
    angle: f64,
    low_cut: f64,
}

struct Chan {
    prox: Biquad,
    peaks: [Biquad; 4],
    lp: Biquad,
    off_axis: Biquad,
    low_cut: Biquad,
    dc: DcBlock,
}

pub struct MicDsp {
    fs: f64,
    chans: Vec<Chan>,
    target: Shape,
    current: Shape,
    built: Option<Shape>,
    valve: Smooth,
    out: Smooth,
    is_valve: bool,
}

impl MicDsp {
    fn rebuild(&mut self) {
        let sh = self.current;
        let m = MICS[sh.mic];
        let fs = self.fs;
        let k = grad_share(sh.pattern);
        let cos = sh.angle.to_radians().cos();
        // pressure part + gradient part with proximity:
        // H(s) = (1-k) + k cos (1 + w0/s), w0 = c/r; times the membrane HP
        let w0 = 343.0 / (sh.distance * 0.01).max(0.005);
        let pat = ((1.0 - k) + k * cos).max(0.03);
        let wh = 2.0 * PI * m.hp.0;
        let n1 = pat;
        let n0 = k * cos * w0;
        // (n1 s + n0) * s / (s^2 + wh/Q s + wh^2)
        let num = [n1, n0, 0.0];
        let den = [1.0, wh / m.hp.1, wh * wh];
        // keep the level steady at 1 kHz as the mic moves (the proximity
        // changes the tone; OUTPUT sets the level)
        let w = 2.0 * PI * 1000.0;
        let mag = |n: [f64; 3], d: [f64; 3]| {
            let (nr, ni) = (n[2] - n[0] * w * w, n[1] * w);
            let (dr, di) = (d[2] - d[0] * w * w, d[1] * w);
            ((nr * nr + ni * ni) / (dr * dr + di * di)).sqrt()
        };
        let norm = 1.0 / mag(num, den).max(1e-6) * pat;
        // off-axis treble: falls above c/(pi d), more the further round
        let f_beam = (343.0 / (PI * m.diaphragm)).min(fs * 0.4);
        let off_db = -12.0 * (1.0 - cos);
        for c in &mut self.chans {
            c.prox.set_analog(fs, [num[0] * norm, num[1] * norm, num[2] * norm], den);
            for (b, &(f, q, g)) in c.peaks.iter_mut().zip(&m.peaks) {
                b.set_peak(fs, f, q, g);
            }
            c.lp.set_lowpass(fs, m.lp.0.min(fs * 0.45), m.lp.1);
            c.off_axis.set_high_shelf(fs, f_beam, 0.7, off_db);
            if sh.low_cut > 20.5 {
                c.low_cut.set_highpass(fs, sh.low_cut, 0.707);
            } else {
                c.low_cut = Biquad::identity();
            }
        }
        self.is_valve = m.valve;
        self.built = Some(sh);
    }

    pub fn effective_pattern(&self) -> usize {
        self.current.pattern
    }
}

impl VintageDsp for MicDsp {
    const METERS: usize = 1;

    fn new(sample_rate: f32, channels: usize) -> Self {
        let fs = sample_rate as f64;
        let sh = Shape { mic: 0, pattern: 1, distance: 20.0, angle: 0.0, low_cut: 20.0 };
        let mut d = Self {
            fs,
            chans: (0..channels.max(1))
                .map(|_| Chan {
                    prox: Biquad::identity(),
                    peaks: [Biquad::identity(); 4],
                    lp: Biquad::identity(),
                    off_axis: Biquad::identity(),
                    low_cut: Biquad::identity(),
                    dc: DcBlock::new(fs),
                })
                .collect(),
            target: sh,
            current: sh,
            built: None,
            valve: Smooth::new(fs, 20.0, 0.3),
            out: Smooth::new(fs, 20.0, 1.0),
            is_valve: true,
        };
        d.rebuild();
        d
    }

    fn reset(&mut self) {
        for c in &mut self.chans {
            c.prox.reset();
            c.peaks.iter_mut().for_each(|b| b.reset());
            c.lp.reset();
            c.off_axis.reset();
            c.low_cut.reset();
            c.dc.reset();
        }
    }

    fn set_params(&mut self, v: &[f32]) {
        let mic = (v[MIC].round() as usize).min(MICS.len() - 1);
        self.target = Shape {
            mic,
            pattern: pattern_for(&MICS[mic], v[PATTERN].round() as usize),
            distance: v[DISTANCE] as f64,
            angle: v[ANGLE] as f64,
            low_cut: v[LOWCUT] as f64,
        };
        self.valve.set(v[VALVE] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }

    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        // glide placement once per block; switch mic/pattern at once
        let t = self.target;
        let c = &mut self.current;
        c.mic = t.mic;
        c.pattern = t.pattern;
        c.low_cut = t.low_cut;
        c.distance *= (t.distance / c.distance).powf(0.4);
        c.angle += (t.angle - c.angle) * 0.4;
        if (c.distance / t.distance - 1.0).abs() < 0.002 {
            c.distance = t.distance;
        }
        if (c.angle - t.angle).abs() < 0.05 {
            c.angle = t.angle;
        }
        if self.built != Some(self.current) {
            self.rebuild();
        }
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        for k in 0..n {
            let drive = self.valve.next();
            let g = self.out.next();
            for (ci, ch) in chans.iter_mut().enumerate() {
                let Some(st) = self.chans.get_mut(ci) else { continue };
                let x = ch[k] as f64;
                let mut y = st.prox.process(x);
                for b in &mut st.peaks {
                    y = b.process(y);
                }
                y = st.lp.process(y);
                y = st.off_axis.process(y);
                if self.is_valve && drive > 0.001 {
                    // mic valve and output transformer: soft, mostly 2nd harmonic
                    let d = 0.4 + 3.0 * drive;
                    y = st.dc.process(tube(y * d, 0.25) / d);
                }
                y = st.low_cut.process(y);
                ch[k] = (y * g) as f32;
            }
        }
        meters[0] = (self.current.pattern + 1) as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "U47 Lead Vocal", values: &[(MIC, 0.0), (PATTERN, 1.0), (DISTANCE, 20.0)] },
        Preset { name: "U48 Shared Harmony (fig-8)", values: &[(MIC, 1.0), (PATTERN, 2.0), (DISTANCE, 40.0)] },
        Preset { name: "U67 on Guitar Amp", values: &[(MIC, 2.0), (PATTERN, 1.0), (DISTANCE, 15.0), (ANGLE, 15.0)] },
        Preset { name: "U67 Amp, Off-Axis (darker)", values: &[(MIC, 2.0), (PATTERN, 1.0), (DISTANCE, 10.0), (ANGLE, 45.0)] },
        Preset { name: "D19 Close Snare/Toms", values: &[(MIC, 5.0), (PATTERN, 1.0), (DISTANCE, 5.0), (ANGLE, 20.0)] },
        Preset { name: "D20 Bass Drum / Bass Amp", values: &[(MIC, 6.0), (PATTERN, 1.0), (DISTANCE, 8.0)] },
        Preset { name: "4038 Drum Overhead", values: &[(MIC, 7.0), (PATTERN, 2.0), (DISTANCE, 120.0)] },
        Preset { name: "4038 Up Close on an Amp (big bass)", values: &[(MIC, 7.0), (PATTERN, 2.0), (DISTANCE, 5.0), (LOWCUT, 60.0)] },
        Preset { name: "KM54 Acoustic Guitar", values: &[(MIC, 3.0), (PATTERN, 1.0), (DISTANCE, 30.0), (ANGLE, 20.0)] },
        Preset { name: "KM56 Piano", values: &[(MIC, 4.0), (PATTERN, 1.0), (DISTANCE, 40.0)] },
        Preset { name: "4033 Warm Amp", values: &[(MIC, 8.0), (PATTERN, 1.0), (DISTANCE, 15.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "STUDIO MICS",
        subtitle: ("ABBEY ROAD'S MICROPHONES", "VALVE · DYNAMIC · RIBBON"),
        accent: ui::SLATE,
        size: (W, H),
        items: vec![
            Item::RadioList { p: MIC, x: 34.0, y: 82.0, w: 178.0, row_h: 22.0, names: NAMES,
                tip: "Choose the microphone." },
            Item::Polar { x: 330.0, y: 152.0, r: 58.0, pattern: 0, angle: ANGLE },
            Item::Caption { p: MIC, x: 420.0, y: 96.0, w: 190.0, size: 11.0, texts: ABOUT },
            Item::Divider { y: 284.0 },
            Item::Selector { p: PATTERN, x: 70.0, y: 337.0, label: "PATTERN", positions: &["OMNI", "CARD", "FIG-8"],
                tip: "Polar pattern. Mics that don't have the one you pick use the nearest they do (the plot shows what's in use)." },
            Item::Knob { p: DISTANCE, x: 168.0, y: 337.0, r: 21.0, label: "DISTANCE",
                tip: "Source to mic. Closer = more proximity bass on cardioid and figure-8 (none on omni); the level is held steady at 1 kHz.", active: None },
            Item::Knob { p: ANGLE, x: 256.0, y: 337.0, r: 21.0, label: "ANGLE",
                tip: "Off-axis angle. On a speaker this is like moving from the cone centre (bright) toward the edge (darker); the pattern also lowers the level.", active: None },
            Item::Knob { p: VALVE, x: 344.0, y: 337.0, r: 21.0, label: "VALVE DRIVE",
                tip: "The valve mics' own valve and transformer (no effect on dynamics and ribbons).", active: None },
            Item::Knob { p: LOWCUT, x: 432.0, y: 337.0, r: 21.0, label: "LOW CUT", tip: "High-pass, like a desk's bass cut. 20 Hz = off.", active: None },
            Item::Knob { p: OUTPUT, x: 520.0, y: 337.0, r: 21.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, MicParams, MicDsp, panel,
    name: "Studio Mics", class_id: *b"S2StudioMicsDIY1", vendor: "PEQUOD Beatles",
    categories: [Fx, Filter]
);

#[cfg(test)]
mod tests {
    use super::*;

    fn gain_db(mic: f32, pattern: f32, dist: f32, angle: f32, f: f64) -> f64 {
        let fs = 48000.0;
        let mut d = MicDsp::new(fs as f32, 1);
        let p = [mic, pattern, dist, angle, 0.0, 20.0, 0.0];
        for _ in 0..30 {
            d.set_params(&p);
            d.process(&mut [&mut vec![0.0f32; 64][..]], &mut [0.0]);
        }
        let n = 48000;
        let mut x: Vec<f32> = (0..n).map(|k| (0.1 * (2.0 * PI * f * k as f64 / fs).sin()) as f32).collect();
        d.process(&mut [&mut x[..]], &mut [0.0]);
        let tail = &x[n / 2..];
        let rms = (tail.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / tail.len() as f64).sqrt();
        20.0 * (rms / (0.1 / 2f64.sqrt())).log10()
    }

    #[test]
    fn proximity_effect_follows_pattern_and_distance() {
        // 4038 figure-8: big bass boost close up, none far away
        let close = gain_db(7.0, 2.0, 3.0, 0.0, 100.0) - gain_db(7.0, 2.0, 3.0, 0.0, 1000.0);
        let far = gain_db(7.0, 2.0, 200.0, 0.0, 100.0) - gain_db(7.0, 2.0, 200.0, 0.0, 1000.0);
        // U67 omni: no proximity effect
        let omni = gain_db(2.0, 0.0, 3.0, 0.0, 100.0) - gain_db(2.0, 0.0, 200.0, 0.0, 100.0);
        // U67 cardioid: in between
        let card = gain_db(2.0, 1.0, 3.0, 0.0, 100.0) - gain_db(2.0, 1.0, 200.0, 0.0, 100.0);
        println!("fig-8 100 Hz re 1 kHz: 3 cm {close:.1} dB, 2 m {far:.1} dB; omni change {omni:.1} dB; cardioid change {card:.1} dB");
        assert!(close > far + 12.0);
        assert!(omni.abs() < 1.0);
        assert!(card > 6.0 && card < close - far);
    }

    #[test]
    fn off_axis_is_darker() {
        let on = gain_db(2.0, 1.0, 50.0, 0.0, 8000.0) - gain_db(2.0, 1.0, 50.0, 0.0, 1000.0);
        let off = gain_db(2.0, 1.0, 50.0, 60.0, 8000.0) - gain_db(2.0, 1.0, 50.0, 60.0, 1000.0);
        println!("U67 8 kHz re 1 kHz: on-axis {on:.1} dB, 60 deg {off:.1} dB");
        assert!(off < on - 3.0);
    }

    #[test]
    fn patterns_fall_back_to_what_the_mic_has() {
        assert_eq!(pattern_for(&MICS[7], 0), 2); // 4038 is figure-8 only
        assert_eq!(pattern_for(&MICS[0], 2), 1); // U47 has no figure-8
        assert_eq!(pattern_for(&MICS[2], 0), 0);
    }

    #[test]
    fn finite_for_every_mic() {
        for mic in 0..9 {
            for pat in 0..3 {
                let g = gain_db(mic as f32, pat as f32, 1.0, 90.0, 60.0);
                assert!(g.is_finite() && g < 40.0, "mic {mic} pattern {pat}: {g}");
            }
        }
    }
}
