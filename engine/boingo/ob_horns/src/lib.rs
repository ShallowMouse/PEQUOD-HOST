//! Mystic Horns: a bus processor for a horn section like Oingo Boingo's (tenor and baritone sax and trumpet, arranged by Steve Bartek): SECTION compression glues the three into one punchy instrument, BRASS adds bite and BODY weight, STACK doubles the section left and right with slightly loose copies for a bigger band, and ROOM puts it in a short, bright space. Settings estimated.

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
    #[id = "section"]
    pub section: FloatParam,
    #[id = "brass"]
    pub brass: FloatParam,
    #[id = "body"]
    pub body: FloatParam,
    #[id = "stack"]
    pub stack: FloatParam,
    #[id = "room"]
    pub room: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            section: pct("Section", 0.5),
            brass: db("Brass", 3.0, -6.0, 9.0),
            body: db("Body", 0.0, -6.0, 6.0),
            stack: pct("Stack", 0.0),
            room: pct("Room", 0.2),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const SECTION: usize = 0;
pub const BRASS: usize = 1;
pub const BODY: usize = 2;
pub const STACK: usize = 3;
pub const ROOM: usize = 4;
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
            self.section.as_ptr(),
            self.brass.as_ptr(),
            self.body.as_ptr(),
            self.stack.as_ptr(),
            self.room.as_ptr(),
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
    rms: f64,
    gr: f64,
    lines: [DelayLine; 2],
    w: [Wander; 2],
    fdn: Fdn,
    key: (i32, i32),
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let (br, bo) = (self.v[BRASS] as f64, self.v[BODY] as f64);
        for e in self.eq.iter_mut() {
            e[0].set_highpass(fs, 70.0, 0.7);
            e[1].set_peak(fs, 300.0, 0.9, bo);
            e[2].set_peak(fs, 3200.0, 0.8, br);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut fdn = Fdn::new(fs, 1.0);
        fdn.configure(0.8, [0.9, 0.8, 0.6], 6000.0);
        let mut d = Self { fs, v, eq: vec![[Biquad::identity(); 3]; ch.max(2)], rms: 0.0, gr: 0.0,
            lines: [DelayLine::new((0.05 * fs) as usize), DelayLine::new((0.05 * fs) as usize)],
            w: [Wander::new(fs, 0.4, 5), Wander::new(fs, 0.33, 17)], fdn, key: (-999, -999), out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.eq.iter_mut().for_each(|e| e.iter_mut().for_each(|b| b.reset()));
        self.lines.iter_mut().for_each(|l| l.reset());
        self.fdn.reset();
        self.rms = 0.0;
        self.gr = 0.0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = ((v[BRASS] * 10.0) as i32, (v[BODY] * 10.0) as i32);
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let sec = self.v[SECTION] as f64;
        let thr = -8.0 - 26.0 * sec;
        let stack = self.v[STACK] as f64;
        let room = self.v[ROOM] as f64;
        let cav = coeff(self.fs, 8.0);
        let ca = coeff(self.fs, 4.0);
        let cr = coeff(self.fs, 120.0);
        let (d1, d2) = (0.014 * self.fs, 0.023 * self.fs);
        let sw = 0.0006 * self.fs;
        let last = self.eq.len() - 1;
        let mut most = 0.0f64;
        for k in 0..n {
            let mut key = 0.0;
            for c in chans.iter() {
                key += (c[k] as f64).powi(2);
            }
            key /= nch.max(1) as f64;
            self.rms = key + cav * (self.rms - key);
            let lev = 10.0 * self.rms.max(1e-12).log10() + 3.0;
            let t = comp_gr(lev, thr, 4.0, 6.0);
            self.gr = t + if t > self.gr { ca } else { cr } * (self.gr - t);
            most = most.max(self.gr);
            let g = db_to_gain(-self.gr + 0.4 * self.gr.min(12.0));
            let o = self.out.next();
            let mut y = [0.0f64; 2];
            for (ci, c) in chans.iter().enumerate().take(2) {
                y[ci] = cascade(&mut self.eq[ci.min(last)], c[k] as f64) * g;
            }
            if nch == 1 {
                y[1] = y[0];
            }
            let mono = 0.5 * (y[0] + y[1]);
            self.lines[0].write(mono);
            self.lines[1].write(mono);
            let a = self.lines[0].read(d1 + sw * self.w[0].next());
            let b = self.lines[1].read(d2 + sw * self.w[1].next());
            let dx = self.fdn.diffuse(mono, 2);
            let (rl, rr) = self.fdn.process(dx);
            let l = y[0] + stack * 0.75 * a + room * 0.5 * rl;
            let r = y[1] + stack * 0.75 * b + room * 0.5 * rr;
            if nch >= 2 {
                chans[0][k] = (l * o) as f32;
                chans[1][k] = (r * o) as f32;
            } else {
                chans[0][k] = (0.5 * (l + r) * o) as f32;
            }
        }
        meters[0] = most as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Boingo Section", values: &[] },
        Preset { name: "Tight and Punchy", values: &[(SECTION, 0.75), (BRASS, 5.0), (ROOM, 0.1)] },
        Preset { name: "Big Band Stack", values: &[(STACK, 0.8), (ROOM, 0.3), (BODY, 2.0)] },
        Preset { name: "Nasty Habits Klezmer Horns", values: &[(SECTION, 0.4), (BRASS, 4.0), (BODY, -2.0), (ROOM, 0.35)] },
        Preset { name: "Bari Weight", values: &[(BODY, 4.0), (BRASS, 1.0)] },
        Preset { name: "Ska Stabs", values: &[(SECTION, 0.65), (BRASS, 6.0), (STACK, 0.4), (ROOM, 0.15)] },
        Preset { name: "Natural (light touch)", values: &[(SECTION, 0.2), (BRASS, 0.0), (ROOM, 0.25)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "MYSTIC HORNS",
        subtitle: ("FOR A HORN SECTION LIKE BOINGO'S", "SECTION · BRASS · BODY · STACK · ROOM"),
        accent: ui::STEEL,
        size: (W, H),
        items: vec![

            Item::Knob { p: SECTION, x: 90.0, y: 160.0, r: 38.0, label: "SECTION", tip: "Compression that glues the horns into one instrument.", active: None },
            Item::Knob { p: BRASS, x: 220.0, y: 160.0, r: 32.0, label: "BRASS", tip: "Bite at 3.2 kHz.", active: None },
            Item::Meter { m: 0, x: 300.0, y: 96.0, w: 160.0, h: 110.0, kind: MeterKind::GainReduction, caption: "SECTION" },
            Item::Knob { p: BODY, x: 550.0, y: 160.0, r: 32.0, label: "BODY", tip: "Weight at 300 Hz.", active: None },
            Item::Divider { y: 240.0 },
            Item::Knob { p: STACK, x: 150.0, y: 305.0, r: 26.0, label: "STACK", tip: "Doubles the section left and right with loose copies.", active: None },
            Item::Knob { p: ROOM, x: 320.0, y: 305.0, r: 26.0, label: "ROOM", tip: "A short, bright room.", active: None },
            Item::Knob { p: OUTPUT, x: 490.0, y: 305.0, r: 26.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Mystic Horns", class_id: *b"OBMysticHornsPQ1", vendor: "PEQUOD Oingo Boingo",
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

    use std::f64::consts::PI;
    fn goertzel(x: &[f32], f: f64) -> f64 {
        let w = 2.0 * PI * f / 48000.0;
        let (mut s1, mut s2) = (0.0, 0.0);
        for v in x {
            let s = *v as f64 + 2.0 * w.cos() * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        ((s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2).max(0.0)).sqrt() / x.len() as f64 * 2.0
    }
    fn stereo(p: &[f32], x: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        let r = input.pop().unwrap();
        (input.pop().unwrap(), r)
    }
    fn corr(a: &[f32], b: &[f32]) -> f64 {
        let ab: f64 = a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum();
        let aa: f64 = a.iter().map(|x| (*x as f64).powi(2)).sum();
        let bb: f64 = b.iter().map(|x| (*x as f64).powi(2)).sum();
        ab / (aa * bb).sqrt().max(1e-12)
    }
    /// short-window rms envelope
    fn env(x: &[f32], w: usize) -> Vec<f64> {
        x.chunks(w).map(rms).collect()
    }

    #[test]
    fn glues_and_stacks() {
        let p = with(defaults(), &[(SECTION, 0.9), (ROOM, 0.0)]);
        let loud = db(rms(&stereo(&p, &sine(500.0, 0.5, 48000)).0[24000..]));
        let quiet = db(rms(&stereo(&p, &sine(500.0, 0.05, 48000)).0[24000..]));
        let (l, r) = stereo(&with(defaults(), &[(STACK, 1.0), (ROOM, 0.0)]), &sine(500.0, 0.2, 48000));
        let c = corr(&l[24000..], &r[24000..]);
        println!("20 dB in -> {:.1} dB out; stacked L/R corr {c:.2}", loud - quiet);
        assert!(loud - quiet < 13.0 && c < 0.95);
    }

}
