//! Synth Room: Benny Andersson's synthesizers as they went onto ABBA's tapes at Polar Music -
//! not straight from the keyboard but played through an amplifier in the studio and picked up
//! by microphones in the room. For "Arrival" Michael B. Tretow had the Polymoog "picked up by
//! two ambient mics only", moving the amp between overdubs. The amp and speaker round the
//! synth off and give it a body; the two room mics give it width and air; a second take at
//! another tape speed (Tretow's varispeed double) thickens it the way Benny thickened his
//! Minimoog lines by playing them again. For the GX-1, the Polymoog and the Minimoog - and the
//! lead at the end of "Lay All Your Love on Me".
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

use vintage::fx::VarispeedTakes as Takes;
use vintage::verb::Fdn;

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "speaker"]
    pub speaker: FloatParam,
    #[id = "room"]
    pub room: FloatParam,
    #[id = "size"]
    pub size: FloatParam,
    #[id = "distance"]
    pub distance: FloatParam,
    #[id = "double"]
    pub double: FloatParam,
    #[id = "width"]
    pub width: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            drive: pct("Amp Drive", 0.3),
            speaker: hz("Speaker", 6500.0, 2500.0, 14000.0),
            room: pct("Room Mics", 0.7),
            size: pct("Room Size", 0.4),
            distance: ms("Mic Distance", 12.0, 3.0, 40.0),
            double: pct("Second Take", 0.0),
            width: pct("Width", 1.0),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const DRIVE: usize = 0;
pub const SPEAKER: usize = 1;
pub const ROOM: usize = 2;
pub const SIZE: usize = 3;
pub const DISTANCE: usize = 4;
pub const DOUBLE: usize = 5;
pub const WIDTH: usize = 6;
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
            self.drive.as_ptr(),
            self.speaker.as_ptr(),
            self.room.as_ptr(),
            self.size.as_ptr(),
            self.distance.as_ptr(),
            self.double.as_ptr(),
            self.width.as_ptr(),
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

// the two room mics' first reflections: (delay as a multiple of the mic distance, level)
const TAPS_L: [(f64, f64); 4] = [(1.0, 0.7), (1.7, 0.42), (2.4, 0.3), (3.3, 0.2)];
const TAPS_R: [(f64, f64); 4] = [(1.15, 0.68), (1.55, 0.45), (2.7, 0.28), (3.6, 0.19)];

