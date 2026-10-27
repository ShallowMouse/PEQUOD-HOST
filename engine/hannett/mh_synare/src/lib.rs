//! Synare 3: analogue synth drum after the Star Instruments Synare 3 that Stephen Morris used on Unknown Pleasures. Oscillator with pitch sweep, noise, a swept filter and LFO; plus a separate HISS layer for the She's Lost Control spray-can snare. Play it from MIDI: every note strikes the pad (TRACK on: notes also change pitch).

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


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Wave { #[name = "Triangle"] Tri, #[name = "Square"] Square }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Track { #[name = "Off"] Off, #[name = "On"] On }
const VOICES: usize = 4;


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "tune"]
    pub tune: FloatParam,
    #[id = "sweep"]
    pub sweep: FloatParam,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "wave"]
    pub wave: EnumParam<Wave>,
    #[id = "noise"]
    pub noise: FloatParam,
    #[id = "cutoff"]
    pub cutoff: FloatParam,
    #[id = "fsweep"]
    pub fsweep: FloatParam,
    #[id = "reso"]
    pub reso: FloatParam,
    #[id = "lforate"]
    pub lfo_rate: FloatParam,
    #[id = "lfodepth"]
    pub lfo_depth: FloatParam,
    #[id = "hiss"]
    pub hiss: FloatParam,
    #[id = "hissdecay"]
    pub hiss_decay: FloatParam,
    #[id = "track"]
    pub track: EnumParam<Track>,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            tune: hz("Tune", 180.0, 30.0, 1200.0),
            sweep: FloatParam::new("Sweep", -1.5, FloatRange::Linear { min: -4.0, max: 4.0 }).with_step_size(0.01).with_unit(" oct"),
            decay: ms("Decay", 350.0, 20.0, 3000.0),
            wave: EnumParam::new("Wave", Wave::Tri),
            noise: pct("Noise", 0.15),
            cutoff: hz("Cutoff", 2500.0, 80.0, 16000.0),
            fsweep: FloatParam::new("Filter Sweep", 2.0, FloatRange::Linear { min: -4.0, max: 4.0 }).with_step_size(0.01).with_unit(" oct"),
            reso: pct("Resonance", 0.3),
            lfo_rate: rate("LFO Rate", 6.0, 0.1, 30.0),
            lfo_depth: pct("LFO Depth", 0.0),
            hiss: pct("Hiss", 0.0),
            hiss_decay: ms("Hiss Decay", 250.0, 20.0, 1500.0),
            track: EnumParam::new("Key Track", Track::Off),
            volume: db("Volume", -6.0, -30.0, 6.0),
        }
    }
}

pub const TUNE: usize = 0;
pub const SWEEP: usize = 1;
pub const DECAY: usize = 2;
pub const WAVE: usize = 3;
pub const NOISE: usize = 4;
pub const CUTOFF: usize = 5;
pub const FSWEEP: usize = 6;
pub const RESO: usize = 7;
pub const LFORATE: usize = 8;
pub const LFODEPTH: usize = 9;
pub const HISS: usize = 10;
pub const HISSDECAY: usize = 11;
pub const TRACK: usize = 12;
pub const VOLUME: usize = 13;
pub const NPARAMS: usize = 14;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.tune.as_ptr(),
            self.sweep.as_ptr(),
            self.decay.as_ptr(),
            self.wave.as_ptr(),
            self.noise.as_ptr(),
            self.cutoff.as_ptr(),
            self.fsweep.as_ptr(),
            self.reso.as_ptr(),
            self.lfo_rate.as_ptr(),
            self.lfo_depth.as_ptr(),
            self.hiss.as_ptr(),
            self.hiss_decay.as_ptr(),
            self.track.as_ptr(),
            self.volume.as_ptr(),
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

