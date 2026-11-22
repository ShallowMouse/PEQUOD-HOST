//! Room acoustics from geometry: image-source early reflections for a
//! rectangular room (each wall with its own frequency-dependent absorption,
//! solved so the room hits its measured reverberation times through the
//! Eyring-Millington formula), then a diffuse tail from a 16-line feedback
//! delay network decaying at the same per-band times, at the level diffuse-
//! field theory gives for the microphone's distance and polar pattern.

use std::f64::consts::PI;
use vintage::dsp::Biquad;

pub const C: f64 = 343.0;
/// Air absorption at 4 kHz, 20 C, ~50% RH: energy attenuation per metre.
const AIR_M_4K: f64 = 0.0069;
/// ...as dB of level per metre.
const AIR_DB_4K: f64 = 0.03;

/// Wall order: x = 0 (behind the source), x = L (far end), y = 0, y = W,
/// z = 0 (floor), z = H (ceiling).
#[derive(Clone, Copy, Debug)]
pub struct RoomSpec {
    pub dims: [f64; 3],
    /// Reverberation time (s) at low (~125-250 Hz), mid (500 Hz-1 kHz) and
    /// high (~4 kHz) frequencies, screens folded away / screens out.
    pub rt_live: [f64; 3],
    pub rt_damped: [f64; 3],
    /// Relative absorption of each wall (1 = average for the room).
    pub bias: [f64; 6],
    /// Source position (m).
    pub src: [f64; 3],
    pub mic_height: f64,
    /// Open air: walls given directly as reflection factors (low, mid, high)
    /// and only a faint distant tail.
    pub open_air: Option<OpenAir>,
}

#[derive(Clone, Copy, Debug)]
pub struct OpenAir {
    pub beta: [[f64; 3]; 6],
    /// Level of the far-off city reverberation relative to the direct sound
    /// at 1 m (energy ratio) and its decay time.
    pub tail_energy: f64,
    pub tail_rt: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pattern {
    Omni,
    Cardioid,
    Figure8,
}

impl Pattern {
    #[inline]
    fn gain(self, cos: f64) -> f64 {
        match self {
            Pattern::Omni => 1.0,
            Pattern::Cardioid => 0.5 * (1.0 + cos),
            Pattern::Figure8 => cos,
        }
    }
    /// Random-energy efficiency: share of a diffuse field picked up.
    fn ree(self) -> f64 {
        match self {
            Pattern::Omni => 1.0,
            _ => 1.0 / 3.0,
        }
    }
}

/// Settings that shape the room as heard by the microphone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub distance: f64,
    pub spacing: f64,
    pub pattern: Pattern,
    /// 0 = screens folded back (live), 1 = screens out (damped).
    pub screens: f64,
}

/// Per-band numbers derived from a spec and placement.
#[derive(Clone, Debug)]
pub struct Acoustics {
    pub rt: [f64; 3],
    pub beta: [[f64; 3]; 6],
    pub volume: f64,
    pub surface: f64,
    pub mean_free_path: f64,
}

pub fn acoustics(spec: &RoomSpec, screens: f64) -> Acoustics {
    let [l, w, h] = spec.dims;
    let areas = [w * h, w * h, l * h, l * h, l * w, l * w];
    let s: f64 = areas.iter().sum();
    let v = l * w * h;
    let mut rt = [0.0; 3];
    for b in 0..3 {
        rt[b] = spec.rt_live[b] + (spec.rt_damped[b] - spec.rt_live[b]) * screens.clamp(0.0, 1.0);
    }
    let mut beta = [[0.0; 3]; 6];
    match spec.open_air {
        Some(o) => beta = o.beta,
        None => {
            let weighted: f64 = areas.iter().zip(&spec.bias).map(|(a, b)| a * b).sum();
            for b in 0..3 {
                // Eyring-Millington: RT = 0.161 V / (-sum S_i ln(1-a_i) + 4mV)
                let air = if b == 2 { 4.0 * AIR_M_4K * v } else { 0.0 };
                let total = (0.161 * v / rt[b] - air).max(0.02 * s);
                // ln(1 - a_i) = bias_i * k, with sum S_i bias_i k = -total
                let k = -total / weighted;
                for (i, bw) in spec.bias.iter().enumerate() {
                    beta[i][b] = (0.5 * bw * k).exp();
                }
            }
        }
    }
    Acoustics { rt, beta, volume: v, surface: s, mean_free_path: 4.0 * v / s }
}

