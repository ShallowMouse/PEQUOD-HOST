//! Vari-Mu — variable-mu valve compressor/limiter.
//!
//! In a vari-mu design the gain-control valves are biased further down as
//! the level rises, so the ratio gets steeper the harder you hit it: gentle
//! on quiet passages, nearly a limiter on loud ones. The six-position
//! TIME CONSTANT switch picks attack/release combinations, the last two
//! being program-dependent (a fast release for peaks riding on a slow one
//! for the overall level). The valves add a little colour, more when
//! working harder.

use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
use vintage::dsp::*;
use vintage::nih_plug_egui::EguiState;
use vintage::params::*;
use vintage::ui::{self, Preset, Item, MeterKind, Panel};
use vintage::{VintageDsp, VintageParams};

const W: u32 = 640;
const H: u32 = 432;

#[derive(Params)]
pub struct VariMuParams {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "input"]
    input: FloatParam,
    #[id = "threshold"]
    threshold: FloatParam,
    #[id = "time"]
    time: IntParam,
    #[id = "colour"]
    colour: FloatParam,
    #[id = "mix"]
    mix: FloatParam,
    #[id = "output"]
    output: FloatParam,
}

impl Default for VariMuParams {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            input: db("Input", 0.0, -20.0, 20.0),
            threshold: db("Threshold", -18.0, -40.0, 0.0),
            time: IntParam::new("Time Constant", 2, IntRange::Linear { min: 1, max: 6 }),
            colour: pct("Valve Colour", 0.3),
            mix: pct("Mix", 1.0),
            output: db("Output", 0.0, -20.0, 20.0),
        }
    }
}

const INPUT: usize = 0;
const THRESHOLD: usize = 1;
const TIME: usize = 2;
const COLOUR: usize = 3;
const MIX: usize = 4;
const OUTPUT: usize = 5;

impl VintageParams for VariMuParams {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.input.as_ptr(),
            self.threshold.as_ptr(),
            self.time.as_ptr(),
            self.colour.as_ptr(),
            self.mix.as_ptr(),
            self.output.as_ptr(),
        ]
    }
}

/// (attack ms, fast release ms, slow release ms, share of slow release)
fn time_constant(pos: i32) -> (f64, f64, f64, f64) {
    match pos {
        1 => (0.2, 300.0, 300.0, 0.0),
        2 => (0.2, 800.0, 800.0, 0.0),
        3 => (0.4, 2000.0, 2000.0, 0.0),
        4 => (0.4, 5000.0, 5000.0, 0.0),
        5 => (0.4, 2000.0, 10000.0, 0.5),    // program dependent
        _ => (0.2, 300.0, 15000.0, 0.6),     // program dependent, peaks + overall
    }
}

/// Static curve: ratio rises with how far over the knee we are.
/// The instantaneous ratio is 1 + A*o^2 (o = dB above the knee start), so
/// the output is the integral of 1/ratio: atan(sqrt(A)*o)/sqrt(A). Gentle
/// (~2.5:1) at the threshold, approaching limiting ~15 dB above it.
#[inline]
fn vari_mu_gr(over_db: f64) -> f64 {
    const KNEE: f64 = 6.0;
    const A: f64 = 0.042;
    let o = over_db + KNEE;
    if o <= 0.0 {
        return 0.0;
    }
    let s = A.sqrt();
    (s * o).atan() / s - o
}

#[derive(Clone)]
struct Chan {
    os: Oversampler,
    dc: DcBlock,
    dry: FixedDelay,
}

pub struct VariMuDsp {
    fs: f64,
    ch: Vec<Chan>,
    input: Smooth,
    out: Smooth,
    mix: Smooth,
    colour: Smooth,
    thr: f64,
    fast: f64,
    slow: f64,
    att: f64,
    rel_fast: f64,
    rel_slow: f64,
    slow_share: f64,
    detector: Envelope,
}

impl VintageDsp for VariMuDsp {
    const METERS: usize = 1;

