//! RS-202: a string machine in the manner of the Roland RS-202 (1976).
//!
//! Roland's answer to the Solina: a divide-down strings section (8' and 4', to mix) and a brass
//! section, whose notes go through a filter that opens as they swell in - the brass "blat". Both
//! share the attack and release, and a delayed vibrato. The ensemble has two settings: I is the
//! gentler, II the deeper and faster swirl.

#![allow(clippy::excessive_precision)]
use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
#[allow(unused_imports)]
use vintage::dsp::*;
use vintage::fx::StringEnsemble;
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
pub const H: u32 = 420;

const VOICES: usize = 32;

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "s8"]
    pub s8: FloatParam,
    #[id = "s4"]
    pub s4: FloatParam,
    #[id = "stone"]
    pub stone: FloatParam,
    #[id = "brass"]
    pub brass: FloatParam,
    #[id = "bright"]
    pub bright: FloatParam,
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "release"]
    pub release: FloatParam,
    #[id = "ens"]
    pub ens: FloatParam,
    #[id = "vrate"]
    pub vrate: FloatParam,
    #[id = "vdepth"]
    pub vdepth: FloatParam,
    #[id = "vdelay"]
    pub vdelay: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            s8: pct("Strings 8'", 0.8),
            s4: pct("Strings 4'", 0.4),
            stone: pct("String Tone", 0.5),
            brass: pct("Brass", 0.0),
            bright: pct("Brass Brilliance", 0.5),
            attack: ms("Attack", 80.0, 3.0, 2000.0),
            release: ms("Release", 800.0, 30.0, 5000.0),
            ens: FloatParam::new("Ensemble", 1.0, FloatRange::Linear { min: 0.0, max: 2.0 }).with_step_size(1.0),
            vrate: rate("Vibrato Rate", 5.5, 2.0, 9.0),
            vdepth: pct("Vibrato Depth", 0.0),
            vdelay: ms("Vibrato Delay", 400.0, 0.0, 2000.0),
            volume: db("Volume", -6.0, -30.0, 6.0),
        }
    }
}

