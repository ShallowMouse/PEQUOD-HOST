//! V72 / V76: valve preamps after the Telefunken V72 and V76 used on bass, guitars and keys for sombr's 12 to 12. The V72 is a fixed-gain two-stage amplifier; the V76 has far more gain in steps. Transformers in and out.

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
pub const H: u32 = 432;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Model { #[name = "V72"] V72, #[name = "V76"] V76 }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "model"]
    pub model: EnumParam<Model>,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "lowcut"]
    pub low_cut: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            model: EnumParam::new("Model", Model::V72),
            drive: db("Drive", 0.0, -12.0, 30.0),
            low_cut: hz("Low Cut", 30.0, 20.0, 300.0),
            output: db("Output", 0.0, -30.0, 12.0),
        }
    }
}

pub const MODEL: usize = 0;
pub const DRIVE: usize = 1;
pub const LOWCUT: usize = 2;
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
            self.model.as_ptr(),
            self.drive.as_ptr(),
            self.low_cut.as_ptr(),
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
    os: [Oversampler; 2],
    t_in: [Biquad; 2],
    t_out: [Biquad; 2],
    top: [Biquad; 2],
    hp: [Biquad; 2],
    dc: [DcBlock; 2],
    model: usize,
    lc: f64,
    drive: Smooth,
    out: Smooth,
}

impl Dsp {
    fn voice(&mut self, m: usize) {
        for c in 0..2 {
            // V72: a touch of low-mid body, gentle top roll-off (~15 kHz); V76: wider, a little more air
            self.t_in[c].set_low_shelf(self.fs, 90.0, 0.7, if m == 0 { 1.5 } else { 0.8 });
            self.t_out[c].set_peak(self.fs, if m == 0 { 250.0 } else { 180.0 }, 0.8, if m == 0 { 1.2 } else { 0.6 });
            self.top[c].set_lowpass(self.fs, if m == 0 { 15000.0 } else { 18000.0 }, 0.6);
        }
        self.model = m;
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut d = Self {
            fs,
            os: [Oversampler::new(), Oversampler::new()],
            t_in: [Biquad::identity(); 2],
            t_out: [Biquad::identity(); 2],
            top: [Biquad::identity(); 2],
            hp: [Biquad::identity(); 2],
            dc: [DcBlock::new(fs), DcBlock::new(fs)],
            model: 9,
            lc: 0.0,
            drive: Smooth::new(fs, 20.0, 1.0),
            out: Smooth::new(fs, 20.0, 1.0),
        };
        d.voice(0);
        d
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.os[c].reset();
            for b in [&mut self.t_in[c], &mut self.t_out[c], &mut self.top[c], &mut self.hp[c]] {
                b.reset();
            }
            self.dc[c].reset();
        }
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn set_params(&mut self, v: &[f32]) {
        let m = (v[MODEL].round() as usize).min(1);
        if m != self.model {
            self.voice(m);
        }
        let lc = v[LOWCUT] as f64;
        if (lc - self.lc).abs() > 0.5 {
            self.lc = lc;
            for b in &mut self.hp {
                b.set_highpass(self.fs, lc, 0.6);
            }
        }
        self.drive.set(db_to_gain(v[DRIVE] as f64));
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let mut peak = 0.0f64;
        let v76 = self.model == 1;
        for k in 0..n {
            let d = self.drive.next();
            let o = self.out.next();
            for (c, ch) in chans.iter_mut().enumerate().take(2) {
                let x = self.hp[c].process(self.t_in[c].process(ch[k] as f64 * d));
                // two valve stages (V76: hotter second stage); output transformer saturates softly on lows
                let y = self.os[c].process(x, |s| {
                    let a = 0.6 * tube(1.2 * s, 0.22) / 1.2;
                    let b = if v76 { tube(2.0 * a, 0.3) / 2.0 } else { tube(a, 0.15) };
                    b * 1.6
                });
                let y = self.dc[c].process(y);
                let y = self.top[c].process(self.t_out[c].process(y)) * o;
                peak = peak.max(y.abs());
                ch[k] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "V72 Bass DI", values: &[(MODEL, 0.0), (DRIVE, 6.0), (LOWCUT, 25.0), (OUTPUT, -3.0)] },
        Preset { name: "V72 Keys (CP80)", values: &[(MODEL, 0.0), (DRIVE, 3.0), (LOWCUT, 60.0)] },
        Preset { name: "V76 Guitar Grit", values: &[(MODEL, 1.0), (DRIVE, 18.0), (LOWCUT, 90.0), (OUTPUT, -12.0)] },
        Preset { name: "V76 Clean Vocal", values: &[(MODEL, 1.0), (DRIVE, 0.0), (LOWCUT, 80.0)] },
        Preset { name: "Pushed Drum Bus", values: &[(MODEL, 0.0), (DRIVE, 14.0), (OUTPUT, -9.0)] },
        Preset { name: "Cooked (V76 flat out)", values: &[(MODEL, 1.0), (DRIVE, 30.0), (OUTPUT, -20.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "V72 / V76",
        subtitle: ("AFTER THE TELEFUNKEN V72 AND V76", "VALVE PREAMPS · TRANSFORMERS"),
        accent: ui::STEEL,
        size: (W, H),
        items: vec![

            Item::Switch { p: MODEL, x: 100.0, y: 160.0, labels: ["V72", "V76"], tip: "V72: fixed-gain two-stage amplifier. V76: more gain and a hotter second stage." },
            Item::Knob { p: DRIVE, x: 230.0, y: 165.0, r: 46.0, label: "DRIVE", tip: "How hard the valves are hit.", active: None },
            Item::Meter { m: 0, x: 330.0, y: 96.0, w: 200.0, h: 128.0, kind: MeterKind::Vu, caption: "OUTPUT" },
            Item::Divider { y: 264.0 },
            Item::Knob { p: LOWCUT, x: 160.0, y: 330.0, r: 24.0, label: "LOW CUT", tip: "High-pass filter.", active: None },
            Item::Knob { p: OUTPUT, x: 480.0, y: 330.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "V72 V76", class_id: *b"SBV72V76PreampPQ", vendor: "PEQUOD sombr",
    categories: [Fx, Distortion]
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
    fn clean_at_low_drive_and_v76_dirtier() {
        let h2 = |model: f32, drive: f32| {
            let p = with(defaults(), &[(MODEL, model), (DRIVE, drive), (LOWCUT, 20.0)]);
            let mut input = vec![sine(200.0, 0.2, 48000)];
            run(&p, &mut input);
            let x = &input[0][24000..];
            let tone = |f: f64| {
                let (mut s, mut c) = (0.0, 0.0);
                for (k, v) in x.iter().enumerate() {
                    let ph = 2.0 * std::f64::consts::PI * f * k as f64 / 48000.0;
                    s += *v as f64 * ph.sin();
                    c += *v as f64 * ph.cos();
                }
                (s * s + c * c).sqrt()
            };
            db((tone(400.0) + tone(600.0)) / tone(200.0))
        };
        let (a, b) = (h2(0.0, -6.0), h2(1.0, 18.0));
        println!("V72 low drive {a:.1} dB, V76 driven {b:.1} dB");
        assert!(a < -30.0 && b > a + 15.0);
    }

}
