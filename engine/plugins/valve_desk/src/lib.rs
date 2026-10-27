//! Valve Desk — valve mixing-console channel preamp.
//!
//! Recreates what running a track through an early-60s valve desk did: an
//! input transformer that thickens and gently saturates the low end, a
//! valve gain stage that adds warm, mostly 2nd-harmonic distortion as it's
//! pushed, and the limited bandwidth of the whole chain.

use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
use vintage::dsp::*;
use vintage::nih_plug_egui::EguiState;
use vintage::params::*;
use vintage::ui::{self, Preset, Item, MeterKind, Panel};
use vintage::{VintageDsp, VintageParams};

const W: u32 = 640;
const H: u32 = 432;

#[derive(Params)]
pub struct DeskParams {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "drive"]
    drive: FloatParam,
    #[id = "iron"]
    iron: FloatParam,
    #[id = "bias"]
    bias: FloatParam,
    #[id = "weight"]
    weight: FloatParam,
    #[id = "bandwidth"]
    bandwidth: FloatParam,
    #[id = "mix"]
    mix: FloatParam,
    #[id = "output"]
    output: FloatParam,
}

impl Default for DeskParams {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            drive: db("Drive", 8.0, 0.0, 30.0),
            iron: pct("Iron", 0.35),
            bias: pct("Valve Bias", 0.35),
            weight: db("Weight", 1.5, -6.0, 6.0),
            bandwidth: hz("Bandwidth", 16000.0, 5000.0, 22000.0),
            mix: pct("Mix", 1.0),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

/// Scale into the valve curve: lower = more headroom before it saturates.
const HEADROOM: f64 = 0.4;

const DRIVE: usize = 0;
const IRON: usize = 1;
const BIAS: usize = 2;
const WEIGHT: usize = 3;
const BANDWIDTH: usize = 4;
const MIX: usize = 5;
const OUTPUT: usize = 6;

impl VintageParams for DeskParams {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.drive.as_ptr(),
            self.iron.as_ptr(),
            self.bias.as_ptr(),
            self.weight.as_ptr(),
            self.bandwidth.as_ptr(),
            self.mix.as_ptr(),
            self.output.as_ptr(),
        ]
    }
}

#[derive(Clone)]
struct Chan {
    weight: Biquad,
    xfmr_lp: OnePole,
    os: Oversampler,
    dc: DcBlock,
    bw1: Biquad,
    bw2: Biquad,
    lowend: Biquad,
    dry: FixedDelay,
}

pub struct DeskDsp {
    fs: f64,
    ch: Vec<Chan>,
    drive: Smooth,
    iron: Smooth,
    bias: Smooth,
    mix: Smooth,
    out: Smooth,
    last: Option<(f32, f32)>,
}

impl VintageDsp for DeskDsp {
    const METERS: usize = 2;

