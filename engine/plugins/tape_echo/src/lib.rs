//! Tape Echo — tape slapback and feedback echo.
//!
//! A record head writes to a loop of tape and a playback head reads it back
//! a moment later. Wow and flutter make the delay time wobble, each trip
//! round the loop loses top and bottom end, and the tape saturates as the
//! repeats build up. Changing Time while echoes ring out makes them bend in
//! pitch, as on a real machine.

use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
use vintage::dsp::*;
use vintage::nih_plug_egui::EguiState;
use vintage::params::*;
use vintage::ui::{self, Preset, Item, MeterKind, Panel};
use vintage::{VintageDsp, VintageParams};

const W: u32 = 640;
const H: u32 = 432;

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Heads {
    #[name = "Mono"]
    Mono,
    #[name = "Stereo"]
    Stereo,
}

#[derive(Params)]
pub struct EchoParams {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "time"]
    time: FloatParam,
    #[id = "repeats"]
    repeats: FloatParam,
    #[id = "wobble"]
    wobble: FloatParam,
    #[id = "tone"]
    tone: FloatParam,
    #[id = "lowcut"]
    low_cut: FloatParam,
    #[id = "drive"]
    drive: FloatParam,
    #[id = "mix"]
    mix: FloatParam,
    #[id = "output"]
    output: FloatParam,
    #[id = "heads"]
    heads: EnumParam<Heads>,
}

impl Default for EchoParams {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            time: ms("Time", 110.0, 40.0, 1200.0),
            repeats: FloatParam::new("Repeats", 0.2, FloatRange::Linear { min: 0.0, max: 1.1 })
                .with_unit("%")
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage()),
            wobble: pct("Wobble", 0.2),
            tone: hz("Tone", 4500.0, 1000.0, 12000.0),
            low_cut: hz("Low Cut", 120.0, 40.0, 600.0),
            drive: db("Tape Drive", 6.0, 0.0, 18.0),
            mix: pct("Mix", 0.35),
            output: db("Output", 0.0, -18.0, 6.0),
            heads: EnumParam::new("Heads", Heads::Mono),
        }
    }
}

const TIME: usize = 0;
const REPEATS: usize = 1;
const WOBBLE: usize = 2;
const TONE: usize = 3;
const LOWCUT: usize = 4;
const DRIVE: usize = 5;
const MIX: usize = 6;
const OUTPUT: usize = 7;
const HEADS: usize = 8;

impl VintageParams for EchoParams {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.time.as_ptr(),
            self.repeats.as_ptr(),
            self.wobble.as_ptr(),
            self.tone.as_ptr(),
            self.low_cut.as_ptr(),
            self.drive.as_ptr(),
            self.mix.as_ptr(),
            self.output.as_ptr(),
            self.heads.as_ptr(),
        ]
    }
}

struct Head {
    line: DelayLine,
    lp: Biquad,
    hp: OnePole,
    dc: DcBlock,
}

pub struct EchoDsp {
    fs: f64,
    heads: [Head; 2],
    time: Smooth,
    fb: Smooth,
    wob: Smooth,
    drive: Smooth,
    mix: Smooth,
    out: Smooth,
    wow: Wander,
    wow_lfo: Lfo,
    flutter: Lfo,
    stereo: bool,
}

impl VintageDsp for EchoDsp {
    const METERS: usize = 2;

    fn new(sample_rate: f32, _channels: usize) -> Self {
        let fs = sample_rate as f64;
        let mk = || Head {
            line: DelayLine::new((fs * 1.2 * 1.6) as usize + 16),
            lp: Biquad::identity(),
            hp: OnePole::default(),
            dc: DcBlock::new(fs),
        };
        Self {
            fs,
            heads: [mk(), mk()],
            // slow glide on time changes = pitch bend, like moving the heads
            time: Smooth::new(fs, 180.0, 110.0),
            fb: Smooth::new(fs, 30.0, 0.2),
            wob: Smooth::new(fs, 30.0, 0.2),
            drive: Smooth::new(fs, 30.0, 2.0),
            mix: Smooth::new(fs, 20.0, 0.35),
            out: Smooth::new(fs, 20.0, 1.0),
            wow: Wander::new(fs, 0.7, 5),
            wow_lfo: Lfo::default(),
            flutter: Lfo::default(),
            stereo: false,
        }
    }

