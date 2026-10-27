//! Valve Jr: a stock Epiphone Valve Junior head (5 W, single-ended class A: one 12AX7, one EL84, a single volume knob, no tone stack, no negative feedback) on its 16 ohm tap into one 16 ohm Celestion Vintage 30. Stage by stage: 12AX7 (100k plate, 1.5k cathode, bypassed) into the 1M volume pot, second 12AX7 stage, then the EL84 cathode-biased with 220 ohm and running hot (about 340 V, 44 mA, as measured in a published teardown), straight into the output transformer. Without feedback the EL84's output follows the speaker's impedance: the V30's bass resonance and rising top shape the amp.

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
use vintage::amp::tube::{GridInput, TriodeStage, ECC83, EL84};
use vintage::amp::output::{OutputTransformer, SpeakerLoad, SpeakerSpec, TransformerSpec};

pub const W: u32 = 640;
pub const H: u32 = 432;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Cab { #[name = "Closed 1x12"] Closed, #[name = "Open-back 1x12"] Open, #[name = "No speaker (DI)"] Di }
const OS: usize = 4;
// stock values (see the doc comment); preamp supply ~297 V, EL84 plate ~342 V, screen ~318 V
const VB_PRE: f64 = 297.0;
const VB_PLATE: f64 = 342.0;
const VB_SCREEN: f64 = 318.0;
const RK_EL84: f64 = 220.0;
// reflected primary load on the 16 ohm tap with a 16 ohm speaker (estimated: typical single-ended EL84 transformer)
const R_PRIMARY: f64 = 5000.0;
const R_SUPPLY: f64 = 220.0;


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "volume"]
    pub volume: FloatParam,
    #[id = "input"]
    pub input: FloatParam,
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
            volume: FloatParam::new("Volume", 4.0, FloatRange::Linear { min: 0.0, max: 10.0 }).with_step_size(0.1),
            input: db("Guitar Level", 0.0, -24.0, 12.0),
            cab: EnumParam::new("Cabinet", Cab::Closed),
            mic: pct("Mic Position", 0.3),
            output: db("Output", 0.0, -30.0, 12.0),
        }
    }
}

pub const VOLUME: usize = 0;
pub const INPUT: usize = 1;
pub const CAB: usize = 2;
pub const MIC: usize = 3;
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
            self.volume.as_ptr(),
            self.input.as_ptr(),
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

/// The amp from input jack to output-transformer secondary, run at OS x the host rate.
struct Amp {
    g1: GridInput,
    v1a: TriodeStage,
    c1: GridInput,
    g2: GridInput,
    v1b: TriodeStage,
    g3: GridInput,
    ip: f64,
    i0: f64,
    vk: f64,
    vb: f64,
    vb_src: f64,
    sag_k: f64,
    ot: OutputTransformer,
    fs_os: f64,
    wiper: f64,
    pub clip: f64,
}

