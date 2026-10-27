//! Vatos Kit: a drum-bus processor for tight early-80s new-wave drums like Johnny 'Vatos' Hernandez's on Only a Lad: TIGHT gates each hit short, CRACK and THUMP set the snare's bite and the kick's weight, ROOM adds a room that is gated shut after each hit (the gated sound of 1980-81), and SMASH blends in a squashed copy. Settings estimated.

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
use vintage::fx::comp_gr;
use vintage::verb::Fdn;

pub const W: u32 = 640;
pub const H: u32 = 410;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "tight"]
    pub tight: FloatParam,
    #[id = "crack"]
    pub crack: FloatParam,
    #[id = "thump"]
    pub thump: FloatParam,
    #[id = "room"]
    pub room: FloatParam,
    #[id = "smash"]
    pub smash: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            tight: pct("Tight", 0.5),
            crack: db("Crack", 3.0, -6.0, 9.0),
            thump: db("Thump", 2.0, -6.0, 9.0),
            room: pct("Room", 0.3),
            smash: pct("Smash", 0.2),
            output: db("Output", -1.0, -24.0, 12.0),
        }
    }
}

pub const TIGHT: usize = 0;
pub const CRACK: usize = 1;
pub const THUMP: usize = 2;
pub const ROOM: usize = 3;
pub const SMASH: usize = 4;
pub const OUTPUT: usize = 5;
pub const NPARAMS: usize = 6;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.tight.as_ptr(),
            self.crack.as_ptr(),
            self.thump.as_ptr(),
            self.room.as_ptr(),
            self.smash.as_ptr(),
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
    v: [f32; NPARAMS],
    eq: Vec<[Biquad; 3]>,
    env: f64,
    peak: f64,
    lock: usize,
    hold: usize,
    gate: f64,
    room_hold: usize,
    room_gate: f64,
    rms: f64,
    gr: f64,
    fdn: Fdn,
    key: (i32, i32),
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let (cr, th) = (self.v[CRACK] as f64, self.v[THUMP] as f64);
        for e in self.eq.iter_mut() {
            e[0].set_low_shelf(fs, 70.0, 0.7, th);
            e[1].set_peak(fs, 200.0, 1.2, 0.5 * cr);
            e[2].set_peak(fs, 5000.0, 0.9, cr);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut fdn = Fdn::new(fs, 1.6);
        fdn.configure(1.5, [1.6, 1.4, 1.0], 5000.0);
        let mut d = Self { fs, v, eq: vec![[Biquad::identity(); 3]; ch.max(2)], env: 0.0, peak: 0.0, lock: 0, hold: 0, gate: 0.0, room_hold: 0, room_gate: 0.0,
            rms: 0.0, gr: 0.0, fdn, key: (-999, -999), out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.eq.iter_mut().for_each(|e| e.iter_mut().for_each(|b| b.reset()));
        self.fdn.reset();
        self.env = 0.0;
        self.peak = 0.0;
        self.lock = 0;
        self.hold = 0;
        self.gate = 0.0;
        self.room_hold = 0;
        self.room_gate = 0.0;
        self.rms = 0.0;
        self.gr = 0.0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = ((v[CRACK] * 10.0) as i32, (v[THUMP] * 10.0) as i32);
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let tight = self.v[TIGHT] as f64;
        let room = self.v[ROOM] as f64;
        let smash = self.v[SMASH] as f64;
        let hold = ((0.03 + 0.35 * (1.0 - tight)) * self.fs) as usize;
        let rhold = (0.22 * self.fs) as usize;
        let floor = db_to_gain(-30.0 * tight);
        let ca = coeff(self.fs, 0.3);
        let cr = coeff(self.fs, 30.0);
        let cs = coeff(self.fs, 60.0);
        let lockout = (0.03 * self.fs) as usize;
        let go = coeff(self.fs, 0.5);
        let gc = coeff(self.fs, 25.0);
        let rc = coeff(self.fs, 15.0);
        let cav = coeff(self.fs, 3.0);
        let sa = coeff(self.fs, 0.3);
        let sr = coeff(self.fs, 60.0);
        let last = self.eq.len() - 1;
        let mut most = 0.0f64;
        for k in 0..n {
            let mut a = 0.0f64;
            let mut key = 0.0;
            for c in chans.iter() {
                let x = c[k] as f64;
                a = a.max(x.abs());
                key += x * x;
            }
            key /= nch.max(1) as f64;
            self.env = a + if a > self.env { ca } else { cr } * (self.env - a);
            // slow envelope: the fast one leaping well above it is a new hit
            self.peak = self.env + cs * (self.peak - self.env);
            if self.lock > 0 {
                self.lock -= 1;
            } else if self.env > 0.002 && self.env > 2.0 * self.peak + 0.001 {
                self.hold = hold;
                self.room_hold = rhold;
                self.lock = lockout;
            }
            let gt = if self.hold > 0 { self.hold -= 1; 1.0 } else { floor };
            self.gate = gt + if gt > self.gate { go } else { gc } * (self.gate - gt);
            let rt = if self.room_hold > 0 { self.room_hold -= 1; 1.0 } else { 0.0 };
            self.room_gate = rt + if rt > self.room_gate { go } else { rc } * (self.room_gate - rt);
            self.rms = key + cav * (self.rms - key);
            let lev = 10.0 * self.rms.max(1e-12).log10() + 3.0;
            let t = comp_gr(lev, -28.0, 12.0, 3.0);
            self.gr = t + if t > self.gr { sa } else { sr } * (self.gr - t);
            most = most.max(self.gr * smash);
            let sm = db_to_gain(-self.gr + 12.0);
            let mut ys = [0.0f64; 2];
            let mut mono = 0.0;
            for (ci, c) in chans.iter().enumerate().take(2) {
                let e = cascade(&mut self.eq[ci.min(last)], c[k] as f64);
                mono += e;
                ys[ci] = e * self.gate + smash * 0.4 * e * sm;
            }
            if nch == 1 {
                ys[1] = ys[0];
            }
            mono /= nch.clamp(1, 2) as f64;
            let dx = self.fdn.diffuse(mono, 3);
            let (rl, rr) = self.fdn.process(dx);
            let o = self.out.next();
            let l = (0.98 * ((ys[0] + room * 0.9 * rl * self.room_gate) / 0.98).tanh()) * o;
            let r = (0.98 * ((ys[1] + room * 0.9 * rr * self.room_gate) / 0.98).tanh()) * o;
            if nch >= 2 {
                chans[0][k] = l as f32;
                chans[1][k] = r as f32;
            } else {
                chans[0][k] = (0.5 * (l + r)) as f32;
            }
        }
        meters[0] = most as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Only a Lad Kit", values: &[] },
        Preset { name: "Very Tight New Wave", values: &[(TIGHT, 0.85), (ROOM, 0.15), (CRACK, 5.0)] },
        Preset { name: "Big Gated Room", values: &[(ROOM, 0.8), (TIGHT, 0.6), (SMASH, 0.3)] },
        Preset { name: "Ska Snare", values: &[(CRACK, 7.0), (THUMP, 0.0), (TIGHT, 0.6)] },
        Preset { name: "Smashed Room", values: &[(SMASH, 0.8), (ROOM, 0.4), (TIGHT, 0.3), (OUTPUT, -4.0)] },
        Preset { name: "Natural Kit", values: &[(TIGHT, 0.1), (ROOM, 0.25), (CRACK, 1.0), (THUMP, 1.0), (SMASH, 0.1)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "VATOS KIT",
        subtitle: ("TIGHT EARLY-80s NEW-WAVE DRUMS", "TIGHT · GATED ROOM · CRACK · THUMP · SMASH"),
        accent: ui::MUSTARD,
        size: (W, H),
        items: vec![

            Item::Knob { p: TIGHT, x: 90.0, y: 160.0, r: 38.0, label: "TIGHT", tip: "Gates each hit short: higher = shorter and deeper.", active: None },
            Item::Knob { p: ROOM, x: 220.0, y: 160.0, r: 32.0, label: "ROOM", tip: "A room that is gated shut after each hit.", active: None },
            Item::Meter { m: 0, x: 300.0, y: 96.0, w: 160.0, h: 110.0, kind: MeterKind::GainReduction, caption: "SMASH" },
            Item::Knob { p: SMASH, x: 550.0, y: 160.0, r: 32.0, label: "SMASH", tip: "Blend of a squashed copy (parallel compression).", active: None },
            Item::Divider { y: 240.0 },
            Item::Knob { p: THUMP, x: 150.0, y: 305.0, r: 26.0, label: "THUMP", tip: "Kick weight (70 Hz shelf).", active: None },
            Item::Knob { p: CRACK, x: 320.0, y: 305.0, r: 26.0, label: "CRACK", tip: "Snare body (200 Hz) and bite (5 kHz).", active: None },
            Item::Knob { p: OUTPUT, x: 490.0, y: 305.0, r: 26.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Vatos Kit", class_id: *b"OBVatosKitPQD001", vendor: "PEQUOD Oingo Boingo",
    categories: [Fx, Dynamics]
);

#[cfg(test)]
#[allow(dead_code)]
pub mod tests {
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

    fn hits() -> Vec<f32> {
        let mut nz = vintage::dsp::Noise::new(4);
        let mut x = vec![0.0f32; 48000];
        for h in 0..2 {
            for k in 0..24000 {
                let t = k as f64 / 48000.0;
                x[h * 24000 + k] = (0.4 * (1.0 - (-t / 0.002).exp()) * (-t / 0.25).exp() * (0.6 * (2.0 * std::f64::consts::PI * 180.0 * t).sin() + 0.4 * nz.next())) as f32;
            }
        }
        x
    }
    #[test]
    fn tight_gates_the_ring() {
        let x = hits();
        let base = with(defaults(), &[(ROOM, 0.0), (SMASH, 0.0), (CRACK, 0.0), (THUMP, 0.0), (OUTPUT, 0.0)]);
        let mut a = vec![x.clone(), x.clone()];
        run(&with(base.clone(), &[(TIGHT, 0.0)]), &mut a);
        let mut b = vec![x.clone(), x];
        run(&with(base, &[(TIGHT, 1.0)]), &mut b);
        let tail = |y: &[f32]| db(rms(&y[24000 + 9600..24000 + 19200])) - db(rms(&y[24000..24000 + 960]));
        println!("tail vs attack: tight 0 {:.1} dB, tight 1 {:.1} dB", tail(&a[0]), tail(&b[0]));
        assert!(tail(&a[0]) - tail(&b[0]) > 15.0);
    }

}
