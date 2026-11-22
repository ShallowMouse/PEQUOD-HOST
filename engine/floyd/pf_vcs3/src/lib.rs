//! VCS3: the EMS VCS3 / Synthi AKS, Pink Floyd's synth from Dark Side (On the Run, Any Colour You Like) and Wish You Were
//! Here: three oscillators (the third as a modulator), noise, ring modulator, the diode filter, the trapezoid envelope and
//! an 8-step sequencer, played from MIDI.

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
pub const H: u32 = 520;


/// On/off switches are stored as 0/1 (as the released plugin saved them).
#[derive(Enum, PartialEq, Clone, Copy)]
pub enum OffOn {
    #[name = "Off"]
    Off,
    #[name = "On"]
    On,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "o1oct"]
    pub o1oct: FloatParam,
    #[id = "o1shape"]
    pub o1shape: FloatParam,
    #[id = "o1lvl"]
    pub o1lvl: FloatParam,
    #[id = "o2tune"]
    pub o2tune: FloatParam,
    #[id = "o2shape"]
    pub o2shape: FloatParam,
    #[id = "o2lvl"]
    pub o2lvl: FloatParam,
    #[id = "o3rate"]
    pub o3rate: FloatParam,
    #[id = "o3pitch"]
    pub o3pitch: FloatParam,
    #[id = "o3filter"]
    pub o3filter: FloatParam,
    #[id = "noise"]
    pub noise: FloatParam,
    #[id = "ring"]
    pub ring: FloatParam,
    #[id = "cutoff"]
    pub cutoff: FloatParam,
    #[id = "res"]
    pub res: FloatParam,
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "envfilter"]
    pub envfilter: FloatParam,
    #[id = "glide"]
    pub glide: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
    #[id = "seq"]
    pub seq: EnumParam<OffOn>,
    #[id = "seqrate"]
    pub seqrate: FloatParam,
    #[id = "s1"]
    pub s1: FloatParam,
    #[id = "s2"]
    pub s2: FloatParam,
    #[id = "s3"]
    pub s3: FloatParam,
    #[id = "s4"]
    pub s4: FloatParam,
    #[id = "s5"]
    pub s5: FloatParam,
    #[id = "s6"]
    pub s6: FloatParam,
    #[id = "s7"]
    pub s7: FloatParam,
    #[id = "s8"]
    pub s8: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            o1oct: FloatParam::new("Osc 1 Octave", 0.0, FloatRange::Linear { min: -2.0, max: 2.0 }).with_step_size(1.0),  // texts '-2' .. '0' .. '2'
            o1shape: pct("Osc 1 Sine-Saw", 0.7),
            o1lvl: pct("Osc 1 Level", 0.8),
            o2tune: FloatParam::new("Osc 2 Tune", 7.05, FloatRange::Linear { min: -24.0, max: 24.0 }).with_unit(" st").with_value_to_string(formatters::v2s_f32_rounded(2)),  // texts '-24.00' .. '0.00' .. '24.00'
            o2shape: pct("Osc 2 Square-Tri", 0.2),
            o2lvl: pct("Osc 2 Level", 0.4),
            o3rate: rate("Osc 3 Rate", 4.0, 0.05, 500.0),
            o3pitch: pct("Osc 3 > Pitch", 0.0),
            o3filter: pct("Osc 3 > Filter", 0.0),
            noise: pct("Noise", 0.0),
            ring: pct("Ring Mod", 0.0),
            cutoff: hz("Filter Frequency", 1800.0, 30.0, 15000.0),
            res: pct("Response", 0.35),
            attack: ms("Attack", 5.0, 1.0, 3000.0),
            decay: ms("Decay", 400.0, 5.0, 6000.0),
            envfilter: pct("Envelope > Filter", 0.5),
            glide: ms("Glide", 1.0, 1.0, 2000.0),
            output: db("Output", -6.0, -30.0, 6.0),
            seq: EnumParam::new("Sequencer", OffOn::Off),
            seqrate: FloatParam::new("Sequencer Rate", 165.0, FloatRange::Linear { min: 40.0, max: 300.0 }).with_step_size(1.0).with_unit(" bpm"),  // texts '40' .. '170' .. '300'
            s1: FloatParam::new("Step 1", 0.0, FloatRange::Linear { min: -12.0, max: 12.0 }).with_step_size(1.0).with_unit(" st"),  // texts '-12' .. '0' .. '12'
            s2: FloatParam::new("Step 2", 3.0, FloatRange::Linear { min: -12.0, max: 12.0 }).with_step_size(1.0).with_unit(" st"),  // texts '-12' .. '0' .. '12'
            s3: FloatParam::new("Step 3", 5.0, FloatRange::Linear { min: -12.0, max: 12.0 }).with_step_size(1.0).with_unit(" st"),  // texts '-12' .. '0' .. '12'
            s4: FloatParam::new("Step 4", 3.0, FloatRange::Linear { min: -12.0, max: 12.0 }).with_step_size(1.0).with_unit(" st"),  // texts '-12' .. '0' .. '12'
            s5: FloatParam::new("Step 5", 10.0, FloatRange::Linear { min: -12.0, max: 12.0 }).with_step_size(1.0).with_unit(" st"),  // texts '-12' .. '0' .. '12'
            s6: FloatParam::new("Step 6", 8.0, FloatRange::Linear { min: -12.0, max: 12.0 }).with_step_size(1.0).with_unit(" st"),  // texts '-12' .. '0' .. '12'
            s7: FloatParam::new("Step 7", 10.0, FloatRange::Linear { min: -12.0, max: 12.0 }).with_step_size(1.0).with_unit(" st"),  // texts '-12' .. '0' .. '12'
            s8: FloatParam::new("Step 8", 12.0, FloatRange::Linear { min: -12.0, max: 12.0 }).with_step_size(1.0).with_unit(" st"),  // texts '-12' .. '0' .. '12'
        }
    }
}

