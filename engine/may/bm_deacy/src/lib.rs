//! Deacy: the tiny amp John Deacon built from a Supersonic PR80 transistor radio's circuit board, run off a 9-volt battery with no controls, that Brian May drives with the Red Special and his treble booster for his guitar orchestras (Procession, Good Company). A transistor class-B output stage (crossover distortion that grows as the battery dies) into a small speaker in a little box; May used a wah pedal as its tone control and moved the mic on and off the speaker for trumpet- and trombone-like tones. Circuit behaviour estimated.

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
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "battery"]
    pub battery: FloatParam,
    #[id = "wahon"]
    pub wahon: BoolParam,
    #[id = "wah"]
    pub wah: FloatParam,
    #[id = "mic"]
    pub mic: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            drive: db("Drive", 18.0, 0.0, 36.0),
            battery: pct("Battery", 0.85),
            wahon: BoolParam::new("Wah Tone", true),
            wah: pct("Wah Position", 0.45),
            mic: pct("Mic Off-Axis", 0.25),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const DRIVE: usize = 0;
pub const BATTERY: usize = 1;
pub const WAHON: usize = 2;
pub const WAH: usize = 3;
pub const MIC: usize = 4;
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
            self.drive.as_ptr(),
            self.battery.as_ptr(),
            self.wahon.as_ptr(),
            self.wah.as_ptr(),
            self.mic.as_ptr(),
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

struct Chan {
    os: Oversampler,
    hp_in: Biquad,
    wah: Biquad,
    ot: [Biquad; 2],
    spk: [Biquad; 5],
    env: f64,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ch: Vec<Chan>,
    key: [i32; 4],
    out: Smooth,
}

