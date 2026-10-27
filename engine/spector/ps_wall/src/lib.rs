//! Wall of Sound: one part becomes a section of players in unison, the way Spector stacked guitars, pianos and basses on the same line. Each player is a little early or late and slightly out of tune, which gives the chorusing no effect box could; the crowded room squeezes the midrange like the packed Studio A did.

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
use vintage::verb::Fdn;

pub const W: u32 = 640;
pub const H: u32 = 432;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Image { #[name = "Mono"] Mono, #[name = "Stereo"] Stereo }
const MAXP: usize = 6;
// fixed "personalities" per chair: timing (fraction of looseness), pitch-drift rate, tone tilt
const CHAIR: [(f64, f64, f64); MAXP] = [(0.0, 0.23, 0.0), (0.55, 0.31, 0.7), (-0.4, 0.27, -0.6), (0.9, 0.37, 0.3), (-0.8, 0.19, -0.9), (0.3, 0.41, 0.9)];
const PAN: [f64; MAXP] = [0.0, -0.6, 0.6, -0.3, 0.3, -0.9];


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "players"]
    pub players: IntParam,
    #[id = "loose"]
    pub loose: FloatParam,
    #[id = "detune"]
    pub detune: FloatParam,
    #[id = "variety"]
    pub variety: FloatParam,
    #[id = "bleed"]
    pub bleed: FloatParam,
    #[id = "squeeze"]
    pub squeeze: FloatParam,
    #[id = "mono"]
    pub mono: EnumParam<Image>,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            players: IntParam::new("Players", 4, IntRange::Linear { min: 1, max: 6 }),
            loose: ms("Looseness", 10.0, 0.5, 35.0),
            detune: FloatParam::new("Detune", 7.0, FloatRange::Linear { min: 0.0, max: 25.0 }).with_step_size(0.1).with_unit(" ct"),
            variety: pct("Tone Variety", 0.5),
            bleed: pct("Room Bleed", 0.3),
            squeeze: pct("Squeeze", 0.4),
            mono: EnumParam::new("Image", Image::Mono),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const PLAYERS: usize = 0;
pub const LOOSE: usize = 1;
pub const DETUNE: usize = 2;
pub const VARIETY: usize = 3;
pub const BLEED: usize = 4;
pub const SQUEEZE: usize = 5;
pub const MONO: usize = 6;
pub const OUTPUT: usize = 7;
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
            self.players.as_ptr(),
            self.loose.as_ptr(),
            self.detune.as_ptr(),
            self.variety.as_ptr(),
            self.bleed.as_ptr(),
            self.squeeze.as_ptr(),
            self.mono.as_ptr(),
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

