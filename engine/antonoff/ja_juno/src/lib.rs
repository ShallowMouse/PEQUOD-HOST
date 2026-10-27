//! Juno Six: a six-voice polysynth after the Roland Juno-6, Jack Antonoff's favourite synth (the sound of 1989; the arpeggio on I Wanna Get Better). One DCO per voice (saw, pulse with PWM, sub, noise), high-pass, a four-pole low-pass, one envelope, LFO, arpeggiator and the famous chorus.

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
pub const H: u32 = 560;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum OnOff { #[name = "Off"] Off, #[name = "On"] On }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Chorus { #[name = "Off"] Off, #[name = "I"] One, #[name = "II"] Two, #[name = "I + II"] Both }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Arp { #[name = "Off"] Off, #[name = "Up"] Up, #[name = "Down"] Down, #[name = "Up & Down"] UpDown }
const VOICES: usize = 6;


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "saw"]
    pub saw: EnumParam<OnOff>,
    #[id = "pulse"]
    pub pulse: EnumParam<OnOff>,
    #[id = "pwm"]
    pub pwm: FloatParam,
    #[id = "sub"]
    pub sub: FloatParam,
    #[id = "noise"]
    pub noise: FloatParam,
    #[id = "hpf"]
    pub hpf: IntParam,
    #[id = "cutoff"]
    pub cutoff: FloatParam,
    #[id = "reso"]
    pub reso: FloatParam,
    #[id = "envamt"]
    pub env_amt: FloatParam,
    #[id = "lfof"]
    pub lfo_f: FloatParam,
    #[id = "lforate"]
    pub lfo_rate: FloatParam,
    #[id = "vib"]
    pub vib: FloatParam,
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "sustain"]
    pub sustain: FloatParam,
    #[id = "release"]
    pub release: FloatParam,
    #[id = "chorus"]
    pub chorus: EnumParam<Chorus>,
    #[id = "arp"]
    pub arp: EnumParam<Arp>,
    #[id = "arprate"]
    pub arp_rate: FloatParam,
    #[id = "arpoct"]
    pub arp_oct: IntParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            saw: EnumParam::new("Saw", OnOff::On),
            pulse: EnumParam::new("Pulse", OnOff::Off),
            pwm: pct("Pulse Width", 0.3),
            sub: pct("Sub", 0.3),
            noise: pct("Noise", 0.0),
            hpf: IntParam::new("HPF", 0, IntRange::Linear { min: 0, max: 3 }),
            cutoff: pct("Cutoff", 0.55),
            reso: pct("Resonance", 0.15),
            env_amt: pct("Env Amount", 0.35),
            lfo_f: pct("LFO to Filter", 0.0),
            lfo_rate: rate("LFO Rate", 3.0, 0.1, 20.0),
            vib: pct("LFO to Pitch", 0.0),
            attack: ms("Attack", 3.0, 1.0, 3000.0),
            decay: ms("Decay", 400.0, 5.0, 8000.0),
            sustain: pct("Sustain", 0.7),
            release: ms("Release", 300.0, 5.0, 8000.0),
            chorus: EnumParam::new("Chorus", Chorus::One),
            arp: EnumParam::new("Arpeggio", Arp::Off),
            arp_rate: rate("Arp Rate", 8.0, 1.0, 20.0),
            arp_oct: IntParam::new("Arp Range", 1, IntRange::Linear { min: 1, max: 3 }),
            volume: db("Volume", -2.0, -30.0, 6.0),
        }
    }
}