impl Amp {
    fn new(fs_os: f64) -> Self {
        // input jack: 68k stopper into 1M grid leak
        let g1 = GridInput::new(fs_os, 68e3, 0.0, 1e6, 120e-12);
        let v1a = TriodeStage::new(fs_os, ECC83, VB_PRE, 100e3, 1.5e3, 22e-6, 1e6);
        // V1A plate -> 22 nF -> top of the 1M volume pot (no grid here, so no grid current)
        let c1 = GridInput::new(fs_os, 100e3, 22e-9, 1e6, 1e-12);
        let g2 = GridInput::new(fs_os, 250e3, 0.0, 1e9, 120e-12);
        let v1b = TriodeStage::new(fs_os, ECC83, VB_PRE, 100e3, 1.5e3, 22e-6, 470e3);
        // V1B plate -> 22 nF -> EL84 grid (470k leak, 1k stopper)
        let g3 = GridInput::new(fs_os, 101e3, 22e-9, 470e3, 250e-12);
        // EL84 idle: cathode bias with 220 ohm, bypassed
        let (mut a, mut b) = (0.0, 0.12);
        for _ in 0..80 {
            let m = 0.5 * (a + b);
            let vk = RK_EL84 * m;
            let i = EL84.ip_pentode(-vk, VB_SCREEN - vk, VB_PLATE - vk) + EL84.ig2(-vk, VB_SCREEN - vk);
            if i > m { a = m } else { b = m }
        }
        let it = 0.5 * (a + b);
        let vk = RK_EL84 * it;
        let i0 = EL84.ip_pentode(-vk, VB_SCREEN - vk, VB_PLATE - vk);
        let ot = OutputTransformer::new(fs_os, TransformerSpec {
            // small single-ended transformer: lean bass, saturates early on low notes (estimated)
            f_low: 70.0,
            f_sat: 90.0,
            v_full: 300.0,
            f_high: 14000.0,
            q_high: 0.8,
            ratio: (16.0f64 / R_PRIMARY).sqrt(),
        });
        Self {
            g1, v1a, c1, g2, v1b, g3,
            ip: i0, i0, vk, vb: VB_PLATE, vb_src: VB_PLATE + R_SUPPLY * it,
            sag_k: 1.0 - (-1.0 / (0.03 * fs_os)).exp(),
            ot, fs_os, wiper: 0.4, clip: 0.0,
        }
    }
    fn reset(&mut self) {
        self.g1.reset();
        self.v1a.reset();
        self.c1.reset();
        self.g2.reset();
        self.v1b.reset();
        self.g3.reset();
        self.ot.reset();
        self.ip = self.i0;
        self.vb = VB_PLATE;
    }
    /// volume 0..10 -> wiper fraction of an audio-taper 1M pot
    fn set_volume(&mut self, vol: f64) {
        let t = (vol / 10.0).clamp(0.0, 1.0);
        // 10% audio taper: about 10% of the signal at half rotation
        let a = if t <= 0.0 { 0.0 } else { (10f64.powf(2.0 * t) - 1.0) / 99.0 };
        self.wiper = a;
        let r_wiper = (a * (1.0 - a) * 1e6).max(1.0) + 0.0;
        self.g2.set_source(self.fs_os, r_wiper, 120e-12);
    }
    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let v1a_bias = self.v1a.vgk_bias();
        let va = &self.v1a;
        let vg = self.g1.process(x, |v| {
            let vgk = v + v1a_bias - va.vk_ac();
            (ECC83.ig(vgk), ECC83.ig_deriv(vgk))
        });
        let p1 = self.v1a.process(vg);
        let top = self.c1.process(p1, |_| (0.0, 0.0));
        let wiper = top * self.wiper;
        let v1b_bias = self.v1b.vgk_bias();
        let vb2 = &self.v1b;
        let vg2 = self.g2.process(wiper, |v| {
            let vgk = v + v1b_bias - vb2.vk_ac();
            (ECC83.ig(vgk), ECC83.ig_deriv(vgk))
        });
        let p2 = self.v1b.process(vg2);
        // EL84 grid: grid current above 0 V charges the coupling cap (blocking)
        let vk = self.vk;
        let vg3 = self.g3.process(p2, |v| (EL84.ig(v - vk), EL84.ig_deriv(v - vk)));
        let vgk = vg3 - vk;
        self.clip = self.clip.max(EL84.ig(vgk));
        // single-ended pentode on a resistive (reflected) load: solve ip = Ip(vgk, vg2, vb - RL (ip - i0))
        let vg2k = (self.vb * VB_SCREEN / VB_PLATE - vk).max(1.0);
        let mut ip = self.ip;
        for _ in 0..4 {
            let vpk = self.vb - R_PRIMARY * (ip - self.i0) - vk;
            let f = ip - EL84.ip_pentode(vgk, vg2k, vpk.max(0.0));
            let d = 1e-5;
            let vpk2 = self.vb - R_PRIMARY * (ip + d - self.i0) - vk;
            let f2 = (ip + d) - EL84.ip_pentode(vgk, vg2k, vpk2.max(0.0));
            let df = (f2 - f) / d;
            let step = f / df.max(1e-6);
            ip -= step;
            ip = ip.max(0.0);
            if step.abs() < 1e-7 {
                break;
            }
        }
        self.ip = ip;
        // supply sag (solid-state rectifier, modest filtering)
        let target = self.vb_src - R_SUPPLY * ip;
        self.vb += (target - self.vb) * self.sag_k;
        // primary voltage (plate swing about idle) through the transformer
        let v_primary = -R_PRIMARY * (ip - self.i0);
        self.ot.process(v_primary)
    }
}

pub struct Dsp {
    fs: f64,
    amps: [Amp; 2],
    up: [Oversampler; 2],
    spk_z: [SpeakerLoad; 2],
    cab: [[Biquad; 9]; 2],
    cab_key: (usize, f64),
    v: [f32; NPARAMS],
    vol_key: f64,
    input: Smooth,
    out: Smooth,
    clip_meter: f64,
}

