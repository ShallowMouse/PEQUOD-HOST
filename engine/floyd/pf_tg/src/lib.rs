//! TG Channel: an EMI TG12345 Mk I channel (1971), the desk The Dark Side of the Moon was made on: the transistor mic amp
//! with its transformer, bass / presence / treble, and the TG12413 limiter (compress or limit, six recovery times).

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
pub enum PresFreq {
    #[name = "500 Hz"]
    V500Hz,
    #[name = "800 Hz"]
    V800Hz,
    #[name = "1.2 kHz"]
    V12Khz,
    #[name = "1.8 kHz"]
    V18Khz,
    #[name = "2.8 kHz"]
    V28Khz,
    #[name = "4.2 kHz"]
    V42Khz,
    #[name = "6.8 kHz"]
    V68Khz,
}

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum LimMode {
    #[name = "Out"]
    Out,
    #[name = "Compress"]
    Compress,
    #[name = "Limit"]
    Limit,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "input"]
    pub input: FloatParam,
    #[id = "bass"]
    pub bass: FloatParam,
    #[id = "presence"]
    pub presence: FloatParam,
    #[id = "pres_freq"]
    pub pres_freq: EnumParam<PresFreq>,
    #[id = "treble"]
    pub treble: FloatParam,
    #[id = "lim_mode"]
    pub lim_mode: EnumParam<LimMode>,
    #[id = "lim_drive"]
    pub lim_drive: FloatParam,
    #[id = "recovery"]
    pub recovery: IntParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            input: db("Input Gain", 0.0, -12.0, 24.0),
            bass: db("Bass", 0.0, -10.0, 10.0),
            presence: db("Presence", 0.0, -10.0, 10.0),
            pres_freq: EnumParam::new("Presence Freq", PresFreq::V28Khz),
            treble: db("Treble", 0.0, -10.0, 10.0),
            lim_mode: EnumParam::new("Limiter", LimMode::Out),
            lim_drive: db("Limiter Drive", 6.0, 0.0, 30.0),
            recovery: IntParam::new("Recovery", 3, IntRange::Linear { min: 1, max: 6 })
                .with_value_to_string(Arc::new(|v| format!("{v} ({} s)", ["0.1", "0.25", "0.5", "1", "2", "5"][(v.clamp(1, 6) - 1) as usize])))
                .with_string_to_value(Arc::new(|s| s.trim().split(' ').next().and_then(|t| t.parse().ok()))),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const INPUT: usize = 0;
pub const BASS: usize = 1;
pub const PRESENCE: usize = 2;
pub const PRES_FREQ: usize = 3;
pub const TREBLE: usize = 4;
pub const LIM_MODE: usize = 5;
pub const LIM_DRIVE: usize = 6;
pub const RECOVERY: usize = 7;
pub const OUTPUT: usize = 8;
pub const NPARAMS: usize = 9;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.input.as_ptr(),
            self.bass.as_ptr(),
            self.presence.as_ptr(),
            self.pres_freq.as_ptr(),
            self.treble.as_ptr(),
            self.lim_mode.as_ptr(),
            self.lim_drive.as_ptr(),
            self.recovery.as_ptr(),
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

const PRES: [f64; 7] = [500.0, 800.0, 1200.0, 1800.0, 2800.0, 4200.0, 6800.0];
const RECOVERIES: [f64; 6] = [100.0, 250.0, 500.0, 1000.0, 2000.0, 5000.0];
/// The limiter's fixed threshold (dBFS).
const THRESH: f64 = -14.0;

