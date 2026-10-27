//! Nonlin: gated and reverse reverb in the manner of the AMS RMX16's NonLin 2 programme, which turned the gated room into a single box: a dense burst that stays flat and then stops, or rises and stops. Built as a sparse multi-tap response shaped by the envelope.

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
use vintage::verb::Diffuser;

pub const W: u32 = 640;
pub const H: u32 = 432;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Prog { #[name = "Nonlin"] Nonlin, #[name = "Reverse"] Reverse, #[name = "Gate Slope"] Slope }
const TAPS: usize = 700;


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "prog"]
    pub prog: EnumParam<Prog>,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "pre"]
    pub pre: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "lowcut"]
    pub low_cut: FloatParam,
    #[id = "width"]
    pub width: FloatParam,
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
            prog: EnumParam::new("Programme", Prog::Nonlin),
            decay: ms("Decay", 350.0, 80.0, 1200.0),
            pre: ms("Pre-delay", 5.0, 0.0, 200.0),
            tone: hz("HF Damping", 8000.0, 2000.0, 16000.0),
            low_cut: hz("Low Cut", 80.0, 20.0, 500.0),
            width: pct("Width", 1.0),
            mix: pct("Mix", 0.35),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const PROG: usize = 0;
pub const DECAY: usize = 1;
pub const PRE: usize = 2;
pub const TONE: usize = 3;
pub const LOWCUT: usize = 4;
pub const WIDTH: usize = 5;
pub const MIX: usize = 6;
pub const OUTPUT: usize = 7;
pub const NPARAMS: usize = 8;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.prog.as_ptr(),
            self.decay.as_ptr(),
            self.pre.as_ptr(),
            self.tone.as_ptr(),
            self.low_cut.as_ptr(),
            self.width.as_ptr(),
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
    pos: [[usize; TAPS]; 2],
    gain: [[f64; TAPS]; 2],
    key: (usize, f64, f64),
    diff: [Vec<Diffuser>; 2],
    tone: [OnePole; 2],
    lc: [Biquad; 2],
    lc_hz: f64,
    width: f64,
    mix: Smooth,
    out: Smooth,
}

