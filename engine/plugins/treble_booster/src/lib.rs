//! Treble Booster — single-transistor germanium treble booster.
//!
//! The 60s British trick for making a Vox-style amp sing: a one-transistor
//! booster with a small input capacitor, so it boosts mostly treble and hits
//! the amp hard. The germanium transistor rounds the peaks off unevenly
//! (lots of 2nd harmonic), and its bias changes the character from sweet
//! to sputtery. RANGE swaps the input capacitor: TREBLE (classic), MID
//! (fatter, more midrange) or FULL (the whole range, a fuzzier boost).

use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
use vintage::dsp::*;
use vintage::nih_plug_egui::EguiState;
use vintage::params::*;
use vintage::ui::{self, Item, MeterKind, Panel, Preset};
use vintage::{VintageDsp, VintageParams};

const W: u32 = 640;
const H: u32 = 432;

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Range {
    #[name = "Treble"]
    Treble,
    #[name = "Mid"]
    Mid,
    #[name = "Full"]
    Full,
}

#[derive(Params)]
pub struct BoosterParams {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "boost"]
    boost: FloatParam,
    #[id = "output"]
    output: FloatParam,
    #[id = "range"]
    range: EnumParam<Range>,
    #[id = "bias"]
    bias: FloatParam,
    #[id = "tone"]
    tone: FloatParam,
    #[id = "mix"]
    mix: FloatParam,
}

impl Default for BoosterParams {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            boost: db("Boost", 18.0, 0.0, 30.0),
            output: db("Output", -6.0, -24.0, 6.0),
            range: EnumParam::new("Range", Range::Treble),
            bias: pct("Bias", 0.5),
            tone: hz("Tone", 9000.0, 2500.0, 16000.0),
            mix: pct("Mix", 1.0),
        }
    }
}

const BOOST: usize = 0;
const OUTPUT: usize = 1;
const RANGE: usize = 2;
const BIAS: usize = 3;
const TONE: usize = 4;
const MIX: usize = 5;

impl VintageParams for BoosterParams {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.boost.as_ptr(),
            self.output.as_ptr(),
            self.range.as_ptr(),
            self.bias.as_ptr(),
            self.tone.as_ptr(),
            self.mix.as_ptr(),
        ]
    }
}

/// (input high-pass Hz, how much full-range signal leaks past the cap)
fn range_profile(r: usize) -> (f64, f64) {
    match r {
        0 => (2400.0, 0.10),
        1 => (750.0, 0.22),
        _ => (100.0, 0.85),
    }
}

/// Germanium transistor stage: soft, asymmetric, with bias moving the
/// operating point. Low bias = "starved": more asymmetry, gated sputter.
#[inline]
fn germanium(x: f64, bias: f64) -> f64 {
    let b = 0.15 + 0.7 * (1.0 - bias); // offset
    let pos = (x + b).tanh() - b.tanh();
    // the other side flattens off earlier and harder
    let neg_knee = 0.55 + 0.6 * bias;
    if pos >= 0.0 {
        pos
    } else {
        -neg_knee * (-pos / neg_knee).tanh()
    }
}

#[derive(Clone)]
struct Chan {
    hp: Biquad,
    os: Oversampler,
    dc: DcBlock,
    lp1: Biquad,
    lp2: Biquad,
    dry: FixedDelay,
}

pub struct BoosterDsp {
    fs: f64,
    ch: Vec<Chan>,
    boost: Smooth,
    out: Smooth,
    bias: Smooth,
    mix: Smooth,
    leak: f64,
    last: Option<(f32, f32)>,
}

impl VintageDsp for BoosterDsp {
    const METERS: usize = 2;