// ------------------------------------------------------- early reflections --

pub const MAX_ORDER: usize = 3;
/// Image sources later than this are left to the diffuse tail.
pub const ER_WINDOW: f64 = 0.15;
const MAX_TAPS: usize = 400;

#[derive(Clone, Copy, Debug, Default)]
pub struct Tap {
    pub delay: f64,
    pub gain: f64,
    pub order: usize,
}

/// All image sources (up to `MAX_ORDER` reflections, within `ER_WINDOW`)
/// seen by a microphone at `mic`, aimed at the real source.
pub fn image_taps(spec: &RoomSpec, ac: &Acoustics, mic: [f64; 3], pattern: Pattern, fs: f64, out: &mut Vec<Tap>, order_shelf: &mut [[f64; 2]; MAX_ORDER + 1]) {
    out.clear();
    let s = spec.src;
    let dims = spec.dims;
    let aim = {
        let d = [s[0] - mic[0], s[1] - mic[1], s[2] - mic[2]];
        let n = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
        [d[0] / n, d[1] / n, d[2] / n]
    };
    // per-order band ratios accumulate here (energy-weighted), see below
    let mut acc = [[0.0f64; 3]; MAX_ORDER + 1];
    let mut wsum = [0.0f64; MAX_ORDER + 1];
    let r = MAX_ORDER as i32;
    let open_z = spec.open_air.is_some();
    for rx in -r..=r {
        for px in 0..2 {
            let nx0 = (rx - px).unsigned_abs() as usize; // hits on x = 0
            let nx1 = rx.unsigned_abs() as usize; // hits on x = L
            if nx0 + nx1 > MAX_ORDER {
                continue;
            }
            let ix = (1 - 2 * px) as f64 * s[0] + 2.0 * rx as f64 * dims[0];
            for ry in -r..=r {
                for py in 0..2 {
                    let ny0 = (ry - py).unsigned_abs() as usize;
                    let ny1 = ry.unsigned_abs() as usize;
                    if nx0 + nx1 + ny0 + ny1 > MAX_ORDER {
                        continue;
                    }
                    let iy = (1 - 2 * py) as f64 * s[1] + 2.0 * ry as f64 * dims[1];
                    for rz in -r..=r {
                        for pz in 0..2 {
                            let nz0 = (rz - pz).unsigned_abs() as usize;
                            let nz1 = rz.unsigned_abs() as usize;
                            let order = nx0 + nx1 + ny0 + ny1 + nz0 + nz1;
                            if order > MAX_ORDER || (open_z && nz1 > 0) {
                                continue;
                            }
                            let iz = (1 - 2 * pz) as f64 * s[2] + 2.0 * rz as f64 * dims[2];
                            let d = [ix - mic[0], iy - mic[1], iz - mic[2]];
                            let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(0.05);
                            let delay = dist / C;
                            if delay > ER_WINDOW {
                                continue;
                            }
                            let hits = [nx0, nx1, ny0, ny1, nz0, nz1];
                            let mut band = [1.0f64; 3];
                            for (wall, &n) in hits.iter().enumerate() {
                                for (b, bv) in band.iter_mut().enumerate() {
                                    *bv *= ac.beta[wall][b].powi(n as i32);
                                }
                            }
                            if band[1] < 1e-4 {
                                continue;
                            }
                            // arrival direction: from the mic toward the image
                            let cos = (d[0] * aim[0] + d[1] * aim[1] + d[2] * aim[2]) / dist;
                            let g = band[1] * pattern.gain(cos) / dist;
                            if g.abs() < 1e-5 || out.len() >= MAX_TAPS {
                                continue;
                            }
                            out.push(Tap { delay: delay * fs, gain: g, order });
                            let e = g * g;
                            acc[order][0] += e * (band[0] / band[1]).ln();
                            acc[order][2] += e * (band[2] / band[1]).ln() - e * AIR_DB_4K * dist / 8.686;
                            wsum[order] += e;
                        }
                    }
                }
            }
        }
    }
    // shelf gains (dB) per order: energy-weighted mean band ratio
    for o in 0..=MAX_ORDER {
        let ws = wsum[o].max(1e-30);
        order_shelf[o] = [8.686 * acc[o][0] / ws, 8.686 * acc[o][2] / ws];
    }
}

