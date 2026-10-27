//! Rectified: a high-gain amp in the manner of the Mesa/Boogie Triple Rectifier heard on blink-182's late-90s and 2000s records: three cascaded gain stages, a scoopable mid, sag from the rectifier, into a closed 4x12 with Vintage 30s, plus a noise gate in front. Circuit behaviour estimated.

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
pub const H: u32 = 400;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Cab { #[name = "4x12 (mic'd)"] Mic, #[name = "DI (no cab)"] Di }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "gain"]
    pub gain: FloatParam,
    #[id = "bass"]
    pub bass: FloatParam,
    #[id = "mid"]
    pub mid: FloatParam,
    #[id = "treble"]
    pub treble: FloatParam,
    #[id = "presence"]
    pub presence: FloatParam,
    #[id = "master"]
    pub master: FloatParam,
    #[id = "gate"]
    pub gate: FloatParam,
    #[id = "cab"]
    pub cab: EnumParam<Cab>,
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
            gain: FloatParam::new("Gain", 7.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            bass: FloatParam::new("Bass", 4.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            mid: FloatParam::new("Middle", 5.5, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            treble: FloatParam::new("Treble", 6.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            presence: FloatParam::new("Presence", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            master: FloatParam::new("Master", 5.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            gate: db("Gate", -60.0, -90.0, -20.0),
            cab: EnumParam::new("Cabinet", Cab::Mic),
            mic: pct("Mic Position", 0.3),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const GAIN: usize = 0;
pub const BASS: usize = 1;
pub const MID: usize = 2;
pub const TREBLE: usize = 3;
pub const PRESENCE: usize = 4;
pub const MASTER: usize = 5;
pub const GATE: usize = 6;
pub const CAB: usize = 7;
pub const MIC: usize = 8;
pub const OUTPUT: usize = 9;
pub const NPARAMS: usize = 10;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.gain.as_ptr(),
            self.bass.as_ptr(),
            self.mid.as_ptr(),
            self.treble.as_ptr(),
            self.presence.as_ptr(),
            self.master.as_ptr(),
            self.gate.as_ptr(),
            self.cab.as_ptr(),
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

// voicing
const IN_HP: f64 = 110.0;
const STAGES: usize = 3;
const G_MIN: f64 = -0.9;
const G_MAX: f64 = 2.0;
const MID_F: f64 = 750.0;
const BRIGHT_DB: f64 = 1.5;
const DIODE: bool = false;
/// cab: (low bump dB, 400 Hz dB, 1.2 kHz dB, upper-mid Hz, dB, dip Hz, dB, top Hz, dB, limit Hz)
const CAB_V: [f64; 10] = [2.5, -2.0, 2.0, 2600.0, 6.0, 3400.0, -4.0, 4600.0, 4.0, 5600.0];

#[derive(Clone, Default)]
struct Chan {
    os: Oversampler,
    hp: Biquad,
    bright: Biquad,
    inter: [Biquad; 3],
    inter_lp: [Biquad; 3],
    stack: [Biquad; 3],
    pres: Biquad,
    cab: [Biquad; 9],
    sag: f64,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ch: Vec<Chan>,
    key: Vec<i32>,
    genv: f64,
    gg: f64,
    out: Smooth,
}

impl Dsp {
    #[inline]
    pub fn stage(x: f64, bias: f64) -> f64 {
        (x + bias).tanh() - bias.tanh()
    }
    fn design(&mut self) {
        let fs = self.fs;
        let ofs = fs * OS as f64;
        let p = |i: usize| self.v[i] as f64;
        let (b, m, t, pr, mic) = (p(BASS), p(MID), p(TREBLE), p(PRESENCE), p(MIC));
        let cv = CAB_V;
        for c in self.ch.iter_mut() {
            c.hp.set_highpass(ofs, IN_HP, 0.7);
            c.bright.set_high_shelf(ofs, 2000.0, 0.7, BRIGHT_DB);
            for i in 0..3 {
                c.inter[i].set_highpass(ofs, 140.0 + 60.0 * i as f64, 0.6);
                c.inter_lp[i].set_lowpass(ofs, 9000.0 - 1500.0 * i as f64, 0.7);
            }
            // the tone stack (each knob 0-10, 5 = flat-ish)
            c.stack[0].set_low_shelf(ofs, 110.0, 0.7, (b - 5.0) * 2.2);
            c.stack[1].set_peak(ofs, MID_F, 0.8, (m - 5.0) * 2.4);
            c.stack[2].set_high_shelf(ofs, 2500.0, 0.7, (t - 5.0) * 2.0);
            // presence: less feedback in the top octave of the power amp
            c.pres.set_high_shelf(ofs, 3500.0, 0.7, (pr - 5.0) * 1.6);
            // 4x12, close-miked
            c.cab[0].set_highpass(fs, 78.0, 1.1);
            c.cab[1].set_peak(fs, 115.0, 1.2, cv[0]);
            c.cab[2].set_peak(fs, 400.0, 1.0, cv[1]);
            c.cab[3].set_peak(fs, 1200.0, 1.0, cv[2]);
            c.cab[4].set_peak(fs, cv[3], 2.2, cv[4] - 3.0 * mic);
            c.cab[5].set_peak(fs, cv[5], 3.0, cv[6]);
            c.cab[6].set_peak(fs, cv[7], 2.5, cv[8] - 3.0 * mic);
            let lp = cv[9] * 0.55f64.powf(mic);
            c.cab[7].set_lowpass(fs, lp, 0.7);
            c.cab[8].set_lowpass(fs, lp * 1.1, 0.6);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, ch: (0..ch.max(2)).map(|_| Chan { os: Oversampler::new(), ..Default::default() }).collect(), key: vec![], genv: 0.0, gg: 1.0, out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn reset(&mut self) {
        for c in self.ch.iter_mut() {
            *c = Chan { os: Oversampler::new(), ..Default::default() };
        }
        self.design();
        self.genv = 0.0;
        self.gg = 1.0;
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k: Vec<i32> = v[..NPARAMS].iter().map(|x| (x * 100.0) as i32).collect();
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().max(1) as f64;
        let gain = self.v[GAIN] as f64 / 10.0;
        let g = 10f64.powf(G_MIN + (G_MAX - G_MIN) * gain);
        let master = 0.3 + 2.5 * (self.v[MASTER] as f64 / 10.0).powi(2);
        let gate_db = self.v[GATE] as f64;
        let di = self.v[CAB] > 0.5;
        let ga = coeff(self.fs, 0.5);
        let gr = coeff(self.fs, 60.0);
        let gs = coeff(self.fs, 4.0);
        let cs = coeff(self.fs * OS as f64, 80.0);
        let last = self.ch.len() - 1;
        let mut most = 0.0f64;
        for k in 0..n {
            // noise gate on the input (as a pedal in front), keyed on all channels
            let mut a = 0.0f64;
            for c in chans.iter() {
                a = a.max((c[k] as f64).abs());
            }
            let _ = nch;
            self.genv = a + if a > self.genv { ga } else { gr } * (self.genv - a);
            let open = gate_db <= -89.0 || 20.0 * self.genv.max(1e-9).log10() > gate_db;
            let target = if open { 1.0 } else { 0.0 };
            self.gg = target + gs * (self.gg - target);
            let o = self.out.next();
            for (ci, chn) in chans.iter_mut().enumerate() {
                let c = &mut self.ch[ci.min(last)];
                let x = chn[k] as f64 * self.gg;
                let Chan { os, hp, bright, inter, inter_lp, stack, pres, sag, .. } = c;
                let y = os.process(x, |u| {
                    let mut s = bright.process(hp.process(u)) * g;
                    for i in 0..STAGES {
                        s = if DIODE && i == STAGES - 1 {
                            // diode clipping (Marshall JCM900)
                            s / (1.0 + (s / 0.7).abs().powf(2.5)).powf(1.0 / 2.5) * 1.2
                        } else {
                            Dsp::stage(s, 0.15 + 0.1 * i as f64) * 2.2
                        };
                        s = inter_lp[i].process(inter[i].process(s));
                        if i + 1 < STAGES {
                            s *= 4.0;
                        }
                    }
                    let s = cascade(&mut stack[..], s);
                    // power amp: soft, with sag from the supply under load
                    let drive = master * (1.0 - 0.25 * *sag);
                    let p = (s * drive).tanh();
                    let pa = p.abs();
                    *sag = pa + cs * (*sag - pa);
                    pres.process(p)
                });
                most = most.max(c.sag);
                let y = if di { y * 0.5 } else { cascade(&mut c.cab, y) * 0.5 };
                chn[k] = (y * o) as f32;
            }
        }
        meters[0] = most as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Enema Rhythm (mids up)", values: &[(GAIN, 7.0), (BASS, 4.0), (MID, 7.0), (TREBLE, 6.0), (PRESENCE, 5.5), (GATE, -60.0)] },
        Preset { name: "Pop-Punk Wall", values: &[(GAIN, 8.0), (BASS, 5.0), (MID, 6.0), (TREBLE, 6.0), (MASTER, 6.0)] },
        Preset { name: "Dammit Crunch", values: &[(GAIN, 5.5), (BASS, 4.0), (MID, 6.5), (TREBLE, 6.5)] },
        Preset { name: "Scooped Modern", values: &[(GAIN, 8.5), (BASS, 6.5), (MID, 2.5), (TREBLE, 7.0), (PRESENCE, 6.0)] },
        Preset { name: "Tight Palm Mutes", values: &[(GAIN, 7.5), (BASS, 3.0), (MID, 6.0), (GATE, -48.0)] },
        Preset { name: "Lead", values: &[(GAIN, 9.5), (MID, 7.0), (TREBLE, 5.5), (MASTER, 6.5)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "RECTIFIED",
        subtitle: ("HIGH GAIN IN THE MESA TRIPLE RECTIFIER MANNER", "3 GAIN STAGES · 4x12 V30 · GATE"),
        accent: ui::OXBLOOD,
        size: (W, H),
        items: vec![

            Item::Knob { p: GAIN, x: 70.0, y: 150.0, r: 30.0, label: "GAIN", tip: "Preamp gain.", active: None },
            Item::Knob { p: BASS, x: 170.0, y: 150.0, r: 30.0, label: "BASS", tip: "Bass.", active: None },
            Item::Knob { p: MID, x: 270.0, y: 150.0, r: 30.0, label: "MIDDLE", tip: "Middle. Pop-punk tones keep it up so the guitars cut.", active: None },
            Item::Knob { p: TREBLE, x: 370.0, y: 150.0, r: 30.0, label: "TREBLE", tip: "Treble.", active: None },
            Item::Knob { p: PRESENCE, x: 470.0, y: 150.0, r: 30.0, label: "PRESENCE", tip: "Top-end bite from the power amp.", active: None },
            Item::Knob { p: MASTER, x: 570.0, y: 150.0, r: 30.0, label: "MASTER", tip: "How hard the power amp is pushed (more sag and compression).", active: None },
            Item::Divider { y: 220.0 },
            Item::Knob { p: GATE, x: 80.0, y: 285.0, r: 24.0, label: "GATE", tip: "Noise gate in front (as a pedal): mutes the hiss and string noise between chords. Fully left = off.", active: None },
            Item::Selector { p: CAB, x: 210.0, y: 285.0, label: "CABINET", positions: &["MIC'D", "DI"], tip: "The 4x12, close-miked, or the amp's output alone (for your own cab)." },
            Item::Knob { p: MIC, x: 330.0, y: 285.0, r: 24.0, label: "MIC POSITION", tip: "Cone centre (bright) toward the edge (darker).", active: Some(ui::ActiveWhen { p: CAB, min: 0.0, max: 0.0 }) },
            Item::Lamp { m: 0, x: 440.0, y: 270.0, label: "SAG", lo: 0.2, hi: 0.95 },
            Item::Knob { p: OUTPUT, x: 560.0, y: 285.0, r: 24.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Rectified", class_id: *b"B182RectifiedAmp", vendor: "PEQUOD blink-182",
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
    /// harmonics (2..6) relative to the fundamental, dB
    fn thd(p: &[f32], f: f64, amp: f64) -> f64 {
        let y = go(p, &sine_amp(f, amp, 48000));
        let a = goertzel(&y[24000..], f);
        let h: f64 = (2..7).map(|k| goertzel(&y[24000..], f * k as f64).powi(2)).sum::<f64>().sqrt();
        db(h / a.max(1e-12))
    }

    #[test]
    fn gain_gate_and_cab() {
        let lo = thd(&with(defaults(), &[(GAIN, 0.0), (CAB, 1.0)]), 200.0, 0.02);
        let hi = thd(&with(defaults(), &[(GAIN, 10.0), (CAB, 1.0)]), 200.0, 0.02);
        // gate: quiet hiss is shut off
        let mut nz = vintage::dsp::Noise::new(3);
        let hiss: Vec<f32> = (0..48000).map(|_| (nz.next() * 0.0005) as f32).collect();
        let open = db(rms(&go(&with(defaults(), &[(GATE, -90.0)]), &hiss)[24000..]));
        let shut = db(rms(&go(&with(defaults(), &[(GATE, -50.0)]), &hiss)[24000..]));
        let di = db(rms(&go(&with(defaults(), &[(CAB, 1.0), (GAIN, 2.0)]), &sine_amp(9000.0, 0.01, 24000))[12000..]));
        let mic = db(rms(&go(&with(defaults(), &[(CAB, 0.0), (GAIN, 2.0)]), &sine_amp(9000.0, 0.01, 24000))[12000..]));
        println!("harmonics gain 0 {lo:.1} / 10 {hi:.1} dB; hiss gate off {open:.1}, on {shut:.1} dB; 9 kHz DI {di:.1} vs cab {mic:.1}");
        assert!(hi > lo + 15.0 && open - shut > 30.0 && di - mic > 15.0);
    }

}
