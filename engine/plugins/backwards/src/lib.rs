//! Backwards — real-time reverse echo.
//!
//! The studio trick was to turn the tape over: everything plays backwards,
//! so notes swell in instead of dying away. This does it live: the incoming
//! audio is cut into slices of LENGTH, and each slice is played back in
//! reverse while the next one is recorded. Two overlapping playback heads
//! cross-fade so there are no clicks between slices. OCTAVE mode plays the
//! slices back at double speed, an octave up (the tape-at-15-recorded-at-7.5
//! trick).

use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
use vintage::dsp::*;
use vintage::nih_plug_egui::EguiState;
use vintage::params::*;
use vintage::ui::{self, Item, MeterKind, Panel, Preset};
use vintage::{VintageDsp, VintageParams};

const W: u32 = 640;
const H: u32 = 432;

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Speed {
    #[name = "Normal"]
    Normal,
    #[name = "Octave Up"]
    Octave,
}

#[derive(Params)]
pub struct BackwardsParams {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "length"]
    length: FloatParam,
    #[id = "mix"]
    mix: FloatParam,
    #[id = "feedback"]
    feedback: FloatParam,
    #[id = "tone"]
    tone: FloatParam,
    #[id = "width"]
    width: FloatParam,
    #[id = "output"]
    output: FloatParam,
    #[id = "speed"]
    speed: EnumParam<Speed>,
}

impl Default for BackwardsParams {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            length: ms("Length", 450.0, 80.0, 2000.0),
            mix: pct("Mix", 0.5),
            feedback: pct("Feedback", 0.2),
            tone: hz("Tone", 9000.0, 1500.0, 20000.0),
            width: pct("Width", 0.5),
            output: db("Output", 0.0, -18.0, 6.0),
            speed: EnumParam::new("Speed", Speed::Normal),
        }
    }
}

const LENGTH: usize = 0;
const MIX: usize = 1;
const FEEDBACK: usize = 2;
const TONE: usize = 3;
const WIDTH: usize = 4;
const OUTPUT: usize = 5;
const SPEED: usize = 6;

impl VintageParams for BackwardsParams {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.length.as_ptr(),
            self.mix.as_ptr(),
            self.feedback.as_ptr(),
            self.tone.as_ptr(),
            self.width.as_ptr(),
            self.output.as_ptr(),
            self.speed.as_ptr(),
        ]
    }
}

/// One reverse playback head. Its slice length is latched at the start of
/// each slice so moving the LENGTH knob never causes a jump mid-slice.
#[derive(Clone, Copy)]
struct Head {
    phase: f64,  // 0..1 through the current slice
    len: f64,    // samples, latched
}

pub struct BackwardsDsp {
    fs: f64,
    lines: Vec<DelayLine>,
    tone: [OnePole; 2],
    heads: [Head; 2],
    target_len: f64,
    octave: bool,
    fb: Smooth,
    mix: Smooth,
    width: Smooth,
    out: Smooth,
    last_wet: [f64; 2],
}

impl VintageDsp for BackwardsDsp {
    const METERS: usize = 2;

    fn new(sample_rate: f32, _channels: usize) -> Self {
        let fs = sample_rate as f64;
        let max = (fs * 2.0 * 3.0) as usize + 64; // 2 s slices, octave mode reads 3x back
        let len = 0.45 * fs;
        Self {
            fs,
            lines: vec![DelayLine::new(max), DelayLine::new(max)],
            tone: [OnePole::default(); 2],
            heads: [Head { phase: 0.0, len }, Head { phase: 0.5, len }],
            target_len: len,
            octave: false,
            fb: Smooth::new(fs, 30.0, 0.2),
            mix: Smooth::new(fs, 20.0, 0.5),
            width: Smooth::new(fs, 20.0, 0.5),
            out: Smooth::new(fs, 20.0, 1.0),
            last_wet: [0.0; 2],
        }
    }

    fn reset(&mut self) {
        self.lines.iter_mut().for_each(|l| l.reset());
        self.tone.iter_mut().for_each(|t| t.reset());
        self.heads[0].phase = 0.0;
        self.heads[1].phase = 0.5;
        self.last_wet = [0.0; 2];
    }

