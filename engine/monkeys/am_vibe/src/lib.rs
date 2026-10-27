//! Deja Vibe: a Uni-Vibe in the manner of the Fulltone Mini Deja Vibe on Alex Turner's AM board. Four phase-shift stages with unequal capacitors, swept by a lamp and light-dependent resistors (so the sweep is lopsided: quick to brighten, slow to fade), in CHORUS (mixed) or VIBRATO (wet only).

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
pub const H: u32 = 440;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "speed"]
    pub speed: FloatParam,
    #[id = "intensity"]
    pub intensity: FloatParam,
    #[id = "mode"]
    pub mode: BoolParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            speed: rate("Speed", 2.0, 0.3, 10.0),
            intensity: pct("Intensity", 0.7),
            mode: BoolParam::new("Vibrato", false),
            output: db("Volume", 0.0, -24.0, 12.0),
        }
    }
}

pub const SPEED: usize = 0;
pub const INTENSITY: usize = 1;
pub const MODE: usize = 2;
pub const OUTPUT: usize = 3;
pub const NPARAMS: usize = 4;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.speed.as_ptr(),
            self.intensity.as_ptr(),
            self.mode.as_ptr(),
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

/// Stage centre frequencies (unequal caps), Hz.
const BASE: [f64; 4] = [80.0, 300.0, 1100.0, 3600.0];

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    z: Vec<[f64; 4]>,
    phase: f64,
    lamp: f64,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, z: vec![[0.0; 4]; ch.max(2)], phase: 0.0, lamp: 0.0, out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.z.iter_mut().for_each(|z| *z = [0.0; 4]);
        self.phase = 0.0;
        self.lamp = 0.0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let speed = self.v[SPEED] as f64;
        let depth = self.v[INTENSITY] as f64;
        let vib = self.v[MODE] > 0.5;
        let up = coeff(self.fs, 8.0);
        let down = coeff(self.fs, 60.0);
        let last = self.z.len() - 1;
        for k in 0..n {
            self.phase = (self.phase + speed / self.fs).fract();
            // the lamp: a rectified sine (bright peaks), heated quickly and cooling slowly
            let drive = (std::f64::consts::PI * self.phase).sin().powi(2);
            let c = if drive > self.lamp { up } else { down };
            self.lamp = drive + c * (self.lamp - drive);
            let m = (depth * 2.2 * self.lamp).exp();
            let mut a = [0.0; 4];
            for i in 0..4 {
                let t = (std::f64::consts::PI * (BASE[i] * m).min(0.45 * self.fs) / self.fs).tan();
                a[i] = (t - 1.0) / (t + 1.0);
            }
            let o = self.out.next();
            for (ci, ch) in chans.iter_mut().enumerate() {
                let z = &mut self.z[ci.min(last)];
                let x = ch[k] as f64;
                let mut y = x;
                for i in 0..4 {
                    // first-order all-pass: y = a*x + z; z = x - a*y
                    let out = a[i] * y + z[i];
                    z[i] = y - a[i] * out;
                    y = out;
                }
                let w = if vib { y } else { 0.5 * (x + y) };
                ch[k] = (w * o * if vib { 1.0 } else { 1.3 }) as f32;
            }
        }
        meters[0] = self.lamp as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Turner Vibe (AM)", values: &[] },
        Preset { name: "Slow Swirl", values: &[(SPEED, 0.8), (INTENSITY, 0.8)] },
        Preset { name: "Fast Throb", values: &[(SPEED, 6.0), (INTENSITY, 0.6)] },
        Preset { name: "Vibrato, Warbly", values: &[(MODE, 1.0), (SPEED, 4.5), (INTENSITY, 0.5)] },
        Preset { name: "Subtle Movement", values: &[(SPEED, 1.5), (INTENSITY, 0.35)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "DEJA VIBE",
        subtitle: ("UNI-VIBE STYLE PHASE-SHIFT", "LAMP · LDR · CHORUS / VIBRATO"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![

            Item::Knob { p: SPEED, x: 130.0, y: 190.0, r: 46.0, label: "SPEED", tip: "Sweep speed.", active: None },
            Item::Knob { p: INTENSITY, x: 320.0, y: 190.0, r: 46.0, label: "INTENSITY", tip: "Sweep depth.", active: None },
            Item::Switch { p: MODE, x: 510.0, y: 175.0, labels: ["CHORUS", "VIBRATO"], tip: "CHORUS: mixed with the dry signal (the classic throb). VIBRATO: shifted signal only (pitch wobble)." },
            Item::Lamp { m: 0, x: 510.0, y: 235.0, label: "LAMP", lo: 0.0, hi: 1.0 },
            Item::Divider { y: 285.0 },
            Item::Knob { p: OUTPUT, x: 320.0, y: 340.0, r: 24.0, label: "VOLUME", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Deja Vibe", class_id: *b"AMkDejaVibePQD01", vendor: "PEQUOD Arctic Monkeys",
    categories: [Fx, Modulation]
);

#[cfg(test)]
#[allow(dead_code)]
pub mod tests {
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

    use std::f64::consts::PI;
    fn sine_amp(f: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n).map(|k| (amp * (2.0 * PI * f * k as f64 / 48000.0).sin()) as f32).collect()
    }
    fn goertzel(x: &[f32], f: f64) -> f64 {
        let w = 2.0 * PI * f / 48000.0;
        let (mut s1, mut s2) = (0.0, 0.0);
        for v in x {
            let s = *v as f64 + 2.0 * w.cos() * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        ((s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2).max(0.0)).sqrt() / x.len() as f64
    }
    fn go(p: &[f32], x: &[f32]) -> Vec<f32> {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        input.remove(0)
    }
    /// harmonics (2..6) relative to the fundamental, dB
    fn thd(p: &[f32], f: f64, amp: f64) -> f64 {
        let y = go(p, &sine_amp(f, amp, 48000));
        let a = goertzel(&y[24000..], f);
        let h: f64 = (2..7).map(|k| goertzel(&y[24000..], f * k as f64).powi(2)).sum::<f64>().sqrt();
        db(h / a.max(1e-12))
    }

    #[test]
    fn it_moves() {
        let p = with(defaults(), &[(SPEED, 2.0), (INTENSITY, 1.0)]);
        let y = go(&p, &sine_amp(800.0, 0.3, 48000));
        let w: Vec<f64> = y.chunks(2400).map(|c| rms(c)).collect();
        let lo = w.iter().cloned().fold(9.0, f64::min);
        let hi = w.iter().cloned().fold(0.0, f64::max);
        println!("800 Hz level swings {:.1} dB", db(hi) - db(lo));
        assert!(db(hi) - db(lo) > 4.0);
    }

}
