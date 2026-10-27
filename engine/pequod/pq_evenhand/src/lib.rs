//! Evenhand: a leveller that works note by note instead of on the signal. It finds the start of every note (with 32 ms of lookahead), measures how hard that note was played, and gives the whole note one fixed gain, changed in the few milliseconds before the attack. Uneven notes come out even, but each note keeps its own attack, sustain and decay exactly (no pumping, no squashed transients, no sustain pulled up), and slow swells and fades longer than PHRASE are kept, so your phrasing survives.

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


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Detect { #[name = "Body (RMS)"] Rms, #[name = "Attack (peak)"] Peak }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "even"]
    pub even: FloatParam,
    #[id = "phrase"]
    pub phrase: FloatParam,
    #[id = "range"]
    pub range: FloatParam,
    #[id = "sens"]
    pub sens: FloatParam,
    #[id = "gap"]
    pub gap: FloatParam,
    #[id = "detect"]
    pub detect: EnumParam<Detect>,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            even: pct("Even", 0.75),
            phrase: secs("Phrase", 2.0, 0.2, 10.0),
            range: db("Range", 9.0, 1.0, 24.0),
            sens: db("Sensitivity", 4.0, 1.5, 12.0),
            gap: ms("Shortest Note", 50.0, 30.0, 400.0),
            detect: EnumParam::new("Measure", Detect::Rms),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const EVEN: usize = 0;
pub const PHRASE: usize = 1;
pub const RANGE: usize = 2;
pub const SENS: usize = 3;
pub const GAP: usize = 4;
pub const DETECT: usize = 5;
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
            self.even.as_ptr(),
            self.phrase.as_ptr(),
            self.range.as_ptr(),
            self.sens.as_ptr(),
            self.gap.as_ptr(),
            self.detect.as_ptr(),
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

use std::collections::VecDeque;

const LOOK_MS: f64 = 32.0;
const MEAS_MS: f64 = 20.0;
const PRE_MS: f64 = 3.0;
const RAMP_START_MS: f64 = 10.0;
const RAMP_END_MS: f64 = 6.0;
const FLOOR_DB: f64 = -55.0;

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    look: usize,
    meas: usize,
    pre: usize,
    r0: usize,
    r1: usize,
    /// per-channel lookahead delay
    dl: Vec<Vec<f32>>,
    dpos: usize,
    /// detector history (mono): squared and absolute values
    hist_sq: Vec<f64>,
    hist_abs: Vec<f64>,
    hmask: usize,
    n: u64,
    // onset detector
    hp: Biquad,
    pow: f64,
    env_lp: Biquad,
    hf: Biquad,
    hf_lp: Biquad,
    pow_hf: f64,
    blk_min_hf: f64,
    mins_hf: [f64; 30],
    ms_len: usize,
    ms_count: usize,
    blk_min: f64,
    mins: [f64; 30],
    mpos: usize,
    last_onset: u64,
    armed: bool,
    pending: VecDeque<u64>,
    // note levels
    trend: Option<f64>,
    last_note: u64,
    // gain ramps (in input-sample time): (start, end, target dB)
    ramps: VecDeque<(u64, u64, f64)>,
    cur_db: f64,
    from_db: f64,
    pub notes: Vec<(u64, f64, f64)>,
    out: Smooth,
    flash: f32,
}