impl Dsp {
    fn set_cab(&mut self, cab: usize, mic: f64) {
        let fs = self.fs;
        for c in &mut self.cab {
            *c = [Biquad::identity(); 9];
            if cab == 2 {
                continue;
            }
            // Vintage 30 in a 1x12 (estimated from its published character: tight bass, upper-mid peaks near 2.5 and 4.5 kHz, steep fall above ~5 kHz)
            let closed = cab == 0;
            c[0].set_highpass(fs, if closed { 80.0 } else { 105.0 }, if closed { 1.1 } else { 0.8 });
            c[1].set_peak(fs, if closed { 115.0 } else { 140.0 }, 1.2, if closed { 2.5 } else { 0.5 });
            c[2].set_peak(fs, 450.0, 1.0, -2.0);
            c[3].set_peak(fs, 1300.0, 1.0, 2.0);
            c[4].set_peak(fs, 2600.0, 2.2, 6.0 - 3.0 * mic);
            c[5].set_peak(fs, 3400.0, 3.0, -4.0);
            c[6].set_peak(fs, 4600.0, 3.0, 4.0 - 3.0 * mic);
            // mic from cone centre (bright) towards the edge (darker)
            let lp = 5600.0 * (0.55f64).powf(mic);
            c[7].set_lowpass(fs, lp, 0.75);
            c[8].set_lowpass(fs, lp * 1.25, 0.6);
        }
        self.cab_key = (cab, mic);
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let fs_os = fs * OS as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        // V30 16 ohm in a small cab: resonance ~95 Hz, impedance peak ~4.5x nominal, coil rise from ~1.2 kHz (estimated)
        let spec = SpeakerSpec { f_res: 95.0, z_peak: 4.5, q_res: 3.0, f_coil: 1200.0 };
        let mut d = Self {
            fs,
            amps: [Amp::new(fs_os), Amp::new(fs_os)],
            up: [Oversampler::new(), Oversampler::new()],
            // pentode, no feedback: the voltage follows the speaker's impedance
            spk_z: [SpeakerLoad::new(fs, spec, 0.85), SpeakerLoad::new(fs, spec, 0.85)],
            cab: [[Biquad::identity(); 9]; 2],
            cab_key: (9, -1.0),
            v,
            vol_key: -1.0,
            input: Smooth::new(fs, 20.0, 1.0),
            out: Smooth::new(fs, 20.0, 1.0),
            clip_meter: 0.0,
        };
        d.set_cab(0, 0.3);
        for a in &mut d.amps {
            a.set_volume(4.0);
        }
        d
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.amps[c].reset();
            self.up[c].reset();
            self.spk_z[c].reset();
            self.cab[c].iter_mut().for_each(|b| b.reset());
        }
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let vol = v[VOLUME] as f64;
        if (vol - self.vol_key).abs() > 1e-4 {
            for a in &mut self.amps {
                a.set_volume(vol);
            }
            self.vol_key = vol;
        }
        let k = ((v[CAB].round() as usize).min(2), v[MIC] as f64);
        if k.0 != self.cab_key.0 || (k.1 - self.cab_key.1).abs() > 1e-3 {
            self.set_cab(k.0, k.1);
        }
        self.input.set(db_to_gain(v[INPUT] as f64));
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(2);
        let di = self.cab_key.0 == 2;
        for c in 0..nch {
            self.amps[c].clip = 0.0;
        }
        for k in 0..n {
            let gi = self.input.next();
            let o = self.out.next();
            for c in 0..nch {
                // full-scale input = about 1 V peak from the guitar
                let x = chans[c][k] as f64 * gi;
                let amp = &mut self.amps[c];
                let sec = self.up[c].process(x, |s| amp.process(s));
                let y = self.spk_z[c].process(sec);
                let y = if di { y } else { cascade(&mut self.cab[c], y) };
                // ~ +/-15 V on the secondary at full power -> about 0 dBFS
                chans[c][k] = (y / 18.0 * o) as f32;
            }
        }
        let clip = self.amps.iter().take(nch).fold(0.0f64, |m, a| m.max(a.clip));
        self.clip_meter = clip;
        meters[0] = (clip * 2000.0).min(1.0) as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Clean-ish (Volume 2)", values: &[(VOLUME, 2.0)] },
        Preset { name: "Edge of Breakup (Volume 3)", values: &[(VOLUME, 3.0)] },
        Preset { name: "Crunch (Volume 5)", values: &[(VOLUME, 5.0)] },
        Preset { name: "Cranked (Volume 8)", values: &[(VOLUME, 8.0)] },
        Preset { name: "Flat Out (Volume 10)", values: &[(VOLUME, 10.0)] },
        Preset { name: "Humbucker Crunch (hot input)", values: &[(VOLUME, 4.5), (INPUT, 4.0)] },
        Preset { name: "Single-Coil Crunch", values: &[(VOLUME, 6.0), (INPUT, -3.0)] },
        Preset { name: "Open-Back Cab", values: &[(VOLUME, 5.0), (CAB, 1.0)] },
        Preset { name: "Mic on the Edge (darker)", values: &[(VOLUME, 6.0), (MIC, 0.8)] },
        Preset { name: "DI (your own cab sim)", values: &[(VOLUME, 6.0), (CAB, 2.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "VALVE JR",
        subtitle: ("STOCK EPIPHONE VALVE JUNIOR HEAD", "16 Ω TAP INTO ONE 16 Ω VINTAGE 30"),
        accent: ui::OXBLOOD,
        size: (W, H),
        items: vec![

            Item::Knob { p: VOLUME, x: 320.0, y: 165.0, r: 58.0, label: "VOLUME", tip: "The only control on a stock Valve Junior. It sits between the two 12AX7 stages, so it sets both loudness and how hard the EL84 is driven: clean down low, breaking up from about 3, saturated by 8.", active: None },
            Item::Lamp { m: 0, x: 470.0, y: 120.0, label: "EL84 GRID", lo: 0.05, hi: 0.8 },
            Item::Text { x: 120.0, y: 120.0, text: "5 W  CLASS A", size: 13.0, align: Align2::CENTER_CENTER },
            Item::Text { x: 120.0, y: 145.0, text: "12AX7  ·  EL84", size: 12.0, align: Align2::CENTER_CENTER },
            Item::Text { x: 120.0, y: 170.0, text: "16 Ω  >  1 x V30 16 Ω", size: 12.0, align: Align2::CENTER_CENTER },
            Item::Divider { y: 264.0 },
            Item::Knob { p: INPUT, x: 90.0, y: 330.0, r: 22.0, label: "GUITAR LEVEL", tip: "How hot the guitar hits the input. 0 dB treats a full-scale signal as about 1 V from the pickups; turn down for single coils, up for a quiet DI.", active: None },
            Item::Selector { p: CAB, x: 240.0, y: 330.0, label: "CABINET", positions: &["CLOSED", "OPEN", "DI"], tip: "One Vintage 30 in a closed-back or open-back 1x12, close-miked. DI: the amp's output with no speaker (the V30's impedance still shapes the amp, as it would on the 16 ohm tap)." },
            Item::Knob { p: MIC, x: 390.0, y: 330.0, r: 22.0, label: "MIC POSITION", tip: "Close mic from the centre of the cone (bright, the V30's upper-mid bite) towards the edge (darker, rounder).", active: None },
            Item::Knob { p: OUTPUT, x: 540.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Valve Jr V30", class_id: *b"PQValveJrV30PQD1", vendor: "PEQUOD",
    categories: [Fx, Distortion]
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

    fn harmonics(set: &[(usize, f32)], amp: f64) -> f64 {
        let p = with(defaults(), set);
        let mut input = vec![sine(220.0, amp, 48000)];
        run(&p, &mut input);
        let x = &input[0][24000..];
        let tone = |f: f64| {
            let (mut s, mut c) = (0.0, 0.0);
            for (k, v) in x.iter().enumerate() {
                let ph = 2.0 * std::f64::consts::PI * f * k as f64 / 48000.0;
                s += *v as f64 * ph.sin();
                c += *v as f64 * ph.cos();
            }
            (s * s + c * c).sqrt()
        };
        let h: f64 = (2..6).map(|k| tone(220.0 * k as f64).powi(2)).sum::<f64>().sqrt();
        db(h / tone(220.0))
    }
    #[test]
    fn breaks_up_as_volume_rises() {
        let a = harmonics(&[(VOLUME, 1.0), (CAB, 2.0)], 0.2);
        let b = harmonics(&[(VOLUME, 8.0), (CAB, 2.0)], 0.2);
        println!("harmonics: volume 1 {a:.1} dB, volume 8 {b:.1} dB");
        assert!(a < -25.0 && b > -12.0);
    }
    #[test]
    fn idle_point_is_hot() {
        let a = Amp::new(192000.0);
        println!("EL84 idle {:.1} mA, cathode {:.2} V", a.i0 * 1000.0, a.vk);
        assert!(a.i0 > 0.03 && a.i0 < 0.06);
    }
    #[test]
    fn v30_cuts_the_fizz() {
        let lvl = |cab: f32, f: f64| {
            let p = with(defaults(), &[(VOLUME, 1.0), (CAB, cab)]);
            let mut input = vec![sine(f, 0.05, 48000)];
            run(&p, &mut input);
            rms(&input[0][24000..])
        };
        let di = db(lvl(2.0, 9000.0) / lvl(2.0, 1000.0));
        let cab = db(lvl(0.0, 9000.0) / lvl(0.0, 1000.0));
        println!("9 kHz rel 1 kHz: DI {di:.1} dB, V30 cab {cab:.1} dB");
        assert!(cab < di - 20.0);
    }

}