pub struct Dsp {
    fs: f64,
    hp: Biquad,
    spk_lp: Biquad,
    spk_lp2: Biquad,
    spk_body: Biquad,
    spk_mid: Biquad,
    speaker: f64,
    drive: Smooth,
    room_line: DelayLine,
    verb: Fdn,
    size: f64,
    takes: Takes,
    room: Smooth,
    distance: Smooth,
    double: Smooth,
    width: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut hp = Biquad::identity();
        hp.set_highpass(fs, 60.0, 0.6);
        let mut spk_body = Biquad::identity();
        spk_body.set_peak(fs, 130.0, 1.0, 2.5);
        let mut spk_mid = Biquad::identity();
        spk_mid.set_peak(fs, 2200.0, 1.2, 2.0);
        let mut d = Self {
            fs,
            hp,
            spk_lp: Biquad::identity(),
            spk_lp2: Biquad::identity(),
            spk_body,
            spk_mid,
            speaker: -1.0,
            drive: Smooth::new(fs, 30.0, 0.3),
            room_line: DelayLine::new((fs * 0.2) as usize),
            verb: Fdn::new(fs, 1.0),
            size: -1.0,
            takes: Takes::new(fs),
            room: Smooth::new(fs, 30.0, 0.7),
            distance: Smooth::new(fs, 120.0, 12.0),
            double: Smooth::new(fs, 30.0, 0.0),
            width: Smooth::new(fs, 30.0, 1.0),
            out: Smooth::new(fs, 20.0, 1.0),
        };
        d.set_speaker(6500.0);
        d.set_size(0.4);
        d
    }
    fn reset(&mut self) {
        for b in [&mut self.hp, &mut self.spk_lp, &mut self.spk_lp2, &mut self.spk_body, &mut self.spk_mid] {
            b.reset();
        }
        self.room_line.reset();
        self.verb.reset();
        self.takes.reset();
    }
    fn set_params(&mut self, v: &[f32]) {
        self.drive.set(v[DRIVE] as f64);
        self.set_speaker(v[SPEAKER] as f64);
        self.room.set(v[ROOM] as f64);
        self.set_size(v[SIZE] as f64);
        self.distance.set(v[DISTANCE] as f64);
        self.double.set(v[DOUBLE] as f64);
        self.width.set(v[WIDTH] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let mut peak = 0.0f64;
        let cents = Takes::residual_cents(3.3);
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            // the amp: a valve stage pushed by DRIVE (level kept), then the speaker's own band
            let dr = self.drive.next();
            let pre = 1.0 + dr * 6.0;
            let a = tube(self.hp.process(x) * pre * 0.7, 0.12) / (0.7 * (1.0 + dr * 1.6));
            let spk = self.spk_mid.process(self.spk_body.process(self.spk_lp2.process(self.spk_lp.process(a))));
            // the second take: the line played again, the tape a little off speed
            self.takes.write(spk);
            let dbl = self.takes.take(1, 0.006 * self.fs, cents);
            let dv = self.double.next();
            let both = spk + dbl * dv;
            // two microphones out in the room: their first reflections, then the room
            self.room_line.write(both);
            let dist = self.distance.next() * 0.001 * self.fs;
            let (mut ml, mut mr) = (0.0, 0.0);
            for (t, g) in TAPS_L {
                ml += g * self.room_line.read((t * dist).max(1.0));
            }
            for (t, g) in TAPS_R {
                mr += g * self.room_line.read((t * dist).max(1.0));
            }
            let dif = self.verb.diffuse(both, 2);
            let (vl, vr) = self.verb.process(dif);
            ml += vl * 0.6;
            mr += vr * 0.6;
            let room = self.room.next();
            // close: the speaker on the left, the second take on the right
            let cl = spk + dbl * dv * 0.3;
            let cr = spk * (1.0 - 0.7 * dv) + dbl * dv;
            let mut l = cl * (1.0 - room) + ml * room;
            let mut r = cr * (1.0 - room) + mr * room;
            let w = self.width.next();
            let (m, s) = (0.5 * (l + r), 0.5 * (l - r) * w);
            l = m + s;
            r = m - s;
            let o = self.out.next();
            let (yl, yr) = (l * o, r * o);
            peak = peak.max(yl.abs()).max(yr.abs());
            if nch >= 2 {
                chans[0][k] = yl as f32;
                chans[1][k] = yr as f32;
                for c in chans.iter_mut().skip(2) {
                    c[k] = 0.0;
                }
            } else if nch == 1 {
                chans[0][k] = (0.5 * (yl + yr)) as f32;
            }
        }
        meters[0] = peak as f32;
    }
}

