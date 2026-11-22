//! TR-606: the Roland TR-606 Drumatix's analog voices played from MIDI (Big Black's drummer): bass drum, snare, two toms,
//! cymbal and hi-hats (closed and open, from the shared metallic source), with accent, tune, decay and an overdriven output.
//! MIDI: 36 BD, 38/40 SD, 41/43/45 low tom, 47/48/50 high tom, 49/51/57 cymbal, 42/44 closed hat, 46 open hat.

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


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "accent"]
    pub accent: FloatParam,
    #[id = "bd"]
    pub bd: FloatParam,
    #[id = "sd"]
    pub sd: FloatParam,
    #[id = "lt"]
    pub lt: FloatParam,
    #[id = "ht"]
    pub ht: FloatParam,
    #[id = "cy"]
    pub cy: FloatParam,
    #[id = "hh"]
    pub hh: FloatParam,
    #[id = "tune"]
    pub tune: FloatParam,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "acc_thr"]
    pub acc_thr: FloatParam,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            accent: FloatParam::new("Accent", 7.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            bd: FloatParam::new("Bass Drum", 8.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            sd: FloatParam::new("Snare Drum", 7.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            lt: FloatParam::new("Low Tom", 6.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            ht: FloatParam::new("High Tom", 6.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            cy: FloatParam::new("Cymbal", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            hh: FloatParam::new("Hi-Hat", 6.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),  // texts '0.0' .. '5.0' .. '10.0'
            tune: FloatParam::new("Tune", 0.0, FloatRange::Linear { min: -12.0, max: 12.0 }).with_step_size(0.1).with_unit(" st"),  // texts '-12.0' .. '0.0' .. '12.0'
            decay: pct("Decay", 0.5),
            acc_thr: pct("Accent Threshold", 0.8),
            drive: pct("Drive", 0.0),
            volume: db("Volume", -6.0, -30.0, 6.0),
        }
    }
}

pub const ACCENT: usize = 0;
pub const BD: usize = 1;
pub const SD: usize = 2;
pub const LT: usize = 3;
pub const HT: usize = 4;
pub const CY: usize = 5;
pub const HH: usize = 6;
pub const TUNE: usize = 7;
pub const DECAY: usize = 8;
pub const ACC_THR: usize = 9;
pub const DRIVE: usize = 10;
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
            self.accent.as_ptr(),
            self.bd.as_ptr(),
            self.sd.as_ptr(),
            self.lt.as_ptr(),
            self.ht.as_ptr(),
            self.cy.as_ptr(),
            self.hh.as_ptr(),
            self.tune.as_ptr(),
            self.decay.as_ptr(),
            self.acc_thr.as_ptr(),
            self.drive.as_ptr(),
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

use std::f64::consts::TAU;

/// The metallic source: six square waves (cymbal and hats).
const METAL: [f64; 6] = [245.0, 306.0, 365.0, 415.0, 437.0, 619.0];

#[derive(Clone, Copy, Default)]
struct Tone {
    ph: f64,
    f: f64,
    drop: f64,
    env: f64,
    dec: f64,
    lvl: f64,
}

impl Tone {
    fn hit(&mut self, f: f64, drop: f64, dec: f64, lvl: f64) {
        self.f = f;
        self.drop = drop;
        self.env = 1.0;
        self.dec = dec;
        self.lvl = lvl;
        self.ph = 0.0;
    }
    #[inline]
    fn next(&mut self, fs: f64) -> f64 {
        if self.env < 1e-5 {
            return 0.0;
        }
        let f = self.f * (1.0 + self.drop * self.env * self.env);
        self.ph = (self.ph + f / fs).fract();
        let y = (TAU * self.ph).sin() * self.env * self.lvl;
        self.env *= self.dec;
        y
    }
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    bd: Tone,
    sd: [Tone; 2],
    lt: Tone,
    ht: Tone,
    noise_env: f64,
    noise_dec: f64,
    noise_lvl: f64,
    snare_hp: Biquad,
    metal_ph: [f64; 6],
    cy_env: f64,
    cy_dec: f64,
    cy_lvl: f64,
    hh_env: f64,
    hh_dec: f64,
    hh_lvl: f64,
    cy_bp: [Biquad; 2],
    hh_bp: [Biquad; 2],
    rng: u32,
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
    /// per-sample multiplier for a decay of `t` seconds (to -60 dB), scaled by the DECAY knob
    fn dk(&self, t: f64) -> f64 {
        let s = 2f64.powf((self.v[DECAY] as f64 - 0.5) * 3.0);
        (-6.9 / (t * s * self.fs)).exp()
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut snare_hp = Biquad::identity();
        snare_hp.set_highpass(fs, 1200.0, 0.7);
        let mut cy = [Biquad::identity(); 2];
        cy[0].set_bandpass(fs, 7000.0f64.min(fs * 0.4), 0.8);
        cy[1].set_highpass(fs, 5000.0f64.min(fs * 0.4), 0.7);
        let mut hh = [Biquad::identity(); 2];
        hh[0].set_bandpass(fs, 9000.0f64.min(fs * 0.4), 1.0);
        hh[1].set_highpass(fs, 7000.0f64.min(fs * 0.4), 0.7);
        let mut dc = Biquad::identity();
        dc.set_highpass(fs, 15.0, 0.7);
        Self {
            fs,
            v,
            bd: Tone::default(),
            sd: [Tone::default(); 2],
            lt: Tone::default(),
            ht: Tone::default(),
            noise_env: 0.0,
            noise_dec: 0.0,
            noise_lvl: 0.0,
            snare_hp,
            metal_ph: [0.0; 6],
            cy_env: 0.0,
            cy_dec: 0.0,
            cy_lvl: 0.0,
            hh_env: 0.0,
            hh_dec: 0.0,
            hh_lvl: 0.0,
            cy_bp: cy,
            hh_bp: hh,
            rng: 0x1234_5678,
            dc,
            out: Smooth::new(fs, 20.0, 1.0),
        }
    }
    fn reset(&mut self) {
        self.bd.env = 0.0;
        self.sd.iter_mut().for_each(|t| t.env = 0.0);
        self.lt.env = 0.0;
        self.ht.env = 0.0;
        self.noise_env = 0.0;
        self.cy_env = 0.0;
        self.hh_env = 0.0;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[VOLUME] as f64));
    }
    fn midi(&mut self, ev: Midi) {
        let Midi::On { note, vel } = ev else { return };
        let v = self.v;
        let acc = if vel >= v[ACC_THR] { 1.0 + v[ACCENT] as f64 / 10.0 } else { 1.0 };
        let lv = |x: f32| (x as f64 / 10.0).powi(2) * acc;
        let tune = 2f64.powf(v[TUNE] as f64 / 12.0);
        match note {
            35 | 36 => {
                let d = self.dk(0.35);
                self.bd.hit(52.0 * tune, 1.5, d, lv(v[BD]) * 1.4);
            }
            37 | 38 | 39 | 40 => {
                let d = self.dk(0.12);
                self.sd[0].hit(180.0 * tune, 0.4, d, lv(v[SD]) * 0.6);
                self.sd[1].hit(330.0 * tune, 0.3, d, lv(v[SD]) * 0.4);
                self.noise_env = 1.0;
                self.noise_dec = self.dk(0.2);
                self.noise_lvl = lv(v[SD]) * 0.6;
            }
            41 | 43 | 45 => {
                let d = self.dk(0.45);
                self.lt.hit(125.0 * tune, 0.25, d, lv(v[LT]));
            }
            47 | 48 | 50 => {
                let d = self.dk(0.4);
                self.ht.hit(190.0 * tune, 0.25, d, lv(v[HT]));
            }
            49 | 51 | 52 | 55 | 57 | 59 => {
                self.cy_env = 1.0;
                self.cy_dec = self.dk(1.2);
                self.cy_lvl = lv(v[CY]) * 0.5;
            }
            42 | 44 => {
                self.hh_env = 1.0;
                self.hh_dec = self.dk(0.06);
                self.hh_lvl = lv(v[HH]) * 0.5;
            }
            46 => {
                self.hh_env = 1.0;
                self.hh_dec = self.dk(0.45);
                self.hh_lvl = lv(v[HH]) * 0.5;
            }
            _ => {}
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let tune = 2f64.powf(self.v[TUNE] as f64 / 12.0);
        let drive = self.v[DRIVE] as f64;
        let dg = 1.0 + 12.0 * drive;
        let mut peak = 0.0f64;
        for k in 0..n {
            let mut y = self.bd.next(fs) + self.sd[0].next(fs) + self.sd[1].next(fs) + self.lt.next(fs) + self.ht.next(fs);
            if self.noise_env > 1e-5 {
                let nz = self.noise();
                y += self.snare_hp.process(nz) * self.noise_env * self.noise_lvl;
                self.noise_env *= self.noise_dec;
            }
            if self.cy_env > 1e-5 || self.hh_env > 1e-5 {
                let mut m = 0.0;
                for (i, f) in METAL.iter().enumerate() {
                    self.metal_ph[i] = (self.metal_ph[i] + f * tune * 8.0 / fs).fract();
                    m += if self.metal_ph[i] < 0.5 { 1.0 } else { -1.0 };
                }
                m /= 6.0;
                let c = self.cy_bp[0].process(m);
                let c = self.cy_bp[1].process(c);
                let h = self.hh_bp[0].process(m);
                let h = self.hh_bp[1].process(h);
                y += c * self.cy_env * self.cy_lvl + h * self.hh_env * self.hh_lvl;
                self.cy_env *= self.cy_dec;
                self.hh_env *= self.hh_dec;
            }
            let y = self.dc.process(y);
            let y = if drive > 0.0 { (y * dg).tanh() / (1.0 + drive) } else { y };
            let y = y * self.out.next();
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
        Preset { name: "Stock 606", values: &[] },
        Preset { name: "Big Black: Hot Mixer", values: &[(ACCENT, 8.0), (BD, 9.0), (SD, 8.0), (DECAY, 0.4), (DRIVE, 0.55)] },
        Preset { name: "Atomizer Tight", values: &[(BD, 9.0), (SD, 8.5), (DECAY, 0.25), (DRIVE, 0.3)] },
        Preset { name: "Calypso Toms", values: &[(LT, 8.0), (HT, 8.0), (TUNE, 2.0), (DECAY, 0.7)] },
        Preset { name: "Deep and Long", values: &[(BD, 10.0), (TUNE, -4.0), (DECAY, 0.9)] },
        Preset { name: "Distorted to Death", values: &[(DRIVE, 1.0), (VOLUME, -10.0)] },
    ]
}

/*
('knob', 51.5, 149.5, 22.0)
('knob', 139.5, 149.5, 22.0)
('knob', 227.5, 149.5, 22.0)
('knob', 315.5, 149.5, 22.0)
('knob', 403.5, 149.5, 22.0)
('knob', 491.5, 149.5, 22.0)
('knob', 579.5, 149.5, 22.0)
('divider', 230)
('knob', 89.5, 319.5, 22.0)
('knob', 199.5, 319.5, 22.0)
('knob', 309.5, 319.5, 22.0)
('knob', 419.5, 319.5, 22.0)
('knob', 529.5, 319.5, 22.0)

crate: ['library/panic_abort', 'sourcealbini/pa_tr606']
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
AFTER THE ROLAND TR-606 DRUMATIXANALOG DRUM VOICES  
  MIDI  
  ACCENTACCENTHow much louder accented notes are (velocity above ACCENT THRESHOLD).BASS DRUMInstrument level (as the 606's level knobs).SNARELOW TOMHIGH TOMCYMBALHI-HATTUNEPitch of the drums (not on the original: 0 = stock).DECAYDecay of every voice (50% = stock).ACCENT THRESH.MIDI velocity at which a note plays accented.DRIVEOverdrive the output, like a 606 pushed into a hot mixer channel.VOLUMEOutput level.TR-606Stock 606
Big Black: Hot Mixer
Atomizer Tight
333?
Calypso Toms
fff?
Deep and Long
Distorted to Death
j?$Dsp

*/
fn panel() -> Panel {
    Panel {
        title: "TR-606",
        subtitle: ("AFTER THE ROLAND TR-606 DRUMATIX", "ANALOG DRUM VOICES  ·  MIDI"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::Knob { p: ACCENT, x: 52.0, y: 150.0, r: 22.0, label: "ACCENT", tip: "How much louder accented notes are (velocity above ACCENT THRESHOLD).", active: None },
            Item::Knob { p: BD, x: 140.0, y: 150.0, r: 22.0, label: "BASS DRUM", tip: "Instrument level (as the 606's level knobs).", active: None },
            Item::Knob { p: SD, x: 228.0, y: 150.0, r: 22.0, label: "SNARE", tip: "Instrument level (as the 606's level knobs).", active: None },
            Item::Knob { p: LT, x: 316.0, y: 150.0, r: 22.0, label: "LOW TOM", tip: "Instrument level (as the 606's level knobs).", active: None },
            Item::Knob { p: HT, x: 404.0, y: 150.0, r: 22.0, label: "HIGH TOM", tip: "Instrument level (as the 606's level knobs).", active: None },
            Item::Knob { p: CY, x: 492.0, y: 150.0, r: 22.0, label: "CYMBAL", tip: "Instrument level (as the 606's level knobs).", active: None },
            Item::Knob { p: HH, x: 580.0, y: 150.0, r: 22.0, label: "HI-HAT", tip: "Instrument level (as the 606's level knobs).", active: None },
            Item::Divider { y: 230.0 },
            Item::Knob { p: TUNE, x: 90.0, y: 320.0, r: 22.0, label: "TUNE", tip: "Pitch of the drums (not on the original: 0 = stock).", active: None },
            Item::Knob { p: DECAY, x: 200.0, y: 320.0, r: 22.0, label: "DECAY", tip: "Decay of every voice (50% = stock).", active: None },
            Item::Knob { p: ACC_THR, x: 310.0, y: 320.0, r: 22.0, label: "ACCENT THRESH.", tip: "MIDI velocity at which a note plays accented.", active: None },
            Item::Knob { p: DRIVE, x: 420.0, y: 320.0, r: 22.0, label: "DRIVE", tip: "Overdrive the output, like a 606 pushed into a hot mixer channel.", active: None },
            Item::Knob { p: VOLUME, x: 530.0, y: 320.0, r: 22.0, label: "VOLUME", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "TR-606", class_id: *b"PATR606DrumsPQD1", vendor: "PEQUOD Albini",
    instrument, categories: [Instrument, Drum]
);
