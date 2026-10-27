//! Shared reverb building blocks: an 8-line feedback delay network with
//! three-band decay control, input diffusion and slow line modulation.

use crate::dsp::*;

pub const LINES: usize = 8;

/// Mutually prime-ish base lengths (ms) for a medium room; scaled by `size`.
const BASE_MS: [f64; LINES] = [29.7, 37.1, 41.1, 43.7, 47.9, 53.3, 59.9, 67.1];

#[derive(Clone)]
pub struct Diffuser {
    line: DelayLine,
    d: f64,
    g: f64,
}

impl Diffuser {
    pub fn new(fs: f64, ms: f64, g: f64) -> Self {
        let d = (ms * 0.001 * fs).max(1.0);
        Self { line: DelayLine::new(d as usize + 4), d, g }
    }
    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let v = self.line.read(self.d);
        let w = x + self.g * v;
        self.line.write(w);
        v - self.g * w
    }
    pub fn reset(&mut self) {
        self.line.reset();
    }
}

/// Three-band loss in the loop: complementary first-order split at
/// 250 Hz and `hi_split` Hz, each band scaled for its decay time.
#[derive(Clone, Copy, Default)]
struct Loss {
    lo: OnePole,
    hi: OnePole,
    g: [f64; 3],
}

impl Loss {
    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let l = self.lo.lp(x);
        let m = self.hi.lp(x);
        self.g[0] * l + self.g[1] * (m - l) + self.g[2] * (x - m)
    }
}

pub struct Fdn {
    fs: f64,
    lines: Vec<DelayLine>,
    len: [f64; LINES],
    loss: [Loss; LINES],
    state: [f64; LINES],
    wander: [Wander; 2],
    pub mod_depth: f64,
    diff: Vec<Diffuser>,
    pub rt: [f64; 3],
}

impl Fdn {
    /// `max_size`: the largest `size` that will be configured (line memory).
    pub fn new(fs: f64, max_size: f64) -> Self {
        let max = (BASE_MS[LINES - 1] * max_size * 0.001 * fs) as usize + 64;
        Self {
            fs,
            lines: (0..LINES).map(|_| DelayLine::new(max)).collect(),
            len: [1.0; LINES],
            loss: [Loss::default(); LINES],
            state: [0.0; LINES],
            wander: [Wander::new(fs, 0.7, 31), Wander::new(fs, 0.9, 57)],
            mod_depth: 0.0006,
            diff: vec![
                Diffuser::new(fs, 4.77, 0.7),
                Diffuser::new(fs, 3.59, 0.7),
                Diffuser::new(fs, 12.73, 0.6),
                Diffuser::new(fs, 9.31, 0.6),
            ],
            rt: [1.0; 3],
        }
    }

    /// size: 1 = medium room (~30-67 ms lines); rt = decay (s) low/mid/high.
    pub fn configure(&mut self, size: f64, rt: [f64; 3], hi_split: f64) {
        self.rt = rt;
        for i in 0..LINES {
            self.len[i] = (BASE_MS[i] * size * 0.001 * self.fs).max(8.0).min(self.lines[i].max_delay() - 8.0);
            let secs = self.len[i] / self.fs;
            let g = |t: f64| 10f64.powf(-3.0 * secs / t.max(0.02));
            self.loss[i].g = [g(rt[0]), g(rt[1]), g(rt[2])];
            self.loss[i].lo.set_lowpass(self.fs, 250.0);
            self.loss[i].hi.set_lowpass(self.fs, hi_split);
        }
    }

    pub fn reset(&mut self) {
        self.lines.iter_mut().for_each(|l| l.reset());
        self.state = [0.0; LINES];
        self.diff.iter_mut().for_each(|d| d.reset());
    }

    /// Input diffusion (smears transients before the network).
    #[inline]
    pub fn diffuse(&mut self, x: f64, amount: usize) -> f64 {
        let mut y = x;
        for d in self.diff.iter_mut().take(amount) {
            y = d.process(y);
        }
        y
    }

    /// Feed `x` (already diffused if wanted) and return the stereo output.
    #[inline]
    pub fn process(&mut self, x: f64) -> (f64, f64) {
        let w0 = self.wander[0].next();
        let w1 = self.wander[1].next();
        let mut out = [0.0; LINES];
        for i in 0..LINES {
            let m = match i {
                1 => w0,
                5 => w1,
                3 => -w0,
                6 => -w1,
                _ => 0.0,
            };
            let d = self.len[i] * (1.0 + self.mod_depth * m);
            out[i] = self.loss[i].process(self.lines[i].read(d));
        }
        // Householder feedback: y = out - 2/N * sum(out)
        let s: f64 = out.iter().sum::<f64>() * (2.0 / LINES as f64);
        for i in 0..LINES {
            let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
            let v = out[i] - s + x * sign;
            self.state[i] = v;
            self.lines[i].write(v);
        }
        let l = out[0] - out[2] + out[4] - out[6] + 0.5 * (out[1] + out[5]);
        let r = out[1] - out[3] + out[5] - out[7] + 0.5 * (out[2] + out[6]);
        (l * 0.35, r * 0.35)
    }
}

/// Schroeder T30 helper for tests: decay (s) of `h` in a band around `f0`.
pub fn t30(h: &[f64], fs: f64, f0: f64) -> f64 {
    let mut bp = Biquad::identity();
    bp.set_bandpass(fs, f0, 1.4);
    let y: Vec<f64> = h.iter().map(|v| bp.process(*v)).collect();
    let mut e: Vec<f64> = y.iter().map(|v| v * v).collect();
    for i in (0..e.len() - 1).rev() {
        e[i] += e[i + 1];
    }
    let e0 = e[0].max(1e-30);
    let db = |i: usize| 10.0 * (e[i] / e0).max(1e-30).log10();
    let i5 = (0..e.len()).find(|&i| db(i) < -5.0).unwrap_or(0);
    let i35 = (0..e.len()).find(|&i| db(i) < -35.0).unwrap_or(e.len() - 1);
    (i35 - i5) as f64 / fs * 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fdn_decays_at_set_times() {
        let fs = 48000.0;
        for rt in [[1.2, 1.0, 0.6], [3.0, 2.5, 1.5]] {
            let mut f = Fdn::new(fs, 2.0);
            f.configure(1.3, rt, 3000.0);
            let n = (fs * rt[1] * 2.5) as usize;
            let mut h = vec![0.0; n];
            for (k, v) in h.iter_mut().enumerate() {
                let x = if k == 0 { 1.0 } else { 0.0 };
                let (l, r) = f.process(x);
                *v = l + r;
            }
            let mid = t30(&h, fs, 800.0);
            let hi = t30(&h, fs, 6000.0);
            println!("set {:?}: mid {mid:.2} hi {hi:.2}", rt);
            assert!((mid / rt[1] - 1.0).abs() < 0.25);
            assert!(hi < mid);
        }
    }
}
