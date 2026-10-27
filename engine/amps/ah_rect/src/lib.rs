//! Rectifier Head: a Mesa/Boogie Dual/Triple Rectifier (three cascaded stages, scoopable mids, rectifier sag), with a noise gate in front. Amp only: no speaker or cabinet, so follow it with a PEQUOD Cabs plugin (or an IR).

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
use vintage::amp::hg::{self, HgAmp, HgControls};

pub const W: u32 = 640;
pub const H: u32 = 385;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "input"]
    pub input: FloatParam,
    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "bass"]
    pub bass: FloatParam,
    #[id = "mid"]
    pub mid: FloatParam,
    #[id = "treble"]
    pub treble: FloatParam,
    #[id = "presence"]
    pub presence: FloatParam,
    #[id = "master"]
    pub master: FloatParam,
    #[id = "gate"]
    pub gate: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            input: db("Guitar Level", 0.0, -12.0, 12.0),
            gain: FloatParam::new("Gain", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1).with_value_to_string(formatters::v2s_f32_rounded(1)),
            bass: FloatParam::new("Bass", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1).with_value_to_string(formatters::v2s_f32_rounded(1)),
            mid: FloatParam::new("Middle", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1).with_value_to_string(formatters::v2s_f32_rounded(1)),
            treble: FloatParam::new("Treble", 6.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1).with_value_to_string(formatters::v2s_f32_rounded(1)),
            presence: FloatParam::new("Presence", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1).with_value_to_string(formatters::v2s_f32_rounded(1)),
            master: FloatParam::new("Master", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1).with_value_to_string(formatters::v2s_f32_rounded(1)),
            gate: db("Gate", -90.0, -90.0, -20.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const INPUT: usize = 0;
pub const GAIN: usize = 1;
pub const BASS: usize = 2;
pub const MID: usize = 3;
pub const TREBLE: usize = 4;
pub const PRESENCE: usize = 5;
pub const MASTER: usize = 6;
pub const GATE: usize = 7;
pub const OUTPUT: usize = 8;
pub const NPARAMS: usize = 9;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.input.as_ptr(),
            self.gain.as_ptr(),
            self.bass.as_ptr(),
            self.mid.as_ptr(),
            self.treble.as_ptr(),
            self.presence.as_ptr(),
            self.master.as_ptr(),
            self.gate.as_ptr(),
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
    amp: HgAmp,
    os: Oversampler,
    dc: DcBlock,
    genv: f64,
    gg: f64,
    key: Vec<i32>,
    inp: Smooth,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let _ = fs;
        self.amp.set_controls(HgControls {
            gain: (self.v[GAIN] as f64 / 10.0),
            bass: (self.v[BASS] as f64 / 10.0),
            mid: (self.v[MID] as f64 / 10.0),
            treble: (self.v[TREBLE] as f64 / 10.0),
            presence: (self.v[PRESENCE] as f64 / 10.0),
            depth: 0.5,
            master: (self.v[MASTER] as f64 / 10.0),
        });
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 2;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, amp: HgAmp::new(fs * OS as f64, hg::rectifier()), os: Oversampler::new(), dc: DcBlock::new(fs), genv: 0.0, gg: 1.0, key: vec![],
            inp: Smooth::new(fs, 20.0, 1.0), out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn reset(&mut self) {
        self.amp.reset();
        self.os.reset();
        self.dc.reset();
        self.design();
        self.inp.snap(db_to_gain(self.v[INPUT] as f64));
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k: Vec<i32> = v[..NPARAMS].iter().map(|x| (x * 100.0) as i32).collect();
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.inp.set(db_to_gain(v[INPUT] as f64));
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().max(1) as f64;
        let (ga, gr, gs) = (coeff(self.fs, 0.5), coeff(self.fs, 60.0), coeff(self.fs, 4.0));
        let mut peak = 0.0f64;
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            let x = x / nch * self.inp.next();
            let a = x.abs();
            self.genv = a + if a > self.genv { ga } else { gr } * (self.genv - a);
            let open = self.v[GATE] <= -89.0 || 20.0 * self.genv.max(1e-9).log10() > self.v[GATE] as f64;
            let t = if open { 1.0 } else { 0.0 };
            self.gg = t + gs * (self.gg - t);
            let x = x * self.gg;
            let amp = &mut self.amp;
            let y = self.os.process(x, |u| amp.process(u));
            let y = self.dc.process(y);
            peak = peak.max(y.abs() * 2.0);
            let y = (y * self.out.next()) as f32;
            for c in chans.iter_mut() {
                c[k] = y;
            }
        }
        meters[0] = (peak * 10f64.powf(-18.0 / 20.0)) as f32;
        meters[1] = (self.amp.sag() * 25.0) as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Clean", values: &[(GAIN, 2.0), (MASTER, 6.0)] },
        Preset { name: "Crunch", values: &[(GAIN, 5.5), (MASTER, 6.0)] },
        Preset { name: "Lead", values: &[(GAIN, 10.0), (MID, 6.5), (MASTER, 6.0)] },
        Preset { name: "Scooped", values: &[(GAIN, 7.5), (MID, 2.5), (BASS, 6.5), (TREBLE, 7.0), (MASTER, 6.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "RECTIFIER HEAD",
        subtitle: ("MESA/BOOGIE RECTIFIER, AMP ONLY", "3 GAIN STAGES · TUBE RECTIFIER SAG"),
        accent: ui::OXBLOOD,
        size: (W, H),
        items: vec![
            Item::Knob { p: GAIN, x: 70.0, y: 150.0, r: 30.0, label: "GAIN", tip: "Preamp gain.", active: None },
            Item::Knob { p: BASS, x: 170.0, y: 150.0, r: 30.0, label: "BASS", tip: "Bass.", active: None },
            Item::Knob { p: MID, x: 270.0, y: 150.0, r: 30.0, label: "MIDDLE", tip: "Middle.", active: None },
            Item::Knob { p: TREBLE, x: 370.0, y: 150.0, r: 30.0, label: "TREBLE", tip: "Treble.", active: None },
            Item::Knob { p: PRESENCE, x: 470.0, y: 150.0, r: 30.0, label: "PRESENCE", tip: "Top-end bite from the power amp's feedback.", active: None },
            Item::Knob { p: MASTER, x: 570.0, y: 150.0, r: 30.0, label: "MASTER", tip: "Master volume: how hard the power amp is pushed.", active: None },
            Item::Divider { y: 215.0 },
            Item::Knob { p: INPUT, x: 70.0, y: 270.0, r: 22.0, label: "GUITAR LEVEL", tip: "How hot the guitar hits the input (0 dB = 1 V peak at full scale). Turn up for a quiet DI.", active: None },
            Item::Knob { p: GATE, x: 160.0, y: 270.0, r: 20.0, label: "GATE", tip: "Noise gate in front (as a pedal). Fully left = off.", active: None },
            Item::Knob { p: OUTPUT, x: 250.0, y: 270.0, r: 20.0, label: "OUTPUT", tip: "Output level. This is the amp alone, with no speaker: follow it with a PEQUOD Cabs plugin or an IR.", active: None },
            Item::Lamp { m: 1, x: 560.0, y: 255.0, label: "SAG", lo: 2.0, hi: 25.0 },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Rectifier Head", class_id: *b"PqAHRect00000000", vendor: "PEQUOD Amps",
    categories: [Fx, Distortion]
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

    #[test]
    fn breaks_up_when_turned_up() {
        let lo = harm(&tone(&with(defaults(), &[(GAIN, 0.0), (MASTER, 3.0)]), 0.2));
        let hi = harm(&tone(&with(defaults(), &[(GAIN, 10.0), (MASTER, 3.0)]), 0.2));
        println!("harmonics gain 0 {lo:.1} dB, gain 10 {hi:.1} dB");
        assert!(hi > lo + 8.0);
    }

}
