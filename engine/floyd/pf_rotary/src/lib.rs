//! Rotary Speakers: the rotating speakers on Pink Floyd records: Rick Wright's Leslie 147 (treble horn and bass drum),
//! the Yamaha RA-200 David Gilmour used on Animals and The Wall (three full-range speakers on their own rotors), and the
//! Maestro Rover (one small rotating speaker). A valve amp, the rotors with their inertia, and two microphones.

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
pub enum Speed {
    #[name = "Slow"]
    Slow,
    #[name = "Fast"]
    Fast,
}

#[derive(Enum, PartialEq, Clone, Copy)]
pub enum Cabinet {
    #[name = "Leslie 147"]
    Leslie147,
    #[name = "Yamaha RA-200"]
    YamahaRa200,
    #[name = "Maestro Rover"]
    MaestroRover,
}

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "speed"]
    pub speed: EnumParam<Speed>,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "balance"]
    pub balance: FloatParam,
    #[id = "accel"]
    pub accel: FloatParam,
    #[id = "spread"]
    pub spread: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
    #[id = "cabinet"]
    pub cabinet: EnumParam<Cabinet>,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            speed: EnumParam::new("Speed", Speed::Slow),
            drive: db("Drive", 6.0, 0.0, 24.0),
            balance: pct("Horn/Drum", 0.55),
            accel: secs("Acceleration", 1.2, 0.2, 5.0),
            spread: pct("Mic Spread", 0.8),
            mix: pct("Mix", 1.0),
            output: db("Output", 0.0, -18.0, 6.0),
            cabinet: EnumParam::new("Cabinet", Cabinet::Leslie147),
        }
    }
}

pub const SPEED: usize = 0;
pub const DRIVE: usize = 1;
pub const BALANCE: usize = 2;
pub const ACCEL: usize = 3;
pub const SPREAD: usize = 4;
pub const MIX: usize = 5;
pub const OUTPUT: usize = 6;
pub const CABINET: usize = 7;
pub const NPARAMS: usize = 8;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.speed.as_ptr(),
            self.drive.as_ptr(),
            self.balance.as_ptr(),
            self.accel.as_ptr(),
            self.spread.as_ptr(),
            self.mix.as_ptr(),
            self.output.as_ptr(),
            self.cabinet.as_ptr(),
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

use std::f64::consts::{PI, TAU};

// rotor speeds in Hz (revolutions per second)
const HORN_SLOW: f64 = 0.83;
const HORN_FAST: f64 = 6.75;
const DRUM_SLOW: f64 = 0.67;
const DRUM_FAST: f64 = 5.9;
/// Horn mouth radius as a time: ~15 cm / speed of sound.
const HORN_DOPPLER_S: f64 = 0.00042;
const DRUM_DOPPLER_S: f64 = 0.00012;

pub struct Dsp {
    fs: f64,
    os: Oversampler,
    dry_delay: [FixedDelay; 2],
    xo_lp: [Biquad; 2],
    xo_hp: [Biquad; 2],
    horn_line: DelayLine,
    drum_line: DelayLine,
    horn_tone: [OnePole; 2],
    horn_speed: f64,
    drum_speed: f64,
    horn_phase: f64,
    drum_phase: f64,
    fast: bool,
    cab: i32,
    accel_s: f64,
    /// RA-200 / Rover: the three (or one) full-range rotors
    fr_line: DelayLine,
    fr_phase: [f64; 3],
    fr_speed: [f64; 3],
    fr_band: [Biquad; 2],
    drive: Smooth,
    balance: Smooth,
    spread: Smooth,
    mix: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 3;

