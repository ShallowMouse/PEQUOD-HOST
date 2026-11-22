//! Clavioline: the Selmer Clavioline (the Telstar lead): a monophonic valve keyboard. A narrow pulse oscillator through
//! the voice tab's tone filters, vibrato, octave register, legato glide and the knee-lever swell (velocity here).

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

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum Voice {
    #[name = "Flute"]
    Flute,
    #[name = "Oboe"]
    Oboe,
    #[name = "Clarinet"]
    Clarinet,
    #[name = "Trumpet"]
    Trumpet,
    #[name = "Violin"]
    Violin,
    #[name = "Saxophone"]
    Saxophone,
    #[name = "Bassoon"]
    Bassoon,
    #[name = "Musette"]
    Musette,
}

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum Register {
    #[name = "Low"]
    Low,
    #[name = "Middle"]
    Middle,
    #[name = "High"]
    High,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "voice"]
    pub voice: EnumParam<Voice>,
    #[id = "register"]
    pub register: EnumParam<Register>,
    #[id = "vibrato"]
    pub vibrato: FloatParam,
    #[id = "vib_rate"]
    pub vib_rate: FloatParam,
    #[id = "glide"]
    pub glide: FloatParam,
    #[id = "velocity"]
    pub velocity: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            voice: EnumParam::new("Voice", Voice::Oboe),
            register: EnumParam::new("Register", Register::Middle),
            vibrato: pct("Vibrato", 0.45),
            vib_rate: rate("Vibrato Rate", 6.0, 3.0, 9.0),
            glide: ms("Glide", 5.0, 1.0, 300.0),
            velocity: pct("Velocity to Swell", 0.4),
            volume: db("Volume", -6.0, -30.0, 6.0),
        }
    }
}

pub const VOICE: usize = 0;
pub const REGISTER: usize = 1;
pub const VIBRATO: usize = 2;
pub const VIB_RATE: usize = 3;
pub const GLIDE: usize = 4;
pub const VELOCITY: usize = 5;
pub const VOLUME: usize = 6;
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
            self.voice.as_ptr(),
            self.register.as_ptr(),
            self.vibrato.as_ptr(),
            self.vib_rate.as_ptr(),
            self.glide.as_ptr(),
            self.velocity.as_ptr(),
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

