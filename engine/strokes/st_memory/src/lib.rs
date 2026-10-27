//! Memory Man: an analogue echo with chorus/vibrato after the Electro-Harmonix Deluxe Memory Man on Nick Valensi's earlier pedalboards.

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
pub enum Mod { #[name = "Chorus"] Chorus, #[name = "Vibrato"] Vibrato }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "delay"]
    pub delay: FloatParam,
    #[id = "feedback"]
    pub feedback: FloatParam,
    #[id = "blend"]
    pub blend: FloatParam,
    #[id = "mode"]
    pub mode: EnumParam<Mod>,
    #[id = "depth"]
    pub depth: FloatParam,
    #[id = "level"]
    pub level: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            delay: ms("Delay", 330.0, 30.0, 550.0),
            feedback: pct("Feedback", 0.4),
            blend: pct("Blend", 0.45),
            mode: EnumParam::new("Modulation", Mod::Chorus),
            depth: pct("Depth", 0.2),
            level: db("Level", 0.0, -12.0, 18.0),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const DELAY: usize = 0;
pub const FEEDBACK: usize = 1;
pub const BLEND: usize = 2;
pub const MODE: usize = 3;
pub const DEPTH: usize = 4;
pub const LEVEL: usize = 5;
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
            self.feedback.as_ptr(),
            self.blend.as_ptr(),
            self.mode.as_ptr(),
            self.depth.as_ptr(),
            self.level.as_ptr(),
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
    lfo: Lfo,
    fb: [f64; 2],
    hp: [OnePole; 2],
    delay: Smooth,
    feedback: f64,
    blend: Smooth,
    vibrato: bool,
    depth: f64,
    level: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut hp = [OnePole::default(); 2];
        for h in &mut hp {
            h.set_lowpass(fs, 80.0);
        }
        Self {
            fs,
            bbd: [Bbd::new(fs, 600.0), Bbd::new(fs, 600.0)],
            comp: [Compander::new(fs), Compander::new(fs)],
            lfo: Lfo::default(),
            fb: [0.0; 2],
            hp,
            delay: Smooth::new(fs, 200.0, 330.0),
            feedback: 0.4,
            blend: Smooth::new(fs, 20.0, 0.45),
            vibrato: false,
            depth: 0.2,
            level: Smooth::new(fs, 20.0, 1.0),
            out: Smooth::new(fs, 20.0, 1.0),
        }
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.bbd[c].reset();
            self.comp[c].reset();
            self.hp[c].reset();
        }
        self.fb = [0.0; 2];
    }
    fn set_params(&mut self, v: &[f32]) {
        self.delay.set(v[DELAY] as f64);
        self.feedback = v[FEEDBACK] as f64 * 1.02;
        self.blend.set(v[BLEND] as f64);
        self.vibrato = v[MODE] > 0.5;
        self.depth = v[DEPTH] as f64;
        self.level.set(db_to_gain(v[LEVEL] as f64));
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let mut peak = 0.0f64;
        for k in 0..n {
            let d = self.delay.next();
            // 2 x 4096-stage chips: clock = 8192 / (2 d); usable bandwidth ~ 0.4 x clock
            let bw = (0.4 * 8192.0 / (2.0 * d * 0.001)).clamp(1500.0, 12000.0);
            let lfo = self.lfo.next(self.fs, if self.vibrato { 5.5 } else { 0.9 });
            let lvl = self.level.next();
            let bl = self.blend.next();
            let o = self.out.next();
            for c in 0..nch.min(2) {
                self.bbd[c].set_bandwidth(bw);
                let x = chans[c][k] as f64 * lvl;
                // preamp: the Level control drives a soft-clipping op-amp stage
                let x = (x * 1.2).tanh() / 1.2;
                let m = if c == 0 { lfo } else { -lfo };
                let dm = d * (1.0 + self.depth * if self.vibrato { 0.012 } else { 0.006 } * m);
                let enc = self.comp[c].encode(x + self.fb[c] * self.feedback);
                let y = self.comp[c].decode(self.bbd[c].process(enc, dm * 0.001 * self.fs));
                let y = (y - self.hp[c].lp(y)).clamp(-3.0, 3.0);
                self.fb[c] = y;
                let z = x * (1.0 - bl) + y * bl;
                let z = z * o;
                peak = peak.max(z.abs());
                chans[c][k] = z as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Valensi Short Echo", values: &[(DELAY, 180.0), (FEEDBACK, 0.25), (BLEND, 0.35), (DEPTH, 0.1)] },
        Preset { name: "Slapback", values: &[(DELAY, 95.0), (FEEDBACK, 0.1), (BLEND, 0.4), (DEPTH, 0.0)] },
        Preset { name: "Room on Fire Wobble", values: &[(DELAY, 30.0), (FEEDBACK, 0.0), (BLEND, 0.45), (DEPTH, 0.6)] },
        Preset { name: "Long Lead Echo", values: &[(DELAY, 420.0), (FEEDBACK, 0.45), (BLEND, 0.35)] },
        Preset { name: "Vibrato (wet only)", values: &[(MODE, 1.0), (DELAY, 30.0), (FEEDBACK, 0.0), (BLEND, 1.0), (DEPTH, 0.45)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "MEMORY MAN",
        subtitle: ("AFTER THE EHX DELUXE MEMORY MAN", "ON VALENSI'S EARLY BOARD"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![

            Item::Knob { p: DELAY, x: 100.0, y: 160.0, r: 46.0, label: "DELAY", tip: "Delay time, 30 to 550 ms. Longer settings clock the chips slower: darker repeats.", active: None },
            Item::Knob { p: FEEDBACK, x: 250.0, y: 160.0, r: 40.0, label: "FEEDBACK", tip: "Repeats. Past about 90% it runs away into oscillation, as the pedal does.", active: None },
            Item::Knob { p: BLEND, x: 390.0, y: 160.0, r: 40.0, label: "BLEND", tip: "Direct vs echo.", active: None },
            Item::Switch { p: MODE, x: 530.0, y: 140.0, labels: ["CHORUS", "VIBRATO"], tip: "Modulation of the delay: slow chorus or fast vibrato." },
            Item::Knob { p: DEPTH, x: 530.0, y: 205.0, r: 24.0, label: "DEPTH", tip: "Modulation depth.", active: None },
            Item::Divider { y: 264.0 },
            Item::Knob { p: LEVEL, x: 160.0, y: 330.0, r: 24.0, label: "LEVEL", tip: "Input level; turned up it drives the preamp into soft clipping.", active: None },
            Item::Knob { p: OUTPUT, x: 480.0, y: 330.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },
            Item::Lamp { m: 0, x: 300.0, y: 330.0, label: "LEVEL", lo: 0.05, hi: 0.9 },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Memory Man", class_id: *b"StrMemoryManPQD1", vendor: "PEQUOD The Strokes",
    categories: [Fx, Delay]
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
    fn echo_at_delay_and_darker_when_long() {
        let p = with(defaults(), &[(DELAY, 200.0), (FEEDBACK, 0.0), (BLEND, 1.0), (DEPTH, 0.0)]);
        let mut x = vec![0.0f32; 72000];
        for v in x.iter_mut().skip(48000).take(48) {
            *v = 0.3;
        }
        let mut input = vec![x];
        run(&p, &mut input);
        let pk = input[0].iter().enumerate().skip(50000).max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap()).unwrap().0 as f64 / 48.0 - 1000.0;
        println!("echo at {pk:.1} ms");
        assert!((pk - 200.0).abs() < 4.0);
        let hi = |d: f32| {
            let p = with(defaults(), &[(DELAY, d), (FEEDBACK, 0.0), (BLEND, 1.0), (DEPTH, 0.0)]);
            let mut input = vec![sine(5000.0, 0.1, 96000)];
            run(&p, &mut input);
            rms(&input[0][60000..])
        };
        assert!(hi(550.0) < hi(100.0) * 0.3);
    }

}
