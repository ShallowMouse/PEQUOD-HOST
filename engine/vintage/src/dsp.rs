//! Shared DSP building blocks for the suite.

use std::f64::consts::PI;

#[inline]
pub fn db_to_gain(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

#[inline]
pub fn gain_to_db(g: f64) -> f64 {
    20.0 * g.abs().max(1e-9).log10()
}

/// One-pole smoothing coefficient for a time constant in milliseconds.
#[inline]
pub fn coeff(fs: f64, ms: f64) -> f64 {
    if ms <= 0.0 {
        0.0
    } else {
        (-1.0 / (fs * ms * 0.001)).exp()
    }
}

// ---------------------------------------------------------------- biquad ---

#[derive(Clone, Copy, Default, Debug)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    pub fn identity() -> Self {
        Self { b0: 1.0, ..Default::default() }
    }

    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    fn set(&mut self, b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) {
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    /// Bilinear transform of an analogue section
    /// (n2 s^2 + n1 s + n0) / (d2 s^2 + d1 s + d0), with s in rad/s.
    pub fn set_analog(&mut self, fs: f64, n: [f64; 3], d: [f64; 3]) {
        let k = 2.0 * fs;
        let k2 = k * k;
        let [n2, n1, n0] = n;
        let [d2, d1, d0] = d;
        self.set(
            n2 * k2 + n1 * k + n0,
            2.0 * (n0 - n2 * k2),
            n2 * k2 - n1 * k + n0,
            d2 * k2 + d1 * k + d0,
            2.0 * (d0 - d2 * k2),
            d2 * k2 - d1 * k + d0,
        );
    }

    fn w(fs: f64, f: f64) -> (f64, f64) {
        let w = 2.0 * PI * (f / fs).clamp(1e-5, 0.49);
        w.sin_cos()
    }

    pub fn set_highpass(&mut self, fs: f64, f: f64, q: f64) {
        let (s, c) = Self::w(fs, f);
        let alpha = s / (2.0 * q);
        self.set((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha);
    }

    pub fn set_lowpass(&mut self, fs: f64, f: f64, q: f64) {
        let (s, c) = Self::w(fs, f);
        let alpha = s / (2.0 * q);
        self.set((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha);
    }

    pub fn set_bandpass(&mut self, fs: f64, f: f64, q: f64) {
        let (s, c) = Self::w(fs, f);
        let alpha = s / (2.0 * q);
        self.set(alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * c, 1.0 - alpha);
    }

    pub fn set_peak(&mut self, fs: f64, f: f64, q: f64, gain_db: f64) {
        let a = 10f64.powf(gain_db / 40.0);
        let (s, c) = Self::w(fs, f);
        let alpha = s / (2.0 * q);
        self.set(1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a);
    }

    /// RBJ low shelf with slope S (1.0 = steepest without overshoot).
    pub fn set_low_shelf(&mut self, fs: f64, f: f64, slope: f64, gain_db: f64) {
        let a = 10f64.powf(gain_db / 40.0);
        let (s, c) = Self::w(fs, f);
        let alpha = s / 2.0 * ((a + 1.0 / a) * (1.0 / slope - 1.0) + 2.0).max(0.0).sqrt();
        let sa = 2.0 * a.sqrt() * alpha;
        self.set(
            a * ((a + 1.0) - (a - 1.0) * c + sa),
            2.0 * a * ((a - 1.0) - (a + 1.0) * c),
            a * ((a + 1.0) - (a - 1.0) * c - sa),
            (a + 1.0) + (a - 1.0) * c + sa,
            -2.0 * ((a - 1.0) + (a + 1.0) * c),
            (a + 1.0) + (a - 1.0) * c - sa,
        );
    }

    pub fn set_high_shelf(&mut self, fs: f64, f: f64, slope: f64, gain_db: f64) {
        let a = 10f64.powf(gain_db / 40.0);
        let (s, c) = Self::w(fs, f);
        let alpha = s / 2.0 * ((a + 1.0 / a) * (1.0 / slope - 1.0) + 2.0).max(0.0).sqrt();
        let sa = 2.0 * a.sqrt() * alpha;
        self.set(
            a * ((a + 1.0) + (a - 1.0) * c + sa),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
            a * ((a + 1.0) + (a - 1.0) * c - sa),
            (a + 1.0) - (a - 1.0) * c + sa,
            2.0 * ((a - 1.0) - (a + 1.0) * c),
            (a + 1.0) - (a - 1.0) * c - sa,
        );
    }
}

// ---------------------------------------------------------- one-pole bits ---

/// One-pole low-pass (6 dB/oct).
#[derive(Clone, Copy, Default)]
pub struct OnePole {
    a: f64,
    z: f64,
}

impl OnePole {
    pub fn set_lowpass(&mut self, fs: f64, f: f64) {
        self.a = (-2.0 * PI * (f / fs).clamp(1e-6, 0.49)).exp();
    }
    #[inline]
    pub fn lp(&mut self, x: f64) -> f64 {
        self.z = x + self.a * (self.z - x);
        self.z
    }
    #[inline]
    pub fn hp(&mut self, x: f64) -> f64 {
        x - self.lp(x)
    }
    pub fn reset(&mut self) {
        self.z = 0.0;
    }
}

/// DC blocker.
#[derive(Clone, Copy)]
pub struct DcBlock {
    r: f64,
    x1: f64,
    y1: f64,
}

impl DcBlock {
    pub fn new(fs: f64) -> Self {
        Self { r: (-2.0 * PI * 8.0 / fs).exp(), x1: 0.0, y1: 0.0 }
    }
    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let y = x - self.x1 + self.r * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.y1 = 0.0;
    }
}

/// Per-sample parameter smoother (one-pole).
#[derive(Clone, Copy)]
pub struct Smooth {
    pub value: f64,
    target: f64,
    a: f64,
}

impl Smooth {
    pub fn new(fs: f64, ms: f64, v: f64) -> Self {
        Self { value: v, target: v, a: coeff(fs, ms) }
    }
    pub fn set(&mut self, t: f64) {
        self.target = t;
    }
    pub fn snap(&mut self, t: f64) {
        self.target = t;
        self.value = t;
    }
    #[inline]
    pub fn next(&mut self) -> f64 {
        self.value = self.target + self.a * (self.value - self.target);
        self.value
    }
}

// ------------------------------------------------------------ saturation ---

/// Asymmetric tanh: unity small-signal gain, zero output for zero input.
/// `bias` > 0 adds even (2nd) harmonics like a single-ended tube stage.
#[inline]
pub fn tube(x: f64, bias: f64) -> f64 {
    let tb = bias.tanh();
    ((x + bias).tanh() - tb) / (1.0 - tb * tb)
}

/// Softer, tape-like symmetric curve (odd harmonics, gentle onset).
#[inline]
pub fn tape_curve(x: f64) -> f64 {
    // x / (1 + |x|^2.5)^(1/2.5): very soft knee, approaches +-1
    let p = 2.5;
    x / (1.0 + x.abs().powf(p)).powf(1.0 / p)
}

// ------------------------------------------------------- 4x oversampling ---

pub const OS: usize = 4;
const TAPS: usize = 97;
const PHASE_LEN: usize = (TAPS + OS - 1) / OS;
/// Latency of up+down filtering, in base-rate samples.
pub const OS_LATENCY: usize = (TAPS - 1) / OS; // 24

fn design_fir() -> [f64; TAPS] {
    let fc = 0.112;
    let beta = 8.0;
    let m = (TAPS - 1) as f64;
    let i0 = |x: f64| {
        let mut sum = 1.0;
        let mut term = 1.0;
        for k in 1..50 {
            term *= (x / 2.0) / k as f64;
            sum += term * term;
        }
        sum
    };
    let mut h = [0.0; TAPS];
    let mut total = 0.0;
    for (n, tap) in h.iter_mut().enumerate() {
        let t = n as f64 - m / 2.0;
        let sinc = if t == 0.0 { 2.0 * fc } else { (2.0 * PI * fc * t).sin() / (PI * t) };
        let r = 2.0 * n as f64 / m - 1.0;
        let w = i0(beta * (1.0 - r * r).max(0.0).sqrt()) / i0(beta);
        *tap = sinc * w;
        total += *tap;
    }
    for tap in h.iter_mut() {
        *tap /= total;
    }
    h
}

#[derive(Clone)]
pub struct Oversampler {
    h: [f64; TAPS],
    up_hist: [f64; PHASE_LEN],
    down_hist: [f64; TAPS],
}

impl Default for Oversampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Oversampler {
    pub fn new() -> Self {
        Self { h: design_fir(), up_hist: [0.0; PHASE_LEN], down_hist: [0.0; TAPS] }
    }

    pub fn reset(&mut self) {
        self.up_hist = [0.0; PHASE_LEN];
        self.down_hist = [0.0; TAPS];
    }

    /// Upsample one sample, run `f` on each hi-rate sample, decimate back.
    #[inline]
    pub fn process(&mut self, x: f64, mut f: impl FnMut(f64) -> f64) -> f64 {
        self.up_hist.copy_within(0..PHASE_LEN - 1, 1);
        self.up_hist[0] = x;
        for p in 0..OS {
            let mut acc = 0.0;
            let mut k = 0;
            while p + OS * k < TAPS {
                acc += self.h[p + OS * k] * self.up_hist[k];
                k += 1;
            }
            let y = f(acc * OS as f64);
            self.down_hist.copy_within(0..TAPS - 1, 1);
            self.down_hist[0] = y;
        }
        let mut out = 0.0;
        for i in 0..TAPS {
            out += self.h[i] * self.down_hist[i];
        }
        out
    }
}

/// Fixed integer delay (for aligning a dry signal with an oversampled path).
#[derive(Clone)]
pub struct FixedDelay {
    buf: Vec<f64>,
    pos: usize,
}

impl FixedDelay {
    pub fn new(samples: usize) -> Self {
        Self { buf: vec![0.0; samples + 1], pos: 0 }
    }
    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let n = self.buf.len();
        self.buf[self.pos] = x;
        self.pos = (self.pos + 1) % n;
        self.buf[self.pos]
    }
    pub fn reset(&mut self) {
        self.buf.iter_mut().for_each(|v| *v = 0.0);
    }
}