    fn new(sample_rate: f32, channels: usize) -> Self {
        let fs = sample_rate as f64;
        let c = Chan {
            hp: Biquad::identity(),
            os: Oversampler::new(),
            dc: DcBlock::new(fs),
            lp1: Biquad::identity(),
            lp2: Biquad::identity(),
            dry: FixedDelay::new(OS_LATENCY),
        };
        Self {
            fs,
            ch: vec![c; channels.max(1)],
            boost: Smooth::new(fs, 30.0, db_to_gain(18.0)),
            out: Smooth::new(fs, 30.0, 0.5),
            bias: Smooth::new(fs, 30.0, 0.5),
            mix: Smooth::new(fs, 30.0, 1.0),
            leak: 0.1,
            last: None,
        }
    }

    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }

    fn reset(&mut self) {
        for c in &mut self.ch {
            c.hp.reset();
            c.os.reset();
            c.dc.reset();
            c.lp1.reset();
            c.lp2.reset();
            c.dry.reset();
        }
    }

    fn set_params(&mut self, v: &[f32]) {
        self.boost.set(db_to_gain(v[BOOST] as f64));
        self.out.set(db_to_gain(v[OUTPUT] as f64));
        self.bias.set(v[BIAS] as f64);
        self.mix.set(v[MIX] as f64);
        let key = (v[RANGE], v[TONE]);
        if self.last != Some(key) {
            let (hp, leak) = range_profile(v[RANGE].round() as usize);
            self.leak = leak;
            for c in &mut self.ch {
                c.hp.set_highpass(self.fs, hp, 0.6);
                c.lp1.set_lowpass(self.fs, v[TONE] as f64, 0.6);
                c.lp2.set_lowpass(self.fs, v[TONE] as f64 * 1.4, 0.7);
            }
            self.last = Some(key);
        }
    }

    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(self.ch.len());
        let mut peak = 0.0f64;
        let mut drive_pk = 0.0f64;
        let leak = self.leak;
        for k in 0..n {
            let g = self.boost.next();
            let o = self.out.next();
            let bias = self.bias.next();
            let mix = self.mix.next();
            for c in 0..nch {
                let st = &mut self.ch[c];
                let x = chans[c][k] as f64;
                // input capacitor: mostly treble gets through
                let shaped = st.hp.process(x) + leak * x;
                let y = st.os.process(shaped, |v| {
                    let d = v * g * 2.0;
                    drive_pk = drive_pk.max(d.abs());
                    germanium(d, bias)
                });
                let y = st.lp2.process(st.lp1.process(st.dc.process(y)));
                let dry = st.dry.process(x);
                let out = (dry + mix * (y - dry)) * o;
                peak = peak.max(out.abs());
                chans[c][k] = out as f32;
            }
        }
        meters[0] = peak as f32;
        meters[1] = drive_pk as f32;
    }
}

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Classic Treble Boost", values: &[] },
        Preset { name: "Singing Lead (into a hot amp)", values: &[(BOOST, 24.0), (RANGE, 0.0), (BIAS, 0.6), (OUTPUT, -4.0)] },
        Preset { name: "Mid Boost Lead", values: &[(RANGE, 1.0), (BOOST, 20.0), (TONE, 7500.0), (OUTPUT, -8.0)] },
        Preset { name: "Full Range Fuzzy", values: &[(RANGE, 2.0), (BOOST, 26.0), (BIAS, 0.4), (OUTPUT, -12.0)] },
        Preset { name: "Starved Germanium Sputter", values: &[(BOOST, 28.0), (BIAS, 0.05), (TONE, 6000.0), (OUTPUT, -10.0)] },
        Preset { name: "Clean Treble Lift", values: &[(BOOST, 6.0), (BIAS, 0.8), (OUTPUT, -2.0), (MIX, 0.6)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "TREBLE BOOSTER",
        subtitle: ("GERMANIUM", "RANGE BOOSTER"),
        accent: ui::TANGERINE,
        size: (W, H),
        items: vec![
            Item::Knob { p: BOOST, x: 130.0, y: 172.0, r: 50.0, label: "BOOST",
                tip: "How hard the booster hits. Run it into an already-crunchy amp (or Valve Desk / Grit Comp) for singing leads.", active: None },
            Item::Knob { p: OUTPUT, x: 510.0, y: 172.0, r: 50.0, label: "OUTPUT",
                tip: "Output level of the pedal.", active: None },
            Item::Meter { m: 0, x: 222.0, y: 92.0, w: 196.0, h: 136.0, kind: MeterKind::Vu, caption: "OUTPUT" },
            Item::Lamp { m: 1, x: 296.0, y: 254.0, label: "CLIPPING", lo: 0.8, hi: 2.5 },
            Item::Divider { y: 284.0 },
            Item::Selector { p: RANGE, x: 140.0, y: 340.0, label: "RANGE", positions: &["TREBLE", "MID", "FULL"],
                tip: "Input capacitor. TREBLE: the classic bright boost. MID: fatter and more midrange. FULL: the whole range, fuzzier." },
            Item::Knob { p: BIAS, x: 270.0, y: 332.0, r: 22.0, label: "BIAS", tip: "Transistor bias. High = sweet and round; low = starved, splatty and gated.", active: None },
            Item::Knob { p: TONE, x: 380.0, y: 332.0, r: 22.0, label: "TONE", tip: "Top-end roll-off after the boost.", active: None },
            Item::Knob { p: MIX, x: 490.0, y: 332.0, r: 22.0, label: "MIX", tip: "Blend with the clean signal.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, BoosterParams, BoosterDsp, panel,
    name: "Treble Booster", class_id: *b"S2TrebleBoostDIY", vendor: "PEQUOD Beatles",
    categories: [Fx, Distortion]
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn level(freq: f64, v: &[f32]) -> f64 {
        let fs = 48000.0;
        let mut d = BoosterDsp::new(fs as f32, 1);
        d.set_params(v);
        let mut x: Vec<f32> = (0..48000).map(|k| (0.001 * (2.0 * PI * freq * k as f64 / fs).sin()) as f32).collect();
        d.process(&mut [&mut x[..]], &mut [0.0, 0.0]);
        let t = &x[24000..];
        (t.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / t.len() as f64).sqrt()
    }

    #[test]
    fn boosts_treble_more_than_bass() {
        let v = [18.0, 0.0, 0.0, 0.5, 16000.0, 1.0];
        let lo = level(150.0, &v);
        let hi = level(3500.0, &v);
        let diff = 20.0 * (hi / lo).log10();
        println!("treble vs bass: {diff:.1} dB");
        assert!(diff > 12.0);
        // FULL range is much flatter
        let v2 = [18.0, 0.0, 2.0, 0.5, 16000.0, 1.0];
        let diff2 = 20.0 * (level(3500.0, &v2) / level(150.0, &v2)).log10();
        assert!(diff2 < diff - 6.0);
    }

    #[test]
    fn germanium_is_asymmetric() {
        // the two halves of the waveform clip differently (even harmonics)
        assert!((germanium(3.0, 0.5) + germanium(-3.0, 0.5)).abs() > 0.1);
        assert!(germanium(0.0, 0.5).abs() < 1e-12);
    }
}
