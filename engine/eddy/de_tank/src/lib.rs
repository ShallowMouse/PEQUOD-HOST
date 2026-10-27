//! Water Tank: the echo chamber on Duane Eddy's records: a 2,000-gallon steel water tank found in a junkyard by the Salt River, with a speaker at one end and a microphone at the other, at Audio Recorders in Phoenix. A long, metallic, ringing echo: steel walls keep the highs, and the sound flutters between the tank's ends. RING sets how much of that flutter you hear. Dimensions estimated (about 3.4 m long).

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
pub const H: u32 = 320;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "ring"]
    pub ring: FloatParam,
    #[id = "pre"]
    pub pre: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            decay: secs("Decay", 3.0, 0.8, 8.0),
            ring: pct("Ring", 0.5),
            pre: ms("Pre-Delay", 10.0, 0.0, 120.0),
            mix: pct("Mix", 0.3),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const DECAY: usize = 0;
pub const RING: usize = 1;
pub const PRE: usize = 2;
pub const MIX: usize = 3;
pub const OUTPUT: usize = 4;
pub const NPARAMS: usize = 5;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.decay.as_ptr(),
            self.ring.as_ptr(),
            self.pre.as_ptr(),
            self.mix.as_ptr(),
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

/// Round trips along and across the tank (s): about 3.4 m long, 1.7 m across.
const COMBS: [f64; 2] = [0.0198, 0.0099];

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    fdn: Fdn,
    pre: DelayLine,
    combs: [DelayLine; 2],
    cg: [f64; 2],
    damp: [OnePole; 2],
    send: [Biquad; 2],
    key: i32,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let rt = self.v[DECAY] as f64;
        // steel: the highs last almost as long as the mids
        self.fdn.configure(0.55, [rt * 0.8, rt, rt * 0.85], 5000.0);
        self.fdn.mod_depth = 0.0002;
        for i in 0..2 {
            self.cg[i] = 10f64.powf(-3.0 * COMBS[i] / rt.max(0.1));
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut send = [Biquad::identity(); 2];
        send[0].set_highpass(fs, 150.0, 0.7);
        send[1].set_lowpass(fs, 6000.0, 0.7);
        let mut damp = [OnePole::default(); 2];
        damp.iter_mut().for_each(|d| d.set_lowpass(fs, 5500.0));
        let mut d = Self { fs, v, fdn: Fdn::new(fs, 0.6), pre: DelayLine::new((0.13 * fs) as usize),
            combs: [DelayLine::new((0.03 * fs) as usize), DelayLine::new((0.03 * fs) as usize)], cg: [0.0; 2], damp, send, key: -1, out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.fdn.reset();
        self.pre.reset();
        self.combs.iter_mut().for_each(|c| c.reset());
        self.send.iter_mut().for_each(|b| b.reset());
        self.design();
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = (v[DECAY] * 100.0) as i32;
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let mix = self.v[MIX] as f64;
        let ring = self.v[RING] as f64;
        let (dg, wg) = ((1.0 - mix).sqrt(), mix.sqrt() * 1.5);
        let pd = (self.v[PRE] as f64 * 0.001 * self.fs).max(1.0);
        let cd = [COMBS[0] * self.fs, COMBS[1] * self.fs];
        let nch = chans.len();
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            self.pre.write(cascade(&mut self.send, x));
            let s = self.pre.read(pd);
            // flutter between the ends and across the tank
            let mut fl = 0.0;
            for i in 0..2 {
                let y = self.damp[i].lp(self.combs[i].read(cd[i]));
                self.combs[i].write(s + self.cg[i] * y);
                fl += y;
            }
            let dx = self.fdn.diffuse(s + ring * 0.3 * fl, 2);
            let (l, r) = self.fdn.process(dx);
            let o = self.out.next();
            let wl = l + ring * 0.25 * fl;
            let wr = r + ring * 0.25 * fl;
            if nch >= 2 {
                chans[0][k] = ((chans[0][k] as f64 * dg + wl * wg) * o) as f32;
                chans[1][k] = ((chans[1][k] as f64 * dg + wr * wg) * o) as f32;
            } else {
                chans[0][k] = ((chans[0][k] as f64 * dg + 0.5 * (wl + wr) * wg) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Rebel Rouser Tank", values: &[] },
        Preset { name: "Big Ringing Tank", values: &[(DECAY, 5.0), (RING, 0.8), (MIX, 0.35)] },
        Preset { name: "Short Tank Slap", values: &[(DECAY, 1.4), (PRE, 40.0), (MIX, 0.3)] },
        Preset { name: "Subtle Steel", values: &[(RING, 0.25), (MIX, 0.18)] },
        Preset { name: "Tank Only (send)", values: &[(MIX, 1.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "WATER TANK",
        subtitle: ("DUANE EDDY'S 2,000-GALLON ECHO TANK", "AUDIO RECORDERS · PHOENIX"),
        accent: ui::PINE,
        size: (W, H),
        items: vec![

            Item::Knob { p: DECAY, x: 90.0, y: 180.0, r: 38.0, label: "DECAY", tip: "How long the tank rings on.", active: None },
            Item::Knob { p: RING, x: 220.0, y: 180.0, r: 32.0, label: "RING", tip: "The metallic flutter between the tank's ends.", active: None },
            Item::Knob { p: PRE, x: 340.0, y: 180.0, r: 32.0, label: "PRE-DELAY", tip: "A gap before the echo.", active: None },
            Item::Knob { p: MIX, x: 460.0, y: 180.0, r: 32.0, label: "MIX", tip: "Tank level.", active: None },
            Item::Knob { p: OUTPUT, x: 570.0, y: 180.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Water Tank", class_id: *b"DEWaterTankEcho1", vendor: "PEQUOD Duane Eddy",
    categories: [Fx, Reverb]
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

    #[test]
    fn decay_follows_the_knob() {
        let mut x = vec![0.0f32; 48000 * 8];
        x[100] = 1.0;
        let mut a = vec![x.clone(), x.clone()];
        run(&with(defaults(), &[(DECAY, 1.5), (MIX, 1.0)]), &mut a);
        let mut b = vec![x.clone(), x];
        run(&with(defaults(), &[(DECAY, 5.0), (MIX, 1.0)]), &mut b);
        let ta = vintage::verb::t30(&a[0].iter().map(|v| *v as f64).collect::<Vec<_>>(), 48000.0, 1000.0);
        let tb = vintage::verb::t30(&b[0].iter().map(|v| *v as f64).collect::<Vec<_>>(), 48000.0, 1000.0);
        println!("T30 at 1 kHz: decay 1.5 -> {ta:.2} s, decay 5 -> {tb:.2} s");
        assert!(tb > ta * 2.0);
    }

}