impl Dsp {
    fn set_speaker(&mut self, f: f64) {
        if (f - self.speaker).abs() > 1e-3 {
            self.speaker = f;
            self.spk_lp.set_lowpass(self.fs, f, 0.7);
            self.spk_lp2.set_lowpass(self.fs, f * 1.4, 0.6);
        }
    }
    fn set_size(&mut self, s: f64) {
        if (s - self.size).abs() > 1e-3 {
            self.size = s;
            // Polar's studio A: a lively live room, not a hall
            let rt = 0.35 + s * 1.3;
            self.verb.configure(0.25 + s * 0.6, [rt * 1.1, rt, rt * 0.6], 3500.0);
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Lay All Your Love (the ending lead)", values: &[(DRIVE, 0.35), (SPEAKER, 7500.0), (ROOM, 0.45), (SIZE, 0.35), (DISTANCE, 9.0), (DOUBLE, 0.55), (WIDTH, 1.0)] },
        Preset { name: "Arrival Polymoog (two room mics only)", values: &[(DRIVE, 0.2), (SPEAKER, 6000.0), (ROOM, 1.0), (SIZE, 0.5), (DISTANCE, 16.0), (DOUBLE, 0.0), (WIDTH, 1.0)] },
        Preset { name: "GX-1 Strings in the Room", values: &[(DRIVE, 0.15), (SPEAKER, 8000.0), (ROOM, 0.8), (SIZE, 0.6), (DISTANCE, 20.0), (DOUBLE, 0.4), (WIDTH, 1.0)] },
        Preset { name: "Minimoog Played Twice", values: &[(DRIVE, 0.4), (SPEAKER, 6500.0), (ROOM, 0.35), (SIZE, 0.3), (DISTANCE, 8.0), (DOUBLE, 0.8), (WIDTH, 0.9)] },
        Preset { name: "Synth Bass Through the Amp", values: &[(DRIVE, 0.5), (SPEAKER, 4000.0), (ROOM, 0.15), (SIZE, 0.2), (DISTANCE, 5.0), (DOUBLE, 0.0), (WIDTH, 0.4)] },
        Preset { name: "Close and Dry", values: &[(DRIVE, 0.25), (SPEAKER, 9000.0), (ROOM, 0.0), (DOUBLE, 0.0), (WIDTH, 0.0)] },
        Preset { name: "Far Back in Studio A", values: &[(DRIVE, 0.2), (SPEAKER, 5500.0), (ROOM, 1.0), (SIZE, 0.85), (DISTANCE, 34.0), (DOUBLE, 0.3), (WIDTH, 1.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "SYNTH ROOM",
        subtitle: ("BENNY'S SYNTHS AT POLAR", "AMP · TWO ROOM MICS · A SECOND TAKE"),
        accent: ui::PINE,
        size: (W, H),
        items: vec![
            Item::Knob { p: ROOM, x: 90.0, y: 160.0, r: 34.0, label: "ROOM MICS", tip: "From the speaker close up to only the two ambient mics out in the room (how the Polymoog on \"Arrival\" was recorded).", active: None },
            Item::Knob { p: DOUBLE, x: 210.0, y: 160.0, r: 34.0, label: "SECOND TAKE", tip: "The line played again with the tape a little off speed (Tretow's varispeed double), on the other side: Benny's fatter Minimoog.", active: None },
            Item::Knob { p: DRIVE, x: 330.0, y: 160.0, r: 34.0, label: "AMP DRIVE", tip: "How hard the synth pushes the amplifier's valves.", active: None },
            Item::Meter { m: 0, x: 410.0, y: 104.0, w: 190.0, h: 112.0, kind: MeterKind::Vu, caption: "OUTPUT" },
            Item::Divider { y: 264.0 },
            Item::Knob { p: SPEAKER, x: 80.0, y: 330.0, r: 22.0, label: "SPEAKER", tip: "Where the speaker's top end rolls off.", active: None },
            Item::Knob { p: SIZE, x: 190.0, y: 330.0, r: 22.0, label: "ROOM SIZE", tip: "How big and live the room sounds.", active: None },
            Item::Knob { p: DISTANCE, x: 300.0, y: 330.0, r: 22.0, label: "MIC DISTANCE", tip: "How far the room mics stand from the amp.", active: None },
            Item::Knob { p: WIDTH, x: 410.0, y: 330.0, r: 22.0, label: "WIDTH", tip: "How wide the two mics and the second take spread.", active: None },
            Item::Knob { p: OUTPUT, x: 550.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Synth Room", class_id: *b"ABSynthRoomPEQUD", vendor: "PEQUOD ABBA",
    categories: [Fx, Reverb]
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
            let l: Vec<f32> = (0..n).map(|k| if (k / 6000) % 2 == 0 { (nz.next() * 0.5) as f32 } else { 0.0 }).collect();
            let mut input = vec![l.clone(), l];
            run(&p, &mut input);
            assert!(input.iter().flatten().all(|v| v.is_finite() && v.abs() < 8.0), "unbounded output for {:?}", p);
        }
    }


    /// The room mics and the second take make it stereo; close and dry stays mono.
    #[test]
    fn room_and_take_widen() {
        let side_ratio = |set: &[(usize, f32)]| {
            let p = with(defaults(), set);
            let s = sine(440.0, 0.3, 96000);
            let mut input = vec![s.clone(), s];
            run(&p, &mut input);
            let (l, r) = (&input[0][48000..], &input[1][48000..]);
            let side: Vec<f32> = l.iter().zip(r).map(|(a, b)| a - b).collect();
            rms(&side) / rms(l).max(1e-9)
        };
        let dry = side_ratio(&[(ROOM, 0.0), (DOUBLE, 0.0)]);
        let wet = side_ratio(&[(ROOM, 1.0), (DOUBLE, 0.5)]);
        println!("side/left: dry {dry:.3}, room + take {wet:.3}");
        assert!(dry < 1e-3 && wet > 0.2);
    }
}
