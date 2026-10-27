//! Cloud: an original PEQUOD ambient reverb: very long, very diffuse and slowly moving, with BLOOM smearing each note over long allpass chains so it swells into the tail instead of starting with a hit. For pads, swells and soundscapes.

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
use vintage::verb::{Diffuser, Fdn};

pub const W: u32 = 640;
pub const H: u32 = 330;



#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "bloom"]
    pub bloom: FloatParam,
    #[id = "mod_amt"]
    pub mod_amt: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            decay: secs("Decay", 8.0, 2.0, 30.0),
            bloom: pct("Bloom", 0.6),
            mod_amt: pct("Mod", 0.5),
            tone: hz("Tone", 5000.0, 1000.0, 14000.0),
            mix: pct("Mix", 0.45),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const DECAY: usize = 0;
pub const BLOOM: usize = 1;
pub const MOD: usize = 2;
pub const TONE: usize = 3;
pub const MIX: usize = 4;
pub const OUTPUT: usize = 5;
pub const NPARAMS: usize = 6;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.decay.as_ptr(),
            self.bloom.as_ptr(),
            self.mod_amt.as_ptr(),
            self.tone.as_ptr(),
            self.mix.as_ptr(),
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
    v: [f32; NPARAMS],
    fdn: Fdn,
    smear: Vec<Diffuser>,
    lp: [Biquad; 2],
    key: Vec<i32>,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let rt = self.v[DECAY] as f64;
        self.fdn.configure(4.0, [rt, rt, rt * 0.55], 3000.0);
        self.fdn.mod_depth = 0.001 + 0.006 * self.v[MOD] as f64;
        let g = 0.45 + 0.3 * self.v[BLOOM] as f64;
        let fs = self.fs;
        self.smear = [31.0, 47.0, 73.0, 109.0, 151.0].iter().map(|ms| Diffuser::new(fs, *ms, g)).collect();
        let f = self.v[TONE] as f64;
        self.lp.iter_mut().for_each(|b| b.set_lowpass(fs, f, 0.7));
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self { fs, v, fdn: Fdn::new(fs, 4.1), smear: vec![], lp: [Biquad::identity(); 2], key: vec![], out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.fdn.reset();
        self.design();
        self.lp.iter_mut().for_each(|b| b.reset());
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k: Vec<i32> = [v[DECAY], v[BLOOM], v[MOD], v[TONE] / 100.0].iter().map(|x| (x * 100.0) as i32).collect();
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let mix = self.v[MIX] as f64;
        let (dg, wg) = ((1.0 - mix).sqrt(), mix.sqrt() * 1.6);
        let bloom = (self.v[BLOOM] as f64 * 5.0).round() as usize;
        let nch = chans.len();
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            let mut s = self.fdn.diffuse(x, 4);
            for d in self.smear.iter_mut().take(bloom) {
                s = d.process(s);
            }
            let (l, r) = self.fdn.process(s);
            let l = self.lp[0].process(l);
            let r = self.lp[1].process(r);
            let o = self.out.next();
            if nch >= 2 {
                chans[0][k] = ((chans[0][k] as f64 * dg + l * wg) * o) as f32;
                chans[1][k] = ((chans[1][k] as f64 * dg + r * wg) * o) as f32;
            } else if nch == 1 {
                chans[0][k] = ((chans[0][k] as f64 * dg + 0.5 * (l + r) * wg) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Cloud", values: &[] },
        Preset { name: "Endless Pad", values: &[(DECAY, 22.0), (BLOOM, 1.0), (MIX, 0.55)] },
        Preset { name: "Soft Bloom", values: &[(DECAY, 5.0), (BLOOM, 0.8), (MIX, 0.35)] },
        Preset { name: "Dark Fog", values: &[(TONE, 2000.0), (DECAY, 12.0)] },
        Preset { name: "Bright Haze", values: &[(TONE, 10000.0), (MOD, 0.7)] },
        Preset { name: "Behind the Band", values: &[(DECAY, 6.0), (MIX, 0.25), (BLOOM, 0.4)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "CLOUD",
        subtitle: ("PEQUOD ORIGINAL · AMBIENT REVERB", "VERY LONG · DIFFUSE · BLOOMING"),
        accent: ui::TEAL,
        size: (W, H),
        items: vec![
            Item::Knob { p: DECAY, x: 70.0, y: 190.0, r: 32.0, label: "DECAY", tip: "Reverb time: up to 30 s.", active: None },
            Item::Knob { p: BLOOM, x: 170.0, y: 190.0, r: 32.0, label: "BLOOM", tip: "How much each note is smeared so it swells into the tail.", active: None },
            Item::Knob { p: MOD, x: 270.0, y: 190.0, r: 32.0, label: "MOD", tip: "Slow movement in the tail.", active: None },
            Item::Knob { p: TONE, x: 370.0, y: 190.0, r: 32.0, label: "TONE", tip: "High cut.", active: None },
            Item::Knob { p: MIX, x: 470.0, y: 190.0, r: 32.0, label: "MIX", tip: "Reverb level.", active: None },
            Item::Knob { p: OUTPUT, x: 570.0, y: 190.0, r: 32.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Cloud", class_id: *b"PqdCloudAmbient1", vendor: "PEQUOD Pedals",
    categories: [Fx, Reverb]
);

#[cfg(test)]
#[allow(dead_code)]
pub mod tests {
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
    fn long_and_finite() {
        let mut x = vec![0.0f32; 48000 * 4];
        for k in 0..4800 {
            x[k] = (0.3 * (2.0 * std::f64::consts::PI * 330.0 * k as f64 / 48000.0).sin()) as f32;
        }
        let mut input = vec![x.clone(), x];
        run(&with(defaults(), &[(MIX, 1.0)]), &mut input);
        let tail = db(rms(&input[0][144000..]));
        println!("tail 3-4 s after the note: {tail:.1} dB");
        assert!(tail > -50.0 && input[0].iter().all(|v| v.is_finite() && v.abs() < 4.0));
    }

}
