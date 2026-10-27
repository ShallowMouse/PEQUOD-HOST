//! Pure DSP for Grit Comp — no plugin framework code here, so it can be
//! unit-tested on its own.
//!
//! Signal path (per channel, compressor is stereo-linked):
//!   in -> low cut -> edge boost -> [4x oversampled tube saturation] -> DC block
//!      -> edge cut -> opto-style compressor -> high cut -> output -> dry/wet mix

use std::f64::consts::PI;

// ---------------------------------------------------------------- biquad ---

#[derive(Clone, Copy, Default)]
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

    pub fn set_highpass(&mut self, fs: f64, f: f64, q: f64) {
        let w = 2.0 * PI * (f / fs).min(0.49);
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q);
        self.set((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha);
    }

    pub fn set_lowpass(&mut self, fs: f64, f: f64, q: f64) {
        let w = 2.0 * PI * (f / fs).min(0.49);
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q);
        self.set((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha);
    }

    pub fn set_peak(&mut self, fs: f64, f: f64, q: f64, gain_db: f64) {
        let a = 10f64.powf(gain_db / 40.0);
        let w = 2.0 * PI * (f / fs).min(0.49);
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q);
        self.set(
            1.0 + alpha * a,
            -2.0 * c,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * c,
            1.0 - alpha / a,
        );
    }
}

// ------------------------------------------------------- 4x oversampling ---

pub const OS: usize = 4;
const TAPS: usize = 97; // linear phase, delay 48 hi-rate samples per filter
const PHASE_LEN: usize = (TAPS + OS - 1) / OS; // 25
/// Total latency of up+down filtering, in base-rate samples.
pub const LATENCY: usize = (TAPS - 1) / OS; // 24

fn design_fir() -> [f64; TAPS] {
    // Kaiser-windowed sinc, cutoff just under the base-rate Nyquist.
    let fc = 0.112; // cycles per hi-rate sample (base Nyquist = 0.125)
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
        let sinc = if t == 0.0 {
            2.0 * fc
        } else {
            (2.0 * PI * fc * t).sin() / (PI * t)
        };
        let r = 2.0 * n as f64 / m - 1.0;
        let w = i0(beta * (1.0 - r * r).max(0.0).sqrt()) / i0(beta);
        *tap = sinc * w;
        total += *tap;
    }
    for tap in h.iter_mut() {
        *tap /= total; // unity DC gain
    }
    h
}

#[derive(Clone)]
pub struct Oversampler {
    h: [f64; TAPS],
    up_hist: [f64; PHASE_LEN], // newest first
    down_hist: [f64; TAPS],    // newest first
}

impl Oversampler {
    pub fn new() -> Self {
        Self {
            h: design_fir(),
            up_hist: [0.0; PHASE_LEN],
            down_hist: [0.0; TAPS],
        }
    }

    pub fn reset(&mut self) {
        self.up_hist = [0.0; PHASE_LEN];
        self.down_hist = [0.0; TAPS];
    }