/// (pulse width, formant Hz, formant dB, low-pass Hz) for each voice tab
const VOICES: [(f64, f64, f64, f64); 8] = [
    (0.5, 800.0, 0.0, 1200.0),
    (0.15, 1400.0, 8.0, 4000.0),
    (0.5, 1500.0, 3.0, 2500.0),
    (0.2, 1200.0, 6.0, 5000.0),
    (0.1, 2500.0, 6.0, 6000.0),
    (0.3, 900.0, 6.0, 3500.0),
    (0.25, 500.0, 6.0, 1500.0),
    (0.08, 2000.0, 10.0, 7000.0),
];

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

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    held: Vec<(u8, f32)>,
    note: f64,
    pitch: f64,
    vel: f64,
    gate: bool,
    env: f64,
    ph: f64,
    vph: f64,
    modw: f64,
    bend: f64,
    voice: i32,
    fmt: Biquad,
    lp: [Biquad; 2],
    hp: Biquad,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let (_, f, g, lp) = VOICES[(self.voice.clamp(0, 7)) as usize];
        self.fmt.set_peak(self.fs, f, 1.4, g);
        self.lp[0].set_lowpass(self.fs, lp, 0.6);
        self.lp[1].set_lowpass(self.fs, lp * 1.5, 0.6);
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut hp = Biquad::identity();
        hp.set_highpass(fs, 40.0, 0.7);
        let mut d = Self {
            fs,
            v,
            held: Vec::new(),
            note: 60.0,
            pitch: 60.0,
            vel: 1.0,
            gate: false,
            env: 0.0,
            ph: 0.0,
            vph: 0.0,
            modw: 0.0,
            bend: 0.0,
            voice: v[VOICE].round() as i32,
            fmt: Biquad::identity(),
            lp: [Biquad::identity(); 2],
            hp,
            out: Smooth::new(fs, 20.0, 1.0),
        };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.held.clear();
        self.gate = false;
        self.env = 0.0;
        self.fmt.reset();
        self.lp.iter_mut().for_each(|b| b.reset());
        self.hp.reset();
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let vo = v[VOICE].round() as i32;
        if vo != self.voice {
            self.voice = vo;
            self.design();
        }
        self.out.set(db_to_gain(v[VOLUME] as f64));
    }
    fn midi(&mut self, ev: Midi) {
        match ev {
            Midi::On { note, vel } => {
                let legato = !self.held.is_empty();
                self.held.retain(|h| h.0 != note);
                self.held.push((note, vel));
                self.note = note as f64;
                if !legato {
                    self.pitch = self.note;
                }
                self.vel = vel as f64;
                self.gate = true;
            }
            Midi::Off { note } => {
                self.held.retain(|h| h.0 != note);
                if let Some(&(n, _)) = self.held.last() {
                    self.note = n as f64;
                } else {
                    self.gate = false;
                }
            }
            Midi::Cc { cc: 1, value } => self.modw = value as f64,
            Midi::Bend { value } => self.bend = value as f64 * 2.0,
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
        let reg = (self.v[REGISTER].round() as f64 - 1.0) * 12.0;
        let depth = (self.v[VIBRATO] as f64 + self.modw).min(1.5) * 0.6;
        let vrate = self.v[VIB_RATE] as f64;
        let cg = coeff(fs, self.v[GLIDE] as f64);
        let vs = self.v[VELOCITY] as f64;
        let pw = VOICES[(self.voice.clamp(0, 7)) as usize].0;
        let ca = coeff(fs, 12.0);
        let cr = coeff(fs, 70.0);
        let mut peak = 0.0f64;
        for k in 0..n {
            self.pitch = self.note + cg * (self.pitch - self.note);
            self.vph = (self.vph + vrate / fs).fract();
            let vib = depth * (2.0 * std::f64::consts::PI * self.vph).sin();
            let f = 440.0 * 2f64.powf((self.pitch + reg + vib + self.bend - 69.0) / 12.0);
            let dt = (f / fs).min(0.45);
            self.ph += dt;
            if self.ph >= 1.0 {
                self.ph -= 1.0;
            }
            // a band-limited pulse
            let mut p = if self.ph < pw { 1.0 } else { -1.0 };
            p += blep(self.ph, dt);
            p -= blep((self.ph - pw).rem_euclid(1.0), dt);
            let p = p - (2.0 * pw - 1.0);
            let t = if self.gate { 1.0 } else { 0.0 };
            self.env = t + if t > self.env { ca } else { cr } * (self.env - t);
            let lvl = 1.0 - vs + vs * self.vel;
            let x = self.lp[0].process(self.fmt.process(p));
            let x = self.lp[1].process(x);
            // the valve output stage
            let y = self.hp.process(tube(x * 0.32 * self.env * lvl, 0.08)) * self.out.next();
            peak = peak.max(y.abs());
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
        Preset { name: "Telstar Lead", values: &[(REGISTER, 2.0), (VIBRATO, 0.5), (VIB_RATE, 6.2)] },
        Preset { name: "Trumpet", values: &[(VOICE, 3.0), (VIBRATO, 0.3)] },
        Preset { name: "Violin", values: &[(VOICE, 4.0), (REGISTER, 2.0), (VIBRATO, 0.55)] },
        Preset { name: "Mellow Flute", values: &[(VOICE, 0.0), (VIBRATO, 0.35)] },
        Preset { name: "Low Bassoon", values: &[(VOICE, 6.0), (REGISTER, 0.0), (VIBRATO, 0.2)] },
        Preset { name: "Musette (reedy)", values: &[(VOICE, 7.0), (VIBRATO, 0.6), (VIB_RATE, 7.0)] },
    ]
}

/*
('knob', 419.5, 149.5, 28.0)
('knob', 529.5, 149.5, 23.2)
('selector?', 299.5, 150.0, 36, 35)
('divider', 284)
('knob', 219.5, 335.5, 22.0)
('knob', 339.5, 335.5, 22.0)
('knob', 459.5, 335.5, 22.0)

crate: ['library/panic_abort', 'meek/jm_clavioline']
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
AFTER THE SELMER CLAVIOLINEMONOPHONIC VALVE KEYBOARD  
  MIDIFluteOboeClarinetTrumpetViolinSaxophoneBassoonMusette
Voice tab: the tone filters the pulse wave goes through.LOWMIDHIGH
REGISTEROctave register.VIBRATOVibrato depth (the mod wheel adds more).RATEVibrato rate.NOTEGLIDESlide between notes played legato.VEL. SWELLHow much velocity sets the level (the knee-lever swell).VOLUMEOutput level.CLAVIOLINE
Telstar Lead
Mellow Flute
Low Bassoon
Musette (reedy)
meek/jm_clavioline/src/lib.rs
@ffffff
@ffffff

*/
fn panel() -> Panel {
    Panel {
        title: "CLAVIOLINE",
        subtitle: ("AFTER THE SELMER CLAVIOLINE", "MONOPHONIC VALVE KEYBOARD  ·  MIDI"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::RadioList { p: VOICE, x: 30.0, y: 84.0, w: 170.0, row_h: 22.0, names: &["Flute", "Oboe", "Clarinet", "Trumpet", "Violin", "Saxophone", "Bassoon", "Musette"], tip: "Voice tab: the tone filters the pulse wave goes through." },
            Item::Selector { p: REGISTER, x: 300.0, y: 150.0, label: "REGISTER", positions: &["LOW", "MID", "HIGH"], tip: "Octave register." },
            Item::Knob { p: VIBRATO, x: 420.0, y: 150.0, r: 30.0, label: "VIBRATO", tip: "Vibrato depth (the mod wheel adds more).", active: None },
            Item::Knob { p: VIB_RATE, x: 530.0, y: 150.0, r: 26.0, label: "RATE", tip: "Vibrato rate.", active: None },
            Item::Lamp { m: 0, x: 420.0, y: 236.0, label: "NOTE", lo: 0.01, hi: 0.3 },
            Item::Divider { y: 284.0 },
            Item::Knob { p: GLIDE, x: 220.0, y: 336.0, r: 22.0, label: "GLIDE", tip: "Slide between notes played legato.", active: None },
            Item::Knob { p: VELOCITY, x: 340.0, y: 336.0, r: 22.0, label: "VEL. SWELL", tip: "How much velocity sets the level (the knee-lever swell).", active: None },
            Item::Knob { p: VOLUME, x: 460.0, y: 336.0, r: 22.0, label: "VOLUME", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Clavioline", class_id: *b"JMClaviolinePQD1", vendor: "PEQUOD Joe Meek",
    instrument, categories: [Instrument, Synth]
);
