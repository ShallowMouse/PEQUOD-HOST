//! Plate 140 — after the EMT 140 steel plate reverb.
//!
//! A 2 x 1 m steel plate carries bending waves, which are dispersive: high
//! frequencies travel faster than low ones, so each reflection off the
//! plate's edges arrives as a little upward-to-downward "chirp", and the
//! echo density is high from the very start (no distinct early
//! reflections). The damper (a porous plate moved toward the steel) sets
//! the decay: about 5 s at 500 Hz fully open, about 1 s fully closed, with
//! the treble always decaying faster (about 1.5 s at 10 kHz open), per
//! EMT's 140 TS data. Two pickups at different distances from the driver
//! give two uncorrelated outputs (the stereo 140 TS); M = A + B.
//!
//! The model: a 16-line feedback delay network sized to the plate's mean
//! free path for bending waves, with dispersive allpass chains in every
//! line and per-line decay filters matched to EMT's curves.

use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
use vintage::dsp::*;
use vintage::nih_plug_egui::EguiState;
use vintage::params::*;
use vintage::ui::{self, Item, MeterKind, Panel, Preset};
use vintage::{VintageDsp, VintageParams};

const W: u32 = 640;
const H: u32 = 432;

#[derive(Params)]
pub struct PlateParams {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "decay"]
    decay: FloatParam,
    #[id = "predelay"]
    predelay: FloatParam,
    #[id = "lowcut"]
    low_cut: FloatParam,
    #[id = "highcut"]
    high_cut: FloatParam,
    #[id = "width"]
    width: FloatParam,
    #[id = "mix"]
    mix: FloatParam,
    #[id = "output"]
    output: FloatParam,
}

impl Default for PlateParams {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            decay: FloatParam::new("Decay (500 Hz)", 2.0, FloatRange::Skewed { min: 1.0, max: 5.0, factor: FloatRange::skew_factor(-1.0) })
                .with_step_size(0.05)
                .with_unit(" s"),
            predelay: ms("Pre-delay", 0.0, 0.0, 250.0),
            low_cut: hz("Return Low Cut", 80.0, 20.0, 600.0),
            high_cut: hz("Return High Cut", 12000.0, 2000.0, 20000.0),
            width: pct("Width", 1.0),
            mix: pct("Mix", 0.3),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

const DECAY: usize = 0;
const PREDELAY: usize = 1;
const LOWCUT: usize = 2;
const HIGHCUT: usize = 3;
const WIDTH: usize = 4;
const MIX: usize = 5;
const OUTPUT: usize = 6;

impl VintageParams for PlateParams {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.decay.as_ptr(),
            self.predelay.as_ptr(),
            self.low_cut.as_ptr(),
            self.high_cut.as_ptr(),
            self.width.as_ptr(),
            self.mix.as_ptr(),
            self.output.as_ptr(),
        ]
    }
}

// ------------------------------------------------------------------ plate --

/// Decay times (s) at 125 Hz, 500 Hz, 4 kHz and 10 kHz for a damper
/// setting, given as the 500 Hz time. Endpoints: EMT 140 TS data (open:
/// ~5 s at 500 Hz, rising toward the bass, ~1.5 s at 10 kHz; closed: ~1 s).
pub fn decay_curve(rt500: f64) -> [f64; 4] {
    const OPEN: [f64; 4] = [6.0, 5.0, 3.0, 1.5];
    const CLOSED: [f64; 4] = [1.4, 1.0, 0.8, 0.6];
    let t = ((rt500.ln() - 1f64.ln()) / (5f64.ln() - 1f64.ln())).clamp(0.0, 1.0);
    let mut out = [0.0; 4];
    for i in 0..4 {
        out[i] = (CLOSED[i].ln() + t * (OPEN[i].ln() - CLOSED[i].ln())).exp();
    }
    out
}

const LINES: usize = 16;
const DISPERSION: usize = 3;

struct Line {
    buf: Vec<f64>,
    pos: usize,
    len: usize,
    ap: [(f64, f64, f64); DISPERSION], // (coefficient, x1, y1)
    lo: Biquad,
    hi: Biquad,
    top: Biquad,
    g: f64,
}

