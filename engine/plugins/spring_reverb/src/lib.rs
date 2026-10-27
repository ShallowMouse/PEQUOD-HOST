//! Spring Reverb — amp-style spring tank.
//!
//! A spring tank sends the signal down coiled springs; high frequencies
//! travel along them slower than low ones, so every reflection comes back
//! as a little falling "chirp" — the drip. Each spring is a delay loop with
//! a chain of stretched all-pass filters (which create that dispersion), a
//! low-pass for the springs' limited bandwidth, and slight wobble. DWELL
//! is how hard the drive circuit hits the tank: turn it up for the
//! crashing "boing" when you hit a note hard.

use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
use vintage::dsp::*;
use vintage::nih_plug_egui::EguiState;
use vintage::params::*;
use vintage::ui::{self, Item, MeterKind, Panel, Preset};
use vintage::{VintageDsp, VintageParams};

const W: u32 = 640;
const H: u32 = 432;

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Springs {
    #[name = "2 Springs"]
    Two,
    #[name = "3 Springs"]
    Three,
}

#[derive(Params)]
pub struct SpringParams {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "dwell"]
    dwell: FloatParam,
    #[id = "mix"]
    mix: FloatParam,
    #[id = "decay"]
    decay: FloatParam,
    #[id = "tone"]
    tone: FloatParam,
    #[id = "drip"]
    drip: FloatParam,
    #[id = "lowcut"]
    low_cut: FloatParam,
    #[id = "output"]
    output: FloatParam,
    #[id = "springs"]
    springs: EnumParam<Springs>,
}

impl Default for SpringParams {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            dwell: pct("Dwell", 0.5),
            mix: pct("Mix", 0.3),
            decay: secs("Decay", 2.2, 0.5, 6.0),
            tone: hz("Tone", 4000.0, 1500.0, 9000.0),
            drip: pct("Drip", 0.6),
            low_cut: hz("Low Cut", 220.0, 60.0, 800.0),
            output: db("Output", 0.0, -18.0, 6.0),
            springs: EnumParam::new("Springs", Springs::Three),
        }
    }
}

const DWELL: usize = 0;
const MIX: usize = 1;
const DECAY: usize = 2;
const TONE: usize = 3;
const DRIP: usize = 4;
const LOWCUT: usize = 5;
const OUTPUT: usize = 6;
const SPRINGS: usize = 7;

impl VintageParams for SpringParams {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.dwell.as_ptr(),
            self.mix.as_ptr(),
            self.decay.as_ptr(),
            self.tone.as_ptr(),
            self.drip.as_ptr(),
            self.low_cut.as_ptr(),
            self.output.as_ptr(),
            self.springs.as_ptr(),
        ]
    }
}

/// Stretched first-order all-pass: H(z) = (a + z^-M) / (1 + a z^-M).
#[derive(Clone)]
struct StretchedAllpass {
    xs: [f64; 4],
    ys: [f64; 4],
    i: usize,
    m: usize,
}

impl StretchedAllpass {
    fn new(m: usize) -> Self {
        Self { xs: [0.0; 4], ys: [0.0; 4], i: 0, m: m.clamp(1, 4) }
    }
    #[inline]
    fn process(&mut self, x: f64, a: f64) -> f64 {
        let j = (self.i + 4 - self.m) % 4;
        let y = a * x + self.xs[j] - a * self.ys[j];
        self.xs[self.i] = x;
        self.ys[self.i] = y;
        self.i = (self.i + 1) % 4;
        y
    }
    fn reset(&mut self) {
        self.xs = [0.0; 4];
        self.ys = [0.0; 4];
    }
}

const SPRING_MS: [f64; 3] = [33.3, 39.1, 45.7];
const ALLPASSES: usize = 14;

struct Spring {
    line: DelayLine,
    aps: Vec<StretchedAllpass>,
    lp: OnePole,
    wob: Wander,
    len: f64,
    g: f64,
}

pub struct SpringDsp {
    fs: f64,
    springs: Vec<Spring>,
    in_hp: OnePole,
    out_hp: [OnePole; 2],
    drive: Smooth,
    mix: Smooth,
    out: Smooth,
    drip: f64,
    count: usize,
}

impl VintageDsp for SpringDsp {
    const METERS: usize = 1;