impl Dsp {
    fn ms(&self, m: f64) -> usize {
        (m * 0.001 * self.fs).round() as usize
    }
    fn note_level(&self, n_det: u64) -> f64 {
        let peak = self.v[DETECT].round() as usize == 1;
        let (mut s, mut p, mut c) = (0.0, 0.0f64, 0usize);
        let start = n_det.saturating_sub(self.pre as u64);
        let end = n_det + self.meas as u64;
        let mut i = start;
        while i < end {
            let k = (i as usize) & self.hmask;
            s += self.hist_sq[k];
            p = p.max(self.hist_abs[k]);
            c += 1;
            i += 1;
        }
        if peak {
            20.0 * p.max(1e-9).log10()
        } else {
            10.0 * (s / c.max(1) as f64).max(1e-18).log10()
        }
    }
    fn on_measured(&mut self, n_det: u64) {
        let level = self.note_level(n_det);
        if level < FLOOR_DB {
            return;
        }
        let even = self.v[EVEN] as f64;
        let range = self.v[RANGE] as f64;
        let phrase = self.v[PHRASE] as f64;
        let trend = match self.trend {
            None => level,
            Some(t) => t,
        };
        let gain = (even * (trend - level)).clamp(-range, range);
        // follow the player's own level over PHRASE: swells and fades slower than that are kept
        let dt = (n_det.saturating_sub(self.last_note)) as f64 / self.fs;
        let a = if self.trend.is_none() { 1.0 } else { 1.0 - (-dt / phrase.max(0.05)).exp() };
        self.trend = Some(trend + a * (level - trend));
        self.last_note = n_det;
        let s = n_det.saturating_sub(self.r0 as u64);
        let e = n_det.saturating_sub(self.r1 as u64).max(s + 1);
        self.ramps.push_back((s, e, gain));
        #[cfg(test)]
        self.notes.push((n_det, level, gain));
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 3;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let look = (LOOK_MS * 0.001 * fs).round() as usize;
        let hlen = ((0.08 * fs) as usize).next_power_of_two();
        let mut hp = Biquad::identity();
        hp.set_highpass(fs, 30.0, 0.7);
        let mut env_lp = Biquad::identity();
        env_lp.set_lowpass(fs, 40.0, 0.707);
        let mut hf = Biquad::identity();
        hf.set_highpass(fs, 2000.0, 0.707);
        let mut hf_lp = Biquad::identity();
        hf_lp.set_lowpass(fs, 150.0, 0.707);
        let mut d = Self {
            env_lp, hf, hf_lp, pow_hf: 0.0, blk_min_hf: 120.0, mins_hf: [-120.0; 30],
            fs, v, look, meas: 0, pre: 0, r0: 0, r1: 0,
            dl: vec![vec![0.0; look + 1]; ch.max(2)], dpos: 0,
            hist_sq: vec![0.0; hlen], hist_abs: vec![0.0; hlen], hmask: hlen - 1, n: 0,
            hp, pow: 0.0, ms_len: (fs / 1000.0).round() as usize, ms_count: 0,
            blk_min: 0.0, mins: [-120.0; 30], mpos: 0, last_onset: 0, armed: true, pending: VecDeque::new(),
            trend: None, last_note: 0, ramps: VecDeque::new(), cur_db: 0.0, from_db: 0.0, notes: Vec::new(),
            out: Smooth::new(fs, 20.0, 1.0), flash: 0.0,
        };
        d.meas = d.ms(MEAS_MS);
        d.pre = d.ms(PRE_MS);
        d.r0 = d.ms(RAMP_START_MS);
        d.r1 = d.ms(RAMP_END_MS);
        d.blk_min = 120.0;
        d
    }
    fn latency(&self) -> u32 {
        self.look as u32
    }
    fn reset(&mut self) {
        for c in self.dl.iter_mut() {
            c.iter_mut().for_each(|x| *x = 0.0);
        }
        self.hist_sq.iter_mut().for_each(|x| *x = 0.0);
        self.hist_abs.iter_mut().for_each(|x| *x = 0.0);
        self.hp.reset();
        self.pow = 0.0;
        self.mins = [-120.0; 30];
        self.mins_hf = [-120.0; 30];
        self.blk_min_hf = 120.0;
        self.env_lp.reset();
        self.hf.reset();
        self.hf_lp.reset();
        self.pow_hf = 0.0;
        self.blk_min = 120.0;
        self.ms_count = 0;
        self.pending.clear();
        self.ramps.clear();
        self.trend = None;
        self.cur_db = 0.0;
        self.from_db = 0.0;
        self.n = (self.look + self.r0 + 1) as u64;
        self.last_onset = 0;
        self.armed = true;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let len = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().max(1);
        let sens = self.v[SENS] as f64;
        let gap = (self.v[GAP] as f64 * 0.001 * self.fs) as u64;
        let mut onset_here = false;
        let mut extreme = 0.0f64;
        for k in 0..len {
            // ---- detector side (input time n) ----
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch as f64;
            let xh = self.hp.process(x);
            let hk = (self.n as usize) & self.hmask;
            self.hist_sq[hk] = xh * xh;
            self.hist_abs[hk] = xh.abs();
            // power envelope: x squared through a 40 Hz low-pass (ripple-free down to a bass's low B,
            // and quick enough to see the dip-and-rise when a note is re-picked)
            self.pow = self.env_lp.process(xh * xh);
            let db = 10.0 * self.pow.max(1e-18).log10();
            // pick / stick transients: the band above 2 kHz, with a faster envelope
            let xhf = self.hf.process(x);
            self.pow_hf = self.hf_lp.process(xhf * xhf);
            let db_hf = 10.0 * self.pow_hf.max(1e-18).log10();
            self.blk_min_hf = self.blk_min_hf.min(db_hf);
            self.blk_min = self.blk_min.min(db);
            self.ms_count += 1;
            if self.ms_count >= self.ms_len {
                self.ms_count = 0;
                self.mins[self.mpos] = self.blk_min;
                self.mins_hf[self.mpos] = self.blk_min_hf;
                self.mpos = (self.mpos + 1) % self.mins.len();
                self.blk_min = 120.0;
                self.blk_min_hf = 120.0;
            }
            let floor = self.mins.iter().cloned().fold(120.0f64, f64::min).min(self.blk_min);
            let floor_hf = self.mins_hf.iter().cloned().fold(120.0f64, f64::min).min(self.blk_min_hf);
            let rise = (db - floor).max(if db_hf > -60.0 && db_hf > db - 30.0 { db_hf - floor_hf - 6.0 } else { 0.0 });
            // re-arm once the rise has settled, so one attack is one note
            if rise < 0.5 * sens {
                self.armed = true;
            }
            if self.armed && rise > sens && db > FLOOR_DB && self.n.saturating_sub(self.last_onset) >= gap {
                self.armed = false;
                // the attack began a little before the threshold was crossed
                let n_det = self.n.saturating_sub(self.ms(4.0) as u64);
                self.last_onset = self.n;
                self.pending.push_back(n_det);
                onset_here = true;
            }
            while let Some(&nd) = self.pending.front() {
                if self.n >= nd + self.meas as u64 {
                    self.pending.pop_front();
                    self.on_measured(nd);
                } else {
                    break;
                }
            }
            // ---- output side (input time n - look) ----
            let n_out = self.n.saturating_sub(self.look as u64);
            while let Some(&(s, e, t)) = self.ramps.front() {
                if n_out < s {
                    break;
                }
                if n_out == s {
                    self.from_db = self.cur_db;
                }
                if n_out >= e {
                    self.cur_db = t;
                    self.from_db = t;
                    self.ramps.pop_front();
                    continue;
                }
                let u = (n_out - s) as f64 / (e - s) as f64;
                // raised-cosine move, finished before the attack arrives
                let w = 0.5 - 0.5 * (std::f64::consts::PI * u).cos();
                self.cur_db = self.from_db + (t - self.from_db) * w;
                break;
            }
            if self.cur_db.abs() > extreme.abs() {
                extreme = self.cur_db;
            }
            let g = db_to_gain(self.cur_db) * self.out.next();
            let p = self.dpos;
            let nl = self.look + 1;
            let nl_ch = self.dl.len();
            for (ci, c) in chans.iter_mut().enumerate() {
                let line = &mut self.dl[ci.min(nl_ch - 1)];
                line[p] = c[k];
                let y = line[(p + 1) % nl];
                c[k] = (y as f64 * g) as f32;
            }
            self.dpos = (p + 1) % nl;
            self.n += 1;
        }
        meters[0] = (-extreme).max(0.0) as f32;
        meters[1] = extreme.max(0.0) as f32;
        self.flash = if onset_here { 1.0 } else { 0.0 };
        meters[2] = self.flash;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Even Out My Playing", values: &[] },
        Preset { name: "Bass: Every Note the Same", values: &[(EVEN, 0.9), (PHRASE, 4.0), (RANGE, 12.0), (GAP, 70.0)] },
        Preset { name: "Bass: Just the Strays", values: &[(EVEN, 0.6), (RANGE, 6.0), (GAP, 70.0)] },
        Preset { name: "Picked Guitar, Keep the Swells", values: &[(EVEN, 0.7), (PHRASE, 1.0)] },
        Preset { name: "Strummed Chords", values: &[(EVEN, 0.6), (PHRASE, 2.5), (GAP, 90.0), (SENS, 5.0)] },
        Preset { name: "Fingerstyle / Arpeggios", values: &[(EVEN, 0.65), (PHRASE, 1.5), (GAP, 40.0), (SENS, 3.0)] },
        Preset { name: "Vocal Syllables", values: &[(EVEN, 0.5), (PHRASE, 1.5), (RANGE, 6.0), (GAP, 120.0), (SENS, 5.0)] },
        Preset { name: "Hi-Hat Velocity Evener", values: &[(EVEN, 0.8), (PHRASE, 3.0), (GAP, 35.0), (DETECT, 1.0), (SENS, 6.0)] },
        Preset { name: "Snare / Kick Hits", values: &[(EVEN, 0.7), (PHRASE, 4.0), (RANGE, 8.0), (GAP, 60.0), (DETECT, 1.0), (SENS, 8.0)] },
        Preset { name: "Piano / Keys", values: &[(EVEN, 0.4), (PHRASE, 1.0), (RANGE, 6.0), (GAP, 40.0)] },
        Preset { name: "Total Machine (100%)", values: &[(EVEN, 1.0), (PHRASE, 10.0), (RANGE, 24.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "EVENHAND",
        subtitle: ("LEVELS EVERY NOTE, NOT THE SIGNAL", "KEEPS EACH NOTE'S SHAPE AND YOUR PHRASING"),
        accent: ui::OLIVE,
        size: (W, H),
        items: vec![

            Item::Knob { p: EVEN, x: 100.0, y: 175.0, r: 48.0, label: "EVEN", tip: "How far each note is moved toward the level you have been playing at. 100% = every note the same; 50% = the differences halved. Each note gets one gain for its whole length, so its own attack and decay are untouched.", active: None },
            Item::Meter { m: 0, x: 210.0, y: 104.0, w: 140.0, h: 104.0, kind: MeterKind::GainReduction, caption: "NOTE CUT" },
            Item::Meter { m: 1, x: 370.0, y: 104.0, w: 140.0, h: 104.0, kind: MeterKind::GainReduction, caption: "NOTE BOOST" },
            Item::Lamp { m: 2, x: 360.0, y: 236.0, label: "NOTE", lo: 0.0, hi: 1.0 },
            Item::Knob { p: PHRASE, x: 565.0, y: 175.0, r: 38.0, label: "PHRASE", tip: "Your own swells and fades that take longer than this are kept. Short: only note-to-note unevenness is removed and crescendos survive. Long: everything is pulled to one level.", active: None },
            Item::Divider { y: 270.0 },
            Item::Knob { p: RANGE, x: 80.0, y: 335.0, r: 22.0, label: "RANGE", tip: "The most any note is turned up or down (dB).", active: None },
            Item::Knob { p: SENS, x: 185.0, y: 335.0, r: 22.0, label: "SENSITIVITY", tip: "How big a jump in level (dB) counts as a new note. Lower finds softer, legato notes; higher only catches clear attacks.", active: None },
            Item::Knob { p: GAP, x: 290.0, y: 335.0, r: 22.0, label: "SHORTEST NOTE", tip: "Two notes closer than this count as one (stops double triggers on a single pick or strum).", active: None },
            Item::Switch { p: DETECT, x: 410.0, y: 335.0, labels: ["BODY", "ATTACK"], tip: "How a note's loudness is judged: BODY (RMS over its first 20 ms, natural for most sources) or ATTACK (its peak, for drums and percussive hits)." },
            Item::Knob { p: OUTPUT, x: 550.0, y: 335.0, r: 22.0, label: "OUTPUT", tip: "Output level. Adds 32 ms of latency (lookahead), which your host compensates.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Evenhand", class_id: *b"PQEvenhandPQD001", vendor: "PEQUOD",
    categories: [Fx, Dynamics]
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

    /// Plucked-style notes: fast attack, exponential decay, given levels (dBFS peak) and spacing.
    fn notes(levels: &[f64], every: f64, f: f64) -> Vec<f32> {
        let step = (every * 48000.0) as usize;
        let mut x = vec![0.0f32; step * levels.len() + 24000];
        for (i, l) in levels.iter().enumerate() {
            let a = 10f64.powf(l / 20.0);
            for k in 0..step {
                let t = k as f64 / 48000.0;
                // the pick touching the string damps it for the last ~12 ms before the next note
                let damp = ((step - k) as f64 / 576.0).min(1.0);
                let env = (1.0 - (-t / 0.002).exp()) * (-t / 0.25).exp() * damp;
                x[i * step + k] += (a * env * (2.0 * std::f64::consts::PI * f * t).sin()) as f32;
            }
        }
        x
    }
    fn note_peaks(x: &[f32], n: usize, every: f64, lat: usize) -> Vec<f64> {
        let step = (every * 48000.0) as usize;
        (0..n).map(|i| {
            let s = i * step + lat;
            db(x[s..s + 4800].iter().fold(0.0f32, |m, v| m.max(v.abs())) as f64)
        }).collect()
    }
    fn go(p: &[f32], x: &[f32]) -> Vec<f32> {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        input.remove(0)
    }
    #[test]
    fn uneven_notes_come_out_even() {
        let lv = [-12.0, -24.0, -14.0, -22.0, -12.0, -20.0, -16.0, -24.0, -13.0, -19.0, -15.0, -23.0];
        let x = notes(&lv, 0.3, 110.0);
        let p = with(defaults(), &[(EVEN, 1.0), (RANGE, 24.0), (PHRASE, 10.0)]);
        let y = go(&p, &x);
        let lat = Dsp::new(48000.0, 2).latency() as usize;
        let pk = note_peaks(&y, lv.len(), 0.3, lat);
        let tail = &pk[4..];
        let spread = tail.iter().cloned().fold(-99.0, f64::max) - tail.iter().cloned().fold(99.0, f64::min);
        println!("in spread 12 dB, out note peaks {:?}, spread {spread:.1} dB", pk.iter().map(|v| (v * 10.0).round() / 10.0).collect::<Vec<_>>());
        assert!(spread < 2.5);
    }
    #[test]
    fn each_note_keeps_its_shape() {
        let lv = [-12.0, -24.0, -12.0, -24.0, -12.0, -24.0];
        let x = notes(&lv, 0.4, 220.0);
        let p = with(defaults(), &[(EVEN, 1.0), (RANGE, 24.0)]);
        let y = go(&p, &x);
        let lat = Dsp::new(48000.0, 2).latency() as usize;
        // inside every note (from its attack to just before the next), out/in gain is one constant
        let step = 19200;
        let mut worst = 0.0f64;
        for i in 1..5 {
            let mut ratios = vec![];
            for k in (i * step + 200..(i + 1) * step - 800).step_by(97) {
                let a = x[k].abs() as f64;
                if a > 1e-3 {
                    ratios.push(db(y[k + lat].abs() as f64) - db(a));
                }
            }
            let r = ratios.iter().cloned().fold(-99.0, f64::max) - ratios.iter().cloned().fold(99.0, f64::min);
            worst = worst.max(r);
        }
        println!("largest gain change inside a note: {worst:.3} dB");
        assert!(worst < 0.05);
    }
    #[test]
    fn slow_crescendo_is_kept() {
        // 12 notes rising 1 dB each (a 3 s crescendo), with +-4 dB unevenness on top
        let lv: Vec<f64> = (0..12).map(|i| -24.0 + i as f64 + if i % 2 == 0 { 4.0 } else { -4.0 }).collect();
        let x = notes(&lv, 0.25, 147.0);
        let p = with(defaults(), &[(EVEN, 1.0), (PHRASE, 0.4), (RANGE, 24.0)]);
        let y = go(&p, &x);
        let lat = Dsp::new(48000.0, 2).latency() as usize;
        let pk = note_peaks(&y, 12, 0.25, lat);
        let rise = (pk[10] + pk[11]) / 2.0 - (pk[2] + pk[3]) / 2.0;
        let jag: f64 = (3..11).map(|i| (pk[i] - (pk[i - 1] + pk[i + 1]) / 2.0).abs()).sum::<f64>() / 8.0;
        println!("crescendo kept: {rise:.1} dB of 8; note-to-note jaggedness {jag:.1} dB (was 8)");
        assert!(rise > 5.0 && jag < 3.5);
    }
    #[test]
    fn zero_even_is_a_clean_delay() {
        let x = notes(&[-12.0, -20.0], 0.3, 330.0);
        let p = with(defaults(), &[(EVEN, 0.0)]);
        let y = go(&p, &x);
        let lat = Dsp::new(48000.0, 2).latency() as usize;
        assert_eq!(lat, (0.032f64 * 48000.0).round() as usize);
        let err = (0..x.len() - lat).map(|k| (y[k + lat] - x[k]).abs()).fold(0.0f32, f32::max);
        assert!(err < 1e-6, "{err}");
    }
    #[test]
    fn finds_every_note() {
        let lv = [-12.0, -30.0, -18.0, -36.0, -15.0, -28.0];
        let x = notes(&lv, 0.2, 98.0);
        let mut d = Dsp::new(48000.0, 2);
        d.set_params(&defaults());
        d.reset();
        let mut l = x.clone();
        let mut r = x.clone();
        let mut m = vec![0.0; 3];
        for (a, b) in l.chunks_mut(256).zip(r.chunks_mut(256)) {
            let mut ch: Vec<&mut [f32]> = vec![a, b];
            d.process(&mut ch, &mut m);
        }
        println!("notes found: {:?}", d.notes.iter().map(|(n, l, g)| (*n as i64 - d.notes[0].0 as i64, (l * 10.0).round() / 10.0, (g * 10.0).round() / 10.0)).collect::<Vec<_>>());
        assert_eq!(d.notes.len(), lv.len());
    }

}