// --------------------------------------------------------------- the tail --

const LINES: usize = 16;

#[derive(Clone)]
struct Line {
    buf: Vec<f64>,
    pos: usize,
    len: usize,
    lo: Biquad,
    hi: Biquad,
    g: f64,
}

#[derive(Clone)]
struct Allpass {
    buf: Vec<f64>,
    pos: usize,
    len: usize,
    g: f64,
}

impl Allpass {
    fn new(max: usize) -> Self {
        Self { buf: vec![0.0; max.max(2)], pos: 0, len: 1, g: 0.6 }
    }
    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let d = self.buf[self.pos];
        let v = x + self.g * d;
        self.buf[self.pos] = v;
        self.pos += 1;
        if self.pos >= self.len {
            self.pos = 0;
        }
        d - self.g * v
    }
    fn clear(&mut self) {
        self.buf.iter_mut().for_each(|v| *v = 0.0);
    }
}

/// Diffuse late reverberation.
pub struct Tail {
    fs: f64,
    lines: Vec<Line>,
    diff: Vec<Allpass>,
    pre: Vec<f64>,
    pre_pos: usize,
    pre_len: usize,
    out_gain: f64,
    signs: [[f64; LINES]; 3],
}

impl Tail {
    pub fn new(fs: f64) -> Self {
        let max_line = (0.16 * fs) as usize + 4;
        let lines = (0..LINES)
            .map(|_| Line { buf: vec![0.0; max_line], pos: 0, len: 1, lo: Biquad::identity(), hi: Biquad::identity(), g: 0.0 })
            .collect();
        let diff = (0..4).map(|_| Allpass::new((0.03 * fs) as usize + 2)).collect();
        // fixed pseudo-random sign patterns: input, left out, right out
        let mut signs = [[1.0; LINES]; 3];
        let mut seed = 0x2545F491u32;
        for row in signs.iter_mut() {
            for v in row.iter_mut() {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                *v = if seed & 1 == 0 { 1.0 } else { -1.0 };
            }
        }
        Self { fs, lines, diff, pre: vec![0.0; (ER_WINDOW * fs) as usize + 8], pre_pos: 0, pre_len: 1, out_gain: 0.0, signs }
    }

    pub fn clear(&mut self) {
        for l in &mut self.lines {
            l.buf.iter_mut().for_each(|v| *v = 0.0);
            l.lo.reset();
            l.hi.reset();
        }
        self.diff.iter_mut().for_each(|d| d.clear());
        self.pre.iter_mut().for_each(|v| *v = 0.0);
    }

    /// Set the decay, the start time (s) and the energy the tail should
    /// carry (relative to a unit impulse).
    pub fn configure(&mut self, mfp: f64, rt: [f64; 3], start: f64, energy: f64) {
        let fs = self.fs;
        // line lengths spread around the mean free path time
        let base = (mfp / C).clamp(0.006, 0.09);
        let max = self.lines[0].buf.len() - 2;
        let mut gsum = 0.0;
        for (i, l) in self.lines.iter_mut().enumerate() {
            let spread = 0.62 * (1.65f64 / 0.62).powf(i as f64 / (LINES - 1) as f64);
            let mut len = (base * spread * fs) as usize | 1;
            // keep lengths mutually distinct-ish (offset by small primes)
            len += [0, 2, 6, 10, 12, 16, 18, 22, 28, 30, 36, 40, 42, 46, 52, 58][i];
            l.len = len.clamp(16, max);
            l.pos %= l.len;
            let d = l.len as f64 / fs;
            let gdb = |t: f64| -60.0 * d / t.max(0.05);
            l.g = 10f64.powf(gdb(rt[1]) / 20.0);
            l.lo.set_low_shelf(fs, 400.0, 0.6, gdb(rt[0]) - gdb(rt[1]));
            l.hi.set_high_shelf(fs, 2200.0, 0.6, gdb(rt[2]) - gdb(rt[1]));
            gsum += l.g * l.g;
        }
        let g2 = (gsum / LINES as f64).min(0.9999);
        // energy out of the network for a unit impulse in (orthogonal mixing)
        let e_net = g2 / (1.0 - g2) / LINES as f64;
        self.out_gain = (energy / e_net.max(1e-12)).sqrt();
        let scale = (mfp / 10.0).clamp(0.25, 1.6);
        for (k, (ap, t)) in self.diff.iter_mut().zip([0.0047, 0.0036, 0.0127, 0.0093]).enumerate() {
            ap.len = ((t * scale * fs) as usize).clamp(3, ap.buf.len() - 1);
            ap.pos %= ap.len;
            ap.g = if k < 2 { 0.65 } else { 0.55 };
        }
        // the first recirculation reaches the output after the shortest line
        let first = self.lines.iter().map(|l| l.len).min().unwrap_or(1) as f64;
        self.pre_len = ((start * fs - first).max(1.0) as usize).min(self.pre.len() - 1);
        self.pre_pos %= self.pre_len;
    }

