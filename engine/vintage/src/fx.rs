//! Shared effect building blocks: a two-grain pitch shifter, a bucket-brigade
//! style modulated delay, a 2:1 compander and simple gate/compressor cores.

use crate::dsp::*;
use std::f64::consts::PI;

/// Delay-line pitch shifter with two crossfaded read heads (raised-cosine).
pub struct PitchShift {
    line: DelayLine,
    phase: f64,
    win: f64,
}

impl PitchShift {
    /// `win_ms`: grain length (longer = smoother, more smeared).
    pub fn new(fs: f64, win_ms: f64) -> Self {
        let win = win_ms * 0.001 * fs;
        Self { line: DelayLine::new(win as usize + 8), phase: 0.0, win }
    }
    pub fn reset(&mut self) {
        self.line.reset();
        self.phase = 0.0;
    }
    /// ratio: output pitch / input pitch.
    #[inline]
    pub fn process(&mut self, x: f64, ratio: f64) -> f64 {
        self.line.write(x);
        if (ratio - 1.0).abs() < 1e-6 {
            return self.line.read(2.0);
        }
        // delay shrinks (pitch up) or grows (pitch down) at rate (1 - ratio)
        self.phase = (self.phase + (1.0 - ratio) / self.win).rem_euclid(1.0);
        let p2 = (self.phase + 0.5).fract();
        let d1 = 2.0 + self.phase * (self.win - 4.0);
        let d2 = 2.0 + p2 * (self.win - 4.0);
        let g1 = (PI * self.phase).sin();
        let g2 = (PI * p2).sin();
        // equal-power-ish: sin^2 + cos^2 windows sum to 1 in power
        (self.line.read(d1) * g1 + self.line.read(d2) * g2) * 0.85
    }
}

/// Bucket-brigade style delay: band-limited in and out, modulated read.
pub struct Bbd {
    pub line: DelayLine,
    pre: [Biquad; 2],
    post: [Biquad; 2],
    fs: f64,
    bw: f64,
}

impl Bbd {
    pub fn new(fs: f64, max_ms: f64) -> Self {
        let mut b = Self {
            line: DelayLine::new((max_ms * 0.001 * fs) as usize + 8),
            pre: [Biquad::identity(); 2],
            post: [Biquad::identity(); 2],
            fs,
            bw: 0.0,
        };
        b.set_bandwidth(8000.0);
        b
    }
    /// Anti-alias / reconstruction bandwidth (4th-order each side).
    pub fn set_bandwidth(&mut self, hz: f64) {
        if (hz - self.bw).abs() < 1.0 {
            return;
        }
        self.bw = hz;
        let hz = hz.min(self.fs * 0.45);
        self.pre[0].set_lowpass(self.fs, hz, 0.54);
        self.pre[1].set_lowpass(self.fs, hz, 1.31);
        self.post[0].set_lowpass(self.fs, hz, 0.54);
        self.post[1].set_lowpass(self.fs, hz, 1.31);
    }
    pub fn reset(&mut self) {
        self.line.reset();
        for b in self.pre.iter_mut().chain(self.post.iter_mut()) {
            b.reset();
        }
    }
    /// Write `x`, read at `delay` samples.
    #[inline]
    pub fn process(&mut self, x: f64, delay: f64) -> f64 {
        let w = cascade(&mut self.pre, x);
        // the chip clips softly near its rails
        self.line.write((w * 0.8).tanh() / 0.8);
        let y = self.line.read(delay.max(1.0));
        cascade(&mut self.post, y)
    }
}

/// 2:1 compander (dbx/NE570-like) around a noisy path: `encode` before,
/// `decode` after, with the decoder tracking the encoder's control slowly
/// enough to "breathe" a little as the hardware did.
pub struct Compander {
    enc: Envelope,
    dec: Envelope,
}

impl Compander {
    pub fn new(fs: f64) -> Self {
        Self { enc: Envelope::new(fs, 3.0, 60.0), dec: Envelope::new(fs, 3.0, 60.0) }
    }
    pub fn reset(&mut self) {
        self.enc.value = 0.0;
        self.dec.value = 0.0;
    }
    #[inline]
    pub fn encode(&mut self, x: f64) -> f64 {
        // 2:1 in the log domain, gain limited to +20 dB at low level
        let e = self.enc.process(x).max(0.01);
        x / e.sqrt() * 0.3
    }
    #[inline]
    pub fn decode(&mut self, y: f64) -> f64 {
        let e = self.dec.process(y / 0.3).max(0.1);
        // y = x / sqrt(ex) * k  ->  |y|/k ~ sqrt(ex)  ->  x ~ y/k * env(y/k)
        y / 0.3 * e
    }
}

