//! Impulse-response cabinet loader: WAV in, resampled to the session rate,
//! run as a uniformly partitioned FFT convolution with a fixed latency of
//! one partition.

use nih_plug::prelude::AtomicF32;
use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

/// Partition size = fixed latency in samples.
pub const BLOCK: usize = 64;
/// Longest IR kept (seconds). Cab IRs are a few hundred ms at most; the
/// rest is room tail and would only cost CPU.
pub const MAX_SECONDS: f64 = 0.25;

pub struct Convolver {
    fft: Arc<dyn RealToComplex<f64>>,
    ifft: Arc<dyn ComplexToReal<f64>>,
    parts: Vec<Vec<Complex<f64>>>,
    fdl: Vec<Vec<Complex<f64>>>,
    fdl_pos: usize,
    time: Vec<f64>,
    input: Vec<f64>,
    acc: Vec<Complex<f64>>,
    out: Vec<f64>,
    scratch: Vec<Complex<f64>>,
    pos: usize,
    pub name: String,
    pub seconds: f64,
}

impl Convolver {
    pub fn new(ir: &[f64], name: String, fs: f64) -> Self {
        let n = 2 * BLOCK;
        let mut planner = RealFftPlanner::<f64>::new();
        let fft = planner.plan_fft_forward(n);
        let ifft = planner.plan_fft_inverse(n);
        let bins = n / 2 + 1;
        let count = ir.len().div_ceil(BLOCK).max(1);
        let mut parts = Vec::with_capacity(count);
        let mut buf = fft.make_input_vec();
        let mut spec = fft.make_output_vec();
        for p in 0..count {
            buf.iter_mut().for_each(|v| *v = 0.0);
            for k in 0..BLOCK {
                if let Some(v) = ir.get(p * BLOCK + k) {
                    // fold the 1/N of the inverse FFT into the filter
                    buf[k] = *v / n as f64;
                }
            }
            fft.process(&mut buf, &mut spec).ok();
            parts.push(spec.clone());
        }
        let scratch_len = fft.get_scratch_len().max(ifft.get_scratch_len());
        Self {
            fdl: vec![vec![Complex::new(0.0, 0.0); bins]; count],
            fdl_pos: 0,
            time: vec![0.0; n],
            input: vec![0.0; n],
            acc: vec![Complex::new(0.0, 0.0); bins],
            out: vec![0.0; BLOCK],
            scratch: vec![Complex::new(0.0, 0.0); scratch_len],
            pos: 0,
            parts,
            fft,
            ifft,
            name,
            seconds: ir.len() as f64 / fs,
        }
    }

    pub fn reset(&mut self) {
        for f in &mut self.fdl {
            f.iter_mut().for_each(|c| *c = Complex::new(0.0, 0.0));
        }
        self.input.iter_mut().for_each(|v| *v = 0.0);
        self.out.iter_mut().for_each(|v| *v = 0.0);
        self.pos = 0;
    }

    /// One sample in, one out, `BLOCK` samples late.
    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let y = self.out[self.pos];
        self.input[BLOCK + self.pos] = x;
        self.pos += 1;
        if self.pos == BLOCK {
            self.pos = 0;
            self.block();
        }
        y
    }

    fn block(&mut self) {
        let count = self.parts.len();
        self.time.copy_from_slice(&self.input);
        self.fdl_pos = (self.fdl_pos + count - 1) % count;
        let slot = &mut self.fdl[self.fdl_pos];
        self.fft.process_with_scratch(&mut self.time, slot, &mut self.scratch).ok();
        self.acc.iter_mut().for_each(|c| *c = Complex::new(0.0, 0.0));
        for (p, h) in self.parts.iter().enumerate() {
            let x = &self.fdl[(self.fdl_pos + p) % count];
            for ((a, xv), hv) in self.acc.iter_mut().zip(x).zip(h) {
                *a += xv * hv;
            }
        }
        // real-signal spectra: DC and Nyquist bins must be purely real
        self.acc[0].im = 0.0;
        let last = self.acc.len() - 1;
        self.acc[last].im = 0.0;
        self.ifft.process_with_scratch(&mut self.acc, &mut self.time, &mut self.scratch).ok();
        // overlap-save: keep the second half
        self.out.copy_from_slice(&self.time[BLOCK..]);
        self.input.copy_within(BLOCK.., 0);
    }
}

