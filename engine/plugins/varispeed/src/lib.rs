//! Varispeed — tape speed control.
//!
//! On a tape machine, running faster raises the pitch and brightens the
//! voice (and slower does the opposite) — formants move with it, which is
//! the "slightly unreal" quality of a sped-up take. In real time we can't
//! actually make a track shorter, so this does the pitch part: two tape
//! heads sweep across a short buffer and cross-fade, like a rotating-head
//! pitch changer. STOP winds the tape down to a halt (and RUN back up)
//! over the RAMP time. A little of a slightly detuned copy (MIX ~30%,
//! +/-10-20 cents) is the old double-tracking-by-varispeed trick.

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
pub enum Transport {
    #[name = "Run"]
    Run,
    #[name = "Stop"]
    Stop,
}

#[derive(Params)]
pub struct VarispeedParams {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "speed"]
    speed: FloatParam,
    #[id = "fine"]
    fine: FloatParam,
    #[id = "mix"]
    mix: FloatParam,
    #[id = "tone"]
    tone: FloatParam,
    #[id = "window"]
    window: FloatParam,
    #[id = "ramp"]
    ramp: FloatParam,
    #[id = "output"]
    output: FloatParam,
    #[id = "transport"]
    transport: EnumParam<Transport>,
}

impl Default for VarispeedParams {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            speed: FloatParam::new("Speed", 0.0, FloatRange::Linear { min: -12.0, max: 12.0 })
                .with_step_size(0.01)
                .with_unit(" st"),
            fine: FloatParam::new("Fine", 0.0, FloatRange::Linear { min: -50.0, max: 50.0 })
                .with_step_size(0.1)
                .with_unit(" ct"),
            mix: pct("Mix", 1.0),
            tone: hz("Tone", 20000.0, 2000.0, 20000.0),
            window: ms("Head Size", 60.0, 20.0, 150.0),
            ramp: secs("Ramp", 0.4, 0.01, 6.0),
            output: db("Output", 0.0, -18.0, 6.0),
            transport: EnumParam::new("Transport", Transport::Run),
        }
    }
}

const SPEED: usize = 0;
const FINE: usize = 1;
const MIX: usize = 2;
const TONE: usize = 3;
const WINDOW: usize = 4;
const RAMP: usize = 5;
const OUTPUT: usize = 6;
const TRANSPORT: usize = 7;

impl VintageParams for VarispeedParams {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.speed.as_ptr(),
            self.fine.as_ptr(),
            self.mix.as_ptr(),
            self.tone.as_ptr(),
            self.window.as_ptr(),
            self.ramp.as_ptr(),
            self.output.as_ptr(),
            self.transport.as_ptr(),
        ]
    }
}

/// Length of the cross-fade when one tape head hands over to the other.
const XFADE_MS: f64 = 12.0;
/// How far either side of the nominal restart point to look for the best
/// phase match, and how much signal to compare.
const SEARCH_MS: f64 = 9.0;
const MATCH_LEN: usize = 384;

/// Lowest speed STOP winds down to (as a ratio).
const STOP_RATIO: f64 = 0.02;
const MIN_DELAY: f64 = 2.0;

pub struct VarispeedDsp {
    fs: f64,
    lines: Vec<DelayLine>,
    tone: [OnePole; 2],
    /// Delay (samples) of the two playback heads.
    d: [f64; 2],
    /// Which head is playing; the other one fades in at a hand-over.
    active: usize,
    /// Hand-over progress 0..1 (None = no hand-over in progress).
    fade: Option<f64>,
    ratio: f64,
    target: f64,
    ramp_c: f64,
    win: Smooth,
    mix: Smooth,
    out: Smooth,
    tone_on: bool,
}

