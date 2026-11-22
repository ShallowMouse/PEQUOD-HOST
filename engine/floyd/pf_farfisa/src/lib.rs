//! Compact Organ: the Farfisa Compact Duo, Rick Wright's combo organ from Piper to Meddle (Astronomy Domine, Interstellar
//! Overdrive, Echoes): transistor top-octave oscillators divided down, the voice tabs' filters, vibrato and tone. Played from MIDI.

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
pub enum Vibrato {
    #[name = "Off"]
    Off,
    #[name = "Light"]
    Light,
    #[name = "Heavy"]
    Heavy,
}

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum Speed {
    #[name = "Slow"]
    Slow,
    #[name = "Fast"]
    Fast,
}

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum Tone {
    #[name = "Mellow"]
    Mellow,
    #[name = "Bright"]
    Bright,
}

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
    #[id = "bass16"]
    pub bass16: EnumParam<OffOn>,
    #[id = "flute16"]
    pub flute16: EnumParam<OffOn>,
    #[id = "flute8"]
    pub flute8: EnumParam<OffOn>,
    #[id = "oboe8"]
    pub oboe8: EnumParam<OffOn>,
    #[id = "trumpet8"]
    pub trumpet8: EnumParam<OffOn>,
    #[id = "strings8"]
    pub strings8: EnumParam<OffOn>,
    #[id = "flute4"]
    pub flute4: EnumParam<OffOn>,
    #[id = "strings4"]
    pub strings4: EnumParam<OffOn>,
    #[id = "vibrato"]
    pub vibrato: EnumParam<Vibrato>,
    #[id = "speed"]
    pub speed: EnumParam<Speed>,
    #[id = "tone"]
    pub tone: EnumParam<Tone>,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            bass16: EnumParam::new("Bass 16'", OffOn::Off),
            flute16: EnumParam::new("Flute 16'", OffOn::Off),
            flute8: EnumParam::new("Flute 8'", OffOn::On),
            oboe8: EnumParam::new("Oboe 8'", OffOn::Off),
            trumpet8: EnumParam::new("Trumpet 8'", OffOn::Off),
            strings8: EnumParam::new("Strings 8'", OffOn::On),
            flute4: EnumParam::new("Flute 4'", OffOn::On),
            strings4: EnumParam::new("Strings 4'", OffOn::Off),
            vibrato: EnumParam::new("Vibrato", Vibrato::Off),
            speed: EnumParam::new("Vibrato Speed", Speed::Fast),
            tone: EnumParam::new("Tone", Tone::Bright),
            volume: db("Volume", 0.0, -24.0, 6.0),
        }
    }
}

pub const BASS16: usize = 0;
pub const FLUTE16: usize = 1;
pub const FLUTE8: usize = 2;
pub const OBOE8: usize = 3;
pub const TRUMPET8: usize = 4;
pub const STRINGS8: usize = 5;
pub const FLUTE4: usize = 6;
pub const STRINGS4: usize = 7;
pub const VIBRATO: usize = 8;
pub const SPEED: usize = 9;
pub const TONE: usize = 10;
pub const VOLUME: usize = 11;
pub const NPARAMS: usize = 12;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.bass16.as_ptr(),
            self.flute16.as_ptr(),
            self.flute8.as_ptr(),
            self.oboe8.as_ptr(),
            self.trumpet8.as_ptr(),
            self.strings8.as_ptr(),
            self.flute4.as_ptr(),
            self.strings4.as_ptr(),
            self.vibrato.as_ptr(),
            self.speed.as_ptr(),
            self.tone.as_ptr(),
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

/// The organ itself (keys/kb_farfisa), driven through this plugin's own parameters.
#[allow(dead_code, clippy::all)]
mod tpl {
    use super::*;
pub const FL16: usize = 0;
pub const FL8: usize = 1;
pub const FL4: usize = 2;
pub const STR8: usize = 3;
pub const STR4: usize = 4;
pub const OBOE: usize = 5;
pub const TRUMPET: usize = 6;
pub const VIB: usize = 7;
pub const VIBFAST: usize = 8;
pub const VIBHEAVY: usize = 9;
pub const VOLUME: usize = 10;
pub const NPARAMS: usize = 11;

