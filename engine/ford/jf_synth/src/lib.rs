//! Twenty-Six: a monophonic synthesiser in the spirit of the ARP 2600, James Ford's desert-island synth. Three oscillators, noise and ring modulator into a four-pole low-pass filter, ADSR, glide and a spring reverb. Not a circuit model of the 2600: a playable instrument with its layout and character.

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
pub const H: u32 = 560;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Wave { #[name = "Saw"] Saw, #[name = "Square"] Square, #[name = "Triangle"] Tri, #[name = "Sine"] Sine }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "v1wave"]
    pub v1_wave: EnumParam<Wave>,
    #[id = "v1oct"]
    pub v1_oct: IntParam,
    #[id = "v1lvl"]
    pub v1_lvl: FloatParam,
    #[id = "v2wave"]
    pub v2_wave: EnumParam<Wave>,
    #[id = "v2tune"]
    pub v2_tune: FloatParam,
    #[id = "v2lvl"]
    pub v2_lvl: FloatParam,
    #[id = "v3tune"]
    pub v3_tune: FloatParam,
    #[id = "v3lvl"]
    pub v3_lvl: FloatParam,
    #[id = "noise"]
    pub noise: FloatParam,
    #[id = "ring"]
    pub ring: FloatParam,
    #[id = "cutoff"]
    pub cutoff: FloatParam,
    #[id = "reso"]
    pub reso: FloatParam,
    #[id = "envamt"]
    pub env_amt: FloatParam,
    #[id = "keytrk"]
    pub keytrk: FloatParam,
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "sustain"]
    pub sustain: FloatParam,
    #[id = "release"]
    pub release: FloatParam,
    #[id = "glide"]
    pub glide: FloatParam,
    #[id = "reverb"]
    pub reverb: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            v1_wave: EnumParam::new("VCO 1 Wave", Wave::Saw),
            v1_oct: IntParam::new("VCO 1 Octave", 0, IntRange::Linear { min: -2, max: 2 }),
            v1_lvl: pct("VCO 1 Level", 0.8),
            v2_wave: EnumParam::new("VCO 2 Wave", Wave::Square),
            v2_tune: FloatParam::new("VCO 2 Tune", 0.08, FloatRange::Linear { min: -24.0, max: 24.0 }).with_step_size(0.01).with_unit(" st"),
            v2_lvl: pct("VCO 2 Level", 0.5),
            v3_tune: FloatParam::new("VCO 3 Tune", -12.0, FloatRange::Linear { min: -24.0, max: 24.0 }).with_step_size(0.01).with_unit(" st"),
            v3_lvl: pct("VCO 3 Level", 0.0),
            noise: pct("Noise", 0.0),
            ring: pct("Ring Mod", 0.0),
            cutoff: hz("Cutoff", 1200.0, 40.0, 16000.0),
            reso: pct("Resonance", 0.2),
            env_amt: FloatParam::new("Env Amount", 3.0, FloatRange::Linear { min: -5.0, max: 7.0 }).with_step_size(0.01).with_unit(" oct"),
            keytrk: pct("Key Track", 0.5),
            attack: ms("Attack", 5.0, 1.0, 3000.0),
            decay: ms("Decay", 300.0, 5.0, 5000.0),
            sustain: pct("Sustain", 0.6),
            release: ms("Release", 200.0, 5.0, 6000.0),
            glide: ms("Glide", 0.0, 0.0, 2000.0),
            reverb: pct("Spring Reverb", 0.15),
            volume: db("Volume", -6.0, -30.0, 6.0),
        }
    }
}