    fn set_params(&mut self, v: &[f32]) {
        self.target_len = (v[LENGTH] as f64 * 0.001 * self.fs).max(64.0);
        self.mix.set(v[MIX] as f64);
        self.fb.set(v[FEEDBACK] as f64 * 0.9);
        self.width.set(v[WIDTH] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
        self.octave = v[SPEED] > 0.5;
        for t in &mut self.tone {
            t.set_lowpass(self.fs, v[TONE] as f64);
        }
    }

    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let stereo = chans.len() >= 2;
        let read_mult = if self.octave { 3.0 } else { 2.0 };
        let mut peak = 0.0f64;
        let mut slice_start = 0.0f64;
        for k in 0..n {
            let x = if stereo { [chans[0][k] as f64, chans[1][k] as f64] } else { [chans[0][k] as f64; 2] };
            let fb = self.fb.next();
            let mix = self.mix.next();
            let width = self.width.next();
            let o = self.out.next();

            for c in 0..2 {
                self.lines[c].write(x[c] + fb * self.last_wet[c]);
            }
            let mut wet = [0.0f64; 2];
            for h in &mut self.heads {
                // reading at delay 2*p (3*p in octave mode) while time moves
                // forward by p plays the last slice backwards
                let d = read_mult * h.phase * h.len + 1.0;
                let w = (std::f64::consts::PI * h.phase).sin().powi(2);
                for c in 0..2 {
                    wet[c] += self.lines[c].read(d) * w;
                }
                h.phase += 1.0 / h.len;
                if h.phase >= 1.0 {
                    h.phase -= 1.0;
                    h.len = self.target_len;
                    slice_start = 1.0;
                }
            }
            for c in 0..2 {
                wet[c] = self.tone[c].lp(wet[c]);
                self.last_wet[c] = wet[c];
            }
            // width: swap a little of each side's slices across
            let (wl, wr) = {
                let mid = 0.5 * (wet[0] + wet[1]);
                let side = 0.5 * (wet[0] - wet[1]) * (0.5 + width);
                (mid + side, mid - side)
            };
            let dry_g = (2.0 * (1.0 - mix)).min(1.0);
            let wet_g = (2.0 * mix).min(1.0);
            let l = (x[0] * dry_g + wl * wet_g) * o;
            let r = (x[1] * dry_g + wr * wet_g) * o;
            if stereo {
                chans[0][k] = l as f32;
                chans[1][k] = r as f32;
            } else {
                chans[0][k] = (0.5 * (l + r)) as f32;
            }
            peak = peak.max(l.abs()).max(r.abs());
        }
        meters[0] = peak as f32;
        // lamp flashes at the start of every slice
        meters[1] = if slice_start > 0.0 { 1.0 } else { 0.0 };
    }
}

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Backwards Vocal Tag", values: &[(LENGTH, 600.0), (MIX, 0.5), (FEEDBACK, 0.15), (TONE, 7000.0)] },
        Preset { name: "Backwards Guitar Solo", values: &[(LENGTH, 380.0), (MIX, 0.65), (FEEDBACK, 0.3), (TONE, 9000.0), (WIDTH, 0.7)] },
        Preset { name: "Reverse Swell Pad", values: &[(LENGTH, 1600.0), (MIX, 0.55), (FEEDBACK, 0.55), (TONE, 5000.0), (WIDTH, 1.0)] },
        Preset { name: "Octave Reverse Sparkle", values: &[(SPEED, 1.0), (LENGTH, 300.0), (MIX, 0.4), (FEEDBACK, 0.25), (TONE, 11000.0)] },
        Preset { name: "Fully Backwards (wet only)", values: &[(LENGTH, 900.0), (MIX, 1.0), (FEEDBACK, 0.0), (TONE, 20000.0)] },
        Preset { name: "Short Reverse Stutter", values: &[(LENGTH, 120.0), (MIX, 0.45), (FEEDBACK, 0.35), (TONE, 8000.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "BACKWARDS",
        subtitle: ("REVERSE TAPE", "ECHO MACHINE"),
        accent: ui::WALNUT,
        size: (W, H),
        items: vec![
            Item::Knob { p: LENGTH, x: 130.0, y: 172.0, r: 50.0, label: "LENGTH",
                tip: "Length of each slice that gets played backwards. Longer = whole phrases reversed; shorter = stuttery reversed fragments.", active: None },
            Item::Knob { p: MIX, x: 510.0, y: 172.0, r: 50.0, label: "MIX",
                tip: "Backwards amount. Up to 50% the dry signal stays at full level; 100% is backwards only.", active: None },
            Item::Meter { m: 0, x: 222.0, y: 92.0, w: 196.0, h: 136.0, kind: MeterKind::Vu, caption: "OUTPUT" },
            Item::Lamp { m: 1, x: 236.0, y: 254.0, label: "SLICE", lo: 0.0, hi: 1.0 },
            Item::Switch { p: SPEED, x: 390.0, y: 254.0, labels: ["NORMAL", "OCTAVE"],
                tip: "NORMAL: slices play backwards at the same pitch.\nOCTAVE: they play backwards at double speed, an octave up." },
            Item::Divider { y: 284.0 },
            Item::Knob { p: FEEDBACK, x: 140.0, y: 332.0, r: 22.0, label: "FEEDBACK", tip: "Sends the reversed sound back in, so slices build up and overlap.", active: None },
            Item::Knob { p: TONE, x: 260.0, y: 332.0, r: 22.0, label: "TONE", tip: "Darkens the backwards signal.", active: None },
            Item::Knob { p: WIDTH, x: 380.0, y: 332.0, r: 22.0, label: "WIDTH", tip: "Stereo spread of the backwards signal.", active: None },
            Item::Knob { p: OUTPUT, x: 500.0, y: 332.0, r: 22.0, label: "OUTPUT", tip: "Final level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, BackwardsParams, BackwardsDsp, panel,
    name: "Backwards", class_id: *b"S2BackwardsDIY01", vendor: "PEQUOD Beatles",
    categories: [Fx, Delay]
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// A rising tone, reversed, should come out falling.
    #[test]
    fn plays_slices_backwards() {
        let fs = 48000.0;
        let mut d = BackwardsDsp::new(fs as f32, 1);
        d.set_params(&[500.0, 1.0, 0.0, 20000.0, 0.5, 0.0, 0.0]);
        let n = (fs * 3.0) as usize;
        // repeating 0.5 s sweep 300 -> 1200 Hz
        let mut ph = 0.0;
        let mut x: Vec<f32> = (0..n)
            .map(|k| {
                let t = (k as f64 / fs) % 0.5;
                let f = 300.0 + 1800.0 * t;
                ph += f / fs;
                (0.4 * (2.0 * PI * ph).sin()) as f32
            })
            .collect();
        let mut m = [0.0f32; 2];
        for c in x.chunks_mut(512) {
            d.process(&mut [c], &mut m);
        }
        // zero-crossing rate over the output: measure trend inside one
        // reversed slice where one head dominates (around its window peak)
        let rate = |a: usize| {
            let seg = &x[a..a + 1200];
            seg.windows(2).filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0)).count()
        };
        // head 0 slices: [k*24000, (k+1)*24000), window peak at the middle
        let early = rate(48000 + 7000);
        let late = rate(48000 + 16000);
        println!("zero crossings early {early}, late {late} (reversed sweep should fall)");
        assert!(late < early);
        assert!(x.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn feedback_is_stable() {
        let mut d = BackwardsDsp::new(48000.0, 2);
        d.set_params(&[200.0, 0.5, 1.0, 20000.0, 1.0, 0.0, 1.0]);
        let mut l: Vec<f32> = (0..240000).map(|k| if k < 4800 { (k as f32 * 0.05).sin() } else { 0.0 }).collect();
        let mut r = l.clone();
        let mut m = [0.0f32; 2];
        for (a, b) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
            d.process(&mut [a, b], &mut m);
        }
        assert!(l.iter().chain(r.iter()).all(|v| v.is_finite() && v.abs() < 4.0));
    }
}
