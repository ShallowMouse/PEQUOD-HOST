//! Echoplex EP-3: Maestro's tape echo (Mike Battle's design, transistorised in 1970), a loop of tape in a
//! cartridge, a record head that slides along it, and the JFET preamp so many guitarists left switched on
//! for its boost alone. DELAY (the sliding head: about 60 to 750 ms), REPEATS (into self-oscillation),
//! ECHO VOLUME and RECORD VOLUME, as on the front. Tape wow and flutter, the head's bandwidth falling on
//! each repeat, the preamp's lift and edge. After Maestro's manual and published analyses; estimated.

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
    #[id = "delay"]
    pub delay: FloatParam,
    #[id = "repeats"]
    pub repeats: FloatParam,
    #[id = "echo"]
    pub echo: FloatParam,
    #[id = "record"]
    pub record: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            delay: ms("Delay", 320.0, 60.0, 750.0),
            repeats: pct("Repeats", 0.35),
            echo: pct("Echo Volume", 0.45),
            record: pct("Record Volume", 0.55),
        }
    }
}

pub const DELAY: usize = 0;
pub const REPEATS: usize = 1;
pub const ECHO: usize = 2;
pub const RECORD: usize = 3;
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
            self.delay.as_ptr(),
            self.repeats.as_ptr(),
            self.echo.as_ptr(),
            self.record.as_ptr(),
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
    line: DelayLine,
    time: Smooth,
    wow: Wander,
    flutter: Lfo,
    head: [Biquad; 2],
    pre: [Biquad; 2],
    fb: f64,
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut h0 = Biquad::identity();
        h0.set_lowpass(fs, 4200.0, 0.6);
        let mut h1 = Biquad::identity();
        h1.set_highpass(fs, 120.0, 0.6);
        // the JFET preamp's lift: a little more top and body
        let mut p0 = Biquad::identity();
        p0.set_high_shelf(fs, 2500.0, 0.7, 3.0);
        let mut p1 = Biquad::identity();
        p1.set_low_shelf(fs, 150.0, 0.7, 1.5);
        Self { fs, v, line: DelayLine::new((fs * 0.9) as usize + 8), time: Smooth::new(fs, 250.0, 320.0),
               wow: Wander::new(fs, 0.7, 17), flutter: Lfo::default(), head: [h0, h1], pre: [p0, p1], fb: 0.0 }
    }
    fn reset(&mut self) {
        self.line.reset();
        self.head.iter_mut().chain(self.pre.iter_mut()).for_each(|b| b.reset());
        self.fb = 0.0;
        self.time.snap(self.v[DELAY] as f64);
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.time.set(v[DELAY] as f64);
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nc = chans.len();
        let rep = self.v[REPEATS] as f64 * 1.08;          // past the top it runs away, as the real one does
        let echo = self.v[ECHO] as f64 * 1.2;
        let rec = 0.4 + self.v[RECORD] as f64 * 1.4;
        for k in 0..n {
            let x = if nc >= 2 { (chans[0][k] + chans[1][k]) as f64 * 0.5 } else { chans[0][k] as f64 };
            // the preamp, always in the path
            let p0 = self.pre[0].process(x * 1.25);
            let p = self.pre[1].process(p0);
            let p = (p * 1.1).tanh() / 1.1;
            // the tape: wow and flutter on the head position
            let t = self.time.next() * (1.0 + 0.0025 * self.wow.next() + 0.0012 * self.flutter.next(self.fs, 6.5));
            let back = self.line.read((t * 0.001 * self.fs).min(self.line.max_delay() - 2.0));
            let h0 = self.head[0].process(back);
            let echo_sig = self.head[1].process(h0);
            self.fb = echo_sig;
            // record: the tape saturates if it's driven hard
            let w = tape_curve((p * rec + self.fb * rep) * 0.8) / 0.8;
            self.line.write(w);
            let y = p + echo_sig * echo;
            for c in 0..nc {
                chans[c][k] = y as f32;
            }
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Slapback", values: &[(DELAY, 110.0), (REPEATS, 0.1), (ECHO, 0.5)] },
        Preset { name: "Rockabilly Double", values: &[(DELAY, 140.0), (REPEATS, 0.25), (ECHO, 0.55)] },
        Preset { name: "Page's Echo", values: &[(DELAY, 420.0), (REPEATS, 0.45), (ECHO, 0.45)] },
        Preset { name: "Runaway", values: &[(DELAY, 300.0), (REPEATS, 0.98), (ECHO, 0.4)] },
        Preset { name: "Just the Preamp", values: &[(ECHO, 0.0), (REPEATS, 0.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "ECHOPLEX EP-3",
        subtitle: ("MAESTRO TAPE ECHO, 1970", "SLIDING RECORD HEAD, THE PREAMP EVERYONE LEFT ON"),
        accent: ui::TANGERINE,
        size: (W, H),
        items: vec![
            Item::Knob { p: DELAY, x: 140.0, y: 200.0, r: 48.0, label: "DELAY", tip: "Slides the record head along the tape.", active: None },
            Item::Knob { p: REPEATS, x: 300.0, y: 200.0, r: 36.0, label: "REPEATS", tip: "Feedback; near the top it runs away.", active: None },
            Item::Knob { p: ECHO, x: 420.0, y: 200.0, r: 30.0, label: "ECHO VOLUME", tip: "How loud the echoes are.", active: None },
            Item::Knob { p: RECORD, x: 535.0, y: 200.0, r: 30.0, label: "RECORD VOLUME", tip: "Onto the tape: hotter, more saturation.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Echoplex EP-3", class_id: *b"HWEchoplexEP3PQD", vendor: "PEQUOD Hardware",
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
    fn makes_sound() {
        let p = with(defaults(), &[]);
        let y = go(&p, &sine(440.0, 0.2, 48000));
        let l = db(rms(&y[24000..]));
        println!("level {:.1} dB", l);
        assert!(l > -40.0 && y.iter().all(|v| v.is_finite()));
    }

}
