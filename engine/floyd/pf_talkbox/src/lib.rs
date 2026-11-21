//! Talk Box: the Heil Talk Box as David Gilmour used it on Pigs (Three Different Ones): the guitar drives a horn driver,
//! the sound goes up a tube into the mouth, and the mouth's shape (here a formant filter: oo - oh - ah - eh - ee) shapes it.
//! The mouth is moved by hand, by the playing (envelope), by an LFO, or by a voice on the right input.

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
pub enum ModeSel {
    #[name = "Mouth"]
    Mouth,
    #[name = "Envelope"]
    Envelope,
    #[name = "LFO"]
    Lfo,
    #[name = "Voice (right input)"]
    VoiceRightInput,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "mode"]
    pub mode: EnumParam<ModeSel>,
    #[id = "mouth"]
    pub mouth: FloatParam,
    #[id = "sens"]
    pub sens: FloatParam,
    #[id = "rate"]
    pub rate: FloatParam,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            mode: EnumParam::new("Mode", ModeSel::Envelope),
            mouth: pct("Mouth", 0.5),
            sens: pct("Sensitivity", 0.6),
            rate: rate("LFO Rate", 0.8, 0.1, 6.0),
            drive: db("Driver", 6.0, 0.0, 24.0),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const MODE: usize = 0;
pub const MOUTH: usize = 1;
pub const SENS: usize = 2;
pub const RATE: usize = 3;
pub const DRIVE: usize = 4;
pub const OUTPUT: usize = 5;
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
            self.mode.as_ptr(),
            self.mouth.as_ptr(),
            self.sens.as_ptr(),
            self.rate.as_ptr(),
            self.drive.as_ptr(),
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

/// Formants (Hz) of oo, oh, ah, eh, ee, with relative levels (dB) of F2 and F3.
const VOWELS: [[f64; 5]; 5] = [
    [300.0, 870.0, 2240.0, -8.0, -24.0],
    [450.0, 800.0, 2830.0, -4.0, -22.0],
    [730.0, 1090.0, 2440.0, -2.0, -14.0],
    [530.0, 1840.0, 2480.0, -6.0, -12.0],
    [270.0, 2290.0, 3010.0, -12.0, -10.0],
];

