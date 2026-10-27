//! A behavioural amp engine for the circuits the component-level model
//! doesn't cover (Marshall, Mesa, Soldano, Dumble, Hiwatt and friends):
//! cascaded triode stages as biased soft clippers with interstage
//! filtering, a tone stack, and a push-pull power stage with rectifier
//! sag, presence and resonance, and the output transformer's bandwidth.
//! Runs at the oversampled rate; voicings are estimated from each amp's
//! published circuit and behaviour.

use crate::dsp::{coeff, Biquad};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Power {
    El34,
    L6L6,
    El84,
    Kt88,
}

#[derive(Clone, Copy, Debug)]
pub struct Stack {
    pub bass_f: f64,
    pub mid_f: f64,
    pub mid_q: f64,
    pub treble_f: f64,
    /// Mid level with every knob at noon (dB): Marshall/Fender stacks scoop.
    pub scoop: f64,
    /// dB per half-turn of each knob.
    pub range: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct HgSpec {
    /// Gain stages (1..=4).
    pub stages: usize,
    /// log10 of the input gain at GAIN 0 and 10.
    pub gain_lo: f64,
    pub gain_hi: f64,
    /// Gain between stages.
    pub stage_gain: f64,
    /// Operating-point offset of each stage (sets the asymmetry).
    pub bias: [f64; 4],
    /// Interstage coupling high-pass (Hz).
    pub couple_hp: [f64; 4],
    /// Plate/Miller low-pass on each stage (Hz).
    pub stage_lp: f64,
    pub in_hp: f64,
    /// Bright cap: shelf frequency and dB at low gain (it fades out as gain goes up).
    pub bright: (f64, f64),
    /// Last preamp stage cold-biased (the JCM800 trick).
    pub cold: bool,
    /// Diode clipping after the preamp (JCM900 and some pedals-in-amps).
    pub diode: bool,
    pub stack: Stack,
    pub power: Power,
    /// Higher = a cleaner power amp.
    pub headroom: f64,
    /// Rectifier sag 0..1.
    pub sag: f64,
    /// Has a master volume; without one, GAIN (the volume) drives the power amp too.
    pub master: bool,
    /// Output transformer band (Hz).
    pub ot: (f64, f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HgControls {
    pub gain: f64,
    pub bass: f64,
    pub mid: f64,
    pub treble: f64,
    pub presence: f64,
    pub depth: f64,
    pub master: f64,
}

impl Default for HgControls {
    fn default() -> Self {
        Self { gain: 0.5, bass: 0.5, mid: 0.5, treble: 0.5, presence: 0.5, depth: 0.5, master: 0.5 }
    }
}

#[inline]
fn triode(x: f64, b: f64) -> f64 {
    // grid conduction squashes the positive side early and softly; cut-off
    // limits the negative side later and harder
    let f = |v: f64| if v >= 0.0 { v.tanh() } else { 1.6 * (v / 1.6).tanh() };
    f(x + b) - f(b)
}

#[inline]
fn power(x: f64, p: Power) -> f64 {
    match p {
        Power::El34 => (x + 0.06).tanh() - 0.06f64.tanh(),
        Power::L6L6 => x / (1.0 + x.abs().powf(2.4)).powf(1.0 / 2.4),
        Power::El84 => {
            let y = (x * 1.25).tanh() / 1.25;
            y + 0.08 * y * y
        }
        Power::Kt88 => x / (1.0 + x.powi(4)).powf(0.25),
    }
}

#[derive(Clone)]
pub struct HgAmp {
    fs: f64,
    s: HgSpec,
    c: HgControls,
    in_hp: Biquad,
    bright: Biquad,
    hp: [Biquad; 4],
    lp: [Biquad; 4],
    stack: [Biquad; 3],
    presence: Biquad,
    depth: Biquad,
    ot: [Biquad; 2],
    sag_env: f64,
    sag_c: f64,
    pre_gain: f64,
    drive: f64,
    dc: f64,
}

impl HgAmp {
    /// `fs`: the oversampled rate it runs at.
    pub fn new(fs: f64, s: HgSpec) -> Self {
        let mut a = Self {
            fs,
            s,
            c: HgControls::default(),
            in_hp: Biquad::identity(),
            bright: Biquad::identity(),
            hp: [Biquad::identity(); 4],
            lp: [Biquad::identity(); 4],
            stack: [Biquad::identity(); 3],
            presence: Biquad::identity(),
            depth: Biquad::identity(),
            ot: [Biquad::identity(); 2],
            sag_env: 0.0,
            sag_c: coeff(fs, 90.0),
            pre_gain: 1.0,
            drive: 1.0,
            dc: 0.0,
        };
        a.in_hp.set_highpass(fs, s.in_hp, 0.7);
        for i in 0..4 {
            a.hp[i].set_highpass(fs, s.couple_hp[i], 0.6);
            a.lp[i].set_lowpass(fs, s.stage_lp * (1.0 - 0.08 * i as f64), 0.7);
        }
        a.ot[0].set_highpass(fs, s.ot.0, 0.7);
        a.ot[1].set_lowpass(fs, s.ot.1, 0.7);
        a.set_controls(HgControls::default());
        a
    }

    pub fn set_controls(&mut self, c: HgControls) {
        if c == self.c && self.drive != 1.0 {
            return;
        }
        self.c = c;
        let s = self.s;
        let fs = self.fs;
        self.pre_gain = 10f64.powf(s.gain_lo + (s.gain_hi - s.gain_lo) * c.gain);
        self.bright.set_high_shelf(fs, s.bright.0, 0.7, s.bright.1 * (1.0 - c.gain).powi(2));
        let st = s.stack;
        self.stack[0].set_low_shelf(fs, st.bass_f, 0.7, (c.bass - 0.5) * 2.0 * st.range);
        self.stack[1].set_peak(fs, st.mid_f, st.mid_q, st.scoop + (c.mid - 0.5) * 2.0 * st.range);
        self.stack[2].set_high_shelf(fs, st.treble_f, 0.7, (c.treble - 0.5) * 2.0 * st.range);
        self.presence.set_high_shelf(fs, 3500.0, 0.7, (c.presence - 0.5) * 12.0);
        self.depth.set_low_shelf(fs, 90.0, 0.7, (c.depth - 0.5) * 12.0);
        let push = if s.master { c.master } else { c.gain };
        self.drive = (0.25 + 3.2 * push * push) / s.headroom;
    }

    pub fn reset(&mut self) {
        self.in_hp.reset();
        self.bright.reset();
        self.hp.iter_mut().chain(self.lp.iter_mut()).for_each(|b| b.reset());
        self.stack.iter_mut().for_each(|b| b.reset());
        self.presence.reset();
        self.depth.reset();
        self.ot.iter_mut().for_each(|b| b.reset());
        self.sag_env = 0.0;
        self.dc = 0.0;
    }

    /// How hard the power stage is working (0..1-ish), for a SAG lamp.
    pub fn sag(&self) -> f64 {
        self.sag_env
    }

    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let s = &self.s;
        let v0 = self.in_hp.process(x);
        let mut v = self.bright.process(v0) * self.pre_gain;
        let n = s.stages.clamp(1, 4);
        for i in 0..n {
            let b = if s.cold && i == n - 1 { -0.7 } else { s.bias[i] };
            v = triode(v, b);
            v = self.hp[i].process(v);
            v = self.lp[i].process(v);
            if i + 1 < n {
                v *= s.stage_gain;
            }
        }
        if s.diode {
            v = v / (1.0 + (v / 0.6).abs().powf(2.5)).powf(0.4);
        }
        let mut t = v;
        for b in self.stack.iter_mut() {
            t = b.process(t);
        }
        // power amp, with the supply sagging under load
        let d = self.drive * (1.0 - 0.35 * s.sag * self.sag_env);
        let p = power(t * d, s.power);
        let a = p.abs().min(1.5) / 1.5;
        self.sag_env = a + self.sag_c * (self.sag_env - a);
        let p = self.presence.process(p);
        let p = self.depth.process(p);
        let y = self.ot[0].process(p);
        let y = self.ot[1].process(y);
        y * 0.5
    }
}

// ------------------------------------------------------------- voicings --

const MARSHALL: Stack = Stack { bass_f: 110.0, mid_f: 650.0, mid_q: 0.7, treble_f: 2200.0, scoop: -4.0, range: 7.0 };
const MESA: Stack = Stack { bass_f: 120.0, mid_f: 750.0, mid_q: 0.8, treble_f: 2600.0, scoop: -2.0, range: 8.0 };
const HIWATT: Stack = Stack { bass_f: 100.0, mid_f: 700.0, mid_q: 0.6, treble_f: 2500.0, scoop: -1.0, range: 8.0 };
const DUMBLE: Stack = Stack { bass_f: 120.0, mid_f: 600.0, mid_q: 0.7, treble_f: 2400.0, scoop: -3.0, range: 6.0 };

/// Marshall 1959 Super Lead "plexi", 1968-69: two preamp stages and a cathode
/// follower, no master volume, four EL34s. Volume drives everything.
pub fn plexi() -> HgSpec {
    HgSpec {
        stages: 2, gain_lo: -0.3, gain_hi: 1.4, stage_gain: 6.0, bias: [0.1, 0.2, 0.0, 0.0],
        couple_hp: [80.0, 120.0, 0.0, 0.0], stage_lp: 12000.0, in_hp: 40.0, bright: (1800.0, 6.0),
        cold: false, diode: false, stack: MARSHALL, power: Power::El34, headroom: 1.0, sag: 0.3, master: false, ot: (40.0, 11000.0),
    }
}

/// Marshall JTM45, 1965: the Bassman-derived first Marshall: KT66s, GZ34 sag,
/// softer and darker than the plexi.
pub fn jtm45() -> HgSpec {
    HgSpec { gain_lo: -0.4, gain_hi: 1.25, bright: (1600.0, 4.0), power: Power::L6L6, sag: 0.6, ot: (45.0, 9500.0), ..plexi() }
}

/// Marshall JCM800 2203, 1981: master volume, an extra cold-biased stage.
pub fn jcm800() -> HgSpec {
    HgSpec {
        stages: 3, gain_lo: -1.2, gain_hi: 1.6, stage_gain: 6.0, bias: [0.1, 0.15, 0.0, 0.0],
        couple_hp: [80.0, 300.0, 150.0, 0.0], stage_lp: 10000.0, in_hp: 60.0, bright: (2000.0, 5.0),
        cold: true, diode: false, stack: MARSHALL, power: Power::El34, headroom: 1.0, sag: 0.25, master: true, ot: (45.0, 11000.0),
    }
}

/// Marshall JCM900 4100, 1990: master volume, diode clipping in the preamp.
pub fn jcm900() -> HgSpec {
    HgSpec { gain_lo: -1.0, gain_hi: 1.7, diode: true, cold: false, bias: [0.1, 0.2, 0.15, 0.0], stack: Stack { scoop: -1.0, ..MARSHALL }, ..jcm800() }
}

/// Hiwatt DR103, early 70s: a very clean, stiff preamp and four EL34s with
/// big transformers; it gets loud long before it distorts.
pub fn hiwatt() -> HgSpec {
    HgSpec {
        stages: 2, gain_lo: -0.7, gain_hi: 1.05, stage_gain: 3.0, bias: [0.05, 0.05, 0.0, 0.0],
        couple_hp: [60.0, 80.0, 0.0, 0.0], stage_lp: 14000.0, in_hp: 35.0, bright: (2500.0, 3.0),
        cold: false, diode: false, stack: HIWATT, power: Power::El34, headroom: 1.8, sag: 0.1, master: true, ot: (30.0, 14000.0),
    }
}

/// Mesa/Boogie Mark IIB lead channel, early 80s: cascaded gain stages, a
/// thick mid-forward lead, 6L6s.
pub fn mark2b() -> HgSpec {
    HgSpec {
        stages: 4, gain_lo: -0.6, gain_hi: 1.3, stage_gain: 4.0, bias: [0.1, 0.2, 0.15, 0.1],
        couple_hp: [60.0, 200.0, 250.0, 150.0], stage_lp: 7000.0, in_hp: 60.0, bright: (2200.0, 3.0),
        cold: false, diode: false, stack: Stack { scoop: 1.0, ..MESA }, power: Power::L6L6, headroom: 1.2, sag: 0.2, master: true, ot: (45.0, 10000.0),
    }
}

/// Mesa/Boogie Dual/Triple Rectifier, 1990s: three stages, scoopable, tube
/// rectifier sag.
pub fn rectifier() -> HgSpec {
    HgSpec {
        stages: 3, gain_lo: -1.2, gain_hi: 1.8, stage_gain: 7.0, bias: [0.1, 0.2, 0.15, 0.0],
        couple_hp: [100.0, 160.0, 220.0, 0.0], stage_lp: 9000.0, in_hp: 90.0, bright: (2000.0, 2.0),
        cold: false, diode: false, stack: MESA, power: Power::L6L6, headroom: 1.0, sag: 0.5, master: true, ot: (45.0, 11000.0),
    }
}

/// Soldano SLO-100 overdrive channel, 1987: four cascaded stages, tight lows,
/// smooth sustaining lead, EL34s.
pub fn slo() -> HgSpec {
    HgSpec {
        stages: 4, gain_lo: -1.1, gain_hi: 1.4, stage_gain: 5.0, bias: [0.1, 0.15, 0.2, 0.1],
        couple_hp: [120.0, 200.0, 180.0, 100.0], stage_lp: 9000.0, in_hp: 90.0, bright: (2200.0, 3.0),
        cold: true, diode: false, stack: MARSHALL, power: Power::El34, headroom: 1.1, sag: 0.2, master: true, ot: (45.0, 11000.0),
    }
}

/// Dumble Overdrive Special style overdrive channel: a Fender-like front end
/// pushed through extra gain stages, vocal mids, 6L6s with plenty of headroom.
pub fn ods() -> HgSpec {
    HgSpec {
        stages: 3, gain_lo: -1.1, gain_hi: 1.2, stage_gain: 5.0, bias: [0.05, 0.12, 0.08, 0.0],
        couple_hp: [60.0, 150.0, 120.0, 0.0], stage_lp: 8000.0, in_hp: 50.0, bright: (1800.0, 4.0),
        cold: false, diode: false, stack: DUMBLE, power: Power::L6L6, headroom: 1.4, sag: 0.15, master: true, ot: (40.0, 12000.0),
    }
}

/// Wizard Modern Classic, 2000s: a hot-rodded plexi with a master volume.
pub fn wizard() -> HgSpec {
    HgSpec {
        stages: 3, gain_lo: -1.1, gain_hi: 1.5, stage_gain: 5.5, bias: [0.1, 0.2, 0.1, 0.0],
        couple_hp: [80.0, 180.0, 160.0, 0.0], stage_lp: 11000.0, in_hp: 60.0, bright: (1900.0, 4.0),
        cold: false, diode: false, stack: Stack { scoop: -2.5, ..MARSHALL }, power: Power::El34, headroom: 1.0, sag: 0.25, master: true, ot: (40.0, 11500.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thd(spec: HgSpec, gain: f64) -> f64 {
        let fs = 192000.0;
        let mut a = HgAmp::new(fs, spec);
        a.set_controls(HgControls { gain, ..Default::default() });
        let n = 192000;
        let f = 220.0;
        let y: Vec<f64> = (0..n).map(|k| a.process(0.1 * (2.0 * std::f64::consts::PI * f * k as f64 / fs).sin())).collect();
        let g = |h: f64| {
            let w = 2.0 * std::f64::consts::PI * f * h / fs;
            let (mut s1, mut s2) = (0.0, 0.0);
            for v in &y[n / 2..] {
                let s = v + 2.0 * w.cos() * s1 - s2;
                s2 = s1;
                s1 = s;
            }
            (s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2).max(0.0).sqrt()
        };
        let h: f64 = (2..7).map(|k| g(k as f64).powi(2)).sum::<f64>().sqrt();
        20.0 * (h / g(1.0)).log10()
    }

    #[test]
    fn every_voicing_cleans_up_and_breaks_up() {
        for (name, s) in [("plexi", plexi()), ("jtm45", jtm45()), ("jcm800", jcm800()), ("jcm900", jcm900()), ("hiwatt", hiwatt()),
                          ("mark2b", mark2b()), ("rectifier", rectifier()), ("slo", slo()), ("ods", ods()), ("wizard", wizard())] {
            let lo = thd(s, 0.0);
            let hi = thd(s, 1.0);
            println!("{name}: harmonics gain 0 {lo:.1} dB, gain 10 {hi:.1} dB");
            assert!(hi > lo + 10.0 && hi > -18.0, "{name}");
        }
    }
}
