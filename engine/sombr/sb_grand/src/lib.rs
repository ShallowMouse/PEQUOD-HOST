//! Electric Grand: a strung electric piano in the manner of the Yamaha CP80 used on sombr's 12 to 12: real strings struck by hammers, read by pickups, so it's bright, a little metallic and beats between its paired strings. Plus the tremolo and chorus it was usually played with.

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
use vintage::fx::Bbd;

pub const W: u32 = 640;
pub const H: u32 = 432;


const VOICES: usize = 10;


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "hammer"]
    pub hammer: FloatParam,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "detune"]
    pub detune: FloatParam,
    #[id = "tremrate"]
    pub trem_rate: FloatParam,
    #[id = "trem"]
    pub trem: FloatParam,
    #[id = "chorus"]
    pub chorus: FloatParam,
    #[id = "volume"]
    pub volume: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            tone: pct("Brightness", 0.6),
            hammer: pct("Hammer", 0.5),
            decay: pct("Sustain", 0.6),
            detune: FloatParam::new("String Detune", 3.0, FloatRange::Linear { min: 0.0, max: 15.0 }).with_step_size(0.1).with_unit(" ct"),
            trem_rate: rate("Tremolo Rate", 4.5, 0.5, 10.0),
            trem: pct("Tremolo", 0.0),
            chorus: pct("Chorus", 0.3),
            volume: db("Volume", -6.0, -30.0, 6.0),
        }
    }
}

pub const TONE: usize = 0;
pub const HAMMER: usize = 1;
pub const DECAY: usize = 2;
pub const DETUNE: usize = 3;
pub const TREMRATE: usize = 4;
pub const TREM: usize = 5;
pub const CHORUS: usize = 6;
pub const VOLUME: usize = 7;
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
            self.tone.as_ptr(),
            self.hammer.as_ptr(),
            self.decay.as_ptr(),
            self.detune.as_ptr(),
            self.trem_rate.as_ptr(),
            self.trem.as_ptr(),
            self.chorus.as_ptr(),
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

#[derive(Clone)]
struct Voice {
    on: bool,
    held: bool,
    note: u8,
    s: [DelayLine; 2],
    len: [f64; 2],
    lp: [f64; 2],
    loss: f64,
    damp: f64,
    age: usize,
    amp: f64,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    voices: Vec<Voice>,
    noise: Noise,
    pickup: [Biquad; 2],
    body: Biquad,
    lfo: Lfo,
    clfo: Lfo,
    bbd: [Bbd; 2],
    sustain_pedal: bool,
}