    /// Upsample one sample, run `f` on each of the 4 hi-rate samples,
    /// then filter and decimate back to one sample.
    #[inline]
    pub fn process(&mut self, x: f64, mut f: impl FnMut(f64) -> f64) -> f64 {
        self.up_hist.copy_within(0..PHASE_LEN - 1, 1);
        self.up_hist[0] = x;
        for p in 0..OS {
            // polyphase interpolation (zero-stuffed signal * OS gain)
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

// ------------------------------------------------------------ saturation ---

/// Asymmetric tanh: unity small-signal gain, zero output for zero input.
/// `bias` > 0 adds even (2nd) harmonics like a single-ended tube stage.
#[inline]
pub fn tube(x: f64, bias: f64) -> f64 {
    let tb = bias.tanh();
    ((x + bias).tanh() - tb) / (1.0 - tb * tb)
}

// ------------------------------------------------------------ compressor ---

#[derive(Clone)]
pub struct Compressor {
    fs: f64,
    fast: f64, // gain reduction in dB (<= 0)
    slow: f64,
    fast_att: f64,
    fast_rel: f64,
    slow_att: f64,
    slow_rel: f64,
    ratio: f64,
    knee: f64,
    opto: f64,
    times: (f64, f64),
    pub last_gr_db: f64,
}

fn coeff(fs: f64, ms: f64) -> f64 {
    (-1.0 / (fs * ms * 0.001)).exp()
}

impl Compressor {
    pub fn new(fs: f64) -> Self {
        let mut c = Self {
            fs,
            fast: 0.0,
            slow: 0.0,
            fast_att: 0.0,
            fast_rel: 0.0,
            slow_att: 0.0,
            slow_rel: 0.0,
            ratio: 4.0,
            knee: 6.0,
            opto: 0.4,
            times: (0.0, 0.0),
            last_gr_db: 0.0,
        };
        c.set_times(8.0, 80.0);
        c
    }

    /// Attack and release of the fast detector (ms). The slow, opto-like
    /// detector follows at 7.5x the attack and 15x the release.
    pub fn set_times(&mut self, attack_ms: f64, release_ms: f64) {
        if self.times == (attack_ms, release_ms) {
            return;
        }
        self.times = (attack_ms, release_ms);
        let fs = self.fs;
        self.fast_att = coeff(fs, attack_ms);
        self.fast_rel = coeff(fs, release_ms);
        self.slow_att = coeff(fs, attack_ms * 7.5);
        self.slow_rel = coeff(fs, release_ms * 15.0);
    }

    pub fn set_shape(&mut self, ratio: f64, knee_db: f64, opto: f64) {
        self.ratio = ratio.max(1.0);
        self.knee = knee_db.max(0.0);
        self.opto = opto.clamp(0.0, 1.0);
    }

    pub fn reset(&mut self) {
        self.fast = 0.0;
        self.slow = 0.0;
        self.last_gr_db = 0.0;
    }

    /// Static curve with a soft knee. Returns gain change in dB (<= 0).
    #[inline]
    fn curve(&self, level_db: f64, thr_db: f64) -> f64 {
        let over = level_db - thr_db;
        let slope = 1.0 / self.ratio - 1.0;
        let knee = self.knee;
        if knee <= 1e-6 {
            if over > 0.0 { slope * over } else { 0.0 }
        } else if 2.0 * over < -knee {
            0.0
        } else if 2.0 * over.abs() <= knee {
            slope * (over + knee / 2.0).powi(2) / (2.0 * knee)
        } else {
            slope * over
        }
    }

    /// Returns a linear gain to apply, given the linked detector level.
    /// `makeup_db` is added on top; `auto` adds the half-strength automatic
    /// make-up for the threshold and ratio.
    #[inline]
    pub fn process(&mut self, detector: f64, thr_db: f64, makeup_db: f64, auto: bool) -> f64 {
        let level_db = 20.0 * detector.abs().max(1e-9).log10();
        let target = self.curve(level_db, thr_db);
        let step = |env: &mut f64, att: f64, rel: f64| {
            let c = if target < *env { att } else { rel };
            *env = target + c * (*env - target);
        };
        step(&mut self.fast, self.fast_att, self.fast_rel);
        step(&mut self.slow, self.slow_att, self.slow_rel);
        // quick grab, then a slower program-dependent recovery (opto blend)
        let gr = (1.0 - self.opto) * self.fast + self.opto * self.slow;
        self.last_gr_db = gr;
        let auto_db = if auto { -thr_db * (1.0 - 1.0 / self.ratio) * 0.5 } else { 0.0 };
        db_to_gain(gr + auto_db + makeup_db)
    }
}

#[inline]
pub fn db_to_gain(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

// ------------------------------------------------------------- settings ---

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub grit_db: f64,     // drive into the saturator, 0..30
    pub edge_db: f64,     // pre-emphasis amount, 0..12
    pub edge_hz: f64,     // pre-emphasis centre, 800..5000
    pub bias: f64,        // asymmetry 0..1 (scaled internally)
    pub compress: f64,    // 0..1
    pub low_cut_hz: f64,  // 20..400
    pub high_cut_hz: f64, // 2000..20000
    pub mix: f64,         // 0..1
    pub output_db: f64,   // -24..12
    /// false = breakup follows input level (tube before compressor);
    /// true  = breakup follows gain reduction (compressor first, the harder
    ///         it squashes the more the tube is driven).
    pub gr_mode: bool,
    /// GR mode: gain reduction (dB) at which breakup starts, 0.5..12.
    pub gr_onset_db: f64,
    pub ratio: f64,
    pub knee_db: f64,
    pub attack_ms: f64,
    pub release_ms: f64,
    /// Share of the slow, opto-like recovery in the gain reduction, 0..1.
    pub opto: f64,
    /// High-pass on the compressor's detector only (20 = off).
    pub sc_hpf_hz: f64,
    pub makeup_db: f64,
    pub auto_makeup: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            grit_db: 14.0,
            edge_db: 6.0,
            edge_hz: 2000.0,
            bias: 0.3,
            compress: 0.45,
            low_cut_hz: 110.0,
            high_cut_hz: 9000.0,
            mix: 1.0,
            output_db: 0.0,
            gr_mode: false,
            gr_onset_db: 2.0,
            ratio: 4.0,
            knee_db: 6.0,
            attack_ms: 8.0,
            release_ms: 80.0,
            opto: 0.4,
            sc_hpf_hz: 20.0,
            makeup_db: 0.0,
            auto_makeup: true,
        }
    }
}

/// Headroom of the tube stage in GR mode below the onset (clean).
pub const GR_HEADROOM_DB: f64 = 12.0;
/// GR mode: the tube fades in over this many dB just below the onset...
pub const GR_KNEE_DB: f64 = 2.0;
/// ...and reaches the full Grit drive this many dB above it.
pub const GR_SPAN_DB: f64 = 6.0;

/// GR mode drive curve: gain reduction (dB, >= 0) -> tube drive (dB).
/// Clean (-12 dB headroom) well below the onset, edge of breakup (0 dB) at
/// the onset, full Grit `GR_SPAN_DB` above it, then keeps rising gently.
pub fn gr_drive_db(gr_db: f64, onset_db: f64, grit_db: f64) -> f64 {
    let knee_start = onset_db - GR_KNEE_DB;
    if gr_db <= knee_start {
        -GR_HEADROOM_DB
    } else if gr_db <= onset_db {
        -GR_HEADROOM_DB * (onset_db - gr_db) / GR_KNEE_DB
    } else if gr_db <= onset_db + GR_SPAN_DB {
        grit_db * (gr_db - onset_db) / GR_SPAN_DB
    } else {
        let extra = (gr_db - onset_db - GR_SPAN_DB) * grit_db / GR_SPAN_DB * 0.5;
        grit_db + extra.min(grit_db * 0.5)
    }
}

// ------------------------------------------------------------ processor ---

#[derive(Clone)]
struct Channel {
    low_cut: Biquad,
    edge_boost: Biquad,
    os: Oversampler,
    dc_x1: f64,
    dc_y1: f64,
    edge_cut: Biquad,
    high_cut: Biquad,
    sc_hpf: Biquad,
    sc_on: bool,
    dry: [f64; LATENCY + 1],
    dry_pos: usize,
}

impl Channel {
    fn new() -> Self {
        Self {
            low_cut: Biquad::default(),
            edge_boost: Biquad::default(),
            os: Oversampler::new(),
            dc_x1: 0.0,
            dc_y1: 0.0,
            edge_cut: Biquad::default(),
            high_cut: Biquad::default(),
            sc_hpf: Biquad::default(),
            sc_on: false,
            dry: [0.0; LATENCY + 1],
            dry_pos: 0,
        }
    }

