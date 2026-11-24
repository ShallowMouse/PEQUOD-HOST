//! Yamaha SPX90: the first affordable multi-effects (1985), in every rack of the late 80s. Its reverbs
//! (Rev 1 Hall, Rev 2 Room, Rev 3 Vocal, Rev 4 Plate, Early Ref, Gate Reverb, Reverse Gate): a little
//! smaller and grainier than the REV7, about 15 kHz bandwidth. Estimated from Yamaha's documentation.

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

use vintage::verb::*;

/// A program: the shape of the space (from the units' manuals and published measurements, estimated).
#[derive(Clone, Copy)]
struct Program {
    size: f64,        // FDN line scale (1 = a medium room)
    diffusion: usize, // input diffusers (0-4)
    er: f64,          // early reflections level
    er_ms: f64,       // span of the early pattern
    lo: f64,          // low decay multiplier
    hi: f64,          // high decay multiplier (before the TREBLE knob)
    kind: u8,         // 0 reverb, 1 non-linear (flat burst then gone), 2 reverse (rising), 3 gated
    chorus: f64,      // line modulation depth
    width: f64,       // stereo width of the tail
}

/// The burst behind the non-linear, reverse and gated programs: dense taps over `span` ms.
struct Burst {
    line: DelayLine,
    taps: Vec<(f64, f64, f64)>,   // (position 0..1, gain L, gain R)
}

impl Burst {
    fn new(fs: f64) -> Self {
        let mut nz = Noise::new(91);
        let taps = (0..48).map(|k| {
            let p = (k as f64 + 0.5 * nz.next().abs()) / 48.0;
            (p, nz.next().signum(), nz.next().signum())
        }).collect();
        Self { line: DelayLine::new((fs * 1.2) as usize + 8), taps }
    }
    #[inline]
    fn process(&mut self, x: f64, fs: f64, span_ms: f64, kind: u8) -> (f64, f64) {
        self.line.write(x);
        let span = (span_ms * 0.001 * fs).clamp(32.0, self.line.max_delay() - 4.0);
        let (mut l, mut r) = (0.0, 0.0);
        for &(p, gl, gr) in self.taps.iter() {
            let shape = match kind { 2 => p * p, 3 => if p < 0.85 { 1.0 } else { (1.0 - p) / 0.15 }, _ => 1.0 - 0.15 * p };
            let v = self.line.read(2.0 + p * span) * shape;
            l += v * gl;
            r += v * gr;
        }
        (l * 0.16, r * 0.16)
    }
    fn reset(&mut self) {
        self.line.reset();
    }
}

/// One unit: pre-delay, the converters' band limit and word length, early reflections, the FDN.
struct Unit {
    fs: f64,
    pre: DelayLine,
    er: DelayLine,
    fdn: Fdn,
    burst: Burst,
    bw_in: [Biquad; 2],
    bw_out: [[Biquad; 2]; 2],
    hp: [Biquad; 2],
    er_taps: [(f64, f64, f64); 8],
}

