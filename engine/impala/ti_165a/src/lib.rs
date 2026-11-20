//! Kit Crush 165A: the dbx 165A compressor as Kevin Parker used it on the drums of Innerspeaker and after.
//! The dbx 165A was a key purchase for Innerspeaker and, with the kit's mics summed to mono through a small
//! mixer into it, behind his crunchy, Bonham-like drum sound (Sound On Sound, 2020). The 165A is an RMS
//! compressor with dbx's Over Easy soft knee, ratios from 1:1 up to limiting, and manual attack and release. Here with the mono summing, the small mixer's push, and a
//! blend for parallel use. Curves follow dbx's published behaviour; the mixer's drive is estimated.

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
    #[id = "mono"]
    pub mono: BoolParam,
    #[id = "push"]
    pub push: FloatParam,
    #[id = "thresh"]
    pub thresh: FloatParam,
    #[id = "ratio"]
    pub ratio: FloatParam,
    #[id = "overeasy"]
    pub overeasy: BoolParam,
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "release"]
    pub release: FloatParam,
    #[id = "blend"]
    pub blend: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            mono: BoolParam::new("Sum to Mono", true),
            push: pct("Mixer Push", 0.35),
            thresh: db("Threshold", -24.0, -50.0, 0.0),
            ratio: FloatParam::new("Ratio", 8.0, FloatRange::Skewed { min: 1.0, max: 40.0, factor: 0.35 }).with_step_size(0.1).with_unit(":1"),
            overeasy: BoolParam::new("Over Easy", true),
            attack: ms("Attack", 8.0, 0.4, 60.0),
            release: ms("Release", 120.0, 20.0, 2000.0),
            blend: pct("Blend", 1.0),
            output: db("Output", 8.0, -24.0, 24.0),
        }
    }
}

