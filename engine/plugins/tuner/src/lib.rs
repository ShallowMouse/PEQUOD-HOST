//! Tuner — chromatic tuner with a cents needle.
//!
//! Passes audio straight through (or mutes it while you tune). Uses the same
//! YIN pitch tracker as Feedback, at a higher analysis rate for accuracy.

use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
use vintage::dsp::*;
use vintage::nih_plug_egui::EguiState;
use vintage::pitch::Pitch;
use vintage::ui::{self, Item, Panel, Preset};
use vintage::{VintageDsp, VintageParams};

const W: u32 = 640;
const H: u32 = 432;

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Mute {
    #[name = "Thru"]
    Thru,
    #[name = "Mute"]
    Mute,
}

#[derive(Params)]
pub struct TunerParams {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "reference"]
    reference: FloatParam,
    #[id = "mute"]
    mute: EnumParam<Mute>,
}

impl Default for TunerParams {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            reference: FloatParam::new("Reference A4", 440.0, FloatRange::Linear { min: 430.0, max: 450.0 })
                .with_step_size(0.5)
                .with_unit(" Hz"),
            mute: EnumParam::new("Output", Mute::Thru),
        }
    }
}

const RING: usize = 8192;
/// Refinement compares the signal with itself several periods later
/// (up to about this many samples), which multiplies the precision.
const REFINE_SPAN: f64 = 2400.0;
const REFINE_LEN: usize = 2048;

const REFERENCE: usize = 0;
const MUTE: usize = 1;

impl VintageParams for TunerParams {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![self.reference.as_ptr(), self.mute.as_ptr()]
    }
}

pub struct TunerDsp {
    fs: f64,
    pitch: Pitch,
    /// Full-rate history used to refine the tracker's estimate.
    ring: Vec<f64>,
    pos: usize,
    #[cfg(feature = "demo")]
    demo_phase: f64,
    env: Envelope,
    mute: bool,
    gain: Smooth,
    fresh: bool,
}

impl TunerDsp {
    #[inline]
    fn at(&self, back: usize) -> f64 {
        self.ring[(self.pos + RING - 1 - back) % RING]
    }

    /// Refine a coarse frequency estimate: measure the difference function
    /// at k whole periods (k as large as fits), interpolate its minimum, and
    /// divide by k. Errors shrink by a factor of k.
    fn refine(&self, f: f64) -> f64 {
        let period = self.fs / f;
        let k = (REFINE_SPAN / period).floor().max(1.0);
        let lag = (k * period).round() as i64;
        let d = |tau: i64| -> f64 {
            let mut acc = 0.0;
            for j in 0..REFINE_LEN {
                let e = self.at(j) - self.at(j + tau as usize);
                acc += e * e;
            }
            acc
        };
        if lag < 3 || lag as usize + REFINE_LEN + 4 >= RING {
            return f;
        }
        // walk to the local minimum, then interpolate
        let mut t = lag;
        let mut dt = d(t);
        for _ in 0..4 {
            let (l, r) = (d(t - 1), d(t + 1));
            if l < dt && l <= r {
                t -= 1;
                dt = l;
            } else if r < dt {
                t += 1;
                dt = r;
            } else {
                break;
            }
        }
        let (a, b, c) = (d(t - 1), dt, d(t + 1));
        let den = a - 2.0 * b + c;
        let shift = if den.abs() > 1e-15 { (0.5 * (a - c) / den).clamp(-1.0, 1.0) } else { 0.0 };
        let refined = self.fs * k / (t as f64 + shift);
        // only trust it if it agrees with the coarse estimate
        if (refined / f - 1.0).abs() < 0.03 { refined } else { f }
    }
}

impl VintageDsp for TunerDsp {
    const METERS: usize = 2;

    fn new(sample_rate: f32, _channels: usize) -> Self {
        let fs = sample_rate as f64;
        // 2x decimation, ~85 ms window at 48 kHz, low B (31 Hz) .. ~1.4 kHz
        let decim = if fs > 60000.0 { 4 } else { 2 };
        let win = ((fs / decim as f64) * 0.085) as usize;
        Self {
            fs,
            ring: vec![0.0; RING],
            pos: 0,
            #[cfg(feature = "demo")]
            demo_phase: 0.0,
            pitch: Pitch::with(fs, decim, win.next_power_of_two(), 384, 30.0, 1400.0),
            env: Envelope::new(fs, 1.0, 200.0),
            mute: false,
            gain: Smooth::new(fs, 10.0, 1.0),
            fresh: false,
        }
    }

    fn reset(&mut self) {
        self.pitch.reset();
    }

