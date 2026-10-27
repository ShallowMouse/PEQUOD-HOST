//! Bass Voice: shapes a modern bass DI toward Paul's Hofner 500/1 or Rickenbacker 4001S: voicing, pickup position, flatwound strings (brightness that fades after each note's attack), foam mute (notes die quickly into a thump), body thump and tone. v1.9: rebuilt; the foam mute now follows each note smoothly instead of re-triggering on the waveform (the old version could stutter).

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
pub enum Bass { #[name = "Hofner 500/1"] Hofner, #[name = "Rickenbacker 4001S"] Rick, #[name = "As played (no voicing)"] AsPlayed }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "bass"]
    pub bass: EnumParam<Bass>,
    #[id = "pickup"]
    pub pickup: FloatParam,
    #[id = "body"]
    pub body: FloatParam,
    #[id = "flats"]
    pub flats: FloatParam,
    #[id = "mute"]
    pub mute: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            bass: EnumParam::new("Bass", Bass::Hofner),
            pickup: pct("Pickup (neck to bridge)", 0.2),
            body: pct("Body", 0.5),
            flats: pct("Flatwounds", 0.6),
            mute: pct("Foam Mute", 0.0),
            tone: pct("Tone", 0.75),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const BASS: usize = 0;
pub const PICKUP: usize = 1;
pub const BODY: usize = 2;
pub const FLATS: usize = 3;
pub const MUTE: usize = 4;
pub const TONE: usize = 5;
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
            self.bass.as_ptr(),
            self.pickup.as_ptr(),
            self.body.as_ptr(),
            self.flats.as_ptr(),
            self.mute.as_ptr(),
            self.tone.as_ptr(),
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

/// Topology-preserving state-variable low-pass: safe to sweep every sample.
#[derive(Clone, Copy, Default)]
struct Svf {
    s1: f64,
    s2: f64,
}

impl Svf {
    #[inline]
    fn lp(&mut self, x: f64, fs: f64, fc: f64, q: f64) -> f64 {
        let g = (std::f64::consts::PI * fc.clamp(20.0, fs * 0.45) / fs).tan();
        let k = 1.0 / q;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let v3 = x - self.s2;
        let v1 = a1 * self.s1 + g * a1 * v3;
        let v2 = self.s2 + g * v1;
        self.s1 = 2.0 * v1 - self.s1;
        self.s2 = 2.0 * v2 - self.s2;
        v2
    }
}

/// Per-channel state.
#[derive(Clone, Copy, Default)]
struct Chan {
    voice: [Biquad; 3],
    pick: [Biquad; 2],
    body_bp: Biquad,
    tone: Biquad,
    svf: Svf,
    pow: f64,
    fast: f64,
    slow: f64,
    d_mute: f64,
    d_flat: f64,
    d_body: f64,
    g: f64,
}

pub struct Dsp {
    fs: f64,
    ch: [Chan; 2],
    v: [f32; NPARAMS],
    key: [f64; 3],
    tone_key: f64,
    out: Smooth,
}

impl Dsp {
    fn voice(&mut self, bass: usize, pickup: f64) {
        let fs = self.fs;
        for c in &mut self.ch {
            match bass {
                // Hofner: hollow body bump, soft top
                0 => {
                    c.voice[0].set_peak(fs, 120.0, 0.9, 4.0);
                    c.voice[1].set_peak(fs, 700.0, 1.0, 2.0);
                    c.voice[2].set_high_shelf(fs, 2500.0, 0.7, -6.0);
                    c.body_bp.set_bandpass(fs, 95.0, 2.5);
                }
                // Rickenbacker: tighter lows, mid presence, a little sparkle
                1 => {
                    c.voice[0].set_low_shelf(fs, 80.0, 0.7, -2.0);
                    c.voice[1].set_peak(fs, 1200.0, 1.0, 3.0);
                    c.voice[2].set_high_shelf(fs, 4000.0, 0.7, 2.0);
                    c.body_bp.set_bandpass(fs, 110.0, 2.0);
                }
                _ => {
                    c.voice = [Biquad::identity(); 3];
                    c.body_bp.set_bandpass(fs, 100.0, 2.0);
                }
            }
            // pickup: neck = round and deep, bridge = tight and nasal
            let p = pickup;
            c.pick[0].set_low_shelf(fs, 200.0, 0.7, 3.0 * (1.0 - p) - 5.0 * p);
            c.pick[1].set_peak(fs, 1500.0, 0.8, -3.0 * (1.0 - p) + 4.0 * p);
        }
        self.key = [bass as f64, pickup, 0.0];
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, ch: [Chan::default(); 2], v, key: [-1.0; 3], tone_key: -1.0, out: Smooth::new(fs, 20.0, 1.0) };
        for c in &mut d.ch {
            c.g = 1.0;
            c.tone = Biquad::identity();
        }
        d.voice(0, 0.2);
        d
    }
    fn reset(&mut self) {
        let (b, p) = (self.key[0] as usize, self.key[1]);
        for c in &mut self.ch {
            *c = Chan { g: 1.0, ..Chan::default() };
        }
        self.voice(b, p);
        self.tone_key = -1.0;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let b = (v[BASS].round() as usize).min(2);
        let p = v[PICKUP] as f64;
        if b as f64 != self.key[0] || (p - self.key[1]).abs() > 1e-3 {
            self.voice(b, p);
        }
        let t = v[TONE] as f64;
        if (t - self.tone_key).abs() > 1e-3 {
            for c in &mut self.ch {
                if t >= 0.995 {
                    c.tone = Biquad::identity();
                } else {
                    c.tone.set_lowpass(self.fs, 700.0 * 2f64.powf(4.5 * t), 0.7);
                }
            }
            self.tone_key = t;
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let fs = self.fs;
        let p = |i: usize| self.v[i] as f64;
        let (body, flats, mute) = (p(BODY), p(FLATS), p(MUTE));
        let body_scale = [1.0, 0.6, 0.4][(p(BASS).round() as usize).min(2)];
        // detectors: power smoothed over 15 ms (no ripple from low notes), then a 40 ms follower to compare against
        let c_pow = 1.0 - coeff(fs, 15.0);
        let c_slow = 1.0 - coeff(fs, 40.0);
        // decay envelopes: foam mute (1.5 s .. 60 ms), flatwound brightness (3 s .. 150 ms), body thump (50 ms)
        let k_mute = coeff(fs, 1500.0 * (60.0f64 / 1500.0).powf(mute));
        let k_flat = coeff(fs, 3000.0 * (150.0f64 / 3000.0).powf(flats));
        let k_body = coeff(fs, 50.0);
        let c_gain = 1.0 - coeff(fs, 2.0);
        let mute_amt = (mute * 1.5).min(0.985);
        let base_cut = 16000.0 * 0.12f64.powf(flats);
        let foam_cut = 16000.0 * (2500.0f64 / 16000.0).powf(mute);
        let mut peak = 0.0f64;
        for k in 0..n {
            let o = self.out.next();
            for (ci, chn) in chans.iter_mut().enumerate().take(2) {
                let c = &mut self.ch[ci];
                let x = chn[k] as f64;
                // note-attack detector: rises only on real attacks, not on the waveform
                c.pow += (x * x - c.pow) * c_pow;
                let fast = c.pow.max(0.0).sqrt();
                c.fast = fast;
                c.slow += (fast - c.slow) * c_slow;
                // a jump of more than ~1.5 dB over the recent level counts as a new note; 2.5 dB or more is a full attack
                let rise = if fast > 1e-6 { (20.0 * (fast / c.slow.max(1e-9)).log10() - 1.5) / 1.0 } else { 0.0 };
                let rise = rise.clamp(0.0, 1.0);
                // each envelope jumps up on an attack and otherwise decays at its own rate
                c.d_mute = (c.d_mute * k_mute).max(rise);
                c.d_flat = (c.d_flat * k_flat).max(rise);
                c.d_body = (c.d_body * k_body).max(rise);
                // voicing
                let mut y = cascade(&mut c.voice, x);
                y = cascade(&mut c.pick, y);
                // body thump on the attack
                y += c.body_bp.process(x) * body * body_scale * c.d_body * 1.5;
                // flatwounds: bright on the attack, dulling as the note rings; foam also dulls
                let cut = (base_cut * (1.0 + 3.0 * flats * c.d_flat)).min(foam_cut.max(base_cut * 0.5)).min(16000.0);
                y = c.svf.lp(y, fs, cut, 0.7);
                // foam mute: the note dies away after each attack (gain smoothed: no clicks, no stutter)
                let target = 1.0 - mute_amt * (1.0 - c.d_mute);
                c.g += (target - c.g) * c_gain;
                y *= c.g;
                y = c.tone.process(y) * o;
                peak = peak.max(y.abs());
                chn[k] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Hofner, Early Beatles", values: &[(BASS, 0.0), (PICKUP, 0.15), (BODY, 0.6), (FLATS, 0.7), (MUTE, 0.2), (TONE, 0.6)] },
        Preset { name: "Hofner, Muted Thump", values: &[(BASS, 0.0), (PICKUP, 0.1), (BODY, 0.75), (FLATS, 0.8), (MUTE, 0.75), (TONE, 0.5)] },
        Preset { name: "Rickenbacker, Paperback Writer", values: &[(BASS, 1.0), (PICKUP, 0.3), (BODY, 0.4), (FLATS, 0.35), (MUTE, 0.0), (TONE, 0.85)] },
        Preset { name: "Rickenbacker, Pepper Melodic", values: &[(BASS, 1.0), (PICKUP, 0.2), (BODY, 0.3), (FLATS, 0.4), (MUTE, 0.0), (TONE, 0.7)] },
        Preset { name: "Rickenbacker, Bridge Bite", values: &[(BASS, 1.0), (PICKUP, 0.85), (BODY, 0.3), (FLATS, 0.2), (MUTE, 0.1), (TONE, 0.9)] },
        Preset { name: "Dead Flats Only", values: &[(BASS, 2.0), (PICKUP, 0.5), (BODY, 0.0), (FLATS, 0.9), (MUTE, 0.0), (TONE, 0.75)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "BASS VOICE",
        subtitle: ("PAUL'S BASSES & STRINGS", "HOFNER 500/1 · RICKENBACKER 4001S"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![

            Item::Selector { p: BASS, x: 90.0, y: 160.0, label: "BASS", positions: &["HOFNER", "RICK", "AS PLAYED"], tip: "Which bass to voice a DI toward: the Hofner 500/1 violin bass, or the Rickenbacker 4001S Paul used from 1965. AS PLAYED leaves your bass alone (strings and mute still work)." },
            Item::Knob { p: PICKUP, x: 230.0, y: 160.0, r: 38.0, label: "PICKUP", tip: "Neck (round and deep) to bridge (tight, nasal). Paul mostly played the neck pickup.", active: None },
            Item::Knob { p: FLATS, x: 360.0, y: 160.0, r: 38.0, label: "FLATWOUNDS", tip: "How fast the brightness of each note fades after the attack, as with flatwound strings. Up = older, deader flats.", active: None },
            Item::Knob { p: MUTE, x: 490.0, y: 160.0, r: 38.0, label: "FOAM MUTE", tip: "Foam under the strings near the bridge: notes die away quickly into a short thump.", active: None },
            Item::Divider { y: 264.0 },
            Item::Knob { p: BODY, x: 120.0, y: 330.0, r: 24.0, label: "BODY", tip: "The low thump of the body on each attack (strongest on the hollow Hofner).", active: None },
            Item::Knob { p: TONE, x: 320.0, y: 330.0, r: 24.0, label: "TONE", tip: "The bass's tone control: rolls off the top.", active: None },
            Item::Knob { p: OUTPUT, x: 520.0, y: 330.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Bass Voice", class_id: *b"S2BassVoiceDIY01", vendor: "PEQUOD Beatles",
    categories: [Fx, Eq]
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

    /// A plucked bass: 60 Hz with harmonics, decaying slowly, re-plucked every 600 ms.
    fn plucks(secs: f64) -> Vec<f32> {
        let fs = 48000.0;
        let n = (fs * secs) as usize;
        (0..n)
            .map(|k| {
                let t = k as f64 / fs;
                let tn = t % 0.6;
                let env = (-tn / 1.2).exp();
                let ph = 2.0 * std::f64::consts::PI * 60.0 * t;
                (0.4 * env * (ph.sin() + 0.4 * (2.0 * ph).sin() + 0.2 * (3.0 * ph).sin())) as f32
            })
            .collect()
    }
    fn windows(p: &[f32]) -> Vec<f64> {
        let mut input = vec![plucks(2.4)];
        run(p, &mut input);
        input[0].chunks(1600).map(|c| db(rms(c))).collect()
    }
    #[test]
    fn foam_mute_shortens_notes() {
        let open = windows(&with(defaults(), &[(MUTE, 0.0)]));
        let muted = windows(&with(defaults(), &[(MUTE, 0.8)]));
        // windows are 2 cycles (33.3 ms); the third note starts at window 36; 400 ms later = window 48
        let (o, m) = (open[48], muted[48]);
        let (o0, m0) = (open[36], muted[36]);
        println!("at attack: open {o0:.1} muted {m0:.1} dB; 400 ms later: open {o:.1} muted {m:.1} dB");
        assert!(m < o - 15.0 && (m0 - o0).abs() < 4.0);
    }
    #[test]
    fn foam_mute_does_not_stutter() {
        // within each note, after the attack, the level must only fall (no repeated re-attacks)
        for mute in [0.3f32, 0.6, 0.8, 1.0] {
            let w = windows(&with(defaults(), &[(MUTE, mute)]));
            let mut worst = 0.0f64;
            for note in 1..4 {
                let s = note * 18 + 1;
                for i in s..(note * 18 + 17) {
                    worst = worst.max(w[i + 1] - w[i]);
                }
            }
            println!("mute {mute}: largest rise inside a note {worst:.2} dB");
            assert!(worst < 0.5, "stutter at mute {mute}: {worst}");
        }
    }
    #[test]
    fn low_e_held_note_does_not_retrigger() {
        // a held low E (41.2 Hz, one slow decay): ripple must never re-open the mute
        let fs = 48000.0;
        let n = 96000;
        let x: Vec<f32> = (0..n).map(|k| {
            let t = k as f64 / fs;
            (0.5 * (-t / 3.0).exp() * (2.0 * std::f64::consts::PI * 41.2 * t).sin()) as f32
        }).collect();
        for mute in [0.5f32, 1.0] {
            let p = with(defaults(), &[(MUTE, mute), (BASS, 2.0), (FLATS, 0.0), (BODY, 0.0)]);
            let mut input = vec![x.clone()];
            run(&p, &mut input);
            // 4-cycle windows (97.1 ms ~ 4661 samples)
            let w: Vec<f64> = input[0].chunks(4661).map(|c| db(rms(c))).collect();
            let worst = w.windows(2).skip(1).map(|p| p[1] - p[0]).fold(f64::MIN, f64::max);
            println!("held low E, mute {mute}: largest rise {worst:.2} dB, end level {:.1} dB", w[w.len() - 2]);
            assert!(worst < 0.5);
        }
    }

}
