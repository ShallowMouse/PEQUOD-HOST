//! Doubler — artificial double tracking (ADT).
//!
//! A second copy of the input is delayed by a few tens of milliseconds and
//! the delay is made to wander slowly, the way a varispeed tape machine
//! drifts, so the copy sounds like a second take. FLANGE mode pulls the
//! delay right down and sweeps it for the classic tape-flange whoosh.

use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
use vintage::dsp::*;
use vintage::nih_plug_egui::EguiState;
use vintage::params::*;
use vintage::ui::{self, Preset, Item, MeterKind, Panel};
use vintage::{VintageDsp, VintageParams};

const W: u32 = 640;
const H: u32 = 432;

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Mode {
    #[name = "ADT"]
    Adt,
    #[name = "Flange"]
    Flange,
}

#[derive(Params)]
pub struct DoublerParams {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,

    #[id = "delay"]
    delay: FloatParam,
    #[id = "wander"]
    wander: FloatParam,
    #[id = "rate"]
    rate: FloatParam,
    #[id = "level"]
    level: FloatParam,
    #[id = "spread"]
    spread: FloatParam,
    #[id = "tone"]
    tone: FloatParam,
    #[id = "output"]
    output: FloatParam,
    #[id = "mode"]
    mode: EnumParam<Mode>,
}

impl Default for DoublerParams {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            delay: ms("Delay", 18.0, 4.0, 60.0),
            wander: pct("Wander", 0.35),
            rate: rate("Rate", 0.5, 0.05, 4.0),
            level: db("Double Level", -3.0, -24.0, 3.0),
            spread: pct("Spread", 0.7),
            tone: hz("Tone", 9000.0, 1500.0, 20000.0),
            output: db("Output", 0.0, -18.0, 6.0),
            mode: EnumParam::new("Mode", Mode::Adt),
        }
    }
}

// linked parameter order
const DELAY: usize = 0;
const WANDER: usize = 1;
const RATE: usize = 2;
const LEVEL: usize = 3;
const SPREAD: usize = 4;
const TONE: usize = 5;
const OUTPUT: usize = 6;
const MODE: usize = 7;

impl VintageParams for DoublerParams {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.delay.as_ptr(),
            self.wander.as_ptr(),
            self.rate.as_ptr(),
            self.level.as_ptr(),
            self.spread.as_ptr(),
            self.tone.as_ptr(),
            self.output.as_ptr(),
            self.mode.as_ptr(),
        ]
    }
}

pub struct DoublerDsp {
    fs: f64,
    line: DelayLine,
    wander: Wander,
    lfo: Lfo,
    tone: OnePole,
    low: OnePole,
    delay_ms: Smooth,
    depth: Smooth,
    level: Smooth,
    spread: Smooth,
    out: Smooth,
    rate: f64,
    flange: bool,
    fb: f64,
}

impl VintageDsp for DoublerDsp {
    const METERS: usize = 1;

    fn new(sample_rate: f32, _channels: usize) -> Self {
        let fs = sample_rate as f64;
        let mut low = OnePole::default();
        low.set_lowpass(fs, 90.0);
        Self {
            fs,
            line: DelayLine::new((fs * 0.08) as usize),
            wander: Wander::new(fs, 0.5, 11),
            lfo: Lfo::default(),
            tone: OnePole::default(),
            low,
            delay_ms: Smooth::new(fs, 60.0, 18.0),
            depth: Smooth::new(fs, 60.0, 0.0),
            level: Smooth::new(fs, 20.0, 0.7),
            spread: Smooth::new(fs, 20.0, 0.7),
            out: Smooth::new(fs, 20.0, 1.0),
            rate: 0.5,
            flange: false,
            fb: 0.0,
        }
    }

    fn reset(&mut self) {
        self.line.reset();
        self.tone.reset();
        self.low.reset();
        self.fb = 0.0;
    }