    pub fn defaults() -> Vec<f32> {
        vec![0.0; NPARAMS]
    }


/// A band-limited square at phase `ph` (cycles), increment `dt`.
#[inline]
fn square(ph: f64, dt: f64) -> f64 {
    let blep = |t: f64| {
        if t < dt {
            let x = t / dt;
            x + x - x * x - 1.0
        } else if t > 1.0 - dt {
            let x = (t - 1.0) / dt;
            x * x + x + x + 1.0
        } else {
            0.0
        }
    };
    let mut q = if ph < 0.5 { 1.0 } else { -1.0 };
    q += blep(ph);
    q -= blep((ph + 0.5).fract());
    q
}

#[derive(Clone, Copy, Default)]
struct Key {
    on: bool,
    f: f64,
    gain: f64,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    keys: [Key; 128],
    held: [bool; 128],
    vib_ph: f64,
    bend_acc: f64,
    flute: [Biquad; 2],
    strings: [Biquad; 2],
    oboe: [Biquad; 2],
    trumpet: [Biquad; 2],
    dc: DcBlock,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        // the voice filters: fixed, on the buses, as in the organ
        let mut flute = [Biquad::identity(); 2];
        flute[0].set_lowpass(fs, 900.0, 0.7);
        flute[1].set_lowpass(fs, 1400.0, 0.6);
        let mut strings = [Biquad::identity(); 2];
        strings[0].set_highpass(fs, 600.0, 0.7);
        strings[1].set_peak(fs, 3200.0, 0.9, 5.0);
        let mut oboe = [Biquad::identity(); 2];
        oboe[0].set_bandpass(fs, 1300.0, 1.6);
        oboe[1].set_peak(fs, 2600.0, 2.0, 4.0);
        let mut trumpet = [Biquad::identity(); 2];
        trumpet[0].set_peak(fs, 1700.0, 1.2, 9.0);
        trumpet[1].set_lowpass(fs, 6000.0, 0.7);
        Self { fs, v, keys: [Key::default(); 128], held: [false; 128], vib_ph: 0.0, bend_acc: 0.0, flute, strings, oboe, trumpet, dc: DcBlock::new(fs) }
    }
    fn reset(&mut self) {
        self.keys = [Key::default(); 128];
        self.held = [false; 128];
        for b in self.flute.iter_mut().chain(self.strings.iter_mut()).chain(self.oboe.iter_mut()).chain(self.trumpet.iter_mut()) {
            b.reset();
        }
        self.dc.reset();
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn midi(&mut self, ev: Midi) {
        match ev {
            Midi::On { note, .. } => {
                // 49 keys, C2 to C6; outside them, folded in
                let mut n = note as i32;
                while n < 36 {
                    n += 12;
                }
                while n > 84 {
                    n -= 12;
                }
                let k = &mut self.keys[note as usize & 127];
                k.on = true;
                k.f = 440.0 * 2f64.powf((n as f64 - 69.0) / 12.0);
                self.held[note as usize & 127] = true;
            }
            Midi::Off { note } => self.held[note as usize & 127] = false,
            Midi::AllOff => self.held = [false; 128],
            _ => {}
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let p = self.v;
        let on = |i: usize| if p[i] >= 0.5 { 1.0 } else { 0.0 };
        let (f16, f8, f4, s8, s4, ob, tr) = (on(FL16), on(FL8), on(FL4), on(STR8), on(STR4), on(OBOE), on(TRUMPET));
        let vib = p[VIB] >= 0.5;
        let vrate = if p[VIBFAST] >= 0.5 { 7.8 } else { 5.6 };
        let vdepth = if p[VIBHEAVY] >= 0.5 { 32.0 } else { 14.0 };
        let vol = db_to_gain(p[VOLUME] as f64);
        // the key contacts: on in a couple of milliseconds, off as fast
        let key_c = coeff(fs, 1.5);
        let mut peak = 0.0f64;
        for k in 0..n {
            // vibrato bends the top-octave oscillators, so every key moves together; the phase is
            // the integral of the bent frequency (a running time that runs a little fast and slow)
            self.vib_ph = (self.vib_ph + vrate / fs).fract();
            let bend = if vib { 2f64.powf(vdepth / 1200.0 * (std::f64::consts::TAU * self.vib_ph).sin()) } else { 1.0 };
            self.bend_acc += bend / fs;
            let t = self.bend_acc;
            let (mut b16, mut b8, mut b4) = (0.0, 0.0, 0.0);
            for (i, key) in self.keys.iter_mut().enumerate() {
                if !key.on {
                    continue;
                }
                let target = if self.held[i] { 1.0 } else { 0.0 };
                key.gain += key_c * (target - key.gain);
                if !self.held[i] && key.gain < 1e-4 {
                    key.on = false;
                    continue;
                }
                let g = key.gain;
                let f = key.f;
                let dt = f * bend / fs;
                b8 += g * square((f * t).fract(), dt.min(0.45));
                if f16 > 0.0 {
                    b16 += g * square((0.5 * f * t).fract(), (0.5 * dt).min(0.45));
                }
                if f4 + s4 > 0.0 {
                    b4 += g * square((2.0 * f * t).fract(), (2.0 * dt).min(0.45));
                }
            }
            let mut x = 0.0;
            if f16 + f8 + f4 > 0.0 {
                x += 1.4 * cascade(&mut self.flute, f16 * b16 + f8 * b8 + f4 * 0.8 * b4);
            }
            if s8 + s4 > 0.0 {
                x += 0.9 * cascade(&mut self.strings, s8 * b8 + s4 * 0.8 * b4);
            }
            if ob > 0.0 {
                x += 1.6 * cascade(&mut self.oboe, b8);
            }
            if tr > 0.0 {
                x += 0.7 * cascade(&mut self.trumpet, b8);
            }
            // the preamp: a little transistor grit
            let y = self.dc.process((x * 0.18).tanh() * 0.9) * vol;
            peak = peak.max(y.abs());
            for c in chans.iter_mut() {
                c[k] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


}

fn map(v: &[f32]) -> [f32; tpl::NPARAMS] {
    let mut t = [0.0f32; tpl::NPARAMS];
    t[tpl::FL16] = v[FLUTE16].max(v[BASS16]);
    t[tpl::FL8] = v[FLUTE8];
    t[tpl::FL4] = v[FLUTE4];
    t[tpl::STR8] = v[STRINGS8];
    t[tpl::STR4] = v[STRINGS4];
    t[tpl::OBOE] = v[OBOE8];
    t[tpl::TRUMPET] = v[TRUMPET8];
    let vib = v[VIBRATO].round();
    t[tpl::VIB] = if vib > 0.5 { 1.0 } else { 0.0 };
    t[tpl::VIBHEAVY] = if vib > 1.5 { 1.0 } else { 0.0 };
    t[tpl::VIBFAST] = v[SPEED];
    // MELLOW takes the edge off (the tone tab cuts the strings and reeds back)
    t[tpl::VOLUME] = v[VOLUME] - 1.5 - if v[TONE] < 0.5 { 2.0 } else { 0.0 };
    if v[TONE] < 0.5 {
        t[tpl::STR4] = 0.0;
    }
    t
}

pub struct Dsp {
    inner: tpl::Dsp,
}

impl VintageDsp for Dsp {
    const METERS: usize = <tpl::Dsp as VintageDsp>::METERS;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let mut inner = tpl::Dsp::new(sample_rate, ch);
        inner.set_params(&map(&defaults()));
        Self { inner }
    }
    fn reset(&mut self) {
        self.inner.reset();
    }
    fn latency(&self) -> u32 {
        self.inner.latency()
    }
    fn set_params(&mut self, v: &[f32]) {
        self.inner.set_params(&map(v));
    }
    fn midi(&mut self, ev: Midi) {
        self.inner.midi(ev);
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        self.inner.process(chans, meters);
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Astronomy Domine", values: &[(VIBRATO, 1.0)] },
        Preset { name: "Interstellar Reed", values: &[(FLUTE8, 0.0), (OBOE8, 1.0), (TRUMPET8, 1.0), (FLUTE4, 0.0)] },
        Preset { name: "Echoes Sustained Tone", values: &[(FLUTE16, 1.0), (STRINGS8, 0.0), (VIBRATO, 1.0), (SPEED, 0.0), (TONE, 0.0)] },
        Preset { name: "Full Buzz (all tabs)", values: &[(BASS16, 1.0), (FLUTE16, 1.0), (OBOE8, 1.0), (TRUMPET8, 1.0), (STRINGS4, 1.0)] },
        Preset { name: "Mellow Flutes", values: &[(STRINGS8, 0.0), (VIBRATO, 1.0), (TONE, 0.0)] },
        Preset { name: "Heavy Vibrato Strings", values: &[(FLUTE8, 0.0), (FLUTE4, 0.0), (STRINGS4, 1.0), (VIBRATO, 2.0)] },
    ]
}

/*
('divider', 238)
('selector?', 119.5, 319.5, 36, 36)
('knob', 519.5, 319.5, 25.6)

crate: ['floyd/pf_farfisa', 'library/panic_abort']
Fno launcher worked, at least one error/root/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/open-5.4.4/src/lib.rs
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
OFFLIGHTHEAVY
VIBRATOVibrato depth.SPEEDSLOWFASTVibrato speed.TONEMELLOWBRIGHTOverall tone.VOLUMEOutput level (the swell pedal).AFTER THE FARFISA COMPACT DUOTRANSISTOR COMBO ORGAN  
  MIDICOMPACT ORGANBASS 16'FLUTE 16'FLUTE 8'OBOE 8'TRUMPET 8'STRINGS 8'FLUTE 4'STRINGS 4'floyd/pf_farfisa/src/lib.rs
ONVoice tab: stack them like the rocker tabs on the organ.
Astronomy Domine
Interstellar Reed
Echoes Sustained Tone
Full Buzz (all tabs)
Mellow Flutes
Heavy Vibrato Strings

*/
fn panel() -> Panel {
    Panel {
        title: "COMPACT ORGAN",
        subtitle: ("AFTER THE FARFISA COMPACT DUO", "TRANSISTOR COMBO ORGAN  ·  MIDI"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::Text { x: 95.0, y: 94.0, text: "BASS 16'", size: 12.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: BASS16, x: 95.0, y: 120.0, labels: ["OFF", "ON"], tip: "Bass 16': a deep flute an octave down." },
            Item::Text { x: 245.0, y: 94.0, text: "FLUTE 16'", size: 12.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: FLUTE16, x: 245.0, y: 120.0, labels: ["OFF", "ON"], tip: "Flute 16'." },
            Item::Text { x: 395.0, y: 94.0, text: "FLUTE 8'", size: 12.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: FLUTE8, x: 395.0, y: 120.0, labels: ["OFF", "ON"], tip: "Flute 8'." },
            Item::Text { x: 545.0, y: 94.0, text: "OBOE 8'", size: 12.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: OBOE8, x: 545.0, y: 120.0, labels: ["OFF", "ON"], tip: "Oboe 8'." },
            Item::Text { x: 95.0, y: 170.0, text: "TRUMPET 8'", size: 12.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: TRUMPET8, x: 95.0, y: 196.0, labels: ["OFF", "ON"], tip: "Trumpet 8'." },
            Item::Text { x: 245.0, y: 170.0, text: "STRINGS 8'", size: 12.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: STRINGS8, x: 245.0, y: 196.0, labels: ["OFF", "ON"], tip: "Strings 8'." },
            Item::Text { x: 395.0, y: 170.0, text: "FLUTE 4'", size: 12.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: FLUTE4, x: 395.0, y: 196.0, labels: ["OFF", "ON"], tip: "Flute 4'." },
            Item::Text { x: 545.0, y: 170.0, text: "STRINGS 4'", size: 12.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: STRINGS4, x: 545.0, y: 196.0, labels: ["OFF", "ON"], tip: "Strings 4'." },
            Item::Divider { y: 238.0 },
            Item::Selector { p: VIBRATO, x: 120.0, y: 320.0, label: "VIBRATO", positions: &["OFF", "LIGHT", "HEAVY"], tip: "Vibrato depth." },
            Item::Text { x: 260.0, y: 280.0, text: "SPEED", size: 13.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: SPEED, x: 260.0, y: 321.0, labels: ["SLOW", "FAST"], tip: "Vibrato speed." },
            Item::Text { x: 380.0, y: 280.0, text: "TONE", size: 13.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: TONE, x: 380.0, y: 321.0, labels: ["MELLOW", "BRIGHT"], tip: "Overall tone." },
            Item::Knob { p: VOLUME, x: 520.0, y: 320.0, r: 26.0, label: "VOLUME", tip: "Output level (the swell pedal).", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Compact Organ", class_id: *b"PFCompactOrgPQD1", vendor: "PEQUOD Pink Floyd",
    instrument, categories: [Instrument, Synth]
);
