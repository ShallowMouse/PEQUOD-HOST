//! Echo Chamber — reverb in the style of a studio echo chamber or plate.
//!
//! CHAMBER: a tiled room fed by a speaker and picked up by microphones, with
//! distinct early reflections and a darker tail.
//! PLATE: a steel plate — no early reflections, instantly dense and bright.
//!
//! Built from an 8-line feedback delay network with input diffusion, per-line
//! damping, and slight modulation to keep the tail from ringing metallic.

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
pub enum Space {
    #[name = "Chamber"]
    Chamber,
    #[name = "Plate"]
    Plate,
}

#[derive(Params)]
pub struct ChamberParams {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "decay"]
    decay: FloatParam,
    #[id = "mix"]
    mix: FloatParam,
    #[id = "predelay"]
    predelay: FloatParam,
    #[id = "size"]
    size: FloatParam,
    #[id = "damping"]
    damping: FloatParam,
    #[id = "lowcut"]
    low_cut: FloatParam,
    #[id = "width"]
    width: FloatParam,
    #[id = "output"]
    output: FloatParam,
    #[id = "space"]
    space: EnumParam<Space>,
}

impl Default for ChamberParams {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            decay: secs("Decay", 1.8, 0.3, 8.0),
            mix: pct("Mix", 0.25),
            predelay: ms("Pre-delay", 20.0, 0.0, 200.0),
            size: pct("Size", 0.6),
            damping: hz("Damping", 6000.0, 1500.0, 16000.0),
            low_cut: hz("Low Cut", 150.0, 40.0, 800.0),
            width: pct("Width", 0.8),
            output: db("Output", 0.0, -18.0, 6.0),
            space: EnumParam::new("Space", Space::Chamber),
        }
    }
}

const DECAY: usize = 0;
const MIX: usize = 1;
const PREDELAY: usize = 2;
const SIZE: usize = 3;
const DAMPING: usize = 4;
const LOWCUT: usize = 5;
const WIDTH: usize = 6;
const OUTPUT: usize = 7;
const SPACE: usize = 8;

impl VintageParams for ChamberParams {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.decay.as_ptr(),
            self.mix.as_ptr(),
            self.predelay.as_ptr(),
            self.size.as_ptr(),
            self.damping.as_ptr(),
            self.low_cut.as_ptr(),
            self.width.as_ptr(),
            self.output.as_ptr(),
            self.space.as_ptr(),
        ]
    }
}

const LINES: usize = 8;
/// Base FDN line lengths in ms at Size = 100% (mutually prime-ish).
const BASE_MS: [f64; LINES] = [37.1, 43.7, 51.3, 59.9, 67.1, 73.3, 79.7, 89.1];
const DIFF_MS: [f64; 4] = [4.7, 3.6, 12.7, 9.3];
/// Early reflections for CHAMBER: (ms, gain, left/right)
const EARLY: [(f64, f64, usize); 8] = [
    (7.3, 0.62, 0), (11.9, 0.55, 1), (17.1, 0.48, 0), (21.7, 0.44, 1),
    (29.3, 0.36, 0), (33.1, 0.33, 1), (41.9, 0.26, 0), (47.3, 0.24, 1),
];

struct Allpass {
    line: DelayLine,
    len: usize,
    g: f64,
}

impl Allpass {
    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let d = self.line.read_int(self.len);
        let v = x + self.g * d;
        self.line.write(v);
        d - self.g * v
    }
}

pub struct ChamberDsp {
    fs: f64,
    pre: DelayLine,
    early: DelayLine,
    diff: Vec<Allpass>,
    lines: Vec<DelayLine>,
    damp: [OnePole; LINES],
    gains: [f64; LINES],
    lens: [f64; LINES],
    lfo: [Lfo; 2],
    in_hp: OnePole,
    out_hp: [OnePole; 2],
    predelay: Smooth,
    mix: Smooth,
    width: Smooth,
    out: Smooth,
    plate: bool,
    last: Vec<f32>,
}

