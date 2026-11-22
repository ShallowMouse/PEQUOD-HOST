//! CE-2 Chorus: the Boss CE-2 (1979), the chorus on David Gilmour's board from The Wall on: a bucket-brigade delay swept
//! by a triangle LFO, mixed with the dry signal (mono).

#![allow(clippy::excessive_precision)]
use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
#[allow(unused_imports)]
use vintage::dsp::*;
use vintage::nih_plug_egui::EguiState;
#[allow(unused_imports)]
use vintage::nih_plug_egui::egui::Align2;
#[allow(unused_imports)]
use vintage::params::*;
#[allow(unused_imports)]
use vintage::ui::{self, ActiveWhen, Item, MeterKind, Panel, Preset};
#[allow(unused_imports)]
use vintage::{Midi, VintageDsp, VintageParams};

pub const W: u32 = 640;
pub const H: u32 = 432;


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "rate"]
    pub rate: FloatParam,
    #[id = "depth"]
    pub depth: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            rate: rate("Rate", 0.8, 0.1, 7.0),
            depth: pct("Depth", 0.5),
            output: db("Output", 0.0, -12.0, 6.0),
        }
    }
}

pub const RATE: usize = 0;
pub const DEPTH: usize = 1;
pub const OUTPUT: usize = 2;
pub const NPARAMS: usize = 3;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.rate.as_ptr(),
            self.depth.as_ptr(),
            self.output.as_ptr(),
        ]
    }
}

/// Default plain values in linked order (for tests).
#[allow(dead_code)]
pub fn defaults() -> Vec<f32> {
    let p = P::default();
    p.linked().iter().map(|q| unsafe { q.preview_plain(q.default_normalized_value()) }).collect()
}

// -------------------------------------------------------------------- dsp --

use vintage::fx::ClockedBbd;

/// MN3007: 1024 stages.
const STAGES: usize = 1024;
/// The delay at the top of the clock's sweep.
const BASE_MS: f64 = 5.0;

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    bbd: ClockedBbd,
    os: Oversampler,
    pre: [Biquad; 2],
    post: [Biquad; 2],
    ph: f64,
    hp: Biquad,
    dry: FixedDelay,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let ofs = fs * OS as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        // the input and output filters around the chip: two 2-pole transistor low-passes each side
        let mut pre = [Biquad::identity(); 2];
        pre[0].set_lowpass(ofs, 7000.0, 0.54);
        pre[1].set_lowpass(ofs, 7000.0, 1.31);
        let post = pre;
        let mut hp = Biquad::identity();
        hp.set_highpass(fs, 20.0, 0.7);
        Self { fs, v, bbd: ClockedBbd::new(STAGES, ofs), os: Oversampler::new(), pre, post, ph: 0.0, hp, dry: FixedDelay::new(OS_LATENCY), out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }
    fn reset(&mut self) {
        self.bbd.reset();
        self.os.reset();
        self.hp.reset();
        self.dry.reset();
        for b in self.pre.iter_mut().chain(self.post.iter_mut()) {
            b.reset();
        }
        self.ph = 0.0;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let rate = self.v[RATE] as f64;
        let depth_ms = 0.2 + 3.0 * self.v[DEPTH] as f64;
        // the MN3101's clock swings between these; the delay follows 1 / clock
        let f_hi = self.bbd.clock_for(BASE_MS * 1e-3);
        let f_lo = self.bbd.clock_for((BASE_MS + depth_ms) * 1e-3);
        let nch = chans.len();
        let mut m = 0.0;
        for k in 0..n {
            self.ph = (self.ph + rate / self.fs).fract();
            // the CE-2's LFO: a triangle into the clock's VCO
            let tri = 1.0 - 4.0 * (self.ph - 0.5).abs();
            m = tri;
            let f_clk = f_hi + (f_lo - f_hi) * 0.5 * (1.0 + tri);
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            let pre = self.hp.process(x);
            let (bbd, fpre, fpost) = (&mut self.bbd, &mut self.pre, &mut self.post);
            let w = self.os.process(pre, |u| {
                // the chip clips softly near its rails
                let a = cascade(fpre, u);
                let d = bbd.process((a * 0.8).tanh() / 0.8, f_clk);
                cascade(fpost, d)
            });
            // the dry path is delayed to line up with the oversampled wet one
            let y = (self.dry.process(pre) + w) * 0.7 * self.out.next();
            for c in chans.iter_mut() {
                c[k] = y as f32;
            }
        }
        meters[0] = (0.5 + 0.5 * m) as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Subtle Thickening", values: &[(RATE, 0.6), (DEPTH, 0.35)] },  // OCR 'Subtle Thickening'
        Preset { name: "Classic CE-2", values: &[] },  // OCR 'Classic CE-2'
        Preset { name: "Lush Slow Clean", values: &[(RATE, 0.4), (DEPTH, 0.8)] },
        Preset { name: "Fast Warble", values: &[(RATE, 4.5), (DEPTH, 0.6)] },
    ]
}

/*
('knob', 209.5, 159.5, 43.9)
('knob', 429.5, 159.5, 43.9)
('divider', 284)
('knob', 319.5, 335.5, 22.0)

crate: ['floyd/pf_ce2', 'library/panic_abort']
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/alloc/src/collections/btree/navigate.rs
/root/.cargo/git/checkouts/nih-plug-d4210f82e920295f/de42101/nih_plug_egui/src/editor.rs
AFTER THE BOSS CE-2BUCKET-BRIGADE CHORUS  
  1979RATELFO rate.DEPTHHow far the delay is swept: more depth = more detune.LFOOUTPUTOutput level.CE-2 CHORUS
Subtle ThickeningClassic CE-2
Lush Slow Clean
Fast Warble$min > max, or either was NaN. min = 
, max = 
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/core/src/num/f64.rs
vintage/src/dsp.rs
RateDepthOutputassertion failed: unsafe { QueryPerformanceCounter(&mut ticks).as_bool() }/root/.cargo/git/checkouts/nih-plug-d4210f82e920295f/de42101/src/event_loop/windows.rs
nih-event-loop-

*/
fn panel() -> Panel {
    Panel {
        title: "CE-2 CHORUS",
        subtitle: ("AFTER THE BOSS CE-2", "BUCKET-BRIGADE CHORUS  ·  1979"),
        accent: ui::VIOLET,
        size: (W, H),
        items: vec![
            Item::Knob { p: RATE, x: 210.0, y: 160.0, r: 44.0, label: "RATE", tip: "LFO rate.", active: None },
            Item::Lamp { m: 0, x: 320.0, y: 160.0, label: "LFO", lo: 0.0, hi: 1.0 },
            Item::Knob { p: DEPTH, x: 430.0, y: 160.0, r: 44.0, label: "DEPTH", tip: "How far the delay is swept: more depth = more detune.", active: None },
            Item::Divider { y: 284.0 },
            Item::Knob { p: OUTPUT, x: 320.0, y: 336.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "CE-2 Chorus", class_id: *b"PFChorusCE2PQD01", vendor: "PEQUOD Pink Floyd",
    categories: [Fx, Modulation]
);