fn vowel(m: f64) -> [f64; 5] {
    let x = m.clamp(0.0, 1.0) * 4.0;
    let i = (x.floor() as usize).min(3);
    let t = x - i as f64;
    let mut o = [0.0; 5];
    for k in 0..5 {
        o[k] = VOWELS[i][k] * (1.0 - t) + VOWELS[i + 1][k] * t;
    }
    o
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    horn_hp: Biquad,
    horn_lp: Biquad,
    tube: Biquad,
    f: [Biquad; 3],
    fg: [f64; 3],
    voice_lo: Biquad,
    voice_hi: Biquad,
    env: f64,
    venv: f64,
    genv: f64,
    vlo: f64,
    vhi: f64,
    phase: f64,
    mouth: f64,
    count: usize,
    drive: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self {
            fs,
            v,
            horn_hp: Biquad::identity(),
            horn_lp: Biquad::identity(),
            tube: Biquad::identity(),
            f: [Biquad::identity(); 3],
            fg: [1.0; 3],
            voice_lo: Biquad::identity(),
            voice_hi: Biquad::identity(),
            env: 0.0,
            venv: 0.0,
            genv: 0.0,
            vlo: 0.0,
            vhi: 0.0,
            phase: 0.0,
            mouth: 0.5,
            count: 0,
            drive: Smooth::new(fs, 20.0, 2.0),
            out: Smooth::new(fs, 20.0, 1.0),
        };
        d.horn_hp.set_highpass(fs, 180.0, 0.7);
        d.horn_lp.set_lowpass(fs, 5500.0, 0.7);
        // the tube: a quarter-wave resonance of about a metre and a half of plastic
        d.tube.set_peak(fs, 115.0 * 5.0, 3.0, 3.0);
        d.voice_lo.set_lowpass(fs, 900.0, 0.7);
        d.voice_hi.set_highpass(fs, 1500.0, 0.7);
        d.set_mouth(0.5);
        d
    }
    fn reset(&mut self) {
        for b in [&mut self.horn_hp, &mut self.horn_lp, &mut self.tube, &mut self.voice_lo, &mut self.voice_hi] {
            b.reset();
        }
        for b in &mut self.f {
            b.reset();
        }
        self.env = 0.0;
        self.venv = 0.0;
        self.phase = 0.0;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.drive.set(db_to_gain(v[DRIVE] as f64));
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let mode = self.v[MODE].round() as i32;
        let sens = self.v[SENS] as f64;
        let rate = self.v[RATE] as f64;
        let ca = coeff(self.fs, 8.0);
        let cr = coeff(self.fs, 160.0);
        let cm = coeff(self.fs, 15.0);
        for k in 0..n {
            let voice = mode == 3 && nch > 1;
            let x = if voice { chans[0][k] as f64 } else { chans.iter().map(|c| c[k] as f64).sum::<f64>() / nch.max(1) as f64 };
            // where the mouth is
            let target = match mode {
                0 => self.v[MOUTH] as f64,
                1 => {
                    let a = x.abs();
                    self.env = a + if a > self.env { ca } else { cr } * (self.env - a);
                    (self.env * sens * 8.0).min(1.0)
                }
                2 => {
                    self.phase = (self.phase + rate / self.fs).fract();
                    0.5 - 0.5 * (2.0 * std::f64::consts::PI * self.phase).cos()
                }
                _ => {
                    // the voice: its loudness opens the mouth, its brightness moves the vowel front-to-back
                    let s = if nch > 1 { chans[1][k] as f64 } else { 0.0 };
                    let a = s.abs();
                    self.venv = a + if a > self.venv { ca } else { cr } * (self.venv - a);
                    let lo = self.voice_lo.process(s).abs();
                    let hi = self.voice_hi.process(s).abs();
                    self.vlo = lo + cr * (self.vlo - lo);
                    self.vhi = hi + cr * (self.vhi - hi);
                    let b = self.vhi / (self.vlo + self.vhi + 1e-6);
                    if self.venv > 0.002 { (0.2 + 1.2 * b).min(1.0) } else { 0.0 }
                }
            };
            self.mouth = target + cm * (self.mouth - target);
            self.count += 1;
            if self.count >= 32 {
                self.count = 0;
                let m = self.mouth;
                self.set_mouth(m);
            }
            // the horn driver: bandlimited and gritty when pushed
            let dr = self.drive.next();
            let d = self.horn_lp.process(self.horn_hp.process(x)) * dr;
            let d = (d * 1.5).tanh() / 1.5;
            let t = self.tube.process(d);
            let y = self.f[0].process(t) * self.fg[0] + self.f[1].process(t) * self.fg[1] + self.f[2].process(t) * self.fg[2];
            // in VOICE mode the mouth only sounds while the voice is there
            let gate = if mode == 3 {
                let a = x.abs();
                self.genv = a + if a > self.genv { ca } else { cr } * (self.genv - a);
                (0.6 / (self.genv + 0.02)).min(30.0)
            } else {
                1.0
            };
            let y = y * 1.6 * gate * self.out.next();
            for c in chans.iter_mut() {
                c[k] = y as f32;
            }
        }
        meters[0] = self.mouth as f32;
    }
}