impl ChamberDsp {
    fn rebuild(&mut self, v: &[f32]) {
        let fs = self.fs;
        let size = 0.35 + 0.65 * v[SIZE] as f64;
        let plate = v[SPACE] > 0.5;
        // plates are smaller and denser
        let scale = if plate { size * 0.7 } else { size };
        let t60 = v[DECAY] as f64;
        let damp_hz = v[DAMPING] as f64 * if plate { 1.4 } else { 0.85 };
        for i in 0..LINES {
            self.lens[i] = BASE_MS[i] * scale * 0.001 * fs;
            // gain for -60 dB after t60 seconds
            self.gains[i] = 10f64.powf(-3.0 * (self.lens[i] / fs) / t60);
            self.damp[i].set_lowpass(fs, damp_hz);
        }
        let dg = if plate { 0.72 } else { 0.6 };
        for (k, ap) in self.diff.iter_mut().enumerate() {
            ap.len = ((DIFF_MS[k] * if plate { 1.0 } else { size }) * 0.001 * fs) as usize;
            ap.g = dg;
        }
        self.in_hp.set_lowpass(fs, v[LOWCUT] as f64);
        for h in &mut self.out_hp {
            h.set_lowpass(fs, v[LOWCUT] as f64 * 0.7);
        }
        self.plate = plate;
    }
}

impl VintageDsp for ChamberDsp {
    const METERS: usize = 1;

    fn new(sample_rate: f32, _channels: usize) -> Self {
        let fs = sample_rate as f64;
        let max_line = (BASE_MS[LINES - 1] * 1.1 * 0.001 * fs) as usize + 64;
        let mut s = Self {
            fs,
            pre: DelayLine::new((0.21 * fs) as usize),
            early: DelayLine::new((0.06 * fs) as usize),
            diff: DIFF_MS
                .iter()
                .map(|ms| Allpass { line: DelayLine::new((ms * 1.1 * 0.001 * fs) as usize + 8), len: 1, g: 0.6 })
                .collect(),
            lines: (0..LINES).map(|_| DelayLine::new(max_line)).collect(),
            damp: [OnePole::default(); LINES],
            gains: [0.0; LINES],
            lens: [1.0; LINES],
            lfo: [Lfo::default(), Lfo { phase: 0.5 }],
            in_hp: OnePole::default(),
            out_hp: [OnePole::default(); 2],
            predelay: Smooth::new(fs, 100.0, 20.0),
            mix: Smooth::new(fs, 20.0, 0.25),
            width: Smooth::new(fs, 20.0, 0.8),
            out: Smooth::new(fs, 20.0, 1.0),
            plate: false,
            last: Vec::new(),
        };
        s.rebuild(&[1.8, 0.25, 20.0, 0.6, 6000.0, 150.0, 0.8, 0.0, 0.0]);
        s
    }

    fn reset(&mut self) {
        self.pre.reset();
        self.early.reset();
        for a in &mut self.diff {
            a.line.reset();
        }
        for l in &mut self.lines {
            l.reset();
        }
        self.damp.iter_mut().for_each(|d| d.reset());
    }

