//! Listen Mic: the In the Air Tonight / Intruder drum sound. A live stone room picked up by the console's talkback 'listen mic', crushed by its very heavy compressor, then cut off by a channel gate keyed from the close drums. Hugh Padgham and Steve Lillywhite found it at the Townhouse in 1980 when the reverse talkback was opened while Phil Collins played.

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
use vintage::fx::Gate;

pub const W: u32 = 640;
pub const H: u32 = 432;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Room { #[name = "Stone Room"] Stone, #[name = "Big Live Room"] Big, #[name = "Small Hard Room"] Small }
// FDN size and decay (s, low/mid/high) per room: estimated (very live stone walls)
const ROOMS: [(f64, [f64; 3]); 3] = [(0.8, [1.4, 1.3, 1.0]), (1.3, [1.9, 1.7, 1.2]), (0.45, [0.9, 0.8, 0.6])];


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "room"]
    pub room: EnumParam<Room>,
    #[id = "squash"]
    pub squash: FloatParam,
    #[id = "thresh"]
    pub thresh: FloatParam,
    #[id = "hold"]
    pub hold: FloatParam,
    #[id = "release"]
    pub release: FloatParam,
    #[id = "roomlvl"]
    pub room_lvl: FloatParam,
    #[id = "dry"]
    pub dry: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            room: EnumParam::new("Room", Room::Stone),
            squash: pct("Listen Compression", 0.8),
            thresh: db("Gate Threshold", -30.0, -60.0, 0.0),
            hold: ms("Gate Hold", 400.0, 20.0, 2000.0),
            release: ms("Gate Release", 40.0, 2.0, 1000.0),
            room_lvl: db("Room Level", 0.0, -30.0, 12.0),
            dry: db("Close Mics", 0.0, -60.0, 6.0),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const ROOM: usize = 0;
