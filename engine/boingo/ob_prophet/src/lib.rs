//! Prophet Five: a five-voice analog polysynth after the Sequential Circuits Prophet-5, one of the synths in Richard Gibbs' Oingo Boingo rig (documented by 1984). Two oscillators per voice with hard sync, the Prophet's POLY-MOD (filter envelope and oscillator B modulating oscillator A's pitch and the filter, for its clangy, vocal and sync-sweep sounds), a 4-pole low-pass, two envelopes and a wheel LFO.

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
pub const H: u32 = 540;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum WaveA { #[name = "Saw"] Saw, #[name = "Pulse"] Pulse, #[name = "Saw + Pulse"] Both }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum WaveB { #[name = "Saw"] Saw, #[name = "Triangle"] Tri, #[name = "Pulse"] Pulse, #[name = "Saw + Pulse"] Both }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum PmDest { #[name = "Freq A"] Freq, #[name = "Filter"] Filter, #[name = "Both"] Both }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum LfoShape { #[name = "Triangle"] Tri, #[name = "Saw"] Saw, #[name = "Square"] Square }
pub const VOICES: usize = 5;


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "awave"]
    pub a_wave: EnumParam<WaveA>,
    #[id = "apw"]
    pub a_pw: FloatParam,
    #[id = "bwave"]
    pub b_wave: EnumParam<WaveB>,
    #[id = "bpw"]
    pub b_pw: FloatParam,
    #[id = "bsemi"]
    pub b_semi: IntParam,
    #[id = "bfine"]
    pub b_fine: FloatParam,
    #[id = "sync"]
    pub sync: BoolParam,
    #[id = "glide"]
    pub glide: FloatParam,
    #[id = "mixa"]
    pub mix_a: FloatParam,
    #[id = "mixb"]
    pub mix_b: FloatParam,
    #[id = "noise"]
    pub noise: FloatParam,
    #[id = "pmenv"]
    pub pm_env: FloatParam,
    #[id = "pmosc"]
    pub pm_osc: FloatParam,
    #[id = "pmdest"]
    pub pm_dest: EnumParam<PmDest>,
    #[id = "cutoff"]
    pub cutoff: FloatParam,
    #[id = "reso"]
    pub reso: FloatParam,
    #[id = "fenv"]
    pub f_env: FloatParam,
    #[id = "kbd"]
    pub kbd: BoolParam,
    #[id = "fa"]
    pub f_a: FloatParam,
    #[id = "fd"]
    pub f_d: FloatParam,
    #[id = "fs_level"]
    pub f_s: FloatParam,
    #[id = "fr"]
    pub f_r: FloatParam,
    #[id = "aa"]
    pub a_a: FloatParam,
    #[id = "ad"]
    pub a_d: FloatParam,
    #[id = "as_level"]
    pub a_s: FloatParam,
    #[id = "ar"]
    pub a_r: FloatParam,
    #[id = "lshape"]
    pub l_shape: EnumParam<LfoShape>,
    #[id = "lrate"]
    pub l_rate: FloatParam,
    #[id = "lpitch"]
    pub l_pitch: FloatParam,
    #[id = "lfilt"]
    pub l_filt: FloatParam,
    #[id = "lpw"]
    pub l_pw: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            a_wave: EnumParam::new("Osc A Wave", WaveA::Saw),
            a_pw: pct("Osc A Pulse Width", 0.5),
            b_wave: EnumParam::new("Osc B Wave", WaveB::Saw),
            b_pw: pct("Osc B Pulse Width", 0.5),
            b_semi: IntParam::new("Osc B Semitones", 0, IntRange::Linear { min: -24, max: 24 }),
            b_fine: FloatParam::new("Osc B Fine", 7.0, FloatRange::Linear { min: -50.0, max: 50.0 }).with_step_size(0.1).with_unit(" ct"),
            sync: BoolParam::new("Sync", false),
            glide: ms("Glide", 0.0, 0.0, 2000.0),
            mix_a: pct("Mixer Osc A", 0.8),
            mix_b: pct("Mixer Osc B", 0.6),
            noise: pct("Mixer Noise", 0.0),
            pm_env: pct("Poly-Mod Filter Env", 0.0),
            pm_osc: pct("Poly-Mod Osc B", 0.0),
            pm_dest: EnumParam::new("Poly-Mod Destination", PmDest::Freq),
            cutoff: pct("Cutoff", 0.55),
            reso: pct("Resonance", 0.15),
            f_env: pct("Filter Env Amount", 0.35),
            kbd: BoolParam::new("Keyboard Track", true),
            f_a: ms("Filter Attack", 2.0, 1.0, 5000.0),
            f_d: ms("Filter Decay", 400.0, 5.0, 8000.0),
            f_s: pct("Filter Sustain", 0.3),
            f_r: ms("Filter Release", 300.0, 5.0, 8000.0),
            a_a: ms("Amp Attack", 2.0, 1.0, 5000.0),
            a_d: ms("Amp Decay", 400.0, 5.0, 8000.0),
            a_s: pct("Amp Sustain", 0.8),
            a_r: ms("Amp Release", 300.0, 5.0, 8000.0),
            l_shape: EnumParam::new("LFO Shape", LfoShape::Tri),
            l_rate: rate("LFO Rate", 5.0, 0.1, 25.0),
            l_pitch: pct("LFO to Pitch", 0.0),
            l_filt: pct("LFO to Filter", 0.0),
            l_pw: pct("LFO to Pulse Width", 0.0),
            volume: db("Volume", -3.0, -30.0, 6.0),
        }
    }
}