    fn reset(&mut self) {
        self.low_cut.reset();
        self.edge_boost.reset();
        self.os.reset();
        self.dc_x1 = 0.0;
        self.dc_y1 = 0.0;
        self.edge_cut.reset();
        self.high_cut.reset();
        self.sc_hpf.reset();
        self.dry = [0.0; LATENCY + 1];
        self.dry_pos = 0;
    }

    /// Delay line for the dry signal so it lines up with the oversampled wet path.
    #[inline]
    fn delay_dry(&mut self, x: f64) -> f64 {
        self.dry[self.dry_pos] = x;
        self.dry_pos = (self.dry_pos + 1) % (LATENCY + 1);
        self.dry[self.dry_pos] // oldest sample = LATENCY samples ago
    }
}

pub struct Processor {
    fs: f64,
    chans: Vec<Channel>,
    comp: Compressor,
    settings: Option<Settings>,
    dc_r: f64,
    /// Peak level hitting the tube stage since last `take_sat_peak()` (1.0 = start of breakup).
    sat_peak: f64,
}

impl Processor {
    pub fn new(fs: f64, channels: usize) -> Self {
        Self {
            fs,
            chans: vec![Channel::new(); channels.max(1)],
            comp: Compressor::new(fs),
            settings: None,
            dc_r: (-2.0 * PI * 8.0 / fs).exp(),
            sat_peak: 0.0,
        }
    }

