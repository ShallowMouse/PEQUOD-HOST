//! Muff R22: a regular (NYC-style) Big Muff Pi with a pot in place of R22, the 100 ohm emitter resistor of the input booster (Q1), in the Kit Rae / ElectroSmash numbering. Stage by stage: input booster, two diode-clipping gain stages, the passive tone stack (39k/10n low-pass and 4n/22k high-pass with the mid scoop), and the output booster. R22 sets Q1's emitter degeneration: turned down it adds a little gain and grit; turned up it starves the booster, so the Muff cleans up, gets quieter and more open.

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


pub const W: u32 = 640;
pub const H: u32 = 432;


const OS_N: usize = 4;
// Q1 (input booster): shunt feedback 470k from collector to base, 39k in series at the input
const RF: f64 = 470e3;
const RS: f64 = 39e3;
// collector load and intrinsic emitter resistance at ~0.2 mA bias (re = 26 mV / Ic)
const RC_EFF: f64 = 9e3;
const RE_INTR: f64 = 130.0;


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "sustain"]
    pub sustain: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
    #[id = "r22"]
    pub r22: FloatParam,
    #[id = "input"]
    pub input: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            sustain: FloatParam::new("Sustain", 6.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            tone: FloatParam::new("Tone", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            volume: FloatParam::new("Volume", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            r22: FloatParam::new("R22", 100.0, FloatRange::Skewed { min: 0.0, max: 10000.0, factor: FloatRange::skew_factor(-2.2) }).with_step_size(1.0).with_unit(" ohm"),
            input: db("Guitar Level", 0.0, -24.0, 12.0),
        }
    }
}

pub const SUSTAIN: usize = 0;
pub const TONE: usize = 1;
pub const VOLUME: usize = 2;
pub const R22: usize = 3;
pub const INPUT: usize = 4;
pub const NPARAMS: usize = 5;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.sustain.as_ptr(),
            self.tone.as_ptr(),
            self.volume.as_ptr(),
            self.r22.as_ptr(),
            self.input.as_ptr(),
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

/// Diode-clipping gain stage (Q2/Q3): high-passed input, gain, diodes in the feedback loop limiting to ~0.6 V, Miller roll-off.
#[derive(Clone, Copy, Default)]
struct ClipStage {
    hp: Biquad,
    lp: Biquad,
    gain: f64,
}

impl ClipStage {
    fn new(fs: f64, gain: f64, f_hp: f64, f_lp: f64) -> Self {
        // coupling cap into the stage: first-order high-pass
        let mut hp = Biquad::identity();
        let t = 1.0 / (2.0 * std::f64::consts::PI * f_hp);
        hp.set_analog(fs, [0.0, t, 0.0], [0.0, t, 1.0]);
        // Miller cap across the feedback: a first-order roll-off
        let mut lp = Biquad::identity();
        lp.set_analog(fs, [0.0, 0.0, 1.0], [0.0, 1.0 / (2.0 * std::f64::consts::PI * f_lp), 1.0]);
        Self { hp, lp, gain }
    }
    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let v = self.hp.process(x) * self.gain;
        // silicon diode pair in the feedback: soft limit around 0.6 V, slightly rounded
        let vd = 0.6;
        let y = vd * (v / vd).tanh() + 0.04 * v / (1.0 + v.abs());
        self.lp.process(y)
    }
    fn reset(&mut self) {
        self.hp.reset();
        self.lp.reset();
    }
}

struct Muff {
    fs_os: f64,
    in_hp: Biquad,
    q1_lp: Biquad,
    q1_av: f64,
    q1_k: f64,
    q2: ClipStage,
    q3: ClipStage,
    lp_tone: Biquad,
    hp_tone: Biquad,
    q4_hp: Biquad,
    sustain: f64,
}