    fn new(sample_rate: f32, _channels: usize) -> Self {
        let fs = sample_rate as f64;
        let mut xo_lp = [Biquad::default(); 2];
        let mut xo_hp = [Biquad::default(); 2];
        for b in &mut xo_lp {
            b.set_lowpass(fs, 800.0, 0.707);
        }
        for b in &mut xo_hp {
            b.set_highpass(fs, 800.0, 0.707);
        }
        Self {
            fs,
            os: Oversampler::new(),
            dry_delay: [FixedDelay::new(OS_LATENCY), FixedDelay::new(OS_LATENCY)],
            xo_lp,
            xo_hp,
            horn_line: DelayLine::new((fs * 0.01) as usize),
            drum_line: DelayLine::new((fs * 0.01) as usize),
            horn_tone: [OnePole::default(); 2],
            horn_speed: HORN_SLOW,
            drum_speed: DRUM_SLOW,
            horn_phase: 0.0,
            drum_phase: 0.3,
            fast: false,
            cab: 0,
            accel_s: 1.2,
            fr_line: DelayLine::new((fs * 0.01) as usize),
            fr_phase: [0.0, 0.33, 0.66],
            fr_speed: [0.8; 3],
            fr_band: {
                let mut lp = Biquad::default();
                lp.set_lowpass(fs, 6000.0, 0.707);
                let mut hp = Biquad::default();
                hp.set_highpass(fs, 120.0, 0.707);
                [lp, hp]
            },
            drive: Smooth::new(fs, 30.0, 2.0),
            balance: Smooth::new(fs, 30.0, 0.55),
            spread: Smooth::new(fs, 30.0, 0.8),
            mix: Smooth::new(fs, 20.0, 1.0),
            out: Smooth::new(fs, 20.0, 1.0),
        }
    }

    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }

    fn reset(&mut self) {
        self.os.reset();
        self.dry_delay.iter_mut().for_each(|d| d.reset());
        self.xo_lp.iter_mut().for_each(|b| b.reset());
        self.xo_hp.iter_mut().for_each(|b| b.reset());
        self.horn_line.reset();
        self.drum_line.reset();
        self.fr_line.reset();
        self.fr_band.iter_mut().for_each(|b| b.reset());
    }

    fn set_params(&mut self, v: &[f32]) {
        let v = |i: usize| v[i] as f64;
        self.fast = v(SPEED) > 0.5;
        self.cab = v(CABINET).round() as i32;
        self.accel_s = v(ACCEL);
        self.drive.set(db_to_gain(v(DRIVE)));
        self.balance.set(v(BALANCE));
        self.spread.set(v(SPREAD));
        self.mix.set(v(MIX));
        self.out.set(db_to_gain(v(OUTPUT)));
    }

    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let stereo = chans.len() >= 2;
        let fs = self.fs;
        // inertia: horn reaches its new speed in ~accel, the heavy drum ~3x slower
        let horn_c = coeff(fs, self.accel_s * 1000.0 / 3.0);
        let drum_c = coeff(fs, self.accel_s * 3000.0 / 3.0);
        let (ht, dt) = if self.fast { (HORN_FAST, DRUM_FAST) } else { (HORN_SLOW, DRUM_SLOW) };
        let mut speed_frac = 0.0;

        for k in 0..n {
            let (xl, xr) = if stereo { (chans[0][k] as f64, chans[1][k] as f64) } else { (chans[0][k] as f64, chans[0][k] as f64) };
            let drive = self.drive.next();
            let bal = self.balance.next();
            let spread = self.spread.next();
            let mix = self.mix.next();
            let o = self.out.next();

            // valve amp in the cabinet
            let mono = 0.5 * (xl + xr);
            let amp = self.os.process(mono, |v| tube(v * drive, 0.15) / drive.sqrt());

            self.horn_speed = ht + horn_c * (self.horn_speed - ht);
            self.drum_speed = dt + drum_c * (self.drum_speed - dt);
            self.horn_phase = (self.horn_phase + self.horn_speed / fs).fract();
            self.drum_phase = (self.drum_phase + self.drum_speed / fs).fract();

            let hi = self.xo_hp[0].process(amp);
            let hi = self.xo_hp[1].process(hi);
            let lo = self.xo_lp[0].process(amp);
            let lo = self.xo_lp[1].process(lo);
            self.horn_line.write(hi);
            self.drum_line.write(lo);

            // two mics, `spread` * 180 degrees apart
            let mic_angle = [-spread * PI / 2.0, spread * PI / 2.0];
            let mut out = [0.0f64; 2];
            if self.cab != 0 {
                // RA-200: three full-range speakers on their own rotors at slightly different speeds;
                // Rover: one small speaker. Neither has a horn/drum split.
                let nr = if self.cab == 1 { 3 } else { 1 };
                let (slow, fastv) = if self.cab == 1 { (1.1, 6.2) } else { (0.8, 5.5) };
                let t = if self.fast { fastv } else { slow };
                let fb = self.fr_band[0].process(amp);
                let fb = self.fr_band[1].process(fb);
                self.fr_line.write(fb);
                for r in 0..nr {
                    let tr = t * [1.0, 1.09, 0.93][r];
                    self.fr_speed[r] = tr + horn_c * (self.fr_speed[r] - tr);
                    self.fr_phase[r] = (self.fr_phase[r] + self.fr_speed[r] / fs).fract();
                }
                for m in 0..2 {
                    let mut acc = 0.0;
                    for r in 0..nr {
                        let ang = self.fr_phase[r] * TAU - mic_angle[m];
                        let d = (0.00025 * (1.0 + ang.sin())) * fs + 2.0;
                        let facing = 0.5 + 0.5 * ang.cos();
                        acc += self.fr_line.read(d) * (0.55 + 0.45 * facing);
                    }
                    out[m] = acc / nr as f64 * 1.6;
                }
            }
            for m in 0..(if self.cab == 0 { 2 } else { 0 }) {
                let hp_ang = self.horn_phase * TAU - mic_angle[m];
                let dp_ang = self.drum_phase * TAU - mic_angle[m];
                // Doppler: path length changes as the horn swings toward/away
                let hd = (HORN_DOPPLER_S * (1.0 + hp_ang.sin())) * fs + 2.0;
                let dd = (DRUM_DOPPLER_S * (1.0 + dp_ang.sin())) * fs + 2.0;
                let facing = 0.5 + 0.5 * hp_ang.cos(); // 1 = horn pointing at the mic
                let horn_amp = 0.45 + 0.55 * facing;
                let drum_amp = 0.72 + 0.28 * dp_ang.cos();
                // horn sounds duller when facing away
                self.horn_tone[m].set_lowpass(fs, 2500.0 + 9000.0 * facing);
                let h = self.horn_tone[m].lp(self.horn_line.read(hd)) * horn_amp;
                let d = self.drum_line.read(dd) * drum_amp;
                out[m] = (h * bal + d * (1.0 - bal)) * 1.6;
            }
            let dl = self.dry_delay[0].process(xl);
            let dr = self.dry_delay[1].process(xr);
            let l = (dl + mix * (out[0] - dl)) * o;
            let r = (dr + mix * (out[1] - dr)) * o;
            if stereo {
                chans[0][k] = l as f32;
                chans[1][k] = r as f32;
            } else {
                chans[0][k] = (0.5 * (l + r)) as f32;
            }
        }
        speed_frac += (self.horn_speed - HORN_SLOW) / (HORN_FAST - HORN_SLOW);
        let (h, d) = if self.cab == 0 { (self.horn_phase, self.drum_phase) } else { (self.fr_phase[0], self.fr_phase[1]) };
        meters[0] = (h as f32).max(1e-3);
        meters[1] = (d as f32).max(1e-3);
        meters[2] = speed_frac.clamp(0.0, 1.0) as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Leslie: Wright's Organ (slow)", values: &[(DRIVE, 8.0)] },
        Preset { name: "Leslie: Organ Fast, Driven", values: &[(SPEED, 1.0), (DRIVE, 16.0), (BALANCE, 0.5), (ACCEL, 1.6)] },
        Preset { name: "Leslie: Dark Side Guitar (low in mix)", values: &[(DRIVE, 3.0), (BALANCE, 0.7), (MIX, 0.35)] },
        Preset { name: "RA-200: Animals / Wall Shimmer", values: &[(DRIVE, 2.0), (MIX, 0.3), (CABINET, 1.0)] },
        Preset { name: "RA-200: Solo Swirl (mixed high)", values: &[(DRIVE, 4.0), (MIX, 0.6), (CABINET, 1.0)] },
        Preset { name: "RA-200: Fast", values: &[(SPEED, 1.0), (DRIVE, 3.0), (MIX, 0.5), (CABINET, 1.0)] },
        Preset { name: "Rover: Slow Rotation", values: &[(DRIVE, 2.0), (MIX, 0.4), (CABINET, 2.0)] },
    ]
}