impl Dsp {
    fn set_mouth(&mut self, m: f64) {
        let p = vowel(m);
        let fs = self.fs;
        self.f[0].set_bandpass(fs, p[0], 6.0);
        self.f[1].set_bandpass(fs, p[1], 8.0);
        self.f[2].set_bandpass(fs, p[2], 10.0);
        self.fg = [1.0, db_to_gain(p[3]), db_to_gain(p[4])];
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Pigs Talk Box (voice on right input)", values: &[(MODE, 3.0), (DRIVE, 10.0)] },
        Preset { name: "Wah-Talk (envelope)", values: &[(DRIVE, 8.0)] },
        Preset { name: "Slow Vowel Cycle", values: &[(MODE, 2.0), (RATE, 0.4)] },
        Preset { name: "Mouth Open 'Ah'", values: &[(MODE, 0.0)] },  // OCR 'Mouth Open ‘Ah’'
        Preset { name: "Mouth 'Oo'", values: &[(MODE, 0.0), (MOUTH, 0.0)] },  // OCR 'Mouth ‘Oo’'
    ]
}

/*
('selector?', 89.5, 159.5, 36, 36)
('knob', 359.5, 159.5, 28.0)
('divider', 284)
('knob', 239.5, 335.5, 26.8)
('knob', 399.5, 335.5, 22.0)

crate: ['floyd/pf_talkbox', 'library/panic_abort']
AFTER THE HEIL TALK BOXDRIVER  
  TUBE  
  MOUTHHANDENVLFOVOICE
MOUTH BYHAND: the MOUTH knob (automate it). ENVELOPE: opens with your playing. LFO: a slow cycle. VOICE: guitar on the left input, your voice on the right: the voice shapes the guitar.MOUTHVowel: oo - oh - ah - eh - ee.SENSITIVITYENVELOPE mode: how far the mouth opens for a given pick strength.LFO RATELFO mode: vowel cycle rate.DRIVERHow hard the guitar drives the horn driver (it gets gritty).OUTPUTOutput level.TALK BOX
Pigs Talk Box (voice on right input)
Wah-Talk (envelope)
Slow Vowel Cycle
Mouth Open 'Ah'
Mouth 'Oo'
j?$Dsp
)"8	
!(El

*/
fn panel() -> Panel {
    Panel {
        title: "TALK BOX",
        subtitle: ("AFTER THE HEIL TALK BOX", "DRIVER  ·  TUBE  ·  MOUTH"),
        accent: ui::COBALT,
        size: (W, H),
        items: vec![
            Item::Selector { p: MODE, x: 90.0, y: 160.0, label: "MOUTH BY", positions: &["HAND", "ENV", "LFO", "VOICE"], tip: "HAND: the MOUTH knob (automate it). ENVELOPE: opens with your playing. LFO: a slow cycle. VOICE: guitar on the left input, your voice on the right: the voice shapes the guitar." },
            Item::Knob { p: MOUTH, x: 230.0, y: 160.0, r: 36.0, label: "MOUTH", tip: "Vowel: oo - oh - ah - eh - ee.", active: Some(ActiveWhen { p: MODE, min: 0.0, max: 0.1 }) },
            Item::Knob { p: SENS, x: 360.0, y: 160.0, r: 28.0, label: "SENSITIVITY", tip: "ENVELOPE mode: how far the mouth opens for a given pick strength.", active: Some(ActiveWhen { p: MODE, min: 0.3, max: 0.4 }) },
            Item::Knob { p: RATE, x: 470.0, y: 160.0, r: 28.0, label: "LFO RATE", tip: "LFO mode: vowel cycle rate.", active: Some(ActiveWhen { p: MODE, min: 0.6, max: 0.7 }) },
            Item::Lamp { m: 0, x: 570.0, y: 159.0, label: "MOUTH", lo: 0.0, hi: 1.0 },
            Item::Divider { y: 284.0 },
            Item::Knob { p: DRIVE, x: 240.0, y: 336.0, r: 22.0, label: "DRIVER", tip: "How hard the guitar drives the horn driver (it gets gritty).", active: None },
            Item::Knob { p: OUTPUT, x: 400.0, y: 336.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Talk Box", class_id: *b"PFTalkBoxPQD0001", vendor: "PEQUOD Pink Floyd",
    categories: [Fx, Filter]
);