/// Read a WAV file, mix to mono, resample to `fs`, trim and normalise.
pub fn load(path: &Path, fs: f64) -> Result<Convolver, String> {
    let mut r = hound::WavReader::open(path).map_err(|e| format!("can't open: {e}"))?;
    let spec = r.spec();
    let ch = spec.channels.max(1) as usize;
    let raw: Vec<f64> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().map(|s| s.map(|v| v as f64)).collect::<Result<_, _>>(),
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample.clamp(8, 32) - 1)) as f64;
            r.samples::<i32>().map(|s| s.map(|v| v as f64 * scale)).collect::<Result<_, _>>()
        }
    }
    .map_err(|e| format!("bad WAV data: {e}"))?;
    if raw.is_empty() {
        return Err("empty file".into());
    }
    let mono: Vec<f64> = raw.chunks(ch).map(|c| c.iter().sum::<f64>() / ch as f64).collect();
    let src_fs = spec.sample_rate as f64;
    let mut ir = resample(&mono, src_fs, fs);
    ir.truncate((MAX_SECONDS * fs) as usize);
    // short fade-out where we cut
    let fade = (0.005 * fs) as usize;
    let len = ir.len();
    if len > fade * 2 {
        for k in 0..fade {
            ir[len - 1 - k] *= k as f64 / fade as f64;
        }
    }
    // level-match: average response over the guitar's core range
    // (150 Hz - 4 kHz) set to +1 dB, about where the built-in cabs sit
    let energy: f64 = ir.iter().map(|v| v * v).sum();
    if energy <= 1e-20 {
        return Err("silent file".into());
    }
    let mut db_sum = 0.0;
    let points = 24;
    for p in 0..points {
        let f = 150.0 * (4000.0f64 / 150.0).powf(p as f64 / (points - 1) as f64);
        let w = 2.0 * std::f64::consts::PI * f / fs;
        let (mut re, mut im) = (0.0, 0.0);
        for (k, v) in ir.iter().enumerate() {
            let ph = w * k as f64;
            re += v * ph.cos();
            im -= v * ph.sin();
        }
        db_sum += 10.0 * (re * re + im * im).max(1e-20).log10();
    }
    let avg_db = db_sum / points as f64;
    let g = 10f64.powf((1.0 - avg_db) / 20.0);
    ir.iter_mut().for_each(|v| *v *= g);
    let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    Ok(Convolver::new(&ir, name, fs))
}

/// Windowed-sinc resampling (offline, so quality over speed).
fn resample(x: &[f64], from: f64, to: f64) -> Vec<f64> {
    if (from - to).abs() < 0.5 {
        return x.to_vec();
    }
    let ratio = to / from;
    let cutoff = ratio.min(1.0) * 0.95;
    let half = 32.0 / cutoff.min(1.0);
    let n_out = ((x.len() as f64) * ratio).ceil() as usize;
    let mut y = vec![0.0; n_out];
    for (j, out) in y.iter_mut().enumerate() {
        let t = j as f64 / ratio;
        let lo = (t - half).floor().max(0.0) as usize;
        let hi = ((t + half).ceil() as usize).min(x.len().saturating_sub(1));
        let mut acc = 0.0;
        for (i, xi) in x.iter().enumerate().take(hi + 1).skip(lo) {
            let d = t - i as f64;
            let arg = std::f64::consts::PI * d * cutoff;
            let sinc = if arg.abs() < 1e-9 { 1.0 } else { arg.sin() / arg };
            let w = 0.5 + 0.5 * (std::f64::consts::PI * d / half).cos();
            if d.abs() < half {
                acc += xi * sinc * w * cutoff;
            }
        }
        *out = acc;
    }
    y
}

/// Shared between the editor (which loads files) and the audio thread
/// (which swaps them in without blocking).
#[derive(Default)]
pub struct IrShared {
    pub pending: Mutex<Option<Box<Convolver>>>,
    /// Old convolvers handed back for the editor thread to free.
    pub trash: Mutex<Vec<Box<Convolver>>>,
    pub clear: AtomicBool,
    pub sample_rate: AtomicF32,
    /// What the panel shows: file name, or an error.
    pub status: RwLock<String>,
    pub loaded: AtomicBool,
}

