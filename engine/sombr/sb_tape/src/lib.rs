//! A800 Bus: tape on the master bus, as the Studer A800 tagged on the 12 to 12 mix. Thickens and rounds the top of a bedroom recording.

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
pub enum TapeSpeed {
    #[name = "7.5 ips"]
    Slow,
    #[name = "15 ips"]
    Standard,
    #[name = "30 ips"]
    Fast,
}

#[derive(Params)]
pub struct TapeParams {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "input"]
    input: FloatParam,
    #[id = "speed"]
    speed: EnumParam<TapeSpeed>,
    #[id = "bias"]
    bias: FloatParam,
    #[id = "bump"]
    bump: FloatParam,
    #[id = "wow"]
    wow: FloatParam,
    #[id = "hiss"]
    hiss: FloatParam,
    #[id = "output"]
    output: FloatParam,
}

impl Default for TapeParams {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            input: db("Input", 0.0, -12.0, 18.0),
            speed: EnumParam::new("Tape Speed", TapeSpeed::Standard),
            bias: pct("Bias", 0.5),
            bump: pct("Head Bump", 0.5),
            wow: pct("Wow & Flutter", 0.15),
            hiss: pct("Hiss", 0.1),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

const INPUT: usize = 0;
const SPEED: usize = 1;
const BIAS: usize = 2;
const BUMP: usize = 3;
const WOW: usize = 4;
const HISS: usize = 5;
const OUTPUT: usize = 6;

impl VintageParams for TapeParams {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.input.as_ptr(),
            self.speed.as_ptr(),
            self.bias.as_ptr(),
            self.bump.as_ptr(),
            self.wow.as_ptr(),
            self.hiss.as_ptr(),
            self.output.as_ptr(),
        ]
    }
}

/// Fixed transport delay so wow/flutter can move both ways around it.
const TRANSPORT_MS: f64 = 4.0;

#[derive(Clone)]
struct Chan {
    pre: Biquad,
    os: Oversampler,
    de: Biquad,
    bump: Biquad,
    sub: Biquad,
    hf1: Biquad,
    hf2: Biquad,
    line: DelayLine,
    hiss_bp: Biquad,
    noise: Noise,
}

pub struct TapeDsp {
    fs: f64,
    ch: Vec<Chan>,
    input: Smooth,
    out: Smooth,
    wow_amt: Smooth,
    hiss: Smooth,
    wow: Wander,
    wow_lfo: Lfo,
    flutter: Lfo,
    flutter2: Lfo,
    sat_drive: f64,
    last: Option<(f32, f32, f32)>,
}

impl TapeDsp {
    fn speed_profile(speed: usize) -> (f64, f64, f64) {
        // (head bump Hz, HF limit Hz, pre-emphasis dB)
        match speed {
            0 => (55.0, 11000.0, 9.0),
            1 => (90.0, 17000.0, 7.0),
            _ => (140.0, 21000.0, 5.0),
        }
    }
}

impl VintageDsp for TapeDsp {
    const METERS: usize = 2;

    fn new(sample_rate: f32, channels: usize) -> Self {
        let fs = sample_rate as f64;
        let mut sub = Biquad::default();
        sub.set_highpass(fs, 22.0, 0.7);
        let mut hiss_bp = Biquad::default();
        hiss_bp.set_bandpass(fs, 6000.0, 0.4);
        let mk = |seed: u32| Chan {
            pre: Biquad::identity(),
            os: Oversampler::new(),
            de: Biquad::identity(),
            bump: Biquad::identity(),
            sub,
            hf1: Biquad::identity(),
            hf2: Biquad::identity(),
            line: DelayLine::new((fs * 0.02) as usize),
            hiss_bp,
            noise: Noise::new(seed),
        };
        Self {
            fs,
            ch: (0..channels.max(1)).map(|c| mk(17 + c as u32 * 101)).collect(),
            input: Smooth::new(fs, 30.0, 1.0),
            out: Smooth::new(fs, 30.0, 1.0),
            wow_amt: Smooth::new(fs, 30.0, 0.15),
            hiss: Smooth::new(fs, 30.0, 0.0),
            wow: Wander::new(fs, 0.6, 9),
            wow_lfo: Lfo::default(),
            flutter: Lfo::default(),
            flutter2: Lfo { phase: 0.37 },
            sat_drive: 1.6,
            last: None,
        }
    }

    fn latency(&self) -> u32 {
        OS_LATENCY as u32 + (TRANSPORT_MS * 0.001 * self.fs).round() as u32
    }