    pub fn reset(&mut self) {
        for c in &mut self.chans {
            c.reset();
        }
        self.comp.reset();
    }

    pub fn take_sat_peak(&mut self) -> f64 {
        std::mem::take(&mut self.sat_peak)
    }

    pub fn gain_reduction_db(&self) -> f64 {
        self.comp.last_gr_db
    }

    /// Update filter coefficients; cheap to call every block.
    pub fn update(&mut self, s: &Settings) {
        let changed = |a: Option<Settings>| match a {
            None => true,
            Some(o) => {
                o.low_cut_hz != s.low_cut_hz
                    || o.sc_hpf_hz != s.sc_hpf_hz
                    || o.high_cut_hz != s.high_cut_hz
                    || o.edge_db != s.edge_db
                    || o.edge_hz != s.edge_hz
            }
        };
        if changed(self.settings) {
            let fs = self.fs;
            for c in &mut self.chans {
                c.low_cut.set_highpass(fs, s.low_cut_hz, 0.707);
                c.high_cut.set_lowpass(fs, s.high_cut_hz, 0.707);
                c.edge_boost.set_peak(fs, s.edge_hz, 0.8, s.edge_db);
                c.edge_cut.set_peak(fs, s.edge_hz, 0.8, -s.edge_db);
                c.sc_on = s.sc_hpf_hz > 20.5;
                if c.sc_on {
                    c.sc_hpf.set_highpass(fs, s.sc_hpf_hz, 0.707);
                }
            }
        }
        self.comp.set_times(s.attack_ms, s.release_ms);
        self.comp.set_shape(s.ratio, s.knee_db, s.opto);
        self.settings = Some(*s);
    }

    /// Process one frame (one sample per channel) in place.
    /// `grit_db`, `mix` and `output_db` are passed per sample so the caller
    /// can smooth them.
    #[inline]
    pub fn process_frame(&mut self, frame: &mut [f32], s: &Settings) {
        let drive = db_to_gain(s.grit_db);
        let inv_drive = 1.0 / drive;
        let bias = s.bias * 0.6;
        let thr_db = -36.0 * s.compress;
        let out_gain = db_to_gain(s.output_db);
        let r = self.dc_r;

        let n = frame.len().min(self.chans.len());
        if s.gr_mode {
            self.process_frame_gr(frame, s, n);
            return;
        }
        let mut wet = [0.0f64; 8];
        let mut detector = 0.0f64;
        let mut sat_pk = 0.0f64;

        for i in 0..n {
            let c = &mut self.chans[i];
            let x = frame[i] as f64;
            let mut y = c.low_cut.process(x);
            y = c.edge_boost.process(y);
            y = c.os.process(y, |v| {
                let d = v * drive;
                sat_pk = sat_pk.max(d.abs());
                tube(d, bias) * inv_drive
            });
            // DC blocker (asymmetric saturation creates a DC offset)
            let dc = y - c.dc_x1 + r * c.dc_y1;
            c.dc_x1 = y;
            c.dc_y1 = dc;
            y = c.edge_cut.process(dc);
            wet[i.min(7)] = y;
            let d = if c.sc_on { c.sc_hpf.process(y) } else { y };
            detector = detector.max(d.abs());
        }

        self.sat_peak = self.sat_peak.max(sat_pk);

        let comp_gain = if s.compress > 0.0 {
            self.comp.process(detector, thr_db, s.makeup_db, s.auto_makeup)
        } else {
            self.comp.last_gr_db = 0.0;
            1.0
        };

        for i in 0..n {
            let c = &mut self.chans[i];
            let x = frame[i] as f64;
            let dry = c.delay_dry(x);
            let y = c.high_cut.process(wet[i.min(7)] * comp_gain) * out_gain;
            let out = dry + s.mix * (y - dry);
            frame[i] = if out.is_finite() { out as f32 } else { 0.0 };
        }
    }