    fn set_params(&mut self, v: &[f32]) {
        self.predelay.set(v[PREDELAY] as f64);
        self.mix.set(v[MIX] as f64);
        self.width.set(v[WIDTH] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
        let key: Vec<f32> = [DECAY, SIZE, DAMPING, LOWCUT, SPACE].iter().map(|i| v[*i]).collect();
        if self.last != key {
            self.rebuild(v);
            self.last = key;
        }
    }

    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let stereo = chans.len() >= 2;
        let fs = self.fs;
        let mut peak = 0.0f64;
        for k in 0..n {
            let (xl, xr) = if stereo { (chans[0][k] as f64, chans[1][k] as f64) } else { (chans[0][k] as f64, chans[0][k] as f64) };
            let pd = self.predelay.next();
            let mix = self.mix.next();
            let width = self.width.next();
            let o = self.out.next();

            let mono = 0.5 * (xl + xr);
            let x = self.in_hp.hp(mono);
            self.pre.write(x);
            let mut s = self.pre.read((pd * 0.001 * fs).max(1.0));

            // early reflections (chamber only)
            let mut er = [0.0f64; 2];
            if !self.plate {
                self.early.write(s);
                for (ms, g, side) in EARLY {
                    er[side] += self.early.read(ms * 0.001 * fs) * g;
                }
            }
            for ap in &mut self.diff {
                s = ap.process(s);
            }

            // FDN: read, damp, Householder mix, write back
            let mut taps = [0.0f64; LINES];
            let m0 = self.lfo[0].next(fs, 0.31) * 0.0004 * fs;
            let m1 = self.lfo[1].next(fs, 0.47) * 0.0004 * fs;
            for i in 0..LINES {
                let modv = match i {
                    2 => m0,
                    5 => m1,
                    _ => 0.0,
                };
                taps[i] = self.damp[i].lp(self.lines[i].read(self.lens[i] + modv)) * self.gains[i];
            }
            let sum: f64 = taps.iter().sum::<f64>() * (2.0 / LINES as f64);
            for i in 0..LINES {
                let fb = taps[i] - sum;
                let inject = if i % 2 == 0 { s } else { -s };
                self.lines[i].write(fb + inject * 0.5);
            }
            let wl = taps[0] + taps[2] + taps[4] + taps[6];
            let wr = taps[1] + taps[3] + taps[5] + taps[7];
            let mut l = 0.6 * wl + er[0] * 0.8;
            let mut r = 0.6 * wr + er[1] * 0.8;
            l = self.out_hp[0].hp(l);
            r = self.out_hp[1].hp(r);
            // width: mid/side
            let mid = 0.5 * (l + r);
            let side = 0.5 * (l - r) * width;
            let (wl, wr) = (mid + side, mid - side);

            let dry_g = (2.0 * (1.0 - mix)).min(1.0);
            let wet_g = (2.0 * mix).min(1.0);
            let ol = (xl * dry_g + wl * wet_g) * o;
            let or = (xr * dry_g + wr * wet_g) * o;
            peak = peak.max(ol.abs()).max(or.abs());
            if stereo {
                chans[0][k] = ol as f32;
                chans[1][k] = or as f32;
            } else {
                chans[0][k] = (0.5 * (ol + or)) as f32;
            }
        }
        meters[0] = peak as f32;
    }
}

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Studio Chamber", values: &[] },
        Preset { name: "Small Chamber", values: &[(DECAY, 0.9), (SIZE, 0.3), (PREDELAY, 8.0), (MIX, 0.2)] },
        Preset { name: "Big Chamber", values: &[(DECAY, 3.2), (SIZE, 0.95), (PREDELAY, 35.0), (MIX, 0.3), (DAMPING, 4500.0)] },
        Preset { name: "Bright Plate", values: &[(SPACE, 1.0), (DECAY, 2.0), (DAMPING, 12000.0), (LOWCUT, 200.0), (MIX, 0.25)] },
        Preset { name: "Vocal Plate", values: &[(SPACE, 1.0), (DECAY, 1.6), (PREDELAY, 60.0), (DAMPING, 9000.0), (MIX, 0.22)] },
        Preset { name: "Long Plate Wash", values: &[(SPACE, 1.0), (DECAY, 6.0), (DAMPING, 7000.0), (WIDTH, 1.0), (MIX, 0.4)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "ECHO CHAMBER",
        subtitle: ("STUDIO CHAMBER", "& PLATE REVERB"),
        accent: ui::PINE,
        size: (W, H),
        items: vec![
            Item::Knob { p: DECAY, x: 130.0, y: 172.0, r: 50.0, label: "DECAY", tip: "Reverb time (to -60 dB).", active: None },
            Item::Knob { p: MIX, x: 510.0, y: 172.0, r: 50.0, label: "MIX", tip: "Reverb amount. Up to 50% the dry signal stays at full level.", active: None },
            Item::Meter { m: 0, x: 222.0, y: 92.0, w: 196.0, h: 136.0, kind: MeterKind::Vu, caption: "OUTPUT" },
            Item::Switch { p: SPACE, x: 320.0, y: 254.0, labels: ["CHAMBER", "PLATE"],
                tip: "CHAMBER: a tiled echo room, with audible early reflections and a darker tail.\nPLATE: a steel plate, instantly dense and bright." },
            Item::Divider { y: 284.0 },
            Item::Knob { p: PREDELAY, x: 92.0, y: 332.0, r: 22.0, label: "PRE-DELAY", tip: "Gap before the reverb starts.", active: None },
            Item::Knob { p: SIZE, x: 183.0, y: 332.0, r: 22.0, label: "SIZE", tip: "Room size.", active: None },
            Item::Knob { p: DAMPING, x: 274.0, y: 332.0, r: 22.0, label: "DAMPING", tip: "How quickly the top end dies away. Lower = darker.", active: None },
            Item::Knob { p: LOWCUT, x: 365.0, y: 332.0, r: 22.0, label: "LOW CUT", tip: "Bass removed from the reverb (keeps it clear of the mix).", active: None },
            Item::Knob { p: WIDTH, x: 456.0, y: 332.0, r: 22.0, label: "WIDTH", tip: "Stereo width of the reverb.", active: None },
            Item::Knob { p: OUTPUT, x: 547.0, y: 332.0, r: 22.0, label: "OUTPUT", tip: "Final level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, ChamberParams, ChamberDsp, panel,
    name: "Echo Chamber", class_id: *b"S2EchoChambDIY01", vendor: "PEQUOD Beatles",
    categories: [Fx, Reverb]
);

#[cfg(test)]
mod tests {
    use super::*;

    fn tail_energy(decay: f32, space: f32) -> (f64, f64) {
        let fs = 48000.0;
        let mut d = ChamberDsp::new(fs as f32, 2);
        d.set_params(&[decay, 1.0, 0.0, 0.6, 6000.0, 100.0, 0.8, 0.0, space]);
        let n = 48000 * 3;
        let mut l = vec![0.0f32; n];
        l[0] = 1.0;
        let mut r = l.clone();
        let mut m = [0.0f32; 1];
        d.process(&mut [&mut l[..], &mut r[..]], &mut m);
        assert!(l.iter().chain(r.iter()).all(|v| v.is_finite()));
        let e = |a: usize, b: usize| l[a..b].iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
        (e(4800, 24000), e(96000, 120000))
    }

    #[test]
    fn longer_decay_longer_tail() {
        let (_, short_late) = tail_energy(0.8, 0.0);
        let (early, long_late) = tail_energy(4.0, 0.0);
        assert!(early > 0.0);
        assert!(long_late > short_late * 50.0, "{short_late} {long_late}");
    }

    #[test]
    fn decays_at_roughly_the_set_time() {
        // energy should drop ~60 dB over the decay time; check -20 dB in ~1/3 of it
        let fs = 48000.0;
        let mut d = ChamberDsp::new(fs as f32, 2);
        d.set_params(&[1.5, 1.0, 0.0, 0.6, 16000.0, 40.0, 0.8, 0.0, 1.0]);
        let n = 48000 * 2;
        let mut l = vec![0.0f32; n];
        l[0] = 1.0;
        let mut r = l.clone();
        d.process(&mut [&mut l[..], &mut r[..]], &mut [0.0]);
        let win = |t: f64| {
            let a = (t * fs) as usize;
            (l[a..a + 2400].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / 2400.0).sqrt()
        };
        let drop = 20.0 * (win(0.6) / win(0.1)).log10();
        println!("level drop 0.1->0.6 s: {drop:.1} dB (expect about -20)");
        assert!(drop < -12.0 && drop > -32.0);
    }
}