#[derive(Clone, Copy, Default)]
struct Voice {
    on: bool,
    age: f64,
    vel: f64,
    note: f64,
    phase: f64,
    amp: f64,
    hiss: f64,
    // state-variable filter
    lp: f64,
    bp: f64,
    hlp: f64,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    voices: [Voice; VOICES],
    next: usize,
    noise: Noise,
    lfo: Lfo,
    hp: [Biquad; 2],
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut hp = [Biquad::identity(); 2];
        for h in &mut hp {
            h.set_highpass(fs, 25.0, 0.7);
        }
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        Self { fs, v, voices: [Voice::default(); VOICES], next: 0, noise: Noise::new(77), lfo: Lfo::default(), hp }
    }
    fn reset(&mut self) {
        self.voices = [Voice::default(); VOICES];
        self.hp.iter_mut().for_each(|h| h.reset());
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn midi(&mut self, ev: Midi) {
        match ev {
            Midi::On { note, vel } => {
                let i = self.next;
                self.next = (self.next + 1) % VOICES;
                self.voices[i] = Voice { on: true, age: 0.0, vel: vel as f64, note: note as f64, phase: 0.0, amp: 1.0, hiss: 1.0, ..Default::default() };
            }
            Midi::AllOff => self.voices.iter_mut().for_each(|v| v.on = false),
            _ => {}
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let v = |i: usize| self.v[i] as f64;
        let (tune, sweep, decay, square, noise_amt) = (v(TUNE), v(SWEEP), v(DECAY), v(WAVE) > 0.5, v(NOISE));
        let (cut, fsweep, reso, lrate, ldepth) = (v(CUTOFF), v(FSWEEP), v(RESO), v(LFORATE), v(LFODEPTH));
        let (hiss_amt, hdec, track, vol) = (v(HISS), v(HISSDECAY), v(TRACK) > 0.5, db_to_gain(v(VOLUME)));
        let dk = (-6.9 / (decay * 0.001 * self.fs)).exp();
        let hk = (-6.9 / (hdec * 0.001 * self.fs)).exp();
        // sweep envelope runs over roughly the decay time
        let sweep_t = decay * 0.001 * 0.5;
        let q = 1.0 / (0.5 + 4.5 * (1.0 - reso));
        let mut peak = 0.0f64;
        for k in 0..n {
            let l = self.lfo.next(self.fs, lrate) * ldepth;
            let mut out = 0.0;
            for vc in self.voices.iter_mut() {
                if !vc.on {
                    continue;
                }
                let env_s = (-vc.age / sweep_t.max(0.005)).exp();
                let key = if track { 2f64.powf((vc.note - 60.0) / 12.0) } else { 1.0 };
                let f = tune * key * 2f64.powf(sweep * env_s + l * 0.5);
                vc.phase = (vc.phase + f / self.fs).fract();
                let osc = if square { if vc.phase < 0.5 { 1.0 } else { -1.0 } } else { 4.0 * (vc.phase - 0.5).abs() - 1.0 };
                let nz = self.noise.next();
                let src = osc * (1.0 - noise_amt) + nz * noise_amt;
                // SVF (Chamberlin), cutoff swept by the envelope
                let fc = (cut * 2f64.powf(fsweep * env_s)).clamp(30.0, self.fs * 0.2);
                let g = 2.0 * (std::f64::consts::PI * fc / self.fs).sin();
                let hp = src - vc.lp - q * vc.bp;
                vc.bp += g * hp;
                vc.lp += g * vc.bp;
                vc.amp *= dk;
                // hiss layer: bright noise, its own decay
                vc.hiss *= hk;
                let h = nz - vc.hlp;
                vc.hlp += 0.3 * h;
                let y = vc.lp * vc.amp * vc.vel + h * vc.hiss * hiss_amt * vc.vel * 0.7;
                out += y;
                vc.age += 1.0 / self.fs;
                if vc.amp < 1e-4 && vc.hiss * hiss_amt < 1e-4 {
                    vc.on = false;
                }
            }
            let y = (out * vol * 0.8).tanh();
            peak = peak.max(y.abs());
            for (ci, c) in chans.iter_mut().enumerate() {
                c[k] = self.hp[ci.min(1)].process(y) as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "She's Lost Control (with hiss)", values: &[(TUNE, 220.0), (SWEEP, -1.2), (DECAY, 260.0), (NOISE, 0.2), (CUTOFF, 3000.0), (FSWEEP, 1.5), (HISS, 0.8), (HISSDECAY, 380.0)] },
        Preset { name: "Disco Tom Drop", values: &[(TUNE, 300.0), (SWEEP, -2.5), (DECAY, 600.0), (NOISE, 0.05), (CUTOFF, 4000.0), (FSWEEP, 0.0)] },
        Preset { name: "Rising Pew", values: &[(TUNE, 150.0), (SWEEP, 2.0), (DECAY, 300.0), (NOISE, 0.0)] },
        Preset { name: "Hollow Snare", values: &[(TUNE, 260.0), (SWEEP, -0.5), (DECAY, 180.0), (NOISE, 0.6), (CUTOFF, 5000.0), (RESO, 0.5)] },
        Preset { name: "Deep Kick", values: &[(TUNE, 90.0), (SWEEP, -1.0), (DECAY, 400.0), (NOISE, 0.0), (CUTOFF, 800.0), (FSWEEP, 1.0)] },
        Preset { name: "LFO Warble", values: &[(TUNE, 200.0), (SWEEP, -0.6), (DECAY, 900.0), (LFORATE, 9.0), (LFODEPTH, 0.6)] },
        Preset { name: "Spray Can Only", values: &[(NOISE, 0.0), (VOLUME, -30.0), (HISS, 1.0), (HISSDECAY, 500.0)] },
        Preset { name: "Melodic (key track)", values: &[(TRACK, 1.0), (TUNE, 260.0), (SWEEP, -0.3), (DECAY, 700.0), (NOISE, 0.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "SYNARE 3",
        subtitle: ("AFTER THE STAR INSTRUMENTS SYNARE 3", "ANALOGUE SYNTH DRUM · MIDI"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![

            Item::Knob { p: TUNE, x: 70.0, y: 140.0, r: 30.0, label: "TUNE", tip: "Oscillator pitch.", active: None },
            Item::Knob { p: SWEEP, x: 160.0, y: 140.0, r: 30.0, label: "SWEEP", tip: "Pitch sweep after the hit, in octaves (down or up): the classic synth-drum 'pew'.", active: None },
            Item::Knob { p: DECAY, x: 250.0, y: 140.0, r: 30.0, label: "DECAY", tip: "Amplitude decay.", active: None },
            Item::Switch { p: WAVE, x: 340.0, y: 140.0, labels: ["TRI", "SQR"], tip: "Oscillator wave." },
            Item::Knob { p: NOISE, x: 430.0, y: 140.0, r: 30.0, label: "NOISE", tip: "Oscillator vs white noise.", active: None },
            Item::Knob { p: VOLUME, x: 560.0, y: 140.0, r: 30.0, label: "VOLUME", tip: "Output level. Velocity sets the hit strength.", active: None },
            Item::Divider { y: 210.0 },
            Item::Knob { p: CUTOFF, x: 70.0, y: 260.0, r: 22.0, label: "CUTOFF", tip: "Filter cutoff.", active: None },
            Item::Knob { p: FSWEEP, x: 150.0, y: 260.0, r: 22.0, label: "F. SWEEP", tip: "Filter sweep after the hit, in octaves.", active: None },
            Item::Knob { p: RESO, x: 230.0, y: 260.0, r: 22.0, label: "RESONANCE", tip: "Filter resonance.", active: None },
            Item::Knob { p: LFORATE, x: 310.0, y: 260.0, r: 22.0, label: "LFO RATE", tip: "Modulation speed.", active: None },
            Item::Knob { p: LFODEPTH, x: 390.0, y: 260.0, r: 22.0, label: "LFO DEPTH", tip: "Pitch modulation depth.", active: None },
            Item::Knob { p: HISS, x: 470.0, y: 260.0, r: 22.0, label: "HISS", tip: "A separate bright noise burst on each hit: the aerosol-spray snare layer on She's Lost Control.", active: None },
            Item::Knob { p: HISSDECAY, x: 550.0, y: 260.0, r: 22.0, label: "HISS DECAY", tip: "How long the hiss lasts.", active: None },
            Item::Switch { p: TRACK, x: 320.0, y: 336.0, labels: ["PAD", "KEYS"], tip: "PAD: every note hits the same drum. KEYS: notes change the pitch (C4 = TUNE)." },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Synare 3", class_id: *b"MHSynare3DrumPQ1", vendor: "PEQUOD Hannett",
    instrument, categories: [Instrument, Drum]
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
    fn notes_strike_and_decay() {
        let mut d = Dsp::new(48000.0, 2);
        d.set_params(&defaults());
        let mut m = [0.0f32];
        let mut l = vec![0.0f32; 48000];
        let mut r = l.clone();
        d.midi(Midi::On { note: 60, vel: 1.0 });
        {
            let mut ch: Vec<&mut [f32]> = vec![&mut l, &mut r];
            d.process(&mut ch, &mut m);
        }
        let early = rms(&l[..4800]);
        let late = rms(&l[40000..]);
        println!("early {:.1} dB late {:.1} dB", db(early), db(late));
        assert!(early > 0.02 && late < early * 0.01);
    }

}