/// Fixed-point quantiser (no dither): `bits` of resolution over +/-1.
#[inline]
pub fn quantise(x: f64, bits: u32) -> f64 {
    let q = (1u64 << (bits - 1)) as f64;
    (x.clamp(-1.0, 1.0) * q).round() / q
}

/// Feed-forward compressor gain computer (log domain, soft knee).
/// Returns gain reduction in dB (>= 0) for an input level in dB.
#[inline]
pub fn comp_gr(level_db: f64, threshold: f64, ratio: f64, knee: f64) -> f64 {
    let over = level_db - threshold;
    let slope = 1.0 - 1.0 / ratio.max(1.0);
    if over <= -knee * 0.5 {
        0.0
    } else if over >= knee * 0.5 || knee <= 0.0 {
        over * slope
    } else {
        let t = over + knee * 0.5;
        slope * t * t / (2.0 * knee)
    }
}

/// Noise gate with hold, as on console channel dynamics: opens fast when the
/// key exceeds the threshold, holds, then closes at the release rate down to
/// `range` dB of attenuation.
pub struct Gate {
    fs: f64,
    env: f64,
    hold_left: f64,
    pub gain: f64,
    att: f64,
    rel: f64,
    det: f64,
}

impl Gate {
    pub fn new(fs: f64) -> Self {
        Self { fs, env: 0.0, hold_left: 0.0, gain: 0.0, att: 0.0, rel: 0.0, det: coeff(fs, 5.0) }
    }
    pub fn reset(&mut self) {
        self.env = 0.0;
        self.hold_left = 0.0;
        self.gain = 0.0;
    }
    pub fn set_times(&mut self, attack_ms: f64, release_ms: f64) {
        self.att = 1.0 - coeff(self.fs, attack_ms.max(0.01));
        // release as time to fall 60 dB in linear gain terms (approx)
        self.rel = 1.0 - coeff(self.fs, (release_ms / 6.9).max(0.05));
    }
    /// Returns linear gain (range..1) for this sample.
    #[inline]
    pub fn process(&mut self, key: f64, threshold_db: f64, hold_ms: f64, range_db: f64) -> f64 {
        let k = key.abs();
        self.env = if k > self.env { k } else { k + self.det * (self.env - k) };
        let floor = db_to_gain(-range_db.abs());
        if gain_to_db(self.env) > threshold_db {
            self.hold_left = hold_ms * 0.001 * self.fs;
            self.gain += (1.0 - self.gain) * self.att;
        } else if self.hold_left > 0.0 {
            self.hold_left -= 1.0;
            self.gain += (1.0 - self.gain) * self.att;
        } else {
            self.gain += (0.0 - self.gain) * self.rel;
        }
        floor + (1.0 - floor) * self.gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitch_shift_moves_frequency() {
        let fs = 48000.0;
        let mut p = PitchShift::new(fs, 40.0);
        let n = 48000;
        let y: Vec<f64> = (0..n).map(|k| p.process((2.0 * PI * 440.0 * k as f64 / fs).sin(), 2.0)).collect();
        // count zero crossings in the second half
        let z = y[24000..].windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        let f = z as f64 / 0.5;
        println!("octave up: {f:.0} Hz");
        assert!((f - 880.0).abs() < 60.0);
    }

    #[test]
    fn gate_holds_then_closes() {
        let fs = 48000.0;
        let mut g = Gate::new(fs);
        g.set_times(0.1, 50.0);
        let mut out = vec![];
        for k in 0..48000 {
            let key = if k < 480 { 0.5 } else { 0.0 };
            out.push(g.process(key, -30.0, 200.0, 80.0));
        }
        // open during hold (at 150 ms), closed by 400 ms
        assert!(out[7200] > 0.9 && out[19200] < 0.01, "{} {}", out[7200], out[19200]);
    }

    #[test]
    fn comp_curve() {
        assert_eq!(comp_gr(-30.0, -20.0, 4.0, 0.0), 0.0);
        assert!((comp_gr(-10.0, -20.0, 4.0, 0.0) - 7.5).abs() < 1e-9);
    }
}
