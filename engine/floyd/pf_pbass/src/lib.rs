//! P-Bass Voice: shapes a bass DI toward Roger Waters' Precision Bass with roundwound strings, played with a pick (his
//! usual way) or fingers: voicing, pick position, string age (brightness that fades after each attack), palm mute, body thump and tone.

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
pub enum Bass {
    #[name = "Precision, pick"]
    PrecisionPick,
    #[name = "Precision, fingers"]
    PrecisionFingers,
    #[name = "As played (no voicing)"]
    AsPlayedNoVoicing,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "bass"]
    pub bass: EnumParam<Bass>,
    #[id = "pickup"]
    pub pickup: FloatParam,
    #[id = "body"]
    pub body: FloatParam,
    #[id = "flats"]
    pub flats: FloatParam,
    #[id = "mute"]
    pub mute: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            bass: EnumParam::new("Bass", Bass::PrecisionPick),
            pickup: pct("Pick Position (neck to bridge)", 0.5),
            body: pct("Body", 0.4),
            flats: pct("String Age", 0.25),
            mute: pct("Palm Mute", 0.0),
            tone: pct("Tone", 0.6),
            output: db("Output", 0.0, -18.0, 18.0),
        }
    }
}

pub const BASS: usize = 0;
pub const PICKUP: usize = 1;
pub const BODY: usize = 2;
pub const FLATS: usize = 3;
pub const MUTE: usize = 4;
pub const TONE: usize = 5;
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
            self.bass.as_ptr(),
            self.pickup.as_ptr(),
            self.body.as_ptr(),
            self.flats.as_ptr(),
            self.mute.as_ptr(),
            self.tone.as_ptr(),
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

/// Topology-preserving state-variable low-pass: safe to sweep every sample.
#[derive(Clone, Copy, Default)]
struct Svf {
    s1: f64,
    s2: f64,
}

impl Svf {
    #[inline]
    fn lp(&mut self, x: f64, fs: f64, fc: f64, q: f64) -> f64 {
        let g = (std::f64::consts::PI * fc.clamp(20.0, fs * 0.45) / fs).tan();
        let k = 1.0 / q;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let v3 = x - self.s2;
        let v1 = a1 * self.s1 + g * a1 * v3;
        let v2 = self.s2 + g * v1;
        self.s1 = 2.0 * v1 - self.s1;
        self.s2 = 2.0 * v2 - self.s2;
        v2
    }
}

/// Per-channel state.
#[derive(Clone, Copy, Default)]
struct Chan {
    voice: [Biquad; 3],
    pick: [Biquad; 2],
    body_bp: Biquad,
    tone: Biquad,
    svf: Svf,
    pow: f64,
    fast: f64,
    slow: f64,
    d_mute: f64,
    d_flat: f64,
    d_body: f64,
    g: f64,
}

pub struct Dsp {
    fs: f64,
    ch: [Chan; 2],
    v: [f32; NPARAMS],
    key: [f64; 3],
    tone_key: f64,
    out: Smooth,
}