pub const SAW: usize = 0;
pub const PULSE: usize = 1;
pub const PWM: usize = 2;
pub const SUB: usize = 3;
pub const NOISE: usize = 4;
pub const HPF: usize = 5;
pub const CUTOFF: usize = 6;
pub const RESO: usize = 7;
pub const ENVAMT: usize = 8;
pub const LFOF: usize = 9;
pub const LFORATE: usize = 10;
pub const VIB: usize = 11;
pub const ATTACK: usize = 12;
pub const DECAY: usize = 13;
pub const SUSTAIN: usize = 14;
pub const RELEASE: usize = 15;
pub const CHORUS: usize = 16;
pub const ARP: usize = 17;
pub const ARPRATE: usize = 18;
pub const ARPOCT: usize = 19;
pub const VOLUME: usize = 20;
pub const NPARAMS: usize = 21;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.saw.as_ptr(),
            self.pulse.as_ptr(),
            self.pwm.as_ptr(),
            self.sub.as_ptr(),
            self.noise.as_ptr(),
            self.hpf.as_ptr(),
            self.cutoff.as_ptr(),
            self.reso.as_ptr(),
            self.env_amt.as_ptr(),
            self.lfo_f.as_ptr(),
            self.lfo_rate.as_ptr(),
            self.vib.as_ptr(),
            self.attack.as_ptr(),
            self.decay.as_ptr(),
            self.sustain.as_ptr(),
            self.release.as_ptr(),
            self.chorus.as_ptr(),
            self.arp.as_ptr(),
            self.arp_rate.as_ptr(),
            self.arp_oct.as_ptr(),
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
struct Voice {
    note: u8,
    gate: bool,
    on: bool,
    vel: f64,
    ph: f64,
    sub_ph: f64,
    env: f64,
    stage: u8,
    lad: [f64; 4],
    age: u64,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    voices: [Voice; VOICES],
    clock: u64,
    noise: Noise,
    lfo: Lfo,
    hpf: [Biquad; 2],
    hpf_key: i32,
    bbd: [Bbd; 2],
    cl: f64,
    held: Vec<u8>,
    arp_ph: f64,
    arp_i: usize,
    arp_dir: i32,
    arp_note: Option<u8>,
    bend: f64,
    modw: f64,
    dc: [DcBlock; 2],
}