    fn new(sample_rate: f32, channels: usize) -> Self {
        let fs = sample_rate as f64;
        let c = Chan { os: Oversampler::new(), dc: DcBlock::new(fs), dry: FixedDelay::new(OS_LATENCY) };
        let mut s = Self {
            fs,
            ch: vec![c; channels.max(1)],
            input: Smooth::new(fs, 30.0, 1.0),
            out: Smooth::new(fs, 30.0, 1.0),
            mix: Smooth::new(fs, 30.0, 1.0),
            colour: Smooth::new(fs, 30.0, 0.3),
            thr: -18.0,
            fast: 0.0,
            slow: 0.0,
            att: 0.0,
            rel_fast: 0.0,
            rel_slow: 0.0,
            slow_share: 0.0,
            // RMS-ish level sensing with a short window
            detector: Envelope::new(fs, 0.05, 5.0),
        };
        s.set_tc(2);
        s
    }

    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }

    fn reset(&mut self) {
        for c in &mut self.ch {
            c.os.reset();
            c.dc.reset();
            c.dry.reset();
        }
        self.fast = 0.0;
        self.slow = 0.0;
    }

    fn set_params(&mut self, v: &[f32]) {
        self.input.set(db_to_gain(v[INPUT] as f64));
        self.out.set(db_to_gain(v[OUTPUT] as f64));
        self.mix.set(v[MIX] as f64);
        self.colour.set(v[COLOUR] as f64);
        self.thr = v[THRESHOLD] as f64;
        self.set_tc(v[TIME].round() as i32);
    }

    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(self.ch.len());
        let mut max_gr = 0.0f64;
        for k in 0..n {
            let g = self.input.next();
            let o = self.out.next();
            let mix = self.mix.next();
            let colour = self.colour.next();
            let mut det = 0.0f64;
            for c in 0..nch {
                det = det.max((chans[c][k] as f64 * g).abs());
            }
            let level = self.detector.process(det);
            let target = vari_mu_gr(gain_to_db(level) - self.thr); // <= 0
            let step = |env: &mut f64, att: f64, rel: f64| {
                let cf = if target < *env { att } else { rel };
                *env = target + cf * (*env - target);
            };
            step(&mut self.fast, self.att, self.rel_fast);
            step(&mut self.slow, self.att, self.rel_slow);
            let gr = (1.0 - self.slow_share) * self.fast + self.slow_share * self.slow;
            let gr = gr.min(0.0);
            max_gr = max_gr.max(-gr);
            // half-strength automatic make-up so the threshold knob doesn't just turn it down
            let makeup = (-self.thr * 0.25).max(0.0);
            let gain = db_to_gain(gr + makeup);
            // valves colour more as they're biased harder
            let drive = 1.0 + colour * (0.6 + (-gr / 12.0).min(1.5));
            for c in 0..nch {
                let st = &mut self.ch[c];
                let x = chans[c][k] as f64;
                let y = x * g * gain;
                let y = st.os.process(y, |v| tube(v * drive, 0.25 * colour) / drive);
                let y = st.dc.process(y);
                let dry = st.dry.process(x);
                chans[c][k] = ((dry + mix * (y - dry)) * o) as f32;
            }
        }
        meters[0] = max_gr as f32;
    }
}

