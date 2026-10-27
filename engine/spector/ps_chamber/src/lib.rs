//! Gold Star Chamber: the cement-plastered echo chambers behind Gold Star's control room, fed by a small speaker and picked up by a ribbon mic. STEAL reproduces Larry Levine's observation that raising a channel's echo send lowered its direct level into the desk.

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
use vintage::verb::Fdn;

pub const W: u32 = 640;
pub const H: u32 = 432;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Chamber { #[name = "Chamber One"] One, #[name = "Chamber Two"] Two }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Steal { #[name = "Off"] Off, #[name = "On"] On }
// size (FDN scale) and decay shape per chamber (estimated: hard plaster, long and bright-ish mid, highs lost to air)
const SIZE: [f64; 2] = [1.05, 1.25];


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "chamber"]
    pub chamber: EnumParam<Chamber>,
    #[id = "send"]
    pub send: FloatParam,
    #[id = "steal"]
    pub steal: EnumParam<Steal>,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "speaker"]
    pub speaker: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "lowcut"]
    pub low_cut: FloatParam,
    #[id = "return"]
    pub ret: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            chamber: EnumParam::new("Chamber", Chamber::One),
            send: pct("Echo Send", 0.45),
            steal: EnumParam::new("Send Steals Direct", Steal::On),
            decay: secs("Decay", 2.6, 1.0, 5.0),
            speaker: pct("Speaker Drive", 0.3),
            tone: hz("Ribbon Tone", 7000.0, 2500.0, 14000.0),
            low_cut: hz("Return Low Cut", 150.0, 40.0, 600.0),
            ret: db("Return", 0.0, -24.0, 6.0),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const CHAMBER: usize = 0;
pub const SEND: usize = 1;
pub const STEAL: usize = 2;
pub const DECAY: usize = 3;
pub const SPEAKER: usize = 4;
pub const TONE: usize = 5;
pub const LOWCUT: usize = 6;
pub const RETURN: usize = 7;
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
            self.chamber.as_ptr(),
            self.send.as_ptr(),
            self.steal.as_ptr(),
            self.decay.as_ptr(),
            self.speaker.as_ptr(),
            self.tone.as_ptr(),
            self.low_cut.as_ptr(),
            self.ret.as_ptr(),
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
    fdn: Fdn,
    key: (usize, f64),
    spk_hp: Biquad,
    spk_lp: Biquad,
    spk_pk: Biquad,
    mic: [Biquad; 2],
    lc: [Biquad; 2],
    tone: f64,
    lowcut: f64,
    send: Smooth,
    steal: bool,
    drive: f64,
    ret: Smooth,
    out: Smooth,
}

