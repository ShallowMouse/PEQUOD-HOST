//! Channel Dynamics: the compressor and gate that sat on every channel of the new SSL desks at the Townhouse and the Manor. VCA compressor with soft knee and auto make-up; expander/gate with hold and range. Laws estimated from typical console dynamics of the period.

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
use vintage::fx::{comp_gr, Gate};

pub const W: u32 = 640;
pub const H: u32 = 432;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Attack { #[name = "Auto"] Auto, #[name = "Fast"] Fast }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "ratio"]
    pub ratio: FloatParam,
    #[id = "thresh"]
    pub thresh: FloatParam,
    #[id = "release"]
    pub release: FloatParam,
    #[id = "attack"]
    pub attack: EnumParam<Attack>,
    #[id = "gthresh"]
    pub g_thresh: FloatParam,
    #[id = "range"]
    pub range: FloatParam,
    #[id = "ghold"]
    pub g_hold: FloatParam,
    #[id = "grel"]
    pub g_rel: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            ratio: FloatParam::new("Ratio", 4.0, FloatRange::Skewed { min: 1.0, max: 20.0, factor: FloatRange::skew_factor(-1.0) }).with_step_size(0.1).with_unit(":1"),
            thresh: db("Threshold", -10.0, -40.0, 10.0),
            release: ms("Release", 200.0, 50.0, 3000.0),
            attack: EnumParam::new("Attack", Attack::Auto),
            g_thresh: db("Gate Threshold", -60.0, -70.0, 0.0),
            range: db("Gate Range", 40.0, 0.0, 80.0),
            g_hold: ms("Gate Hold", 50.0, 0.0, 2000.0),
            g_rel: ms("Gate Release", 100.0, 5.0, 2000.0),
            output: db("Output", 0.0, -18.0, 18.0),
        }
    }
}

pub const RATIO: usize = 0;
pub const THRESH: usize = 1;
pub const RELEASE: usize = 2;
pub const ATTACK: usize = 3;
pub const GTHRESH: usize = 4;
pub const RANGE: usize = 5;
pub const GHOLD: usize = 6;
pub const GREL: usize = 7;
pub const OUTPUT: usize = 8;
pub const NPARAMS: usize = 9;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.ratio.as_ptr(),
            self.thresh.as_ptr(),
            self.release.as_ptr(),
            self.attack.as_ptr(),
            self.g_thresh.as_ptr(),
            self.range.as_ptr(),
            self.g_hold.as_ptr(),
            self.g_rel.as_ptr(),
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