impl Dsp {
    fn strike(&mut self, note: u8, vel: f64) {
        let i = self.voices.iter().position(|v| !v.on).unwrap_or_else(|| {
            // steal the oldest
            self.voices.iter().enumerate().max_by_key(|(_, v)| v.age).map(|(i, _)| i).unwrap_or(0)
        });
        let fs = self.fs;
        let f = 440.0 * 2f64.powf((note as f64 - 69.0) / 12.0);
        let det = self.v[DETUNE] as f64 / 1200.0;
        let hammer = self.v[HAMMER] as f64;
        let tone = self.v[TONE] as f64;
        let sus = self.v[DECAY] as f64;
        // decay time: long in the bass, short at the top (seconds to -60 dB)
        let t60 = (12.0 * (110.0 / f).powf(0.6)).clamp(0.8, 20.0) * (0.4 + 1.2 * sus);
        let vc = &mut self.voices[i];
        vc.on = true;
        vc.held = true;
        vc.note = note;
        vc.age = 0;
        vc.amp = vel;
        for s in 0..2 {
            let fr = f * 2f64.powf(if s == 0 { det } else { -det });
            // loop = delay + the averaging filter's half-sample-ish group delay
            vc.len[s] = (fs / fr - 1.0 - (0.08 + 0.42 * (1.0 - tone))).min(vc.s[s].max_delay() - 2.0);
            vc.s[s].reset();
            vc.lp[s] = 0.0;
        }
        vc.loss = 10f64.powf(-3.0 / (t60 * f));
        vc.damp = 0.08 + 0.42 * (1.0 - tone);
        // hammer: a short, velocity-shaped burst written along the string
        let n = vc.len[0] as usize + 2;
        let soft = (1.0 - hammer * vel).clamp(0.05, 0.95);
        for s in 0..2 {
            let mut lpv = 0.0;
            for k in 0..n {
                let pos = k as f64 / n as f64;
                let shape = (std::f64::consts::PI * pos).sin().powf(1.0 + 3.0 * soft);
                let nz = self.noise.next();
                lpv += (1.0 - soft * 0.9) * (nz - lpv);
                vc.s[s].write(shape * (0.6 * lpv + 0.4) * vel);
            }
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mk = || Voice { on: false, held: false, note: 0, s: [DelayLine::new((fs / 25.0) as usize + 8), DelayLine::new((fs / 25.0) as usize + 8)], len: [100.0; 2], lp: [0.0; 2], loss: 0.99, damp: 0.3, age: 0, amp: 0.0 };
        let mut pickup = [Biquad::identity(); 2];
        pickup[0].set_highpass(fs, 120.0, 0.7);
        pickup[1].set_peak(fs, 2800.0, 0.9, 4.0);
        let mut body = Biquad::identity();
        body.set_peak(fs, 400.0, 1.0, -3.0);
        let mut bbd = [Bbd::new(fs, 20.0), Bbd::new(fs, 20.0)];
        for b in &mut bbd {
            b.set_bandwidth(10000.0);
        }
        Self { fs, v, voices: (0..VOICES).map(|_| mk()).collect(), noise: Noise::new(5), pickup, body, lfo: Lfo::default(), clfo: Lfo::default(), bbd, sustain_pedal: false }
    }
    fn reset(&mut self) {
        for vc in &mut self.voices {
            vc.on = false;
            vc.s.iter_mut().for_each(|s| s.reset());
        }
        self.pickup.iter_mut().for_each(|b| b.reset());
        self.body.reset();
        self.bbd.iter_mut().for_each(|b| b.reset());
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
    }
    fn midi(&mut self, ev: Midi) {
        match ev {
            Midi::On { note, vel } => self.strike(note, vel as f64),
            Midi::Off { note } => {
                for vc in self.voices.iter_mut().filter(|v| v.on && v.note == note) {
                    vc.held = false;
                }
            }
            Midi::Cc { cc: 64, value } => self.sustain_pedal = value >= 0.5,
            Midi::AllOff => self.voices.iter_mut().for_each(|v| v.on = false),
            _ => {}
        }
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let vol = db_to_gain(self.v[VOLUME] as f64);
        let (tr_rate, tr) = (self.v[TREMRATE] as f64, self.v[TREM] as f64);
        let chorus = self.v[CHORUS] as f64;
        let pedal = self.sustain_pedal;
        let mut peak = 0.0f64;
        for k in 0..n {
            let mut sum = 0.0;
            for vc in self.voices.iter_mut() {
                if !vc.on {
                    continue;
                }
                // dampers down when the key (and pedal) is released
                let damper = if vc.held || pedal { 1.0 } else { 0.992 };
                let mut out = 0.0;
                for s in 0..2 {
                    let y = vc.s[s].read(vc.len[s]);
                    // loop filter: one-pole low-pass (brightness) and loss
                    // two-point averaging loop filter (Karplus-Strong): highs die faster than the fundamental
                    let f = (1.0 - vc.damp) * y + vc.damp * vc.lp[s];
                    vc.lp[s] = y;
                    let fb = f * vc.loss * damper;
                    vc.s[s].write(fb);
                    out += y;
                }
                sum += out * 0.5;
                vc.age += 1;
                if vc.age % 256 == 0 {
                    // retire silent voices
                    let e = vc.lp[0].abs() + vc.lp[1].abs();
                    if e < 1e-5 && vc.age > 4800 {
                        vc.on = false;
                    }
                }
            }
            // pickups: bright and thin, a little body notch
            let y = self.body.process(cascade(&mut self.pickup, sum));
            let t = 1.0 - tr * 0.5 * (1.0 + self.lfo.next(fs, tr_rate));
            let y = y * t * vol;
            let cl = self.clfo.next(fs, 0.7);
            let wl = self.bbd[0].process(y, (6.0 + 2.5 * cl) * 0.001 * fs);
            let wr = self.bbd[1].process(y, (6.0 - 2.5 * cl) * 0.001 * fs);
            let l = y * (1.0 - 0.5 * chorus) + wl * chorus * 0.7;
            let r = y * (1.0 - 0.5 * chorus) + wr * chorus * 0.7;
            peak = peak.max(l.abs()).max(r.abs());
            if chans.len() >= 2 {
                chans[0][k] = l as f32;
                chans[1][k] = r as f32;
            } else if let Some(c) = chans.get_mut(0) {
                c[k] = (0.5 * (l + r)) as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "12 to 12 Grand", values: &[(TONE, 0.65), (HAMMER, 0.55), (CHORUS, 0.25)] },
        Preset { name: "Dry Bright Grand", values: &[(TONE, 0.8), (CHORUS, 0.0), (TREM, 0.0)] },
        Preset { name: "80s Chorus Ballad", values: &[(TONE, 0.6), (CHORUS, 0.7), (DETUNE, 5.0)] },
        Preset { name: "Tremolo Keys", values: &[(TREM, 0.5), (TREMRATE, 5.0), (CHORUS, 0.2)] },
        Preset { name: "Soft and Dark", values: &[(TONE, 0.3), (HAMMER, 0.2), (CHORUS, 0.3)] },
        Preset { name: "Honky Detuned", values: &[(DETUNE, 12.0), (TONE, 0.7), (CHORUS, 0.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "ELECTRIC GRAND",
        subtitle: ("IN THE MANNER OF THE YAMAHA CP80", "STRUCK STRINGS · PICKUPS · MIDI"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![

            Item::Knob { p: TONE, x: 90.0, y: 160.0, r: 36.0, label: "BRIGHTNESS", tip: "How long the top end rings on the strings.", active: None },
            Item::Knob { p: HAMMER, x: 210.0, y: 160.0, r: 36.0, label: "HAMMER", tip: "Hammer hardness: harder = more attack and clank. Velocity also hits harder.", active: None },
            Item::Knob { p: DECAY, x: 330.0, y: 160.0, r: 36.0, label: "SUSTAIN", tip: "How long the strings ring. Sustain pedal (CC64) works.", active: None },
            Item::Knob { p: DETUNE, x: 450.0, y: 160.0, r: 36.0, label: "DETUNE", tip: "Each note has two strings; detuning them makes them beat.", active: None },
            Item::Knob { p: VOLUME, x: 565.0, y: 160.0, r: 30.0, label: "VOLUME", tip: "Output level.", active: None },
            Item::Divider { y: 264.0 },
            Item::Knob { p: TREMRATE, x: 120.0, y: 330.0, r: 22.0, label: "TREM RATE", tip: "Tremolo speed.", active: None },
            Item::Knob { p: TREM, x: 240.0, y: 330.0, r: 22.0, label: "TREMOLO", tip: "Tremolo depth.", active: None },
            Item::Knob { p: CHORUS, x: 400.0, y: 330.0, r: 22.0, label: "CHORUS", tip: "Stereo chorus, as these pianos were usually played through.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Electric Grand", class_id: *b"SBElectricGrandP", vendor: "PEQUOD sombr",
    instrument, categories: [Instrument, Piano]
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
    fn plays_in_tune_and_decays() {
        let mut d = Dsp::new(48000.0, 2);
        d.set_params(&with(defaults(), &[(CHORUS, 0.0), (TREM, 0.0), (DETUNE, 0.0), (TONE, 0.0), (HAMMER, 0.0)]));
        let mut m = [0.0f32];
        let mut l = vec![0.0f32; 96000];
        let mut r = l.clone();
        d.midi(Midi::On { note: 69, vel: 0.8 });
        {
            let mut ch: Vec<&mut [f32]> = vec![&mut l, &mut r];
            d.process(&mut ch, &mut m);
        }
        let z = l[24000..72000].windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        let f = z as f64;
        println!("A4 -> {f:.0} Hz; early {:.1} dB late {:.1} dB", db(rms(&l[0..4800])), db(rms(&l[90000..])));
        assert!((f - 440.0).abs() < 6.0);
        assert!(rms(&l[90000..]) < rms(&l[0..4800]));
    }

}