    fn reset(&mut self) {
        for c in &mut self.ch {
            c.pre.reset();
            c.os.reset();
            c.de.reset();
            c.bump.reset();
            c.sub.reset();
            c.hf1.reset();
            c.hf2.reset();
            c.line.reset();
            c.hiss_bp.reset();
        }
    }

    fn set_params(&mut self, v: &[f32]) {
        self.input.set(db_to_gain(v[INPUT] as f64));
        self.out.set(db_to_gain(v[OUTPUT] as f64));
        self.wow_amt.set(v[WOW] as f64);
        // hiss: 0 = off, 100% = about -48 dBFS
        let h = v[HISS] as f64;
        self.hiss.set(if h <= 0.0 { 0.0 } else { db_to_gain(-80.0 + 32.0 * h) });
        let speed = (v[SPEED] as f64).round() as usize;
        let bias = v[BIAS] as f64;
        // under-bias (low) = brighter but grittier; over-bias = cleaner, darker
        self.sat_drive = 1.6 * (1.0 + 0.6 * (0.5 - bias));
        let key = (v[SPEED], v[BIAS], v[BUMP]);
        if self.last != Some(key) {
            let (bump_hz, hf, emph) = Self::speed_profile(speed);
            let hf = hf * (1.0 + 0.35 * (0.5 - bias));
            let bump_db = 5.0 * v[BUMP] as f64;
            for c in &mut self.ch {
                c.pre.set_high_shelf(self.fs, 3000.0, 0.7, emph);
                c.de.set_high_shelf(self.fs, 3000.0, 0.7, -emph);
                c.bump.set_peak(self.fs, bump_hz, 1.1, bump_db);
                c.hf1.set_lowpass(self.fs, hf, 0.54);
                c.hf2.set_lowpass(self.fs, hf * 1.25, 1.3);
            }
            self.last = Some(key);
        }
    }

    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(self.ch.len());
        let fs = self.fs;
        let base = TRANSPORT_MS * 0.001 * fs;
        let sd = self.sat_drive;
        let mut in_peak = 0.0f64;
        let mut sat = 0.0f64;
        for k in 0..n {
            let g = self.input.next();
            let o = self.out.next();
            let wa = self.wow_amt.next();
            let hiss = self.hiss.next();
            // shared transport: both channels wobble together
            let wow = 0.7 * self.wow.next() + 0.3 * self.wow_lfo.next(fs, 0.55);
            let flut = 0.6 * self.flutter.next(fs, 6.3) + 0.4 * self.flutter2.next(fs, 11.7);
            let d = base * (1.0 + wa * (0.35 * wow + 0.04 * flut));
            for c in 0..nch {
                let st = &mut self.ch[c];
                let x = chans[c][k] as f64 * g;
                in_peak = in_peak.max(x.abs());
                let mut y = st.pre.process(x);
                y = st.os.process(y, |v| {
                    sat = sat.max((v * sd).abs());
                    tape_curve(v * sd) / sd
                });
                y = st.de.process(y);
                y = st.bump.process(y);
                y = st.sub.process(y);
                y = st.hf2.process(st.hf1.process(y));
                st.line.write(y);
                y = st.line.read(d);
                if hiss > 0.0 {
                    y += st.hiss_bp.process(st.noise.next()) * hiss * 3.0;
                }
                chans[c][k] = (y * o) as f32;
            }
        }
        meters[0] = in_peak as f32;
        meters[1] = sat as f32;
    }
}

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Master Bus, 30 ips", values: &[(SPEED, 2.0), (INPUT, 3.0), (WOW, 0.03), (HISS, 0.0), (BUMP, 0.35)] },
        Preset { name: "Master Bus, 15 ips Warm", values: &[(SPEED, 1.0), (INPUT, 4.0), (WOW, 0.05), (HISS, 0.02)] },
        Preset { name: "Hit the Tape (bass)", values: &[(SPEED, 1.0), (INPUT, 9.0), (BUMP, 0.7), (OUTPUT, -5.0)] },
        Preset { name: "Bedroom Demo Wobble", values: &[(SPEED, 0.0), (INPUT, 5.0), (WOW, 0.45), (HISS, 0.25)] },
        Preset { name: "Drum Bus Crunch", values: &[(SPEED, 1.0), (INPUT, 11.0), (OUTPUT, -7.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "A800 BUS",
        subtitle: ("TAPE ON THE MASTER BUS", "AFTER THE STUDER A800"),
        accent: ui::WALNUT,
        size: (W, H),
        items: vec![
            Item::Knob { p: INPUT, x: 130.0, y: 172.0, r: 50.0, label: "INPUT",
                tip: "Record level. Hit the tape harder for more saturation and soft compression. The meter shows the level going onto tape.", active: None },
            Item::Knob { p: OUTPUT, x: 510.0, y: 172.0, r: 50.0, label: "OUTPUT",
                tip: "Playback level.", active: None },
            Item::Meter { m: 0, x: 222.0, y: 92.0, w: 196.0, h: 136.0, kind: MeterKind::Vu, caption: "RECORD LEVEL" },
            Item::Lamp { m: 1, x: 296.0, y: 254.0, label: "SATURATE", lo: 0.7, hi: 1.6 },
            Item::Divider { y: 284.0 },
            Item::Selector { p: SPEED, x: 110.0, y: 338.0, label: "SPEED", positions: &["7.5", "15", "30"],
                tip: "Tape speed in inches per second. 7.5 = darker, bigger low-end bump, more saturation. 15 = the classic studio speed. 30 = cleanest and widest." },
            Item::Knob { p: BIAS, x: 220.0, y: 332.0, r: 22.0, label: "BIAS", tip: "Recording bias. Below 50% = brighter but grittier; above = cleaner and darker.", active: None },
            Item::Knob { p: BUMP, x: 320.0, y: 332.0, r: 22.0, label: "HEAD BUMP", tip: "The low-end lift from the playback head. Its frequency follows the tape speed.", active: None },
            Item::Knob { p: WOW, x: 420.0, y: 332.0, r: 22.0, label: "WOW/FLUTTER", tip: "Speed wavering of the transport.", active: None },
            Item::Knob { p: HISS, x: 520.0, y: 332.0, r: 22.0, label: "HISS", tip: "Tape noise. 0 = off.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, TapeParams, TapeDsp, panel,
    name: "A800 Bus", class_id: *b"SBA800BusTapePQ1", vendor: "PEQUOD sombr",
    categories: [Fx, Distortion]
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn level_and_thd(amp: f64, input_db: f32, speed: f32) -> (f64, f64) {
        let fs = 48000.0;
        let mut d = TapeDsp::new(fs as f32, 1);
        d.set_params(&[input_db, speed, 0.5, 0.5, 0.0, 0.0, 0.0]);
        let n = 48000;
        let f0 = 1000.0;
        let mut x: Vec<f32> = (0..n).map(|k| (amp * (2.0 * PI * f0 * k as f64 / fs).sin()) as f32).collect();
        let mut m = [0.0f32; 2];
        d.process(&mut [&mut x[..]], &mut m);
        let tail = &x[n / 2..];
        let mag = |h: f64| {
            let (mut re, mut im) = (0.0, 0.0);
            for (k, v) in tail.iter().enumerate() {
                let ph = 2.0 * PI * f0 * h * k as f64 / fs;
                re += *v as f64 * ph.cos();
                im += *v as f64 * ph.sin();
            }
            (re * re + im * im).sqrt()
        };
        let f = mag(1.0);
        let rms = (tail.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / tail.len() as f64).sqrt();
        (rms, (2..=7).map(|h| mag(h as f64).powi(2)).sum::<f64>().sqrt() / f)
    }

    #[test]
    fn clean_at_low_level_saturates_when_hot() {
        let (rms, thd_low) = level_and_thd(0.05, 0.0, 1.0);
        let (_, thd_hot) = level_and_thd(0.05, 18.0, 1.0);
        let gain_db = 20.0 * (rms / (0.05 / 2f64.sqrt())).log10();
        println!("unity gain {gain_db:.2} dB, THD low {:.2}%, hot {:.2}%", thd_low * 100.0, thd_hot * 100.0);
        assert!(gain_db.abs() < 1.5);
        assert!(thd_low < 0.01);
        assert!(thd_hot > thd_low * 5.0);
    }

    #[test]
    fn slower_tape_is_darker() {
        let fs = 48000.0;
        let energy_above = |speed: f32| {
            let mut d = TapeDsp::new(fs as f32, 1);
            d.set_params(&[0.0, speed, 0.5, 0.5, 0.0, 0.0, 0.0]);
            let n = 48000;
            let mut x: Vec<f32> = (0..n).map(|k| (0.05 * (2.0 * PI * 12000.0 * k as f64 / fs).sin()) as f32).collect();
            let mut m = [0.0f32; 2];
            d.process(&mut [&mut x[..]], &mut m);
            x[n / 2..].iter().map(|v| (*v as f64).powi(2)).sum::<f64>()
        };
        assert!(energy_above(0.0) < energy_above(2.0) * 0.5);
    }
}
