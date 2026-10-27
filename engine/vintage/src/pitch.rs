//! YIN pitch tracker (de Cheveigné & Kawahara) with continuity tracking
//! for bends and vibrato. Shared by Feedback and Tuner.

use crate::dsp::OnePole;


/// YIN pitch tracker running on a decimated copy of the input.
pub struct Pitch {
    decim: usize,
    win: usize,
    hop: usize,
    fs_dec: f64,
    aa1: OnePole,
    aa2: OnePole,
    phase: usize,
    ring: Vec<f64>,
    pos: usize,
    since: usize,
    diff: Vec<f64>,
    frame: Vec<f64>,
    tau_min: usize,
    tau_max: usize,
    /// Last confidently detected frequency (Hz), if any.
    pub freq: Option<f64>,
    /// Period being followed (decimated samples), for continuous tracking
    /// through bends and vibrato.
    track: Option<f64>,
}

impl Pitch {
    /// Defaults suited to following a guitar note: 4x decimation, 85 ms window.
    pub fn new(fs: f64) -> Self {
        Self::with(fs, 4, 1024, 128, 55.0, 1100.0)
    }

    /// `decim`: decimation factor; `win`/`hop` in decimated samples.
    pub fn with(fs: f64, decim: usize, win: usize, hop: usize, fmin: f64, fmax: f64) -> Self {
        let fs_dec = fs / decim as f64;
        let mut aa1 = OnePole::default();
        let mut aa2 = OnePole::default();
        aa1.set_lowpass(fs, fs_dec * 0.2);
        aa2.set_lowpass(fs, fs_dec * 0.2);
        let tau_min = (fs_dec / fmax).floor().max(2.0) as usize;
        let tau_max = (fs_dec / fmin).ceil() as usize;
        Self {
            decim,
            win,
            hop,
            fs_dec,
            aa1,
            aa2,
            phase: 0,
            ring: vec![0.0; win],
            pos: 0,
            since: 0,
            diff: vec![0.0; tau_max + 2],
            frame: vec![0.0; win],
            tau_min,
            tau_max: tau_max.min(win / 2 - 1),
            freq: None,
            track: None,
        }
    }

    pub fn reset(&mut self) {
        self.ring.iter_mut().for_each(|v| *v = 0.0);
        self.freq = None;
        self.track = None;
    }

    /// A new note was picked: look for its pitch afresh.
    pub fn new_note(&mut self) {
        self.track = None;
    }

    #[inline]
    pub fn push(&mut self, x: f64) {
        let y = self.aa2.lp(self.aa1.lp(x));
        self.phase += 1;
        if self.phase < self.decim {
            return;
        }
        self.phase = 0;
        self.ring[self.pos] = y;
        self.pos = (self.pos + 1) % self.win;
        self.since += 1;
        if self.since >= self.hop {
            self.since = 0;
            self.analyse();
        }
    }

    fn analyse(&mut self) {
        for i in 0..self.win {
            self.frame[i] = self.ring[(self.pos + i) % self.win];
        }
        let n = self.win - self.tau_max - 1;
        // difference function
        self.diff[0] = 1.0;
        let mut running = 0.0;
        let mut found = None;
        for tau in 1..=self.tau_max {
            let mut d = 0.0;
            for j in 0..n {
                let e = self.frame[j] - self.frame[j + tau];
                d += e * e;
            }
            running += d;
            // cumulative mean normalised difference
            self.diff[tau] = if running > 0.0 { d * tau as f64 / running } else { 1.0 };
        }
        // 1) following a note: look for the best dip near the last period.
        //    A bend or vibrato moves the period smoothly, so this keeps
        //    locked on even when the string is quiet and the dip is shallow.
        if let Some(prev) = self.track {
            let lo = ((prev / 1.3).floor() as usize).max(self.tau_min);
            let hi = ((prev * 1.3).ceil() as usize).min(self.tau_max - 1);
            let mut best = (f64::MAX, 0);
            for tau in lo..=hi {
                if self.diff[tau] < best.0 {
                    best = (self.diff[tau], tau);
                }
            }
            if best.0 < 0.45 && best.1 > lo && best.1 < hi {
                found = Some(best.1);
            }
        }
        // 2) otherwise (new note, or a jump to a different fret): standard
        //    YIN, first dip below threshold, walked down to its minimum
        if found.is_none() {
            let thr = 0.15;
            let mut tau = self.tau_min;
            while tau < self.tau_max {
                if self.diff[tau] < thr {
                    while tau + 1 < self.tau_max && self.diff[tau + 1] < self.diff[tau] {
                        tau += 1;
                    }
                    found = Some(tau);
                    break;
                }
                tau += 1;
            }
        }
        if let Some(t) = found {
            // parabolic interpolation for sub-sample accuracy
            let (a, b, c) = (self.diff[t - 1], self.diff[t], self.diff[t + 1]);
            let den = a - 2.0 * b + c;
            let shift = if den.abs() > 1e-12 { 0.5 * (a - c) / den } else { 0.0 };
            let period = t as f64 + shift.clamp(-1.0, 1.0);
            self.freq = Some(self.fs_dec / period);
            self.track = Some(period);
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn tracks_guitar_range() {
        let fs = 48000.0;
        for f0 in [82.41, 110.0, 196.0, 329.63, 659.26] {
            let mut p = Pitch::new(fs);
            for k in 0..(fs * 0.3) as usize {
                let t = k as f64 / fs;
                let x = (2.0 * PI * f0 * t).sin() + 0.5 * (4.0 * PI * f0 * t).sin() + 0.3 * (6.0 * PI * f0 * t).sin();
                p.push(0.3 * x);
            }
            let f = p.freq.expect("pitch");
            assert!((f / f0 - 1.0).abs() < 0.02, "{f0} -> {f}");
        }
    }
}