pub const AWAVE: usize = 0;
pub const APW: usize = 1;
pub const BWAVE: usize = 2;
pub const BPW: usize = 3;
pub const BSEMI: usize = 4;
pub const BFINE: usize = 5;
pub const SYNC: usize = 6;
pub const GLIDE: usize = 7;
pub const MIXA: usize = 8;
pub const MIXB: usize = 9;
pub const NOISE: usize = 10;
pub const PMENV: usize = 11;
pub const PMOSC: usize = 12;
pub const PMDEST: usize = 13;
pub const CUTOFF: usize = 14;
pub const RESO: usize = 15;
pub const FENV: usize = 16;
pub const KBD: usize = 17;
pub const FA: usize = 18;
pub const FD: usize = 19;
pub const FS: usize = 20;
pub const FR: usize = 21;
pub const AA: usize = 22;
pub const AD: usize = 23;
pub const AS: usize = 24;
pub const AR: usize = 25;
pub const LSHAPE: usize = 26;
pub const LRATE: usize = 27;
pub const LPITCH: usize = 28;
pub const LFILT: usize = 29;
pub const LPW: usize = 30;
pub const VOLUME: usize = 31;
pub const NPARAMS: usize = 32;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.a_wave.as_ptr(),
            self.a_pw.as_ptr(),
            self.b_wave.as_ptr(),
            self.b_pw.as_ptr(),
            self.b_semi.as_ptr(),
            self.b_fine.as_ptr(),
            self.sync.as_ptr(),
            self.glide.as_ptr(),
            self.mix_a.as_ptr(),
            self.mix_b.as_ptr(),
            self.noise.as_ptr(),
            self.pm_env.as_ptr(),
            self.pm_osc.as_ptr(),
            self.pm_dest.as_ptr(),
            self.cutoff.as_ptr(),
            self.reso.as_ptr(),
            self.f_env.as_ptr(),
            self.kbd.as_ptr(),
            self.f_a.as_ptr(),
            self.f_d.as_ptr(),
            self.f_s.as_ptr(),
            self.f_r.as_ptr(),
            self.a_a.as_ptr(),
            self.a_d.as_ptr(),
            self.a_s.as_ptr(),
            self.a_r.as_ptr(),
            self.l_shape.as_ptr(),
            self.l_rate.as_ptr(),
            self.l_pitch.as_ptr(),
            self.l_filt.as_ptr(),
            self.l_pw.as_ptr(),
            self.volume.as_ptr(),
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

#[inline]
fn blep(t: f64, dt: f64) -> f64 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Default)]
struct Env {
    v: f64,
    stage: u8,
}