impl Unit {
    fn new(fs: f64, bandwidth: f64) -> Self {
        let mut lp = Biquad::identity();
        lp.set_lowpass(fs, bandwidth.min(fs * 0.45), 0.7);
        let mut hp = Biquad::identity();
        hp.set_highpass(fs, 40.0, 0.7);
        Self {
            fs,
            pre: DelayLine::new((fs * 0.5) as usize + 8),
            er: DelayLine::new((fs * 0.25) as usize + 8),
            fdn: Fdn::new(fs, 4.0),
            burst: Burst::new(fs),
            bw_in: [lp; 2],
            bw_out: [[lp; 2]; 2],
            hp: [hp; 2],
            er_taps: [(0.07, 0.9, 0.4), (0.13, 0.5, 0.85), (0.21, 0.7, 0.6), (0.29, 0.45, 0.75), (0.38, 0.6, 0.35), (0.52, 0.3, 0.55), (0.67, 0.4, 0.3), (0.86, 0.25, 0.35)],
        }
    }
    fn reset(&mut self) {
        self.pre.reset();
        self.er.reset();
        self.fdn.reset();
        self.burst.reset();
        self.bw_in.iter_mut().chain(self.hp.iter_mut()).for_each(|b| b.reset());
        self.bw_out.iter_mut().flatten().for_each(|b| b.reset());
    }
    fn configure(&mut self, p: &Program, decay: f64, treble: f64, bass: f64) {
        let rt = decay.max(0.1);
        let hi = (p.hi * (0.25 + 1.5 * treble)).clamp(0.05, 1.3);
        self.fdn.configure(p.size, [rt * p.lo * bass, rt, rt * hi], 4000.0);
        self.fdn.mod_depth = p.chorus;
    }
    /// `bits`: the converters' word length (0 = clean).
    #[inline]
    fn process(&mut self, x: f64, p: &Program, pre_ms: f64, span_ms: f64, bits: u32) -> (f64, f64) {
        let s0 = self.bw_in[0].process(x);
        let mut s = self.bw_in[1].process(s0);
        if bits > 0 { s = quantise(s * 0.5, bits) * 2.0; }
        self.pre.write(s);
        let d = self.pre.read((pre_ms * 0.001 * self.fs).max(1.0));
        let (mut l, mut r);
        if p.kind == 0 {
            self.er.write(d);
            let span = (p.er_ms * 0.001 * self.fs).min(self.er.max_delay() - 4.0);
            let (mut el, mut er) = (0.0, 0.0);
            for &(t, a, b) in self.er_taps.iter() {
                let v = self.er.read(2.0 + t * span);
                el += v * a;
                er += v * b;
            }
            let dd = self.fdn.diffuse(d, p.diffusion);
            let (tl, tr) = self.fdn.process(dd);
            l = tl + el * p.er * 0.35;
            r = tr + er * p.er * 0.35;
        } else {
            let dd = self.fdn.diffuse(d, p.diffusion.min(2));
            let (bl, br) = self.burst.process(dd, self.fs, span_ms, p.kind);
            l = bl;
            r = br;
        }
        let m = (l + r) * 0.5;
        let sd = (l - r) * 0.5 * p.width;
        l = m + sd;
        r = m - sd;
        if bits > 0 { l = quantise(l * 0.5, bits) * 2.0; r = quantise(r * 0.5, bits) * 2.0; }
        for (c, v) in [&mut l, &mut r].into_iter().enumerate() {
            let a = self.hp[c].process(*v);
            let b = self.bw_out[c][0].process(a);
            *v = self.bw_out[c][1].process(b);
        }
        (l, r)
    }
}

pub const W: u32 = 640;
pub const H: u32 = 432;

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "program"]
    pub program: FloatParam,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "predelay"]
    pub predelay: FloatParam,
    #[id = "treble"]
    pub treble: FloatParam,
    #[id = "bass"]
    pub bass: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            program: FloatParam::new("Program", 0.0, FloatRange::Linear { min: 0.0, max: 6.0 }).with_step_size(1.0),
            decay: secs("Decay", 2.2, 0.2, 12.0),
            predelay: ms("Pre-Delay", 12.0, 0.0, 250.0),
            treble: pct("Treble Decay", 0.5),
            bass: FloatParam::new("Bass Multiply", 1.0, FloatRange::Linear { min: 0.5, max: 2.0 }).with_step_size(0.01).with_unit("x"),
            mix: pct("Mix", 0.3),
        }
    }
}

pub const PROGRAM: usize = 0;
pub const DECAY: usize = 1;
pub const PREDELAY: usize = 2;
pub const TREBLE: usize = 3;
pub const BASS: usize = 4;
pub const MIX: usize = 5;
pub const NPARAMS: usize = 6;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.program.as_ptr(),
            self.decay.as_ptr(),
            self.predelay.as_ptr(),
            self.treble.as_ptr(),
            self.bass.as_ptr(),
            self.mix.as_ptr(),
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

const PROGRAMS: [Program; 7] = [
    Program { size: 2.2, diffusion: 3, er: 0.5, er_ms: 60.0, lo: 1.0, hi: 0.75, kind: 0, chorus: 0.0008, width: 0.95 },
    Program { size: 0.9, diffusion: 2, er: 0.8, er_ms: 30.0, lo: 1.0, hi: 0.7, kind: 0, chorus: 0.0006, width: 0.8 },
    Program { size: 1.4, diffusion: 3, er: 0.4, er_ms: 35.0, lo: 0.9, hi: 0.85, kind: 0, chorus: 0.0006, width: 0.85 },
    Program { size: 1.1, diffusion: 3, er: 0.1, er_ms: 20.0, lo: 0.9, hi: 0.9, kind: 0, chorus: 0.0008, width: 0.85 },
    Program { size: 0.8, diffusion: 1, er: 1.0, er_ms: 50.0, lo: 0.8, hi: 0.8, kind: 0, chorus: 0.0, width: 0.8 },
    Program { size: 1.0, diffusion: 2, er: 0.0, er_ms: 0.0, lo: 1.0, hi: 0.8, kind: 3, chorus: 0.0, width: 1.0 },
    Program { size: 1.0, diffusion: 2, er: 0.0, er_ms: 0.0, lo: 1.0, hi: 0.8, kind: 2, chorus: 0.0, width: 1.0 }
];

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    unit: Unit,
    mix: Smooth,
}