impl VarispeedDsp {
    /// Find the delay near `target` whose upcoming audio best matches what
    /// the head at `current` is playing (normalised cross-correlation), so
    /// the hand-over is phase-coherent: no lost or gained cycles, no flam.
    fn best_match(&self, current: f64, target: f64, lo: f64, hi: f64) -> f64 {
        let line = &self.lines[0];
        let search = (SEARCH_MS * 0.001 * self.fs) as i64;
        let cur = current.round() as i64;
        let mut best = (f64::MIN, target);
        let mut e_ref = 0.0;
        for j in 0..MATCH_LEN as i64 {
            let v = line.read_int((cur + j).max(1) as usize);
            e_ref += v * v;
        }
        if e_ref < 1e-12 {
            return target;
        }
        let t0 = target.round() as i64;
        let mut c = t0 - search;
        while c <= t0 + search {
            if (c as f64) >= lo && (c as f64) <= hi {
                let (mut num, mut e) = (0.0, 0.0);
                for j in 0..MATCH_LEN as i64 {
                    let a = line.read_int((cur + j).max(1) as usize);
                    let b = line.read_int((c + j).max(1) as usize);
                    num += a * b;
                    e += b * b;
                }
                let score = num / (e * e_ref).sqrt().max(1e-12);
                if score > best.0 {
                    best = (score, c as f64 + (current - current.round()));
                }
            }
            c += 1;
        }
        best.1
    }
}

impl VintageDsp for VarispeedDsp {
    const METERS: usize = 2;

    fn new(sample_rate: f32, _channels: usize) -> Self {
        let fs = sample_rate as f64;
        let max = (fs * 0.16) as usize + 16;
        Self {
            fs,
            lines: vec![DelayLine::new(max), DelayLine::new(max)],
            tone: [OnePole::default(); 2],
            d: [MIN_DELAY + 0.03 * fs, MIN_DELAY + 0.03 * fs],
            active: 0,
            fade: None,
            ratio: 1.0,
            target: 1.0,
            ramp_c: coeff(fs, 130.0),
            win: Smooth::new(fs, 200.0, 0.06 * fs),
            mix: Smooth::new(fs, 20.0, 1.0),
            out: Smooth::new(fs, 20.0, 1.0),
            tone_on: false,
        }
    }

    fn reset(&mut self) {
        self.lines.iter_mut().for_each(|l| l.reset());
        self.tone.iter_mut().for_each(|t| t.reset());
        self.d = [MIN_DELAY + 0.03 * self.fs; 2];
        self.active = 0;
        self.fade = None;
    }