impl Dsp {
    fn build(&mut self, prog: usize, decay_ms: f64, pre_ms: f64) {
        let len = decay_ms * 0.001 * self.fs;
        let pre = pre_ms * 0.001 * self.fs;
        let mut rng = Noise::new(1234);
        for c in 0..2 {
            let mut e = 0.0;
            for i in 0..TAPS {
                // jittered even spread across the response
                let t = ((i as f64 + 0.5 + 0.45 * rng.next()) / TAPS as f64).clamp(0.0, 1.0);
                let env = match prog {
                    // flat, a slight sag, then a fast fade over the last 12%
                    0 => (1.0 - 0.3 * t) * if t > 0.88 { (0.5 + 0.5 * (std::f64::consts::PI * (t - 0.88) / 0.12).cos()).max(0.0) } else { 1.0 },
                    // rising then cut off
                    1 => t.powf(1.6) * if t > 0.95 { (1.0 - t) / 0.05 } else { 1.0 },
                    // linear slope down to nothing
                    _ => 1.0 - t,
                };
                let sign = if rng.next() > 0.0 { 1.0 } else { -1.0 };
                self.pos[c][i] = (pre + t * len) as usize + 1;
                self.gain[c][i] = env * sign;
                e += env * env;
            }
            let norm = 1.0 / e.max(1e-9).sqrt() * 2.5;
            self.gain[c].iter_mut().for_each(|g| *g *= norm);
        }
        self.key = (prog, decay_ms, pre_ms);
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mk = |a: f64, b: f64| vec![Diffuser::new(fs, a, 0.6), Diffuser::new(fs, b, 0.6), Diffuser::new(fs, a * 0.37, 0.5)];
        let mut d = Self {
            fs,
            line: DelayLine::new((fs * 1.5) as usize),
            pos: [[1; TAPS]; 2],
            gain: [[0.0; TAPS]; 2],
            key: (9, 0.0, 0.0),
            diff: [mk(1.7, 2.9), mk(2.3, 3.3)],
            tone: [OnePole::default(); 2],
            lc: [Biquad::identity(); 2],
            lc_hz: 0.0,
            width: 1.0,
            mix: Smooth::new(fs, 20.0, 0.35),
            out: Smooth::new(fs, 20.0, 1.0),
        };
        d.build(0, 350.0, 5.0);
        d
    }
    fn reset(&mut self) {
        self.line.reset();
        for c in 0..2 {
            self.diff[c].iter_mut().for_each(|d| d.reset());
            self.tone[c].reset();
            self.lc[c].reset();
        }
    }
    fn set_params(&mut self, v: &[f32]) {
        let k = ((v[PROG].round() as usize).min(2), v[DECAY] as f64, v[PRE] as f64);
        if k.0 != self.key.0 || (k.1 - self.key.1).abs() > 0.5 || (k.2 - self.key.2).abs() > 0.2 {
            self.build(k.0, k.1, k.2);
        }
        for t in &mut self.tone {
            t.set_lowpass(self.fs, v[TONE] as f64);
        }
        let lc = v[LOWCUT] as f64;
        if (lc - self.lc_hz).abs() > 0.5 {
            self.lc_hz = lc;
            for b in &mut self.lc {
                b.set_highpass(self.fs, lc, 0.7);
            }
        }
        self.width = v[WIDTH] as f64;
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let mut peak = 0.0f64;
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            self.line.write(x);
            let mut w = [0.0; 2];
            for c in 0..2 {
                let mut acc = 0.0;
                for i in 0..TAPS {
                    acc += self.line.read_int(self.pos[c][i]) * self.gain[c][i];
                }
                let mut y = acc;
                for d in self.diff[c].iter_mut() {
                    y = d.process(y);
                }
                w[c] = self.lc[c].process(self.tone[c].lp(y));
            }
            let mid = 0.5 * (w[0] + w[1]);
            let side = 0.5 * (w[0] - w[1]) * self.width;
            let (wl, wr) = (mid + side, mid - side);
            let mx = self.mix.next();
            let o = self.out.next();
            for (ci, c) in chans.iter_mut().enumerate() {
                let wv = if nch == 1 { mid } else if ci == 0 { wl } else { wr };
                let y = (c[k] as f64 * (1.0 - mx) + wv * mx) * o;
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
        Preset { name: "Classic Gated Snare", values: &[(PROG, 0.0), (DECAY, 300.0), (TONE, 9000.0), (MIX, 0.4)] },
        Preset { name: "Big Gated Toms", values: &[(PROG, 0.0), (DECAY, 500.0), (TONE, 7000.0), (LOWCUT, 60.0), (MIX, 0.4)] },
        Preset { name: "Short Tight Nonlin", values: &[(PROG, 0.0), (DECAY, 160.0), (MIX, 0.35)] },
        Preset { name: "Reverse Snare", values: &[(PROG, 1.0), (DECAY, 400.0), (MIX, 0.45)] },
        Preset { name: "Reverse Vocal Swell", values: &[(PROG, 1.0), (DECAY, 700.0), (PRE, 0.0), (LOWCUT, 200.0), (MIX, 0.35)] },
        Preset { name: "Gate Slope Kick", values: &[(PROG, 2.0), (DECAY, 250.0), (TONE, 5000.0), (MIX, 0.3)] },
        Preset { name: "Nonlin on a Send", values: &[(PROG, 0.0), (DECAY, 400.0), (MIX, 1.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "NONLIN",
        subtitle: ("GATED AND REVERSE REVERB", "IN THE MANNER OF THE RMX16 NONLIN 2"),
        accent: ui::TEAL,
        size: (W, H),
        items: vec![

            Item::Selector { p: PROG, x: 90.0, y: 160.0, label: "PROGRAMME", positions: &["NONLIN", "REVERSE", "SLOPE"], tip: "Nonlin: a dense burst that holds and then stops. Reverse: it builds and stops. Slope: falls away linearly." },
            Item::Knob { p: DECAY, x: 230.0, y: 160.0, r: 44.0, label: "DECAY", tip: "Length of the burst.", active: None },
            Item::Knob { p: PRE, x: 370.0, y: 160.0, r: 34.0, label: "PRE-DELAY", tip: "Delay before the burst starts.", active: None },
            Item::Lamp { m: 0, x: 500.0, y: 160.0, label: "LEVEL", lo: 0.05, hi: 0.9 },
            Item::Divider { y: 264.0 },
            Item::Knob { p: TONE, x: 90.0, y: 330.0, r: 22.0, label: "HF DAMP", tip: "Top-end of the burst.", active: None },
            Item::Knob { p: LOWCUT, x: 200.0, y: 330.0, r: 22.0, label: "LOW CUT", tip: "High-pass on the burst.", active: None },
            Item::Knob { p: WIDTH, x: 310.0, y: 330.0, r: 22.0, label: "WIDTH", tip: "Stereo width of the burst.", active: None },
            Item::Knob { p: MIX, x: 420.0, y: 330.0, r: 22.0, label: "MIX", tip: "Dry vs effect.", active: None },
            Item::Knob { p: OUTPUT, x: 540.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Nonlin", class_id: *b"TPNonlinRevPQD01", vendor: "PEQUOD Padgham & Lillywhite",
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
    fn nonlin_stops_and_reverse_rises() {
        let ir = |prog: f32| {
            let p = with(defaults(), &[(PROG, prog), (DECAY, 400.0), (PRE, 0.0), (MIX, 1.0), (LOWCUT, 20.0)]);
            let mut x = vec![0.0f32; 48000];
            x[0] = 0.5;
            let mut input = vec![x];
            run(&p, &mut input);
            input.remove(0)
        };
        let a = ir(0.0);
        let early = rms(&a[480..4800]);
        let late = rms(&a[12000..16800]);
        let after = rms(&a[24000..30000]);
        println!("nonlin early {:.1} late {:.1} after {:.1} dB", db(early), db(late), db(after));
        assert!(db(late / early).abs() < 6.0 && db(after / early) < -40.0);
        let r = ir(1.0);
        assert!(rms(&r[14000..18000]) > 2.0 * rms(&r[480..4800]));
    }

}