pub const V1WAVE: usize = 0;
pub const V1OCT: usize = 1;
pub const V1LVL: usize = 2;
pub const V2WAVE: usize = 3;
pub const V2TUNE: usize = 4;
pub const V2LVL: usize = 5;
pub const V3TUNE: usize = 6;
pub const V3LVL: usize = 7;
pub const NOISE: usize = 8;
pub const RING: usize = 9;
pub const CUTOFF: usize = 10;
pub const RESO: usize = 11;
pub const ENVAMT: usize = 12;
pub const KEYTRK: usize = 13;
pub const ATTACK: usize = 14;
pub const DECAY: usize = 15;
pub const SUSTAIN: usize = 16;
pub const RELEASE: usize = 17;
pub const GLIDE: usize = 18;
pub const REVERB: usize = 19;
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
            self.v1_wave.as_ptr(),
            self.v1_oct.as_ptr(),
            self.v1_lvl.as_ptr(),
            self.v2_wave.as_ptr(),
            self.v2_tune.as_ptr(),
            self.v2_lvl.as_ptr(),
            self.v3_tune.as_ptr(),
            self.v3_lvl.as_ptr(),
            self.noise.as_ptr(),
            self.ring.as_ptr(),
            self.cutoff.as_ptr(),
            self.reso.as_ptr(),
            self.env_amt.as_ptr(),
            self.keytrk.as_ptr(),
            self.attack.as_ptr(),
            self.decay.as_ptr(),
            self.sustain.as_ptr(),
            self.release.as_ptr(),
            self.glide.as_ptr(),
            self.reverb.as_ptr(),
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

#[derive(Clone, Copy, Default)]
struct Osc {
    ph: f64,
    tri: f64,
}

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

