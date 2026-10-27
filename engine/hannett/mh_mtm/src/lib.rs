//! Time Modulator: after the Marshall Time Modulator, the analogue delay Hannett used alongside the AMS. A companded bucket-brigade line swept by an LFO, by random wander or by the input's own envelope, with regeneration: doubling, chorus, deep flanging and odd warps. Modelled from its published behaviour; internals estimated.

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
use vintage::fx::{Bbd, Compander};

pub const W: u32 = 640;
pub const H: u32 = 432;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Wave { #[name = "Sine"] Sine, #[name = "Triangle"] Tri, #[name = "Random"] Random, #[name = "Envelope"] Env }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "delay"]
    pub delay: FloatParam,
    #[id = "depth"]
    pub depth: FloatParam,
    #[id = "rate"]
    pub rate_: FloatParam,
    #[id = "wave"]
    pub wave: EnumParam<Wave>,
    #[id = "regen"]
    pub regen: FloatParam,
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
            delay: ms("Delay", 6.0, 0.5, 50.0),
            depth: pct("Depth", 0.5),
            rate_: rate("Rate", 0.3, 0.02, 12.0),
            wave: EnumParam::new("Modulation", Wave::Sine),
            regen: FloatParam::new("Regeneration", 0.4, FloatRange::Linear { min: -0.95, max: 0.95 }).with_step_size(0.01),
            mix: pct("Mix", 0.5),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const DELAY: usize = 0;
pub const DEPTH: usize = 1;
pub const RATE: usize = 2;
pub const WAVE: usize = 3;
pub const REGEN: usize = 4;
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
            self.delay.as_ptr(),
            self.depth.as_ptr(),
            self.rate_.as_ptr(),
            self.wave.as_ptr(),
            self.regen.as_ptr(),
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
    bbd: [Bbd; 2],
    comp: [Compander; 2],
    lfo: [f64; 2],
    wander: [Wander; 2],
    env: Envelope,
    fb: [f64; 2],
    delay: Smooth,
    depth: f64,
    rate: f64,
    wave: usize,
    regen: f64,
    mix: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut bbd = [Bbd::new(fs, 110.0), Bbd::new(fs, 110.0)];
        for b in &mut bbd {
            b.set_bandwidth(12000.0);
        }
        Self {
            fs,
            bbd,
            comp: [Compander::new(fs), Compander::new(fs)],
            lfo: [0.0, 0.5],
            wander: [Wander::new(fs, 0.3, 5), Wander::new(fs, 0.3, 9)],
            env: Envelope::new(fs, 5.0, 120.0),
            fb: [0.0; 2],
            delay: Smooth::new(fs, 60.0, 6.0),
            depth: 0.5,
            rate: 0.3,
            wave: 0,
            regen: 0.4,
            mix: Smooth::new(fs, 20.0, 0.5),
            out: Smooth::new(fs, 20.0, 1.0),
        }
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.bbd[c].reset();
            self.comp[c].reset();
        }
        self.fb = [0.0; 2];
        self.env.value = 0.0;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.delay.set(v[DELAY] as f64);
        self.depth = v[DEPTH] as f64;
        self.rate = v[RATE] as f64;
        for w in &mut self.wander {
            w.set_rate(self.fs, self.rate);
        }
        self.wave = (v[WAVE].round() as usize).min(3);
        self.regen = v[REGEN] as f64;
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let mut peak = 0.0f64;
        for k in 0..n {
            let xl = chans[0][k] as f64;
            let xr = if nch > 1 { chans[1][k] as f64 } else { xl };
            let e = self.env.process(0.5 * (xl + xr));
            let base = self.delay.next();
            let mx = self.mix.next();
            let o = self.out.next();
            let mut wet = [0.0; 2];
            for c in 0..2 {
                self.lfo[c] = (self.lfo[c] + self.rate / self.fs).fract();
                let ph = self.lfo[c];
                // -1..1 modulation value
                let m = match self.wave {
                    0 => (2.0 * std::f64::consts::PI * ph).sin(),
                    1 => 4.0 * (ph - 0.5).abs() - 1.0,
                    2 => self.wander[c].next().clamp(-1.0, 1.0),
                    _ => (gain_to_db(e) / 30.0 + 1.0).clamp(-1.0, 1.0) * if c == 0 { 1.0 } else { 0.8 },
                };
                // depth sweeps the delay over up to 3 octaves around the setting (exponential, like a VCO clock)
                let d_ms = base * 2f64.powf(1.5 * self.depth * m);
                let x = if c == 0 { xl } else { xr };
                let enc = self.comp[c].encode(x + self.fb[c] * self.regen);
                let y = self.bbd[c].process(enc, (d_ms * 0.001 * self.fs).max(1.0));
                let y = self.comp[c].decode(y);
                let y = y.clamp(-4.0, 4.0);
                self.fb[c] = y;
                wet[c] = y;
            }
            let yl = (xl * (1.0 - 0.5 * mx) + wet[0] * mx) * o;
            let yr = (xr * (1.0 - 0.5 * mx) + wet[1] * mx) * o;
            peak = peak.max(yl.abs()).max(yr.abs());
            if nch > 1 {
                chans[0][k] = yl as f32;
                chans[1][k] = yr as f32;
            } else {
                chans[0][k] = (0.5 * (yl + yr)) as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Slow Deep Flange", values: &[(DELAY, 3.0), (DEPTH, 0.8), (RATE, 0.12), (REGEN, 0.7), (MIX, 0.5)] },
        Preset { name: "Atmosphere Wash", values: &[(DELAY, 12.0), (DEPTH, 0.5), (RATE, 0.2), (WAVE, 2.0), (REGEN, 0.5), (MIX, 0.5)] },
        Preset { name: "Doubling (random)", values: &[(DELAY, 25.0), (DEPTH, 0.25), (RATE, 0.4), (WAVE, 2.0), (REGEN, 0.0), (MIX, 0.5)] },
        Preset { name: "Chorus Guitar", values: &[(DELAY, 8.0), (DEPTH, 0.35), (RATE, 0.9), (REGEN, 0.1), (MIX, 0.5)] },
        Preset { name: "Negative Regen Hollow", values: &[(DELAY, 2.0), (DEPTH, 0.6), (RATE, 0.3), (REGEN, -0.8), (MIX, 0.5)] },
        Preset { name: "Envelope Warp", values: &[(DELAY, 10.0), (DEPTH, 0.7), (WAVE, 3.0), (REGEN, 0.5), (MIX, 0.5)] },
        Preset { name: "Vibrato (wet only)", values: &[(DELAY, 5.0), (DEPTH, 0.3), (RATE, 5.5), (REGEN, 0.0), (MIX, 1.0)] },
        Preset { name: "Metallic Drum Ring", values: &[(DELAY, 1.5), (DEPTH, 0.1), (RATE, 0.1), (REGEN, 0.9), (MIX, 0.45)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "TIME MODULATOR",
        subtitle: ("AFTER THE MARSHALL TIME MODULATOR", "ANALOGUE DELAY · FLANGE · CHORUS"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![

            Item::Knob { p: DELAY, x: 90.0, y: 160.0, r: 40.0, label: "DELAY", tip: "Centre delay time, 0.5 to 50 ms.", active: None },
            Item::Knob { p: DEPTH, x: 220.0, y: 160.0, r: 40.0, label: "DEPTH", tip: "How far the delay is swept (up to 3 octaves of delay time).", active: None },
            Item::Knob { p: RATE, x: 350.0, y: 160.0, r: 34.0, label: "RATE", tip: "Sweep speed.", active: None },
            Item::Selector { p: WAVE, x: 470.0, y: 160.0, label: "MODULATION", positions: &["SINE", "TRI", "RANDOM", "ENVELOPE"], tip: "What moves the delay: an LFO, a random wander, or the input's own level." },
            Item::Divider { y: 264.0 },
            Item::Knob { p: REGEN, x: 120.0, y: 330.0, r: 24.0, label: "REGENERATION", tip: "Feedback, positive or negative. Negative gives a hollow, notched flange.", active: None },
            Item::Knob { p: MIX, x: 320.0, y: 330.0, r: 24.0, label: "MIX", tip: "Dry vs delayed.", active: None },
            Item::Knob { p: OUTPUT, x: 520.0, y: 330.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Time Modulator", class_id: *b"MHTimeModulatPQ1", vendor: "PEQUOD Hannett",
    categories: [Fx, Modulation]
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
    fn flange_makes_moving_notches() {
        let p = with(defaults(), &[(DELAY, 3.0), (DEPTH, 0.8), (RATE, 0.5), (REGEN, 0.6), (MIX, 0.5)]);
        let mut nz = vintage::dsp::Noise::new(3);
        let x: Vec<f32> = (0..96000).map(|_| (nz.next() * 0.3) as f32).collect();
        let mut input = vec![x.clone()];
        run(&p, &mut input);
        // spectrum of 2 different 20 ms windows differ (moving comb)
        let band = |w: &[f32], f: f64| {
            let mut b = vintage::dsp::Biquad::identity();
            b.set_bandpass(48000.0, f, 10.0);
            let y: Vec<f32> = w.iter().map(|v| b.process(*v as f64) as f32).collect();
            rms(&y[200..])
        };
        let a = band(&input[0][47000..50400], 300.0);
        let c = band(&input[0][71000..74400], 300.0);
        println!("300 Hz band at two times: {:.1} dB apart", db(a / c).abs());
        assert!(input[0].iter().all(|v| v.is_finite()));
        assert!(db(a / c).abs() > 1.0);
    }

}