    fn new(sample_rate: f32, channels: usize) -> Self {
        let fs = sample_rate as f64;
        let mut lowend = Biquad::default();
        lowend.set_highpass(fs, 22.0, 0.6); // coupling caps / transformer low limit
        let mut xfmr_lp = OnePole::default();
        xfmr_lp.set_lowpass(fs, 150.0);
        let c = Chan {
            weight: Biquad::identity(),
            xfmr_lp,
            os: Oversampler::new(),
            dc: DcBlock::new(fs),
            bw1: Biquad::identity(),
            bw2: Biquad::identity(),
            lowend,
            dry: FixedDelay::new(OS_LATENCY),
        };
        Self {
            fs,
            ch: vec![c; channels.max(1)],
            drive: Smooth::new(fs, 30.0, 2.5),
            iron: Smooth::new(fs, 30.0, 0.35),
            bias: Smooth::new(fs, 30.0, 0.2),
            mix: Smooth::new(fs, 20.0, 1.0),
            out: Smooth::new(fs, 20.0, 1.0),
            last: None,
        }
    }

    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }

    fn reset(&mut self) {
        for c in &mut self.ch {
            c.weight.reset();
            c.xfmr_lp.reset();
            c.os.reset();
            c.dc.reset();
            c.bw1.reset();
            c.bw2.reset();
            c.lowend.reset();
            c.dry.reset();
        }
    }

    fn set_params(&mut self, v: &[f32]) {
        self.drive.set(db_to_gain(v[DRIVE] as f64));
        self.iron.set(v[IRON] as f64);
        self.bias.set(v[BIAS] as f64 * 0.7);
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
        let key = (v[WEIGHT], v[BANDWIDTH]);
        if self.last != Some(key) {
            for c in &mut self.ch {
                c.weight.set_low_shelf(self.fs, 90.0, 0.8, v[WEIGHT] as f64);
                c.bw1.set_lowpass(self.fs, v[BANDWIDTH] as f64, 0.54);
                c.bw2.set_lowpass(self.fs, v[BANDWIDTH] as f64 * 1.3, 1.3);
            }
            self.last = Some(key);
        }
    }

    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(self.ch.len());
        let mut peak = 0.0f64;
        let mut sat = 0.0f64;
        for k in 0..n {
            let drive = self.drive.next();
            let iron = self.iron.next();
            let bias = self.bias.next();
            let mix = self.mix.next();
            let o = self.out.next();
            // a real desk gets louder as you push it; keep ~70% of the gain
            let comp = drive.powf(-0.7);
            for c in 0..nch {
                let st = &mut self.ch[c];
                let x = chans[c][k] as f64;
                let mut y = st.weight.process(x);
                // input transformer: the low band saturates first
                let low = st.xfmr_lp.lp(y);
                let a = 1.0 + 6.0 * iron;
                y = y - low + (a * low * 1.4).tanh() / (a * 1.4) * (1.0 + 0.6 * iron);
                // valve gain stage
                // the valve stage has ~8 dB of headroom at 0 dB drive
                y = st.os.process(y, |v| {
                    let d = v * drive * HEADROOM;
                    sat = sat.max(d.abs());
                    tube(d, bias) / HEADROOM
                }) * comp;
                y = st.dc.process(y);
                y = st.bw2.process(st.bw1.process(y));
                y = st.lowend.process(y);
                let dry = st.dry.process(x);
                let out = (dry + mix * (y - dry)) * o;
                peak = peak.max(out.abs());
                chans[c][k] = out as f32;
            }
        }
        meters[0] = peak as f32;
        meters[1] = sat as f32;
    }
}

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Clean Desk", values: &[(DRIVE, 3.0), (IRON, 0.2)] },
        Preset { name: "Warm Bus", values: &[(DRIVE, 8.0), (IRON, 0.45), (WEIGHT, 2.0), (BANDWIDTH, 15000.0)] },
        Preset { name: "Hot Channel", values: &[(DRIVE, 18.0), (IRON, 0.5), (BIAS, 0.5), (OUTPUT, -4.0)] },
        Preset { name: "Overloaded Desk Guitar", values: &[(DRIVE, 28.0), (IRON, 0.7), (BIAS, 0.6), (BANDWIDTH, 9000.0), (OUTPUT, -10.0)] },
        Preset { name: "Thick Bass DI", values: &[(DRIVE, 12.0), (IRON, 0.8), (WEIGHT, 3.5), (BANDWIDTH, 8000.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "VALVE DESK",
        subtitle: ("VALVE CONSOLE", "CHANNEL PREAMP"),
        accent: ui::STEEL,
        size: (W, H),
        items: vec![
            Item::Knob { p: DRIVE, x: 130.0, y: 172.0, r: 50.0, label: "DRIVE",
                tip: "Gain into the valve stage. Low = subtle warmth; high = thick, saturated desk overload. The output gets louder too, as on a real desk.", active: None },
            Item::Knob { p: IRON, x: 510.0, y: 172.0, r: 50.0, label: "IRON",
                tip: "Input transformer: thickens and softly saturates the low end.", active: None },
            Item::Meter { m: 0, x: 222.0, y: 92.0, w: 196.0, h: 136.0, kind: MeterKind::Vu, caption: "OUTPUT" },
            Item::Lamp { m: 1, x: 296.0, y: 254.0, label: "OVERLOAD", lo: 0.8, hi: 2.0 },
            Item::Divider { y: 284.0 },
            Item::Knob { p: BIAS, x: 120.0, y: 332.0, r: 22.0, label: "VALVE BIAS", tip: "Asymmetry of the valve: more = warmer, more 2nd harmonic.", active: None },
            Item::Knob { p: WEIGHT, x: 220.0, y: 332.0, r: 22.0, label: "WEIGHT", tip: "Low shelf at 90 Hz before the transformer.", active: None },
            Item::Knob { p: BANDWIDTH, x: 320.0, y: 332.0, r: 22.0, label: "BANDWIDTH", tip: "Top-end limit of the desk.", active: None },
            Item::Knob { p: MIX, x: 420.0, y: 332.0, r: 22.0, label: "MIX", tip: "Blend with the dry signal (latency compensated).", active: None },
            Item::Knob { p: OUTPUT, x: 520.0, y: 332.0, r: 22.0, label: "OUTPUT", tip: "Final level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, DeskParams, DeskDsp, panel,
    name: "Valve Desk", class_id: *b"S2ValveDeskDIY01", vendor: "PEQUOD Beatles",
    categories: [Fx, Distortion]
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn thd(amp: f64, drive_db: f32) -> f64 {
        let fs = 48000.0;
        let mut d = DeskDsp::new(fs as f32, 1);
        d.set_params(&[drive_db, 0.35, 0.35, 0.0, 22000.0, 1.0, 0.0]);
        let n = 48000;
        let mut x: Vec<f32> = (0..n).map(|k| (amp * (2.0 * PI * 500.0 * k as f64 / fs).sin()) as f32).collect();
        let mut m = [0.0f32; 2];
        d.process(&mut [&mut x[..]], &mut m);
        let tail = &x[n / 2..];
        let mag = |h: f64| {
            let (mut re, mut im) = (0.0, 0.0);
            for (k, v) in tail.iter().enumerate() {
                let ph = 2.0 * PI * 500.0 * h * k as f64 / fs;
                re += *v as f64 * ph.cos();
                im += *v as f64 * ph.sin();
            }
            (re * re + im * im).sqrt()
        };
        let f = mag(1.0);
        (2..=6).map(|h| mag(h as f64).powi(2)).sum::<f64>().sqrt() / f
    }

    #[test]
    fn more_drive_more_harmonics() {
        let low = thd(0.25, 2.0);
        let high = thd(0.25, 24.0);
        println!("THD drive 2 dB {:.2}%, 24 dB {:.2}%", low * 100.0, high * 100.0);
        assert!(high > low * 4.0);
        assert!(low < 0.03);
    }
}
