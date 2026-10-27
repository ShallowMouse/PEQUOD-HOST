//! Fender 2x15: the Showman's closed 2x15 with JBL D130Fs. Cabinet only: put it after an amp with no cabinet (a PEQUOD Amps head, or any amp plugin set to DI). Responses estimated from published curves and typical close-mic recordings.

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
use vintage::amp::cab::{self, Cabinet};

pub const W: u32 = 640;
pub const H: u32 = 400;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Mic { #[name = "Dynamic"] Dynamic, #[name = "Ribbon"] Ribbon, #[name = "Condenser"] Condenser }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "mictype"]
    pub mic_type: EnumParam<Mic>,
    #[id = "position"]
    pub position: FloatParam,
    #[id = "distance"]
    pub distance: FloatParam,
    #[id = "axis"]
    pub axis: FloatParam,
    #[id = "lowcut"]
    pub lowcut: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            mic_type: EnumParam::new("Microphone", Mic::Dynamic),
            position: pct("Mic Position", 0.3),
            distance: pct("Distance", 0.0),
            axis: pct("Off-Axis", 0.0),
            lowcut: hz("Low Cut", 40.0, 20.0, 300.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const MICTYPE: usize = 0;
pub const POSITION: usize = 1;
pub const DISTANCE: usize = 2;
pub const AXIS: usize = 3;
pub const LOWCUT: usize = 4;
pub const OUTPUT: usize = 5;
pub const NPARAMS: usize = 6;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.mic_type.as_ptr(),
            self.position.as_ptr(),
            self.distance.as_ptr(),
            self.axis.as_ptr(),
            self.lowcut.as_ptr(),
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
const SPEC: cab::CabSpec = cab::FENDER_2X15_JBL;

struct Chan {
    close: Cabinet,
    far: Cabinet,
    dl: DelayLine,
    mic: [Biquad; 4],
    axis: Biquad,
    lowcut: Biquad,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ch: Vec<Chan>,
    key: Vec<i32>,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let v = self.v;
        let near = 1.0 - v[DISTANCE] as f64;
        let mt = v[MICTYPE].round() as usize;
        for c in self.ch.iter_mut() {
            c.close.set_mic(v[POSITION] as f64);
            c.far.set_mic(v[POSITION] as f64);
            match mt {
                // dynamic (SM57-type): presence peak, bass build-up up close
                0 => {
                    c.mic[0].set_highpass(fs, 55.0, 0.7);
                    c.mic[1].set_low_shelf(fs, 160.0, 0.7, 4.0 * near * near);
                    c.mic[2].set_peak(fs, 5200.0, 1.2, 5.0);
                    c.mic[3].set_lowpass(fs, 14000.0, 0.7);
                }
                // ribbon (R-121-type): dark and round, the most proximity bass
                1 => {
                    c.mic[0].set_highpass(fs, 30.0, 0.7);
                    c.mic[1].set_low_shelf(fs, 220.0, 0.7, 6.0 * near * near);
                    c.mic[2].set_high_shelf(fs, 4500.0, 0.7, -7.0);
                    c.mic[3].set_peak(fs, 900.0, 0.8, 1.0);
                }
                // condenser (U87-type): flat with a little air
                _ => {
                    c.mic[0].set_highpass(fs, 35.0, 0.7);
                    c.mic[1].set_low_shelf(fs, 150.0, 0.7, 2.0 * near * near);
                    c.mic[2].set_peak(fs, 9500.0, 0.8, 2.5);
                    c.mic[3].set_peak(fs, 3000.0, 1.0, -1.0);
                }
            }
            c.axis.set_high_shelf(fs, 3000.0, 0.7, -10.0 * v[AXIS] as f64);
            c.lowcut.set_highpass(fs, v[LOWCUT] as f64, 0.7);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mk = || Chan { close: Cabinet::new(fs, SPEC), far: Cabinet::new(fs, SPEC), dl: DelayLine::new((0.01 * fs) as usize),
            mic: [Biquad::identity(); 4], axis: Biquad::identity(), lowcut: Biquad::identity() };
        let mut d = Self { fs, v, ch: (0..ch.max(2)).map(|_| mk()).collect(), key: vec![], out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        for c in self.ch.iter_mut() {
            c.close.reset();
            c.far.reset();
            c.dl.reset();
            c.mic.iter_mut().for_each(|b| b.reset());
            c.axis.reset();
            c.lowcut.reset();
        }
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k: Vec<i32> = v[..NPARAMS].iter().map(|x| (x * 1000.0) as i32).collect();
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let d = self.v[DISTANCE] as f64;
        // backing the mic off: the close sound fades into the cabinet's room sound, about 3 ms later
        let dly = 1.0 + d * 0.003 * self.fs;
        let last = self.ch.len() - 1;
        for k in 0..n {
            let o = self.out.next();
            for (ci, chn) in chans.iter_mut().enumerate() {
                let c = &mut self.ch[ci.min(last)];
                let x = chn[k] as f64;
                let near = c.close.process(x);
                let farv = c.far.process_speaker(x);
                c.dl.write(farv);
                let y = near * (1.0 - d) + c.dl.read(dly) * d * 1.2;
                let y = cascade(&mut c.mic, y);
                let y = c.axis.process(y);
                let y = c.lowcut.process(y);
                chn[k] = (y * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "SM57 on the Cap Edge", values: &[(MICTYPE, 0.0), (POSITION, 0.35)] },
        Preset { name: "Ribbon, Warm", values: &[(MICTYPE, 1.0), (POSITION, 0.3)] },
        Preset { name: "Condenser at 30 cm", values: &[(MICTYPE, 2.0), (DISTANCE, 0.4)] },
        Preset { name: "Bright Centre", values: &[(MICTYPE, 0.0), (POSITION, 0.0)] },
        Preset { name: "Dark Edge, Off-Axis", values: &[(MICTYPE, 0.0), (POSITION, 0.8), (AXIS, 0.6)] },
        Preset { name: "Room Mic (1 m)", values: &[(MICTYPE, 2.0), (DISTANCE, 1.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "FENDER 2x15",
        subtitle: ("SHOWMAN CABINET", "CLOSED BACK · JBL D130F"),
        accent: ui::OXBLOOD,
        size: (W, H),
        items: vec![

            Item::Selector { p: MICTYPE, x: 90.0, y: 165.0, label: "MICROPHONE", positions: &["DYNAMIC", "RIBBON", "CONDENSER"], tip: "Dynamic (SM57-type: forward and bright), ribbon (R-121-type: dark, round, big low end up close) or condenser (U87-type: open and even)." },
            Item::Knob { p: POSITION, x: 230.0, y: 165.0, r: 34.0, label: "POSITION", tip: "Mic from the cone centre (bright, direct) out to the edge (darker, rounder).", active: None },
            Item::Knob { p: DISTANCE, x: 360.0, y: 165.0, r: 34.0, label: "DISTANCE", tip: "Up against the grille, back to about a metre: less proximity bass, more of the cabinet's room sound.", active: None },
            Item::Knob { p: AXIS, x: 490.0, y: 165.0, r: 34.0, label: "OFF-AXIS", tip: "Angle the mic away from the cone: softer top.", active: None },
            Item::Divider { y: 245.0 },
            Item::Knob { p: LOWCUT, x: 200.0, y: 305.0, r: 24.0, label: "LOW CUT", tip: "High-pass after the mic.", active: None },
            Item::Knob { p: OUTPUT, x: 440.0, y: 305.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Fender 2x15", class_id: *b"PqCBF21500000000", vendor: "PEQUOD Cabs",
    categories: [Fx, Tools]
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

    fn tone(p: &[f32], amp: f64) -> Vec<f32> {
        let x: Vec<f32> = (0..48000).map(|k| (amp * (2.0 * std::f64::consts::PI * 220.0 * k as f64 / 48000.0).sin()) as f32).collect();
        let mut input = vec![x.clone(), x];
        run(p, &mut input);
        input.remove(0)
    }
    fn goertzel(x: &[f32], f: f64) -> f64 {
        let w = 2.0 * std::f64::consts::PI * f / 48000.0;
        let (mut s1, mut s2) = (0.0, 0.0);
        for v in x {
            let s = *v as f64 + 2.0 * w.cos() * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        (s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2).max(0.0).sqrt()
    }
    fn harm(y: &[f32]) -> f64 {
        let h: f64 = (2..7).map(|k| goertzel(&y[24000..], 220.0 * k as f64).powi(2)).sum::<f64>().sqrt();
        db(h / goertzel(&y[24000..], 220.0).max(1e-12))
    }

    fn level(p: &[f32], f: f64) -> f64 {
        let x: Vec<f32> = (0..24000).map(|k| (0.1 * (2.0 * std::f64::consts::PI * f * k as f64 / 48000.0).sin()) as f32).collect();
        let mut input = vec![x.clone(), x];
        run(p, &mut input);
        db(rms(&input[0][12000..]))
    }
    #[test]
    fn speaker_and_mics() {
        let p = defaults();
        let mid = level(&p, 1000.0);
        let top = level(&p, (SPEC.f_hi * 2.2).min(20000.0));
        let dyn5 = level(&with(defaults(), &[(MICTYPE, 0.0)]), 5000.0);
        let rib5 = level(&with(defaults(), &[(MICTYPE, 1.0)]), 5000.0);
        println!("1 kHz {mid:.1} dB, 2.2 x top {top:.1} dB; 5 kHz dynamic {dyn5:.1} / ribbon {rib5:.1}");
        assert!(mid - top > 15.0 && dyn5 - rib5 > 4.0);
    }

}