pub const SQUASH: usize = 1;
pub const THRESH: usize = 2;
pub const HOLD: usize = 3;
pub const RELEASE: usize = 4;
pub const ROOMLVL: usize = 5;
pub const DRY: usize = 6;
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
            self.room.as_ptr(),
            self.squash.as_ptr(),
            self.thresh.as_ptr(),
            self.hold.as_ptr(),
            self.release.as_ptr(),
            self.room_lvl.as_ptr(),
            self.dry.as_ptr(),
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
    fdn: Fdn,
    room: usize,
    gate: Gate,
    env: f64,
    gr: f64,
    hp: [Biquad; 2],
    squash: f64,
    thresh: f64,
    hold: f64,
    rel: f64,
    room_lvl: Smooth,
    dry: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 2;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut fdn = Fdn::new(fs, 1.4);
        fdn.configure(ROOMS[0].0, ROOMS[0].1, 4000.0);
        let mut gate = Gate::new(fs);
        gate.set_times(0.05, 40.0);
        let mut hp = [Biquad::identity(); 2];
        for h in &mut hp {
            h.set_highpass(fs, 60.0, 0.7);
        }
        Self {
            fs,
            fdn,
            room: 0,
            gate,
            env: 0.0,
            gr: 1.0,
            hp,
            squash: 0.8,
            thresh: -30.0,
            hold: 400.0,
            rel: 40.0,
            room_lvl: Smooth::new(fs, 20.0, 1.0),
            dry: Smooth::new(fs, 20.0, 1.0),
            out: Smooth::new(fs, 20.0, 1.0),
        }
    }
    fn reset(&mut self) {
        self.fdn.reset();
        self.gate.reset();
        self.env = 0.0;
        self.gr = 1.0;
        self.hp.iter_mut().for_each(|h| h.reset());
    }
    fn set_params(&mut self, v: &[f32]) {
        let r = (v[ROOM].round() as usize).min(2);
        if r != self.room {
            self.room = r;
            self.fdn.configure(ROOMS[r].0, ROOMS[r].1, 4000.0);
        }
        self.squash = v[SQUASH] as f64;
        self.thresh = v[THRESH] as f64;
        self.hold = v[HOLD] as f64;
        let rel = v[RELEASE] as f64;
        if (rel - self.rel).abs() > 1e-3 {
            self.rel = rel;
            self.gate.set_times(0.05, rel);
        }
        self.room_lvl.set(db_to_gain(v[ROOMLVL] as f64));
        self.dry.set(db_to_gain(v[DRY] as f64));
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let mut peak = 0.0f64;
        let mut grmax = 0.0f64;
        let mut gate_open = 0.0f64;
        let att = 1.0 - coeff(self.fs, 0.3);
        let rel = 1.0 - coeff(self.fs, 120.0);
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            // the room, heard by one mic hanging in it
            let (rl, rr) = { let dif = self.fdn.diffuse(x, 2); self.fdn.process(dif) };
            let room = 0.5 * (rl + rr) * 3.0;
            // talkback compressor: fast, heavy, effectively a limiter with lots of make-up
            let lvl = room.abs();
            self.env += (lvl - self.env) * if lvl > self.env { att } else { rel };
            let thr = db_to_gain(-6.0 - 30.0 * self.squash);
            let g = if self.env > thr { (thr / self.env).powf(0.95) } else { 1.0 };
            self.gr = g;
            grmax = grmax.max(-gain_to_db(g));
            let makeup = db_to_gain(30.0 * self.squash * 0.85);
            let squashed = room * g * makeup;
            // channel gate keyed by the close drums
            let gg = self.gate.process(x, self.thresh, self.hold, 90.0);
            gate_open = gate_open.max(gg);
            let wet = squashed * gg * self.room_lvl.next();
            let o = self.out.next();
            let d = self.dry.next();
            let wl = self.hp[0].process(wet + 0.15 * (rl - rr) * gg * g * makeup);
            let wr = self.hp[1].process(wet - 0.15 * (rl - rr) * gg * g * makeup);
            for (ci, c) in chans.iter_mut().enumerate() {
                let w = if nch == 1 { 0.5 * (wl + wr) } else if ci == 0 { wl } else { wr };
                let y = (c[k] as f64 * d + w) * o;
                peak = peak.max(y.abs());
                c[k] = y as f32;
            }
        }
        meters[0] = grmax as f32;
        meters[1] = gate_open as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "In the Air Tonight", values: &[(ROOM, 0.0), (SQUASH, 0.9), (THRESH, -28.0), (HOLD, 450.0), (RELEASE, 30.0), (ROOMLVL, 0.0), (DRY, -6.0)] },
        Preset { name: "Intruder (no cymbals!)", values: &[(ROOM, 0.0), (SQUASH, 0.85), (THRESH, -30.0), (HOLD, 350.0), (RELEASE, 20.0), (DRY, -4.0)] },
        Preset { name: "Long Gate (1.5 s)", values: &[(ROOM, 1.0), (SQUASH, 0.8), (HOLD, 1500.0), (RELEASE, 60.0)] },
        Preset { name: "Snare Explosion", values: &[(ROOM, 0.0), (SQUASH, 1.0), (THRESH, -20.0), (HOLD, 250.0), (RELEASE, 15.0), (ROOMLVL, 3.0)] },
        Preset { name: "Big Room, Open", values: &[(ROOM, 1.0), (SQUASH, 0.6), (HOLD, 2000.0), (RELEASE, 400.0)] },
        Preset { name: "Tight 80s Toms", values: &[(ROOM, 2.0), (SQUASH, 0.8), (HOLD, 200.0), (RELEASE, 20.0)] },
        Preset { name: "Room Only (for a bus)", values: &[(DRY, -60.0)] },
        Preset { name: "Gentle Room Lift", values: &[(SQUASH, 0.4), (HOLD, 600.0), (RELEASE, 200.0), (ROOMLVL, -8.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "LISTEN MIC",
        subtitle: ("THE GATED ROOM DRUM SOUND, 1980", "TALKBACK COMPRESSOR · KEYED GATE"),
        accent: ui::MUSTARD,
        size: (W, H),
        items: vec![

            Item::Selector { p: ROOM, x: 80.0, y: 160.0, label: "ROOM", positions: &["STONE", "BIG", "SMALL"], tip: "The live room the listen mic hangs in. Stone: the Townhouse's stone-walled room (decay estimated)." },
            Item::Knob { p: SQUASH, x: 200.0, y: 160.0, r: 40.0, label: "LISTEN COMP", tip: "The talkback mic's compressor: built only to make voices audible, it is fast and very heavy. This is what blows the room up.", active: None },
            Item::Meter { m: 0, x: 280.0, y: 100.0, w: 170.0, h: 110.0, kind: MeterKind::GainReduction, caption: "LISTEN COMP" },
            Item::Lamp { m: 1, x: 480.0, y: 120.0, label: "GATE", lo: 0.1, hi: 0.9 },
            Item::Knob { p: ROOMLVL, x: 560.0, y: 175.0, r: 30.0, label: "ROOM LEVEL", tip: "Level of the gated, compressed room.", active: None },
            Item::Divider { y: 264.0 },
            Item::Knob { p: THRESH, x: 80.0, y: 330.0, r: 22.0, label: "THRESHOLD", tip: "Gate threshold on the close drums. Each hit opens the room.", active: None },
            Item::Knob { p: HOLD, x: 180.0, y: 330.0, r: 22.0, label: "HOLD", tip: "How long the gate stays open after each hit. Collins played to a click so it shut consistently.", active: None },
            Item::Knob { p: RELEASE, x: 280.0, y: 330.0, r: 22.0, label: "RELEASE", tip: "How quickly it shuts: short = the abrupt 'shutdown'.", active: None },
            Item::Knob { p: DRY, x: 420.0, y: 330.0, r: 22.0, label: "CLOSE MICS", tip: "The close drums (dry input).", active: None },
            Item::Knob { p: OUTPUT, x: 540.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Listen Mic", class_id: *b"TPListenMicPQD01", vendor: "PEQUOD Padgham & Lillywhite",
    categories: [Fx, Reverb, Dynamics]
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
    fn room_is_cut_off_after_hold() {
        let p = with(defaults(), &[(DRY, -60.0), (HOLD, 300.0), (RELEASE, 20.0)]);
        let mut x = vec![0.0f32; 48000];
        for v in x.iter_mut().take(240) {
            *v = 0.6;
        }
        let mut input = vec![x];
        run(&p, &mut input);
        let during = rms(&input[0][4800..12000]);
        let after = rms(&input[0][24000..36000]);
        println!("room during hold {:.1} dB, after {:.1} dB", db(during), db(after));
        assert!(during > 0.01 && db(after / during) < -40.0);
    }

}