impl IrShared {
    /// Load `path` and queue it for the audio thread. Updates `status`.
    pub fn load_file(&self, path: &Path) -> bool {
        let fs = self.sample_rate.load(Ordering::Relaxed) as f64;
        let fs = if fs > 1000.0 { fs } else { 48000.0 };
        match load(path, fs) {
            Ok(c) => {
                if let Ok(mut s) = self.status.write() {
                    *s = format!("{} ({:.0} ms)", c.name, c.seconds * 1000.0);
                }
                if let Ok(mut p) = self.pending.lock() {
                    *p = Some(Box::new(c));
                }
                self.clear.store(false, Ordering::Relaxed);
                true
            }
            Err(e) => {
                if let Ok(mut s) = self.status.write() {
                    *s = format!("IR not loaded: {e}");
                }
                false
            }
        }
    }

    pub fn unload(&self) {
        self.clear.store(true, Ordering::Relaxed);
        if let Ok(mut s) = self.status.write() {
            s.clear();
        }
    }

    /// Editor thread: free convolvers the audio thread has retired.
    pub fn collect_garbage(&self) {
        if let Ok(mut t) = self.trash.try_lock() {
            t.clear();
        }
    }
}

/// Audio-thread side: owns the active convolver.
#[derive(Default)]
pub struct IrSlot {
    pub active: Option<Box<Convolver>>,
}

impl IrSlot {
    /// Pick up a newly loaded IR or an unload request. Never blocks.
    pub fn poll(&mut self, shared: &IrShared) {
        if shared.clear.swap(false, Ordering::Relaxed) {
            if let Some(old) = self.active.take() {
                if let Ok(mut t) = shared.trash.try_lock() {
                    t.push(old);
                }
            }
            shared.loaded.store(false, Ordering::Relaxed);
        }
        if let Ok(mut p) = shared.pending.try_lock() {
            if let Some(new) = p.take() {
                if let Some(old) = self.active.replace(new) {
                    if let Ok(mut t) = shared.trash.try_lock() {
                        t.push(old);
                    }
                }
                shared.loaded.store(true, Ordering::Relaxed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convolution_matches_direct() {
        let fs = 48000.0;
        let ir: Vec<f64> = (0..1000).map(|k| ((k as f64) * 0.37).sin() * (-(k as f64) / 200.0).exp()).collect();
        let mut c = Convolver::new(&ir, "t".into(), fs);
        let x: Vec<f64> = (0..3000).map(|k| ((k as f64) * 0.11).sin() + if k == 7 { 1.0 } else { 0.0 }).collect();
        let y: Vec<f64> = x.iter().map(|&v| c.process(v)).collect();
        for n in 0..(3000 - BLOCK) {
            let mut d = 0.0;
            for (k, h) in ir.iter().enumerate() {
                if n >= k {
                    d += h * x[n - k];
                }
            }
            assert!((y[n + BLOCK] - d).abs() < 1e-9, "n={n}: {} vs {}", y[n + BLOCK], d);
        }
    }

    #[test]
    fn resampler_keeps_a_tone() {
        let x: Vec<f64> = (0..44100).map(|k| (2.0 * std::f64::consts::PI * 1000.0 * k as f64 / 44100.0).sin()).collect();
        let y = resample(&x, 44100.0, 48000.0);
        assert!((y.len() as i64 - 48000).abs() < 2);
        let mid = &y[10000..40000];
        let rms = (mid.iter().map(|v| v * v).sum::<f64>() / mid.len() as f64).sqrt();
        assert!((rms - 0.7071).abs() < 0.01, "{rms}");
    }

    #[test]
    fn loads_a_wav() {
        let dir = std::env::temp_dir().join("vintage_ir_test.wav");
        let spec = hound::WavSpec { channels: 2, sample_rate: 44100, bits_per_sample: 24, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(&dir, spec).unwrap();
        for k in 0..4410 {
            let v = ((1 << 22) as f64 * (-(k as f64) / 300.0).exp()) as i32;
            w.write_sample(v).unwrap();
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
        let c = load(&dir, 48000.0).unwrap();
        assert!((c.seconds - 0.1).abs() < 0.002);
    }
}