impl Env {
    #[inline]
    fn step(&mut self, ca: f64, cd: f64, s: f64, cr: f64) -> f64 {
        match self.stage {
            1 => {
                self.v += (1.25 - self.v) * ca;
                if self.v >= 1.0 {
                    self.v = 1.0;
                    self.stage = 2;
                }
            }
            2 => self.v += (s - self.v) * cd,
            4 => self.v += (0.0 - self.v) * cr,
            _ => {}
        }
        self.v
    }
}

#[derive(Clone, Copy, Default)]
struct Voice {
    note: u8,
    gate: bool,
    on: bool,
    vel: f64,
    pitch: f64,
    pa: f64,
    pb: f64,
    fenv: Env,
    aenv: Env,
    lad: [f64; 4],
    age: u64,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    voices: [Voice; VOICES],
    clock: u64,
    noise: Noise,
    lph: f64,
    bend: f64,
    modw: f64,
    last_note: f64,
    dc: DcBlock,
}

impl Dsp {
    fn note_on(&mut self, note: u8, vel: f64) {
        self.clock += 1;
        let i = self.voices.iter().position(|v| v.on && v.note == note)
            .or_else(|| self.voices.iter().position(|v| !v.on))
            .unwrap_or_else(|| self.voices.iter().enumerate().min_by_key(|(_, v)| v.age).map(|(i, _)| i).unwrap_or(0));
        let glide = self.v[GLIDE] > 0.5;
        let start = if glide && self.last_note > 0.0 { self.last_note } else { note as f64 };
        let v = &mut self.voices[i];
        if !v.on {
            v.pitch = start;
        } else if !glide {
            v.pitch = note as f64;
        }
        v.note = note;
        v.gate = true;
        v.on = true;
        v.vel = 0.65 + 0.35 * vel;
        v.fenv.stage = 1;
        v.aenv.stage = 1;
        v.age = self.clock;
        self.last_note = note as f64;
    }
    fn note_off(&mut self, note: u8) {
        for v in self.voices.iter_mut().filter(|v| v.on && v.gate && v.note == note) {
            v.gate = false;
            v.fenv.stage = 4;
            v.aenv.stage = 4;
        }
    }
    pub fn voices_on(&self) -> usize {
        self.voices.iter().filter(|v| v.on).count()
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, voices: [Voice::default(); VOICES], clock: 0, noise: Noise::new(9), lph: 0.0, bend: 0.0, modw: 0.0, last_note: 0.0, dc: DcBlock::new(fs) }
    }
    fn reset(&mut self) {
        self.voices = [Voice::default(); VOICES];
        self.lph = 0.0;
        self.last_note = 0.0;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn midi(&mut self, ev: Midi) {
        match ev {
            Midi::On { note, vel } => self.note_on(note, vel as f64),
            Midi::Off { note } => self.note_off(note),
            Midi::Bend { value } => self.bend = value as f64 * 2.0,
            Midi::Cc { cc: 1, value } => self.modw = value as f64,
            Midi::AllOff => {
                for v in self.voices.iter_mut() {
                    v.gate = false;
                    v.fenv.stage = 4;
                    v.aenv.stage = 4;
                }
            }
            _ => {}
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let p = |i: usize| self.v[i] as f64;
        let awave = p(AWAVE).round() as usize;
        let bwave = p(BWAVE).round() as usize;
        let (apw, bpw) = (p(APW), p(BPW));
        let bratio = 2f64.powf((p(BSEMI) + p(BFINE) * 0.01) / 12.0);
        let sync = p(SYNC) > 0.5;
        let glide_c = if p(GLIDE) < 1.0 { 0.0 } else { coeff(fs, p(GLIDE) / 3.0) };
        let (mixa, mixb, noise) = (p(MIXA), p(MIXB), p(NOISE));
        let (pmenv, pmosc) = (p(PMENV), p(PMOSC));
        let pmdest = p(PMDEST).round() as usize;
        let (pm_freq, pm_filt) = (pmdest != 1, pmdest != 0);
        let (cut, reso, famt, kbd) = (p(CUTOFF), p(RESO), p(FENV), p(KBD) > 0.5);
        let c = |ms: f64, k: f64| 1.0 - coeff(fs, ms / k);
        let (fa, fd, fsus, fr) = (c(p(FA), 2.0), c(p(FD), 4.0), p(FS), c(p(FR), 4.0));
        let (aa, ad, asus, ar) = (c(p(AA), 2.0), c(p(AD), 4.0), p(AS), c(p(AR), 4.0));
        let lshape = p(LSHAPE).round() as usize;
        let (lrate, lpitch, lfilt, lpw) = (p(LRATE), p(LPITCH), p(LFILT), p(LPW));
        let vol = db_to_gain(p(VOLUME));
        let k = 4.0 * reso;
        let mut peak = 0.0f64;
        for i in 0..n {
            self.lph = (self.lph + lrate / fs).fract();
            let l = match lshape {
                0 => 1.0 - 4.0 * (self.lph - 0.5).abs(),
                1 => 2.0 * self.lph - 1.0,
                _ => if self.lph < 0.5 { 1.0 } else { -1.0 },
            };
            // the mod wheel adds to the LFO amounts, as on the Prophet's wheel-mod section
            let wheel = self.modw;
            let vib = (lpitch + 0.6 * wheel) * l * 0.5;
            let mut sum = 0.0;
            for v in self.voices.iter_mut() {
                if !v.on {
                    continue;
                }
                let fe = v.fenv.step(fa, fd, fsus, fr);
                let ae = v.aenv.step(aa, ad, asus, ar);
                if v.aenv.stage == 4 && ae < 1e-5 {
                    v.on = false;
                    v.aenv.v = 0.0;
                    v.fenv.v = 0.0;
                    continue;
                }
                v.pitch = v.note as f64 + glide_c * (v.pitch - v.note as f64);
                let base = 440.0 * 2f64.powf((v.pitch - 69.0 + self.bend + vib) / 12.0);
                // oscillator B
                let fb = base * bratio;
                let dtb = (fb / fs).min(0.45);
                v.pb += dtb;
                let wrapped = v.pb >= 1.0;
                if wrapped {
                    v.pb -= 1.0;
                }
                let tb = v.pb;
                let wb = (bpw * 0.9 + 0.05 + 0.4 * lpw * l).clamp(0.05, 0.95);
                let saw_b = 2.0 * tb - 1.0 - blep(tb, dtb);
                let mut pul_b = if tb < wb { 1.0 } else { -1.0 };
                pul_b += blep(tb, dtb) - blep((tb - wb + 1.0).fract(), dtb);
                let ob = match bwave {
                    0 => saw_b,
                    1 => 1.0 - 4.0 * (tb - 0.5).abs(),
                    2 => pul_b,
                    _ => 0.5 * (saw_b + pul_b),
                };
                // poly-mod: filter envelope and osc B into osc A's pitch and/or the filter
                let pm = pmenv * fe * 4.0 + pmosc * ob * 2.0;
                let fa_hz = base * if pm_freq { 2f64.powf(pm) } else { 1.0 };
                let dta = (fa_hz / fs).clamp(0.0, 0.45);
                v.pa += dta;
                if v.pa >= 1.0 {
                    v.pa -= 1.0;
                }
                if sync && wrapped {
                    v.pa = tb * dta / dtb.max(1e-9);
                }
                let ta = v.pa;
                let wa = (apw * 0.9 + 0.05 + 0.4 * lpw * l).clamp(0.05, 0.95);
                let saw_a = 2.0 * ta - 1.0 - blep(ta, dta);
                let mut pul_a = if ta < wa { 1.0 } else { -1.0 };
                pul_a += blep(ta, dta) - blep((ta - wa + 1.0).fract(), dta);
                let oa = match awave {
                    0 => saw_a,
                    1 => pul_a,
                    _ => 0.5 * (saw_a + pul_a),
                };
                let x = oa * mixa + ob * mixb + noise * self.noise.next();
                // CEM-style 4-pole low-pass
                let key = if kbd { (base / 261.6).log2() } else { 0.0 };
                let mut ex = 9.0 * cut + 7.0 * famt * fe + 2.5 * (lfilt + 0.3 * wheel) * l + key;
                if pm_filt {
                    ex += pm;
                }
                let fc = (40.0 * 2f64.powf(ex)).clamp(20.0, fs * 0.42);
                let g = 1.0 - (-2.0 * std::f64::consts::PI * fc / fs).exp();
                let u = (x * 0.3 - k * v.lad[3]).tanh();
                v.lad[0] += g * (u - v.lad[0]);
                v.lad[1] += g * (v.lad[0] - v.lad[1]);
                v.lad[2] += g * (v.lad[1] - v.lad[2]);
                v.lad[3] += g * (v.lad[2] - v.lad[3]);
                sum += v.lad[3] * (1.0 + 0.5 * reso) * ae * v.vel;
            }
            let y = self.dc.process(sum * vol * 1.2);
            peak = peak.max(y.abs());
            for ch in chans.iter_mut() {
                ch[i] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Brassy Poly (Little Girls-ish stabs)", values: &[(AWAVE, 0.0), (BWAVE, 0.0), (BFINE, 8.0), (CUTOFF, 0.38), (RESO, 0.1), (FENV, 0.55), (FA, 20.0), (FD, 350.0), (FS, 0.35), (AA, 5.0), (AS, 0.85), (AR, 250.0)] },
        Preset { name: "Sync Sweep Lead", values: &[(AWAVE, 0.0), (BWAVE, 0.0), (SYNC, 1.0), (PMENV, 0.6), (PMDEST, 0.0), (MIXB, 0.0), (CUTOFF, 0.6), (FENV, 0.3), (FD, 900.0), (FS, 0.2), (GLIDE, 60.0)] },
        Preset { name: "Poly-Mod Bells", values: &[(AWAVE, 1.0), (BWAVE, 1.0), (BSEMI, 19.0), (PMOSC, 0.6), (PMDEST, 0.0), (MIXB, 0.0), (CUTOFF, 0.7), (FENV, 0.2), (AD, 1400.0), (AS, 0.0), (AR, 1200.0)] },
        Preset { name: "Warm Strings", values: &[(AWAVE, 0.0), (BWAVE, 0.0), (BFINE, 12.0), (CUTOFF, 0.5), (FENV, 0.1), (AA, 450.0), (AR, 1400.0), (AS, 1.0), (LPITCH, 0.04), (LRATE, 5.5)] },
        Preset { name: "Fat Bass", values: &[(AWAVE, 2.0), (BWAVE, 0.0), (BSEMI, -12.0), (BFINE, 4.0), (CUTOFF, 0.28), (RESO, 0.3), (FENV, 0.45), (FD, 260.0), (FS, 0.15), (AS, 0.7), (AR, 120.0)] },
        Preset { name: "Clangy Poly-Mod Filter", values: &[(AWAVE, 0.0), (BWAVE, 2.0), (BSEMI, 7.0), (PMOSC, 0.5), (PMDEST, 1.0), (CUTOFF, 0.45), (RESO, 0.45), (FENV, 0.35)] },
        Preset { name: "Square Organ", values: &[(AWAVE, 1.0), (APW, 0.5), (BWAVE, 2.0), (BSEMI, 12.0), (CUTOFF, 0.62), (FENV, 0.0), (AA, 3.0), (AS, 1.0), (AR, 80.0)] },
        Preset { name: "Resonant Sweep Pad", values: &[(CUTOFF, 0.3), (RESO, 0.65), (LFILT, 0.35), (LRATE, 0.25), (AA, 700.0), (AR, 2000.0), (AS, 1.0), (FENV, 0.05)] },
        Preset { name: "Nervous Arpeggio Pluck", values: &[(AWAVE, 1.0), (APW, 0.3), (BWAVE, 0.0), (CUTOFF, 0.35), (FENV, 0.6), (FD, 140.0), (FS, 0.0), (AD, 220.0), (AS, 0.0), (AR, 120.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "PROPHET FIVE",
        subtitle: ("AFTER THE SEQUENTIAL PROPHET-5", "5 VOICE · SYNC · POLY-MOD · MIDI"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::Selector { p: AWAVE, x: 60.0, y: 140.0, label: "A WAVE", positions: &["SAW", "PULSE", "BOTH"], tip: "Oscillator A waveform." },
            Item::Knob { p: APW, x: 135.0, y: 140.0, r: 19.0, label: "A PW", tip: "Oscillator A pulse width.", active: None },
            Item::Selector { p: BWAVE, x: 215.0, y: 140.0, label: "B WAVE", positions: &["SAW", "TRI", "PULSE", "BOTH"], tip: "Oscillator B waveform." },
            Item::Knob { p: BPW, x: 305.0, y: 140.0, r: 19.0, label: "B PW", tip: "Oscillator B pulse width.", active: None },
            Item::Knob { p: BSEMI, x: 365.0, y: 140.0, r: 19.0, label: "B SEMI", tip: "Oscillator B tuning in semitones.", active: None },
            Item::Knob { p: BFINE, x: 425.0, y: 140.0, r: 19.0, label: "B FINE", tip: "Oscillator B fine tune: detune against A for the fat Prophet sound.", active: None },
            Item::Switch { p: SYNC, x: 475.0, y: 140.0, labels: ["", "SYNC"], tip: "Hard-syncs A to B: sweep A's pitch (poly-mod from the filter envelope) for the sync-lead sound." },
            Item::Knob { p: GLIDE, x: 585.0, y: 140.0, r: 19.0, label: "GLIDE", tip: "Portamento between notes.", active: None },
            Item::Divider { y: 190.0 },
            Item::Knob { p: MIXA, x: 50.0, y: 240.0, r: 19.0, label: "MIX A", tip: "Oscillator A level.", active: None },
            Item::Knob { p: MIXB, x: 115.0, y: 240.0, r: 19.0, label: "MIX B", tip: "Oscillator B level.", active: None },
            Item::Knob { p: NOISE, x: 180.0, y: 240.0, r: 19.0, label: "NOISE", tip: "Noise level.", active: None },
            Item::Knob { p: PMENV, x: 265.0, y: 240.0, r: 19.0, label: "PM ENV", tip: "Poly-mod: the filter envelope modulating the destination.", active: None },
            Item::Knob { p: PMOSC, x: 330.0, y: 240.0, r: 19.0, label: "PM OSC B", tip: "Poly-mod: oscillator B modulating the destination (FM).", active: None },
            Item::Selector { p: PMDEST, x: 410.0, y: 240.0, label: "PM DEST", positions: &["FREQ A", "FILT", "BOTH"], tip: "Poly-mod destination: oscillator A's pitch, the filter, or both." },
            Item::Knob { p: VOLUME, x: 570.0, y: 240.0, r: 24.0, label: "VOLUME", tip: "Output level. Mod wheel adds LFO vibrato and filter wobble; pitch bend +/- 2 semitones.", active: None },
            Item::Divider { y: 290.0 },
            Item::Knob { p: CUTOFF, x: 50.0, y: 340.0, r: 19.0, label: "CUTOFF", tip: "Low-pass cutoff.", active: None },
            Item::Knob { p: RESO, x: 115.0, y: 340.0, r: 19.0, label: "RESO", tip: "Resonance.", active: None },
            Item::Knob { p: FENV, x: 180.0, y: 340.0, r: 19.0, label: "ENV AMT", tip: "How far the filter envelope opens the filter.", active: None },
            Item::Switch { p: KBD, x: 220.0, y: 340.0, labels: ["", "KEYBD"], tip: "Filter follows the keyboard." },
            Item::Knob { p: FA, x: 350.0, y: 340.0, r: 19.0, label: "F ATT", tip: "Filter envelope attack.", active: None },
            Item::Knob { p: FD, x: 415.0, y: 340.0, r: 19.0, label: "F DEC", tip: "Filter envelope decay.", active: None },
            Item::Knob { p: FS, x: 480.0, y: 340.0, r: 19.0, label: "F SUS", tip: "Filter envelope sustain.", active: None },
            Item::Knob { p: FR, x: 545.0, y: 340.0, r: 19.0, label: "F REL", tip: "Filter envelope release.", active: None },
            Item::Divider { y: 390.0 },
            Item::Knob { p: AA, x: 50.0, y: 440.0, r: 19.0, label: "ATTACK", tip: "Amp attack.", active: None },
            Item::Knob { p: AD, x: 115.0, y: 440.0, r: 19.0, label: "DECAY", tip: "Amp decay.", active: None },
            Item::Knob { p: AS, x: 180.0, y: 440.0, r: 19.0, label: "SUSTAIN", tip: "Amp sustain.", active: None },
            Item::Knob { p: AR, x: 245.0, y: 440.0, r: 19.0, label: "RELEASE", tip: "Amp release.", active: None },
            Item::Selector { p: LSHAPE, x: 330.0, y: 440.0, label: "LFO", positions: &["TRI", "SAW", "SQR"], tip: "LFO shape." },
            Item::Knob { p: LRATE, x: 405.0, y: 440.0, r: 19.0, label: "LFO RATE", tip: "LFO speed.", active: None },
            Item::Knob { p: LPITCH, x: 465.0, y: 440.0, r: 19.0, label: "> PITCH", tip: "LFO to pitch (vibrato).", active: None },
            Item::Knob { p: LFILT, x: 525.0, y: 440.0, r: 19.0, label: "> FILT", tip: "LFO to the filter.", active: None },
            Item::Knob { p: LPW, x: 585.0, y: 440.0, r: 19.0, label: "> PW", tip: "LFO to both pulse widths.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Prophet Five", class_id: *b"OBProphetFivePQ1", vendor: "PEQUOD Oingo Boingo",
    instrument, categories: [Instrument, Synth]
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

    fn render(d: &mut Dsp, n: usize) -> Vec<f32> {
        let mut m = [0.0f32];
        let mut l = vec![0.0f32; n];
        let mut r = l.clone();
        {
            let mut ch: Vec<&mut [f32]> = vec![&mut l, &mut r];
            d.process(&mut ch, &mut m);
        }
        l
    }
    #[test]
    fn in_tune() {
        let mut d = Dsp::new(48000.0, 2);
        d.set_params(&with(defaults(), &[(MIXB, 0.0), (CUTOFF, 1.0), (FENV, 0.0), (AS, 1.0)]));
        d.midi(Midi::On { note: 57, vel: 1.0 });
        let l = render(&mut d, 48000);
        let z = l[24000..].windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        println!("A3 -> {} Hz", z * 2);
        assert!((z as f64 * 2.0 - 220.0).abs() < 8.0);
    }
    #[test]
    fn five_voices() {
        let mut d = Dsp::new(48000.0, 2);
        d.set_params(&defaults());
        for nt in [48u8, 52, 55, 59, 62, 65] {
            d.midi(Midi::On { note: nt, vel: 1.0 });
        }
        let l = render(&mut d, 4800);
        println!("voices on: {}", d.voices_on());
        assert!(d.voices_on() == 5 && rms(&l) > 1e-3);
    }
    #[test]
    fn poly_mod_brightens() {
        let mut a = Dsp::new(48000.0, 2);
        a.set_params(&with(defaults(), &[(MIXB, 0.0), (AWAVE, 1.0), (CUTOFF, 0.9), (FENV, 0.0), (AS, 1.0)]));
        a.midi(Midi::On { note: 45, vel: 1.0 });
        let x = render(&mut a, 24000);
        let mut b = Dsp::new(48000.0, 2);
        b.set_params(&with(defaults(), &[(MIXB, 0.0), (AWAVE, 1.0), (CUTOFF, 0.9), (FENV, 0.0), (AS, 1.0), (PMOSC, 0.7), (BSEMI, 7.0)]));
        b.midi(Midi::On { note: 45, vel: 1.0 });
        let y = render(&mut b, 24000);
        // how much of the energy is still the fundamental: FM from osc B spreads it into sidebands
        let fund = |s: &[f32]| {
            let w = 2.0 * std::f64::consts::PI * 110.0 / 48000.0;
            let (mut s1, mut s2) = (0.0, 0.0);
            for v in s {
                let t = *v as f64 + 2.0 * w.cos() * s1 - s2;
                s2 = s1;
                s1 = t;
            }
            let g = ((s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2).max(0.0)).sqrt() / s.len() as f64 * 2.0;
            db(g / (rms(s) * 2f64.sqrt()))
        };
        println!("fundamental share plain {:.1} dB vs poly-mod {:.1} dB", fund(&x[12000..]), fund(&y[12000..]));
        assert!(fund(&y[12000..]) < fund(&x[12000..]) - 6.0);
    }

}