    /// GR mode: compressor first, then the tube, driven in proportion to the
    /// current gain reduction. Quiet passages (no reduction) stay clean no
    /// matter how loud they are in absolute terms; the harder the compressor
    /// works, the dirtier it gets.
    #[inline]
    fn process_frame_gr(&mut self, frame: &mut [f32], s: &Settings, n: usize) {
        let bias = s.bias * 0.6;
        let thr_db = -36.0 * s.compress;
        let out_gain = db_to_gain(s.output_db);
        let r = self.dc_r;

        // 1) filters + detector on the pre-saturation signal
        let mut pre = [0.0f64; 8];
        let mut detector = 0.0f64;
        for i in 0..n {
            let c = &mut self.chans[i];
            let y = c.low_cut.process(frame[i] as f64);
            let d = if c.sc_on { c.sc_hpf.process(y) } else { y };
            detector = detector.max(d.abs());
            pre[i.min(7)] = c.edge_boost.process(y);
        }

        // 2) compressor -> gain and how hard it is working
        let comp_gain = if s.compress > 0.0 {
            self.comp.process(detector, thr_db, s.makeup_db, s.auto_makeup)
        } else {
            self.comp.last_gr_db = 0.0;
            1.0
        };
        let drive = db_to_gain(gr_drive_db(-self.comp.last_gr_db, s.gr_onset_db, s.grit_db));
        let inv_drive = 1.0 / drive;

        // 3) tube driven by the amount of gain reduction
        let mut sat_pk = 0.0f64;
        for i in 0..n {
            let c = &mut self.chans[i];
            let x = frame[i] as f64;
            let mut y = c.os.process(pre[i.min(7)] * comp_gain, |v| {
                let d = v * drive;
                sat_pk = sat_pk.max(d.abs());
                tube(d, bias) * inv_drive
            });
            let dc = y - c.dc_x1 + r * c.dc_y1;
            c.dc_x1 = y;
            c.dc_y1 = dc;
            y = c.edge_cut.process(dc);
            let dry = c.delay_dry(x);
            let y = c.high_cut.process(y) * out_gain;
            let out = dry + s.mix * (y - dry);
            frame[i] = if out.is_finite() { out as f32 } else { 0.0 };
        }
        self.sat_peak = self.sat_peak.max(sat_pk);
    }
}

// ----------------------------------------------------------------- tests ---

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f64 = 48000.0;

    /// Run a mono sine through and return (rms_out, thd) measured on the
    /// settled second half, using a simple DFT at harmonics.
    fn measure(amp: f64, freq: f64, s: Settings) -> (f64, f64) {
        let mut p = Processor::new(FS, 1);
        p.update(&s);
        let n = (FS * 1.0) as usize;
        let mut out = Vec::with_capacity(n);
        for k in 0..n {
            let x = amp * (2.0 * PI * freq * k as f64 / FS).sin();
            let mut f = [x as f32];
            p.process_frame(&mut f, &s);
            out.push(f[0] as f64);
        }
        let tail = &out[n / 2..];
        let rms = (tail.iter().map(|v| v * v).sum::<f64>() / tail.len() as f64).sqrt();
        let mag = |h: f64| {
            let (mut re, mut im) = (0.0, 0.0);
            for (k, v) in tail.iter().enumerate() {
                let ph = 2.0 * PI * freq * h * k as f64 / FS;
                re += v * ph.cos();
                im += v * ph.sin();
            }
            (re * re + im * im).sqrt()
        };
        let fund = mag(1.0);
        let harm: f64 = (2..=6).map(|h| mag(h as f64).powi(2)).sum::<f64>().sqrt();
        (rms, harm / fund)
    }

    fn no_comp() -> Settings {
        Settings {
            compress: 0.0,
            ..Settings::default()
        }
    }

    #[test]
    fn quiet_is_clean_loud_is_gritty() {
        let (_, thd_quiet) = measure(db_to_gain(-36.0), 700.0, no_comp());
        let (_, thd_loud) = measure(db_to_gain(-6.0), 700.0, no_comp());
        println!("THD quiet {:.3}%  loud {:.3}%", thd_quiet * 100.0, thd_loud * 100.0);
        assert!(thd_quiet < 0.01, "quiet signal should be nearly clean");
        assert!(thd_loud > 0.05, "loud signal should distort clearly");
    }