pub const S8: usize = 0;
pub const S4: usize = 1;
pub const STONE: usize = 2;
pub const BRASS: usize = 3;
pub const BRIGHT: usize = 4;
pub const ATTACK: usize = 5;
pub const RELEASE: usize = 6;
pub const ENS: usize = 7;
pub const VRATE: usize = 8;
pub const VDEPTH: usize = 9;
pub const VDELAY: usize = 10;
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
            self.s8.as_ptr(),
            self.s4.as_ptr(),
            self.stone.as_ptr(),
            self.brass.as_ptr(),
            self.bright.as_ptr(),
            self.attack.as_ptr(),
            self.release.as_ptr(),
            self.ens.as_ptr(),
            self.vrate.as_ptr(),
            self.vdepth.as_ptr(),
            self.vdelay.as_ptr(),
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
    on: bool,
    held: bool,
    note: u8,
    f: f64,
    env: f64,
    age: u64,
    lp: [f64; 2],
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    voices: [Voice; VOICES],
    clock: u64,
    order: u64,
    pedal: bool,
    /// time since the first key of a phrase went down (the vibrato's delay)
    phrase: f64,
    vib: f64,
    tone: Biquad,
    tone_set: f32,
    body: Biquad,
    ens: StringEnsemble,
    dc: [DcBlock; 2],
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut body = Biquad::identity();
        body.set_peak(fs, 1100.0, 1.0, 4.0);
        Self { fs, v, voices: [Voice::default(); VOICES], clock: 0, order: 0, pedal: false, phrase: 0.0, vib: 0.0, tone: Biquad::identity(), tone_set: f32::NAN, body, ens: StringEnsemble::new(fs), dc: [DcBlock::new(fs), DcBlock::new(fs)] }
    }
    fn reset(&mut self) {
        self.voices = [Voice::default(); VOICES];
        self.ens.reset();
        self.tone.reset();
        self.body.reset();
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn midi(&mut self, ev: Midi) {
        match ev {
            Midi::On { note, .. } => {
                self.order += 1;
                if !self.voices.iter().any(|v| v.on && v.held) {
                    self.phrase = 0.0;
                }
                let i = self.voices.iter().position(|v| v.on && v.note == note).or_else(|| self.voices.iter().position(|v| !v.on)).unwrap_or_else(|| self.voices.iter().enumerate().min_by_key(|(_, v)| v.age).map(|(i, _)| i).unwrap_or(0));
                let vc = &mut self.voices[i];
                if !(vc.on && vc.note == note) {
                    vc.env = 0.0;
                    vc.lp = [0.0; 2];
                }
                vc.on = true;
                vc.held = true;
                vc.note = note;
                vc.age = self.order;
                vc.f = 440.0 * 2f64.powf((note as f64 - 69.0) / 12.0);
            }
            Midi::Off { note } => self.voices.iter_mut().filter(|v| v.on && v.note == note).for_each(|v| v.held = false),
            Midi::Cc { cc: 64, value } => self.pedal = value >= 0.5,
            Midi::AllOff => self.voices.iter_mut().for_each(|v| v.on = false),
            _ => {}
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let p = self.v;
        if self.tone_set != p[STONE] {
            self.tone_set = p[STONE];
            self.tone.set_lowpass(fs, 1500.0 * 2f64.powf(3.0 * p[STONE] as f64), 0.7);
        }
        let (s8, s4, brass) = (p[S8] as f64, p[S4] as f64, p[BRASS] as f64);
        let bright = p[BRIGHT] as f64;
        let att = 1.0 / (p[ATTACK].max(1.0) as f64 * 0.001 * fs);
        let rel = (-6.9 / (p[RELEASE].max(1.0) as f64 * 0.001 * fs)).exp();
        let ens = p[ENS].round() as i32;
        let (vr, vd, vdl) = (p[VRATE] as f64, p[VDEPTH] as f64, p[VDELAY] as f64 * 0.001);
        let vol = db_to_gain(p[VOLUME] as f64);
        let pedal = self.pedal;
        let mut peak = 0.0f64;
        for k in 0..n {
            let t = self.clock as f64 / fs;
            self.clock += 1;
            self.phrase += 1.0 / fs;
            self.vib = (self.vib + vr / fs).fract();
            // delayed vibrato: up to about 20 cents, fading in after the delay
            let vamt = vd * ((self.phrase - vdl) / 0.5).clamp(0.0, 1.0);
            let bend = 2f64.powf(vamt * 20.0 / 1200.0 * (std::f64::consts::TAU * self.vib).sin());
            let (mut strings, mut br) = (0.0, 0.0);
            for vc in self.voices.iter_mut().filter(|v| v.on) {
                if vc.held || pedal {
                    vc.env = (vc.env + att).min(1.0);
                } else {
                    vc.env *= rel;
                    if vc.env < 1e-5 {
                        vc.on = false;
                        continue;
                    }
                }
                let e = vc.env;
                // divide-down: the phase is the master clock's, so every key is locked
                let f = vc.f;
                let ph = (f * t).fract();
                let dt = f * bend / fs;
                let saw8 = 2.0 * ph - 1.0 - blep(ph, dt);
                if s8 > 0.0 || s4 > 0.0 {
                    let ph4 = (2.0 * f * t).fract();
                    let saw4 = 2.0 * ph4 - 1.0 - blep(ph4, (2.0 * dt).min(0.45));
                    strings += (s8 * saw8 + s4 * saw4) * e;
                }
                if brass > 0.0 {
                    // the brass filter follows the note's swell: it opens as the note comes in
                    let fc = (300.0 + e * (800.0 + 5000.0 * bright)).min(0.4 * fs);
                    let a = 1.0 - (-std::f64::consts::TAU * fc / fs).exp();
                    vc.lp[0] += a * (saw8 - vc.lp[0]);
                    vc.lp[1] += a * (vc.lp[0] - vc.lp[1]);
                    br += vc.lp[1] * e;
                }
            }
            let x = (1.5 * self.tone.process(strings) + brass * 1.6 * self.body.process(br)) * 0.12;
            let (l, r) = match ens {
                1 => self.ens.process(x, 0.5, 5.5, 0.75),
                2 => self.ens.process(x, 0.8, 7.0, 1.15),
                _ => (x, x),
            };
            let l = self.dc[0].process(l) * vol;
            let r = self.dc[1].process(r) * vol;
            peak = peak.max(l.abs()).max(r.abs());
            if chans.len() >= 2 {
                chans[0][k] = l as f32;
                chans[1][k] = r as f32;
            } else if let Some(c) = chans.get_mut(0) {
                c[k] = (0.5 * (l + r)) as f32;
            }
        }
        meters[0] = peak as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Strings, Ensemble I", values: &[(S8, 0.8), (S4, 0.4), (BRASS, 0.0), (ENS, 1.0)] },
        Preset { name: "Strings, Ensemble II", values: &[(S8, 0.8), (S4, 0.6), (BRASS, 0.0), (ENS, 2.0), (STONE, 0.65)] },
        Preset { name: "Slow Strings Pad", values: &[(S8, 0.9), (S4, 0.3), (ATTACK, 900.0), (RELEASE, 3000.0), (ENS, 2.0)] },
        Preset { name: "Brass Section", values: &[(S8, 0.0), (S4, 0.0), (BRASS, 0.9), (BRIGHT, 0.6), (ATTACK, 60.0), (RELEASE, 250.0), (ENS, 1.0), (VDEPTH, 0.3)] },
        Preset { name: "Strings and Brass", values: &[(S8, 0.6), (S4, 0.3), (BRASS, 0.6), (BRIGHT, 0.4), (ENS, 2.0)] },
        Preset { name: "Vibrato Strings", values: &[(S8, 0.8), (S4, 0.5), (ENS, 1.0), (VDEPTH, 0.6), (VDELAY, 600.0)] },
    ]
}

fn panel() -> Panel {
    let k = |p, x, y, r, label, tip| Item::Knob { p, x, y, r, label, tip, active: None };
    Panel {
        title: "RS-202",
        subtitle: ("IN THE MANNER OF THE ROLAND RS-202", "STRINGS 8' 4' · BRASS · ENSEMBLE I / II · VIBRATO"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            k(S8, 70.0, 150.0, 24.0, "STRINGS 8'", "The strings at the key's pitch."),
            k(S4, 160.0, 150.0, 24.0, "STRINGS 4'", "The strings an octave up."),
            k(STONE, 250.0, 150.0, 24.0, "TONE", "The strings' tone: darker to brighter."),
            k(BRASS, 380.0, 150.0, 24.0, "BRASS", "The brass section's level."),
            k(BRIGHT, 470.0, 150.0, 24.0, "BRILLIANCE", "How far the brass filter opens as each note swells in."),
            k(VOLUME, 570.0, 150.0, 24.0, "VOLUME", "Output level."),
            Item::Divider { y: 220.0 },
            k(ATTACK, 70.0, 300.0, 24.0, "ATTACK", "How slowly each note swells in (the brass opens with it)."),
            k(RELEASE, 160.0, 300.0, 24.0, "RELEASE", "How long notes fade after the key comes up. The sustain pedal holds them."),
            Item::Selector { p: ENS, x: 270.0, y: 300.0, label: "ENSEMBLE", positions: &["OFF", "I", "II"], tip: "The ensemble: I is the gentler swirl, II the deeper and faster one." },
            k(VRATE, 390.0, 300.0, 20.0, "VIB RATE", "Vibrato speed."),
            k(VDEPTH, 470.0, 300.0, 20.0, "VIB DEPTH", "Vibrato depth."),
            k(VDELAY, 550.0, 300.0, 20.0, "VIB DELAY", "How long after you start playing the vibrato fades in."),
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "RS-202", class_id: *b"PQKeysRS202Rolnd", vendor: "PEQUOD Keys",
    instrument, categories: [Instrument, Synth]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    fn tune_params() -> Vec<f32> {
        with(defaults(), &[(S4, 0.0), (ENS, 0.0), (ATTACK, 3.0), (RELEASE, 100.0)])
    }

    include!("../../instrument_tests.rs");
}