impl Dsp {
    fn program(&self) -> Program {
        PROGRAMS[(self.v[PROGRAM].round() as usize).min(6)]
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, unit: Unit::new(fs, 15000.0), mix: Smooth::new(fs, 20.0, 0.3) };
        let p = d.program();
        d.unit.configure(&p, v[DECAY] as f64, v[TREBLE] as f64, v[BASS] as f64);
        d
    }
    fn reset(&mut self) {
        self.unit.reset();
        self.mix.snap(self.v[MIX] as f64);
    }
    fn set_params(&mut self, v: &[f32]) {
        let changed = self.v[..] != v[..NPARAMS];
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.mix.set(v[MIX] as f64);
        if changed {
            let p = self.program();
            self.unit.configure(&p, v[DECAY] as f64, v[TREBLE] as f64, v[BASS] as f64);
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nc = chans.len();
        let p = self.program();
        let pre = self.v[PREDELAY] as f64;
        // the non-linear programs: DECAY sets how long the burst lasts (100 to 1200 ms)
        let span = (self.v[DECAY] as f64 * 100.0).clamp(100.0, 1200.0);
        let mut peak = 0.0f64;
        for k in 0..n {
            let m = self.mix.next();
            let x = if nc >= 2 { (chans[0][k] + chans[1][k]) as f64 * 0.5 } else { chans[0][k] as f64 };
            let (l, r) = self.unit.process(x, &p, pre, span, 16);
            peak = peak.max(l.abs()).max(r.abs());
            let wet = [l, r];
            for c in 0..nc {
                let dry = chans[c][k] as f64;
                chans[c][k] = (dry * (1.0 - m) + wet[c.min(1)] * m) as f32;
            }
        }
        meters[0] = peak as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Rev 1 Hall", values: &[(PROGRAM, 0.0), (DECAY, 2.0)] },
        Preset { name: "Rev 4 Plate", values: &[(PROGRAM, 3.0), (DECAY, 1.5), (MIX, 0.25)] },
        Preset { name: "Gate Reverb", values: &[(PROGRAM, 5.0), (DECAY, 2.5), (MIX, 0.4)] },
        Preset { name: "Reverse Gate", values: &[(PROGRAM, 6.0), (DECAY, 4.0), (MIX, 0.4)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "YAMAHA SPX90",
        subtitle: ("DIGITAL MULTI-EFFECT PROCESSOR, 1985", "THE REVERB PROGRAMS"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![
            Item::Selector { p: PROGRAM, x: 120.0, y: 160.0, label: "PROGRAM", positions: &["REV1 HALL", "REV2 ROOM", "REV3 VOCAL", "REV4 PLATE", "EARLY REF", "GATE", "REVERSE"], tip: "Rev 1 Hall, Rev 2 Room, Rev 3 Vocal, Rev 4 Plate, Early Reflections, Gate, Reverse Gate." },
            Item::Meter { m: 0, x: 250.0, y: 95.0, w: 150.0, h: 100.0, kind: MeterKind::Vu, caption: "REVERB" },
            Item::Knob { p: DECAY, x: 520.0, y: 160.0, r: 40.0, label: "DECAY", tip: "Reverb time (mid band). On the non-linear and reverse programs: how long the burst lasts.", active: None },
            Item::Divider { y: 255.0 },
            Item::Knob { p: PREDELAY, x: 90.0, y: 335.0, r: 26.0, label: "PRE-DELAY", tip: "Before the reverb starts.", active: None },
            Item::Knob { p: TREBLE, x: 210.0, y: 335.0, r: 26.0, label: "TREBLE DECAY", tip: "How long the highs last, against the mids.", active: None },
            Item::Knob { p: BASS, x: 330.0, y: 335.0, r: 26.0, label: "BASS MULT", tip: "How long the lows last, against the mids.", active: None },
            Item::Knob { p: MIX, x: 470.0, y: 335.0, r: 30.0, label: "MIX", tip: "Dry vs. reverb; 100% on a send.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Yamaha SPX90", class_id: *b"HWYamahaSPX90RvQ", vendor: "PEQUOD Hardware",
    categories: [Fx, Reverb]
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
    fn rings_on() {
        let p = with(defaults(), &[(MIX, 1.0), (DECAY, 2.5)]);
        let mut x = vec![0.0f32; 96000];
        x[100] = 0.8;
        let y = go(&p, &x);
        let late = db(rms(&y[24000..48000]));
        println!("tail 0.5-1 s: {:.1} dB", late);
        assert!(late > -80.0 && y.iter().all(|v| v.is_finite()));
    }

}