pub const O1OCT: usize = 0;
pub const O1SHAPE: usize = 1;
pub const O1LVL: usize = 2;
pub const O2TUNE: usize = 3;
pub const O2SHAPE: usize = 4;
pub const O2LVL: usize = 5;
pub const O3RATE: usize = 6;
pub const O3PITCH: usize = 7;
pub const O3FILTER: usize = 8;
pub const NOISE: usize = 9;
pub const RING: usize = 10;
pub const CUTOFF: usize = 11;
pub const RES: usize = 12;
pub const ATTACK: usize = 13;
pub const DECAY: usize = 14;
pub const ENVFILTER: usize = 15;
pub const GLIDE: usize = 16;
pub const OUTPUT: usize = 17;
pub const SEQ: usize = 18;
pub const SEQRATE: usize = 19;
pub const S1: usize = 20;
pub const S2: usize = 21;
pub const S3: usize = 22;
pub const S4: usize = 23;
pub const S5: usize = 24;
pub const S6: usize = 25;
pub const S7: usize = 26;
pub const S8: usize = 27;
pub const NPARAMS: usize = 28;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.o1oct.as_ptr(),
            self.o1shape.as_ptr(),
            self.o1lvl.as_ptr(),
            self.o2tune.as_ptr(),
            self.o2shape.as_ptr(),
            self.o2lvl.as_ptr(),
            self.o3rate.as_ptr(),
            self.o3pitch.as_ptr(),
            self.o3filter.as_ptr(),
            self.noise.as_ptr(),
            self.ring.as_ptr(),
            self.cutoff.as_ptr(),
            self.res.as_ptr(),
            self.attack.as_ptr(),
            self.decay.as_ptr(),
            self.envfilter.as_ptr(),
            self.glide.as_ptr(),
            self.output.as_ptr(),
            self.seq.as_ptr(),
            self.seqrate.as_ptr(),
            self.s1.as_ptr(),
            self.s2.as_ptr(),
            self.s3.as_ptr(),
            self.s4.as_ptr(),
            self.s5.as_ptr(),
            self.s6.as_ptr(),
            self.s7.as_ptr(),
            self.s8.as_ptr(),
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
    held: Vec<u8>,
    note: f64,
    pitch: f64,
    gate: bool,
    env: f64,
    p1: f64,
    p2: f64,
    p3: f64,
    lp: [f64; 4],
    rng: u32,
    step: usize,
    step_t: f64,
    dc: Biquad,
    out: Smooth,
}

