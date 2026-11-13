//! Combo Compact: a transistor combo organ in the manner of the Farfisa Compact.
//!
//! Twelve top-octave oscillators divided down to every key, so the whole organ is locked in tune:
//! square waves, keyed on instantly by the key contacts. The voice tabs pick footages and filter
//! them into the Farfisa's voices - FLUTE (soft and round), STRINGS (thin and buzzy), OBOE and
//! TRUMPET (nasal, the 60s garage sound) - and can be combined. VIBRATO wobbles the oscillators
//! themselves, light or heavy, slow or fast. A little transistor grit from the preamp.

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
pub const H: u32 = 380;


fn tab(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 }).with_step_size(1.0)
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "fl16"]
    pub fl16: FloatParam,
    #[id = "fl8"]
    pub fl8: FloatParam,
    #[id = "fl4"]
    pub fl4: FloatParam,
    #[id = "str8"]
    pub str8: FloatParam,
    #[id = "str4"]
    pub str4: FloatParam,
    #[id = "oboe"]
    pub oboe: FloatParam,
    #[id = "trumpet"]
    pub trumpet: FloatParam,
    #[id = "vib"]
    pub vib: FloatParam,
    #[id = "vibfast"]
    pub vibfast: FloatParam,
    #[id = "vibheavy"]
    pub vibheavy: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            fl16: tab("Flute 16'", 0.0),
            fl8: tab("Flute 8'", 1.0),
            fl4: tab("Flute 4'", 0.0),
            str8: tab("Strings 8'", 1.0),
            str4: tab("Strings 4'", 0.0),
            oboe: tab("Oboe 8'", 0.0),
            trumpet: tab("Trumpet 8'", 0.0),
            vib: tab("Vibrato", 1.0),
            vibfast: tab("Vibrato Speed", 0.0),
            vibheavy: tab("Vibrato Depth", 0.0),
            volume: db("Volume", -6.0, -30.0, 6.0),
        }
    }
}

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

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.fl16.as_ptr(),
            self.fl8.as_ptr(),
            self.fl4.as_ptr(),
            self.str8.as_ptr(),
            self.str4.as_ptr(),
            self.oboe.as_ptr(),
            self.trumpet.as_ptr(),
            self.vib.as_ptr(),
            self.vibfast.as_ptr(),
            self.vibheavy.as_ptr(),
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

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Garage Combo", values: &[(FL16, 0.0), (FL8, 1.0), (FL4, 0.0), (STR8, 1.0), (STR4, 0.0), (OBOE, 1.0), (TRUMPET, 0.0), (VIB, 1.0), (VIBFAST, 1.0), (VIBHEAVY, 0.0)] },
        Preset { name: "Soft Flutes", values: &[(FL16, 1.0), (FL8, 1.0), (FL4, 0.0), (STR8, 0.0), (STR4, 0.0), (OBOE, 0.0), (TRUMPET, 0.0), (VIB, 1.0), (VIBFAST, 0.0)] },
        Preset { name: "Buzzy Strings", values: &[(FL16, 0.0), (FL8, 0.0), (FL4, 0.0), (STR8, 1.0), (STR4, 1.0), (OBOE, 0.0), (TRUMPET, 0.0), (VIB, 1.0)] },
        Preset { name: "Nasal Trumpet", values: &[(FL16, 0.0), (FL8, 0.0), (STR8, 0.0), (STR4, 0.0), (OBOE, 0.0), (TRUMPET, 1.0), (VIB, 0.0)] },
        Preset { name: "Everything On", values: &[(FL16, 1.0), (FL8, 1.0), (FL4, 1.0), (STR8, 1.0), (STR4, 1.0), (OBOE, 1.0), (TRUMPET, 1.0), (VIB, 1.0), (VIBHEAVY, 1.0)] },
    ]
}

fn panel() -> Panel {
    let tab = |p, x, y, label, tip| Item::Switch { p, x, y, labels: ["", label], tip };
    Panel {
        title: "COMBO COMPACT",
        subtitle: ("IN THE MANNER OF THE FARFISA COMPACT", "DIVIDE-DOWN SQUARES · VOICE TABS · VIBRATO"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::Text { x: 320.0, y: 92.0, text: "VOICES", size: 11.0, align: Align2::CENTER_CENTER },
            tab(FL16, 100.0, 128.0, "FLUTE 16'", "Flute 16': soft and round, an octave down."),
            tab(FL8, 290.0, 128.0, "FLUTE 8'", "Flute 8': soft and round."),
            tab(FL4, 480.0, 128.0, "FLUTE 4'", "Flute 4': an octave up."),
            tab(STR8, 100.0, 164.0, "STRINGS 8'", "Strings 8': thin and buzzy."),
            tab(STR4, 290.0, 164.0, "STRINGS 4'", "Strings 4': thin and buzzy, an octave up."),
            tab(OBOE, 480.0, 164.0, "OBOE 8'", "Oboe 8': nasal, the reedy one."),
            tab(TRUMPET, 100.0, 200.0, "TRUMPET 8'", "Trumpet 8': bright and brassy."),
            Item::Knob { p: VOLUME, x: 590.0, y: 300.0, r: 24.0, label: "VOLUME", tip: "Output level.", active: None },
            Item::Divider { y: 240.0 },
            tab(VIB, 100.0, 300.0, "VIBRATO", "Vibrato on: the whole organ's pitch wobbles."),
            Item::Switch { p: VIBFAST, x: 290.0, y: 300.0, labels: ["SLOW", "FAST"], tip: "Vibrato speed." },
            Item::Switch { p: VIBHEAVY, x: 450.0, y: 300.0, labels: ["LIGHT", "HEAVY"], tip: "Vibrato depth." },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Combo Compact", class_id: *b"PQKeysComboFarfs", vendor: "PEQUOD Keys",
    instrument, categories: [Instrument, Synth]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        with(defaults(), &[(STR8, 0.0), (VIB, 0.0)])
    }

    include!("../../instrument_tests.rs");
}