    fn set_params(&mut self, v: &[f32]) {
        let semis = v[SPEED] as f64 + v[FINE] as f64 / 100.0;
        self.target = if v[TRANSPORT] > 0.5 { STOP_RATIO } else { 2f64.powf(semis / 12.0) };
        // ramp time = time to get ~95% of the way there
        self.ramp_c = coeff(self.fs, v[RAMP] as f64 * 1000.0 / 3.0);
        self.win.set(v[WINDOW] as f64 * 0.001 * self.fs);
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
        self.tone_on = v[TONE] < 19900.0;
        for t in &mut self.tone {
            t.set_lowpass(self.fs, v[TONE] as f64);
        }
    }

    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let stereo = chans.len() >= 2;
        let mut peak = 0.0f64;
        for k in 0..n {
            let x = if stereo { [chans[0][k] as f64, chans[1][k] as f64] } else { [chans[0][k] as f64; 2] };
            // tape inertia: glide to the target speed (in pitch, i.e. log domain)
            let lr = self.target.ln() + self.ramp_c * (self.ratio.ln() - self.target.ln());
            self.ratio = lr.exp();
            let win = self.win.next();
            let mix = self.mix.next();
            let o = self.out.next();

            for c in 0..2 {
                self.lines[c].write(x[c]);
            }
            // both heads move through the buffer at (1 - ratio) per sample:
            // moving towards "now" plays faster, away from it plays slower
            for h in 0..2 {
                self.d[h] += 1.0 - self.ratio;
            }
            let (lo, hi) = (MIN_DELAY, MIN_DELAY + win);
            let xf = (XFADE_MS * 0.001 * self.fs).min(win * 0.45);
            if self.fade.is_none() {
                let a = self.active;
                let rate = 1.0 - self.ratio;
                let running_out = (rate < 0.0 && self.d[a] + rate * xf <= lo) || (rate > 0.0 && self.d[a] + rate * xf >= hi);
                if running_out || self.d[a] < lo - 1.0 || self.d[a] > hi + 1.0 {
                    // restart the other head at the far end of the window,
                    // at the spot where its signal lines up with this one
                    let target = if rate < 0.0 { hi - xf * (-rate) } else { lo + xf * rate };
                    let b = 1 - a;
                    self.d[b] = self.best_match(self.d[a], target.clamp(lo, hi), lo, hi);
                    self.fade = Some(0.0);
                }
            }
            let mut g = [0.0f64; 2];
            match self.fade {
                Some(f) => {
                    let t = 0.5 - 0.5 * (std::f64::consts::PI * f).cos();
                    g[self.active] = 1.0 - t;
                    g[1 - self.active] = t;
                    let nf = f + 1.0 / xf.max(1.0);
                    if nf >= 1.0 {
                        self.active = 1 - self.active;
                        self.fade = None;
                    } else {
                        self.fade = Some(nf);
                    }
                }
                None => g[self.active] = 1.0,
            }
            let mut wet = [0.0f64; 2];
            for h in 0..2 {
                if g[h] > 0.0 {
                    let d = self.d[h].clamp(1.0, self.lines[0].max_delay());
                    for c in 0..2 {
                        wet[c] += self.lines[c].read(d) * g[h];
                    }
                }
            }
            let mut out = [0.0f64; 2];
            for c in 0..2 {
                if self.tone_on {
                    wet[c] = self.tone[c].lp(wet[c]);
                }
                out[c] = (x[c] + mix * (wet[c] - x[c])) * o;
            }
            if stereo {
                chans[0][k] = out[0] as f32;
                chans[1][k] = out[1] as f32;
            } else {
                chans[0][k] = out[0] as f32;
            }
            peak = peak.max(out[0].abs()).max(out[1].abs());
        }
        meters[0] = peak as f32;
        // lamp: how far off normal speed we are (1 octave = full)
        meters[1] = (self.ratio.log2().abs()) as f32;
    }
}

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Normal Speed", values: &[] },
        Preset { name: "Sped-Up Take (+1 st)", values: &[(SPEED, 1.0)] },
        Preset { name: "Slowed Down (-1 st)", values: &[(SPEED, -1.0), (TONE, 12000.0)] },
        Preset { name: "Varispeed Double", values: &[(SPEED, 0.0), (FINE, 14.0), (MIX, 0.35), (WINDOW, 45.0)] },
        Preset { name: "Chipmunks (+1 octave)", values: &[(SPEED, 12.0), (WINDOW, 35.0)] },
        Preset { name: "Half Speed (-1 octave)", values: &[(SPEED, -12.0), (WINDOW, 90.0), (TONE, 9000.0)] },
        Preset { name: "Tape Stop", values: &[(RAMP, 1.5)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "VARISPEED",
        subtitle: ("TAPE SPEED", "CONTROL"),
        accent: ui::WALNUT,
        size: (W, H),
        items: vec![
            Item::Knob { p: SPEED, x: 130.0, y: 172.0, r: 50.0, label: "SPEED",
                tip: "Tape speed, in semitones. +12 = double speed (an octave up), -12 = half speed.", active: None },
            Item::Knob { p: FINE, x: 510.0, y: 172.0, r: 50.0, label: "FINE",
                tip: "Fine speed in cents (hundredths of a semitone). A few cents mixed with the dry signal thickens a voice.", active: None },
            Item::Meter { m: 0, x: 222.0, y: 92.0, w: 196.0, h: 136.0, kind: MeterKind::Vu, caption: "OUTPUT" },
            Item::Lamp { m: 1, x: 236.0, y: 254.0, label: "OFF SPEED", lo: 0.005, hi: 0.5 },
            Item::Switch { p: TRANSPORT, x: 400.0, y: 254.0, labels: ["RUN", "STOP"],
                tip: "STOP winds the tape down to a halt over the RAMP time; RUN spins it back up." },
            Item::Divider { y: 284.0 },
            Item::Knob { p: MIX, x: 120.0, y: 332.0, r: 22.0, label: "MIX", tip: "100% = varispeed only. Lower values blend in the original.", active: None },
            Item::Knob { p: RAMP, x: 220.0, y: 332.0, r: 22.0, label: "RAMP", tip: "How long the tape takes to change speed (and to stop/start).", active: None },
            Item::Knob { p: WINDOW, x: 320.0, y: 332.0, r: 22.0, label: "HEAD SIZE", tip: "Length of the rotating-head window. Shorter = tighter but grainier; longer = smoother but smeared.", active: None },
            Item::Knob { p: TONE, x: 420.0, y: 332.0, r: 22.0, label: "TONE", tip: "High cut on the varispeed signal. Fully right = off.", active: None },
            Item::Knob { p: OUTPUT, x: 520.0, y: 332.0, r: 22.0, label: "OUTPUT", tip: "Final level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, VarispeedParams, VarispeedDsp, panel,
    name: "Varispeed", class_id: *b"S2VarispeedDIY01", vendor: "PEQUOD Beatles",
    categories: [Fx, PitchShift]
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn dominant(x: &[f32], lo: f64, hi: f64) -> f64 {
        let fs = 48000.0;
        let mut best = (0.0, lo);
        let mut f = lo;
        while f <= hi {
            let (mut re, mut im) = (0.0, 0.0);
            for (k, v) in x.iter().enumerate() {
                let ph = 2.0 * PI * f * k as f64 / fs;
                re += *v as f64 * ph.cos();
                im += *v as f64 * ph.sin();
            }
            if re * re + im * im > best.0 {
                best = (re * re + im * im, f);
            }
            f += 1.0;
        }
        best.1
    }

    fn run(semis: f32, transport: f32) -> Vec<f32> {
        let fs = 48000.0;
        let mut d = VarispeedDsp::new(fs as f32, 1);
        d.set_params(&[semis, 0.0, 1.0, 20000.0, 60.0, 0.01, 0.0, transport]);
        let n = 48000;
        let mut x: Vec<f32> = (0..n).map(|k| (0.3 * (2.0 * PI * 440.0 * k as f64 / fs).sin()) as f32).collect();
        for c in x.chunks_mut(256) {
            d.process(&mut [c], &mut [0.0, 0.0]);
        }
        x
    }

    #[test]
    fn shifts_pitch_by_the_speed() {
        for (st, expect) in [(0.0, 440.0), (7.0, 659.3), (-12.0, 220.0), (12.0, 880.0)] {
            let y = run(st as f32, 0.0);
            let f = dominant(&y[24000..], expect * 0.85, expect * 1.15);
            println!("{st:+} st -> {f:.0} Hz (expect {expect:.0})");
            assert!((f / expect - 1.0).abs() < 0.01);
        }
    }

    #[test]
    fn unity_speed_is_transparent() {
        let y = run(0.0, 0.0);
        let tail = &y[30000..];
        let rms = (tail.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / tail.len() as f64).sqrt();
        let db = 20.0 * (rms / (0.3 / 2f64.sqrt())).log10();
        println!("unity gain {db:.2} dB");
        assert!(db.abs() < 0.5);
    }

    #[test]
    fn stop_winds_down() {
        let y = run(0.0, 1.0);
        let late: f64 = y[40000..].iter().map(|v| (*v as f64).abs()).sum::<f64>();
        assert!(y.iter().all(|v| v.is_finite()));
        // at 2% speed the tone is ~9 Hz and heavily windowed: far below input
        let f = dominant(&y[40000..], 2.0, 100.0);
        println!("stopped tape dominant {f:.0} Hz");
        assert!(f < 60.0);
        let _ = late;
    }
}