    fn set_params(&mut self, v: &[f32]) {
        self.mute = v[MUTE] > 0.5;
        self.gain.set(if self.mute { 0.0 } else { 1.0 });
    }

    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let mut peak = 0.0f64;
        let before = self.pitch.freq;
        for k in 0..n {
            let mut sum = 0.0;
            for c in 0..nch {
                sum += chans[c][k] as f64;
            }
            #[allow(unused_mut)]
            let mut mono = sum / nch.max(1) as f64;
            #[cfg(feature = "demo")]
            {
                self.demo_phase = (self.demo_phase + 194.8 / self.fs).fract();
                let ph = 2.0 * std::f64::consts::PI * self.demo_phase;
                mono += 0.2 * (ph.sin() + 0.5 * (2.0 * ph).sin());
            }
            self.pitch.push(mono);
            self.ring[self.pos] = mono;
            self.pos = (self.pos + 1) % RING;
            peak = peak.max(self.env.process(mono));
            let g = self.gain.next();
            if g < 1.0 {
                for c in 0..nch {
                    chans[c][k] *= g as f32;
                }
            }
        }
        // report a pitch only while there's a signal to tune
        let audible = peak > 0.002;
        if !audible {
            self.pitch.freq = None;
            self.pitch.new_note();
        }
        self.fresh = self.pitch.freq.is_some() && (self.pitch.freq != before || audible);
        let f = if audible { self.pitch.freq.map(|f| self.refine(f)) } else { None };
        meters[0] = f.unwrap_or(0.0) as f32;
        meters[1] = peak as f32;
    }
}

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Standard A440", values: &[(REFERENCE, 440.0)] },
        Preset { name: "A442 (orchestral)", values: &[(REFERENCE, 442.0)] },
        Preset { name: "A432", values: &[(REFERENCE, 432.0)] },
        Preset { name: "Silent Tuning (mute)", values: &[(MUTE, 1.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "TUNER",
        subtitle: ("CHROMATIC", "STROBE-STYLE TUNER"),
        accent: ui::GRAPHITE,
        size: (W, H),
        items: vec![
            Item::Tuner { freq: 0, level: 1, reference: REFERENCE, x: 120.0, y: 96.0, w: 400.0, h: 160.0 },
            Item::Divider { y: 284.0 },
            Item::Knob { p: REFERENCE, x: 220.0, y: 332.0, r: 22.0, label: "REFERENCE A4",
                tip: "Pitch of A above middle C. 440 Hz is standard; change it to match a recording that's slightly off.", active: None },
            Item::Switch { p: MUTE, x: 420.0, y: 332.0, labels: ["THRU", "MUTE"],
                tip: "MUTE silences the output while you tune (the tuner still listens)." },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, TunerParams, TunerDsp, panel,
    name: "Tuner", class_id: *b"S2TunerDIY000001", vendor: "PEQUOD Beatles",
    categories: [Fx, Analyzer]
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn detect(f0: f64) -> f32 {
        let fs = 48000.0;
        let mut d = TunerDsp::new(fs as f32, 1);
        d.set_params(&[440.0, 0.0]);
        let n = (fs * 0.6) as usize;
        let mut x: Vec<f32> = (0..n)
            .map(|k| {
                let t = k as f64 / fs;
                (0.2 * ((2.0 * PI * f0 * t).sin() + 0.6 * (4.0 * PI * f0 * t).sin() + 0.3 * (6.0 * PI * f0 * t).sin())) as f32
            })
            .collect();
        let mut m = [0.0f32; 2];
        let mut last = 0.0;
        for c in x.chunks_mut(512) {
            d.process(&mut [c], &mut m);
            last = m[0];
        }
        last
    }

    #[test]
    fn accurate_to_a_cent_across_the_guitar() {
        // low E, A, D, G, B, high E, plus a bass low E and a high fret
        for f0 in [41.20, 82.41, 110.0, 146.83, 196.0, 246.94, 329.63, 659.26, 987.77] {
            let f = detect(f0) as f64;
            let cents = 1200.0 * (f / f0).log2();
            println!("{f0} Hz -> {f:.2} Hz ({cents:+.2} cents)");
            assert!(cents.abs() < 1.0, "{f0}: {cents} cents");
        }
    }

    #[test]
    fn note_names() {
        assert_eq!(ui::note_of(440.0, 440.0).0, "A");
        assert_eq!(ui::note_of(82.41, 440.0), ("E", 2, ui::note_of(82.41, 440.0).2));
        let (_, _, c) = ui::note_of(445.0, 440.0);
        assert!((c - 19.56).abs() < 0.1);
    }

    #[test]
    fn mute_silences() {
        let mut d = TunerDsp::new(48000.0, 1);
        d.set_params(&[440.0, 1.0]);
        let mut x = vec![0.5f32; 48000];
        d.process(&mut [&mut x[..]], &mut [0.0, 0.0]);
        assert!(x[40000..].iter().all(|v| v.abs() < 1e-4));
    }
}