    #[test]
    fn more_grit_more_distortion() {
        let amp = db_to_gain(-18.0);
        let (_, low) = measure(amp, 700.0, Settings { grit_db: 4.0, ..no_comp() });
        let (_, high) = measure(amp, 700.0, Settings { grit_db: 24.0, ..no_comp() });
        println!("THD grit4 {:.3}%  grit24 {:.3}%", low * 100.0, high * 100.0);
        assert!(high > low * 3.0);
    }

    #[test]
    fn clean_level_roughly_unity() {
        let amp = db_to_gain(-36.0);
        let (rms, _) = measure(amp, 700.0, no_comp());
        let in_rms = amp / 2f64.sqrt();
        let diff_db = 20.0 * (rms / in_rms).log10();
        println!("clean gain {:.2} dB", diff_db);
        assert!(diff_db.abs() < 1.5);
    }

    #[test]
    fn compressor_evens_out_levels() {
        let s = Settings::default();
        let (q, _) = measure(db_to_gain(-30.0), 700.0, s);
        let (l, _) = measure(db_to_gain(-6.0), 700.0, s);
        let spread = 20.0 * (l / q).log10();
        println!("24 dB input spread -> {:.1} dB output spread", spread);
        assert!(spread < 16.0 && spread > 3.0);
    }

    #[test]
    fn dry_mix_aligned() {
        // With effect neutralised, 50% mix must not comb-filter.
        let s = Settings {
            grit_db: 0.0,
            edge_db: 0.0,
            edge_hz: 2000.0,
            bias: 0.0,
            compress: 0.0,
            low_cut_hz: 20.0,
            high_cut_hz: 20000.0,
            mix: 0.5,
            output_db: 0.0,
            gr_mode: false,
            gr_onset_db: 2.0,
            ..Settings::default()
        };
        for f in [1000.0, 5000.0, 9000.0] {
            let amp = db_to_gain(-40.0);
            let (rms, _) = measure(amp, f, s);
            let g = 20.0 * (rms / (amp / 2f64.sqrt())).log10();
            println!("{} Hz mix gain {:.2} dB", f, g);
            assert!(g.abs() < 1.0, "comb filtering at {f} Hz: {g} dB");
        }
    }