impl Dsp {
    fn voice(&mut self, bass: usize, pickup: f64) {
        let fs = self.fs;
        for c in &mut self.ch {
            match bass {
                // Precision with a pick: a tight low end, the pick's click and a forward upper-mid
                0 => {
                    c.voice[0].set_low_shelf(fs, 160.0, 0.7, 6.5);
                    c.voice[1].set_peak(fs, 900.0, 0.9, 2.5);
                    c.voice[2].set_peak(fs, 2800.0, 1.2, 4.0);
                    c.body_bp.set_bandpass(fs, 100.0, 2.0);
                }
                // Precision with fingers: rounder, more low-mid, less top
                1 => {
                    c.voice[0].set_peak(fs, 150.0, 0.8, 5.0);
                    c.voice[1].set_peak(fs, 700.0, 1.0, 1.0);
                    c.voice[2].set_high_shelf(fs, 3000.0, 0.7, -3.0);
                    c.body_bp.set_bandpass(fs, 95.0, 2.5);
                }
                _ => {
                    c.voice = [Biquad::identity(); 3];
                    c.body_bp.set_bandpass(fs, 100.0, 2.0);
                }
            }
            // pickup: neck = round and deep, bridge = tight and nasal
            let p = pickup;
            c.pick[0].set_low_shelf(fs, 200.0, 0.7, 3.0 * (1.0 - p) - 5.0 * p);
            c.pick[1].set_peak(fs, 1500.0, 0.8, -3.0 * (1.0 - p) + 4.0 * p);
        }
        self.key = [bass as f64, pickup, 0.0];
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, ch: [Chan::default(); 2], v, key: [-1.0; 3], tone_key: -1.0, out: Smooth::new(fs, 20.0, 1.0) };
        for c in &mut d.ch {
            c.g = 1.0;
            c.tone = Biquad::identity();
        }
        d.voice(0, 0.2);
        d
    }
    fn reset(&mut self) {
        let (b, p) = (self.key[0] as usize, self.key[1]);
        for c in &mut self.ch {
            *c = Chan { g: 1.0, ..Chan::default() };
        }
        self.voice(b, p);
        self.tone_key = -1.0;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let b = (v[BASS].round() as usize).min(2);
        let p = v[PICKUP] as f64;
        if b as f64 != self.key[0] || (p - self.key[1]).abs() > 1e-3 {
            self.voice(b, p);
        }
        let t = v[TONE] as f64;
        if (t - self.tone_key).abs() > 1e-3 {
            for c in &mut self.ch {
                if t >= 0.995 {
                    c.tone = Biquad::identity();
                } else {
                    c.tone.set_lowpass(self.fs, 700.0 * 2f64.powf(4.5 * t), 0.7);
                }
            }
            self.tone_key = t;
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let p = |i: usize| self.v[i] as f64;
        let (body, flats, mute) = (p(BODY), p(FLATS), p(MUTE));
        let body_scale = [1.0, 0.6, 0.4][(p(BASS).round() as usize).min(2)];
        // detectors: power smoothed over 15 ms (no ripple from low notes), then a 40 ms follower to compare against
        let c_pow = 1.0 - coeff(fs, 15.0);
        let c_slow = 1.0 - coeff(fs, 40.0);
        // decay envelopes: foam mute (1.5 s .. 60 ms), flatwound brightness (3 s .. 150 ms), body thump (50 ms)
        let k_mute = coeff(fs, 1500.0 * (60.0f64 / 1500.0).powf(mute));
        let k_flat = coeff(fs, 3000.0 * (150.0f64 / 3000.0).powf(flats));
        let k_body = coeff(fs, 50.0);
        let c_gain = 1.0 - coeff(fs, 2.0);
        let mute_amt = (mute * 1.5).min(0.985);
        let base_cut = 16000.0 * 0.12f64.powf(flats);
        let foam_cut = 16000.0 * (2500.0f64 / 16000.0).powf(mute);
        let mut peak = 0.0f64;
        for k in 0..n {
            let o = self.out.next();
            for (ci, chn) in chans.iter_mut().enumerate().take(2) {
                let c = &mut self.ch[ci];
                let x = chn[k] as f64;
                // note-attack detector: rises only on real attacks, not on the waveform
                c.pow += (x * x - c.pow) * c_pow;
                let fast = c.pow.max(0.0).sqrt();
                c.fast = fast;
                c.slow += (fast - c.slow) * c_slow;
                // a jump of more than ~1.5 dB over the recent level counts as a new note; 2.5 dB or more is a full attack
                let rise = if fast > 1e-6 { (20.0 * (fast / c.slow.max(1e-9)).log10() - 1.5) / 1.0 } else { 0.0 };
                let rise = rise.clamp(0.0, 1.0);
                // each envelope jumps up on an attack and otherwise decays at its own rate
                c.d_mute = (c.d_mute * k_mute).max(rise);
                c.d_flat = (c.d_flat * k_flat).max(rise);
                c.d_body = (c.d_body * k_body).max(rise);
                // voicing
                let mut y = cascade(&mut c.voice, x);
                y = cascade(&mut c.pick, y);
                // body thump on the attack
                y += c.body_bp.process(x) * body * body_scale * c.d_body * 1.5;
                // flatwounds: bright on the attack, dulling as the note rings; foam also dulls
                let cut = (base_cut * (1.0 + 3.0 * flats * c.d_flat)).min(foam_cut.max(base_cut * 0.5)).min(16000.0);
                y = c.svf.lp(y, fs, cut, 0.7);
                // foam mute: the note dies away after each attack (gain smoothed: no clicks, no stutter)
                let target = 1.0 - mute_amt * (1.0 - c.d_mute);
                c.g += (target - c.g) * c_gain;
                y *= c.g;
                y = c.tone.process(y) * o;
                peak = peak.max(y.abs());
                chn[k] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Money (pick, roundwounds)", values: &[(PICKUP, 0.55), (TONE, 0.75)] },
        Preset { name: "One of These Days", values: &[(PICKUP, 0.7), (BODY, 0.3), (FLATS, 0.15), (TONE, 0.85)] },
        Preset { name: "Have a Cigar", values: &[(BODY, 0.45), (FLATS, 0.3), (TONE, 0.7)] },
        Preset { name: "Dogs (Animals)", values: &[(PICKUP, 0.45), (BODY, 0.5), (FLATS, 0.4)] },
        Preset { name: "Fingers, Round", values: &[(BASS, 1.0), (PICKUP, 0.3), (BODY, 0.5), (FLATS, 0.5), (TONE, 0.5)] },
        Preset { name: "Old Strings, Muted", values: &[(PICKUP, 0.4), (FLATS, 0.85), (MUTE, 0.5), (TONE, 0.45)] },
    ]
}

/*
('selector?', 109.5, 169.5, 36, 36)
('knob', 249.5, 169.5, 39.0)
('knob', 389.5, 169.5, 39.0)
('knob', 529.5, 169.5, 39.0)
('divider', 284)
('knob', 159.5, 334.5, 22.0)
('knob', 319.5, 334.5, 22.0)
('knob', 479.5, 334.5, 22.0)

crate: ['floyd/pf_pbass', 'library/panic_abort']
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
ROGER'S PRECISION BASSROUNDWOUNDS 
 PICK OR FINGERSPICKFINGERSAS PLAYEDtYE
BASSVoice a DI toward a Precision played with a pick (Waters' usual way) or with fingers. AS PLAYED leaves your bass alone (strings and mute still work).PICK POSITIONWhere you pick: near the neck (round and deep) to near the bridge (tight and snappy).STRING AGEHow fast each note's brightness fades: new roundwounds (low) keep their zing; old strings (high) go dull quickly.PALM MUTEPalm on the strings at the bridge: notes die away quickly.BODYThe low thump of the body on each attack.TONEThe bass's tone control: rolls off the top.OUTPUTOutput level.P-BASS VOICE
Money (pick, roundwounds)
333?
One of These Days
333?
Have a Cigar
Dogs (Animals)
Fingers, Round
Old Strings, Muted

*/
fn panel() -> Panel {
    Panel {
        title: "P-BASS VOICE",
        subtitle: ("ROGER'S PRECISION BASS", "ROUNDWOUNDS · PICK OR FINGERS"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::Selector { p: BASS, x: 110.0, y: 170.0, label: "BASS", positions: &["PICK", "FINGERS", "AS PLAYED"], tip: "Voice a DI toward a Precision played with a pick (Waters' usual way) or with fingers. AS PLAYED leaves your bass alone (strings and mute still work)." },
            Item::Knob { p: PICKUP, x: 250.0, y: 170.0, r: 40.0, label: "PICK POSITION", tip: "Where you pick: near the neck (round and deep) to near the bridge (tight and snappy).", active: None },
            Item::Knob { p: FLATS, x: 390.0, y: 170.0, r: 40.0, label: "STRING AGE", tip: "How fast each note's brightness fades: new roundwounds (low) keep their zing; old strings (high) go dull quickly.", active: None },
            Item::Knob { p: MUTE, x: 530.0, y: 170.0, r: 40.0, label: "PALM MUTE", tip: "Palm on the strings at the bridge: notes die away quickly.", active: None },
            Item::Divider { y: 284.0 },
            Item::Knob { p: BODY, x: 160.0, y: 335.0, r: 22.0, label: "BODY", tip: "The low thump of the body on each attack.", active: None },
            Item::Knob { p: TONE, x: 320.0, y: 335.0, r: 22.0, label: "TONE", tip: "The bass's tone control: rolls off the top.", active: None },
            Item::Knob { p: OUTPUT, x: 480.0, y: 335.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "P-Bass Voice", class_id: *b"PFPBassVoicePQD1", vendor: "PEQUOD Pink Floyd",
    categories: [Fx, Filter]
);
