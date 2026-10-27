//! Bigsby: the vibrato tailpiece on Brian Setzer's 6120. SHAKE: the gentle wobble that comes in as a chord or note rings on, the way he shakes the arm at the end of a phrase. CONSTANT: an even vibrato. MANUAL: only the ARM, a pitch dip you can automate (a Bigsby mostly goes down).

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
use vintage::fx::PitchShift;

pub const W: u32 = 640;
pub const H: u32 = 432;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Mode { #[name = "Shake"] Shake, #[name = "Constant"] Constant, #[name = "Manual"] Manual }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "mode"]
    pub mode: EnumParam<Mode>,
    #[id = "depth"]
    pub depth: FloatParam,
    #[id = "rate"]
    pub rate: FloatParam,
    #[id = "wait"]
    pub wait: FloatParam,
    #[id = "arm"]
    pub arm: FloatParam,
    #[id = "sens"]
    pub sens: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            mode: EnumParam::new("Mode", Mode::Shake),
            depth: FloatParam::new("Depth", 25.0, FloatRange::Linear { min: 0.0, max: 80.0 }).with_step_size(0.5).with_unit(" ct"),
            rate: rate("Rate", 5.5, 2.0, 9.0),
            wait: ms("Shake After", 350.0, 50.0, 2000.0),
            arm: FloatParam::new("Arm", 0.0, FloatRange::Linear { min: -100.0, max: 25.0 }).with_step_size(0.5).with_unit(" ct"),
            sens: db("Sensitivity", 6.0, 2.0, 15.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const MODE: usize = 0;
pub const DEPTH: usize = 1;
pub const RATE: usize = 2;
pub const WAIT: usize = 3;
pub const ARM: usize = 4;
pub const SENS: usize = 5;
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
            self.depth.as_ptr(),
            self.rate.as_ptr(),
            self.wait.as_ptr(),
            self.arm.as_ptr(),
            self.sens.as_ptr(),
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

/// Note-onset detector: power envelope (x^2 through a 40 Hz low-pass) and its rise
/// over the last 30 ms. Fires once per attack.
#[derive(Clone)]
pub struct Onset {
    lp: Biquad,
    mins: [f64; 30],
    mpos: usize,
    blk: f64,
    count: usize,
    ms: usize,
    armed: bool,
    since: usize,
    pub db: f64,
}

impl Onset {
    pub fn new(fs: f64) -> Self {
        let mut lp = Biquad::identity();
        lp.set_lowpass(fs, 40.0, 0.707);
        Self { lp, mins: [-120.0; 30], mpos: 0, blk: 120.0, count: 0, ms: (fs / 1000.0).round() as usize, armed: true, since: 1 << 30, db: -120.0 }
    }
    pub fn reset(&mut self) {
        self.lp.reset();
        self.mins = [-120.0; 30];
        self.blk = 120.0;
        self.armed = true;
        self.since = 1 << 30;
        self.db = -120.0;
    }
    /// Returns true on the sample a new note is found. `gap` in samples.
    #[inline]
    pub fn process(&mut self, x: f64, sens: f64, floor: f64, gap: usize) -> bool {
        let p = self.lp.process(x * x);
        self.db = 10.0 * p.max(1e-18).log10();
        self.blk = self.blk.min(self.db);
        self.count += 1;
        if self.count >= self.ms {
            self.count = 0;
            self.mins[self.mpos] = self.blk;
            self.mpos = (self.mpos + 1) % 30;
            self.blk = 120.0;
        }
        let lo = self.mins.iter().cloned().fold(self.blk, f64::min);
        let rise = self.db - lo;
        self.since = self.since.saturating_add(1);
        if rise < 0.5 * sens {
            self.armed = true;
        }
        if self.armed && rise > sens && self.db > floor && self.since >= gap {
            self.armed = false;
            self.since = 0;
            return true;
        }
        false
    }
}

const BASE_MS: f64 = 4.0;

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    dl: [DelayLine; 2],
    ps: [PitchShift; 2],
    onset: Onset,
    t: f64,
    phase: f64,
    amt: f64,
    arm: Smooth,
    out: Smooth,
    pub shaking: f64,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let n = (0.02 * fs) as usize;
        Self {
            fs, v, dl: [DelayLine::new(n), DelayLine::new(n)], ps: [PitchShift::new(fs, 40.0), PitchShift::new(fs, 40.0)],
            onset: Onset::new(fs), t: 10.0, phase: 0.0, amt: 0.0, arm: Smooth::new(fs, 30.0, 0.0), out: Smooth::new(fs, 20.0, 1.0), shaking: 0.0,
        }
    }
    fn reset(&mut self) {
        self.dl.iter_mut().for_each(|d| d.reset());
        self.ps.iter_mut().for_each(|p| p.reset());
        self.onset.reset();
        self.t = 10.0;
        self.phase = 0.0;
        self.amt = 0.0;
        self.arm.snap(self.v[ARM] as f64);
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.arm.set(v[ARM] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn latency(&self) -> u32 {
        (BASE_MS * 0.001 * self.fs) as u32
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().max(1) as f64;
        let mode = (self.v[MODE].round() as usize).min(2);
        let depth = self.v[DEPTH] as f64;
        let rate = self.v[RATE] as f64;
        let wait = self.v[WAIT] as f64 * 0.001;
        let sens = self.v[SENS] as f64;
        let w = 2.0 * std::f64::consts::PI * rate;
        let base = BASE_MS * 0.001 * self.fs;
        let ca = coeff(self.fs, 250.0);
        let cr = coeff(self.fs, 40.0);
        let gap = (0.06 * self.fs) as usize;
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch;
            if self.onset.process(x, sens, -50.0, gap) {
                self.t = 0.0;
            }
            self.t += 1.0 / self.fs;
            // how much vibrato: none on the attack, easing in once the note rings on
            let target = match mode {
                0 => {
                    if self.t > wait && self.onset.db > -60.0 { 1.0 } else { 0.0 }
                }
                1 => 1.0,
                _ => 0.0,
            };
            let c = if target > self.amt { ca } else { cr };
            self.amt = target + c * (self.amt - target);
            self.phase = (self.phase + rate / self.fs).fract();
            // a delay swing of A seconds at w gives a pitch swing of A*w
            let cents = depth * self.amt;
            let a = (2f64.powf(cents / 1200.0) - 1.0) / w * self.fs;
            let d = base + a * (2.0 * std::f64::consts::PI * self.phase).sin();
            let arm = self.arm.next();
            let ratio = 2f64.powf(arm / 1200.0);
            let o = self.out.next();
            for (ci, ch) in chans.iter_mut().enumerate() {
                let c = ci.min(1);
                self.dl[c].write(ch[k] as f64);
                let y = self.dl[c].read(d);
                // the arm itself (a held dip): a pitch shifter, faded out when the arm is at rest
                let y = if arm.abs() < 0.5 { y } else {
                    let s = self.ps[c].process(y, ratio);
                    let m = ((arm.abs() - 0.5) / 2.0).min(1.0);
                    y * (1.0 - m) + s * m
                };
                ch[k] = (y * o) as f32;
            }
        }
        self.shaking = self.amt;
        meters[0] = self.amt as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Setzer End-of-Phrase Shake", values: &[] },
        Preset { name: "Quick Shake", values: &[(WAIT, 150.0), (DEPTH, 20.0), (RATE, 6.5)] },
        Preset { name: "Deep Slow Wobble", values: &[(WAIT, 500.0), (DEPTH, 45.0), (RATE, 4.0)] },
        Preset { name: "Chord Shimmer (Constant)", values: &[(MODE, 1.0), (DEPTH, 15.0), (RATE, 5.5)] },
        Preset { name: "Sleepwalk Vibrato", values: &[(MODE, 1.0), (DEPTH, 35.0), (RATE, 4.5)] },
        Preset { name: "Arm Dip (automate ARM)", values: &[(MODE, 2.0), (ARM, -30.0)] },
        Preset { name: "Dive (Manual, -1 tone)", values: &[(MODE, 2.0), (ARM, -100.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "BIGSBY",
        subtitle: ("THE VIBRATO ON SETZER'S 6120", "SHAKE · CONSTANT · MANUAL ARM"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![

            Item::Selector { p: MODE, x: 90.0, y: 165.0, label: "MODE", positions: &["SHAKE", "CONSTANT", "MANUAL"], tip: "SHAKE: vibrato eases in as a note or chord rings on (none on the attack). CONSTANT: always on. MANUAL: only the ARM." },
            Item::Knob { p: DEPTH, x: 230.0, y: 165.0, r: 38.0, label: "DEPTH", tip: "Vibrato width in cents (100 = a semitone each way).", active: None },
            Item::Knob { p: RATE, x: 360.0, y: 165.0, r: 38.0, label: "RATE", tip: "How fast the arm is shaken.", active: None },
            Item::Knob { p: WAIT, x: 490.0, y: 165.0, r: 30.0, label: "SHAKE AFTER", tip: "SHAKE mode: how long after the attack the shake starts.", active: Some(ui::ActiveWhen { p: MODE, min: 0.0, max: 0.0 }) },
            Item::Lamp { m: 0, x: 545.0, y: 232.0, label: "SHAKE", lo: 0.0, hi: 1.0 },
            Item::Divider { y: 250.0 },
            Item::Knob { p: ARM, x: 120.0, y: 320.0, r: 30.0, label: "ARM", tip: "Holds the arm down (or pulls it up a little): a pitch dip in cents. Automate it for real Bigsby dips and dives.", active: None },
            Item::Knob { p: SENS, x: 320.0, y: 320.0, r: 22.0, label: "SENSITIVITY", tip: "How big a jump in level counts as a new note or chord (resets the shake).", active: Some(ui::ActiveWhen { p: MODE, min: 0.0, max: 0.0 }) },
            Item::Knob { p: OUTPUT, x: 500.0, y: 320.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Bigsby", class_id: *b"BSzBigsbyVibPQD1", vendor: "PEQUOD Brian Setzer",
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
    fn tone(f: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n).map(|k| {
            let t = k as f64 / 48000.0;
            (1..=6).map(|h| amp / h as f64 * (2.0 * PI * f * h as f64 * t).sin()).sum::<f64>() as f32
        }).collect()
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
    fn noise(n: usize, amp: f64) -> Vec<f32> {
        let mut nz = vintage::dsp::Noise::new(11);
        (0..n).map(|_| (nz.next() * amp) as f32).collect()
    }

    /// instantaneous frequency spread from zero crossings over a window
    fn spread(y: &[f32]) -> (f64, f64) {
        let mut z = vec![];
        for i in 1..y.len() {
            if y[i - 1] < 0.0 && y[i] >= 0.0 {
                let f = y[i - 1] as f64 / (y[i - 1] as f64 - y[i] as f64);
                z.push(i as f64 - 1.0 + f);
            }
        }
        let p: Vec<f64> = z.windows(2).map(|w| 48000.0 / (w[1] - w[0])).collect();
        let lo = p.iter().cloned().fold(1e9, f64::min);
        let hi = p.iter().cloned().fold(0.0, f64::max);
        (lo, hi)
    }
    #[test]
    fn shake_waits_then_wobbles() {
        let p = with(defaults(), &[(DEPTH, 40.0), (WAIT, 400.0)]);
        let mut x: Vec<f32> = vec![0.0; 4800];
        x.extend((0..72000).map(|k| (0.3 * (2.0 * PI * 220.0 * k as f64 / 48000.0).sin()) as f32));
        let y = go(&p, &x);
        let (a0, a1) = spread(&y[4800 + 2400..4800 + 14400]);
        let (b0, b1) = spread(&y[4800 + 48000..4800 + 72000]);
        println!("first 250 ms {a0:.1}-{a1:.1} Hz; after 1 s {b0:.1}-{b1:.1} Hz");
        assert!(a1 - a0 < 1.5 && b1 - b0 > 6.0);
    }
    #[test]
    fn arm_dips_the_pitch() {
        let p = with(defaults(), &[(MODE, 2.0), (ARM, -100.0)]);
        let x: Vec<f32> = (0..48000).map(|k| (0.3 * (2.0 * PI * 440.0 * k as f64 / 48000.0).sin()) as f32).collect();
        let y = go(&p, &x);
        let a = goertzel(&y[24000..], 415.3);
        let b = goertzel(&y[24000..], 440.0);
        println!("arm -1 semitone: 415 Hz {:.1} dB vs 440 Hz {:.1} dB", db(a), db(b));
        assert!(db(a) > db(b) + 10.0);
    }

}