impl Dsp {
    /// Class-B pair: a dead band either side of zero (crossover) that widens as the
    /// battery runs down, then the rails.
    #[inline]
    pub fn stage(x: f64, dead: f64, rail: f64) -> f64 {
        let y = x - dead * (x / dead.max(1e-6)).tanh();
        let y = y * (1.0 + dead);
        rail * (y / rail).tanh()
    }
    fn design(&mut self) {
        let fs = self.fs;
        let ofs = fs * OS as f64;
        let wah = self.v[WAH] as f64;
        let mic = self.v[MIC] as f64;
        for c in self.ch.iter_mut() {
            c.hp_in.set_highpass(fs, 110.0, 0.7);
            // wah as a fixed tone control (Cry Baby range)
            let f = 380.0 * (2200.0f64 / 380.0).powf(wah);
            c.wah.set_bandpass(fs, f, 4.5);
            // the radio's little output transformer
            c.ot[0].set_highpass(ofs, 160.0, 0.7);
            c.ot[1].set_lowpass(ofs, 7000.0, 0.7);
            // small twin-cone speaker in a little bookshelf box
            c.spk[0].set_highpass(fs, 190.0, 1.2);
            c.spk[1].set_peak(fs, 1200.0, 1.2, 4.0);
            c.spk[2].set_peak(fs, 2800.0, 2.0, 5.0 - 6.0 * mic);
            c.spk[3].set_lowpass(fs, 6500.0 * 0.45f64.powf(mic), 0.7);
            c.spk[4].set_peak(fs, 600.0, 1.0, 2.0 * mic);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mk = || Chan { os: Oversampler::new(), hp_in: Biquad::identity(), wah: Biquad::identity(), ot: [Biquad::identity(); 2], spk: [Biquad::identity(); 5], env: 0.0 };
        let mut d = Self { fs, v, ch: (0..ch.max(2)).map(|_| mk()).collect(), key: [-1; 4], out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn reset(&mut self) {
        for c in self.ch.iter_mut() {
            c.os.reset();
            c.hp_in.reset();
            c.wah.reset();
            c.ot.iter_mut().chain(c.spk.iter_mut()).for_each(|b| b.reset());
            c.env = 0.0;
        }
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = [(v[WAH] * 400.0) as i32, (v[MIC] * 400.0) as i32, 0, 0];
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let drive = db_to_gain(self.v[DRIVE] as f64);
        let batt = self.v[BATTERY] as f64;
        let wah_on = self.v[WAHON] > 0.5;
        // a fresh 9 V battery: rails ~ +-4 V into the speaker; a tired one sags and the bias collapses
        let rail = 0.35 + 0.65 * batt;
        let dead = 0.002 + 0.08 * (1.0 - batt).powi(2);
        let ce = coeff(self.fs * OS as f64, 60.0);
        let mut most = 0.0f64;
        for k in 0..n {
            let o = self.out.next();
            for (ci, chn) in chans.iter_mut().enumerate() {
                let c = &mut self.ch[ci.min(1)];
                let mut x = c.hp_in.process(chn[k] as f64);
                if wah_on {
                    x = c.wah.process(x) * 2.5;
                }
                let x = x * drive;
                let (ot, env) = (&mut c.ot, &mut c.env);
                let y = c.os.process(x, |u| {
                    // germanium driver: soft and lopsided
                    let pre = if u >= 0.0 { u / (1.0 + u.abs()) } else { u / (1.0 + 1.4 * u.abs()) };
                    // the battery sags under load
                    let r = rail * (1.0 - 0.35 * *env);
                    let s = Dsp::stage(pre * 1.6, dead, r);
                    let a = s.abs() / rail.max(1e-3);
                    *env = a + ce * (*env - a);
                    cascade(&mut ot[..], s)
                });
                most = most.max(c.env);
                let y = cascade(&mut c.spk, y);
                chn[k] = (y * 0.9 * o) as f32;
            }
        }
        meters[0] = most as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Procession Violin", values: &[(DRIVE, 22.0), (WAH, 0.55), (MIC, 0.2)] },
        Preset { name: "Good Company Trumpet", values: &[(DRIVE, 20.0), (WAH, 0.65), (MIC, 0.0)] },
        Preset { name: "Trombone (off-axis, low wah)", values: &[(DRIVE, 20.0), (WAH, 0.2), (MIC, 0.7)] },
        Preset { name: "Clarinet", values: &[(DRIVE, 16.0), (WAH, 0.4), (MIC, 0.45)] },
        Preset { name: "Cello", values: &[(DRIVE, 18.0), (WAH, 0.12), (MIC, 0.55)] },
        Preset { name: "No Wah, Raw Radio Amp", values: &[(WAHON, 0.0), (DRIVE, 18.0)] },
        Preset { name: "Dying Battery Fuzz", values: &[(WAHON, 0.0), (BATTERY, 0.2), (DRIVE, 24.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "DEACY",
        subtitle: ("JOHN DEACON'S TRANSISTOR RADIO AMP", "9 V · CLASS B · SMALL SPEAKER · WAH AS TONE"),
        accent: ui::OXBLOOD,
        size: (W, H),
        items: vec![

            Item::Knob { p: DRIVE, x: 110.0, y: 165.0, r: 42.0, label: "DRIVE", tip: "How hard the guitar and treble booster hit it. The Deacy has no controls of its own: May set the sound with the guitar, the booster and the mic.", active: None },
            Item::Knob { p: BATTERY, x: 250.0, y: 165.0, r: 30.0, label: "BATTERY", tip: "The 9 V battery: fresh (right) to tired (left). A tired battery sags and the crossover distortion grows, fuzzier and spittier.", active: None },
            Item::Meter { m: 0, x: 330.0, y: 104.0, w: 150.0, h: 110.0, kind: MeterKind::Vu, caption: "OUTPUT" },
            Item::Knob { p: MIC, x: 560.0, y: 165.0, r: 30.0, label: "MIC", tip: "Mic on the speaker axis (bright, trumpet-like) or moved off it (rounder, trombone- and cello-like), as May did.", active: None },
            Item::Divider { y: 250.0 },
            Item::Switch { p: WAHON, x: 90.0, y: 320.0, labels: ["OFF", "WAH"], tip: "A wah pedal set in one place in front of the amp: May's tone control for the Deacy." },
            Item::Knob { p: WAH, x: 230.0, y: 320.0, r: 30.0, label: "WAH POSITION", tip: "Where the wah pedal sits: heel (low, cello and trombone) to toe (high, violin and trumpet).", active: Some(ui::ActiveWhen { p: WAHON, min: 1.0, max: 1.0 }) },
            Item::Knob { p: OUTPUT, x: 520.0, y: 320.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Deacy", class_id: *b"BMyDeacyAmpPQD01", vendor: "PEQUOD Brian May",
    categories: [Fx, Distortion]
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
    fn sine_amp(f: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n).map(|k| (amp * (2.0 * PI * f * k as f64 / 48000.0).sin()) as f32).collect()
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
    fn go2(p: &[f32], x: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        let r = input.pop().unwrap();
        (input.pop().unwrap(), r)
    }

    fn thd(set: &[(usize, f32)], amp: f64) -> f64 {
        let p = with(defaults(), &[&[(WAHON, 0.0f32)][..], set].concat());
        let y = go(&p, &sine_amp(400.0, amp, 48000));
        let f = goertzel(&y[24000..], 400.0);
        let h: f64 = (2..6).map(|k| goertzel(&y[24000..], 400.0 * k as f64).powi(2)).sum::<f64>().sqrt();
        db(h / f)
    }
    #[test]
    fn drive_and_battery() {
        let lo = thd(&[(DRIVE, 0.0)], 0.05);
        let hi = thd(&[(DRIVE, 30.0)], 0.1);
        let tired = thd(&[(DRIVE, 0.0), (BATTERY, 0.1)], 0.02);
        println!("harmonics: low drive {lo:.1} dB, high drive {hi:.1} dB, tired battery quiet {tired:.1} dB");
        assert!(hi > lo + 15.0 && tired > lo + 6.0);
    }
    #[test]
    fn little_speaker_has_no_bass() {
        let p = with(defaults(), &[(WAHON, 0.0), (DRIVE, 0.0)]);
        let a = db(rms(&go(&p, &sine_amp(70.0, 0.02, 48000))[24000..]));
        let b = db(rms(&go(&p, &sine_amp(1000.0, 0.02, 48000))[24000..]));
        println!("70 Hz {a:.1} dB vs 1 kHz {b:.1} dB");
        assert!(b - a > 18.0);
    }

}