impl Line {
    #[inline]
    fn read(&mut self) -> f64 {
        let mut y = self.buf[self.pos];
        // dispersion: first-order allpasses delay the bass more than the
        // treble, as bending waves do
        for ap in &mut self.ap {
            let out = -ap.0 * y + ap.1 + ap.0 * ap.2;
            ap.1 = y;
            ap.2 = out;
            y = out;
        }
        let y = self.lo.process(y * self.g);
        let y = self.hi.process(y);
        self.top.process(y)
    }
    #[inline]
    fn write(&mut self, x: f64) {
        self.buf[self.pos] = x;
        self.pos += 1;
        if self.pos >= self.len {
            self.pos = 0;
        }
    }
}

pub struct Plate {
    fs: f64,
    lines: Vec<Line>,
    chirp: Vec<(f64, f64, f64)>,
    drive_hp: Biquad,
    drive_lp: Biquad,
    signs: [[f64; LINES]; 3],
    out_gain: f64,
    rt: f64,
}

impl Plate {
    pub fn new(fs: f64) -> Self {
        // bending-wave speed on 0.5 mm steel ~ 2 sqrt(w) (D/rho h)^(1/4): about
        // 140 m/s at 1 kHz. Mean free path of the 2 x 1 m plate: pi S / P = 1.05 m,
        // so ~7.5 ms between edge reflections; lines spread around that.
        let base = 0.0075;
        let mut lines = Vec::with_capacity(LINES);
        for i in 0..LINES {
            let spread = 0.45 * (1.9f64 / 0.45).powf(i as f64 / (LINES - 1) as f64);
            let len = (((base * spread * fs) as usize) | 1) + [0, 2, 6, 10, 12, 16, 18, 22, 28, 30, 36, 40, 42, 46, 52, 58][i];
            let a = 0.45 + 0.2 * ((i * 7) % LINES) as f64 / LINES as f64;
            lines.push(Line {
                buf: vec![0.0; len + 1],
                pos: 0,
                len,
                ap: [(a, 0.0, 0.0), (a * 0.9, 0.0, 0.0), (a * 0.8, 0.0, 0.0)],
                lo: Biquad::identity(),
                hi: Biquad::identity(),
                top: Biquad::identity(),
                g: 0.0,
            });
        }
        // the first chirp from driver to pickup: low frequencies ~10 ms later
        // than the top (0.5 m of travel at 45 m/s vs 450 m/s)
        let stages = ((0.010 * fs) / 4.0).round().max(4.0) as usize;
        let chirp = vec![(0.6, 0.0, 0.0); stages.min(128)];
        let mut drive_hp = Biquad::identity();
        drive_hp.set_highpass(fs, 60.0, 0.7);
        let mut drive_lp = Biquad::identity();
        drive_lp.set_lowpass(fs, 14000.0, 0.7);
        let mut signs = [[1.0; LINES]; 3];
        let mut seed = 0x9E3779B9u32;
        for row in signs.iter_mut() {
            for v in row.iter_mut() {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                *v = if seed & 2 == 0 { 1.0 } else { -1.0 };
            }
        }
        let mut p = Self { fs, lines, chirp, drive_hp, drive_lp, signs, out_gain: 0.0, rt: 0.0 };
        p.set_decay(2.0);
        p
    }

    pub fn set_decay(&mut self, rt500: f64) {
        if (rt500 - self.rt).abs() < 1e-4 {
            return;
        }
        self.rt = rt500;
        let fs = self.fs;
        let [r125, r500, r4k, r10k] = decay_curve(rt500);
        let mut gsum = 0.0;
        for l in &mut self.lines {
            // loop delay includes the allpasses' mid-band group delay
            let ap_delay: f64 = l.ap.iter().map(|a| (1.0 - a.0) / (1.0 + a.0)).sum();
            let d = (l.len as f64 + ap_delay) / fs;
            let db = |t: f64| -60.0 * d / t;
            l.g = 10f64.powf(db(r500) / 20.0);
            l.lo.set_low_shelf(fs, 300.0, 0.6, db(r125) - db(r500));
            l.hi.set_high_shelf(fs, 2000.0, 0.6, db(r4k) - db(r500));
            l.top.set_high_shelf(fs, 8000.0, 0.7, db(r10k) - db(r4k));
            gsum += l.g * l.g;
        }
        let g2 = (gsum / LINES as f64).min(0.99999);
        // impulse energy out of the network; aim for energy ~ 0.25 * RT
        let e_net = g2 / (1.0 - g2) / LINES as f64;
        self.out_gain = (0.25 * rt500 / e_net).sqrt();
    }