impl VariMuDsp {
    fn set_tc(&mut self, pos: i32) {
        let (a, rf, rs, share) = time_constant(pos);
        self.att = coeff(self.fs, a);
        self.rel_fast = coeff(self.fs, rf / 3.0);
        self.rel_slow = coeff(self.fs, rs / 3.0);
        self.slow_share = share;
    }
}

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Gentle Bus Glue", values: &[(THRESHOLD, -12.0), (TIME, 3.0), (COLOUR, 0.2)] },
        Preset { name: "Vocal Levelling", values: &[(THRESHOLD, -22.0), (TIME, 2.0), (COLOUR, 0.3)] },
        Preset { name: "Drum Squash", values: &[(INPUT, 6.0), (THRESHOLD, -28.0), (TIME, 1.0), (COLOUR, 0.5)] },
        Preset { name: "Parallel Smash", values: &[(INPUT, 10.0), (THRESHOLD, -34.0), (TIME, 1.0), (COLOUR, 0.6), (MIX, 0.35)] },
        Preset { name: "Program-Dependent Master", values: &[(THRESHOLD, -10.0), (TIME, 5.0), (COLOUR, 0.15)] },
        Preset { name: "Bass Guitar Evener", values: &[(THRESHOLD, -20.0), (TIME, 4.0), (COLOUR, 0.35)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "VARI-MU",
        subtitle: ("VARIABLE-MU VALVE", "LIMITING AMPLIFIER"),
        accent: ui::MUSTARD,
        size: (W, H),
        items: vec![
            Item::Knob { p: INPUT, x: 130.0, y: 172.0, r: 50.0, label: "INPUT",
                tip: "Gain into the compressor. Push harder to squash more: the ratio rises the more you go over the threshold.", active: None },
            Item::Knob { p: THRESHOLD, x: 510.0, y: 172.0, r: 50.0, label: "THRESHOLD",
                tip: "Where compression starts. The knee is very wide, so it begins gently a few dB below this.", active: None },
            Item::Meter { m: 0, x: 222.0, y: 92.0, w: 196.0, h: 136.0, kind: MeterKind::GainReduction, caption: "GAIN REDUCTION" },
            Item::Divider { y: 284.0 },
            Item::Selector { p: TIME, x: 130.0, y: 340.0, label: "TIME CONSTANT", positions: &["1", "2", "3", "4", "5", "6"],
                tip: "Attack / release:\n1: 0.2 ms / 0.3 s\n2: 0.2 ms / 0.8 s\n3: 0.4 ms / 2 s\n4: 0.4 ms / 5 s\n5: 0.4 ms / 2 s + 10 s (program dependent)\n6: 0.2 ms / 0.3 s + 15 s (program dependent)" },
            Item::Knob { p: COLOUR, x: 290.0, y: 332.0, r: 22.0, label: "VALVE COLOUR", tip: "Valve saturation, increasing with gain reduction.", active: None },
            Item::Knob { p: MIX, x: 400.0, y: 332.0, r: 22.0, label: "MIX", tip: "Parallel compression: blend with the dry signal.", active: None },
            Item::Knob { p: OUTPUT, x: 510.0, y: 332.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, VariMuParams, VariMuDsp, panel,
    name: "Vari-Mu", class_id: *b"S2VariMuDIY00001", vendor: "PEQUOD Beatles",
    categories: [Fx, Dynamics]
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn out_db(in_db: f64, thr: f32, tc: f32) -> (f64, f32) {
        let fs = 48000.0;
        let mut d = VariMuDsp::new(fs as f32, 1);
        d.set_params(&[0.0, thr, tc, 0.0, 1.0, 0.0]);
        let n = 96000;
        let a = db_to_gain(in_db);
        let mut x: Vec<f32> = (0..n).map(|k| (a * (2.0 * PI * 400.0 * k as f64 / fs).sin()) as f32).collect();
        let mut m = [0.0f32; 1];
        d.process(&mut [&mut x[..]], &mut m);
        let tail = &x[n * 3 / 4..];
        let rms = (tail.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / tail.len() as f64).sqrt();
        (gain_to_db(rms * 2f64.sqrt()), m[0])
    }

    #[test]
    fn ratio_rises_with_level() {
        // slope (output dB change per input dB) should fall as level rises
        let (a, _) = out_db(-30.0, -18.0, 2.0);
        let (b, _) = out_db(-18.0, -18.0, 2.0);
        let (c, _) = out_db(-6.0, -18.0, 2.0);
        let (d, gr) = out_db(0.0, -18.0, 2.0);
        let s1 = (b - a) / 12.0;
        let s2 = (d - c) / 6.0;
        println!("slopes {s1:.2} then {s2:.2}, GR at 0 dB in: {gr:.1}");
        assert!(s1 > 0.6 && s2 < 0.25);
        assert!(gr > 6.0);
    }

    #[test]
    fn all_time_constants_stable() {
        for tc in 1..=6 {
            let (o, _) = out_db(0.0, -30.0, tc as f32);
            assert!(o.is_finite() && o < 6.0);
        }
    }
}
