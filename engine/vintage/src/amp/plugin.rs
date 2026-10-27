//! The DSP wrapper every amp plugin uses: 4x oversampled amplifier, then a
//! built-in cabinet, a loaded impulse response, or straight out (DI).

use super::cab::{CabSpec, Cabinet};
use super::ir::{IrShared, IrSlot, BLOCK};
use super::model::{Amp, AmpSpec, Controls};
use crate::dsp::{db_to_gain, DcBlock, FixedDelay, Oversampler, Smooth, OS, OS_LATENCY};
use crate::VintageDsp;
use std::marker::PhantomData;
use std::sync::Arc;

/// Cab selector positions.
pub const CAB_BUILT_IN: usize = 0;
pub const CAB_IR: usize = 1;
pub const CAB_DI: usize = 2;
/// The cabinet without its close mic (for Studio Spaces / Mic after it).
pub const CAB_SPEAKER: usize = 3;
pub const CAB_POSITIONS: &[&str] = &["MIC'D", "IR", "DI", "SPEAKER"];

/// What the panel's parameters ask the amp to do.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub controls: Controls,
    /// Guitar level into the amp (0 dB = 1 V peak at 0 dBFS).
    pub input_db: f32,
    pub output_db: f32,
    pub cab: usize,
    pub mic: f32,
}

/// One amplifier: its circuit, its cabinet and its panel mapping.
pub trait AmpModel: Send + 'static {
    fn spec() -> AmpSpec;
    fn cab() -> CabSpec;
    /// Map the plugin's plain parameter values to settings.
    fn settings(plain: &[f32]) -> Settings;
}

/// Meter slots: speaker level relative to full power, supply sag (volts).
pub const METER_POWER: usize = 0;
pub const METER_SAG: usize = 1;

pub struct AmpDsp<M: AmpModel> {
    amp: Amp,
    os: Oversampler,
    cab: Cabinet,
    ir: Option<Arc<IrShared>>,
    slot: IrSlot,
    align: FixedDelay,
    dc: DcBlock,
    in_gain: Smooth,
    out_gain: Smooth,
    target: Controls,
    current: Controls,
    smooth_k: f64,
    counter: usize,
    cab_mode: usize,
    to_digital: f64,
    _m: PhantomData<M>,
}

/// Controls are glided toward their targets every this many samples.
const CONTROL_STEP: usize = 32;

fn glide(cur: f64, target: f64, k: f64) -> f64 {
    let v = cur + (target - cur) * k;
    if (v - target).abs() < 1e-4 {
        target
    } else {
        v
    }
}

impl<M: AmpModel> VintageDsp for AmpDsp<M> {
    const METERS: usize = 2;

    fn new(sample_rate: f32, _channels: usize) -> Self {
        let fs = sample_rate as f64;
        let amp = Amp::new(fs * OS as f64, M::spec());
        // full rated power -> -6 dBFS before the cabinet
        let to_digital = 0.5 / amp.v_full_secondary;
        let c = Controls::default();
        Self {
            amp,
            os: Oversampler::new(),
            cab: Cabinet::new(fs, M::cab()),
            ir: None,
            slot: IrSlot::default(),
            align: FixedDelay::new(BLOCK),
            dc: DcBlock::new(fs),
            in_gain: Smooth::new(fs, 20.0, 1.0),
            out_gain: Smooth::new(fs, 20.0, 1.0),
            target: c,
            current: c,
            smooth_k: 1.0 - (-(CONTROL_STEP as f64) / (0.03 * fs)).exp(),
            counter: 0,
            cab_mode: CAB_BUILT_IN,
            to_digital,
            _m: PhantomData,
        }
    }

    fn reset(&mut self) {
        self.amp.reset();
        self.os.reset();
        self.cab.reset();
        self.align.reset();
        self.dc.reset();
        if let Some(c) = self.slot.active.as_mut() {
            c.reset();
        }
        self.current = self.target;
        self.amp.set_controls(self.current);
    }

    fn latency(&self) -> u32 {
        (OS_LATENCY + BLOCK) as u32
    }

    fn attach_ir(&mut self, ir: Arc<IrShared>) {
        self.ir = Some(ir);
    }

    fn set_params(&mut self, plain: &[f32]) {
        let s = M::settings(plain);
        self.target = s.controls;
        // switches act at once (as the real ones do)
        self.current.bright = s.controls.bright;
        self.current.alt = s.controls.alt;
        self.in_gain.set(db_to_gain(s.input_db as f64));
        self.out_gain.set(db_to_gain(s.output_db as f64));
        self.cab_mode = s.cab;
        self.cab.set_mic(s.mic as f64);
    }

    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        if let Some(ir) = self.ir.as_ref() {
            self.slot.poll(ir);
        }
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().max(1);
        let mut power: f64 = 0.0;
        for k in 0..n {
            if self.counter == 0 {
                let t = self.target;
                let c = &mut self.current;
                let kk = self.smooth_k;
                c.volume = glide(c.volume, t.volume, kk);
                c.treble = glide(c.treble, t.treble, kk);
                c.bass = glide(c.bass, t.bass, kk);
                c.middle = glide(c.middle, t.middle, kk);
                c.cut = glide(c.cut, t.cut, kk);
                self.amp.set_controls(*c);
            }
            self.counter = (self.counter + 1) % CONTROL_STEP;

            // an amp is a mono device: sum the inputs
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x *= self.in_gain.next() / nch as f64;
            let amp = &mut self.amp;
            let y = self.os.process(x, |u| amp.process(u));
            let y = self.dc.process(y) * self.to_digital;
            power = power.max(y.abs() * 2.0);
            let y = match (self.cab_mode, self.slot.active.as_mut()) {
                (CAB_DI, _) => self.align.process(y),
                (CAB_SPEAKER, _) => {
                    let c = self.cab.process_speaker(y);
                    self.align.process(c)
                }
                (CAB_IR, Some(conv)) => conv.process(y),
                _ => {
                    let c = self.cab.process(y);
                    self.align.process(c)
                }
            };
            let y = (y * self.out_gain.next()) as f32;
            for c in chans.iter_mut() {
                c[k] = y;
            }
        }
        // 0 VU on the panel meter = full rated power
        meters[METER_POWER] = (power * 10f64.powf(-18.0 / 20.0)) as f32;
        meters[METER_SAG] = self.amp.sag().max(0.0) as f32;
    }
}