// ---------------------------------------------------- modulated delay line ---

/// Ring-buffer delay with fractional (4-point Hermite) reads.
#[derive(Clone)]
pub struct DelayLine {
    buf: Vec<f64>,
    mask: usize,
    pos: usize,
}

impl DelayLine {
    pub fn new(max_samples: usize) -> Self {
        let n = (max_samples + 4).next_power_of_two();
        Self { buf: vec![0.0; n], mask: n - 1, pos: 0 }
    }

    pub fn max_delay(&self) -> f64 {
        (self.buf.len() - 4) as f64
    }

    #[inline]
    pub fn write(&mut self, x: f64) {
        self.pos = (self.pos + 1) & self.mask;
        self.buf[self.pos] = x;
    }

    /// Read `d` samples behind the most recent write (d >= 1 recommended).
    #[inline]
    pub fn read(&self, d: f64) -> f64 {
        let d = d.clamp(1.0, self.max_delay());
        let di = d.floor();
        let t = d - di;
        let i = di as usize;
        let at = |k: usize| self.buf[(self.pos.wrapping_sub(k)) & self.mask];
        let xm1 = at(i.wrapping_sub(1));
        let x0 = at(i);
        let x1 = at(i + 1);
        let x2 = at(i + 2);
        // Hermite between x0 (t=0) and x1 (t=1)
        let c0 = x0;
        let c1 = 0.5 * (x1 - xm1);
        let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
        let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
        ((c3 * t + c2) * t + c1) * t + c0
    }

