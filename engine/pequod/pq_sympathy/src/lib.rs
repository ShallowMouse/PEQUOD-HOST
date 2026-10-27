//! Sympathy: sympathetic strings that learn your harmony. A bank of 48 virtual strings (two octaves, two detuned strings per note) sits behind your sound like the sympathetic strings of a sitar or a piano with the pedal down. It listens to what you play (polyphonic pitch analysis with harmonic peeling), opens only the strings whose notes are in your current harmony and mutes the rest, so the resonance always follows your chords and is never out of key. It also follows your instrument's own tuning (up to 50 cents off A440). The strings are excited only by your sound, so they ring with your timbre and only on the notes you played.

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
use vintage::realfft::{RealFftPlanner, RealToComplex};

pub const W: u32 = 640;
pub const H: u32 = 468;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Learn { #[name = "Follow"] Follow, #[name = "Hold"] Hold, #[name = "All Strings"] All }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Register { #[name = "Low"] Low, #[name = "Mid"] Mid, #[name = "High"] High }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "sustain"]
    pub sustain: FloatParam,
    #[id = "memory"]
    pub memory: FloatParam,
    #[id = "notes"]
    pub notes: IntParam,
    #[id = "learn"]
    pub learn: EnumParam<Learn>,
    #[id = "register"]
    pub register: EnumParam<Register>,
    #[id = "bright"]
    pub bright: FloatParam,
    #[id = "width"]
    pub width: FloatParam,
    #[id = "duck"]
    pub duck: FloatParam,
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
            sustain: secs("Sustain", 6.0, 0.5, 30.0),
            memory: secs("Memory", 3.0, 0.3, 30.0),
            notes: IntParam::new("Notes", 4, IntRange::Linear { min: 1, max: 12 }),
            learn: EnumParam::new("Learn", Learn::Follow),
            register: EnumParam::new("Register", Register::Mid),
            bright: pct("Brightness", 0.5),
            width: pct("Width", 0.6),
            duck: db("Duck", 6.0, 0.0, 24.0),
            mix: pct("Mix", 0.3),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const SUSTAIN: usize = 0;
pub const MEMORY: usize = 1;
pub const NOTES: usize = 2;
pub const LEARN: usize = 3;
pub const REGISTER: usize = 4;
pub const BRIGHT: usize = 5;
pub const WIDTH: usize = 6;
pub const DUCK: usize = 7;
pub const MIX: usize = 8;
pub const OUTPUT: usize = 9;
pub const NPARAMS: usize = 10;

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
            self.memory.as_ptr(),
            self.notes.as_ptr(),
            self.learn.as_ptr(),
            self.register.as_ptr(),
            self.bright.as_ptr(),
            self.width.as_ptr(),
            self.duck.as_ptr(),
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

const NOTES_PER: usize = 24; // two octaves
const NSTR: usize = NOTES_PER * 2; // two strings per note (left and right, detuned)
const CLOSED_T60: f64 = 0.06;
const UPD: usize = 32;
const LO_NOTE: i32 = 38;
const HI_NOTE: i32 = 86;

#[derive(Clone)]
struct Str {
    dl: DelayLine,
    len: f64,
    b: f64,
    z1: f64,
    z2: f64,
    g: f64,
    inj: f64,
    pl: f64,
    pr: f64,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    strings: Vec<Str>,
    /// pitch-class memory (0..1) and smoothed open amount per pitch class
    pub pc: [f64; 12],
    pub open: [f64; 12],
    target: [f64; 12],
    /// learned tuning offset from A440, cents
    pub tune: f64,
    // analysis
    n: usize,
    hop: usize,
    ring: Vec<f64>,
    rpos: usize,
    count: usize,
    win: Vec<f64>,
    fft: std::sync::Arc<dyn RealToComplex<f64>>,
    fin: Vec<f64>,
    fout: Vec<vintage::realfft::num_complex::Complex<f64>>,
    mag: Vec<f64>,
    // signal
    hp_in: Biquad,
    hp_wet: [Biquad; 2],
    env: Envelope,
    k: usize,
    mix: Smooth,
    out: Smooth,
}

fn midi_hz(m: f64, cents: f64) -> f64 {
    440.0 * 2f64.powf((m - 69.0) / 12.0 + cents / 1200.0)
}

impl Dsp {
    /// The strongest true spectral peak (local maximum) within about a quarter-tone of f.
    fn peak(&self, f: f64) -> (f64, usize) {
        let df = self.fs / self.n as f64;
        let b = f / df;
        let tol = (0.02 * b).max(0.6);
        let k0 = ((b - tol).round().max(1.0)) as usize;
        let k1 = ((b + tol).round() as usize).min(self.mag.len() - 2);
        let mut best = (0.0, 0);
        for k in k0..=k1.max(k0) {
            let v = self.mag[k];
            if v > best.0 && v >= self.mag[k - 1] && v >= self.mag[k + 1] {
                best = (v, k);
            }
        }
        best
    }
    /// Harmonic sum, but only for a candidate whose own fundamental is really there
    /// (stops sub-harmonics of a chord collecting everyone's partials).
    fn salience(&self, m: i32) -> f64 {
        let f0 = midi_hz(m as f64, self.tune);
        let mut s = 0.0;
        let mut top: f64 = 0.0;
        let mut fund = 0.0;
        let mut second = 0.0;
        for h in 1..=20 {
            let f = f0 * h as f64;
            if f > 6000.0f64.min(0.45 * self.fs) {
                break;
            }
            let a = self.peak(f).0;
            if h == 1 {
                fund = a;
            }
            if h == 2 {
                second = a;
            }
            top = top.max(a);
            s += a / (h as f64).sqrt();
        }
        if fund < 0.15 * top || fund <= 0.0 || second < 0.05 * fund {
            return 0.0;
        }
        // a "note" that is really the 3rd or 5th harmonic of a lower note that is sounding
        // (a ghost fifth or third) gives way to that note. Its strings would ring there anyway:
        // a string's own harmonics include its fifth and third.
        for d in [3.0, 5.0] {
            let fs_ = f0 / d;
            if fs_ < 70.0 {
                continue;
            }
            let (a, _) = self.peak(fs_);
            if a >= 0.25 * fund {
                return s * 0.2;
            }
        }
        s
    }
    fn analyse(&mut self) {
        let n = self.n;
        let mut e = 0.0;
        for i in 0..n {
            let x = self.ring[(self.rpos + i) % n];
            e += x * x;
            self.fin[i] = x * self.win[i];
        }
        let frame_rms = (e / n as f64).sqrt();
        // a note just starting (or stopping) inside the window smears the spectrum: wait for a steady frame
        let mut e1 = 0.0;
        for i in 0..n / 2 {
            let x = self.ring[(self.rpos + i) % n];
            e1 += x * x;
        }
        let e2 = e - e1;
        let steady = e1 > 0.2 * e2 && e2 > 0.2 * e1;
        let mode = (self.v[LEARN].round() as usize).min(2);
        let decay = (-(self.hop as f64) / (self.fs * self.v[MEMORY].max(0.1) as f64)).exp();
        let mut det = [0.0f64; 12];
        if frame_rms > 0.0018 && steady && mode == 0 {
            if self.fft.process(&mut self.fin, &mut self.fout).is_ok() {
                for (m, c) in self.mag.iter_mut().zip(self.fout.iter()) {
                    *m = c.norm();
                }
                let mut first = 0.0;
                for it in 0..6 {
                    let (mut bm, mut bs) = (LO_NOTE, 0.0);
                    for m in LO_NOTE..=HI_NOTE {
                        let s = self.salience(m);
                        if s > bs {
                            bs = s;
                            bm = m;
                        }
                    }
                    if it == 0 {
                        first = bs;
                        self.track_tuning(bm);
                    } else if bs < 0.3 * first {
                        break;
                    }
                    if bs <= 0.0 {
                        break;
                    }
                    let pc = (bm.rem_euclid(12)) as usize;
                    #[cfg(test)]
                    if std::env::var("SYMDBG").is_ok() { println!("  it {it} note {bm} sal {bs:.3} rel {:.2}", bs / first); }
                    det[pc] = det[pc].max(bs / first);
                    // peel this note out of the spectrum. Each partial is reduced by the amount the
                    // note itself explains (its spectrally smoothed level), so partials shared with
                    // other notes of the chord keep what belongs to them.
                    let f0 = midi_hz(bm as f64, self.tune);
                    let fmax = 6000.0f64.min(0.45 * self.fs);
                    let hmax = ((fmax / f0) as usize).clamp(1, 40);
                    let mut amps = [(0.0f64, 0usize); 41];
                    for h in 1..=hmax {
                        amps[h] = self.peak(f0 * h as f64);
                    }
                    for h in 1..=hmax {
                        let (a, kp) = amps[h];
                        if a <= 0.0 {
                            continue;
                        }
                        let lo = if h > 1 { amps[h - 1].0 } else { a };
                        let hi = if h < hmax { amps[h + 1].0 } else { a };
                        let smooth = a.min((lo + a + hi) / 3.0);
                        let keep = (1.0 - smooth / a).clamp(0.02, 1.0);
                        for k in kp.saturating_sub(2)..=(kp + 2).min(self.mag.len() - 1) {
                            self.mag[k] *= keep;
                        }
                    }
                }
            }
        }
        match mode {
            0 => {
                for i in 0..12 {
                    self.pc[i] = (self.pc[i] * decay).max(det[i]);
                }
            }
            2 => self.pc = [1.0; 12],
            _ => {}
        }
        if mode != 1 {
            let mut idx: Vec<usize> = (0..12).collect();
            idx.sort_by(|a, b| self.pc[*b].partial_cmp(&self.pc[*a]).unwrap_or(std::cmp::Ordering::Equal));
            let keep = (self.v[NOTES].round() as usize).clamp(1, 12);
            self.target = [0.0; 12];
            for (r, &i) in idx.iter().enumerate() {
                if r < keep && self.pc[i] > 0.3 {
                    self.target[i] = 1.0;
                }
            }
        }
    }
    /// Measure how far the strongest note's well-resolved harmonics sit from the grid, and follow it slowly.
    fn track_tuning(&mut self, m: i32) {
        let df = self.fs / self.n as f64;
        let f0 = midi_hz(m as f64, self.tune);
        let (mut acc, mut wsum) = (0.0, 0.0);
        for h in 1..=6 {
            let f = f0 * h as f64;
            if f / df < 24.0 || f > 0.4 * self.fs {
                continue;
            }
            let (a, k) = self.peak(f);
            if a <= 0.0 || k == 0 || k + 1 >= self.mag.len() {
                continue;
            }
            let (l, c, r) = (self.mag[k - 1].max(1e-12).ln(), a.max(1e-12).ln(), self.mag[k + 1].max(1e-12).ln());
            let den = l - 2.0 * c + r;
            let d = if den.abs() > 1e-12 { (0.5 * (l - r) / den).clamp(-0.5, 0.5) } else { 0.0 };
            let fp = (k as f64 + d) * df;
            let cents = 1200.0 * (fp / f).log2();
            if cents.abs() < 60.0 {
                acc += cents * a;
                wsum += a;
            }
        }
        if wsum > 0.0 {
            let err = acc / wsum;
            self.tune = (self.tune + 0.08 * err).clamp(-50.0, 50.0);
        }
    }
    fn update_strings(&mut self) {
        let fs = self.fs;
        let ca = (-(UPD as f64) / (fs * 0.03)).exp();
        let cr = (-(UPD as f64) / (fs * 0.15)).exp();
        for i in 0..12 {
            let t = self.target[i];
            let c = if t > self.open[i] { ca } else { cr };
            self.open[i] = t + c * (self.open[i] - t);
        }
        let base = match (self.v[REGISTER].round() as i32).clamp(0, 2) {
            0 => 40,
            1 => 52,
            _ => 64,
        };
        let sus = self.v[SUSTAIN] as f64;
        let bright = self.v[BRIGHT] as f64;
        let width = self.v[WIDTH] as f64;
        for j in 0..NOTES_PER {
            let m = base + j as i32;
            let pc = m.rem_euclid(12) as usize;
            let o = self.open[pc];
            let t60 = (CLOSED_T60.ln() + o * (sus.ln() - CLOSED_T60.ln())).exp();
            for side in 0..2 {
                let det = if side == 0 { -3.0 * width } else { 3.0 * width };
                let f = midi_hz(m as f64, self.tune + det);
                // symmetric 3-tap loop filter: linear phase (exactly one sample), so the
                // string's partials stay exactly harmonic; darker for low BRIGHTNESS
                let per = fs / f;
                let b = (0.25 * (1.0 - bright).powf(1.3) * (per / 200.0).min(1.0).sqrt()).clamp(0.0, 0.25);
                let w = 2.0 * std::f64::consts::PI * f / fs;
                let mag_h = (1.0 - 2.0 * b) + 2.0 * b * w.cos();
                let g = (10f64.powf(-3.0 / (t60 * f)) / mag_h).min(0.99995);
                let s = &mut self.strings[j * 2 + side];
                s.len = (per - 1.0).max(2.0);
                s.b = b;
                s.g = g;
                s.inj = o * 1.5 * (1.0 - g).max(1e-6).powf(0.75);
                // spread: left strings left, right strings right, higher notes wider
                let p = width * if side == 0 { -1.0 } else { 1.0 } * (0.45 + 0.55 * j as f64 / (NOTES_PER - 1) as f64);
                let th = (p + 1.0) * std::f64::consts::FRAC_PI_4;
                s.pl = th.cos();
                s.pr = th.sin();
            }
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 12;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let n = if fs > 120000.0 { 32768 } else if fs > 60000.0 { 16384 } else { 8192 };
        let mut planner = RealFftPlanner::<f64>::new();
        let fft = planner.plan_fft_forward(n);
        let fin = fft.make_input_vec();
        let fout = fft.make_output_vec();
        let win = (0..n).map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos()).collect();
        let maxlen = (fs / 70.0) as usize + 16;
        let s = Str { dl: DelayLine::new(maxlen), len: 100.0, b: 0.0, z1: 0.0, z2: 0.0, g: 0.0, inj: 0.0, pl: 0.7, pr: 0.7 };
        let mut hp_in = Biquad::identity();
        hp_in.set_highpass(fs, 70.0, 0.7);
        let mut hw = Biquad::identity();
        hw.set_highpass(fs, 60.0, 0.7);
        let mut d = Self {
            fs, v, strings: vec![s; NSTR], pc: [0.0; 12], open: [0.0; 12], target: [0.0; 12], tune: 0.0,
            n, hop: n / 4, ring: vec![0.0; n], rpos: 0, count: 0, win, fft, fin, fout, mag: vec![0.0; n / 2 + 1],
            hp_in, hp_wet: [hw, hw], env: Envelope::new(fs, 5.0, 250.0), k: 0,
            mix: Smooth::new(fs, 20.0, 0.3), out: Smooth::new(fs, 20.0, 1.0),
        };
        d.update_strings();
        d
    }
    fn reset(&mut self) {
        for s in self.strings.iter_mut() {
            s.dl.reset();
            s.z1 = 0.0;
            s.z2 = 0.0;
        }
        self.ring.iter_mut().for_each(|x| *x = 0.0);
        self.pc = [0.0; 12];
        self.open = [0.0; 12];
        self.target = if (self.v[LEARN].round() as usize) == 2 { [1.0; 12] } else { [0.0; 12] };
        self.hp_in.reset();
        self.hp_wet.iter_mut().for_each(|b| b.reset());
        self.env.value = 0.0;
        self.mix.snap(self.v[MIX] as f64);
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
        self.update_strings();
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
        if (v[LEARN].round() as usize) == 2 {
            self.pc = [1.0; 12];
            self.target = [1.0; 12];
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().max(1);
        let duck = self.v[DUCK] as f64;
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch as f64;
            // analysis
            self.ring[self.rpos] = x;
            self.rpos = (self.rpos + 1) % self.n;
            self.count += 1;
            if self.count >= self.hop {
                self.count = 0;
                self.analyse();
            }
            if self.k == 0 {
                self.update_strings();
            }
            self.k = (self.k + 1) % UPD;
            // the strings: excited only by your sound
            let xi = self.hp_in.process(x);
            let (mut wl, mut wr) = (0.0, 0.0);
            for s in self.strings.iter_mut() {
                if s.inj == 0.0 && s.z1.abs() < 1e-9 && s.z2.abs() < 1e-9 {
                    s.dl.write(0.0);
                    continue;
                }
                let y = s.dl.read(s.len);
                let lp = s.b * (y + s.z2) + (1.0 - 2.0 * s.b) * s.z1;
                s.z2 = s.z1;
                s.z1 = y;
                s.dl.write(xi * s.inj + s.g * lp);
                wl += y * s.pl;
                wr += y * s.pr;
            }
            // duck: the resonance steps back while you play and blooms in the gaps
            let e = self.env.process(x);
            let lev = 20.0 * e.max(1e-9).log10();
            let dg = db_to_gain(-duck * ((lev + 45.0) / 30.0).clamp(0.0, 1.0));
            let wl = self.hp_wet[0].process(wl) * dg * 0.5;
            let wr = self.hp_wet[1].process(wr) * dg * 0.5;
            let mx = self.mix.next();
            let o = self.out.next();
            let dry_g = (2.0 * (1.0 - mx)).min(1.0);
            let wet_g = (2.0 * mx).min(1.0);
            let soft = |w: f64| if w.abs() > 1.0 { w.signum() * (1.0 + (w.abs() - 1.0).tanh() * 0.5) } else { w };
            let (wl, wr) = (soft(wl), soft(wr));
            if nch >= 2 {
                for (ci, ch) in chans.iter_mut().enumerate() {
                    let w = if ci == 0 { wl } else if ci == 1 { wr } else { 0.5 * (wl + wr) };
                    ch[k] = ((ch[k] as f64 * dry_g + w * wet_g) * o) as f32;
                }
            } else {
                let ch = &mut chans[0];
                ch[k] = ((ch[k] as f64 * dry_g + 0.5 * (wl + wr) * wet_g) * o) as f32;
            }
        }
        for i in 0..12 {
            meters[i] = self.open[i] as f32;
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Guitar Halo (follows your chords)", values: &[] },
        Preset { name: "Sitar Sympathetics (all strings)", values: &[(LEARN, 2.0), (REGISTER, 2.0), (SUSTAIN, 10.0), (BRIGHT, 0.8), (DUCK, 3.0), (MIX, 0.3)] },
        Preset { name: "Piano Pedal Down", values: &[(LEARN, 2.0), (REGISTER, 0.0), (SUSTAIN, 12.0), (BRIGHT, 0.4), (DUCK, 0.0), (MIX, 0.35)] },
        Preset { name: "Vocal Halo", values: &[(NOTES, 3.0), (MEMORY, 2.0), (BRIGHT, 0.35), (DUCK, 9.0), (MIX, 0.3)] },
        Preset { name: "Bloom Between Phrases", values: &[(SUSTAIN, 9.0), (DUCK, 18.0), (MIX, 0.5)] },
        Preset { name: "Frozen Chord (play, then HOLD)", values: &[(LEARN, 1.0), (SUSTAIN, 30.0), (MIX, 0.45)] },
        Preset { name: "Drone Bed (long memory)", values: &[(SUSTAIN, 25.0), (MEMORY, 20.0), (NOTES, 3.0), (REGISTER, 0.0), (BRIGHT, 0.3), (MIX, 0.45)] },
        Preset { name: "Tight Strings", values: &[(SUSTAIN, 1.5), (MEMORY, 0.8), (BRIGHT, 0.7), (MIX, 0.3)] },
        Preset { name: "Single-Note Lines, Shimmer", values: &[(NOTES, 6.0), (MEMORY, 5.0), (REGISTER, 2.0), (BRIGHT, 0.75), (DUCK, 6.0), (MIX, 0.35)] },
        Preset { name: "Mono Sympathy (narrow)", values: &[(WIDTH, 0.0), (MIX, 0.3)] },
        Preset { name: "Send: Strings Only", values: &[(MIX, 1.0), (DUCK, 4.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "SYMPATHY",
        subtitle: ("SYMPATHETIC STRINGS THAT LEARN", "YOUR HARMONY AND RING ONLY IN KEY"),
        accent: ui::INDIGO,
        size: (W, H),
        items: vec![

            Item::Text { x: 320.0, y: 84.0, text: "OPEN STRINGS", size: 11.0, align: Align2::CENTER_CENTER },
            Item::Lamp { m: 0, x: 66.0, y: 112.0, label: "C", lo: 0.0, hi: 1.0 },
            Item::Lamp { m: 1, x: 112.0, y: 112.0, label: "C#", lo: 0.0, hi: 1.0 },
            Item::Lamp { m: 2, x: 158.0, y: 112.0, label: "D", lo: 0.0, hi: 1.0 },
            Item::Lamp { m: 3, x: 204.0, y: 112.0, label: "D#", lo: 0.0, hi: 1.0 },
            Item::Lamp { m: 4, x: 250.0, y: 112.0, label: "E", lo: 0.0, hi: 1.0 },
            Item::Lamp { m: 5, x: 296.0, y: 112.0, label: "F", lo: 0.0, hi: 1.0 },
            Item::Lamp { m: 6, x: 342.0, y: 112.0, label: "F#", lo: 0.0, hi: 1.0 },
            Item::Lamp { m: 7, x: 388.0, y: 112.0, label: "G", lo: 0.0, hi: 1.0 },
            Item::Lamp { m: 8, x: 434.0, y: 112.0, label: "G#", lo: 0.0, hi: 1.0 },
            Item::Lamp { m: 9, x: 480.0, y: 112.0, label: "A", lo: 0.0, hi: 1.0 },
            Item::Lamp { m: 10, x: 526.0, y: 112.0, label: "A#", lo: 0.0, hi: 1.0 },
            Item::Lamp { m: 11, x: 572.0, y: 112.0, label: "B", lo: 0.0, hi: 1.0 },
            Item::Knob { p: SUSTAIN, x: 100.0, y: 215.0, r: 38.0, label: "SUSTAIN", tip: "How long the open strings ring (time to fall 60 dB). Muted strings stop in a few tens of milliseconds.", active: None },
            Item::Knob { p: MEMORY, x: 230.0, y: 215.0, r: 38.0, label: "MEMORY", tip: "How long a note you played keeps its strings open after you stop playing it. Short: the strings follow every chord change. Long: they build up the harmony of the whole passage.", active: None },
            Item::Knob { p: NOTES, x: 360.0, y: 215.0, r: 38.0, label: "NOTES", tip: "The most notes (pitch classes) that can be open at once. 3-4 keeps it to the chord; higher lets melodies build up a scale.", active: None },
            Item::Selector { p: LEARN, x: 520.0, y: 210.0, label: "LEARN", positions: &["FOLLOW", "HOLD", "ALL"], tip: "FOLLOW: listen and open the strings of what you play. HOLD: freeze the strings that are open now (play a chord, switch to HOLD, then play anything over it). ALL: every string open, like a piano with the sustain pedal down." },
            Item::Divider { y: 290.0 },
            Item::Selector { p: REGISTER, x: 70.0, y: 352.0, label: "REGISTER", positions: &["LOW", "MID", "HIGH"], tip: "Where the two octaves of strings sit: LOW E2-D#4, MID E3-D#5, HIGH E4-D#6. Strings resonate with any harmonic of your notes, so HIGH shimmers even on bass notes." },
            Item::Knob { p: BRIGHT, x: 180.0, y: 352.0, r: 22.0, label: "BRIGHTNESS", tip: "How long the upper harmonics of each string ring compared with its fundamental. Low: soft, nylon-like. High: wiry and metallic.", active: None },
            Item::Knob { p: WIDTH, x: 270.0, y: 352.0, r: 22.0, label: "WIDTH", tip: "Each note has two strings, detuned up to 3 cents apart and spread left and right. 0 = mono and perfectly in tune.", active: None },
            Item::Knob { p: DUCK, x: 360.0, y: 352.0, r: 22.0, label: "DUCK", tip: "Turns the resonance down while you play and lets it bloom in the gaps, so it never clouds the notes themselves.", active: None },
            Item::Knob { p: MIX, x: 450.0, y: 352.0, r: 22.0, label: "MIX", tip: "Up to 50%: full dry signal with more and more resonance. Past 50%: the dry signal falls away. 100% = strings only (for a send).", active: None },
            Item::Knob { p: OUTPUT, x: 550.0, y: 352.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Sympathy", class_id: *b"PQSympathyPQD001", vendor: "PEQUOD",
    categories: [Fx, Reverb]
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
    /// A note with a few harmonics, like a plucked string's spectrum.
    fn tone(f: f64, amp: f64, n: usize) -> Vec<f64> {
        (0..n).map(|k| {
            let t = k as f64 / 48000.0;
            (1..=6).map(|h| amp / h as f64 * (2.0 * PI * f * h as f64 * t).sin()).sum::<f64>()
        }).collect()
    }
    fn drive(d: &mut Dsp, sig: &[f64]) -> Vec<f32> {
        let mut out = Vec::new();
        for c in sig.chunks(512) {
            let mut l: Vec<f32> = c.iter().map(|v| *v as f32).collect();
            let mut r = l.clone();
            let mut m = vec![0.0f32; 12];
            {
                let mut ch: Vec<&mut [f32]> = vec![&mut l[..], &mut r[..]];
                d.process(&mut ch, &mut m);
            }
            out.extend(l);
        }
        out
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
    /// Karplus-Strong plucks: a G major arpeggio at 44.1 kHz and 96 kHz, as a guitar would play it.
    #[test]
    fn learns_plucked_arpeggio_at_other_rates() {
        for fs in [44100.0f64, 96000.0] {
            let mut d = Dsp::new(fs as f32, 2);
            d.set_params(&defaults());
            d.reset();
            let notes = [98.0, 123.47, 146.83, 196.0, 146.83, 123.47, 98.0, 196.0];
            let mut sig = vec![0.0f64; (fs * 4.2) as usize];
            let mut nz = vintage::dsp::Noise::new(3);
            for (i, f) in notes.iter().enumerate() {
                let start = (i as f64 * 0.5 * fs) as usize;
                let len = (fs / f) as usize;
                let mut buf: Vec<f64> = (0..len).map(|_| nz.next() * 0.4).collect();
                let mut idx = 0;
                for k in start..sig.len() {
                    let nxt = (idx + 1) % len;
                    let v = buf[idx];
                    buf[idx] = 0.996 * 0.5 * (buf[idx] + buf[nxt]);
                    sig[k] += v;
                    idx = nxt;
                }
            }
            let mut m = vec![0.0f32; 12];
            for c in sig.chunks(512) {
                let mut l: Vec<f32> = c.iter().map(|v| *v as f32).collect();
                let mut r = l.clone();
                let mut ch: Vec<&mut [f32]> = vec![&mut l[..], &mut r[..]];
                d.process(&mut ch, &mut m);
                assert!(l.iter().all(|v| v.is_finite() && v.abs() < 4.0));
            }
            println!("{fs} Hz plucked G major: open {:?}", d.open.iter().map(|v| (v * 100.0).round() / 100.0).collect::<Vec<_>>());
            assert!(d.open[7] > 0.9 && d.open[11] > 0.9 && d.open[2] > 0.9, "G B D open");
            assert!(d.open.iter().enumerate().filter(|(i, _)| ![2, 7, 11].contains(i)).all(|(_, v)| *v < 0.1));
        }
    }
    #[test]
    fn learns_a_chord() {
        let mut d = Dsp::new(48000.0, 2);
        d.set_params(&defaults());
        d.reset();
        // C major triad: C3 E3 G3
        let n = 96000;
        let (c, e, g) = (tone(130.81, 0.1, n), tone(164.81, 0.1, n), tone(196.0, 0.1, n));
        let sig: Vec<f64> = (0..n).map(|k| c[k] + e[k] + g[k]).collect();
        drive(&mut d, &sig);
        println!("open: {:?}", d.open.iter().map(|v| (v * 100.0).round() / 100.0).collect::<Vec<_>>());
        assert!(d.open[0] > 0.9 && d.open[4] > 0.9 && d.open[7] > 0.9, "C E G should be open");
        for pc in [1, 2, 3, 5, 6, 8, 10, 11] {
            assert!(d.open[pc] < 0.1, "pitch class {pc} should be muted");
        }
    }
    #[test]
    fn follows_a_chord_change() {
        let mut d = Dsp::new(48000.0, 2);
        d.set_params(&with(defaults(), &[(MEMORY, 0.5)]));
        d.reset();
        let n = 72000;
        let a: Vec<f64> = { let (x, y, z) = (tone(110.0, 0.1, n), tone(138.59, 0.1, n), tone(164.81, 0.1, n)); (0..n).map(|k| x[k] + y[k] + z[k]).collect() };
        let f: Vec<f64> = { let (x, y, z) = (tone(87.31, 0.1, n), tone(110.0, 0.1, n), tone(130.81, 0.1, n)); (0..n).map(|k| x[k] + y[k] + z[k]).collect() };
        drive(&mut d, &a);
        let c_sharp_open = d.open[1];
        drive(&mut d, &f);
        println!("A major: C# {c_sharp_open:.2}; then F major: F {:.2} A {:.2} C {:.2} C# {:.2}", d.open[5], d.open[9], d.open[0], d.open[1]);
        assert!(c_sharp_open > 0.9);
        assert!(d.open[5] > 0.9 && d.open[9] > 0.9 && d.open[0] > 0.9 && d.open[1] < 0.1);
    }
    #[test]
    fn rings_on_after_you_stop_only_in_key() {
        let mut d = Dsp::new(48000.0, 2);
        d.set_params(&with(defaults(), &[(DUCK, 0.0), (MIX, 1.0), (WIDTH, 0.0), (MEMORY, 10.0)]));
        d.reset();
        let note = tone(220.0, 0.2, 48000);
        let mut sig = note.clone();
        sig.extend(std::iter::repeat(0.0).take(48000));
        let out = drive(&mut d, &sig);
        let tail = &out[60000..84000];
        let a = goertzel(tail, 220.0);
        let b = goertzel(tail, 233.08);
        let level = db(rms(tail)) - db(rms(&note.iter().map(|v| *v as f32).collect::<Vec<_>>()));
        println!("tail level {level:.1} dB re input; A {:.1} dB vs A# {:.1} dB", db(a), db(b));
        assert!(level > -30.0 && level < 6.0);
        assert!(db(a) - db(b) > 20.0);
    }
    #[test]
    fn follows_a_detuned_instrument() {
        let mut d = Dsp::new(48000.0, 2);
        d.set_params(&defaults());
        d.reset();
        let f = 220.0 * 2f64.powf(20.0 / 1200.0);
        let sig = tone(f, 0.2, 48000 * 4);
        drive(&mut d, &sig);
        println!("learned tuning {:.1} cents (played +20)", d.tune);
        assert!((d.tune - 20.0).abs() < 5.0);
    }
    #[test]
    fn mix_zero_is_dry() {
        let p = with(defaults(), &[(MIX, 0.0)]);
        let mut input = vec![sine(330.0, 0.3, 24000), sine(330.0, 0.3, 24000)];
        let orig = input[0].clone();
        run(&p, &mut input);
        let err = input[0].iter().zip(orig.iter()).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(err < 1e-3, "{err}");
    }
    #[test]
    fn duck_steps_back_while_playing() {
        let lvl = |duck: f32| {
            let p = with(defaults(), &[(DUCK, duck), (MIX, 1.0)]);
            let sig: Vec<f32> = tone(220.0, 0.2, 48000 * 2).iter().map(|v| *v as f32).collect();
            let mut input = vec![sig.clone(), sig];
            run(&p, &mut input);
            db(rms(&input[0][72000..]))
        };
        let (a, b) = (lvl(0.0), lvl(12.0));
        println!("wet while playing: duck 0 {a:.1} dB, duck 12 {b:.1} dB");
        assert!(a - b > 8.0);
    }

}