/*
('knob', 319.5, 151.5, 9.8)
('selector?', 69.5, 159.5, 36, 36)
('knob', 179.5, 171.5, 41.5)
('knob', 479.5, 171.5, 41.5)
('divider', 284)
('knob', 139.5, 331.5, 22.0)
('knob', 259.5, 331.5, 22.0)
('knob', 379.5, 331.5, 22.0)
('knob', 499.5, 331.5, 26.8)

crate: ['abinetfloyd/pf_rotary', 'floyd/pf_rotary', 'library/panic_abort']
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
LESLIE 147  
  YAMAHA RA-200MAESTRO ROVERLESLIERA-200ROVER
CABINETLESLIE 147: treble horn and bass drum. RA-200: three full-range speakers on their own rotors at slightly different speeds (a lush chorus). ROVER: one small rotating speaker.DRIVEThe cabinet's valve amp. Push it for the growl of an overdriven rotating speaker.HORN / DRUMLeslie only: balance between the spinning treble horn and the bass drum. More horn = more shimmer and Doppler wobble.FASTSLOWChorale (slow) or tremolo (fast). The rotors take a moment to get there, so switching mid-phrase gives the classic speed-up swirl.ACCELHow long the horn takes to change speed (the drum takes about three times longer).MIC SPREADAngle between the two microphones: 0 = mono, 100% = opposite sides of the cabinet.MIXBlend with the dry signal.OUTPUTFinal level.ROTARY SPEAKERS
Leslie: Wright's Organ (slow)
Leslie: Organ Fast, Driven
333?
Leslie: Dark Side Guitar (low in mix)
RA-200: Animals / Wall Shimmer
RA-200: Solo Swirl (mixed high)
RA-200: Fast
Rover: Slow Rotation$min > max, or either was NaN. min = 

*/
fn panel() -> Panel {
    Panel {
        title: "ROTARY SPEAKERS",
        subtitle: ("LESLIE 147  ·  YAMAHA RA-200", "MAESTRO ROVER"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![
            Item::Selector { p: CABINET, x: 70.0, y: 160.0, label: "CABINET", positions: &["LESLIE", "RA-200", "ROVER"], tip: "LESLIE 147: treble horn and bass drum. RA-200: three full-range speakers on their own rotors at slightly different speeds (a lush chorus). ROVER: one small rotating speaker." },
            Item::Knob { p: DRIVE, x: 180.0, y: 172.0, r: 42.0, label: "DRIVE", tip: "The cabinet's valve amp. Push it for the growl of an overdriven rotating speaker.", active: None },
            Item::Knob { p: BALANCE, x: 480.0, y: 172.0, r: 42.0, label: "HORN / DRUM", tip: "Leslie only: balance between the spinning treble horn and the bass drum. More horn = more shimmer and Doppler wobble.", active: None },
            Item::Rotor { horn: 0, drum: 1, x: 320.0, y: 152.0, r: 56.0 },
            Item::Lamp { m: 2, x: 236.0, y: 254.0, label: "FAST", lo: 0.05, hi: 0.95 },
            Item::Switch { p: SPEED, x: 376.0, y: 254.0, labels: ["SLOW", "FAST"], tip: "Chorale (slow) or tremolo (fast). The rotors take a moment to get there, so switching mid-phrase gives the classic speed-up swirl." },
            Item::Divider { y: 284.0 },
            Item::Knob { p: ACCEL, x: 140.0, y: 332.0, r: 22.0, label: "ACCEL", tip: "How long the horn takes to change speed (the drum takes about three times longer).", active: None },
            Item::Knob { p: SPREAD, x: 260.0, y: 332.0, r: 22.0, label: "MIC SPREAD", tip: "Angle between the two microphones: 0 = mono, 100% = opposite sides of the cabinet.", active: None },
            Item::Knob { p: MIX, x: 380.0, y: 332.0, r: 22.0, label: "MIX", tip: "Blend with the dry signal.", active: None },
            Item::Knob { p: OUTPUT, x: 500.0, y: 332.0, r: 22.0, label: "OUTPUT", tip: "Final level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Rotary Speakers", class_id: *b"PFRotaryPQD00001", vendor: "PEQUOD Pink Floyd",
    categories: [Fx, Modulation]
);