pub const MONO: usize = 0;
pub const PUSH: usize = 1;
pub const THRESH: usize = 2;
pub const RATIO: usize = 3;
pub const OVEREASY: usize = 4;
pub const ATTACK: usize = 5;
pub const RELEASE: usize = 6;
pub const BLEND: usize = 7;
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
            self.mono.as_ptr(),
            self.push.as_ptr(),
            self.thresh.as_ptr(),
            self.ratio.as_ptr(),
            self.overeasy.as_ptr(),
            self.attack.as_ptr(),
            self.release.as_ptr(),
            self.blend.as_ptr(),
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
    rms: f64,
    gr: f64,
    tone: [[Biquad; 2]; 2],
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut t = Biquad::identity();
        // the little mixer's transformer and op-amps: a touch of low-mid weight, top eased off
        t.set_peak(fs, 110.0, 0.8, 1.2);
        let mut l = Biquad::identity();
        l.set_lowpass(fs, 16000.0f64.min(fs * 0.45), 0.7);
        Self { fs, v, rms: 0.0, gr: 0.0, tone: [[t, l], [t, l]], out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.rms = 0.0;
        self.gr = 0.0;
        self.tone.iter_mut().flatten().for_each(|b| b.reset());
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let v = self.v;
        let fs = self.fs;
        let mono = v[MONO] > 0.5;
        let push = v[PUSH] as f64;
        let th = v[THRESH] as f64;
        let ratio = v[RATIO] as f64;
        let knee = if v[OVEREASY] > 0.5 { 14.0 } else { 0.0 };
        // RMS detection (a ~ 5 ms window), then the attack and release on the gain
        let crms = coeff(fs, 5.0);
        let ca = coeff(fs, v[ATTACK] as f64);
        let cr = coeff(fs, v[RELEASE] as f64);
        let blend = v[BLEND] as f64;
        let nch = chans.len().max(1);
        let mut most = 0.0f64;
        for k in 0..n {
            let o = self.out.next();
            let sum = chans.iter().map(|c| c[k] as f64).sum::<f64>() / nch as f64;
            let key = if mono { sum } else { chans.iter().map(|c| (c[k] as f64).abs()).fold(0.0, f64::max) };
            self.rms = key * key + crms * (self.rms - key * key);
            let lvl = 10.0 * self.rms.max(1e-12).log10();
            let want = comp_gr(lvl, th, ratio, knee);
            let c = if want > self.gr { ca } else { cr };
            self.gr = want + c * (self.gr - want);
            most = most.max(self.gr);
            let g = db_to_gain(-self.gr);
            for (ci, ch) in chans.iter_mut().enumerate() {
                let x = if mono { sum } else { ch[k] as f64 };
                // the summing mixer pushed: soft, a little uneven
                let d = 1.0 + 6.0 * push * push;
                let y = tube(x * d, 0.08) / d.sqrt();
                let wet = cascade(&mut self.tone[ci.min(1)], y) * g;
                let dry = x;
                ch[k] = ((wet * blend + dry * (1.0 - blend)) * o) as f32;
            }
        }
        meters[0] = most as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Innerspeaker Kit Crunch", values: &[] },
        Preset { name: "Bonham Room Squash", values: &[(THRESH, -32.0), (RATIO, 20.0), (ATTACK, 15.0), (RELEASE, 250.0), (PUSH, 0.5), (OUTPUT, 12.0)] },
        Preset { name: "Over the Top", values: &[(THRESH, -36.0), (RATIO, 40.0), (ATTACK, 3.0), (RELEASE, 80.0), (OVEREASY, 0.0), (PUSH, 0.6), (OUTPUT, 14.0)] },
        Preset { name: "Parallel Smash", values: &[(THRESH, -40.0), (RATIO, 30.0), (ATTACK, 1.0), (RELEASE, 60.0), (BLEND, 0.45), (OUTPUT, 4.0)] },
        Preset { name: "Gentle Glue (Stereo)", values: &[(MONO, 0.0), (THRESH, -14.0), (RATIO, 2.5), (ATTACK, 20.0), (RELEASE, 300.0), (PUSH, 0.1), (OUTPUT, 2.0)] },
        Preset { name: "Bass Through the 165A", values: &[(MONO, 0.0), (THRESH, -22.0), (RATIO, 5.0), (ATTACK, 25.0), (RELEASE, 200.0), (PUSH, 0.25), (OUTPUT, 6.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "KIT CRUSH 165A",
        subtitle: ("THE DBX 165A ON THE DRUMS, SUMMED TO MONO", "OVER EASY · RMS · RATIO TO LIMITING"),
        accent: ui::GRAPHITE,
        size: (W, H),
        items: vec![
            Item::Switch { p: MONO, x: 70.0, y: 120.0, labels: ["STEREO", "MONO"], tip: "Sum the kit to mono first, as the mics were summed through a small mixer before the 165A." },
            Item::Knob { p: PUSH, x: 70.0, y: 220.0, r: 24.0, label: "MIXER PUSH", tip: "How hard the summing mixer is driven: a little crunch and weight.", active: None },
            Item::Knob { p: THRESH, x: 185.0, y: 160.0, r: 28.0, label: "THRESHOLD", tip: "Where compression starts.", active: None },
            Item::Knob { p: RATIO, x: 300.0, y: 160.0, r: 28.0, label: "RATIO", tip: "1:1 up to 40:1, which is effectively limiting.", active: None },
            Item::Switch { p: OVEREASY, x: 300.0, y: 245.0, labels: ["HARD", "OVER EASY"], tip: "dbx's soft knee." },
            Item::Knob { p: ATTACK, x: 480.0, y: 100.0, r: 22.0, label: "ATTACK", tip: "How quickly it clamps down: slower lets the stick through.", active: None },
            Item::Knob { p: RELEASE, x: 580.0, y: 100.0, r: 22.0, label: "RELEASE", tip: "How quickly it lets go: fast makes the room and cymbals pump up.", active: None },
            Item::Meter { m: 0, x: 470.0, y: 175.0, w: 150.0, h: 100.0, kind: MeterKind::GainReduction, caption: "GAIN REDUCTION" },
            Item::Divider { y: 290.0 },
            Item::Knob { p: BLEND, x: 240.0, y: 350.0, r: 22.0, label: "BLEND", tip: "Compressed against untouched, for parallel compression.", active: None },
            Item::Knob { p: OUTPUT, x: 400.0, y: 350.0, r: 22.0, label: "OUTPUT", tip: "Output level (make-up gain).", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Kit Crush 165A", class_id: *b"TImpKitCrush165A", vendor: "PEQUOD Tame Impala",
    categories: [Fx, Dynamics]
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
    fn squashes_hard_above_threshold() {
        // both above threshold at 40:1: 20 dB more in comes out only a little louder
        let p = with(defaults(), &[(OUTPUT, 0.0), (PUSH, 0.0), (THRESH, -40.0), (RATIO, 40.0)]);
        let a = db(rms(&go(&p, &sine(200.0, 0.05, 48000))[24000..]));
        let b = db(rms(&go(&p, &sine(200.0, 0.5, 48000))[24000..]));
        println!("in +20 dB -> out {:.1} dB", b - a);
        assert!(b - a < 4.0);
    }
    #[test]
    fn mono_sums_the_sides() {
        let p = with(defaults(), &[(RATIO, 1.0), (PUSH, 0.0), (OUTPUT, 0.0)]);
        let x = sine(300.0, 0.1, 24000);
        let mut input = vec![x.clone(), vec![0.0; 24000]];
        run(&p, &mut input);
        let (l, r) = (rms(&input[0][12000..]), rms(&input[1][12000..]));
        println!("left only in -> L {:.3} R {:.3}", l, r);
        assert!((l - r).abs() < 0.002 && r > 0.02);
    }

}
