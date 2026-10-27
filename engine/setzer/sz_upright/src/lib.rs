//! Upright Slap: pushes an electric bass DI toward a rockabilly upright (Lee Rocker's slapped double bass): wood body, short gut-string notes, and the slap click on every note found. Voicing estimated.

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



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "click"]
    pub click: FloatParam,
    #[id = "ctone"]
    pub ctone: FloatParam,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "body"]
    pub body: FloatParam,
    #[id = "wood"]
    pub wood: FloatParam,
    #[id = "sens"]
    pub sens: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            click: db("Slap Click", -6.0, -40.0, 6.0),
            ctone: hz("Click Tone", 3000.0, 1200.0, 7000.0),
            decay: ms("Note Length", 450.0, 120.0, 2000.0),
            body: pct("Body", 0.6),
            wood: pct("Wood", 0.6),
            sens: db("Sensitivity", 5.0, 2.0, 15.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const CLICK: usize = 0;
pub const CTONE: usize = 1;
pub const DECAY: usize = 2;
pub const BODY: usize = 3;
pub const WOOD: usize = 4;
pub const SENS: usize = 5;
pub const OUTPUT: usize = 6;
pub const NPARAMS: usize = 7;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.click.as_ptr(),
            self.ctone.as_ptr(),
            self.decay.as_ptr(),
            self.body.as_ptr(),
            self.wood.as_ptr(),
            self.sens.as_ptr(),
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

/// Note-onset detector: power envelope (x^2 through a 40 Hz low-pass) and its rise
/// over the last 30 ms. Fires once per attack.
#[derive(Clone)]
pub struct Onset {
    lp: Biquad,
    mins: [f64; 30],
    mpos: usize,
    blk: f64,
    count: usize,
    ms: usize,
    armed: bool,
    since: usize,
    pub db: f64,
}

impl Onset {
    pub fn new(fs: f64) -> Self {
        let mut lp = Biquad::identity();
        lp.set_lowpass(fs, 40.0, 0.707);
        Self { lp, mins: [-120.0; 30], mpos: 0, blk: 120.0, count: 0, ms: (fs / 1000.0).round() as usize, armed: true, since: 1 << 30, db: -120.0 }
    }
    pub fn reset(&mut self) {
        self.lp.reset();
        self.mins = [-120.0; 30];
        self.blk = 120.0;
        self.armed = true;
        self.since = 1 << 30;
        self.db = -120.0;
    }
    /// Returns true on the sample a new note is found. `gap` in samples.
    #[inline]
    pub fn process(&mut self, x: f64, sens: f64, floor: f64, gap: usize) -> bool {
        let p = self.lp.process(x * x);
        self.db = 10.0 * p.max(1e-18).log10();
        self.blk = self.blk.min(self.db);
        self.count += 1;
        if self.count >= self.ms {
            self.count = 0;
            self.mins[self.mpos] = self.blk;
            self.mpos = (self.mpos + 1) % 30;
            self.blk = 120.0;
        }
        let lo = self.mins.iter().cloned().fold(self.blk, f64::min);
        let rise = self.db - lo;
        self.since = self.since.saturating_add(1);
        if rise < 0.5 * sens {
            self.armed = true;
        }
        if self.armed && rise > sens && self.db > floor && self.since >= gap {
            self.armed = false;
            self.since = 0;
            return true;
        }
        false
    }
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    onset: Onset,
    eq: [[Biquad; 5]; 2],
    click_bp: [Biquad; 2],
    nz: Noise,
    t: f64,
    g: f64,
    peak: f64,
    cenv: f64,
    key: (i32, i32, i32),
    out: Smooth,
    pub notes: usize,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let body = self.v[BODY] as f64;
        let wood = self.v[WOOD] as f64;
        let mut b = [Biquad::identity(); 5];
        b[0].set_peak(fs, 95.0, 2.0, 6.0 * body);
        b[1].set_peak(fs, 185.0, 2.5, 3.0 * body);
        b[2].set_peak(fs, 650.0, 1.0, -3.0 * body);
        let fc = 6000.0 * 0.15f64.powf(wood);
        b[3].set_lowpass(fs, fc, 0.7);
        b[4].set_highpass(fs, 35.0, 0.7);
        self.eq = [b, b];
        let mut c = Biquad::identity();
        c.set_bandpass(fs, self.v[CTONE] as f64, 1.4);
        self.click_bp = [c, c];
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self {
            fs, v, onset: Onset::new(fs), eq: [[Biquad::identity(); 5]; 2], click_bp: [Biquad::identity(); 2], nz: Noise::new(5),
            t: 10.0, g: 1.0, peak: 0.0, cenv: 0.0, key: (-1, -1, -1), out: Smooth::new(fs, 20.0, 1.0), notes: 0,
        };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.onset.reset();
        for c in 0..2 {
            self.eq[c].iter_mut().for_each(|b| b.reset());
            self.click_bp[c].reset();
        }
        self.t = 10.0;
        self.g = 1.0;
        self.cenv = 0.0;
        self.notes = 0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = ((v[BODY] * 200.0) as i32, (v[WOOD] * 200.0) as i32, v[CTONE] as i32);
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().max(1) as f64;
        let click = db_to_gain(self.v[CLICK] as f64);
        let decay = self.v[DECAY] as f64 * 0.001;
        let sens = self.v[SENS] as f64;
        let gap = (0.06 * self.fs) as usize;
        // note length: -30 dB at DECAY
        let tau = decay / 3.45;
        let cdec = (-1.0 / (0.010 * self.fs)).exp();
        let gatk = coeff(self.fs, 2.0);
        let mut flash = 0.0f32;
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch;
            if self.onset.process(x, sens, -50.0, gap) {
                self.t = 0.0;
                self.cenv = 1.0;
                self.notes += 1;
                flash = 1.0;
            }
            self.t += 1.0 / self.fs;
            // gut strings die quickly: shorten every note from its attack
            let target = (-(self.t - 0.02).max(0.0) / tau).exp();
            self.g = target + gatk * (self.g - target);
            // the slap: a short burst of fingerboard click, as loud as the note's attack
            self.peak = self.peak.max(x.abs()) * 0.9995;
            let burst = self.nz.next() * self.cenv * self.peak.max(0.02) * click * 2.0;
            self.cenv *= cdec;
            let o = self.out.next();
            for (ci, ch) in chans.iter_mut().enumerate() {
                let c = ci.min(1);
                let y = cascade(&mut self.eq[c], ch[k] as f64) * self.g;
                let cl = self.click_bp[c].process(burst);
                ch[k] = ((y + cl) * o) as f32;
            }
        }
        meters[0] = flash;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Rockabilly Slap", values: &[] },
        Preset { name: "Heavy Slap (Stray Cats)", values: &[(CLICK, 0.0), (CTONE, 3500.0), (DECAY, 380.0)] },
        Preset { name: "Walking Upright, No Slap", values: &[(CLICK, -40.0), (DECAY, 700.0), (BODY, 0.7)] },
        Preset { name: "Short Thumpy Notes", values: &[(DECAY, 220.0), (WOOD, 0.8), (CLICK, -10.0)] },
        Preset { name: "Bright Click, Long Notes", values: &[(CTONE, 5000.0), (DECAY, 900.0), (WOOD, 0.4)] },
        Preset { name: "Subtle Upright Flavour", values: &[(CLICK, -18.0), (DECAY, 1200.0), (BODY, 0.4), (WOOD, 0.4)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "UPRIGHT SLAP",
        subtitle: ("ELECTRIC BASS DI TO ROCKABILLY UPRIGHT", "WOOD · GUT STRINGS · SLAP CLICK"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![

            Item::Knob { p: CLICK, x: 110.0, y: 165.0, r: 40.0, label: "SLAP CLICK", tip: "The slap: the click of the strings hitting the fingerboard, added on every note found (as loud as the note's attack).", active: None },
            Item::Knob { p: CTONE, x: 250.0, y: 165.0, r: 28.0, label: "CLICK TONE", tip: "Pitch of the click: low is woody, high is a sharp clack.", active: None },
            Item::Knob { p: DECAY, x: 390.0, y: 165.0, r: 40.0, label: "NOTE LENGTH", tip: "Gut strings on an upright die away quickly: each note is shortened to fall 30 dB by this time.", active: None },
            Item::Lamp { m: 0, x: 560.0, y: 110.0, label: "NOTE", lo: 0.0, hi: 1.0 },
            Item::Divider { y: 250.0 },
            Item::Knob { p: BODY, x: 90.0, y: 320.0, r: 22.0, label: "BODY", tip: "The big wooden body: resonance around 95 and 185 Hz, less boxy midrange.", active: None },
            Item::Knob { p: WOOD, x: 210.0, y: 320.0, r: 22.0, label: "WOOD", tip: "How much of the electric bass's brightness is taken away.", active: None },
            Item::Knob { p: SENS, x: 330.0, y: 320.0, r: 22.0, label: "SENSITIVITY", tip: "How big a jump in level counts as a new note.", active: None },
            Item::Knob { p: OUTPUT, x: 520.0, y: 320.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Upright Slap", class_id: *b"BSzUprightSlapPQ", vendor: "PEQUOD Brian Setzer",
    categories: [Fx, Filter]
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
    fn tone(f: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n).map(|k| {
            let t = k as f64 / 48000.0;
            (1..=6).map(|h| amp / h as f64 * (2.0 * PI * f * h as f64 * t).sin()).sum::<f64>() as f32
        }).collect()
    }
    fn goertzel(x: &[f32], f: f64) -> f64 {
        let w = 2.0 * PI * f / 48000.0;
        let (mut s1, mut s2) = (0.0, 0.0);
        for v in x {
            let s = *v as f64 + 2.0 * w.cos() * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        ((s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2).max(0.0)).sqrt() / x.len() as f64
    }
    fn go(p: &[f32], x: &[f32]) -> Vec<f32> {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        input.remove(0)
    }
    fn noise(n: usize, amp: f64) -> Vec<f32> {
        let mut nz = vintage::dsp::Noise::new(11);
        (0..n).map(|_| (nz.next() * amp) as f32).collect()
    }

    fn plucks(n_notes: usize) -> Vec<f32> {
        let step = 19200;
        let mut x = vec![0.0f32; step * n_notes];
        for i in 0..n_notes {
            for k in 0..step - 600 {
                let t = k as f64 / 48000.0;
                let env = (1.0 - (-t / 0.003).exp()) * (-t / 1.5).exp();
                x[i * step + k] = (0.3 * env * (2.0 * PI * 55.0 * t).sin()) as f32;
            }
        }
        x
    }
    #[test]
    fn finds_notes_and_shortens_them() {
        let x = plucks(5);
        let mut d = Dsp::new(48000.0, 2);
        d.set_params(&defaults());
        d.reset();
        let mut l = x.clone();
        let mut r = x.clone();
        let mut m = vec![0.0; 1];
        for (a, b) in l.chunks_mut(256).zip(r.chunks_mut(256)) {
            let mut ch: Vec<&mut [f32]> = vec![a, b];
            d.process(&mut ch, &mut m);
        }
        let early = db(rms(&l[19200 + 2400..19200 + 4800]));
        let late = db(rms(&l[19200 + 16000..19200 + 18000]));
        let early_in = db(rms(&x[19200 + 2400..19200 + 4800]));
        let late_in = db(rms(&x[19200 + 16000..19200 + 18000]));
        println!("notes {}; decay over the note: in {:.1} dB, out {:.1} dB", d.notes, early_in - late_in, early - late);
        assert_eq!(d.notes, 5);
        assert!((early - late) - (early_in - late_in) > 10.0);
    }
    #[test]
    fn click_adds_top_on_the_attack() {
        let x = plucks(2);
        let a = go(&with(defaults(), &[(CLICK, -40.0)]), &x);
        let b = go(&with(defaults(), &[(CLICK, 0.0)]), &x);
        let ga = goertzel(&a[19200..19200 + 1200], 3000.0);
        let gb = goertzel(&b[19200..19200 + 1200], 3000.0);
        println!("3 kHz on the attack: click off {:.1} dB, on {:.1} dB", db(ga), db(gb));
        assert!(db(gb) > db(ga) + 12.0);
    }

}
