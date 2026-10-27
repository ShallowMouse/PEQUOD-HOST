//! Red Special: voices a guitar DI toward Brian May's home-built Red Special: three Burns Tri-Sonic single coils at their own places along the 24-inch scale, each with an on/off and a phase switch, wired in SERIES (May's way: louder and fatter) or parallel. Each pickup hears the string as a single coil at that position does (more fundamental at the neck, less at the bridge); out-of-phase pairs cancel what they share, giving May's nasal, vocal tones. SIXPENCE adds the scrape of his coin pick. Curves estimated.

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


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Source { #[name = "Single coils"] Single, #[name = "Humbuckers"] Humbucker }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum OnOff { #[name = "Off"] Off, #[name = "On"] On }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Phase { #[name = "In"] In, #[name = "Reversed"] Out }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Wiring { #[name = "Series (stock)"] Series, #[name = "Parallel"] Parallel }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "neck"]
    pub neck: EnumParam<OnOff>,
    #[id = "mid"]
    pub mid: EnumParam<OnOff>,
    #[id = "bridge"]
    pub bridge: EnumParam<OnOff>,
    #[id = "pn"]
    pub pn: EnumParam<Phase>,
    #[id = "pm"]
    pub pm: EnumParam<Phase>,
    #[id = "pb"]
    pub pb: EnumParam<Phase>,
    #[id = "wiring"]
    pub wiring: EnumParam<Wiring>,
    #[id = "source"]
    pub source: EnumParam<Source>,
    #[id = "sixpence"]
    pub sixpence: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            neck: EnumParam::new("Neck", OnOff::Off),
            mid: EnumParam::new("Middle", OnOff::On),
            bridge: EnumParam::new("Bridge", OnOff::On),
            pn: EnumParam::new("Neck Phase", Phase::In),
            pm: EnumParam::new("Middle Phase", Phase::In),
            pb: EnumParam::new("Bridge Phase", Phase::In),
            wiring: EnumParam::new("Wiring", Wiring::Series),
            source: EnumParam::new("Your Guitar", Source::Single),
            sixpence: pct("Sixpence", 0.5),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const NECK: usize = 0;
pub const MID: usize = 1;
pub const BRIDGE: usize = 2;
pub const PN: usize = 3;
pub const PM: usize = 4;
pub const PB: usize = 5;
pub const WIRING: usize = 6;
pub const SOURCE: usize = 7;
pub const SIXPENCE: usize = 8;
pub const OUTPUT: usize = 9;
pub const NPARAMS: usize = 10;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.neck.as_ptr(),
            self.mid.as_ptr(),
            self.bridge.as_ptr(),
            self.pn.as_ptr(),
            self.pm.as_ptr(),
            self.pb.as_ptr(),
            self.wiring.as_ptr(),
            self.source.as_ptr(),
            self.sixpence.as_ptr(),
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

/// Positions from the bridge as a fraction of the 24" scale (estimated).
pub const POS: [f64; 3] = [0.26, 0.155, 0.062];
/// Single coil inductance (Tri-Sonic, ~1.9 H) and the guitar lead and pots (~600 pF).
const L_PU: f64 = 1.9;
const C_LEAD: f64 = 600e-12;

struct Chan {
    src: [Biquad; 3],
    pos: [Biquad; 3],
    ap: [Biquad; 3],
    res: Biquad,
    tilt: Biquad,
    fast: f64,
    slow: f64,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ch: Vec<Chan>,
    gains: [f64; 3],
    key: [i32; 8],
    out: Smooth,
}

impl Dsp {
    /// Resonant peak of the switched-in coils with the lead's capacitance.
    pub fn res_freq(n: usize, series: bool) -> f64 {
        let n = n.max(1) as f64;
        let l = if series { L_PU * n } else { L_PU / n };
        1.0 / (2.0 * std::f64::consts::PI * (l * C_LEAD).sqrt())
    }
    fn design(&mut self) {
        let fs = self.fs;
        let on = [self.v[NECK] > 0.5, self.v[MID] > 0.5, self.v[BRIDGE] > 0.5];
        let ph = [self.v[PN] > 0.5, self.v[PM] > 0.5, self.v[PB] > 0.5];
        let series = self.v[WIRING] < 0.5;
        let n = on.iter().filter(|b| **b).count();
        // series coils add their voltages; parallel coils average them
        let norm = if series { 1.0 } else { 1.0 / n.max(1) as f64 };
        for i in 0..3 {
            self.gains[i] = if on[i] { if ph[i] { -norm } else { norm } } else { 0.0 };
        }
        let fr = Dsp::res_freq(n, series).min(0.45 * fs);
        let typical_f0 = 196.0;
        for c in self.ch.iter_mut() {
            // your guitar: take its own pickup (assumed near the middle) back toward the bare string
            let f_src = typical_f0 / (2.0 * 0.12);
            c.src[0].set_analog(fs, [0.0, 1.0, 2.0 * std::f64::consts::PI * f_src], [0.0, 1.0, 2.0 * std::f64::consts::PI * 70.0]);
            if self.v[SOURCE] > 0.5 {
                c.src[1].set_peak(fs, 5000.0, 1.0, 3.0);
                c.src[2].set_peak(fs, 2400.0, 1.2, -2.0);
            } else {
                c.src[1].set_peak(fs, 4500.0, 1.4, -3.0);
                c.src[2] = Biquad::identity();
            }
            for i in 0..3 {
                // a single coil at fraction p hears harmonic n as sin(n pi p): rising until n ~ 1/(2p)
                let fi = typical_f0 / (2.0 * POS[i]);
                c.pos[i].set_analog(fs, [0.0, 1.0, 0.0], [0.0, 1.0, 2.0 * std::f64::consts::PI * fi]);
                // and the pickups see the string's waves at different times: their tops don't line up
                let fa = [1200.0, 2600.0, 5200.0][i];
                let w = 2.0 * std::f64::consts::PI * fa;
                let q = 0.7;
                c.ap[i].set_analog(fs, [1.0, -w / q, w * w], [1.0, w / q, w * w]);
            }
            c.res.set_lowpass(fs, fr, if series { 2.2 } else { 1.6 });
            c.tilt.set_high_shelf(fs, 2500.0, 0.7, 6.0);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mk = || Chan { src: [Biquad::identity(); 3], pos: [Biquad::identity(); 3], ap: [Biquad::identity(); 3], res: Biquad::identity(), tilt: Biquad::identity(), fast: 0.0, slow: 0.0 };
        let mut d = Self { fs, v, ch: (0..ch.max(2)).map(|_| mk()).collect(), gains: [0.0; 3], key: [-1; 8], out: Smooth::new(fs, 20.0, 1.0) };
        d.design();
        d
    }
    fn reset(&mut self) {
        for c in self.ch.iter_mut() {
            c.src.iter_mut().chain(c.pos.iter_mut()).chain(c.ap.iter_mut()).for_each(|b| b.reset());
            c.res.reset();
            c.tilt.reset();
            c.fast = 0.0;
            c.slow = 0.0;
        }
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = [v[NECK] as i32, v[MID] as i32, v[BRIDGE] as i32, v[PN] as i32, v[PM] as i32, v[PB] as i32, v[WIRING] as i32, v[SOURCE] as i32];
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let six = self.v[SIXPENCE] as f64;
        let cf = coeff(self.fs, 2.0);
        let cs = coeff(self.fs, 40.0);
        let g = self.gains;
        let last = self.ch.len() - 1;
        for k in 0..n {
            let o = self.out.next();
            for (ci, chn) in chans.iter_mut().enumerate() {
                let c = &mut self.ch[ci.min(last)];
                let x = cascade(&mut c.src, chn[k] as f64);
                let mut y = 0.0;
                for i in 0..3 {
                    let p = c.ap[i].process(c.pos[i].process(x));
                    y += g[i] * p;
                }
                let y = c.res.process(y);
                // sixpence: the coin's edge scrapes the string on every attack
                let a = y.abs();
                c.fast = a + cf * (c.fast - a);
                c.slow = a + cs * (c.slow - a);
                let tr = ((c.fast - c.slow) / (c.slow + 1e-4)).clamp(0.0, 1.0);
                let bright = c.tilt.process(y) - y;
                chn[k] = ((y + bright * six * tr) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Bridge + Middle (the classic lead)", values: &[] },
        Preset { name: "Bridge Only", values: &[(MID, 0.0)] },
        Preset { name: "Neck Only", values: &[(NECK, 1.0), (MID, 0.0), (BRIDGE, 0.0)] },
        Preset { name: "Neck + Bridge", values: &[(NECK, 1.0), (MID, 0.0), (BRIDGE, 1.0)] },
        Preset { name: "All Three (fat series)", values: &[(NECK, 1.0), (MID, 1.0), (BRIDGE, 1.0)] },
        Preset { name: "Neck + Middle, Out of Phase", values: &[(NECK, 1.0), (MID, 1.0), (BRIDGE, 0.0), (PM, 1.0)] },
        Preset { name: "Middle + Bridge, Out of Phase", values: &[(PB, 1.0)] },
        Preset { name: "Neck + Bridge, Out of Phase", values: &[(NECK, 1.0), (MID, 0.0), (PB, 1.0)] },
        Preset { name: "Parallel Bridge + Middle (cleaner)", values: &[(WIRING, 1.0)] },
        Preset { name: "No Coin (softer attack)", values: &[(SIXPENCE, 0.0)] },
        Preset { name: "From a Les Paul", values: &[(SOURCE, 1.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "RED SPECIAL",
        subtitle: ("BRIAN MAY'S HOME-BUILT GUITAR FROM YOUR DI", "3 TRI-SONICS · ON/OFF + PHASE · SERIES"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![

            Item::Text { x: 115.0, y: 92.0, text: "NECK", size: 12.0, align: Align2::CENTER_CENTER },
            Item::Text { x: 265.0, y: 92.0, text: "MIDDLE", size: 12.0, align: Align2::CENTER_CENTER },
            Item::Text { x: 415.0, y: 92.0, text: "BRIDGE", size: 12.0, align: Align2::CENTER_CENTER },
            Item::Switch { p: NECK, x: 115.0, y: 130.0, labels: ["OFF", "ON"], tip: "Neck pickup on or off." },
            Item::Switch { p: MID, x: 265.0, y: 130.0, labels: ["OFF", "ON"], tip: "Middle pickup on or off." },
            Item::Switch { p: BRIDGE, x: 415.0, y: 130.0, labels: ["OFF", "ON"], tip: "Bridge pickup on or off." },
            Item::Switch { p: PN, x: 115.0, y: 180.0, labels: ["IN", "REV"], tip: "Neck pickup phase. Reversing one of two pickups gives the out-of-phase sound (reversing both changes nothing)." },
            Item::Switch { p: PM, x: 265.0, y: 180.0, labels: ["IN", "REV"], tip: "Middle pickup phase." },
            Item::Switch { p: PB, x: 415.0, y: 180.0, labels: ["IN", "REV"], tip: "Bridge pickup phase." },
            Item::Text { x: 265.0, y: 214.0, text: "ON / OFF  ·  PHASE", size: 10.0, align: Align2::CENTER_CENTER },
            Item::Selector { p: WIRING, x: 555.0, y: 150.0, label: "WIRING", positions: &["SERIES", "PARALLEL"], tip: "SERIES: May's wiring, the coils add up into one big pickup (louder, fatter, the resonance lower). PARALLEL: the common mod, more like a Strat." },
            Item::Divider { y: 240.0 },
            Item::Selector { p: SOURCE, x: 90.0, y: 305.0, label: "YOUR GUITAR", positions: &["SINGLE", "HUMBUCK"], tip: "What you are actually playing, so its own pickup can be taken back toward the bare string first." },
            Item::Knob { p: SIXPENCE, x: 290.0, y: 305.0, r: 30.0, label: "SIXPENCE", tip: "May plays with an old sixpence coin: the serrated edge adds a scrape to each attack.", active: None },
            Item::Knob { p: OUTPUT, x: 500.0, y: 305.0, r: 30.0, label: "OUTPUT", tip: "Output level. Note that with no pickup switched on, the guitar is silent (as the real one is).", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Red Special", class_id: *b"BMyRedSpecialPQ1", vendor: "PEQUOD Brian May",
    categories: [Fx, Filter]
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

    use std::f64::consts::PI;
    fn sine_amp(f: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n).map(|k| (amp * (2.0 * PI * f * k as f64 / 48000.0).sin()) as f32).collect()
    }
    fn goertzel(x: &[f32], f: f64) -> f64 {
        let w = 2.0 * PI * f / 48000.0;
        let (mut s1, mut s2) = (0.0, 0.0);
        for v in x {
            let s = *v as f64 + 2.0 * w.cos() * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        ((s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2).max(0.0)).sqrt() / x.len() as f64
    }
    fn go(p: &[f32], x: &[f32]) -> Vec<f32> {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        input.remove(0)
    }
    fn go2(p: &[f32], x: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        let r = input.pop().unwrap();
        (input.pop().unwrap(), r)
    }

    fn lvl(set: &[(usize, f32)], f: f64) -> f64 {
        let p = with(defaults(), &[&[(SIXPENCE, 0.0f32)][..], set].concat());
        let y = go(&p, &sine_amp(f, 0.1, 48000));
        db(rms(&y[24000..]))
    }
    #[test]
    fn neck_has_more_fundamental_than_bridge() {
        let n = lvl(&[(NECK, 1.0), (MID, 0.0), (BRIDGE, 0.0)], 150.0);
        let b = lvl(&[(NECK, 0.0), (MID, 0.0), (BRIDGE, 1.0)], 150.0);
        println!("150 Hz: neck {n:.1}, bridge {b:.1}");
        assert!(n - b > 8.0);
    }
    #[test]
    fn out_of_phase_cancels_the_lows() {
        let a = lvl(&[(NECK, 1.0), (MID, 1.0), (BRIDGE, 0.0)], 200.0);
        let b = lvl(&[(NECK, 1.0), (MID, 1.0), (BRIDGE, 0.0), (PM, 1.0)], 200.0);
        let h = lvl(&[(NECK, 1.0), (MID, 1.0), (BRIDGE, 0.0), (PM, 1.0)], 2500.0) - lvl(&[(NECK, 1.0), (MID, 1.0), (BRIDGE, 0.0)], 2500.0);
        println!("200 Hz neck+middle: in phase {a:.1}, reversed {b:.1}; 2.5 kHz change {h:.1}");
        assert!(a - b > 8.0 && h > -8.0);
    }
    #[test]
    fn series_rolls_off_sooner_and_is_louder() {
        let s = Dsp::res_freq(2, true);
        let p = Dsp::res_freq(2, false);
        let ls = lvl(&[], 400.0);
        let lp = lvl(&[(WIRING, 1.0)], 400.0);
        println!("two coils: series peak {s:.0} Hz, parallel {p:.0} Hz; 400 Hz series {ls:.1} vs parallel {lp:.1}");
        assert!(s < p && ls - lp > 4.0);
    }

}