    pub fn clear(&mut self) {
        for l in &mut self.lines {
            l.buf.iter_mut().for_each(|v| *v = 0.0);
            l.ap.iter_mut().for_each(|a| {
                a.1 = 0.0;
                a.2 = 0.0
            });
            l.lo.reset();
            l.hi.reset();
            l.top.reset();
        }
        self.chirp.iter_mut().for_each(|a| {
            a.1 = 0.0;
            a.2 = 0.0
        });
        self.drive_hp.reset();
        self.drive_lp.reset();
    }

    /// Mono drive in; pickups A and B out.
    #[inline]
    pub fn process(&mut self, x: f64) -> (f64, f64) {
        let mut u = self.drive_lp.process(self.drive_hp.process(x));
        for ap in &mut self.chirp {
            let out = -ap.0 * u + ap.1 + ap.0 * ap.2;
            ap.1 = u;
            ap.2 = out;
            u = out;
        }
        let mut o = [0.0; LINES];
        for (i, l) in self.lines.iter_mut().enumerate() {
            o[i] = l.read();
        }
        let (mut a, mut b) = (0.0, 0.0);
        for i in 0..LINES {
            a += self.signs[1][i] * o[i];
            b += self.signs[2][i] * o[i];
        }
        let mut h = 1;
        while h < LINES {
            let mut i = 0;
            while i < LINES {
                for j in i..i + h {
                    let (p, q) = (o[j], o[j + h]);
                    o[j] = p + q;
                    o[j + h] = p - q;
                }
                i += 2 * h;
            }
            h *= 2;
        }
        let norm = 0.25; // 1/sqrt(16)
        let inj = u * norm;
        for (i, l) in self.lines.iter_mut().enumerate() {
            l.write(o[i] * norm + self.signs[0][i] * inj);
        }
        let g = self.out_gain * norm;
        (a * g, b * g)
    }
}

// -------------------------------------------------------------------- dsp --

pub struct PlateDsp {
    fs: f64,
    plate: Plate,
    pre: Vec<f64>,
    pre_pos: usize,
    pre_len: Smooth,
    lc: [Biquad; 2],
    hc: [Biquad; 2],
    filt: (f64, f64),
    width: Smooth,
    mix: Smooth,
    out: Smooth,
    decay: Smooth,
}

impl VintageDsp for PlateDsp {
    const METERS: usize = 1;

    fn new(sample_rate: f32, _channels: usize) -> Self {
        let fs = sample_rate as f64;
        Self {
            fs,
            plate: Plate::new(fs),
            pre: vec![0.0; (0.26 * fs) as usize + 4],
            pre_pos: 0,
            pre_len: Smooth::new(fs, 60.0, 0.0),
            lc: [Biquad::identity(), Biquad::identity()],
            hc: [Biquad::identity(), Biquad::identity()],
            filt: (0.0, 0.0),
            width: Smooth::new(fs, 20.0, 1.0),
            mix: Smooth::new(fs, 20.0, 0.3),
            out: Smooth::new(fs, 20.0, 1.0),
            decay: Smooth::new(fs, 80.0, 2.0),
        }
    }

    fn reset(&mut self) {
        self.plate.clear();
        self.pre.iter_mut().for_each(|v| *v = 0.0);
        self.lc.iter_mut().chain(self.hc.iter_mut()).for_each(|b| b.reset());
    }

