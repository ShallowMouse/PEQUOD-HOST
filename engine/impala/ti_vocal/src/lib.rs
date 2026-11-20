//! Lonerism Vocal: Kevin Parker's vocal chain of the Innerspeaker and Lonerism years. A Bellari RP503 valve
//! mic preamp (Gearspace's list of his chain), pushed into heavy compression, then a Roland RE-201 Space Echo,
//! which Parker used on his vocals throughout: the slapback and short repeats that put the voice in its
//! Lennon-ish haze, plus a phaser to swirl it as on the Lonerism records. Drive, compression, echo time,
//! repeats, the tape's wobble, the phaser, and a low cut. Settings are estimated.

#![allow(clippy::excessive_precision)]
use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
#[allow(unused_imports)]
use vintage::dsp::*;
#[allow(unused_imports)]
use vintage::fx::*;
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

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "comp"]
    pub comp: FloatParam,
    #[id = "time"]
    pub time: FloatParam,
    #[id = "repeats"]
    pub repeats: FloatParam,
    #[id = "echo"]
    pub echo: FloatParam,
    #[id = "wobble"]
    pub wobble: FloatParam,
    #[id = "phase"]
    pub phase: FloatParam,
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
            drive: pct("Valve Drive", 0.35),
            comp: pct("Compression", 0.6),
            time: ms("Echo Time", 140.0, 50.0, 600.0),
            repeats: pct("Repeats", 0.3),
            echo: pct("Echo Level", 0.4),
            wobble: pct("Wobble", 0.35),
            phase: pct("Phaser", 0.0),
            lowcut: hz("Low Cut", 120.0, 20.0, 400.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const DRIVE: usize = 0;
pub const COMP: usize = 1;
pub const TIME: usize = 2;
pub const REPEATS: usize = 3;
pub const ECHO: usize = 4;
pub const WOBBLE: usize = 5;
pub const PHASE: usize = 6;
pub const LOWCUT: usize = 7;
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
            self.drive.as_ptr(),
            self.comp.as_ptr(),
            self.time.as_ptr(),
            self.repeats.as_ptr(),
            self.echo.as_ptr(),
            self.wobble.as_ptr(),
            self.phase.as_ptr(),
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

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    lc: Vec<Biquad>,
    env: f64,
    gr: f64,
    tape: Vec<DelayLine>,
    fb_tone: Vec<[Biquad; 2]>,
    fb: Vec<f64>,
    wow: Wander,
    flutter: f64,
    ap: Vec<[f64; 8]>,
    ph: f64,
    key: i32,
    out: Smooth,
    time: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let f = self.v[LOWCUT] as f64;
        for b in self.lc.iter_mut() {
            b.set_highpass(self.fs, f, 0.7);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let n = ch.max(2);
        // the echo's repeats lose top and bottom each time round the tape loop
        let mut t = [Biquad::identity(); 2];
        t[0].set_lowpass(fs, 3800.0, 0.6);
        t[1].set_highpass(fs, 150.0, 0.6);
        let mut d = Self {
            fs, v, lc: vec![Biquad::identity(); n], env: 0.0, gr: 0.0,
            tape: (0..n).map(|_| DelayLine::new((0.7 * fs) as usize)).collect(),
            fb_tone: vec![t; n], fb: vec![0.0; n], wow: Wander::new(fs, 0.7, 5), flutter: 0.0,
            ap: vec![[0.0; 8]; n], ph: 0.0, key: -1,
            out: Smooth::new(fs, 20.0, 1.0), time: Smooth::new(fs, 120.0, 140.0),
        };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.lc.iter_mut().for_each(|b| b.reset());
        self.tape.iter_mut().for_each(|t| t.reset());
        self.fb_tone.iter_mut().flatten().for_each(|b| b.reset());
        self.fb.iter_mut().for_each(|x| *x = 0.0);
        self.ap.iter_mut().for_each(|a| *a = [0.0; 8]);
        self.env = 0.0;
        self.gr = 0.0;
        self.time.snap(self.v[TIME] as f64);
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = v[LOWCUT] as i32;
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.time.set(v[TIME] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let v = self.v;
        let fs = self.fs;
        let drive = v[DRIVE] as f64;
        let dg = 1.0 + 8.0 * drive * drive;
        let comp = v[COMP] as f64;
        let th = -10.0 - 26.0 * comp;
        let ratio = 1.5 + 8.0 * comp;
        let (ca, cr) = (coeff(fs, 3.0), coeff(fs, 150.0));
        let rep = v[REPEATS] as f64 * 0.85;
        let echo = v[ECHO] as f64;
        let wob = v[WOBBLE] as f64;
        let phase = v[PHASE] as f64;
        let last = self.lc.len() - 1;
        let mut most = 0.0f64;
        for k in 0..n {
            let o = self.out.next();
            let t = self.time.next() * 0.001 * fs;
            // the RE-201's wow (slow wander) and flutter (the capstan, ~ 7 Hz)
            self.flutter = (self.flutter + 7.0 / fs).fract();
            let mo = (self.wow.next() * 0.006 + (std::f64::consts::TAU * self.flutter).sin() * 0.0015) * wob;
            // the phaser's sweep (a Small Stone-like 4-stage pair, slow)
            self.ph = (self.ph + 0.35 / fs).fract();
            let sweep = 0.5 + 0.5 * (std::f64::consts::TAU * self.ph).sin();
            let fa = 300.0 * (1.0 + 9.0 * sweep);
            let tn = (std::f64::consts::PI * fa / fs).tan();
            let apc = (tn - 1.0) / (tn + 1.0);
            // the compressor keys off the driven voice
            let key = chans.iter().map(|c| (c[k] as f64).abs()).fold(0.0, f64::max) * dg;
            let c = if key > self.env { ca } else { cr };
            self.env = key + c * (self.env - key);
            let want = comp_gr(gain_to_db(self.env.max(1e-9)), th, ratio, 6.0);
            self.gr = want;
            most = most.max(want);
            let g = db_to_gain(-want + want * 0.35);
            for (ci, ch) in chans.iter_mut().enumerate() {
                let j = ci.min(last);
                let x = self.lc[j].process(ch[k] as f64);
                let pre = tube(x * dg, 0.1) / dg.sqrt();
                let dry = pre * g;
                // the tape loop
                self.tape[j].write(dry + self.fb[j]);
                let e = self.tape[j].read(t * (1.0 + mo)).max(-2.0).min(2.0);
                self.fb[j] = cascade(&mut self.fb_tone[j], tape_curve(e)) * rep;
                let mut y = dry + e * echo;
                if phase > 0.0 {
                    let mut a = y;
                    for s in self.ap[j].iter_mut().take(6) {
                        let out = apc * a + *s;
                        *s = a - apc * out;
                        a = out;
                    }
                    y = y * (1.0 - 0.5 * phase) + a * 0.5 * phase;
                }
                ch[k] = (y * o) as f32;
            }
        }
        meters[0] = most as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Lonerism Lead Vocal", values: &[] },
        Preset { name: "Innerspeaker Slapback", values: &[(TIME, 95.0), (REPEATS, 0.1), (ECHO, 0.5), (DRIVE, 0.45)] },
        Preset { name: "Lennon Haze", values: &[(TIME, 180.0), (REPEATS, 0.35), (ECHO, 0.5), (WOBBLE, 0.5), (PHASE, 0.35)] },
        Preset { name: "Swirling Backing Vocals", values: &[(TIME, 260.0), (REPEATS, 0.5), (ECHO, 0.45), (PHASE, 0.8), (LOWCUT, 220.0)] },
        Preset { name: "Dry and Squashed", values: &[(ECHO, 0.0), (COMP, 0.85), (DRIVE, 0.25)] },
        Preset { name: "Runaway Tape", values: &[(TIME, 330.0), (REPEATS, 0.95), (ECHO, 0.6), (WOBBLE, 0.7)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "LONERISM VOCAL",
        subtitle: ("VALVE PRE · HEAVY COMPRESSION · SPACE ECHO · PHASER", "KEVIN PARKER'S VOCAL CHAIN"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![
            Item::Knob { p: LOWCUT, x: 60.0, y: 140.0, r: 20.0, label: "LOW CUT", tip: "High-pass before the preamp.", active: None },
            Item::Knob { p: DRIVE, x: 150.0, y: 140.0, r: 24.0, label: "VALVE DRIVE", tip: "How hard the valve preamp is pushed.", active: None },
            Item::Knob { p: COMP, x: 250.0, y: 140.0, r: 24.0, label: "COMPRESSION", tip: "How hard the voice is squashed.", active: None },
            Item::Meter { m: 0, x: 330.0, y: 95.0, w: 140.0, h: 90.0, kind: MeterKind::GainReduction, caption: "COMP" },
            Item::Knob { p: PHASE, x: 560.0, y: 140.0, r: 24.0, label: "PHASER", tip: "A slow phaser swirling the voice and its echo.", active: None },
            Item::Divider { y: 222.0 },
            Item::Text { x: 320.0, y: 240.0, text: "SPACE ECHO", size: 12.0, align: Align2::CENTER_CENTER },
            Item::Knob { p: TIME, x: 110.0, y: 320.0, r: 28.0, label: "ECHO TIME", tip: "The repeat time: short for slapback, longer for echoes.", active: None },
            Item::Knob { p: REPEATS, x: 230.0, y: 320.0, r: 24.0, label: "REPEATS", tip: "Intensity: how many times it comes back round the tape.", active: None },
            Item::Knob { p: ECHO, x: 340.0, y: 320.0, r: 24.0, label: "ECHO LEVEL", tip: "How loud the echo is against the voice.", active: None },
            Item::Knob { p: WOBBLE, x: 450.0, y: 320.0, r: 22.0, label: "WOBBLE", tip: "The tape's wow and flutter on the repeats.", active: None },
            Item::Knob { p: OUTPUT, x: 560.0, y: 320.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Lonerism Vocal", class_id: *b"TImpLonerismVox1", vendor: "PEQUOD Tame Impala",
    categories: [Fx, Delay]
);

#[cfg(test)]
#[allow(dead_code)]
pub mod tests {
    use super::*;
    use std::f64::consts::PI;

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
    pub fn sine(f: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n).map(|k| (amp * (2.0 * PI * f * k as f64 / 48000.0).sin()) as f32).collect()
    }
    pub fn noise(n: usize, amp: f64) -> Vec<f32> {
        let mut nz = vintage::dsp::Noise::new(11);
        (0..n).map(|_| (nz.next() * amp) as f32).collect()
    }
    /// Both channels out for a mono input.
    pub fn go2(p: &[f32], x: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        let r = input.pop().unwrap();
        (input.pop().unwrap(), r)
    }
    pub fn go(p: &[f32], x: &[f32]) -> Vec<f32> {
        go2(p, x).0
    }
    /// Steady-state level of a sine at f through the settings, in dB.
    pub fn level(set: &[(usize, f32)], f: f64) -> f64 {
        let p = with(defaults(), set);
        let y = go(&p, &sine(f, 0.1, 48000));
        db(rms(&y[24000..]))
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
            let l: Vec<f32> = (0..n).map(|k| if (k / 6000) % 2 == 0 { (nz.next() * 0.5) as f32 } else { 0.0 }).collect();
            let mut input = vec![l.clone(), l];
            run(&p, &mut input);
            assert!(input.iter().flatten().all(|v| v.is_finite() && v.abs() < 8.0), "unbounded output for {:?}", p);
        }
    }

    #[test]
    fn echo_comes_back_at_its_time() {
        let p = with(defaults(), &[(TIME, 200.0), (WOBBLE, 0.0), (REPEATS, 0.0), (COMP, 0.0), (DRIVE, 0.0), (LOWCUT, 20.0)]);
        let mut x = vec![0.0f32; 48000];
        for k in 0..240 { x[k] = (0.3 * (k as f64 * 0.2).sin()) as f32; }
        let y = go(&p, &x);
        let at = |a: usize, b: usize| rms(&y[a..b]);
        println!("at 200 ms {:.4}, at 100 ms {:.4}", at(9500, 10000), at(4700, 5200));
        assert!(at(9500, 10000) > 5.0 * at(4700, 5200));
    }

}