impl Dsp {
    fn noise(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f64 / u32::MAX as f64 * 2.0 - 1.0
    }
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

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut dc = Biquad::identity();
        dc.set_highpass(fs, 15.0, 0.7);
        Self { fs, v, held: Vec::new(), note: 60.0, pitch: 60.0, gate: false, env: 0.0, p1: 0.0, p2: 0.0, p3: 0.0, lp: [0.0; 4], rng: 0x9e37_79b9, step: 0, step_t: 0.0, dc, out: Smooth::new(fs, 20.0, 0.5) }
    }
    fn reset(&mut self) {
        self.held.clear();
        self.gate = false;
        self.env = 0.0;
        self.lp = [0.0; 4];
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn midi(&mut self, ev: Midi) {
        match ev {
            Midi::On { note, .. } => {
                let legato = !self.held.is_empty();
                self.held.retain(|&n| n != note);
                self.held.push(note);
                self.note = note as f64;
                if !legato {
                    self.pitch = self.note;
                    self.step = 0;
                    self.step_t = 0.0;
                }
                self.gate = true;
            }
            Midi::Off { note } => {
                self.held.retain(|&n| n != note);
                match self.held.last() {
                    Some(&n) => self.note = n as f64,
                    None => self.gate = false,
                }
            }
            Midi::AllOff => {
                self.held.clear();
                self.gate = false;
            }
            _ => {}
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let v = self.v;
        let seq = v[SEQ] > 0.5;
        let step_len = 60.0 / v[SEQRATE].max(1.0) as f64 / 4.0;
        let cg = coeff(fs, v[GLIDE] as f64);
        let att = 1.0 / (v[ATTACK] as f64 * 0.001 * fs).max(1.0);
        let dec = coeff(fs, v[DECAY] as f64 / 3.0);
        let (s1, l1, s2, l2) = (v[O1SHAPE] as f64, v[O1LVL] as f64, v[O2SHAPE] as f64, v[O2LVL] as f64);
        let res = v[RES] as f64 * 4.2;
        let mut peak = 0.0f64;
        for k in 0..n {
            // the sequencer: steps from the held key, each retriggering the trapezoid
            let mut gate = self.gate;
            let mut target = self.note;
            if seq && self.gate {
                self.step_t += 1.0 / fs;
                if self.step_t >= step_len {
                    self.step_t -= step_len;
                    self.step = (self.step + 1) % 8;
                }
                target = self.note + v[S1 + self.step] as f64;
                gate = self.step_t < step_len * 0.6;
            }
            self.pitch = target + cg * (self.pitch - target);
            // the trapezoid: linear rise, hold while the key is down, then decay
            if gate {
                self.env = (self.env + att).min(1.0);
            } else {
                self.env *= dec;
            }
            // oscillator 3: a triangle modulator
            self.p3 = (self.p3 + v[O3RATE] as f64 / fs).fract();
            let o3 = 1.0 - 4.0 * (self.p3 - 0.5).abs();
            let semis = self.pitch + 12.0 * v[O1OCT] as f64 + 12.0 * v[O3PITCH] as f64 * o3 - 69.0;
            let f1 = 440.0 * 2f64.powf(semis / 12.0);
            let f2 = f1 * 2f64.powf(v[O2TUNE] as f64 / 12.0);
            let (d1, d2) = ((f1 / fs).min(0.45), (f2 / fs).min(0.45));
            self.p1 = (self.p1 + d1).fract();
            self.p2 = (self.p2 + d2).fract();
            let sine = (std::f64::consts::TAU * self.p1).sin();
            let saw = 2.0 * self.p1 - 1.0 - blep(self.p1, d1);
            let a = sine * (1.0 - s1) + saw * s1;
            let mut sq = if self.p2 < 0.5 { 1.0 } else { -1.0 };
            sq += blep(self.p2, d2) - blep((self.p2 + 0.5).fract(), d2);
            let tri = 1.0 - 4.0 * (self.p2 - 0.5).abs();
            let b = sq * (1.0 - s2) + tri * s2;
            let nz = self.noise();
            let x = a * l1 + b * l2 + nz * v[NOISE] as f64 * 0.7 + a * b * v[RING] as f64;
            // the diode filter: four poles with tanh stages and resonance feedback
            let oct = 5.0 * v[ENVFILTER] as f64 * self.env + 3.0 * v[O3FILTER] as f64 * o3;
            let fc = (v[CUTOFF] as f64 * 2f64.powf(oct)).min(fs * 0.45);
            let g = 1.0 - (-std::f64::consts::TAU * fc / fs).exp();
            let fb = res * self.lp[3];
            let mut u = (x * 0.5 - fb).tanh();
            for p in self.lp.iter_mut() {
                *p += g * (u - *p);
                u = p.tanh();
            }
            let y = self.dc.process(self.lp[3] * 1.5) * self.env * self.out.next();
            peak = peak.max(self.env);
            for c in chans.iter_mut() {
                c[k] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Sequence (Synthi-style)", values: &[(NOISE, 0.1), (CUTOFF, 900.0), (RES, 0.6), (ATTACK, 2.0), (DECAY, 90.0), (ENVFILTER, 0.7), (SEQ, 1.0)] },
        Preset { name: "Machine Drone", values: &[(O1SHAPE, 1.0), (O2TUNE, -12.0), (O2LVL, 0.6), (O3RATE, 0.15), (O3FILTER, 0.6), (CUTOFF, 300.0), (RES, 0.7), (ATTACK, 900.0), (DECAY, 2500.0)] },
        Preset { name: "Whoosh (noise sweep)", values: &[(O1LVL, 0.0), (O2LVL, 0.0), (O3RATE, 0.25), (O3FILTER, 1.0), (NOISE, 1.0), (CUTOFF, 400.0), (RES, 0.8), (ATTACK, 400.0), (DECAY, 3000.0)] },
        Preset { name: "Ring Mod Bells", values: &[(O1SHAPE, 0.0), (O1LVL, 0.2), (O2TUNE, 13.7), (O2LVL, 0.0), (RING, 0.9), (CUTOFF, 6000.0), (RES, 0.2), (DECAY, 1500.0)] },
        Preset { name: "Lead with Vibrato", values: &[(O3RATE, 5.5), (O3PITCH, 0.03), (CUTOFF, 2500.0), (RES, 0.4), (GLIDE, 80.0)] },
        Preset { name: "Self-Oscillating Filter", values: &[(O1LVL, 0.0), (O2LVL, 0.0), (CUTOFF, 800.0), (RES, 1.0), (ATTACK, 50.0), (DECAY, 1500.0), (ENVFILTER, 0.0)] },
        Preset { name: "Fat Bass", values: &[(O1OCT, -1.0), (O1SHAPE, 1.0), (O2TUNE, -11.95), (O2LVL, 0.7), (CUTOFF, 400.0), (RES, 0.3), (DECAY, 250.0), (ENVFILTER, 0.6)] },
    ]
}

/*
('knob', 45.5, 115.5, 17.1)
('knob', 123.5, 115.5, 17.1)
('knob', 201.5, 115.5, 17.1)
('knob', 279.5, 115.5, 17.1)
('knob', 357.5, 115.5, 17.1)
('knob', 435.5, 115.5, 17.1)
('knob', 513.5, 115.5, 17.1)
('knob', 591.5, 115.5, 17.1)
('knob', 45.5, 209.5, 17.1)
('knob', 123.5, 209.5, 17.1)
('knob', 201.5, 209.5, 17.1)
('knob', 279.5, 209.5, 17.1)
('knob', 357.5, 209.5, 17.1)
('knob', 435.5, 209.5, 17.1)
('knob', 513.5, 209.5, 17.1)
('knob', 591.5, 209.5, 17.1)
('divider', 262)
('knob', 45.5, 315.5, 17.1)
('knob', 357.5, 315.5, 17.1)
('knob', 591.5, 315.5, 17.1)
('divider', 362)

crate: ['8floyd/pf_vcs3', 'floyd/pf_vcs3', 'library/panic_abort']
central_panel
vintage/src/ui.rs
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
 stOSC 1 OCTOscillator 1 octave.SINE - SAWOscillator 1 waveform: sine to sawtooth.OSC 1 LEVELOscillator 1 into the filter.OSC 2 TUNEOscillator 2 relative to oscillator 1 (semitones).SQU - TRIOscillator 2 waveform: square to triangle.OSC 2 LEVELOscillator 2 into the filter.OSC 3 RATEOscillator 3: a triangle modulator from very slow to audio rate.O3 > PITCHOscillator 3 modulating the pitch (up to an octave).O3 > FILTOscillator 3 sweeping the filter.NOISENoise generator into the filter.RING MODRing modulator: oscillator 1 x oscillator 2.FREQUENCYFilter frequency.RESPONSEFilter resonance; at the top it whistles on its own.ATTACKTrapezoid attack.DECAYTrapezoid decay after the key (or step) is released.ENV > FILTHow far the trapezoid opens the filter.GLIDEPortamento between held notes.SEQUENCEROFFONHold a key: the 8-step pattern below plays, transposed to that key, retriggering the envelope each step.SEQ RATESequencer tempo (16th notes).OUTPUTOutput level.ENVAFTER THE EMS VCS3 / SYNTHI AKS3 OSCILLATORS  
  DIODE FILTER  
  SEQUENCER  
  MIDIVCS3STEP 1STEP 2STEP 3STEP 4STEP 5STEP 6STEP 7STEP 8floyd/pf_vcs3/src/lib.rs
Sequencer step: semitones from the held key.
333?
Sequence (Synthi-style)
333?
Machine Drone

*/
fn panel() -> Panel {
    Panel {
        title: "VCS3",
        subtitle: ("AFTER THE EMS VCS3 / SYNTHI AKS", "3 OSCILLATORS  ·  DIODE FILTER  ·  SEQUENCER  ·  MIDI"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::Knob { p: O1OCT, x: 46.0, y: 116.0, r: 18.0, label: "OSC 1 OCT", tip: "Oscillator 1 octave.", active: None },
            Item::Knob { p: O1SHAPE, x: 124.0, y: 116.0, r: 18.0, label: "SINE - SAW", tip: "Oscillator 1 waveform: sine to sawtooth.", active: None },
            Item::Knob { p: O1LVL, x: 202.0, y: 116.0, r: 18.0, label: "OSC 1 LEVEL", tip: "Oscillator 1 into the filter.", active: None },
            Item::Knob { p: O2TUNE, x: 280.0, y: 116.0, r: 18.0, label: "OSC 2 TUNE", tip: "Oscillator 2 relative to oscillator 1 (semitones).", active: None },
            Item::Knob { p: O2SHAPE, x: 358.0, y: 116.0, r: 18.0, label: "SQU - TRI", tip: "Oscillator 2 waveform: square to triangle.", active: None },
            Item::Knob { p: O2LVL, x: 436.0, y: 116.0, r: 18.0, label: "OSC 2 LEVEL", tip: "Oscillator 2 into the filter.", active: None },
            Item::Knob { p: O3RATE, x: 514.0, y: 116.0, r: 18.0, label: "OSC 3 RATE", tip: "Oscillator 3: a triangle modulator from very slow to audio rate.", active: None },
            Item::Knob { p: O3PITCH, x: 592.0, y: 116.0, r: 18.0, label: "O3 > PITCH", tip: "Oscillator 3 modulating the pitch (up to an octave).", active: None },
            Item::Knob { p: O3FILTER, x: 46.0, y: 210.0, r: 18.0, label: "O3 > FILT", tip: "Oscillator 3 sweeping the filter.", active: None },
            Item::Knob { p: NOISE, x: 124.0, y: 210.0, r: 18.0, label: "NOISE", tip: "Noise generator into the filter.", active: None },
            Item::Knob { p: RING, x: 202.0, y: 210.0, r: 18.0, label: "RING MOD", tip: "Ring modulator: oscillator 1 x oscillator 2.", active: None },
            Item::Knob { p: CUTOFF, x: 280.0, y: 210.0, r: 18.0, label: "FREQUENCY", tip: "Filter frequency.", active: None },
            Item::Knob { p: RES, x: 358.0, y: 210.0, r: 18.0, label: "RESPONSE", tip: "Filter resonance; at the top it whistles on its own.", active: None },
            Item::Knob { p: ATTACK, x: 436.0, y: 210.0, r: 18.0, label: "ATTACK", tip: "Trapezoid attack.", active: None },
            Item::Knob { p: DECAY, x: 514.0, y: 210.0, r: 18.0, label: "DECAY", tip: "Trapezoid decay after the key (or step) is released.", active: None },
            Item::Knob { p: ENVFILTER, x: 592.0, y: 210.0, r: 18.0, label: "ENV > FILT", tip: "How far the trapezoid opens the filter.", active: None },
            Item::Divider { y: 262.0 },
            Item::Knob { p: GLIDE, x: 46.0, y: 316.0, r: 18.0, label: "GLIDE", tip: "Portamento between held notes.", active: None },
            Item::Text { x: 201.0, y: 290.0, text: "SEQUENCER", size: 13.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: SEQ, x: 201.0, y: 326.0, labels: ["OFF", "ON"], tip: "Hold a key: the 8-step pattern below plays, transposed to that key, retriggering the envelope each step." },
            Item::Knob { p: SEQRATE, x: 358.0, y: 316.0, r: 18.0, label: "SEQ RATE", tip: "Sequencer tempo (16th notes).", active: None },
            Item::Lamp { m: 0, x: 513.0, y: 316.0, label: "ENV", lo: 0.05, hi: 0.9 },
            Item::Knob { p: OUTPUT, x: 592.0, y: 316.0, r: 18.0, label: "OUTPUT", tip: "Output level.", active: None },
            Item::Divider { y: 362.0 },
            Item::Knob { p: S1 + 0, x: 46.0, y: 410.0, r: 18.0, label: "STEP 1", tip: "Sequencer step: semitones from the held key.", active: Some(ActiveWhen { p: SEQ, min: 0.5, max: 1.0 }) },
            Item::Knob { p: S1 + 1, x: 124.0, y: 410.0, r: 18.0, label: "STEP 2", tip: "Sequencer step: semitones from the held key.", active: Some(ActiveWhen { p: SEQ, min: 0.5, max: 1.0 }) },
            Item::Knob { p: S1 + 2, x: 202.0, y: 410.0, r: 18.0, label: "STEP 3", tip: "Sequencer step: semitones from the held key.", active: Some(ActiveWhen { p: SEQ, min: 0.5, max: 1.0 }) },
            Item::Knob { p: S1 + 3, x: 280.0, y: 410.0, r: 18.0, label: "STEP 4", tip: "Sequencer step: semitones from the held key.", active: Some(ActiveWhen { p: SEQ, min: 0.5, max: 1.0 }) },
            Item::Knob { p: S1 + 4, x: 358.0, y: 410.0, r: 18.0, label: "STEP 5", tip: "Sequencer step: semitones from the held key.", active: Some(ActiveWhen { p: SEQ, min: 0.5, max: 1.0 }) },
            Item::Knob { p: S1 + 5, x: 436.0, y: 410.0, r: 18.0, label: "STEP 6", tip: "Sequencer step: semitones from the held key.", active: Some(ActiveWhen { p: SEQ, min: 0.5, max: 1.0 }) },
            Item::Knob { p: S1 + 6, x: 514.0, y: 410.0, r: 18.0, label: "STEP 7", tip: "Sequencer step: semitones from the held key.", active: Some(ActiveWhen { p: SEQ, min: 0.5, max: 1.0 }) },
            Item::Knob { p: S1 + 7, x: 592.0, y: 410.0, r: 18.0, label: "STEP 8", tip: "Sequencer step: semitones from the held key.", active: Some(ActiveWhen { p: SEQ, min: 0.5, max: 1.0 }) },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "VCS3", class_id: *b"PFVCS3SynthPQD01", vendor: "PEQUOD Pink Floyd",
    instrument, categories: [Instrument, Synth]
);