    fn set_params(&mut self, v: &[f32]) {
        self.decay.set(v[DECAY] as f64);
        self.pre_len.set(v[PREDELAY] as f64 * 0.001 * self.fs);
        let f = (v[LOWCUT] as f64, v[HIGHCUT] as f64);
        if f != self.filt {
            self.filt = f;
            for b in &mut self.lc {
                b.set_highpass(self.fs, f.0, 0.707);
            }
            for b in &mut self.hc {
                b.set_lowpass(self.fs, f.1, 0.707);
            }
        }
        self.width.set(v[WIDTH] as f64);
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }

    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let len = self.pre.len();
        let mut peak: f64 = 0.0;
        for k in 0..n {
            let d = self.decay.next();
            if k % 32 == 0 {
                self.plate.set_decay(d);
            }
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            // pre-delay (Abbey Road fed its plates through a tape machine for this)
            self.pre[self.pre_pos] = x;
            let d = self.pre_len.next();
            let read = (self.pre_pos as f64 - d + len as f64) % len as f64;
            let i = read as usize;
            let fr = read - i as f64;
            let xd = self.pre[i] * (1.0 - fr) + self.pre[(i + 1) % len] * fr;
            self.pre_pos = (self.pre_pos + 1) % len;

            let (a, b) = self.plate.process(xd);
            let w = self.width.next();
            let mid = 0.5 * (a + b);
            let side = 0.5 * (a - b) * w;
            let wet = [
                self.hc[0].process(self.lc[0].process(mid + side)),
                self.hc[1].process(self.lc[1].process(mid - side)),
            ];
            let m = self.mix.next();
            let g = self.out.next();
            for (ci, c) in chans.iter_mut().enumerate() {
                let wv = if nch == 1 { 0.5 * (wet[0] + wet[1]) } else { wet[ci.min(1)] };
                let y = (c[k] as f64 * (1.0 - m) + wv * m) * g;
                peak = peak.max(y.abs());
                c[k] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Abbey Road Vocal Plate", values: &[(DECAY, 1.8), (PREDELAY, 0.0), (LOWCUT, 150.0), (MIX, 0.25)] },
        Preset { name: "Plate + Tape Pre-delay", values: &[(DECAY, 2.2), (PREDELAY, 90.0), (LOWCUT, 120.0), (MIX, 0.3)] },
        Preset { name: "Short Snare Plate", values: &[(DECAY, 1.1), (LOWCUT, 200.0), (HIGHCUT, 9000.0), (MIX, 0.3)] },
        Preset { name: "Damper Open (5 s)", values: &[(DECAY, 5.0), (LOWCUT, 100.0), (MIX, 0.35)] },
        Preset { name: "Mono Plate (M = A + B)", values: &[(DECAY, 2.0), (WIDTH, 0.0), (MIX, 0.3)] },
        Preset { name: "100% Wet (for a send)", values: &[(DECAY, 2.0), (MIX, 1.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "PLATE 140",
        subtitle: ("AFTER THE EMT 140 TS", "STEEL PLATE REVERBERATION UNIT"),
        accent: ui::TEAL,
        size: (W, H),
        items: vec![
            Item::Knob { p: DECAY, x: 140.0, y: 172.0, r: 54.0, label: "DAMPER",
                tip: "The damper's position, shown as the decay time at 500 Hz: 1 s (damper closed) to 5 s (open). Treble always decays faster, bass slower, as on the real plate.", active: None },
            Item::Meter { m: 0, x: 238.0, y: 92.0, w: 180.0, h: 126.0, kind: MeterKind::Vu, caption: "OUTPUT" },
            Item::Knob { p: PREDELAY, x: 510.0, y: 172.0, r: 40.0, label: "PRE-DELAY",
                tip: "Delay before the plate. EMI often sent to the plate through a tape machine to get this.", active: None },
            Item::Divider { y: 284.0 },
            Item::Knob { p: LOWCUT, x: 90.0, y: 332.0, r: 22.0, label: "LOW CUT", tip: "High-pass on the plate return. Abbey Road usually thinned the plate out.", active: None },
            Item::Knob { p: HIGHCUT, x: 190.0, y: 332.0, r: 22.0, label: "HIGH CUT", tip: "Low-pass on the plate return.", active: None },
            Item::Knob { p: WIDTH, x: 290.0, y: 332.0, r: 22.0, label: "WIDTH", tip: "0 = mono (M = A + B, as a single pickup). Up = the two pickups left and right.", active: None },
            Item::Knob { p: MIX, x: 390.0, y: 332.0, r: 22.0, label: "MIX", tip: "Dry vs. plate. Use 100% on a send.", active: None },
            Item::Knob { p: OUTPUT, x: 490.0, y: 332.0, r: 22.0, label: "OUTPUT", tip: "Final level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, PlateParams, PlateDsp, panel,
    name: "Plate 140", class_id: *b"S2EMTPlateDIY001", vendor: "PEQUOD Beatles",
    categories: [Fx, Reverb]
);

#[cfg(test)]
mod tests {
    use super::*;

    fn ir(decay: f32, secs: f64) -> Vec<f64> {
        let fs = 48000.0;
        let mut d = PlateDsp::new(fs as f32, 1);
        let p = [decay, 0.0, 20.0, 20000.0, 1.0, 1.0, 0.0];
        // settle the smoothed decay
        let mut z = vec![0.0f32; 48000];
        d.set_params(&p);
        d.process(&mut [&mut z[..]], &mut [0.0]);
        d.reset();
        let mut x = vec![0.0f32; (fs * secs) as usize];
        x[0] = 1.0;
        for c in x.chunks_mut(512) {
            d.process(&mut [c], &mut [0.0]);
        }
        x.into_iter().map(|v| v as f64).collect()
    }

    fn t30(h: &[f64], f0: f64) -> f64 {
        let fs = 48000.0;
        let mut bp = Biquad::identity();
        bp.set_bandpass(fs, f0, 1.4);
        let mut e: Vec<f64> = h.iter().map(|v| bp.process(*v).powi(2)).collect();
        for i in (0..e.len() - 1).rev() {
            e[i] += e[i + 1];
        }
        let db = |i: usize| 10.0 * (e[i] / e[0]).max(1e-30).log10();
        let a = (0..e.len()).find(|&i| db(i) < -5.0).unwrap();
        let b = (0..e.len()).find(|&i| db(i) < -35.0).unwrap_or(e.len() - 1);
        (b - a) as f64 / fs * 2.0
    }

    #[test]
    fn decays_like_the_emt_curves() {
        for rt in [1.0f32, 2.0, 3.5, 5.0] {
            let h = ir(rt, rt as f64 * 2.2 + 0.5);
            let want = decay_curve(rt as f64);
            let (m, l, t) = (t30(&h, 500.0), t30(&h, 150.0), t30(&h, 8000.0));
            println!("damper {rt} s: 150 Hz {l:.2} ({:.2}), 500 Hz {m:.2} ({:.2}), 8 kHz {t:.2}", want[0], want[1]);
            assert!((m / rt as f64 - 1.0).abs() < 0.15, "500 Hz {m} vs {rt}");
            assert!(l > m * 0.95, "bass should ring at least as long");
            assert!(t < m, "treble should decay faster");
        }
    }

    #[test]
    fn dense_from_the_start() {
        // plates have no gap of discrete early reflections: energy in the
        // first 20 ms is spread over many samples
        let h = ir(2.0, 0.1);
        let fs = 48000.0;
        let early = &h[..(0.02 * fs) as usize];
        let nz = early.iter().filter(|v| v.abs() > 1e-4).count();
        println!("non-negligible samples in first 20 ms: {nz} of {}", early.len());
        assert!(nz > early.len() / 3);
    }

    #[test]
    fn finite_everywhere() {
        let mut d = PlateDsp::new(48000.0, 2);
        d.set_params(&[5.0, 250.0, 20.0, 20000.0, 1.0, 1.0, 12.0]);
        let mut l: Vec<f32> = (0..96000).map(|k| ((k as f64 * 0.03).sin() * 0.9) as f32).collect();
        let mut r = l.clone();
        d.process(&mut [&mut l[..], &mut r[..]], &mut [0.0]);
        assert!(l.iter().chain(r.iter()).all(|v| v.is_finite() && v.abs() < 20.0));
    }
}