    fn new(sample_rate: f32, _channels: usize) -> Self {
        let fs = sample_rate as f64;
        let springs = SPRING_MS
            .iter()
            .enumerate()
            .map(|(i, ms)| Spring {
                line: DelayLine::new((ms * 1.05 * 0.001 * fs) as usize + 16),
                aps: (0..ALLPASSES).map(|k| StretchedAllpass::new(2 + (k + i) % 2)).collect(),
                lp: OnePole::default(),
                wob: Wander::new(fs, 1.7 + i as f64 * 0.6, 31 + i as u32 * 7),
                len: ms * 0.001 * fs,
                g: 0.7,
            })
            .collect();
        Self {
            fs,
            springs,
            in_hp: OnePole::default(),
            out_hp: [OnePole::default(); 2],
            drive: Smooth::new(fs, 30.0, 3.0),
            mix: Smooth::new(fs, 20.0, 0.3),
            out: Smooth::new(fs, 20.0, 1.0),
            drip: 0.6,
            count: 3,
        }
    }

    fn reset(&mut self) {
        for s in &mut self.springs {
            s.line.reset();
            s.aps.iter_mut().for_each(|a| a.reset());
            s.lp.reset();
        }
    }

    fn set_params(&mut self, v: &[f32]) {
        let t60 = v[DECAY] as f64;
        for s in &mut self.springs {
            s.g = 10f64.powf(-3.0 * (s.len / self.fs) / t60);
            s.lp.set_lowpass(self.fs, v[TONE] as f64);
        }
        self.in_hp.set_lowpass(self.fs, v[LOWCUT] as f64);
        for h in &mut self.out_hp {
            h.set_lowpass(self.fs, v[LOWCUT] as f64 * 0.6);
        }
        self.drive.set(1.0 + 7.0 * v[DWELL] as f64);
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
        self.drip = v[DRIP] as f64;
        self.count = if v[SPRINGS] > 0.5 { 3 } else { 2 };
    }

    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let stereo = chans.len() >= 2;
        let a = -(0.45 + 0.35 * self.drip); // dispersion strength
        let count = self.count;
        let norm = 1.0 / (count as f64).sqrt();
        let mut peak = 0.0f64;
        for k in 0..n {
            let (xl, xr) = if stereo { (chans[0][k] as f64, chans[1][k] as f64) } else { (chans[0][k] as f64, chans[0][k] as f64) };
            let drive = self.drive.next();
            let mix = self.mix.next();
            let o = self.out.next();

            // the tank's drive circuit: springs don't carry bass, and a hot
            // signal slams them
            let send = self.in_hp.hp(0.5 * (xl + xr));
            let send = (send * drive).tanh() / drive.sqrt();

            let mut outs = [0.0f64; 3];
            for (i, s) in self.springs.iter_mut().take(count).enumerate() {
                let d = s.len * (1.0 + 0.004 * s.wob.next());
                let mut v = s.line.read(d);
                for ap in &mut s.aps {
                    v = ap.process(v, a);
                }
                v = s.lp.lp(v);
                s.line.write(send + s.g * v);
                outs[i] = v;
            }
            // springs spread across the stereo field
            let (mut l, mut r) = if count == 3 {
                (outs[0] + 0.5 * outs[2], outs[1] + 0.5 * outs[2])
            } else {
                (outs[0], outs[1])
            };
            l = self.out_hp[0].hp(l) * norm * 1.6;
            r = self.out_hp[1].hp(r) * norm * 1.6;

            let dry_g = (2.0 * (1.0 - mix)).min(1.0);
            let wet_g = (2.0 * mix).min(1.0);
            let ol = (xl * dry_g + l * wet_g) * o;
            let or = (xr * dry_g + r * wet_g) * o;
            if stereo {
                chans[0][k] = ol as f32;
                chans[1][k] = or as f32;
            } else {
                chans[0][k] = (0.5 * (ol + or)) as f32;
            }
            peak = peak.max(ol.abs()).max(or.abs());
        }
        meters[0] = peak as f32;
    }
}

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Amp Spring", values: &[] },
        Preset { name: "Surf Drip", values: &[(DWELL, 0.85), (MIX, 0.45), (DECAY, 3.0), (TONE, 5500.0), (DRIP, 0.9)] },
        Preset { name: "Rockabilly Spring", values: &[(DWELL, 0.55), (MIX, 0.25), (DECAY, 1.8), (TONE, 4500.0)] },
        Preset { name: "Long Dark Tank", values: &[(DWELL, 0.4), (MIX, 0.4), (DECAY, 5.0), (TONE, 2500.0), (DRIP, 0.4)] },
        Preset { name: "Subtle Ambience", values: &[(DWELL, 0.3), (MIX, 0.15), (DECAY, 1.0), (DRIP, 0.3), (SPRINGS, 0.0)] },
        Preset { name: "Kicked Tank Crash", values: &[(DWELL, 1.0), (MIX, 0.6), (DECAY, 3.5), (TONE, 7000.0), (DRIP, 1.0), (LOWCUT, 120.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "SPRING REVERB",
        subtitle: ("AMPLIFIER-STYLE", "SPRING TANK"),
        accent: ui::TEAL,
        size: (W, H),
        items: vec![
            Item::Knob { p: DWELL, x: 130.0, y: 172.0, r: 50.0, label: "DWELL",
                tip: "How hard the drive circuit hits the springs. Up high, hard notes make the tank crash and boing.", active: None },
            Item::Knob { p: MIX, x: 510.0, y: 172.0, r: 50.0, label: "MIX",
                tip: "Reverb amount. Up to 50% the dry signal stays at full level.", active: None },
            Item::Meter { m: 0, x: 222.0, y: 92.0, w: 196.0, h: 136.0, kind: MeterKind::Vu, caption: "OUTPUT" },
            Item::Switch { p: SPRINGS, x: 320.0, y: 254.0, labels: ["2 SPRINGS", "3"],
                tip: "Number of springs in the tank. Three is denser and smoother; two is sparser and more 'drippy'." },
            Item::Divider { y: 284.0 },
            Item::Knob { p: DECAY, x: 120.0, y: 332.0, r: 22.0, label: "DECAY", tip: "How long the springs ring.", active: None },
            Item::Knob { p: TONE, x: 220.0, y: 332.0, r: 22.0, label: "TONE", tip: "Brightness of the springs.", active: None },
            Item::Knob { p: DRIP, x: 320.0, y: 332.0, r: 22.0, label: "DRIP", tip: "How much the reflections chirp and splash (spring dispersion).", active: None },
            Item::Knob { p: LOWCUT, x: 420.0, y: 332.0, r: 22.0, label: "LOW CUT", tip: "Bass kept out of the tank.", active: None },
            Item::Knob { p: OUTPUT, x: 520.0, y: 332.0, r: 22.0, label: "OUTPUT", tip: "Final level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, SpringParams, SpringDsp, panel,
    name: "Spring Reverb", class_id: *b"S2SpringRevDIY01", vendor: "PEQUOD Beatles",
    categories: [Fx, Reverb]
);

#[cfg(test)]
mod tests {
    use super::*;

    fn impulse(decay: f32) -> (Vec<f32>, Vec<f32>) {
        let fs = 48000.0;
        let mut d = SpringDsp::new(fs as f32, 2);
        d.set_params(&[0.5, 1.0, decay, 4000.0, 0.6, 150.0, 0.0, 1.0]);
        let n = (fs * 3.0) as usize;
        let mut l = vec![0.0f32; n];
        l[0] = 0.5;
        let mut r = l.clone();
        for (a, b) in l.chunks_mut(512).zip(r.chunks_mut(512)) {
            d.process(&mut [a, b], &mut [0.0]);
        }
        (l, r)
    }

    #[test]
    fn rings_and_decays() {
        let (l, r) = impulse(2.0);
        assert!(l.iter().chain(r.iter()).all(|v| v.is_finite() && v.abs() < 2.0));
        let e = |a: usize, b: usize| l[a..b].iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
        let early = e(1000, 20000);
        let late = e(100000, 120000);
        println!("early {early:.2e} late {late:.2e}");
        assert!(early > 0.0 && late < early * 0.1 && late > 0.0);
    }

    #[test]
    fn longer_decay_rings_longer() {
        let e = |x: &Vec<f32>| x[96000..120000].iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
        let (s, _) = impulse(0.8);
        let (l, _) = impulse(5.0);
        assert!(e(&l) > e(&s) * 20.0);
    }
}