impl Muff {
    fn new(fs_os: f64) -> Self {
        let mut in_hp = Biquad::identity();
        in_hp.set_highpass(fs_os, 4.0, 0.5);
        let mut m = Self {
            fs_os,
            in_hp,
            q1_lp: Biquad::identity(),
            q1_av: 9.0,
            q1_k: 0.5,
            // clipping stages: ~26x and ~30x small-signal, 100 nF into ~15k (about 110 Hz), Miller roll-off
            q2: ClipStage::new(fs_os, 26.0, 110.0, 6000.0),
            q3: ClipStage::new(fs_os, 30.0, 110.0, 5000.0),
            lp_tone: Biquad::identity(),
            hp_tone: Biquad::identity(),
            q4_hp: Biquad::identity(),
            sustain: 0.5,
        };
        // tone stack: 39k / 10 nF low-pass (~408 Hz), 4 nF / 22k high-pass (~1.8 kHz)
        m.lp_tone.set_analog(fs_os, [0.0, 0.0, 1.0], [0.0, 39e3 * 10e-9, 1.0]);
        m.hp_tone.set_analog(fs_os, [0.0, 22e3 * 4e-9, 0.0], [0.0, 22e3 * 4e-9, 1.0]);
        m.q4_hp.set_highpass(fs_os, 20.0, 0.5);
        m.set_r22(100.0);
        m
    }
    fn reset(&mut self) {
        for b in [&mut self.in_hp, &mut self.q1_lp, &mut self.lp_tone, &mut self.hp_tone, &mut self.q4_hp] {
            b.reset();
        }
        self.q2.reset();
        self.q3.reset();
    }
    /// R22 (Q1 emitter resistor) -> open-loop gain -> closed-loop gain of the shunt-feedback booster, and its bandwidth.
    fn set_r22(&mut self, r22: f64) {
        let a = RC_EFF / (RE_INTR + r22.max(0.0));
        let ratio = RF / RS;
        self.q1_av = ratio * a / (a + 1.0 + ratio);
        // un-degenerated, the transistor's exponential curve shows through more
        self.q1_k = RE_INTR / (RE_INTR + r22.max(0.0));
        // feedback cap 470 pF across RF: the pole widens as loop gain falls
        let fc = (1.0 / (2.0 * std::f64::consts::PI * RF * 470e-12)) * (1.0 + ratio / (a + 1.0));
        // first order: the gain falls back toward the feedback limit rather than to nothing
        let fc = fc.min(self.fs_os * 0.2);
        let shelf = 1.0 / self.q1_av.max(1.0);
        self.q1_lp.set_analog(self.fs_os, [0.0, shelf / (2.0 * std::f64::consts::PI * fc * shelf.max(1e-3)), 1.0], [0.0, 1.0 / (2.0 * std::f64::consts::PI * fc), 1.0]);
    }
    #[inline]
    fn process(&mut self, x: f64, tone: f64) -> f64 {
        // input booster Q1: collector idles near 7 V on a 9 V supply, so it runs out of room upward first
        let v = self.in_hp.process(x) * self.q1_av;
        let v = v + self.q1_k * 0.08 * v * v.abs().min(4.0);
        let v = if v > 0.0 { 1.9 * (v / 1.9).tanh() } else { -6.0 * (-v / 6.0).tanh() };
        let v = self.q1_lp.process(v);
        // sustain pot (with R23 keeping a little signal at zero)
        let v = v * self.sustain;
        let v = self.q2.process(v);
        let v = self.q3.process(v);
        // passive tone stack: low-pass and high-pass legs mixed by the tone pot (mid scoop near 1 kHz in the middle)
        let lo = self.lp_tone.process(v);
        let hi = self.hp_tone.process(v);
        let t = tone;
        let v = lo * (1.0 - t) + hi * t;
        // output booster Q4 (~4.5x) with a little headroom limit
        let v = self.q4_hp.process(v) * 4.5;
        3.5 * (v / 3.5).tanh()
    }
}