impl Osc {
    #[inline]
    fn next(&mut self, f: f64, fs: f64, wave: usize) -> f64 {
        let dt = (f / fs).min(0.45);
        self.ph += dt;
        if self.ph >= 1.0 {
            self.ph -= 1.0;
        }
        let t = self.ph;
        match wave {
            0 => 2.0 * t - 1.0 - blep(t, dt),
            1 | 2 => {
                let mut sq = if t < 0.5 { 1.0 } else { -1.0 };
                sq += blep(t, dt);
                sq -= blep((t + 0.5).fract(), dt);
                if wave == 1 {
                    sq
                } else {
                    // integrated square = triangle
                    self.tri = self.tri * 0.9995 + sq * 4.0 * dt;
                    self.tri
                }
            }
            _ => (2.0 * std::f64::consts::PI * t).sin(),
        }
    }
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    osc: [Osc; 3],
    noise: Noise,
    stack: Vec<u8>,
    vel: f64,
    freq: f64,
    target: f64,
    gate: bool,
    env: f64,
    stage: u8,
    ladder: [f64; 4],
    bend: f64,
    modw: f64,
    vib: Lfo,
    spring: [DelayLine; 2],
    sp_ap: [vintage::verb::Diffuser; 3],
    sp_lp: OnePole,
    sp_fb: f64,
    hp: [Biquad; 2],
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut sp_lp = OnePole::default();
        sp_lp.set_lowpass(fs, 4000.0);
        let mut hp = [Biquad::identity(); 2];
        for h in &mut hp {
            h.set_highpass(fs, 20.0, 0.7);
        }
        Self {
            fs,
            v,
            osc: [Osc::default(); 3],
            noise: Noise::new(9),
            stack: Vec::with_capacity(128),
            vel: 1.0,
            freq: 220.0,
            target: 220.0,
            gate: false,
            env: 0.0,
            stage: 0,
            ladder: [0.0; 4],
            bend: 0.0,
            modw: 0.0,
            vib: Lfo::default(),
            spring: [DelayLine::new((fs * 0.05) as usize), DelayLine::new((fs * 0.05) as usize)],
            sp_ap: [vintage::verb::Diffuser::new(fs, 1.3, 0.6), vintage::verb::Diffuser::new(fs, 2.1, 0.6), vintage::verb::Diffuser::new(fs, 3.7, 0.6)],
            sp_lp,
            sp_fb: 0.0,
            hp,
        }
    }
    fn reset(&mut self) {
        self.stack.clear();
        self.gate = false;
        self.env = 0.0;
        self.stage = 0;
        self.ladder = [0.0; 4];
        self.spring.iter_mut().for_each(|s| s.reset());
        self.sp_ap.iter_mut().for_each(|s| s.reset());
        self.sp_fb = 0.0;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn midi(&mut self, ev: Midi) {
        let hz = |n: u8| 440.0 * 2f64.powf((n as f64 - 69.0) / 12.0);
        match ev {
            Midi::On { note, vel } => {
                self.stack.retain(|n| *n != note);
                if self.stack.len() < self.stack.capacity() {
                    self.stack.push(note);
                }
                let legato = self.gate;
                self.target = hz(note);
                if !legato || self.v[GLIDE] < 0.5 {
                    if self.v[GLIDE] < 0.5 {
                        self.freq = self.target;
                    }
                }
                self.vel = 0.4 + 0.6 * vel as f64;
                if !legato {
                    self.stage = 1;
                }
                self.gate = true;
            }
            Midi::Off { note } => {
                self.stack.retain(|n| *n != note);
                if let Some(n) = self.stack.last() {
                    self.target = hz(*n);
                } else {
                    self.gate = false;
                    self.stage = 4;
                }
            }
            Midi::Bend { value } => self.bend = value as f64 * 2.0,
            Midi::Cc { cc: 1, value } => self.modw = value as f64,
            Midi::AllOff => {
                self.stack.clear();
                self.gate = false;
                self.stage = 4;
            }
            _ => {}
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let p = |i: usize| self.v[i] as f64;
        let (w1, o1, l1) = (p(V1WAVE).round() as usize, p(V1OCT), p(V1LVL));
        let (w2, t2, l2) = (p(V2WAVE).round() as usize, p(V2TUNE), p(V2LVL));
        let (t3, l3, nz, ring) = (p(V3TUNE), p(V3LVL), p(NOISE), p(RING));
        let (cut, reso, envamt, kt) = (p(CUTOFF), p(RESO), p(ENVAMT), p(KEYTRK));
        let (a, d, s, r) = (p(ATTACK), p(DECAY), p(SUSTAIN), p(RELEASE));
        let (glide, rev, vol) = (p(GLIDE), p(REVERB), db_to_gain(p(VOLUME)));
        let ca = 1.0 - coeff(fs, a / 2.0);
        let cd = 1.0 - coeff(fs, d / 4.0);
        let cr = 1.0 - coeff(fs, r / 4.0);
        let cg = if glide > 0.5 { 1.0 - coeff(fs, glide / 3.0) } else { 1.0 };
        let k = 4.2 * reso;
        let mut peak = 0.0f64;
        let sp_len = [33.7 * 0.001 * fs, 41.3 * 0.001 * fs];
        for i in 0..n {
            self.freq += (self.target - self.freq) * cg;
            let vib = 1.0 + self.modw * 0.01 * self.vib.next(fs, 5.5);
            let f = self.freq * 2f64.powf(self.bend / 12.0) * vib;
            let f1 = f * 2f64.powf(o1);
            let f2 = f * 2f64.powf(t2 / 12.0);
            let f3 = f * 2f64.powf(t3 / 12.0);
            let s1 = self.osc[0].next(f1, fs, w1);
            let s2 = self.osc[1].next(f2, fs, w2);
            let s3 = self.osc[2].next(f3, fs, 0);
            let mix = s1 * l1 + s2 * l2 + s3 * l3 + self.noise.next() * nz + s1 * s2 * ring;
            // envelope (ADSR, exponential segments)
            match self.stage {
                1 => {
                    self.env += (1.3 - self.env) * ca;
                    if self.env >= 1.0 {
                        self.env = 1.0;
                        self.stage = 2;
                    }
                }
                2 => {
                    self.env += (s - self.env) * cd;
                }
                4 => {
                    self.env += (0.0 - self.env) * cr;
                    if self.env < 1e-5 {
                        self.env = 0.0;
                        self.stage = 0;
                    }
                }
                _ => {}
            }
            // 4-pole ladder with saturating feedback
            let key = (f / 261.6).log2() * kt;
            let fc = (cut * 2f64.powf(envamt * self.env + key)).clamp(20.0, fs * 0.42);
            let g = 1.0 - (-2.0 * std::f64::consts::PI * fc / fs).exp();
            let x = (mix * 0.5 - k * self.ladder[3]).tanh();
            self.ladder[0] += g * (x - self.ladder[0]);
            self.ladder[1] += g * (self.ladder[0] - self.ladder[1]);
            self.ladder[2] += g * (self.ladder[1] - self.ladder[2]);
            self.ladder[3] += g * (self.ladder[2] - self.ladder[3]);
            let y = self.ladder[3] * (1.0 + 0.6 * reso) * self.env * self.vel * vol * 2.0;
            // spring reverb (two springs, chirpy allpasses)
            let mut sp = y + self.sp_fb * 0.7;
            for ap in self.sp_ap.iter_mut() {
                sp = ap.process(sp);
            }
            let a0 = self.spring[0].read(sp_len[0]);
            let b0 = self.spring[1].read(sp_len[1]);
            self.spring[0].write(sp);
            self.spring[1].write(sp);
            self.sp_fb = self.sp_lp.lp(0.5 * (a0 + b0));
            let wet = [a0, b0];
            for (ci, c) in chans.iter_mut().enumerate() {
                let z = self.hp[ci.min(1)].process(y + rev * wet[ci.min(1)] * 0.8);
                peak = peak.max(z.abs());
                c[i] = z as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Fat Bass", values: &[(V1WAVE, 0.0), (V1OCT, -1.0), (V2WAVE, 1.0), (V2TUNE, -12.05), (V2LVL, 0.6), (CUTOFF, 300.0), (RESO, 0.25), (ENVAMT, 3.5), (DECAY, 250.0), (SUSTAIN, 0.3), (REVERB, 0.0)] },
        Preset { name: "Squelch Lead", values: &[(CUTOFF, 500.0), (RESO, 0.7), (ENVAMT, 4.5), (DECAY, 400.0), (SUSTAIN, 0.4), (GLIDE, 80.0)] },
        Preset { name: "Ring Mod Bells", values: &[(V1WAVE, 3.0), (V2WAVE, 3.0), (V2TUNE, 7.3), (V1LVL, 0.2), (V2LVL, 0.2), (RING, 0.9), (CUTOFF, 6000.0), (ENVAMT, 0.0), (ATTACK, 2.0), (DECAY, 1500.0), (SUSTAIN, 0.0), (RELEASE, 1500.0), (REVERB, 0.35)] },
        Preset { name: "Spring Pad", values: &[(V2TUNE, 0.12), (ATTACK, 800.0), (RELEASE, 2000.0), (CUTOFF, 1500.0), (ENVAMT, 1.0), (REVERB, 0.5)] },
        Preset { name: "Sci-Fi Sweep", values: &[(NOISE, 0.3), (RESO, 0.85), (CUTOFF, 200.0), (ENVAMT, 6.0), (ATTACK, 1500.0), (DECAY, 2000.0), (SUSTAIN, 0.0), (REVERB, 0.4)] },
        Preset { name: "Gliding Mono Line", values: &[(GLIDE, 250.0), (V2TUNE, 7.0), (V2LVL, 0.4), (CUTOFF, 1800.0), (ENVAMT, 2.0)] },
        Preset { name: "Noise Snare", values: &[(V1LVL, 0.0), (V2LVL, 0.0), (NOISE, 1.0), (CUTOFF, 3000.0), (ENVAMT, 2.0), (ATTACK, 1.0), (DECAY, 150.0), (SUSTAIN, 0.0), (RELEASE, 150.0), (REVERB, 0.3)] },
        Preset { name: "Three-VCO Brass", values: &[(V3LVL, 0.5), (V3TUNE, -12.0), (V2TUNE, 0.15), (ATTACK, 60.0), (CUTOFF, 700.0), (ENVAMT, 3.0), (SUSTAIN, 0.7)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "TWENTY-SIX",
        subtitle: ("IN THE SPIRIT OF THE ARP 2600", "3 VCO · 4-POLE VCF · SPRING · MIDI"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![

            Item::Selector { p: V1WAVE, x: 82.0, y: 130.0, label: "VCO 1", positions: &["SAW", "SQR", "TRI", "SIN"], tip: "VCO 1 waveform." },
            Item::Knob { p: V1OCT, x: 165.0, y: 130.0, r: 22.0, label: "OCTAVE", tip: "VCO 1 octave.", active: None },
            Item::Knob { p: V1LVL, x: 235.0, y: 130.0, r: 22.0, label: "LEVEL", tip: "VCO 1 level into the mixer.", active: None },
            Item::Selector { p: V2WAVE, x: 345.0, y: 130.0, label: "VCO 2", positions: &["SAW", "SQR", "TRI", "SIN"], tip: "VCO 2 waveform." },
            Item::Knob { p: V2TUNE, x: 440.0, y: 130.0, r: 22.0, label: "TUNE", tip: "VCO 2 tuning (semitones). A few hundredths detune it against VCO 1.", active: None },
            Item::Knob { p: V2LVL, x: 510.0, y: 130.0, r: 22.0, label: "LEVEL", tip: "VCO 2 level.", active: None },
            Item::Knob { p: VOLUME, x: 590.0, y: 130.0, r: 24.0, label: "VOLUME", tip: "Output level. Mod wheel adds vibrato; pitch bend is +/- 2 semitones.", active: None },
            Item::Knob { p: V3TUNE, x: 70.0, y: 225.0, r: 22.0, label: "VCO 3 TUNE", tip: "VCO 3 tuning (saw).", active: None },
            Item::Knob { p: V3LVL, x: 155.0, y: 225.0, r: 22.0, label: "VCO 3 LEVEL", tip: "VCO 3 level.", active: None },
            Item::Knob { p: NOISE, x: 240.0, y: 225.0, r: 22.0, label: "NOISE", tip: "Noise level.", active: None },
            Item::Knob { p: RING, x: 325.0, y: 225.0, r: 22.0, label: "RING MOD", tip: "Ring modulator: VCO 1 x VCO 2.", active: None },
            Item::Knob { p: GLIDE, x: 410.0, y: 225.0, r: 22.0, label: "GLIDE", tip: "Portamento between notes.", active: None },
            Item::Knob { p: REVERB, x: 495.0, y: 225.0, r: 22.0, label: "REVERB", tip: "The built-in spring reverb.", active: None },
            Item::Divider { y: 278.0 },
            Item::Knob { p: CUTOFF, x: 90.0, y: 335.0, r: 26.0, label: "CUTOFF", tip: "Four-pole low-pass cutoff.", active: None },
            Item::Knob { p: RESO, x: 210.0, y: 335.0, r: 24.0, label: "RESONANCE", tip: "Filter resonance; near the top it starts to whistle.", active: None },
            Item::Knob { p: ENVAMT, x: 320.0, y: 335.0, r: 24.0, label: "ENV AMT", tip: "How far the envelope opens (or closes) the filter, in octaves.", active: None },
            Item::Knob { p: KEYTRK, x: 430.0, y: 335.0, r: 24.0, label: "KEY TRACK", tip: "Filter follows the keyboard.", active: None },
            Item::Knob { p: ATTACK, x: 120.0, y: 452.0, r: 22.0, label: "ATTACK", tip: "Envelope attack.", active: None },
            Item::Knob { p: DECAY, x: 240.0, y: 452.0, r: 22.0, label: "DECAY", tip: "Envelope decay.", active: None },
            Item::Knob { p: SUSTAIN, x: 360.0, y: 452.0, r: 22.0, label: "SUSTAIN", tip: "Envelope sustain.", active: None },
            Item::Knob { p: RELEASE, x: 480.0, y: 452.0, r: 22.0, label: "RELEASE", tip: "Envelope release.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Twenty-Six", class_id: *b"JFTwentySixPQD01", vendor: "PEQUOD James Ford",
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
    fn plays_the_right_pitch_and_releases() {
        let mut d = Dsp::new(48000.0, 2);
        let p = with(defaults(), &[(V2LVL, 0.0), (CUTOFF, 16000.0), (ENVAMT, 0.0), (REVERB, 0.0), (SUSTAIN, 1.0)]);
        d.set_params(&p);
        let mut m = [0.0f32];
        let mut l = vec![0.0f32; 48000];
        let mut r = l.clone();
        d.midi(Midi::On { note: 57, vel: 1.0 });
        {
            let mut ch: Vec<&mut [f32]> = vec![&mut l, &mut r];
            d.process(&mut ch, &mut m);
        }
        let z = l[24000..].windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        let f = z as f64 * 2.0;
        println!("A3 -> {f:.0} Hz");
        assert!((f - 220.0).abs() < 6.0);
        d.midi(Midi::Off { note: 57 });
        let mut l2 = vec![0.0f32; 48000];
        let mut r2 = l2.clone();
        {
            let mut ch: Vec<&mut [f32]> = vec![&mut l2, &mut r2];
            d.process(&mut ch, &mut m);
        }
        assert!(rms(&l2[40000..]) < 1e-3);
    }

}