impl Dsp {
    fn configure(&mut self, ch: usize, decay: f64) {
        self.fdn.configure(SIZE[ch], [decay * 1.1, decay, decay * 0.5], 3500.0);
        self.key = (ch, decay);
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut spk_hp = Biquad::identity();
        spk_hp.set_highpass(fs, 130.0, 0.8);
        let mut spk_lp = Biquad::identity();
        spk_lp.set_lowpass(fs, 6500.0, 0.7);
        let mut spk_pk = Biquad::identity();
        spk_pk.set_peak(fs, 2400.0, 1.2, 3.0);
        let mut d = Self {
            fs,
            fdn: Fdn::new(fs, 1.4),
            key: (9, 0.0),
            spk_hp,
            spk_lp,
            spk_pk,
            mic: [Biquad::identity(); 2],
            lc: [Biquad::identity(); 2],
            tone: 0.0,
            lowcut: 0.0,
            send: Smooth::new(fs, 20.0, 0.45),
            steal: true,
            drive: 0.3,
            ret: Smooth::new(fs, 20.0, 1.0),
            out: Smooth::new(fs, 20.0, 1.0),
        };
        d.configure(0, 2.6);
        d
    }
    fn reset(&mut self) {
        self.fdn.reset();
        for b in [&mut self.spk_hp, &mut self.spk_lp, &mut self.spk_pk] {
            b.reset();
        }
        for c in 0..2 {
            self.mic[c].reset();
            self.lc[c].reset();
        }
    }
    fn set_params(&mut self, v: &[f32]) {
        let ch = (v[CHAMBER].round() as usize).min(1);
        let decay = v[DECAY] as f64;
        if ch != self.key.0 || (decay - self.key.1).abs() > 1e-3 {
            self.configure(ch, decay);
        }
        let t = v[TONE] as f64;
        if (t - self.tone).abs() > 0.5 {
            self.tone = t;
            for b in &mut self.mic {
                b.set_lowpass(self.fs, t, 0.6);
            }
        }
        let lc = v[LOWCUT] as f64;
        if (lc - self.lowcut).abs() > 0.5 {
            self.lowcut = lc;
            for b in &mut self.lc {
                b.set_highpass(self.fs, lc, 0.7);
            }
        }
        self.send.set(v[SEND] as f64);
        self.steal = v[STEAL] > 0.5;
        self.drive = v[SPEAKER] as f64;
        self.ret.set(db_to_gain(v[RETURN] as f64));
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let mut peak = 0.0f64;
        let dg = 1.0 + 6.0 * self.drive;
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            let s = self.send.next();
            // small chamber speaker: band-limited, breaks up when pushed
            let sp = self.spk_pk.process(self.spk_lp.process(self.spk_hp.process(x * s)));
            let sp = (sp * dg).tanh() / dg;
            let (wl, wr) = { let dif = self.fdn.diffuse(sp, 3); self.fdn.process(dif) };
            let rg = self.ret.next() * 2.2;
            let wet = [self.lc[0].process(self.mic[0].process(wl)) * rg, self.lc[1].process(self.mic[1].process(wr)) * rg];
            // Levine: more echo send, less direct into the desk
            let direct = if self.steal { 1.0 - 0.6 * s } else { 1.0 };
            let o = self.out.next();
            for (ci, c) in chans.iter_mut().enumerate() {
                let w = if nch == 1 { 0.5 * (wet[0] + wet[1]) } else { wet[ci.min(1)] };
                let y = (c[k] as f64 * direct + w) * o;
                peak = peak.max(y.abs());
                c[k] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Be My Baby Chamber", values: &[(SEND, 0.5), (DECAY, 2.6)] },
        Preset { name: "Whole Band Into the Chamber", values: &[(SEND, 0.7), (DECAY, 2.8), (SPEAKER, 0.45)] },
        Preset { name: "Vocal Echo (Ronnie)", values: &[(SEND, 0.35), (DECAY, 2.2), (TONE, 8500.0), (LOWCUT, 220.0)] },
        Preset { name: "Drums Far Away", values: &[(SEND, 0.8), (DECAY, 3.0), (RETURN, 2.0)] },
        Preset { name: "Chamber Two, Long", values: &[(CHAMBER, 1.0), (SEND, 0.5), (DECAY, 3.6)] },
        Preset { name: "Strings Halo", values: &[(CHAMBER, 1.0), (SEND, 0.4), (DECAY, 3.2), (TONE, 9000.0), (LOWCUT, 250.0)] },
        Preset { name: "Send Only (no steal)", values: &[(STEAL, 0.0), (SEND, 0.4)] },
        Preset { name: "River Deep Overload", values: &[(SEND, 0.9), (DECAY, 3.2), (SPEAKER, 0.8), (RETURN, 3.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "GOLD STAR CHAMBER",
        subtitle: ("CEMENT ECHO CHAMBERS, GOLD STAR", "SPEAKER · RIBBON MIC · HOLLYWOOD"),
        accent: ui::PINE,
        size: (W, H),
        items: vec![

            Item::Selector { p: CHAMBER, x: 80.0, y: 160.0, label: "CHAMBER", positions: &["ONE", "TWO"], tip: "Gold Star had two cement-plastered chambers behind the control room, reached through a crawl hole. Chamber Two here is the slightly larger, longer one (estimated)." },
            Item::Knob { p: SEND, x: 200.0, y: 160.0, r: 40.0, label: "ECHO SEND", tip: "How much goes to the chamber speaker.", active: None },
            Item::Knob { p: DECAY, x: 330.0, y: 160.0, r: 40.0, label: "DECAY", tip: "Chamber decay time (mid frequencies). Estimated: no survey of the chambers was published.", active: None },
            Item::Switch { p: STEAL, x: 460.0, y: 160.0, labels: ["OFF", "STEAL"], tip: "Levine: sending more to the chamber lowered the channel's direct level into the desk, so the sound moved back into the echo. On by default." },
            Item::Lamp { m: 0, x: 540.0, y: 160.0, label: "LEVEL", lo: 0.05, hi: 0.9 },
            Item::Divider { y: 264.0 },
            Item::Knob { p: SPEAKER, x: 80.0, y: 330.0, r: 22.0, label: "SPEAKER DRIVE", tip: "How hard the small chamber speaker is driven.", active: None },
            Item::Knob { p: TONE, x: 180.0, y: 330.0, r: 22.0, label: "RIBBON TONE", tip: "Top-end roll-off of the ribbon mic in the chamber.", active: None },
            Item::Knob { p: LOWCUT, x: 280.0, y: 330.0, r: 22.0, label: "LOW CUT", tip: "High-pass on the chamber return.", active: None },
            Item::Knob { p: RETURN, x: 380.0, y: 330.0, r: 22.0, label: "RETURN", tip: "Chamber return level.", active: None },
            Item::Knob { p: OUTPUT, x: 540.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Gold Star Chamber", class_id: *b"PSGoldStarChamPQ", vendor: "PEQUOD Spector",
    categories: [Fx, Reverb]
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
    fn decays_near_set_time_and_steals_direct() {
        for decay in [1.8f32, 3.2] {
            let p = with(defaults(), &[(DECAY, decay), (SEND, 1.0), (STEAL, 0.0), (SPEAKER, 0.0)]);
            let mut x = vec![0.0f32; 48000 * 5];
            x[0] = 0.3;
            let mut input = vec![x];
            run(&p, &mut input);
            let h: Vec<f64> = input[0][10..].iter().map(|v| *v as f64).collect();
            let t = vintage::verb::t30(&h, 48000.0, 800.0);
            println!("decay set {decay} measured {t:.2}");
            assert!((t / decay as f64 - 1.0).abs() < 0.3);
        }
        let lvl = |steal: f32| {
            let p = with(defaults(), &[(SEND, 0.8), (STEAL, steal), (RETURN, -24.0)]);
            let mut input = vec![sine(1000.0, 0.1, 24000)];
            run(&p, &mut input);
            rms(&input[0][12000..])
        };
        let (on, off) = (lvl(1.0), lvl(0.0));
        println!("direct with steal {:.1} dB vs without", db(on / off));
        assert!(db(on / off) < -3.0);
    }

}
