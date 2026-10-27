//! Output transformer and loudspeaker load.

use crate::dsp::Biquad;
use std::f64::consts::PI;

#[derive(Clone, Copy, Debug)]
pub struct TransformerSpec {
    /// Low-frequency corner: reflected source/load resistance against the
    /// primary inductance (Hz).
    pub f_low: f64,
    /// Frequency at which the core reaches saturation at full rated swing.
    pub f_sat: f64,
    /// Primary voltage (peak, plate-to-plate) at full rated power.
    pub v_full: f64,
    /// Leakage-inductance / winding-capacitance resonance (Hz) and its Q.
    pub f_high: f64,
    pub q_high: f64,
    /// Turns ratio secondary/primary.
    pub ratio: f64,
}

/// Primary inductance as a flux integrator with a saturating core: the
/// magnetising current rises steeply as the flux nears saturation, so big
/// low notes lose bass and pick up odd harmonics, as a small Vox or Fender
/// transformer does when pushed.
#[derive(Clone)]
pub struct OutputTransformer {
    w_low: f64,
    phi: f64,
    inv_sat: f64,
    dt: f64,
    leak: f64,
    hf: Biquad,
    ratio: f64,
}

impl OutputTransformer {
    pub fn new(fs: f64, s: TransformerSpec) -> Self {
        let mut hf = Biquad::identity();
        hf.set_lowpass(fs, s.f_high.min(fs * 0.45), s.q_high);
        // flux (V*s) reached by a full-power sine at f_sat
        let phi_sat = s.v_full / (2.0 * PI * s.f_sat);
        Self {
            w_low: 2.0 * PI * s.f_low,
            phi: 0.0,
            inv_sat: 1.0 / phi_sat,
            dt: 1.0 / fs,
            leak: (-2.0 * PI * 2.0 / fs).exp(),
            hf,
            ratio: s.ratio,
        }
    }

    pub fn reset(&mut self) {
        self.phi = 0.0;
        self.hf.reset();
    }

    /// Primary voltage in, secondary voltage out.
    #[inline]
    pub fn process(&mut self, v: f64) -> f64 {
        let u = self.phi * self.inv_sat;
        // inductance falls as the core saturates (halved where a full-power
        // sine at f_sat peaks): i_m ~ phi * (1 + u^2 + 2u^6), soft at first
        let u2 = u * u;
        let m = self.phi * (1.0 + u2 * (0.5 + 1.5 * u2 * u2));
        let y = v - self.w_low * m;
        self.phi = self.phi * self.leak + y * self.dt;
        self.hf.process(y) * self.ratio
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SpeakerSpec {
    /// Free-air resonance (Hz), impedance peak relative to nominal, and Q.
    pub f_res: f64,
    pub z_peak: f64,
    pub q_res: f64,
    /// Frequency where voice-coil inductance doubles the impedance.
    pub f_coil: f64,
}

/// The loudspeaker's impedance curve. A pentode output stage is close to a
/// current source, so its voltage follows the impedance (bass bump at the
/// resonance, rising top); negative feedback, taken from across the speaker,
/// pulls that back toward flat — automatically, since the amp's feedback
/// loop sees this signal.
#[derive(Clone)]
pub struct SpeakerLoad {
    res: Biquad,
    coil: Biquad,
    peak: f64,
    depth: f64,
}

impl SpeakerLoad {
    /// `depth`: how much the stage behaves as a current source (pentode
    /// ~0.85, ultra-linear ~0.55, triode ~0.3).
    pub fn new(fs: f64, s: SpeakerSpec, depth: f64) -> Self {
        let mut res = Biquad::identity();
        res.set_bandpass(fs, s.f_res, s.q_res);
        let mut coil = Biquad::identity();
        // rising inductive impedance: +6 dB/oct shelf starting at f_coil,
        // easing off as eddy currents take over (semi-inductance)
        coil.set_high_shelf(fs, s.f_coil * 1.6, 0.5, 9.0);
        Self { res, coil, peak: s.z_peak - 1.0, depth }
    }

    pub fn reset(&mut self) {
        self.res.reset();
        self.coil.reset();
    }

    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let z = self.coil.process(x) + self.peak * self.res.process(x);
        x + self.depth * (z - x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain(f: f64, mut proc: impl FnMut(f64) -> f64, amp: f64) -> f64 {
        let fs = 96000.0;
        let n = 96000;
        let (mut re, mut im, mut c) = (0.0, 0.0, 0.0);
        for k in 0..n {
            let ph = 2.0 * PI * f * k as f64 / fs;
            let y = proc(amp * ph.sin());
            if k > n / 2 {
                re += y * ph.sin();
                im += y * ph.cos();
                c += 1.0;
            }
        }
        2.0 * (re * re + im * im).sqrt() / c / amp
    }

    #[test]
    fn transformer_corner_and_saturation() {
        let spec = TransformerSpec { f_low: 40.0, f_sat: 40.0, v_full: 400.0, f_high: 16000.0, q_high: 0.7, ratio: 1.0 };
        let mut ot = OutputTransformer::new(96000.0, spec);
        let g40 = gain(40.0, |x| ot.process(x), 1.0);
        let mut ot = OutputTransformer::new(96000.0, spec);
        let g1k = gain(1000.0, |x| ot.process(x), 1.0);
        println!("40 Hz {:.2} dB, 1 kHz {:.2} dB", 20.0 * g40.log10(), 20.0 * g1k.log10());
        assert!((20.0 * (g40 / g1k).log10() + 3.0).abs() < 0.6);
        // at full swing the 40 Hz response sags further (core saturation)
        let mut ot = OutputTransformer::new(96000.0, spec);
        let g40_loud = gain(40.0, |x| ot.process(x), 400.0);
        println!("40 Hz loud {:.2} dB", 20.0 * g40_loud.log10());
        assert!(g40_loud < g40 * 0.93);
    }

    #[test]
    fn speaker_bump_at_resonance() {
        let spec = SpeakerSpec { f_res: 85.0, z_peak: 5.0, q_res: 2.5, f_coil: 1500.0 };
        let mut sp = SpeakerLoad::new(96000.0, spec, 1.0);
        let r = gain(85.0, |x| sp.process(x), 1.0);
        let mut sp = SpeakerLoad::new(96000.0, spec, 1.0);
        let m = gain(400.0, |x| sp.process(x), 1.0);
        let mut sp = SpeakerLoad::new(96000.0, spec, 1.0);
        let h = gain(6000.0, |x| sp.process(x), 1.0);
        println!("Z(85)={r:.2} Z(400)={m:.2} Z(6k)={h:.2}");
        assert!(r > 3.5 && (0.85..1.4).contains(&m) && h > 2.0);
    }
}
