//! DM-1: the Boss DM-1 analog delay Alex Turner used from Humbug on. A bucket-brigade chip: dark, slightly gritty repeats that soften as they repeat, with the compander's breathing. Repeat rate about 50 to 400 ms.

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
use vintage::fx::Bbd;

pub const W: u32 = 640;
pub const H: u32 = 440;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "rate"]
    pub rate: FloatParam,
    #[id = "intensity"]
    pub intensity: FloatParam,
    #[id = "echo"]
    pub echo: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            rate: ms("Repeat Rate", 300.0, 50.0, 400.0),
            intensity: pct("Intensity", 0.35),
            echo: pct("Echo Level", 0.45),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const RATE: usize = 0;
pub const INTENSITY: usize = 1;
pub const ECHO: usize = 2;
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
            self.rate.as_ptr(),
            self.intensity.as_ptr(),
            self.echo.as_ptr(),
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
    v: [f32; NPARAMS],
    bbd: Vec<Bbd>,
    fbf: Vec<Biquad>,
    fb: Vec<f64>,
    t: Smooth,
    lfo: f64,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut b = Bbd::new(fs, 450.0);
        // the longer the delay, the slower the clock and the darker the repeats
        b.set_bandwidth(4500.0);
        let mut f = Biquad::identity();
        f.set_highpass(fs, 120.0, 0.7);
        Self { fs, v, bbd: (0..ch.max(2)).map(|_| Bbd::new(fs, 450.0)).collect(), fbf: vec![f; ch.max(2)], fb: vec![0.0; ch.max(2)],
               t: Smooth::new(fs, 80.0, 0.3 * fs), lfo: 0.0, out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.bbd.iter_mut().for_each(|b| b.reset());
        self.fbf.iter_mut().for_each(|b| b.reset());
        self.fb.iter_mut().for_each(|x| *x = 0.0);
        self.t.snap(self.v[RATE] as f64 * 0.001 * self.fs);
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let ms = v[RATE] as f64;
        // clock bandwidth: ~7 kHz at 50 ms down to ~2.8 kHz at 400 ms
        let bw = 7000.0 * (50.0 / ms).powf(0.45);
        for b in self.bbd.iter_mut() {
            b.set_bandwidth(bw);
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fbk = self.v[INTENSITY] as f64 * 1.05;
        let echo = self.v[ECHO] as f64 * 1.4;
        self.t.set(self.v[RATE] as f64 * 0.001 * self.fs);
        for k in 0..n {
            self.lfo = (self.lfo + 0.4 / self.fs).fract();
            let d = self.t.next() * (1.0 + 0.002 * (2.0 * std::f64::consts::PI * self.lfo).sin());
            let o = self.out.next();
            for (ci, ch) in chans.iter_mut().enumerate() {
                let c = ci.min(self.bbd.len() - 1);
                let x = ch[k] as f64;
                let y = self.bbd[c].process(x + self.fb[c], d);
                self.fb[c] = self.fbf[c].process(y) * fbk;
                ch[k] = ((x + y * echo) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Turner DM-1 (Humbug)", values: &[] },
        Preset { name: "Slapback", values: &[(RATE, 90.0), (INTENSITY, 0.1), (ECHO, 0.5)] },
        Preset { name: "Crying Lightning Echo", values: &[(RATE, 350.0), (INTENSITY, 0.45), (ECHO, 0.45)] },
        Preset { name: "Long Dark Trails", values: &[(RATE, 400.0), (INTENSITY, 0.65), (ECHO, 0.4)] },
        Preset { name: "Short Doubling", values: &[(RATE, 60.0), (INTENSITY, 0.0), (ECHO, 0.55)] },
        Preset { name: "Self-Oscillation", values: &[(RATE, 280.0), (INTENSITY, 1.0), (ECHO, 0.4), (OUTPUT, -6.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "DM-1",
        subtitle: ("AFTER THE BOSS DM-1 ANALOG DELAY", "BUCKET BRIGADE · 50-400 MS"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![

            Item::Knob { p: RATE, x: 130.0, y: 190.0, r: 46.0, label: "REPEAT RATE", tip: "Delay time. Longer delays run the chip slower: darker repeats.", active: None },
            Item::Knob { p: INTENSITY, x: 320.0, y: 190.0, r: 46.0, label: "INTENSITY", tip: "Repeats. Near the top it runs away into oscillation.", active: None },
            Item::Knob { p: ECHO, x: 510.0, y: 190.0, r: 46.0, label: "ECHO", tip: "Echo level (the dry signal is always full).", active: None },
            Item::Divider { y: 285.0 },
            Item::Knob { p: OUTPUT, x: 320.0, y: 340.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "DM-1 Delay", class_id: *b"AMkDM1AnalogDly1", vendor: "PEQUOD Arctic Monkeys",
    categories: [Fx, Delay]
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
    fn echo_at_the_rate() {
        let p = with(defaults(), &[(RATE, 200.0), (INTENSITY, 0.0)]);
        let mut x = vec![0.0f32; 24000];
        x[100] = 1.0;
        let y = go(&p, &x);
        let pk = y.iter().enumerate().skip(1000).max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap()).unwrap().0 as f64 / 48.0;
        println!("echo at {pk:.1} ms (impulse at 2.1 ms)");
        assert!((pk - 202.1).abs() < 3.0);
    }

}