pub struct Dsp {
    fs: f64,
    os: [Oversampler; 2],
    eq: [[Biquad; 3]; 2],
    xf: [Biquad; 2],
    key: Vec<i32>,
    gr: f64,
    v: [f32; NPARAMS],
    gain: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 2;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut xf = Biquad::identity();
        xf.set_lowpass(fs * 2.0, 120.0, 0.7);
        let mut d = Self { fs, os: [Oversampler::new(), Oversampler::new()], eq: [[Biquad::identity(); 3]; 2], xf: [xf; 2], key: vec![], gr: 0.0, v, gain: Smooth::new(fs, 20.0, 1.0), out: Smooth::new(fs, 20.0, 1.0) };
        let v2 = v;
        d.set_params(&v2);
        d
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.os[c].reset();
            self.eq[c].iter_mut().for_each(|b| b.reset());
            self.xf[c].reset();
        }
        self.gr = 0.0;
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = vec![(v[BASS] * 10.0) as i32, (v[TREBLE] * 10.0) as i32, (v[PRESENCE] * 10.0) as i32, v[PRES_FREQ].round() as i32];
        if k != self.key {
            let pf = PRES[(v[PRES_FREQ].round() as usize).min(6)];
            for c in &mut self.eq {
                c[0].set_low_shelf(self.fs, 110.0, 0.6, v[BASS] as f64);
                c[1].set_high_shelf(self.fs, 10000.0f64.min(self.fs * 0.4), 0.6, v[TREBLE] as f64);
                c[2].set_peak(self.fs, pf, 0.9, v[PRESENCE] as f64);
            }
            self.key = k;
        }
        self.gain.set(db_to_gain(v[INPUT] as f64));
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(2);
        let mode = self.v[LIM_MODE].round() as i32;
        let drive = if mode == 0 { 0.0 } else { self.v[LIM_DRIVE] as f64 };
        let dg = db_to_gain(drive);
        let ratio = if mode == 2 { 20.0 } else { 3.0 };
        let att = coeff(self.fs, if mode == 2 { 0.3 } else { 5.0 });
        let rel = coeff(self.fs, RECOVERIES[(self.v[RECOVERY].round() as usize).clamp(1, 6) - 1]);
        let mut grmax = 0.0f64;
        let mut peak = 0.0f64;
        for k in 0..n {
            let g = self.gain.next();
            let o = self.out.next();
            // mic amp: the input transformer saturates in the lows when hot
            let mut pre = [0.0f64; 2];
            for c in 0..nch {
                let x = chans[c][k] as f64 * g;
                let xf = &mut self.xf[c];
                pre[c] = self.os[c].process(x, |s| {
                    let lo = xf.process(s);
                    let s = s - lo + 1.6 * (lo / 1.6).tanh();
                    let s = 1.5 * (s / 1.5).tanh();
                    s + 0.01 * s * s
                });
            }
            let mut y = pre;
            if mode != 0 {
                let lvl = pre.iter().take(nch).fold(0.0f64, |a, &b| a.max((b * dg).abs()));
                let over = (gain_to_db(lvl) - THRESH).max(0.0);
                let target = over * (1.0 - 1.0 / ratio);
                self.gr = if target > self.gr { target + att * (self.gr - target) } else { target + rel * (self.gr - target) };
                grmax = grmax.max(self.gr);
                // levelled back: the drive is taken off again after the limiter
                let gc = db_to_gain(-self.gr + 0.6 * drive);
                for c in 0..nch {
                    y[c] = pre[c] * gc;
                }
            }
            for c in 0..nch {
                let z = cascade(&mut self.eq[c], y[c]) * o;
                peak = peak.max(z.abs());
                chans[c][k] = z as f32;
            }
        }
        meters[0] = grmax as f32;
        meters[1] = peak as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Flat Channel", values: &[] },
        Preset { name: "Dark Side Vocal", values: &[(BASS, -2.0), (PRESENCE, 3.0), (LIM_MODE, 1.0), (LIM_DRIVE, 8.0)] },
        Preset { name: "Hiwatt Guitar", values: &[(BASS, -3.0), (PRESENCE, 2.0), (PRES_FREQ, 3.0), (TREBLE, 1.5)] },
        Preset { name: "Waters' Bass (limited)", values: &[(BASS, 3.0), (PRESENCE, 2.0), (PRES_FREQ, 1.0), (LIM_MODE, 2.0), (LIM_DRIVE, 10.0), (RECOVERY, 3.0)] },
        Preset { name: "Mason's Kit, Squashed", values: &[(BASS, 2.0), (TREBLE, 3.0), (LIM_MODE, 2.0), (LIM_DRIVE, 16.0), (RECOVERY, 1.0)] },
        Preset { name: "Piano (Great Gig)", values: &[(PRESENCE, 2.0), (PRES_FREQ, 2.0), (TREBLE, 2.0), (LIM_MODE, 1.0), (LIM_DRIVE, 5.0), (RECOVERY, 3.0)] },
        Preset { name: "Hot Input (transformer)", values: &[(INPUT, 18.0), (OUTPUT, -14.0)] },
    ]
}

