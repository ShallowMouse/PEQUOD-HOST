//! Jazz Chorus: clean solid-state stereo amp with built-in chorus, after the Roland JC-120 Andy Summers used with chorus always on. As Padgham recorded it: one speaker carries the dry guitar, the other the chorused one, each close-miked and panned.

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
pub const H: u32 = 432;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Effect { #[name = "Off"] Off, #[name = "Chorus"] Chorus, #[name = "Vibrato"] Vibrato }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Cab { #[name = "2x12 Mic'd"] Mic, #[name = "DI"] Di }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Bright { #[name = "Off"] Off, #[name = "On"] On }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "volume"]
    pub volume: FloatParam,
    #[id = "bright"]
    pub bright: EnumParam<Bright>,
    #[id = "treble"]
    pub treble: FloatParam,
    #[id = "middle"]
    pub middle: FloatParam,
    #[id = "bass"]
    pub bass: FloatParam,
    #[id = "dist"]
    pub dist: FloatParam,
    #[id = "effect"]
    pub effect: EnumParam<Effect>,
    #[id = "speed"]
    pub speed: FloatParam,
    #[id = "depth"]
    pub depth: FloatParam,
    #[id = "cab"]
    pub cab: EnumParam<Cab>,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            volume: pct("Volume", 0.4),
            bright: EnumParam::new("Bright", Bright::Off),
            treble: db("Treble", 0.0, -12.0, 12.0),
            middle: db("Middle", 0.0, -12.0, 12.0),
            bass: db("Bass", 0.0, -12.0, 12.0),
            dist: pct("Distortion", 0.0),
            effect: EnumParam::new("Effect", Effect::Chorus),
            speed: rate("Speed", 0.6, 0.1, 8.0),
            depth: pct("Depth", 0.5),
            cab: EnumParam::new("Output", Cab::Mic),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const VOLUME: usize = 0;
pub const BRIGHT: usize = 1;
pub const TREBLE: usize = 2;
pub const MIDDLE: usize = 3;
pub const BASS: usize = 4;
pub const DIST: usize = 5;
pub const EFFECT: usize = 6;
pub const SPEED: usize = 7;
pub const DEPTH: usize = 8;
pub const CAB: usize = 9;
pub const OUTPUT: usize = 10;
pub const NPARAMS: usize = 11;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.volume.as_ptr(),
            self.bright.as_ptr(),
            self.treble.as_ptr(),
            self.middle.as_ptr(),
            self.bass.as_ptr(),
            self.dist.as_ptr(),
            self.effect.as_ptr(),
            self.speed.as_ptr(),
            self.depth.as_ptr(),
            self.cab.as_ptr(),
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
    eq: [Biquad; 4],
    key: [f64; 4],
    bbd: Bbd,
    lfo: Lfo,
    os: Oversampler,
    cab: [[Biquad; 3]; 2],
    v: [f32; NPARAMS],
    out: Smooth,
    vol: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut cab = [[Biquad::identity(); 3]; 2];
        for c in &mut cab {
            // open-back 2x12 close-miked with a dynamic mic (estimated response)
            c[0].set_highpass(fs, 85.0, 0.8);
            c[1].set_peak(fs, 2600.0, 1.0, 4.0);
            c[2].set_lowpass(fs, 5500.0, 0.75);
        }
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self {
            fs,
            eq: [Biquad::identity(); 4],
            key: [f64::NAN; 4],
            bbd: Bbd::new(fs, 30.0),
            lfo: Lfo::default(),
            os: Oversampler::new(),
            cab,
            v,
            out: Smooth::new(fs, 20.0, 1.0),
            vol: Smooth::new(fs, 20.0, 0.4),
        }
    }
    fn reset(&mut self) {
        self.eq.iter_mut().for_each(|b| b.reset());
        self.bbd.reset();
        self.os.reset();
        for c in &mut self.cab {
            c.iter_mut().for_each(|b| b.reset());
        }
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = [v[TREBLE] as f64, v[MIDDLE] as f64, v[BASS] as f64, v[BRIGHT] as f64];
        if k != self.key {
            self.eq[0].set_high_shelf(self.fs, 3000.0, 0.7, k[0]);
            self.eq[1].set_peak(self.fs, 700.0, 0.7, k[1]);
            self.eq[2].set_low_shelf(self.fs, 120.0, 0.7, k[2]);
            self.eq[3].set_high_shelf(self.fs, 5000.0, 0.7, if k[3] > 0.5 { 6.0 } else { 0.0 });
            self.key = k;
        }
        self.vol.set(v[VOLUME] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let dist = self.v[DIST] as f64;
        let effect = self.v[EFFECT].round() as usize;
        let speed = self.v[SPEED] as f64;
        let depth = self.v[DEPTH] as f64;
        let mic = self.v[CAB] < 0.5;
        let mut peak = 0.0f64;
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            let vol = self.vol.next();
            // solid-state preamp: clean until the distortion knob or a hot input
            let drive = 0.5 + 6.0 * vol + 30.0 * dist;
            let y = self.os.process(x * drive, |s| if dist > 0.0 { (s * (1.0 + 4.0 * dist)).tanh() / (1.0 + 2.0 * dist) } else { s / (1.0 + 0.02 * s * s).sqrt() });
            let y = cascade(&mut self.eq, y) * 0.5;
            // chorus: BBD around 5 ms; one speaker dry, the other modulated
            let l = self.lfo.next(self.fs, speed);
            let d_ms = 5.0 + depth * 3.5 * l;
            let wet = self.bbd.process(y, d_ms * 0.001 * self.fs);
            let (a, b) = match effect {
                0 => (y, y),
                1 => (y, wet),
                _ => (wet, wet),
            };
            let o = self.out.next();
            let mut outs = [a, b];
            if mic {
                for (c, s) in outs.iter_mut().enumerate() {
                    *s = cascade(&mut self.cab[c], *s);
                }
            }
            for (ci, c) in chans.iter_mut().enumerate() {
                let z = if nch == 1 { 0.5 * (outs[0] + outs[1]) } else { outs[ci.min(1)] } * o;
                peak = peak.max(z.abs());
                c[k] = z as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Every Breath (chorus always on)", values: &[(VOLUME, 0.4), (EFFECT, 1.0), (SPEED, 0.6), (DEPTH, 0.45), (TREBLE, 2.0)] },
        Preset { name: "Message in a Bottle Arpeggio", values: &[(VOLUME, 0.45), (EFFECT, 1.0), (SPEED, 0.8), (DEPTH, 0.55), (BRIGHT, 1.0)] },
        Preset { name: "Clean Bright", values: &[(VOLUME, 0.35), (EFFECT, 0.0), (BRIGHT, 1.0), (TREBLE, 3.0)] },
        Preset { name: "Vibrato Warble", values: &[(EFFECT, 2.0), (SPEED, 5.0), (DEPTH, 0.4)] },
        Preset { name: "Slow Lush Chorus", values: &[(EFFECT, 1.0), (SPEED, 0.3), (DEPTH, 0.8), (MIDDLE, -3.0)] },
        Preset { name: "JC Distortion", values: &[(DIST, 0.6), (VOLUME, 0.5), (EFFECT, 1.0), (OUTPUT, -6.0)] },
        Preset { name: "DI Chorus (for your own cab)", values: &[(CAB, 1.0), (EFFECT, 1.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "JAZZ CHORUS",
        subtitle: ("AFTER THE ROLAND JC-120", "SOLID STATE · STEREO CHORUS · 2 X 12"),
        accent: ui::OXBLOOD,
        size: (W, H),
        items: vec![

            Item::Knob { p: VOLUME, x: 70.0, y: 160.0, r: 32.0, label: "VOLUME", tip: "Volume. Stays clean: this amp's point is headroom.", active: None },
            Item::Knob { p: TREBLE, x: 165.0, y: 160.0, r: 26.0, label: "TREBLE", tip: "Treble.", active: None },
            Item::Knob { p: MIDDLE, x: 245.0, y: 160.0, r: 26.0, label: "MIDDLE", tip: "Middle.", active: None },
            Item::Knob { p: BASS, x: 325.0, y: 160.0, r: 26.0, label: "BASS", tip: "Bass.", active: None },
            Item::Knob { p: DIST, x: 415.0, y: 160.0, r: 26.0, label: "DISTORTION", tip: "The built-in distortion.", active: None },
            Item::Selector { p: EFFECT, x: 540.0, y: 160.0, label: "EFFECT", positions: &["OFF", "CHORUS", "VIB"], tip: "Chorus: one speaker dry, the other chorused, mixing in the air (or here, across your stereo pair). Vibrato: both modulated." },
            Item::Divider { y: 264.0 },
            Item::Switch { p: BRIGHT, x: 70.0, y: 330.0, labels: ["OFF", "BRIGHT"], tip: "Bright switch." },
            Item::Selector { p: CAB, x: 185.0, y: 330.0, label: "OUTPUT", positions: &["MIC'D", "DI"], tip: "MIC'D: the two 12-inch speakers each close-miked (with a 421, as Padgham did), panned left and right. DI: no speaker." },
            Item::Knob { p: SPEED, x: 300.0, y: 330.0, r: 22.0, label: "SPEED", tip: "Chorus/vibrato speed.", active: None },
            Item::Knob { p: DEPTH, x: 390.0, y: 330.0, r: 22.0, label: "DEPTH", tip: "Chorus/vibrato depth.", active: None },
            Item::Lamp { m: 0, x: 460.0, y: 330.0, label: "LEVEL", lo: 0.05, hi: 0.9 },
            Item::Knob { p: OUTPUT, x: 575.0, y: 330.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Jazz Chorus", class_id: *b"TPJazzChorusPQD1", vendor: "PEQUOD Padgham & Lillywhite",
    categories: [Fx, Modulation]
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
    fn chorus_splits_dry_and_wet() {
        let p = with(defaults(), &[(EFFECT, 1.0), (CAB, 1.0)]);
        let s = sine(440.0, 0.1, 48000);
        let mut input = vec![s.clone(), s];
        run(&p, &mut input);
        let diff: Vec<f32> = input[0].iter().zip(&input[1]).map(|(a, b)| a - b).collect();
        println!("L-R {:.1} dB rel L", db(rms(&diff[24000..]) / rms(&input[0][24000..])));
        assert!(rms(&diff[24000..]) > 0.1 * rms(&input[0][24000..]));
        let p = with(defaults(), &[(EFFECT, 0.0), (CAB, 1.0)]);
        let s = sine(440.0, 0.1, 24000);
        let mut input = vec![s.clone(), s];
        run(&p, &mut input);
        assert!(input[0].iter().zip(&input[1]).all(|(a, b)| (a - b).abs() < 1e-6));
    }

}