    fn set_params(&mut self, v: &[f32]) {
        let v = |i: usize| v[i] as f64;
        self.flange = v(MODE) > 0.5;
        if self.flange {
            // short delay, wide sweep: 0.3..~6 ms centre
            self.delay_ms.set((v(DELAY) / 10.0).clamp(0.3, 6.0));
            self.depth.set(v(WANDER) * 0.9);
        } else {
            self.delay_ms.set(v(DELAY));
            self.depth.set(v(WANDER) * 4.0); // up to +-4 ms of drift
        }
        self.rate = v(RATE);
        self.wander.set_rate(self.fs, self.rate);
        self.level.set(db_to_gain(v(LEVEL)));
        self.spread.set(v(SPREAD));
        self.out.set(db_to_gain(v(OUTPUT)));
        self.tone.set_lowpass(self.fs, v(TONE));
    }

    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let stereo = chans.len() >= 2;
        let mut peak = 0.0f64;
        for k in 0..n {
            let (xl, xr) = if stereo { (chans[0][k] as f64, chans[1][k] as f64) } else { (chans[0][k] as f64, chans[0][k] as f64) };
            let mono = 0.5 * (xl + xr);

            let base = self.delay_ms.next();
            let depth = self.depth.next();
            let sine = self.lfo.next(self.fs, self.rate);
            let modv = if self.flange {
                // classic sweep: delay moves between ~0.1x and ~1.9x the centre
                base * depth * sine
            } else {
                // varispeed drift: mostly random, a little periodic
                depth * (0.75 * self.wander.next() + 0.25 * sine)
            };
            let d_ms = (base + modv).max(0.1);
            let fb_amt = if self.flange { 0.45 } else { 0.0 };
            self.line.write(mono + self.fb * fb_amt);
            let mut dbl = self.line.read(d_ms * 0.001 * self.fs);
            self.fb = dbl;
            dbl = self.tone.lp(dbl);
            dbl -= self.low.lp(dbl) * 0.7; // thin the double's low end a little

            let g = self.level.next();
            let s = self.spread.next();
            let o = self.out.next();
            if stereo {
                // original leans left, double leans right as Spread increases
                let a_dry = (0.5 - 0.35 * s) * std::f64::consts::FRAC_PI_2;
                let a_dbl = (0.5 + 0.5 * s) * std::f64::consts::FRAC_PI_2;
                let (dl, dr) = (a_dry.cos() * 2f64.sqrt(), a_dry.sin() * 2f64.sqrt());
                let (bl, br) = (a_dbl.cos() * 2f64.sqrt(), a_dbl.sin() * 2f64.sqrt());
                let side = 0.5 * (xl - xr);
                let l = ((mono * dl + side) + dbl * g * bl) * o;
                let r = ((mono * dr - side) + dbl * g * br) * o;
                chans[0][k] = l as f32;
                chans[1][k] = r as f32;
                peak = peak.max(l.abs()).max(r.abs());
            } else {
                let y = (mono + dbl * g) * o;
                chans[0][k] = y as f32;
                peak = peak.max(y.abs());
            }
        }
        meters[0] = peak as f32;
    }
}

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Classic ADT Vocal", values: &[] },
        Preset { name: "Wide Double", values: &[(DELAY, 24.0), (WANDER, 0.45), (SPREAD, 1.0), (LEVEL, -2.0)] },
        Preset { name: "Subtle Thickener", values: &[(DELAY, 12.0), (WANDER, 0.25), (LEVEL, -9.0), (SPREAD, 0.4)] },
        Preset { name: "Wobbly Second Take", values: &[(DELAY, 30.0), (WANDER, 0.8), (RATE, 0.9), (LEVEL, -4.0), (TONE, 7000.0)] },
        Preset { name: "Tape Flange", values: &[(MODE, 1.0), (DELAY, 25.0), (WANDER, 0.7), (RATE, 0.25), (LEVEL, 0.0), (SPREAD, 0.3)] },
        Preset { name: "Jet Whoosh", values: &[(MODE, 1.0), (DELAY, 40.0), (WANDER, 1.0), (RATE, 0.12), (LEVEL, 0.0), (SPREAD, 0.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "DOUBLER",
        subtitle: ("ARTIFICIAL DOUBLE", "TRACKING UNIT"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![
            Item::Knob { p: DELAY, x: 130.0, y: 172.0, r: 50.0, label: "DELAY",
                tip: "How far behind the original the double sits. 15-30 ms is the classic ADT spot; in FLANGE mode this is scaled down to a few ms.", active: None },
            Item::Knob { p: WANDER, x: 510.0, y: 172.0, r: 50.0, label: "WANDER",
                tip: "How much the delay drifts, like a varispeed tape machine. This is what makes it sound like a second take instead of an echo.", active: None },
            Item::Meter { m: 0, x: 222.0, y: 92.0, w: 196.0, h: 136.0, kind: MeterKind::Vu, caption: "OUTPUT" },
            Item::Switch { p: MODE, x: 320.0, y: 254.0, labels: ["ADT", "FLANGE"],
                tip: "ADT: a natural-sounding second voice.\nFLANGE: short, sweeping delay with feedback, the tape-flange effect." },
            Item::Divider { y: 284.0 },
            Item::Knob { p: RATE, x: 92.0, y: 332.0, r: 22.0, label: "RATE", tip: "Speed of the drift or sweep.", active: None },
            Item::Knob { p: LEVEL, x: 206.0, y: 332.0, r: 22.0, label: "DOUBLE", tip: "Level of the double.", active: None },
            Item::Knob { p: SPREAD, x: 320.0, y: 332.0, r: 22.0, label: "SPREAD", tip: "Stereo: pushes the original left and the double right.", active: None },
            Item::Knob { p: TONE, x: 434.0, y: 332.0, r: 22.0, label: "TONE", tip: "Darkens the double, like a second tape generation.", active: None },
            Item::Knob { p: OUTPUT, x: 548.0, y: 332.0, r: 22.0, label: "OUTPUT", tip: "Final level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, DoublerParams, DoublerDsp, panel,
    name: "Doubler", class_id: *b"S2DoublerDIY0001", vendor: "PEQUOD Beatles",
    categories: [Fx, Modulation]
);

#[cfg(test)]
mod tests {
    use super::*;

    fn run(mode: f32, secs: f64) -> (Vec<f32>, Vec<f32>) {
        let fs = 48000.0;
        let mut d = DoublerDsp::new(fs as f32, 2);
        // defaults in plain units, in linked order
        d.set_params(&[18.0, 0.35, 0.5, -3.0, 0.7, 9000.0, 0.0, mode]);
        let n = (fs * secs) as usize;
        let mut l: Vec<f32> = (0..n).map(|k| if k % 4800 == 0 { 1.0 } else { 0.0 }).collect();
        let mut r = l.clone();
        let mut m = [0.0f32; 1];
        {
            let mut chans: Vec<&mut [f32]> = vec![&mut l, &mut r];
            d.process(&mut chans, &mut m);
        }
        (l, r)
    }

    #[test]
    fn double_arrives_after_delay() {
        let (_, r) = run(0.0, 0.1);
        // the double's impulse should land roughly 14-22 ms after the click
        let first = r.iter().enumerate().skip(48).filter(|(_, v)| v.abs() > 0.05).map(|(i, _)| i).next().unwrap();
        let ms = first as f64 / 48.0;
        println!("double at {ms:.1} ms");
        assert!((10.0..26.0).contains(&ms));
    }

    #[test]
    fn stays_finite() {
        for mode in [0.0, 1.0] {
            let (l, r) = run(mode, 2.0);
            assert!(l.iter().chain(r.iter()).all(|v| v.is_finite() && v.abs() < 8.0));
        }
    }
}
