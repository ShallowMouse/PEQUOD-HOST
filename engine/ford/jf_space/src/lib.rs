//! Ford Space Station: the Ursa Major Space Station (SST-282) in James Ford's rack, voiced for his records. A multi-tap delay with recirculation, 7 kHz bandwidth and coarse converters. Tap patterns estimated.

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
use vintage::fx::quantise;

pub const W: u32 = 640;
pub const H: u32 = 432;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Mode { #[name = "Room"] Room, #[name = "Hall"] Hall, #[name = "Plate"] Plate, #[name = "Multi-Echo A"] EchoA, #[name = "Multi-Echo B"] EchoB, #[name = "Slap"] Slap, #[name = "Flat (gated)"] Flat, #[name = "Rising"] Rising }

// tap patterns: (fraction of DELAY, gain) per mode (estimated; the real SST-282 tap tables were never published)
const MODES: [[(f64, f64); 8]; 8] = [
    // Room
    [(0.11, 0.9), (0.19, 0.8), (0.27, 0.75), (0.36, 0.7), (0.47, 0.65), (0.6, 0.6), (0.77, 0.55), (1.0, 0.5)],
    // Hall
    [(0.21, 0.7), (0.29, 0.7), (0.41, 0.68), (0.53, 0.66), (0.62, 0.64), (0.74, 0.62), (0.88, 0.6), (1.0, 0.58)],
    // Plate-ish (dense, even)
    [(0.05, 0.8), (0.09, 0.8), (0.14, 0.78), (0.2, 0.76), (0.27, 0.74), (0.35, 0.72), (0.45, 0.7), (0.57, 0.68)],
    // Multi-echo A (even)
    [(0.125, 0.9), (0.25, 0.8), (0.375, 0.7), (0.5, 0.6), (0.625, 0.5), (0.75, 0.4), (0.875, 0.3), (1.0, 0.2)],
    // Multi-echo B (accelerating)
    [(0.4, 0.9), (0.62, 0.8), (0.76, 0.7), (0.85, 0.6), (0.91, 0.5), (0.95, 0.4), (0.98, 0.3), (1.0, 0.2)],
    // Slap
    [(1.0, 1.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0)],
    // Gated-ish (flat then stop)
    [(0.1, 0.8), (0.22, 0.8), (0.34, 0.8), (0.46, 0.8), (0.58, 0.8), (0.7, 0.8), (0.82, 0.8), (0.94, 0.8)],
    // Reverse (rising)
    [(0.12, 0.2), (0.24, 0.3), (0.36, 0.4), (0.48, 0.5), (0.6, 0.6), (0.72, 0.7), (0.84, 0.8), (0.96, 0.9)],
];


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "mode"]
    pub mode: EnumParam<Mode>,
    #[id = "delay"]
    pub delay: FloatParam,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "reverb"]
    pub reverb: FloatParam,
    #[id = "echo"]
    pub echo: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            mode: EnumParam::new("Program", Mode::Room),
            delay: ms("Delay", 120.0, 5.0, 255.0),
            decay: pct("Decay", 0.55),
            reverb: pct("Reverb Level", 0.7),
            echo: pct("Echo Level", 0.3),
            mix: pct("Mix", 0.3),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const MODE: usize = 0;
pub const DELAY: usize = 1;
pub const DECAY: usize = 2;
pub const REVERB: usize = 3;
pub const ECHO: usize = 4;
pub const MIX: usize = 5;
pub const OUTPUT: usize = 6;
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
            self.mode.as_ptr(),
            self.delay.as_ptr(),
            self.decay.as_ptr(),
            self.reverb.as_ptr(),
            self.echo.as_ptr(),
            self.mix.as_ptr(),
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