    #[inline]
    pub fn read_int(&self, d: usize) -> f64 {
        self.buf[(self.pos.wrapping_sub(d.min(self.mask))) & self.mask]
    }

    pub fn reset(&mut self) {
        self.buf.iter_mut().for_each(|v| *v = 0.0);
    }
}

// ----------------------------------------------------------- modulation ---

/// Cheap deterministic noise.
#[derive(Clone, Copy)]
pub struct Noise(u32);

impl Noise {
    pub fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }
    /// White noise in [-1, 1).
    #[inline]
    pub fn next(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f64 / u32::MAX as f64) * 2.0 - 1.0
    }
}

/// Smoothly wandering random signal in roughly [-1, 1], centred on 0.
#[derive(Clone, Copy)]
pub struct Wander {
    noise: Noise,
    a: OnePole,
    b: OnePole,
    gain: f64,
}

impl Wander {
    pub fn new(fs: f64, rate_hz: f64, seed: u32) -> Self {
        let mut w = Self { noise: Noise::new(seed), a: OnePole::default(), b: OnePole::default(), gain: 1.0 };
        w.set_rate(fs, rate_hz);
        w
    }
    pub fn set_rate(&mut self, fs: f64, rate_hz: f64) {
        let r = rate_hz.max(0.01);
        self.a.set_lowpass(fs, r);
        self.b.set_lowpass(fs, r);
        // two cascaded one-poles on white noise: rms ~ sqrt(pi*f/fs)/..; normalise empirically
        self.gain = (fs / (r * 2.0)).sqrt() * 0.9;
    }
    #[inline]
    pub fn next(&mut self) -> f64 {
        let n = self.noise.next();
        (self.b.lp(self.a.lp(n)) * self.gain).clamp(-1.5, 1.5)
    }
}