    /// One mono sample in, (left, right) out.
    #[inline]
    pub fn process(&mut self, x: f64) -> (f64, f64) {
        let d = self.pre[self.pre_pos];
        self.pre[self.pre_pos] = x;
        self.pre_pos += 1;
        if self.pre_pos >= self.pre_len {
            self.pre_pos = 0;
        }
        let mut u = d;
        for ap in &mut self.diff {
            u = ap.process(u);
        }
        let mut o = [0.0; LINES];
        for (i, l) in self.lines.iter_mut().enumerate() {
            let y = l.buf[l.pos];
            let y = l.hi.process(l.lo.process(y * l.g));
            o[i] = y;
        }
        let (mut yl, mut yr) = (0.0, 0.0);
        for i in 0..LINES {
            yl += self.signs[1][i] * o[i];
            yr += self.signs[2][i] * o[i];
        }
        // Hadamard mixing (fast Walsh-Hadamard, orthonormal)
        let mut h = 1;
        while h < LINES {
            let mut i = 0;
            while i < LINES {
                for j in i..i + h {
                    let (a, b) = (o[j], o[j + h]);
                    o[j] = a + b;
                    o[j + h] = a - b;
                }
                i += 2 * h;
            }
            h *= 2;
        }
        let norm = 1.0 / (LINES as f64).sqrt();
        let inj = u * norm;
        for (i, l) in self.lines.iter_mut().enumerate() {
            l.buf[l.pos] = o[i] * norm + self.signs[0][i] * inj;
            l.pos += 1;
            if l.pos >= l.len {
                l.pos = 0;
            }
        }
        let g = self.out_gain * norm;
        (yl * g, yr * g)
    }
}

/// Reverberant energy picked up by the mic, relative to the direct sound at
/// 1 m (diffuse-field theory: 16 pi / A), for the part arriving after `t0`.
pub fn tail_energy(ac: &Acoustics, pattern: Pattern, t0: f64) -> f64 {
    let a = 0.161 * ac.volume / ac.rt[1];
    16.0 * PI / a * pattern.ree() * (-13.82 * t0 / ac.rt[1]).exp()
}

/// Simple per-order shelving for early reflections (wall absorption and air).
#[derive(Clone)]
pub struct OrderFilter {
    lo: Biquad,
    hi: Biquad,
}

impl OrderFilter {
    pub fn new() -> Self {
        Self { lo: Biquad::identity(), hi: Biquad::identity() }
    }
    pub fn set(&mut self, fs: f64, lo_db: f64, hi_db: f64) {
        self.lo.set_low_shelf(fs, 250.0, 0.6, lo_db.clamp(-30.0, 12.0));
        self.hi.set_high_shelf(fs, 3000.0, 0.6, hi_db.clamp(-30.0, 12.0));
    }
    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let y = self.lo.process(x);
        self.hi.process(y)
    }
    pub fn reset(&mut self) {
        self.lo.reset();
        self.hi.reset();
    }
}

impl Default for OrderFilter {
    fn default() -> Self {
        Self::new()
    }
}
