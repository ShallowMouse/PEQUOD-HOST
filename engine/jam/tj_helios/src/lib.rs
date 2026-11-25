//! Helios Channel: a channel of the 72-input Helios desk in Studio One at the Townhouse, where Vic
//! Coppersmith-Heaven recorded and mixed Setting Sons, Sound Affects and The Gift with The Jam. A Helios mic
//! amplifier (transformer in, transistor gain) and the Helios EQ: a bass section that boosts at 50, 100 or
//! 200 Hz with its own separate bass cut, a presence section stepped from 700 Hz to 5 kHz, and a 10 kHz
//! treble shelf. Curves estimated from published measurements of Helios modules.

#![allow(clippy::excessive_precision)]
use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
#[allow(unused_imports)]
use vintage::dsp::*;
#[allow(unused_imports)]
use vintage::fx::*;
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

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum BassF { #[name = "50 Hz"] F50, #[name = "100 Hz"] F100, #[name = "200 Hz"] F200 }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Cut { #[name = "Off"] Off, #[name = "50 Hz"] C50, #[name = "100 Hz"] C100, #[name = "200 Hz"] C200 }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum MidF { #[name = "700 Hz"] F700, #[name = "1.5 kHz"] F1500, #[name = "3 kHz"] F3000, #[name = "5 kHz"] F5000 }

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "bassf"]
    pub bassf: EnumParam<BassF>,
    #[id = "bass"]
    pub bass: FloatParam,
    #[id = "cut"]
    pub cut: EnumParam<Cut>,
    #[id = "midf"]
    pub midf: EnumParam<MidF>,
    #[id = "mid"]
    pub mid: FloatParam,
    #[id = "treble"]
    pub treble: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            drive: pct("Mic Amp", 0.3),
            bassf: EnumParam::new("Bass Frequency", BassF::F100),
            bass: FloatParam::new("Bass", 0.0, FloatRange::Linear { min: -16.0, max: 16.0 }).with_step_size(0.5).with_unit(" dB"),
            cut: EnumParam::new("Bass Cut", Cut::Off),
            midf: EnumParam::new("Presence Frequency", MidF::F3000),
            mid: FloatParam::new("Presence", 0.0, FloatRange::Linear { min: -10.0, max: 10.0 }).with_step_size(0.5).with_unit(" dB"),
            treble: FloatParam::new("Treble", 0.0, FloatRange::Linear { min: -12.0, max: 12.0 }).with_step_size(0.5).with_unit(" dB"),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const DRIVE: usize = 0;