    #[test]
    fn survives_extremes() {
        let s = Settings {
            grit_db: 30.0,
            edge_db: 12.0,
            edge_hz: 3000.0,
            bias: 1.0,
            compress: 1.0,
            low_cut_hz: 400.0,
            high_cut_hz: 2000.0,
            mix: 1.0,
            output_db: 12.0,
            gr_mode: false,
            gr_onset_db: 2.0,
            ..Settings::default()
        };
        let mut p = Processor::new(44100.0, 2);
        p.update(&s);
        let mut seed = 1u32;
        for _ in 0..44100 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let v = (seed as f32 / u32::MAX as f32) * 4.0 - 2.0;
            let mut f = [v, -v];
            p.process_frame(&mut f, &s);
            assert!(f[0].is_finite() && f[1].is_finite());
            assert!(f[0].abs() < 20.0);
        }
    }

    #[test]
    fn gr_mode_breakup_follows_gain_reduction() {
        let gr = Settings { gr_mode: true, ..Settings::default() };
        let (_, thd_quiet) = measure(db_to_gain(-36.0), 700.0, gr);
        let (_, thd_loud) = measure(db_to_gain(-6.0), 700.0, gr);
        println!("GR mode THD quiet {:.3}%  loud {:.3}%", thd_quiet * 100.0, thd_loud * 100.0);
        assert!(thd_quiet < 0.01);
        assert!(thd_loud > 0.05);
        // no compression -> no gain reduction -> stays (nearly) clean even when loud
        let (_, thd_nocomp) = measure(db_to_gain(-6.0), 700.0, Settings { compress: 0.0, ..gr });
        println!("GR mode, compress off, loud: {:.3}%", thd_nocomp * 100.0);
        assert!(thd_nocomp < thd_loud * 0.3);
        // more compression -> more grit at the same input level
        let amp = db_to_gain(-14.0);
        let (_, light) = measure(amp, 700.0, Settings { compress: 0.3, ..gr });
        let (_, heavy) = measure(amp, 700.0, Settings { compress: 0.9, ..gr });
        println!("GR mode compress 30%: {:.3}%  90%: {:.3}%", light * 100.0, heavy * 100.0);
        assert!(heavy > light * 2.0);
    }

    #[test]
    fn gr_mode_survives_extremes() {
        let s = Settings { grit_db: 30.0, compress: 1.0, bias: 1.0, output_db: 12.0, gr_mode: true, ..Settings::default() };
        let mut p = Processor::new(44100.0, 2);
        p.update(&s);
        let mut seed = 7u32;
        for _ in 0..44100 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let v = (seed as f32 / u32::MAX as f32) * 4.0 - 2.0;
            let mut f = [v, -v];
            p.process_frame(&mut f, &s);
            assert!(f[0].is_finite() && f[1].is_finite() && f[0].abs() < 20.0);
        }
    }

    #[test]
    fn onset_controls_when_breakup_starts() {
        // same input and compression; a lower onset gives more grit
        let base = Settings { gr_mode: true, compress: 0.4, ..Settings::default() };
        let amp = db_to_gain(-10.0);
        let (_, early) = measure(amp, 700.0, Settings { gr_onset_db: 0.5, ..base });
        let (_, late) = measure(amp, 700.0, Settings { gr_onset_db: 10.0, ..base });
        println!("onset 0.5 dB: {:.3}%  onset 10 dB: {:.3}%", early * 100.0, late * 100.0);
        assert!(early > late * 3.0);
        assert!(late < 0.01);
    }

    #[test]
    fn drive_curve_is_continuous() {
        let mut prev = gr_drive_db(0.0, 3.0, 14.0);
        let mut g = 0.0;
        while g < 20.0 {
            g += 0.01;
            let d = gr_drive_db(g, 3.0, 14.0);
            assert!(d >= prev - 1e-9 && d - prev < 0.2, "jump at {g}");
            prev = d;
        }
    }

    #[test]
    fn defaults_match_the_original_compressor() {
        // the new controls at their defaults reproduce the old fixed design
        let (rms, _) = measure(db_to_gain(-6.0), 700.0, Settings::default());
        let old = Settings::default();
        let (rms2, _) = measure(db_to_gain(-6.0), 700.0, Settings { ratio: 4.0, knee_db: 6.0, attack_ms: 8.0, release_ms: 80.0, opto: 0.4, ..old });
        assert!((rms - rms2).abs() < 1e-12);
    }

    #[test]
    fn ratio_and_makeup_do_what_they_say() {
        let base = Settings { grit_db: 0.0, edge_db: 0.0, compress: 0.5, mix: 1.0, auto_makeup: false, ..Settings::default() };
        let loud = db_to_gain(-6.0);
        let (gentle, _) = measure(loud, 700.0, Settings { ratio: 1.5, ..base });
        let (hard, _) = measure(loud, 700.0, Settings { ratio: 20.0, ..base });
        println!("ratio 1.5: {:.1} dB, 20: {:.1} dB", 20.0 * gentle.log10(), 20.0 * hard.log10());
        assert!(hard < gentle * 0.6);
        let (plus, _) = measure(loud, 700.0, Settings { ratio: 20.0, makeup_db: 6.0, ..base });
        assert!(((plus / hard).log10() * 20.0 - 6.0).abs() < 0.3);
    }

    #[test]
    fn sidechain_filter_ignores_the_bass() {
        let base = Settings { grit_db: 0.0, edge_db: 0.0, compress: 0.6, low_cut_hz: 20.0, ..Settings::default() };
        let (open, _) = measure(db_to_gain(-6.0), 80.0, base);
        let (filtered, _) = measure(db_to_gain(-6.0), 80.0, Settings { sc_hpf_hz: 300.0, ..base });
        println!("80 Hz tone: SC off {:.1} dB, SC 300 Hz {:.1} dB", 20.0 * open.log10(), 20.0 * filtered.log10());
        assert!(filtered > open * 1.4);
    }
}