pub struct Dsp {
    fs: f64,
    line: DelayLine,
    rec: [DelayLine; 2],
    aa: [Biquad; 2],
    rf: [OnePole; 2],
    mode: usize,
    delay: Smooth,
    decay: f64,
    reverb: f64,
    echo: f64,
    fb: [f64; 2],
    mix: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut aa = [Biquad::identity(); 2];
        // ~7 kHz bandwidth (early digital reverb)
        aa[0].set_lowpass(fs, 7000.0, 0.54);
        aa[1].set_lowpass(fs, 7000.0, 1.31);
        let mut rf = [OnePole::default(); 2];
        for r in &mut rf {
            r.set_lowpass(fs, 4500.0);
        }
        Self {
            fs,
            line: DelayLine::new((fs * 0.3) as usize),
            rec: [DelayLine::new((fs * 0.2) as usize), DelayLine::new((fs * 0.2) as usize)],
            aa,
            rf,
            mode: 0,
            delay: Smooth::new(fs, 120.0, 120.0),
            decay: 0.55,
            reverb: 0.7,
            echo: 0.3,
            fb: [0.0; 2],
            mix: Smooth::new(fs, 20.0, 0.3),
            out: Smooth::new(fs, 20.0, 1.0),
        }
    }
    fn reset(&mut self) {
        self.line.reset();
        self.rec.iter_mut().for_each(|r| r.reset());
        self.aa.iter_mut().for_each(|b| b.reset());
        self.rf.iter_mut().for_each(|b| b.reset());
        self.fb = [0.0; 2];
    }
    fn set_params(&mut self, v: &[f32]) {
        self.mode = (v[MODE].round() as usize).min(7);
        self.delay.set(v[DELAY] as f64);
        self.decay = v[DECAY] as f64 * 0.93;
        self.reverb = v[REVERB] as f64;
        self.echo = v[ECHO] as f64;
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let mut peak = 0.0f64;
        let taps = MODES[self.mode];
        // recirculation lengths (ms) for the two reverb loops
        let rlen = [61.3 * 0.001 * self.fs, 79.7 * 0.001 * self.fs];
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            let a = cascade(&mut self.aa, x);
            // 12-bit converters (estimated), a little headroom
            let a = quantise(a * 0.5, 12) * 2.0;
            self.line.write(a);
            let span = self.delay.next() * 0.001 * self.fs;
            let mut e = [0.0; 2];
            for (i, (f, g)) in taps.iter().enumerate() {
                if *g == 0.0 {
                    continue;
                }
                let v = self.line.read((f * span).max(1.0)) * g;
                e[i % 2] += v;
            }
            // reverb: taps recirculated through two lossy loops
            let mut r = [0.0; 2];
            for c in 0..2 {
                let fbv = self.rec[c].read(rlen[c]);
                let inp = e[c] * 0.35 + self.rf[c].lp(fbv) * self.decay - self.fb[1 - c] * 0.25 * self.decay;
                self.rec[c].write(inp);
                r[c] = fbv;
            }
            self.fb = r;
            let mx = self.mix.next();
            let o = self.out.next();
            let wl = e[0] * self.echo * 0.5 + r[0] * self.reverb;
            let wr = e[1] * self.echo * 0.5 + r[1] * self.reverb;
            for (ci, c) in chans.iter_mut().enumerate() {
                let w = if nch == 1 { 0.5 * (wl + wr) } else if ci == 0 { wl } else { wr };
                let y = (c[k] as f64 * (1.0 - mx) + w * mx) * o;
                peak = peak.max(y.abs());
                c[k] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Lo-Fi Room on Drums", values: &[(MODE, 0.0), (DELAY, 70.0), (DECAY, 0.45), (MIX, 0.3)] },
        Preset { name: "Vocal Space", values: &[(MODE, 1.0), (DELAY, 150.0), (DECAY, 0.55), (MIX, 0.25)] },
        Preset { name: "Grainy Plate", values: &[(MODE, 2.0), (DELAY, 90.0), (DECAY, 0.65), (MIX, 0.35)] },
        Preset { name: "Synth Multi-Echo", values: &[(MODE, 3.0), (DELAY, 255.0), (DECAY, 0.35), (ECHO, 0.8), (REVERB, 0.4), (MIX, 0.4)] },
        Preset { name: "Ringy Karplus Delay", values: &[(MODE, 5.0), (DELAY, 8.0), (DECAY, 0.9), (ECHO, 0.6), (REVERB, 0.6), (MIX, 0.35)] },
        Preset { name: "Rising Tail", values: &[(MODE, 7.0), (DELAY, 200.0), (DECAY, 0.5), (MIX, 0.35)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "SPACE STATION",
        subtitle: ("AFTER THE URSA MAJOR SST-282", "DIGITAL REVERBERATOR · 1978"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![

            Item::RadioList { p: MODE, x: 34.0, y: 84.0, w: 150.0, row_h: 21.0, names: &["Room", "Hall", "Plate", "Multi-Echo A", "Multi-Echo B", "Slap", "Flat (gated)", "Rising"], tip: "Tap pattern program (the SST-282's multi-tap delay structure; patterns estimated)." },
            Item::Knob { p: DELAY, x: 280.0, y: 160.0, r: 40.0, label: "DELAY", tip: "Overall span of the taps, 5 to 255 ms: the size of the space.", active: None },
            Item::Knob { p: DECAY, x: 410.0, y: 160.0, r: 40.0, label: "DECAY", tip: "How much is fed back: the length of the tail.", active: None },
            Item::Lamp { m: 0, x: 530.0, y: 160.0, label: "LEVEL", lo: 0.05, hi: 0.9 },
            Item::Divider { y: 264.0 },
            Item::Knob { p: REVERB, x: 90.0, y: 330.0, r: 22.0, label: "REVERB", tip: "Level of the recirculated reverb.", active: None },
            Item::Knob { p: ECHO, x: 200.0, y: 330.0, r: 22.0, label: "ECHO", tip: "Level of the first taps (the multi-echo).", active: None },
            Item::Knob { p: MIX, x: 380.0, y: 330.0, r: 22.0, label: "MIX", tip: "Dry vs processed.", active: None },
            Item::Knob { p: OUTPUT, x: 540.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Ford Space Station", class_id: *b"JFSpaceStationPQ", vendor: "PEQUOD James Ford",
    categories: [Fx, Reverb]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    pub fn run(p: &[f32], input: &mut [Vec<f32>]) -> Vec<f32> {
        let mut d = Dsp::new(48000.0, input.len());
        d.set_params(p);
        d.reset();
        let mut m = vec![0.0f32; Dsp::METERS.max(1)];
        let n = input[0].len();
        let mut k = 0;
        while k < n {
            let e = (k + 512).min(n);
            d.set_params(p);
            let mut chans: Vec<&mut [f32]> = input.iter_mut().map(|c| &mut c[k..e]).collect();
            d.process(&mut chans, &mut m);
            k = e;
        }
        m
    }
    pub fn sine(f: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n).map(|k| (amp * (2.0 * std::f64::consts::PI * f * k as f64 / 48000.0).sin()) as f32).collect()
    }
    pub fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
    }
    pub fn db(x: f64) -> f64 {
        20.0 * x.max(1e-12).log10()
    }
    pub fn with(mut p: Vec<f32>, set: &[(usize, f32)]) -> Vec<f32> {
        for (i, v) in set {
            p[*i] = *v;
        }
        p
    }
    /// Every preset runs finite and bounded on noise bursts.
    #[test]
    fn presets_run_clean() {
        let base = defaults();
        let mut all = vec![base.clone()];
        for pr in presets() {
            all.push(with(base.clone(), pr.values));
        }
        for p in all {
            let mut nz = vintage::dsp::Noise::new(7);
            let n = 48000;
            let mut l: Vec<f32> = (0..n).map(|k| if (k / 6000) % 2 == 0 { (nz.next() * 0.5) as f32 } else { 0.0 }).collect();
            let mut r = l.clone();
            {
                let mut input = vec![std::mem::take(&mut l), std::mem::take(&mut r)];
                run(&p, &mut input);
                assert!(input.iter().flatten().all(|v| v.is_finite() && v.abs() < 8.0), "unbounded output for {:?}", p);
            }
        }
    }

    #[test]
    fn makes_a_tail_and_is_band_limited() {
        let p = with(defaults(), &[(MIX, 1.0), (DECAY, 0.8)]);
        let mut x = vec![0.0f32; 96000];
        x[0] = 0.5;
        let mut input = vec![x];
        run(&p, &mut input);
        let early = rms(&input[0][..9600]);
        let late = rms(&input[0][24000..48000]);
        println!("early {:.1} late {:.1} dB", db(early), db(late));
        assert!(late > 1e-6 && late < early);
        let hi = |f: f64| {
            let p = with(defaults(), &[(MIX, 1.0)]);
            let mut input = vec![sine(f, 0.2, 24000)];
            run(&p, &mut input);
            rms(&input[0][12000..])
        };
        assert!(db(hi(14000.0) / hi(1000.0)) < -20.0);
    }

}