pub const BASSF: usize = 1;
pub const BASS: usize = 2;
pub const CUT: usize = 3;
pub const MIDF: usize = 4;
pub const MID: usize = 5;
pub const TREBLE: usize = 6;
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
            self.bassf.as_ptr(),
            self.bass.as_ptr(),
            self.cut.as_ptr(),
            self.midf.as_ptr(),
            self.mid.as_ptr(),
            self.treble.as_ptr(),
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
    eq: [[Biquad; 6]; 2],
    xfmr: [[Biquad; 2]; 2],
    dc: [DcBlock; 2],
    key: [i32; 7],
    drive: Smooth,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let v = self.v;
        let bf = [50.0, 100.0, 200.0][(v[BASSF].round() as usize).min(2)];
        let bass = v[BASS] as f64;
        let mf = [700.0, 1500.0, 3000.0, 5000.0][(v[MIDF].round() as usize).min(3)];
        let mid = v[MID] as f64;
        let mut b = [Biquad::identity(); 6];
        // the bass: a broad lift that peaks at the switched frequency and stays up below it
        b[0].set_low_shelf(fs, bf * 1.3, 0.8, bass * 0.6);
        b[1].set_peak(fs, bf, 0.9, bass * 0.5);
        // the separate bass cut: a gentle high-pass
        match (v[CUT].round() as usize).min(3) {
            0 => {}
            c => b[2].set_highpass(fs, [0.0, 50.0, 100.0, 200.0][c], 0.6),
        }
        // presence: wide boosts, narrower cuts
        b[3].set_peak(fs, mf, if mid >= 0.0 { 0.9 } else { 1.4 }, mid);
        // treble: a 10 kHz shelf
        b[4].set_high_shelf(fs, 10000.0, 0.6, v[TREBLE] as f64);
        b[5].set_lowpass(fs, 24000.0f64.min(fs * 0.45), 0.7);
        self.eq = [b, b];
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut x = [Biquad::identity(); 2];
        // the input transformer: a little weight low down, and its own low end corner
        x[0].set_peak(fs, 45.0, 0.7, 0.8);
        x[1].set_highpass(fs, 14.0, 0.7);
        let mut d = Self {
            fs, v, eq: [[Biquad::identity(); 6]; 2], xfmr: [x, x], dc: [DcBlock::new(fs), DcBlock::new(fs)],
            key: [-1; 7], drive: Smooth::new(fs, 20.0, 0.3), out: Smooth::new(fs, 20.0, 1.0),
        };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.eq.iter_mut().flatten().chain(self.xfmr.iter_mut().flatten()).for_each(|b| b.reset());
        self.dc.iter_mut().for_each(|d| d.reset());
        self.drive.snap(self.v[DRIVE] as f64);
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = [v[BASSF].round() as i32, (v[BASS] * 2.0) as i32, v[CUT].round() as i32, v[MIDF].round() as i32, (v[MID] * 2.0) as i32, (v[TREBLE] * 2.0) as i32, 0];
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.drive.set(v[DRIVE] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        for k in 0..n {
            let dr = self.drive.next();
            let o = self.out.next();
            // the mic amp: clean at low gain; pushed, the transistors and transformer thicken it
            let g = 1.0 + dr * dr * 10.0;
            for (ci, ch) in chans.iter_mut().enumerate() {
                let c = ci.min(1);
                let x = cascade(&mut self.xfmr[c], ch[k] as f64);
                let y = tube(x * g, 0.12) / g;
                let y = self.dc[c].process(cascade(&mut self.eq[c], y));
                ch[k] = (y * o) as f32;
            }
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Flat Through the Desk", values: &[] },
        Preset { name: "Rickenbacker Guitar Bite", values: &[(CUT, 2.0), (MIDF, 2.0), (MID, 4.0), (TREBLE, 3.0)] },
        Preset { name: "Bass DI Weight", values: &[(BASSF, 0.0), (BASS, 6.0), (MIDF, 0.0), (MID, 2.5), (DRIVE, 0.5)] },
        Preset { name: "Kick at 100", values: &[(BASSF, 1.0), (BASS, 8.0), (MIDF, 2.0), (MID, 3.0), (TREBLE, -2.0)] },
        Preset { name: "Snare Crack", values: &[(BASSF, 2.0), (BASS, 3.0), (MIDF, 3.0), (MID, 5.0), (TREBLE, 2.0)] },
        Preset { name: "Vocal Presence (U67)", values: &[(CUT, 1.0), (MIDF, 3.0), (MID, 3.0), (TREBLE, 2.5)] },
        Preset { name: "Overheads Air", values: &[(CUT, 3.0), (TREBLE, 6.0)] },
        Preset { name: "Hot Mic Amp", values: &[(DRIVE, 0.85), (MIDF, 1.0), (MID, 2.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "HELIOS CHANNEL",
        subtitle: ("TOWNHOUSE STUDIO ONE'S HELIOS DESK", "MIC AMP · BASS · BASS CUT · PRESENCE · TREBLE"),
        accent: ui::STEEL,
        size: (W, H),
        items: vec![
            Item::Knob { p: DRIVE, x: 70.0, y: 170.0, r: 26.0, label: "MIC AMP", tip: "How hard the Helios mic amplifier is driven: clean low down, thicker as it is pushed.", active: None },
            Item::Selector { p: BASSF, x: 180.0, y: 130.0, label: "BASS FREQ", positions: &["50", "100", "200"], tip: "Where the bass lift is centred." },
            Item::Knob { p: BASS, x: 180.0, y: 224.0, r: 22.0, label: "BASS", tip: "Bass boost or cut at the chosen frequency.", active: None },
            Item::Selector { p: CUT, x: 290.0, y: 170.0, label: "BASS CUT", positions: &["OFF", "50", "100", "200"], tip: "The separate bass cut: a gentle high-pass." },
            Item::Selector { p: MIDF, x: 400.0, y: 130.0, label: "PRESENCE", positions: &["700", "1.5K", "3K", "5K"], tip: "The presence frequency." },
            Item::Knob { p: MID, x: 400.0, y: 210.0, r: 22.0, label: "PRESENCE", tip: "Presence boost (wide) or cut (narrower).", active: None },
            Item::Knob { p: TREBLE, x: 510.0, y: 170.0, r: 26.0, label: "TREBLE", tip: "The 10 kHz treble shelf.", active: None },
            Item::Divider { y: 262.0 },
            Item::Knob { p: OUTPUT, x: 320.0, y: 330.0, r: 26.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Helios Channel", class_id: *b"TJamHeliosChanQ1", vendor: "PEQUOD The Jam",
    categories: [Fx, Eq]
);

#[cfg(test)]
#[allow(dead_code)]
pub mod tests {
    use super::*;
    use std::f64::consts::PI;

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
    pub fn sine(f: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n).map(|k| (amp * (2.0 * PI * f * k as f64 / 48000.0).sin()) as f32).collect()
    }
    pub fn noise(n: usize, amp: f64) -> Vec<f32> {
        let mut nz = vintage::dsp::Noise::new(11);
        (0..n).map(|_| (nz.next() * amp) as f32).collect()
    }
    /// Both channels out for a mono input.
    pub fn go2(p: &[f32], x: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        let r = input.pop().unwrap();
        (input.pop().unwrap(), r)
    }
    pub fn go(p: &[f32], x: &[f32]) -> Vec<f32> {
        go2(p, x).0
    }
    /// Steady-state level of a sine at f through the settings, in dB.
    pub fn level(set: &[(usize, f32)], f: f64) -> f64 {
        let p = with(defaults(), set);
        let y = go(&p, &sine(f, 0.1, 48000));
        db(rms(&y[24000..]))
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

    #[test]
    fn flat_is_flat() {
        for f in [100.0, 1000.0, 5000.0] {
            let l = level(&[(DRIVE, 0.0)], f);
            println!("flat {f} Hz: {l:.2} dB");
            assert!((l - db(0.1 / 2f64.sqrt())).abs() < 1.0);
        }
    }
    #[test]
    fn bass_follows_its_frequency() {
        let at = |bf: f32, f: f64| level(&[(BASSF, bf), (BASS, 12.0), (DRIVE, 0.0)], f) - level(&[(DRIVE, 0.0)], f);
        println!("bass +12: at 50 Hz setting 50 {:.1}; at 200 Hz: setting 50 {:.1}, setting 200 {:.1}", at(0.0, 50.0), at(0.0, 200.0), at(2.0, 200.0));
        assert!(at(0.0, 50.0) > 9.0 && at(2.0, 200.0) - at(0.0, 200.0) > 5.0);
    }
    #[test]
    fn presence_and_treble() {
        let m = level(&[(MIDF, 2.0), (MID, 10.0), (DRIVE, 0.0)], 3000.0) - level(&[(DRIVE, 0.0)], 3000.0);
        let t = level(&[(TREBLE, 12.0), (DRIVE, 0.0)], 15000.0) - level(&[(DRIVE, 0.0)], 15000.0);
        let c = level(&[(CUT, 3.0), (DRIVE, 0.0)], 60.0) - level(&[(DRIVE, 0.0)], 60.0);
        println!("presence 3k {m:.1}, treble 15k {t:.1}, cut 200 at 60 Hz {c:.1}");
        assert!(m > 8.0 && t > 9.0 && c < -10.0);
    }

}
