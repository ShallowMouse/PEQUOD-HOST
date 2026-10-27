//! Rotor — rotating speaker cabinet.
//!
//! The signal is driven through a valve amp, split at 800 Hz, and sent to a
//! spinning horn (treble) and a spinning drum baffle (bass). Two microphones
//! hear each rotor coming and going: the horn's movement gives Doppler pitch
//! wobble plus loud/soft and bright/dull sweeps; the drum mostly throbs. The
//! rotors speed up and slow down with real inertia — the horn is light and
//! gets there quickly, the drum is heavy and lags behind.

use nih_plug::prelude::*;
use std::f64::consts::{PI, TAU};
use std::sync::{Arc, RwLock};
use vintage::dsp::*;
use vintage::nih_plug_egui::EguiState;
use vintage::params::*;
use vintage::ui::{self, Preset, Item, Panel};
use vintage::{VintageDsp, VintageParams};

const W: u32 = 640;
const H: u32 = 432;

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Speed {
    #[name = "Slow"]
    Slow,
    #[name = "Fast"]
    Fast,
}

#[derive(Params)]
pub struct RotorParams {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "speed"]
    speed: EnumParam<Speed>,
    #[id = "drive"]
    drive: FloatParam,
    #[id = "balance"]
    balance: FloatParam,
    #[id = "accel"]
    accel: FloatParam,
    #[id = "spread"]
    spread: FloatParam,
    #[id = "mix"]
    mix: FloatParam,
    #[id = "output"]
    output: FloatParam,
}

impl Default for RotorParams {
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
        }
    }
}

const SPEED: usize = 0;
const DRIVE: usize = 1;
const BALANCE: usize = 2;
const ACCEL: usize = 3;
const SPREAD: usize = 4;
const MIX: usize = 5;
const OUTPUT: usize = 6;

impl VintageParams for RotorParams {
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
        ]
    }
}

// rotor speeds in Hz (revolutions per second)
const HORN_SLOW: f64 = 0.83;
const HORN_FAST: f64 = 6.75;
const DRUM_SLOW: f64 = 0.67;
const DRUM_FAST: f64 = 5.9;
/// Horn mouth radius as a time: ~15 cm / speed of sound.
const HORN_DOPPLER_S: f64 = 0.00042;
const DRUM_DOPPLER_S: f64 = 0.00012;

pub struct RotorDsp {
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
    accel_s: f64,
    drive: Smooth,
    balance: Smooth,
    spread: Smooth,
    mix: Smooth,
    out: Smooth,
}

impl VintageDsp for RotorDsp {
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
            accel_s: 1.2,
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
    }

    fn set_params(&mut self, v: &[f32]) {
        let v = |i: usize| v[i] as f64;
        self.fast = v(SPEED) > 0.5;
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
            for m in 0..2 {
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
        meters[0] = (self.horn_phase as f32).max(1e-3);
        meters[1] = (self.drum_phase as f32).max(1e-3);
        meters[2] = speed_frac.clamp(0.0, 1.0) as f32;
    }
}

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Chorale Vocal", values: &[(SPEED, 0.0), (DRIVE, 3.0), (BALANCE, 0.65), (MIX, 0.8)] },
        Preset { name: "Tremolo Guitar", values: &[(SPEED, 1.0), (DRIVE, 6.0), (BALANCE, 0.6)] },
        Preset { name: "Overdriven Organ", values: &[(SPEED, 1.0), (DRIVE, 18.0), (BALANCE, 0.5), (ACCEL, 1.6)] },
        Preset { name: "Swirling Vocal (Tomorrow style)", values: &[(SPEED, 0.0), (DRIVE, 9.0), (BALANCE, 0.8), (SPREAD, 1.0), (MIX, 1.0)] },
        Preset { name: "Subtle Swirl", values: &[(SPEED, 0.0), (DRIVE, 1.0), (MIX, 0.35)] },
        Preset { name: "Slow Spin-Up", values: &[(SPEED, 0.0), (ACCEL, 4.5), (DRIVE, 6.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "ROTOR",
        subtitle: ("ROTATING SPEAKER", "CABINET"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![
            Item::Knob { p: DRIVE, x: 130.0, y: 172.0, r: 50.0, label: "DRIVE",
                tip: "The cabinet's valve amp. Push it for the growl of an overdriven rotating speaker.", active: None },
            Item::Knob { p: BALANCE, x: 510.0, y: 172.0, r: 50.0, label: "HORN / DRUM",
                tip: "Balance between the spinning treble horn and the bass drum. More horn = more shimmer and Doppler wobble.", active: None },
            Item::Rotor { horn: 0, drum: 1, x: 320.0, y: 152.0, r: 56.0 },
            Item::Lamp { m: 2, x: 236.0, y: 254.0, label: "FAST", lo: 0.05, hi: 0.95 },
            Item::Switch { p: SPEED, x: 376.0, y: 254.0, labels: ["SLOW", "FAST"],
                tip: "Chorale (slow) or tremolo (fast). The rotors take a moment to get there, so switching mid-phrase gives the classic speed-up swirl." },
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
    Plugin, RotorParams, RotorDsp, panel,
    name: "Rotor", class_id: *b"S2RotorDIY000001", vendor: "PEQUOD Beatles",
    categories: [Fx, Modulation]
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spins_up_and_modulates() {
        let fs = 48000.0;
        let mut d = RotorDsp::new(fs as f32, 2);
        let mut p = [0.0f32, 6.0, 0.55, 1.2, 0.8, 1.0, 0.0];
        d.set_params(&p);
        p[SPEED] = 1.0;
        d.set_params(&p);
        let n = 48000 * 4;
        let mut l: Vec<f32> = (0..n).map(|k| (0.3 * (2.0 * PI * 2000.0 * k as f64 / fs).sin()) as f32).collect();
        let mut r = l.clone();
        let mut m = [0.0f32; 3];
        d.process(&mut [&mut l[..], &mut r[..]], &mut m);
        assert!(m[2] > 0.9, "horn should be at fast speed after 4 s: {}", m[2]);
        // amplitude of the horn signal should be modulated (tremolo)
        let seg = &l[n - 48000..];
        let env: Vec<f32> = seg.chunks(240).map(|c| c.iter().map(|v| v.abs()).fold(0.0, f32::max)).collect();
        let (mn, mx) = env.iter().fold((f32::MAX, 0.0f32), |(a, b), v| (a.min(*v), b.max(*v)));
        println!("envelope {mn:.3}..{mx:.3}");
        assert!(mx > mn * 1.4);
        assert!(l.iter().chain(r.iter()).all(|v| v.is_finite()));
    }
}