pub struct Dsp {
    fs: f64,
    muff: [Muff; 2],
    os: [Oversampler; 2],
    out_lp: [Biquad; 2],
    v: [f32; NPARAMS],
    r22_key: f64,
    input: Smooth,
    sus: Smooth,
    tone: Smooth,
    vol: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let fs_os = fs * OS_N as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut out_lp = [Biquad::identity(); 2];
        for b in &mut out_lp {
            b.set_lowpass(fs, 12000.0f64.min(fs * 0.45), 0.707);
        }
        Self {
            fs,
            muff: [Muff::new(fs_os), Muff::new(fs_os)],
            os: [Oversampler::new(), Oversampler::new()],
            out_lp,
            v,
            r22_key: -1.0,
            input: Smooth::new(fs, 20.0, 1.0),
            sus: Smooth::new(fs, 20.0, 0.5),
            tone: Smooth::new(fs, 20.0, 0.5),
            vol: Smooth::new(fs, 20.0, 0.5),
        }
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.muff[c].reset();
            self.os[c].reset();
            self.out_lp[c].reset();
        }
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let r = v[R22] as f64;
        if (r - self.r22_key).abs() > 0.5 {
            for m in &mut self.muff {
                m.set_r22(r);
            }
            self.r22_key = r;
        }
        self.input.set(db_to_gain(v[INPUT] as f64));
        // 100k linear pots: sustain as the divider into Q2 (R23 = 1k keeps ~1% at zero)
        let s = v[SUSTAIN] as f64 / 10.0;
        self.sus.set((s * 100e3 + 1e3) / 101e3);
        self.tone.set(v[TONE] as f64 / 10.0);
        let vol = v[VOLUME] as f64 / 10.0;
        self.vol.set(vol * vol);
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(2);
        let mut peak = 0.0f64;
        for k in 0..n {
            let gi = self.input.next();
            let sus = self.sus.next();
            let tone = self.tone.next();
            let vol = self.vol.next();
            for c in 0..nch {
                // full scale ~ 1 V peak from the guitar
                let x = chans[c][k] as f64 * gi;
                let m = &mut self.muff[c];
                m.sustain = sus;
                let y = self.os[c].process(x, |s| m.process(s, tone));
                let y = self.out_lp[c].process(y) * vol * 0.6;
                peak = peak.max(y.abs());
                chans[c][k] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Stock (R22 = 100 ohm)", values: &[] },
        Preset { name: "R22 Shorted (0 ohm): max grit", values: &[(R22, 0.0)] },
        Preset { name: "R22 470 ohm: a bit cleaner", values: &[(R22, 470.0)] },
        Preset { name: "R22 1k: open, dynamic", values: &[(R22, 1000.0), (VOLUME, 6.0)] },
        Preset { name: "R22 3.3k: low-gain crunch", values: &[(R22, 3300.0), (SUSTAIN, 8.0), (VOLUME, 7.0)] },
        Preset { name: "R22 10k: starved booster", values: &[(R22, 10000.0), (SUSTAIN, 10.0), (VOLUME, 8.0)] },
        Preset { name: "Gilmour-ish Lead", values: &[(SUSTAIN, 7.0), (TONE, 6.0), (R22, 220.0)] },
        Preset { name: "Dark Doom Wall", values: &[(SUSTAIN, 10.0), (TONE, 2.5), (R22, 0.0)] },
        Preset { name: "Bright Cutting Fuzz", values: &[(SUSTAIN, 6.0), (TONE, 8.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "MUFF R22",
        subtitle: ("AFTER THE EHX BIG MUFF PI (NYC)", "WITH A POT ON R22"),
        accent: ui::TANGERINE,
        size: (W, H),
        items: vec![

            Item::Knob { p: VOLUME, x: 110.0, y: 165.0, r: 42.0, label: "VOLUME", tip: "Output volume (100k pot after the output booster).", active: None },
            Item::Knob { p: TONE, x: 250.0, y: 165.0, r: 42.0, label: "TONE", tip: "The passive tone stack: bass-heavy low-pass at 0, bright high-pass at 10, the famous mid scoop around 1 kHz in between.", active: None },
            Item::Knob { p: SUSTAIN, x: 390.0, y: 165.0, r: 42.0, label: "SUSTAIN", tip: "How hard the two diode-clipping stages are driven.", active: None },
            Item::Knob { p: R22, x: 545.0, y: 165.0, r: 36.0, label: "R22", tip: "A pot in place of R22, the input booster's emitter resistor (stock 100 ohm). Down to 0: a little more gain and grit. Up towards 10k: the booster is starved, so the Muff gets cleaner, quieter and more open (turn VOLUME up to match).", active: None },
            Item::Divider { y: 264.0 },
            Item::Knob { p: INPUT, x: 160.0, y: 330.0, r: 24.0, label: "GUITAR LEVEL", tip: "How hot the guitar hits the pedal. 0 dB treats a full-scale signal as about 1 V from the pickups.", active: None },
            Item::Lamp { m: 0, x: 330.0, y: 330.0, label: "LEVEL", lo: 0.05, hi: 0.9 },
            Item::Text { x: 500.0, y: 320.0, text: "STOCK R22 = 100 Ω", size: 12.0, align: Align2::CENTER_CENTER },
            Item::Text { x: 500.0, y: 342.0, text: "Q1 EMITTER RESISTOR", size: 11.0, align: Align2::CENTER_CENTER },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Muff R22", class_id: *b"PQMuffR22PotPQD1", vendor: "PEQUOD",
    categories: [Fx, Distortion]
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
    fn r22_sets_the_booster_gain() {
        let m0 = { let mut m = Muff::new(192000.0); m.set_r22(0.0); m.q1_av };
        let m100 = { let mut m = Muff::new(192000.0); m.set_r22(100.0); m.q1_av };
        let m10k = { let mut m = Muff::new(192000.0); m.set_r22(10000.0); m.q1_av };
        println!("Q1 gain: R22 0 = {m0:.1}x, 100 = {m100:.1}x, 10k = {m10k:.2}x");
        assert!(m0 > m100 && m100 > 8.0 && m100 < 11.0 && m10k < 1.5);
    }
    fn level(set: &[(usize, f32)], amp: f64) -> f64 {
        let p = with(defaults(), set);
        let mut input = vec![sine(196.0, amp, 48000)];
        run(&p, &mut input);
        db(rms(&input[0][24000..]))
    }
    #[test]
    fn stock_is_compressed_and_10k_is_dynamic() {
        // at low sustain: stock still squashes 12 dB of input; with R22 at 10k the booster no longer drives the clippers as hard
        let stock = level(&[(SUSTAIN, 0.0)], 0.1) - level(&[(SUSTAIN, 0.0)], 0.025);
        let open = level(&[(R22, 10000.0), (SUSTAIN, 0.0)], 0.1) - level(&[(R22, 10000.0), (SUSTAIN, 0.0)], 0.025);
        println!("output change for +12 dB input: stock {stock:.1} dB, R22 10k {open:.1} dB");
        assert!(stock < 4.5 && open > stock + 4.0);
    }
    #[test]
    fn tone_scoops_mids_in_the_middle() {
        let p = with(defaults(), &[(SUSTAIN, 0.0), (TONE, 5.0)]);
        let lvl = |f: f64| {
            let mut input = vec![sine(f, 0.002, 48000)];
            run(&p, &mut input);
            db(rms(&input[0][24000..]))
        };
        let (a, b, c) = (lvl(200.0), lvl(1000.0), lvl(4000.0));
        println!("tone 5 small-signal: 200 Hz {a:.1}, 1 kHz {b:.1}, 4 kHz {c:.1} dB");
        assert!(b < a - 3.0 && b < c - 3.0);
    }

}