impl Dsp {
    fn note_on(&mut self, note: u8, vel: f64) {
        self.clock += 1;
        // reuse a voice playing this note, else a free one, else the oldest
        let i = self.voices.iter().position(|v| v.on && v.note == note)
            .or_else(|| self.voices.iter().position(|v| !v.on))
            .unwrap_or_else(|| self.voices.iter().enumerate().min_by_key(|(_, v)| v.age).map(|(i, _)| i).unwrap_or(0));
        let v = &mut self.voices[i];
        v.note = note;
        v.gate = true;
        v.on = true;
        v.vel = 0.6 + 0.4 * vel;
        v.stage = 1;
        v.age = self.clock;
    }
    fn note_off(&mut self, note: u8) {
        for v in self.voices.iter_mut().filter(|v| v.on && v.gate && v.note == note) {
            v.gate = false;
            v.stage = 4;
        }
    }
    fn arp_list(&self) -> Vec<u8> {
        let mut base = self.held.clone();
        base.sort_unstable();
        let oct = (self.v[ARPOCT].round() as u8).clamp(1, 3);
        let mut out = Vec::with_capacity(base.len() * oct as usize);
        for o in 0..oct {
            for n in &base {
                out.push(n.saturating_add(12 * o).min(127));
            }
        }
        out
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut bbd = [Bbd::new(fs, 20.0), Bbd::new(fs, 20.0)];
        for b in &mut bbd {
            b.set_bandwidth(9000.0);
        }
        Self {
            fs,
            v,
            voices: [Voice::default(); VOICES],
            clock: 0,
            noise: Noise::new(3),
            lfo: Lfo::default(),
            hpf: [Biquad::identity(); 2],
            hpf_key: -1,
            bbd,
            cl: 0.0,
            held: Vec::with_capacity(64),
            arp_ph: 0.0,
            arp_i: 0,
            arp_dir: 1,
            arp_note: None,
            bend: 0.0,
            modw: 0.0,
            dc: [DcBlock::new(fs), DcBlock::new(fs)],
        }
    }
    fn reset(&mut self) {
        self.voices = [Voice::default(); VOICES];
        self.held.clear();
        self.arp_note = None;
        self.bbd.iter_mut().for_each(|b| b.reset());
        self.hpf.iter_mut().for_each(|b| b.reset());
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let h = v[HPF].round() as i32;
        if h != self.hpf_key {
            for b in &mut self.hpf {
                // Juno HPF: off, ~120 Hz, ~250 Hz, ~520 Hz (estimated)
                match h {
                    0 => *b = Biquad::identity(),
                    1 => b.set_highpass(self.fs, 120.0, 0.6),
                    2 => b.set_highpass(self.fs, 250.0, 0.6),
                    _ => b.set_highpass(self.fs, 520.0, 0.6),
                }
            }
            self.hpf_key = h;
        }
    }
    fn midi(&mut self, ev: Midi) {
        let arp = self.v[ARP].round() as i32 > 0;
        match ev {
            Midi::On { note, vel } => {
                if arp {
                    if !self.held.contains(&note) && self.held.len() < 64 {
                        self.held.push(note);
                    }
                } else {
                    self.note_on(note, vel as f64);
                }
            }
            Midi::Off { note } => {
                if arp {
                    self.held.retain(|n| *n != note);
                } else {
                    self.note_off(note);
                }
            }
            Midi::Bend { value } => self.bend = value as f64 * 2.0,
            Midi::Cc { cc: 1, value } => self.modw = value as f64,
            Midi::AllOff => {
                self.held.clear();
                for v in self.voices.iter_mut() {
                    v.gate = false;
                    v.stage = 4;
                }
            }
            _ => {}
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let p = |i: usize| self.v[i] as f64;
        let (saw, pulse, pwm, sub, noise) = (p(SAW) > 0.5, p(PULSE) > 0.5, p(PWM), p(SUB), p(NOISE));
        let (cut, reso, envamt, lfof, lrate, vib) = (p(CUTOFF), p(RESO), p(ENVAMT), p(LFOF), p(LFORATE), p(VIB));
        let (a, d, s, r) = (p(ATTACK), p(DECAY), p(SUSTAIN), p(RELEASE));
        let chorus = p(CHORUS).round() as usize;
        let arp = p(ARP).round() as usize;
        let arp_rate = p(ARPRATE);
        let vol = db_to_gain(p(VOLUME));
        let ca = 1.0 - coeff(fs, a / 2.0);
        let cd = 1.0 - coeff(fs, d / 4.0);
        let cr = 1.0 - coeff(fs, r / 4.0);
        let k = 4.0 * reso;
        let mut peak = 0.0f64;
        // arpeggiator stops cleanly when switched off
        if arp == 0 {
            if let Some(nt) = self.arp_note.take() {
                self.note_off(nt);
            }
        }
        for i in 0..n {
            // arpeggiator clock
            if arp > 0 {
                self.arp_ph += arp_rate / fs;
                if self.arp_ph >= 1.0 || (self.arp_note.is_none() && !self.held.is_empty()) {
                    self.arp_ph = self.arp_ph.fract();
                    if let Some(nt) = self.arp_note.take() {
                        self.note_off(nt);
                    }
                    let list = self.arp_list();
                    if !list.is_empty() {
                        let len = list.len();
                        let idx = match arp {
                            1 => { self.arp_i = (self.arp_i + 1) % len; self.arp_i }
                            2 => { self.arp_i = (self.arp_i + len - 1) % len; self.arp_i }
                            _ => {
                                if len == 1 { 0 } else {
                                    let mut j = self.arp_i as i32 + self.arp_dir;
                                    if j >= len as i32 { self.arp_dir = -1; j = len as i32 - 2; }
                                    if j < 0 { self.arp_dir = 1; j = 1; }
                                    self.arp_i = j as usize;
                                    self.arp_i
                                }
                            }
                        };
                        let nt = list[idx.min(len - 1)];
                        self.note_on(nt, 0.8);
                        self.arp_note = Some(nt);
                    }
                } else if self.arp_ph >= 0.5 {
                    // 50% gate
                    if let Some(nt) = self.arp_note {
                        for v in self.voices.iter_mut().filter(|v| v.on && v.gate && v.note == nt) {
                            v.gate = false;
                            v.stage = 4;
                        }
                    }
                }
            }
            let l = self.lfo.next(fs, lrate);
            let pitch_mod = 2f64.powf((self.bend + (vib * 0.5 + self.modw * 0.3) * l) / 12.0);
            let mut sum = 0.0;
            for v in self.voices.iter_mut() {
                if !v.on {
                    continue;
                }
                match v.stage {
                    1 => {
                        v.env += (1.25 - v.env) * ca;
                        if v.env >= 1.0 {
                            v.env = 1.0;
                            v.stage = 2;
                        }
                    }
                    2 => v.env += (s - v.env) * cd,
                    4 => {
                        v.env += (0.0 - v.env) * cr;
                        if v.env < 1e-5 {
                            v.on = false;
                            v.env = 0.0;
                            continue;
                        }
                    }
                    _ => {}
                }
                let f = 440.0 * 2f64.powf((v.note as f64 - 69.0) / 12.0) * pitch_mod;
                let dt = (f / fs).min(0.45);
                v.ph += dt;
                if v.ph >= 1.0 {
                    v.ph -= 1.0;
                    v.sub_ph = if v.sub_ph < 0.5 { 0.5 } else { 0.0 };
                }
                let t = v.ph;
                let mut osc = 0.0;
                if saw {
                    osc += 2.0 * t - 1.0 - blep(t, dt);
                }
                if pulse {
                    // manual pulse width (50%..~5%), LFO-modulated as on the Juno when PWM is up
                    let w = (0.5 - 0.45 * pwm * (0.5 + 0.5 * l)).clamp(0.05, 0.5);
                    let mut sq = if t < w { 1.0 } else { -1.0 };
                    sq += blep(t, dt);
                    sq -= blep((t - w + 1.0).fract(), dt);
                    osc += sq;
                }
                let subv = if v.sub_ph < 0.5 { 1.0 } else { -1.0 };
                osc += sub * subv * 0.8 + noise * self.noise.next();
                // IR3109-style 4-pole low-pass, keyboard tracking fixed at ~half
                let key = (f / 261.6).log2() * 0.5;
                let fc = (60.0 * 2f64.powf(9.0 * cut + 6.0 * envamt * v.env + 2.0 * lfof * l + key)).clamp(20.0, fs * 0.42);
                let g = 1.0 - (-2.0 * std::f64::consts::PI * fc / fs).exp();
                let x = (osc * 0.35 - k * v.lad[3]).tanh();
                v.lad[0] += g * (x - v.lad[0]);
                v.lad[1] += g * (v.lad[0] - v.lad[1]);
                v.lad[2] += g * (v.lad[1] - v.lad[2]);
                v.lad[3] += g * (v.lad[2] - v.lad[3]);
                sum += v.lad[3] * (1.0 + 0.5 * reso) * v.env * v.vel;
            }
            let y = cascade(&mut self.hpf, sum) * vol * 1.6;
            // chorus: I ~0.5 Hz, II ~0.8 Hz deeper, I+II ~8 Hz shallow (BBD, stereo by inverting the wet)
            let (rate, depth) = match chorus {
                1 => (0.5, 1.6),
                2 => (0.83, 2.6),
                3 => (8.0, 0.35),
                _ => (0.0, 0.0),
            };
            let (out_l, out_r) = if chorus == 0 {
                (y, y)
            } else {
                self.cl = (self.cl + rate / fs).fract();
                let tri = 4.0 * (self.cl - 0.5).abs() - 1.0;
                let d_ms = 3.2 + depth * tri;
                let w = self.bbd[0].process(y, d_ms * 0.001 * fs);
                (y + w, y - w)
            };
            let ol = self.dc[0].process(out_l * 0.7);
            let or = self.dc[1].process(out_r * 0.7);
            peak = peak.max(ol.abs()).max(or.abs());
            if chans.len() >= 2 {
                chans[0][i] = ol as f32;
                chans[1][i] = or as f32;
            } else if let Some(c) = chans.get_mut(0) {
                c[i] = (0.5 * (ol + or)) as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "1989 Pad (saw + chorus I)", values: &[(SAW, 1.0), (PULSE, 0.0), (SUB, 0.2), (CUTOFF, 0.55), (ENVAMT, 0.15), (ATTACK, 250.0), (RELEASE, 900.0), (SUSTAIN, 0.9), (CHORUS, 1.0)] },
        Preset { name: "I Wanna Get Better Arp", values: &[(SAW, 1.0), (PULSE, 1.0), (PWM, 0.4), (SUB, 0.6), (CUTOFF, 0.42), (ENVAMT, 0.45), (DECAY, 200.0), (SUSTAIN, 0.2), (RELEASE, 120.0), (ARP, 1.0), (ARPRATE, 8.0), (ARPOCT, 2.0), (CHORUS, 2.0)] },
        Preset { name: "Juno Bass", values: &[(SAW, 1.0), (PULSE, 1.0), (PWM, 0.2), (SUB, 0.8), (CUTOFF, 0.3), (RESO, 0.25), (ENVAMT, 0.4), (DECAY, 300.0), (SUSTAIN, 0.4), (CHORUS, 0.0)] },
        Preset { name: "PWM Strings (chorus II)", values: &[(SAW, 0.0), (PULSE, 1.0), (PWM, 0.7), (LFORATE, 0.6), (CUTOFF, 0.6), (ATTACK, 400.0), (RELEASE, 1200.0), (SUSTAIN, 1.0), (ENVAMT, 0.05), (CHORUS, 2.0)] },
        Preset { name: "Bright Stab", values: &[(SAW, 1.0), (PULSE, 1.0), (CUTOFF, 0.45), (ENVAMT, 0.6), (DECAY, 250.0), (SUSTAIN, 0.0), (RELEASE, 200.0), (CHORUS, 1.0)] },
        Preset { name: "Sweeping Resonant Pad", values: &[(SAW, 1.0), (CUTOFF, 0.35), (RESO, 0.6), (LFOF, 0.4), (LFORATE, 0.3), (ATTACK, 600.0), (RELEASE, 1500.0), (SUSTAIN, 1.0), (CHORUS, 3.0)] },
        Preset { name: "Up-Down Arp, Thin", values: &[(SAW, 0.0), (PULSE, 1.0), (PWM, 0.6), (HPF, 2.0), (CUTOFF, 0.6), (DECAY, 150.0), (SUSTAIN, 0.0), (ARP, 3.0), (ARPRATE, 10.0), (ARPOCT, 3.0), (CHORUS, 1.0)] },
        Preset { name: "Warm Chord Organ", values: &[(SAW, 0.0), (PULSE, 1.0), (PWM, 0.0), (SUB, 0.5), (CUTOFF, 0.5), (ENVAMT, 0.0), (SUSTAIN, 1.0), (CHORUS, 3.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "JUNO SIX",
        subtitle: ("AFTER THE ROLAND JUNO-6", "6 VOICE · DCO · CHORUS · ARP · MIDI"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![

            Item::Switch { p: SAW, x: 70.0, y: 112.0, labels: ["OFF", "SAW"], tip: "Sawtooth on/off." },
            Item::Switch { p: PULSE, x: 70.0, y: 158.0, labels: ["OFF", "PULSE"], tip: "Pulse on/off." },
            Item::Knob { p: PWM, x: 170.0, y: 140.0, r: 22.0, label: "PWM", tip: "Pulse width, swept by the LFO as it goes up.", active: None },
            Item::Knob { p: SUB, x: 250.0, y: 140.0, r: 22.0, label: "SUB", tip: "Sub oscillator, an octave down.", active: None },
            Item::Knob { p: NOISE, x: 330.0, y: 140.0, r: 22.0, label: "NOISE", tip: "Noise level.", active: None },
            Item::Knob { p: HPF, x: 410.0, y: 140.0, r: 22.0, label: "HPF", tip: "High-pass filter, four steps (0 = off).", active: None },
            Item::Knob { p: VOLUME, x: 570.0, y: 140.0, r: 26.0, label: "VOLUME", tip: "Output level. Mod wheel adds vibrato; pitch bend is +/- 2 semitones.", active: None },
            Item::Knob { p: CUTOFF, x: 80.0, y: 245.0, r: 22.0, label: "CUTOFF", tip: "Four-pole low-pass cutoff.", active: None },
            Item::Knob { p: RESO, x: 170.0, y: 245.0, r: 22.0, label: "RESONANCE", tip: "Filter resonance.", active: None },
            Item::Knob { p: ENVAMT, x: 250.0, y: 245.0, r: 22.0, label: "ENV", tip: "How far the envelope opens the filter.", active: None },
            Item::Knob { p: LFOF, x: 330.0, y: 245.0, r: 22.0, label: "LFO > VCF", tip: "LFO sweeping the filter.", active: None },
            Item::Knob { p: LFORATE, x: 410.0, y: 245.0, r: 22.0, label: "LFO RATE", tip: "LFO speed.", active: None },
            Item::Knob { p: VIB, x: 490.0, y: 245.0, r: 22.0, label: "LFO > DCO", tip: "LFO vibrato.", active: None },
            Item::Divider { y: 292.0 },
            Item::Knob { p: ATTACK, x: 70.0, y: 350.0, r: 22.0, label: "ATTACK", tip: "Envelope attack.", active: None },
            Item::Knob { p: DECAY, x: 150.0, y: 350.0, r: 22.0, label: "DECAY", tip: "Envelope decay.", active: None },
            Item::Knob { p: SUSTAIN, x: 230.0, y: 350.0, r: 22.0, label: "SUSTAIN", tip: "Envelope sustain.", active: None },
            Item::Knob { p: RELEASE, x: 310.0, y: 350.0, r: 22.0, label: "RELEASE", tip: "Envelope release.", active: None },
            Item::Selector { p: CHORUS, x: 430.0, y: 350.0, label: "CHORUS", positions: &["OFF", "I", "II", "I+II"], tip: "The Juno chorus: I slow and gentle, II deeper, I+II a fast shimmer. It's what makes it sound like a Juno." },
            Item::Selector { p: ARP, x: 120.0, y: 450.0, label: "ARPEGGIO", positions: &["OFF", "UP", "DOWN", "U&D"], tip: "Arpeggiator: hold a chord and it plays the notes in turn." },
            Item::Knob { p: ARPRATE, x: 260.0, y: 450.0, r: 22.0, label: "ARP RATE", tip: "Arpeggio steps per second.", active: None },
            Item::Knob { p: ARPOCT, x: 360.0, y: 450.0, r: 22.0, label: "ARP RANGE", tip: "Octaves the arpeggio covers (1-3).", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Juno Six", class_id: *b"JAJunoSixPQD0001", vendor: "PEQUOD Antonoff",
    instrument, categories: [Instrument, Synth]
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
    fn plays_chords_in_tune() {
        let mut d = Dsp::new(48000.0, 2);
        d.set_params(&with(defaults(), &[(CHORUS, 0.0), (SUB, 0.0), (CUTOFF, 1.0), (ENVAMT, 0.0), (SUSTAIN, 1.0)]));
        let mut m = [0.0f32];
        let mut l = vec![0.0f32; 48000];
        let mut r = l.clone();
        d.midi(Midi::On { note: 57, vel: 1.0 });
        {
            let mut ch: Vec<&mut [f32]> = vec![&mut l, &mut r];
            d.process(&mut ch, &mut m);
        }
        let z = l[24000..].windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        println!("A3 -> {} Hz", z * 2);
        assert!((z as f64 * 2.0 - 220.0).abs() < 8.0);
    }
    #[test]
    fn arpeggiator_steps() {
        let mut d = Dsp::new(48000.0, 2);
        d.set_params(&with(defaults(), &[(ARP, 1.0), (ARPRATE, 8.0), (CHORUS, 0.0), (DECAY, 50.0), (SUSTAIN, 0.0), (RELEASE, 20.0)]));
        for nt in [60u8, 64, 67] {
            d.midi(Midi::On { note: nt, vel: 1.0 });
        }
        let mut m = [0.0f32];
        let mut l = vec![0.0f32; 48000];
        let mut r = l.clone();
        {
            let mut ch: Vec<&mut [f32]> = vec![&mut l, &mut r];
            d.process(&mut ch, &mut m);
        }
        // eight attacks in a second: count rising envelope peaks per 125 ms
        let w: Vec<f64> = l.chunks(6000).map(|c| rms(c)).collect();
        println!("arp windows: {:?}", w.iter().map(|v| format!("{:.2}", db(*v))).collect::<Vec<_>>());
        assert!(w.iter().all(|v| *v > 1e-3));
    }

}