/*
('meter', 480, 90, 140, 90)
('knob', 335.5, 147.5, 9.8)
('knob', 59.5, 149.5, 25.6)
('knob', 149.5, 149.5, 23.2)
('knob', 239.5, 149.5, 23.2)
('knob', 429.5, 149.5, 23.2)
('divider', 236)
('selector?', 89.5, 321.5, 36, 36)
('selector?', 344.5, 321.5, 36, 36)
('knob', 469.5, 321.5, 23.2)

crate: ['floyd/pf_tgdesk', 'library/panic_abort']
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
AFTER THE EMI TG12345 MK I, 1971MIC AMP  
  EQ  
  TG12413 LIMITERINPUTMic/line gain into the transformer and transistor amp. Hot settings thicken and saturate the lows.BASSBass shelf at about 110 Hz, +-10 dB.PRESENCEPresence boost/cut, +-10 dB, at the frequency chosen..5.81.21.82.84.26.8
PRES. FREQPresence frequency (kHz).TREBLETreble shelf at about 10 kHz, +-10 dB.LIMITEROUTCOMP.LIMIT
The TG12413: COMPRESS (soft, about 3:1) or LIMIT (hard, fast).LIMITER DRIVEThe limiter has a fixed threshold: this sets how hard the signal hits it (the output is levelled back for you).123456"NE
RECOVERYRelease time: 0.1, 0.25, 0.5, 1, 2 or 5 seconds.OUTPUTChannel output.PEAKTG CHANNELFlat Channel
Dark Side Vocal
Hiwatt Guitar
Waters' Bass (limited)
Mason's Kit, Squashed
Piano (Great Gig)

*/
fn panel() -> Panel {
    Panel {
        title: "TG CHANNEL",
        subtitle: ("AFTER THE EMI TG12345 MK I, 1971", "MIC AMP  ·  EQ  ·  TG12413 LIMITER"),
        accent: ui::STEEL,
        size: (W, H),
        items: vec![
            Item::Knob { p: INPUT, x: 60.0, y: 150.0, r: 26.0, label: "INPUT", tip: "Mic/line gain into the transformer and transistor amp. Hot settings thicken and saturate the lows.", active: None },
            Item::Knob { p: BASS, x: 150.0, y: 150.0, r: 24.0, label: "BASS", tip: "Bass shelf at about 110 Hz, +-10 dB.", active: None },
            Item::Knob { p: PRESENCE, x: 240.0, y: 150.0, r: 24.0, label: "PRESENCE", tip: "Presence boost/cut, +-10 dB, at the frequency chosen.", active: None },
            Item::Selector { p: PRES_FREQ, x: 336.0, y: 150.0, label: "PRES. FREQ", positions: &[".5", ".8", "1.2", "1.8", "2.8", "4.2", "6.8"], tip: "Presence frequency (kHz)." },
            Item::Knob { p: TREBLE, x: 430.0, y: 150.0, r: 24.0, label: "TREBLE", tip: "Treble shelf at about 10 kHz, +-10 dB.", active: None },
            Item::Meter { m: 0, x: 480.0, y: 90.0, w: 140.0, h: 100.0, kind: MeterKind::GainReduction, caption: "LIMITER" },
            Item::Divider { y: 236.0 },
            Item::Selector { p: LIM_MODE, x: 90.0, y: 322.0, label: "LIMITER", positions: &["OUT", "COMP.", "LIMIT"], tip: "The TG12413: COMPRESS (soft, about 3:1) or LIMIT (hard, fast)." },
            Item::Knob { p: LIM_DRIVE, x: 216.0, y: 322.0, r: 26.0, label: "LIMITER DRIVE", tip: "The limiter has a fixed threshold: this sets how hard the signal hits it (the output is levelled back for you).", active: Some(ActiveWhen { p: LIM_MODE, min: 0.3, max: 1.0 }) },
            Item::Selector { p: RECOVERY, x: 345.0, y: 322.0, label: "RECOVERY", positions: &["1", "2", "3", "4", "5", "6"], tip: "Release time: 0.1, 0.25, 0.5, 1, 2 or 5 seconds." },
            Item::Knob { p: OUTPUT, x: 470.0, y: 322.0, r: 24.0, label: "OUTPUT", tip: "Channel output.", active: None },
            Item::Lamp { m: 1, x: 570.0, y: 322.0, label: "PEAK", lo: 0.7, hi: 1.0 },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "TG Channel", class_id: *b"PFTGChannelPQD01", vendor: "PEQUOD Pink Floyd",
    categories: [Fx, Dynamics]
);