pub struct Dsp {
    fs: f64,
    line: DelayLine,
    wander: Vec<Wander>,
    drift: Vec<Wander>,
    tone: Vec<OnePole>,
    players: usize,
    loose: Smooth,
    detune: Smooth,
    variety: f64,
    bleed: Smooth,
    squeeze: f64,
    stereo: bool,
    room: Fdn,
    det_bp: Biquad,
    env: Envelope,
    gr: f64,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 2;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut room = Fdn::new(fs, 0.6);
        room.configure(0.45, [0.6, 0.5, 0.35], 2500.0);
        let mut det_bp = Biquad::identity();
        det_bp.set_bandpass(fs, 900.0, 0.5);
        Self {
            fs,
            line: DelayLine::new((fs * 0.12) as usize),
            wander: (0..MAXP).map(|i| Wander::new(fs, 0.35, 101 + i as u32 * 17)).collect(),
            drift: (0..MAXP).map(|i| Wander::new(fs, CHAIR[i].1, 303 + i as u32 * 29)).collect(),
            tone: vec![OnePole::default(); MAXP],
            players: 4,
            loose: Smooth::new(fs, 80.0, 10.0),
            detune: Smooth::new(fs, 80.0, 7.0),
            variety: -1.0,
            bleed: Smooth::new(fs, 20.0, 0.3),
            squeeze: 0.4,
            stereo: false,
            room,
            det_bp,
            env: Envelope::new(fs, 30.0, 300.0),
            gr: 1.0,
            out: Smooth::new(fs, 20.0, 1.0),
        }
    }
    fn reset(&mut self) {
        self.line.reset();
        self.room.reset();
        self.tone.iter_mut().for_each(|t| t.reset());
        self.det_bp.reset();
        self.env.value = 0.0;
        self.gr = 1.0;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.players = (v[PLAYERS].round() as usize).clamp(1, MAXP);
        self.loose.set(v[LOOSE] as f64);
        self.detune.set(v[DETUNE] as f64);
        let var = v[VARIETY] as f64;
        if (var - self.variety).abs() > 1e-4 {
            self.variety = var;
            for (i, t) in self.tone.iter_mut().enumerate() {
                // each chair's instrument is a bit brighter or darker
                let tilt = CHAIR[i].2 * var;
                t.set_lowpass(self.fs, 1200.0 * 2f64.powf(tilt * 1.5));
            }
        }
        self.bleed.set(v[BLEED] as f64);
        self.squeeze = v[SQUEEZE] as f64;
        self.stereo = v[MONO] > 0.5;
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let np = self.players;
        let norm = 1.0 / (np as f64).sqrt();
        let mut peak = 0.0f64;
        let mut grmax = 0.0f64;
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            self.line.write(x);
            let loose = self.loose.next() * 0.001 * self.fs;
            let det = self.detune.next();
            let mut l = 0.0;
            let mut r = 0.0;
            for i in 0..np {
                let y = if i == 0 {
                    // the first chair is the player you recorded
                    self.line.read(1.0 + 0.5 * MAXP as f64)
                } else {
                    // timing: a fixed lean plus a slowly wandering push and pull
                    let lean = (0.5 + 0.5 * CHAIR[i].0) * loose;
                    let push = 0.25 * loose * self.wander[i].next();
                    // tuning: delay drift whose slope is a pitch offset of up to `det` cents
                    let rate = CHAIR[i].1;
                    let depth = (det / 1731.0) / (2.0 * std::f64::consts::PI * rate) * self.fs;
                    let d = 4.0 + lean + push.abs() + depth * (1.0 + self.drift[i].next());
                    self.line.read(d.max(1.0))
                };
                // tone: mix of the low-passed and the full signal
                let lp = self.tone[i].lp(y);
                let t = CHAIR[i].2 * self.variety;
                let y = y + t * 0.6 * (y - lp);
                let (pl, pr) = if self.stereo {
                    let a = (PAN[i] + 1.0) * std::f64::consts::FRAC_PI_4;
                    (a.cos() * 2f64.sqrt(), a.sin() * 2f64.sqrt())
                } else {
                    (1.0, 1.0)
                };
                l += y * pl;
                r += y * pr;
            }
            l *= norm;
            r *= norm;
            // the packed room: everyone leaks into everyone's mics
            let b = self.bleed.next();
            let mono = 0.5 * (l + r);
            let (rl, rr) = { let dif = self.room.diffuse(mono, 2); self.room.process(dif) };
            l += b * rl * 0.9;
            r += b * if self.stereo { rr } else { rl } * 0.9;
            // acoustic squeeze: midrange-weighted, slow, soft-knee
            let det_in = self.det_bp.process(0.5 * (l + r));
            let e = self.env.process(det_in * 2.0);
            let over = (gain_to_db(e) + 24.0).max(0.0);
            let target = db_to_gain(-over * self.squeeze * 0.6);
            self.gr += (target - self.gr) * 0.002;
            grmax = grmax.max(-gain_to_db(self.gr));
            let o = self.out.next() * self.gr;
            let (yl, yr) = (l * o, r * o);
            peak = peak.max(yl.abs()).max(yr.abs());
            if nch >= 2 {
                chans[0][k] = yl as f32;
                chans[1][k] = yr as f32;
            } else if nch == 1 {
                chans[0][k] = (0.5 * (yl + yr)) as f32;
            }
        }
        meters[0] = peak as f32;
        meters[1] = grmax as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Four Acoustic Guitars", values: &[(PLAYERS, 4.0), (LOOSE, 9.0), (DETUNE, 6.0), (VARIETY, 0.6), (BLEED, 0.3), (SQUEEZE, 0.4)] },
        Preset { name: "Three Pianos (one shared)", values: &[(PLAYERS, 3.0), (LOOSE, 14.0), (DETUNE, 9.0), (VARIETY, 0.5), (BLEED, 0.35)] },
        Preset { name: "Two Basses (upright + electric)", values: &[(PLAYERS, 2.0), (LOOSE, 6.0), (DETUNE, 4.0), (VARIETY, 0.9), (BLEED, 0.15), (SQUEEZE, 0.3)] },
        Preset { name: "Double Drummers", values: &[(PLAYERS, 2.0), (LOOSE, 18.0), (DETUNE, 1.0), (VARIETY, 0.4), (BLEED, 0.5), (SQUEEZE, 0.6)] },
        Preset { name: "Full Wrecking Crew", values: &[(PLAYERS, 6.0), (LOOSE, 16.0), (DETUNE, 10.0), (VARIETY, 0.7), (BLEED, 0.5), (SQUEEZE, 0.7)] },
        Preset { name: "Horn Section Blend", values: &[(PLAYERS, 4.0), (LOOSE, 12.0), (DETUNE, 8.0), (VARIETY, 0.5), (BLEED, 0.4)] },
        Preset { name: "Castanets x3", values: &[(PLAYERS, 3.0), (LOOSE, 22.0), (DETUNE, 2.0), (VARIETY, 0.8), (BLEED, 0.4)] },
        Preset { name: "Tight Unison (stereo)", values: &[(PLAYERS, 3.0), (LOOSE, 3.0), (DETUNE, 4.0), (MONO, 1.0), (BLEED, 0.15)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "WALL OF SOUND",
        subtitle: ("ONE PART, A WHOLE SECTION", "UNISON PLAYERS · PACKED ROOM"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![

            Item::Selector { p: PLAYERS, x: 80.0, y: 160.0, label: "PLAYERS", positions: &["1", "2", "3", "4", "5", "6"], tip: "How many musicians play the part in unison. Spector typically booked three or four guitars, three or four keyboards and two basses." },
            Item::Knob { p: LOOSE, x: 200.0, y: 160.0, r: 34.0, label: "LOOSENESS", tip: "How far the players drift ahead and behind each other.", active: None },
            Item::Knob { p: DETUNE, x: 320.0, y: 160.0, r: 34.0, label: "DETUNE", tip: "How far each player's tuning wanders (cents). This is what makes the section shimmer like a chorus.", active: None },
            Item::Meter { m: 1, x: 410.0, y: 104.0, w: 190.0, h: 112.0, kind: MeterKind::GainReduction, caption: "ROOM SQUEEZE" },
            Item::Divider { y: 264.0 },
            Item::Knob { p: VARIETY, x: 80.0, y: 330.0, r: 22.0, label: "TONE VARIETY", tip: "How different the instruments sound from each other (brighter and darker guitars, pianos).", active: None },
            Item::Knob { p: BLEED, x: 190.0, y: 330.0, r: 22.0, label: "ROOM BLEED", tip: "Everyone leaking into everyone's microphones in a small, packed room.", active: None },
            Item::Knob { p: SQUEEZE, x: 300.0, y: 330.0, r: 22.0, label: "SQUEEZE", tip: "The crowded room and loud playback acting as a midrange compressor.", active: None },
            Item::Switch { p: MONO, x: 420.0, y: 330.0, labels: ["MONO", "STEREO"], tip: "Back to Mono: Spector mixed in mono. Stereo spreads the players across the image." },
            Item::Knob { p: OUTPUT, x: 550.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Wall of Sound", class_id: *b"PSWallOfSoundPQD", vendor: "PEQUOD Spector",
    categories: [Fx, Modulation]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    pub fn run(p: &[f32], input: &mut [Vec<f32>]) -> Vec<f32> {
        let mut d = Dsp::new(48000.0, input.len());
        d.set_params(p);
        d.reset();
        let mut m = vec![0.0f32; Dsp::METERS.max(1)];
        let n = input[0].len();
        let mut k = 0;
        while k < n {
            let e = (k + 512).min(n);
            d.set_params(p);
            let mut chans: Vec<&mut [f32]> = input.iter_mut().map(|c| &mut c[k..e]).collect();
            d.process(&mut chans, &mut m);
            k = e;
        }
        m
    }
    pub fn sine(f: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n).map(|k| (amp * (2.0 * std::f64::consts::PI * f * k as f64 / 48000.0).sin()) as f32).collect()
    }
    pub fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
    }
    pub fn db(x: f64) -> f64 {
        20.0 * x.max(1e-12).log10()
    }
    pub fn with(mut p: Vec<f32>, set: &[(usize, f32)]) -> Vec<f32> {
        for (i, v) in set {
            p[*i] = *v;
        }
        p
    }
    /// Every preset runs finite and bounded on noise bursts.
    #[test]
    fn presets_run_clean() {
        let base = defaults();
        let mut all = vec![base.clone()];
        for pr in presets() {
            all.push(with(base.clone(), pr.values));
        }
        for p in all {
            let mut nz = vintage::dsp::Noise::new(7);
            let n = 48000;
            let mut l: Vec<f32> = (0..n).map(|k| if (k / 6000) % 2 == 0 { (nz.next() * 0.5) as f32 } else { 0.0 }).collect();
            let mut r = l.clone();
            {
                let mut input = vec![std::mem::take(&mut l), std::mem::take(&mut r)];
                run(&p, &mut input);
                assert!(input.iter().flatten().all(|v| v.is_finite() && v.abs() < 8.0), "unbounded output for {:?}", p);
            }
        }
    }

    #[test]
    fn more_players_more_shimmer() {
        // a steady tone: with players detuned, the envelope beats; with one player it doesn't
        let mut res = vec![];
        for np in [1.0f32, 4.0] {
            let p = with(defaults(), &[(PLAYERS, np), (BLEED, 0.0), (SQUEEZE, 0.0), (DETUNE, 12.0)]);
            let mut input = vec![sine(440.0, 0.3, 96000)];
            run(&p, &mut input);
            // envelope variance over 10 ms windows
            let w: Vec<f64> = input[0][24000..].chunks(480).map(|c| rms(c)).collect();
            let m = w.iter().sum::<f64>() / w.len() as f64;
            let var = (w.iter().map(|v| (v - m).powi(2)).sum::<f64>() / w.len() as f64).sqrt() / m;
            println!("players {np}: envelope variation {var:.3}");
            res.push(var);
        }
        assert!(res[0] < 0.02 && res[1] > 0.05);
    }

}