pub struct Dsp {
    fs: f64,
    env: f64,
    gr_db: f64,
    gate: Gate,
    ratio: f64,
    thresh: f64,
    release: f64,
    fast: bool,
    g_thresh: f64,
    range: f64,
    hold: f64,
    g_rel: f64,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 2;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut gate = Gate::new(fs);
        gate.set_times(0.1, 100.0);
        Self { fs, env: 0.0, gr_db: 0.0, gate, ratio: 4.0, thresh: -10.0, release: 200.0, fast: false, g_thresh: -60.0, range: 40.0, hold: 50.0, g_rel: 100.0, out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.env = 0.0;
        self.gr_db = 0.0;
        self.gate.reset();
    }
    fn set_params(&mut self, v: &[f32]) {
        self.ratio = v[RATIO] as f64;
        self.thresh = v[THRESH] as f64;
        self.release = v[RELEASE] as f64;
        self.fast = v[ATTACK] > 0.5;
        self.g_thresh = v[GTHRESH] as f64;
        self.range = v[RANGE] as f64;
        self.hold = v[GHOLD] as f64;
        let gr = v[GREL] as f64;
        if (gr - self.g_rel).abs() > 1e-3 {
            self.g_rel = gr;
            self.gate.set_times(0.1, gr);
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let mut peak = 0.0f64;
        let mut grmax = 0.0f64;
        let mut gate_min = 1.0f64;
        // auto attack ~ 3-30 ms programme-dependent; fast ~ 0.1 ms
        let rel = coeff(self.fs, self.release / 2.2);
        // dBu-ish scale: 0 dBFS = +18
        let makeup = comp_gr(18.0, self.thresh + 18.0, self.ratio, 10.0) * 0.5;
        for k in 0..n {
            let mut key = 0.0f64;
            for c in chans.iter() {
                key = key.max((c[k] as f64).abs());
            }
            let lvl = gain_to_db(key) + 18.0;
            let target = comp_gr(lvl, self.thresh + 18.0, self.ratio, 10.0);
            let att = if self.fast { coeff(self.fs, 0.1) } else { coeff(self.fs, 3.0 + 27.0 * (1.0 - (target - self.gr_db).clamp(0.0, 20.0) / 20.0)) };
            self.gr_db = if target > self.gr_db { target + att * (self.gr_db - target) } else { target + rel * (self.gr_db - target) };
            grmax = grmax.max(self.gr_db);
            let gg = self.gate.process(key, self.g_thresh, self.hold, self.range);
            gate_min = gate_min.min(gg);
            let g = db_to_gain(makeup - self.gr_db) * gg * self.out.next();
            for c in chans.iter_mut().take(nch) {
                let y = c[k] as f64 * g;
                peak = peak.max(y.abs());
                c[k] = y as f32;
            }
        }
        meters[0] = grmax as f32;
        meters[1] = (1.0 - gate_min) as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Gentle Channel Comp", values: &[(RATIO, 2.0), (THRESH, -8.0), (RELEASE, 300.0)] },
        Preset { name: "Snare Smack", values: &[(RATIO, 6.0), (THRESH, -16.0), (RELEASE, 100.0), (ATTACK, 0.0), (GTHRESH, -30.0), (RANGE, 30.0), (GHOLD, 80.0), (GREL, 60.0)] },
        Preset { name: "Gated Tom (Collins)", values: &[(RATIO, 4.0), (THRESH, -14.0), (GTHRESH, -28.0), (RANGE, 80.0), (GHOLD, 250.0), (GREL, 30.0)] },
        Preset { name: "Kick Tighten", values: &[(RATIO, 4.0), (THRESH, -12.0), (RELEASE, 80.0), (GTHRESH, -30.0), (RANGE, 40.0), (GHOLD, 30.0), (GREL, 40.0)] },
        Preset { name: "Vocal Level (1176-ish fast)", values: &[(RATIO, 4.0), (THRESH, -14.0), (ATTACK, 1.0), (RELEASE, 200.0)] },
        Preset { name: "Bass Even", values: &[(RATIO, 4.0), (THRESH, -12.0), (RELEASE, 400.0)] },
        Preset { name: "Room Mic Crush", values: &[(RATIO, 20.0), (THRESH, -30.0), (ATTACK, 1.0), (RELEASE, 100.0)] },
        Preset { name: "Gate Only", values: &[(RATIO, 1.0), (GTHRESH, -35.0), (RANGE, 60.0), (GHOLD, 100.0), (GREL, 150.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "CHANNEL DYNAMICS",
        subtitle: ("CONSOLE COMPRESSOR + GATE", "ON EVERY CHANNEL · 1980"),
        accent: ui::MUSTARD,
        size: (W, H),
        items: vec![

            Item::Knob { p: RATIO, x: 80.0, y: 150.0, r: 30.0, label: "RATIO", tip: "Compression ratio, 1:1 (off) to 20:1.", active: None },
            Item::Knob { p: THRESH, x: 180.0, y: 150.0, r: 30.0, label: "THRESHOLD", tip: "Compressor threshold. Make-up gain follows it, as on the desk.", active: None },
            Item::Knob { p: RELEASE, x: 280.0, y: 150.0, r: 30.0, label: "RELEASE", tip: "Compressor release.", active: None },
            Item::Switch { p: ATTACK, x: 80.0, y: 222.0, labels: ["AUTO", "FAST"], tip: "Auto: programme-dependent attack (3-30 ms). Fast: 0.1 ms." },
            Item::Meter { m: 0, x: 360.0, y: 92.0, w: 230.0, h: 120.0, kind: MeterKind::GainReduction, caption: "COMPRESSOR" },
            Item::Divider { y: 264.0 },
            Item::Knob { p: GTHRESH, x: 80.0, y: 330.0, r: 22.0, label: "GATE THRESH", tip: "Gate threshold.", active: None },
            Item::Knob { p: RANGE, x: 180.0, y: 330.0, r: 22.0, label: "RANGE", tip: "How far the gate closes (dB). Low settings = an expander.", active: None },
            Item::Knob { p: GHOLD, x: 280.0, y: 330.0, r: 22.0, label: "HOLD", tip: "Gate hold time.", active: None },
            Item::Knob { p: GREL, x: 380.0, y: 330.0, r: 22.0, label: "GATE REL", tip: "Gate release.", active: None },
            Item::Lamp { m: 1, x: 460.0, y: 330.0, label: "GATE", lo: 0.1, hi: 0.9 },
            Item::Knob { p: OUTPUT, x: 570.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Channel Dynamics", class_id: *b"TPChannelDynPQD1", vendor: "PEQUOD Padgham & Lillywhite",
    categories: [Fx, Dynamics]
);

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;

    pub fn run(p: &[f32], input: &mut [Vec<f32>]) -> Vec<f32> {
        let mut d = Dsp::new(48000.0, input.len());
        d.set_params(p);
        d.reset();
        let mut m = vec![0.0f32; Dsp::METERS.max(1)];
        let n = input[0].len();
        let mut k = 0;
        while k < n {
            let e = (k + 512).min(n);
            d.set_params(p);
            let mut chans: Vec<&mut [f32]> = input.iter_mut().map(|c| &mut c[k..e]).collect();
            d.process(&mut chans, &mut m);
            k = e;
        }
        m
    }
    pub fn sine(f: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n).map(|k| (amp * (2.0 * std::f64::consts::PI * f * k as f64 / 48000.0).sin()) as f32).collect()
    }
    pub fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
    }
    pub fn db(x: f64) -> f64 {
        20.0 * x.max(1e-12).log10()
    }
    pub fn with(mut p: Vec<f32>, set: &[(usize, f32)]) -> Vec<f32> {
        for (i, v) in set {
            p[*i] = *v;
        }
        p
    }
    /// Every preset runs finite and bounded on noise bursts.
    #[test]
    fn presets_run_clean() {
        let base = defaults();
        let mut all = vec![base.clone()];
        for pr in presets() {
            all.push(with(base.clone(), pr.values));
        }
        for p in all {
            let mut nz = vintage::dsp::Noise::new(7);
            let n = 48000;
            let mut l: Vec<f32> = (0..n).map(|k| if (k / 6000) % 2 == 0 { (nz.next() * 0.5) as f32 } else { 0.0 }).collect();
            let mut r = l.clone();
            {
                let mut input = vec![std::mem::take(&mut l), std::mem::take(&mut r)];
                run(&p, &mut input);
                assert!(input.iter().flatten().all(|v| v.is_finite() && v.abs() < 8.0), "unbounded output for {:?}", p);
            }
        }
    }

    #[test]
    fn ratio_holds_in_steady_state() {
        let out_db = |amp: f64| {
            let p = with(defaults(), &[(RATIO, 4.0), (THRESH, -20.0), (GTHRESH, -70.0), (ATTACK, 1.0)]);
            let mut input = vec![sine(1000.0, amp, 48000)];
            run(&p, &mut input);
            db(rms(&input[0][36000..]))
        };
        let a = out_db(0.5);
        let b = out_db(0.25);
        let slope = (a - b) / 6.02;
        println!("output slope above threshold: {slope:.2} (expect 0.25)");
        assert!((slope - 0.25).abs() < 0.06);
    }

}