/// Sine LFO.
#[derive(Clone, Copy, Default)]
pub struct Lfo {
    pub phase: f64, // 0..1
}

impl Lfo {
    #[inline]
    pub fn next(&mut self, fs: f64, hz: f64) -> f64 {
        self.phase = (self.phase + hz / fs).fract();
        (2.0 * PI * self.phase).sin()
    }
}

// --------------------------------------------------------- envelope etc. ---

/// Attack/release envelope follower in the linear domain.
#[derive(Clone, Copy)]
pub struct Envelope {
    pub value: f64,
    att: f64,
    rel: f64,
}

impl Envelope {
    pub fn new(fs: f64, att_ms: f64, rel_ms: f64) -> Self {
        Self { value: 0.0, att: coeff(fs, att_ms), rel: coeff(fs, rel_ms) }
    }
    pub fn set_times(&mut self, fs: f64, att_ms: f64, rel_ms: f64) {
        self.att = coeff(fs, att_ms);
        self.rel = coeff(fs, rel_ms);
    }
    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let x = x.abs();
        let c = if x > self.value { self.att } else { self.rel };
        self.value = x + c * (self.value - x);
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_line_fractional_read_is_accurate() {
        let mut d = DelayLine::new(1024);
        let f = 0.01;
        for n in 0..2000 {
            d.write((2.0 * PI * f * n as f64).sin());
        }
        let n = 1999.0;
        let delay = 100.37;
        let expect = (2.0 * PI * f * (n - delay)).sin();
        assert!((d.read(delay) - expect).abs() < 1e-3);
    }

    #[test]
    fn shelves_have_expected_gain() {
        let fs = 48000.0;
        let mut lo = Biquad::default();
        lo.set_low_shelf(fs, 200.0, 1.0, 6.0);
        // DC gain should be +6 dB
        let mut y = 0.0;
        for _ in 0..48000 {
            y = lo.process(1.0);
        }
        assert!((gain_to_db(y) - 6.0).abs() < 0.1, "{}", gain_to_db(y));
    }

    #[test]
    fn wander_is_bounded_and_moves() {
        let mut w = Wander::new(48000.0, 1.0, 3);
        let (mut lo, mut hi) = (0.0f64, 0.0f64);
        for _ in 0..480000 {
            let v = w.next();
            lo = lo.min(v);
            hi = hi.max(v);
        }
        assert!(hi > 0.3 && lo < -0.3 && hi <= 1.5 && lo >= -1.5, "{lo} {hi}");
    }
}

/// Run `x` through a chain of biquads in order.
#[inline]
pub fn cascade(chain: &mut [Biquad], x: f64) -> f64 {
    chain.iter_mut().fold(x, |y, b| b.process(y))
}