    fn reset(&mut self) {
        for h in &mut self.heads {
            h.line.reset();
            h.lp.reset();
            h.hp.reset();
            h.dc.reset();
        }
    }

    fn set_params(&mut self, v: &[f32]) {
        let v = |i: usize| v[i] as f64;
        self.time.set(v(TIME));
        self.fb.set(v(REPEATS));
        self.wob.set(v(WOBBLE));
        self.drive.set(db_to_gain(v(DRIVE)));
        self.mix.set(v(MIX));
        self.out.set(db_to_gain(v(OUTPUT)));
        self.stereo = v(HEADS) > 0.5;
        for h in &mut self.heads {
            h.lp.set_lowpass(self.fs, v(TONE), 0.6);
            h.hp.set_lowpass(self.fs, v(LOWCUT));
        }
    }

    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let stereo_io = chans.len() >= 2;
        let mut peak = 0.0f64;
        let mut loop_peak = 0.0f64;
        for k in 0..n {
            let (xl, xr) = if stereo_io { (chans[0][k] as f64, chans[1][k] as f64) } else { (chans[0][k] as f64, chans[0][k] as f64) };
            let t = self.time.next();
            let fb = self.fb.next();
            let wob = self.wob.next();
            let drive = self.drive.next();
            let mix = self.mix.next();
            let o = self.out.next();

            // wow (slow, random + periodic) and flutter (fast, small)
            let wow = 0.7 * self.wow.next() + 0.3 * self.wow_lfo.next(self.fs, 0.9);
            let flut = self.flutter.next(self.fs, 7.3);
            let speed_err = wob * (0.006 * wow + 0.0012 * flut);
            let base = t * 0.001 * self.fs * (1.0 + speed_err);

            let mut wet = [0.0f64; 2];
            for (c, h) in self.heads.iter_mut().enumerate() {
                // second head: 1.5x the time on the right in STEREO mode
                let d = if self.stereo && c == 1 { base * 1.5 } else { base };
                let y = h.line.read(d);
                // each pass: lose top and bottom, saturate on the way back to tape
                let shaped = h.lp.process(y);
                let shaped = shaped - h.hp.lp(shaped);
                let sat = tape_curve(shaped * drive) / drive;
                loop_peak = loop_peak.max((shaped * drive).abs());
                let input = if c == 0 { xl } else { xr };
                let send = if self.stereo { 0.5 * (xl + xr) } else { input };
                h.line.write(h.dc.process(send + sat * fb));
                wet[c] = y;
            }
            // dry stays full until 50%, then fades
            let dry_g = (2.0 * (1.0 - mix)).min(1.0);
            let wet_g = (2.0 * mix).min(1.0);
            let l = (xl * dry_g + wet[0] * wet_g) * o;
            let r = (xr * dry_g + wet[1] * wet_g) * o;
            if stereo_io {
                chans[0][k] = l as f32;
                chans[1][k] = r as f32;
            } else {
                chans[0][k] = (0.5 * (l + r)) as f32;
            }
            peak = peak.max(l.abs()).max(r.abs());
        }
        meters[0] = peak as f32;
        meters[1] = loop_peak as f32;
    }
}

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Rock'n'Roll Slapback", values: &[(TIME, 110.0), (REPEATS, 0.12), (MIX, 0.35)] },
        Preset { name: "Rockabilly Double Slap", values: &[(TIME, 140.0), (REPEATS, 0.3), (MIX, 0.4), (TONE, 5500.0)] },
        Preset { name: "Vocal Echo", values: &[(TIME, 320.0), (REPEATS, 0.35), (MIX, 0.25), (TONE, 3800.0), (LOWCUT, 200.0)] },
        Preset { name: "Dark Long Repeats", values: &[(TIME, 650.0), (REPEATS, 0.6), (TONE, 2200.0), (WOBBLE, 0.35), (MIX, 0.3)] },
        Preset { name: "Stereo Bounce", values: &[(HEADS, 1.0), (TIME, 260.0), (REPEATS, 0.45), (MIX, 0.35)] },
        Preset { name: "Runaway Oscillation", values: &[(TIME, 380.0), (REPEATS, 1.05), (DRIVE, 12.0), (WOBBLE, 0.4), (MIX, 0.5)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "TAPE ECHO",
        subtitle: ("TAPE DELAY &", "SLAPBACK UNIT"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![
            Item::Knob { p: TIME, x: 130.0, y: 172.0, r: 50.0, label: "TIME",
                tip: "Gap between the record and playback heads. 80-140 ms is classic slapback; longer for proper echoes. Moving it while echoes ring makes them bend in pitch.", active: None },
            Item::Knob { p: REPEATS, x: 510.0, y: 172.0, r: 50.0, label: "REPEATS",
                tip: "How much of the echo goes back onto the tape. Above 100% it runs away into self-oscillation (the tape saturation keeps it from exploding).", active: None },
            Item::Meter { m: 0, x: 222.0, y: 92.0, w: 196.0, h: 136.0, kind: MeterKind::Vu, caption: "OUTPUT" },
            Item::Lamp { m: 1, x: 236.0, y: 254.0, label: "SATURATE", lo: 0.6, hi: 1.6 },
            Item::Switch { p: HEADS, x: 380.0, y: 254.0, labels: ["MONO", "STEREO"],
                tip: "MONO: one head per channel.\nSTEREO: the right side uses a second head at 1.5x the time, for a wide bouncing echo." },
            Item::Divider { y: 284.0 },
            Item::Knob { p: WOBBLE, x: 120.0, y: 332.0, r: 22.0, label: "WOBBLE", tip: "Wow and flutter: how much the tape speed wavers.", active: None },
            Item::Knob { p: TONE, x: 200.0, y: 332.0, r: 22.0, label: "TONE", tip: "Top end kept on each repeat. Lower = darker, older tape.", active: None },
            Item::Knob { p: LOWCUT, x: 280.0, y: 332.0, r: 22.0, label: "LOW CUT", tip: "Bass removed on each repeat, keeps the echoes from muddying the mix.", active: None },
            Item::Knob { p: DRIVE, x: 360.0, y: 332.0, r: 22.0, label: "DRIVE", tip: "How hard the repeats hit the tape. More = grittier, more compressed echoes.", active: None },
            Item::Knob { p: MIX, x: 440.0, y: 332.0, r: 22.0, label: "MIX", tip: "Echo amount. Up to 50% the dry signal stays at full level.", active: None },
            Item::Knob { p: OUTPUT, x: 520.0, y: 332.0, r: 22.0, label: "OUTPUT", tip: "Final level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, EchoParams, EchoDsp, panel,
    name: "Tape Echo", class_id: *b"S2TapeEchoDIY001", vendor: "PEQUOD Beatles",
    categories: [Fx, Delay]
);

#[cfg(test)]
mod tests {
    use super::*;

    fn impulse_response(params: &[f32], secs: f64) -> Vec<f32> {
        let fs = 48000.0;
        let mut d = EchoDsp::new(fs as f32, 1);
        d.set_params(params);
        // let smoothers settle with silence
        let mut warm = vec![0.0f32; 48000];
        let mut m = [0.0f32; 2];
        d.process(&mut [&mut warm[..]], &mut m);
        let n = (fs * secs) as usize;
        let mut x = vec![0.0f32; n];
        x[0] = 1.0;
        d.process(&mut [&mut x[..]], &mut m);
        x
    }

    const DEFAULTS: [f32; 9] = [110.0, 0.2, 0.0, 4500.0, 120.0, 6.0, 0.5, 0.0, 0.0];

    #[test]
    fn first_echo_at_time() {
        let x = impulse_response(&DEFAULTS, 0.5);
        let first = x.iter().enumerate().skip(100).max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap()).unwrap().0;
        let ms = first as f64 / 48.0;
        println!("first echo at {ms:.1} ms");
        assert!((105.0..118.0).contains(&ms));
    }

    #[test]
    fn repeats_decay_and_runaway_is_bounded() {
        let x = impulse_response(&DEFAULTS, 2.0);
        let early: f32 = x[4800..12000].iter().map(|v| v.abs()).fold(0.0, f32::max);
        let late: f32 = x[80000..96000].iter().map(|v| v.abs()).fold(0.0, f32::max);
        assert!(late < early * 0.2, "echoes should die away: {early} {late}");
        let mut p = DEFAULTS;
        p[REPEATS] = 1.1;
        let y = impulse_response(&p, 6.0);
        assert!(y.iter().all(|v| v.is_finite() && v.abs() < 4.0));
    }
}
