//! 6120 Voice: voices a guitar DI toward Brian Setzer's late-50s Gretsch 6120 hollowbody with Filter'Tron-style pickups (his are TV Jones). Undoes the character of your own pickups, then adds the Filter'Tron's glassy, scooped twang, the pickup selector, the Gretsch three-way tone switch and the ring of a big hollow body. Curves estimated from published descriptions and measurements of these pickups.

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
pub enum Source { #[name = "Single coils"] Single, #[name = "Humbuckers"] Humbucker, #[name = "Already a hollowbody"] Flat }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Pickup { #[name = "Neck"] Neck, #[name = "Both"] Both, #[name = "Bridge"] Bridge }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum ToneSw { #[name = "Bright"] Bright, #[name = "Mid"] Mid, #[name = "Dark"] Dark }


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "source"]
    pub source: EnumParam<Source>,
    #[id = "pickup"]
    pub pickup: EnumParam<Pickup>,
    #[id = "tonesw"]
    pub tonesw: EnumParam<ToneSw>,
    #[id = "twang"]
    pub twang: FloatParam,
    #[id = "body"]
    pub body: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            source: EnumParam::new("Your Guitar", Source::Single),
            pickup: EnumParam::new("Pickup", Pickup::Bridge),
            tonesw: EnumParam::new("Tone Switch", ToneSw::Bright),
            twang: pct("Twang", 0.6),
            body: pct("Body", 0.4),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const SOURCE: usize = 0;
pub const PICKUP: usize = 1;
pub const TONESW: usize = 2;
pub const TWANG: usize = 3;
pub const BODY: usize = 4;
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
            self.source.as_ptr(),
            self.pickup.as_ptr(),
            self.tonesw.as_ptr(),
            self.twang.as_ptr(),
            self.body.as_ptr(),
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
    eq: [[Biquad; 10]; 2],
    res: [[Biquad; 2]; 2],
    key: (usize, usize, usize, i32),
    body: Smooth,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let src = (self.v[SOURCE].round() as usize).min(2);
        let pu = (self.v[PICKUP].round() as usize).min(2);
        let ts = (self.v[TONESW].round() as usize).min(2);
        let tw = self.v[TWANG] as f64;
        let mut b = [Biquad::identity(); 10];
        // 1) undo your pickups
        match src {
            0 => {
                b[0].set_peak(fs, 3600.0, 1.2, -2.5);
                b[1].set_low_shelf(fs, 220.0, 0.7, 1.0);
            }
            1 => {
                b[0].set_peak(fs, 4300.0, 0.9, 3.0);
                b[1].set_peak(fs, 1300.0, 0.9, -2.0);
            }
            _ => {}
        }
        // 2) pickup selector (position along the string)
        match pu {
            0 => {
                b[2].set_low_shelf(fs, 260.0, 0.7, 2.0);
                b[3].set_peak(fs, 2500.0, 1.0, -1.5);
            }
            1 => {
                b[2].set_peak(fs, 900.0, 0.8, -2.5);
                b[3].set_peak(fs, 3000.0, 1.0, 1.0);
            }
            _ => {
                b[2].set_low_shelf(fs, 300.0, 0.7, -3.5);
                b[3].set_peak(fs, 2200.0, 1.0, 2.5);
            }
        }
        // 3) Filter'Tron twang: glassy top, scooped low mids
        b[4].set_peak(fs, 5000.0, 1.0, 4.5 * tw);
        b[5].set_peak(fs, 450.0, 0.9, -2.5 * tw);
        // 4) the three-way tone switch (fixed caps to ground)
        match ts {
            0 => {}
            1 => b[6].set_lowpass(fs, 3200.0, 0.7),
            _ => {
                b[6].set_lowpass(fs, 1600.0, 0.7);
                b[7].set_low_shelf(fs, 160.0, 0.7, 2.0);
            }
        }
        // 5) the hollow body's broad air
        b[8].set_peak(fs, 180.0, 1.0, 1.5);
        b[9].set_high_shelf(fs, 9000.0, 0.7, -2.0);
        self.eq = [b, b];
        let mut r = [Biquad::identity(); 2];
        r[0].set_bandpass(fs, 110.0, 7.0);
        r[1].set_bandpass(fs, 225.0, 6.0);
        self.res = [r, r];
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self {
            fs, v, eq: [[Biquad::identity(); 10]; 2], res: [[Biquad::identity(); 2]; 2], key: (9, 9, 9, -1),
            body: Smooth::new(fs, 20.0, 0.4), out: Smooth::new(fs, 20.0, 1.0),
        };
        d.design();
        d
    }
    fn reset(&mut self) {
        for c in 0..2 {
            self.eq[c].iter_mut().for_each(|b| b.reset());
            self.res[c].iter_mut().for_each(|b| b.reset());
        }
        self.body.snap(self.v[BODY] as f64);
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = (v[SOURCE].round() as usize, v[PICKUP].round() as usize, v[TONESW].round() as usize, (v[TWANG] * 200.0) as i32);
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.body.set(v[BODY] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        for k in 0..n {
            let body = self.body.next();
            let o = self.out.next();
            for (ci, ch) in chans.iter_mut().enumerate() {
                let c = ci.min(1);
                let x = ch[k] as f64;
                let y = cascade(&mut self.eq[c], x);
                // the top and back ringing at their main resonances
                let ring = self.res[c][0].process(x) * 1.6 + self.res[c][1].process(x) * 0.9;
                ch[k] = ((y + body * ring) * o) as f32;
            }
        }
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Rockabilly Bridge (Rock This Town)", values: &[] },
        Preset { name: "Both Pickups, Bright", values: &[(PICKUP, 1.0), (TWANG, 0.7)] },
        Preset { name: "Neck Pickup, Jazzy Swing", values: &[(PICKUP, 0.0), (TONESW, 1.0), (TWANG, 0.3), (BODY, 0.5)] },
        Preset { name: "Dark Tone Switch Rhythm", values: &[(PICKUP, 1.0), (TONESW, 2.0), (BODY, 0.5)] },
        Preset { name: "Max Twang", values: &[(PICKUP, 2.0), (TWANG, 1.0), (BODY, 0.3)] },
        Preset { name: "From a Les Paul", values: &[(SOURCE, 1.0), (PICKUP, 2.0), (TWANG, 0.7)] },
        Preset { name: "From a Les Paul, Neck", values: &[(SOURCE, 1.0), (PICKUP, 0.0), (TWANG, 0.5)] },
        Preset { name: "Big Hollow Body", values: &[(PICKUP, 1.0), (BODY, 0.9), (TWANG, 0.5)] },
        Preset { name: "Already a Gretsch (body only)", values: &[(SOURCE, 2.0), (TWANG, 0.0), (BODY, 0.4)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "6120 VOICE",
        subtitle: ("A GRETSCH 6120 FROM YOUR DI", "FILTER'TRONS · TONE SWITCH · HOLLOW BODY"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![

            Item::RadioList { p: SOURCE, x: 40.0, y: 100.0, w: 160.0, row_h: 22.0, names: &["Single coils", "Humbuckers", "Already a hollowbody"], tip: "What you are actually playing, so its own pickup character can be taken out first." },
            Item::Selector { p: PICKUP, x: 300.0, y: 150.0, label: "PICKUP", positions: &["NECK", "BOTH", "BRIDGE"], tip: "The 6120's pickup selector: neck (round, jazzy), both (scooped, the classic Gretsch rhythm) or bridge (Setzer's cutting lead)." },
            Item::Selector { p: TONESW, x: 450.0, y: 150.0, label: "TONE SWITCH", positions: &["BRIGHT", "MID", "DARK"], tip: "The Gretsch three-way tone switch: fixed capacitors that cut the top in two steps." },
            Item::Text { x: 120.0, y: 196.0, text: "YOUR GUITAR", size: 11.0, align: Align2::CENTER_CENTER },
            Item::Divider { y: 236.0 },
            Item::Knob { p: TWANG, x: 140.0, y: 310.0, r: 30.0, label: "TWANG", tip: "The Filter'Tron character: glassy top end and scooped low mids.", active: None },
            Item::Knob { p: BODY, x: 320.0, y: 310.0, r: 30.0, label: "BODY", tip: "The ring of the big hollow body (resonances around 110 and 225 Hz) under every note.", active: None },
            Item::Knob { p: OUTPUT, x: 500.0, y: 310.0, r: 30.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "6120 Voice", class_id: *b"BSz6120VoicePQD1", vendor: "PEQUOD Brian Setzer",
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
    fn tone(f: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n).map(|k| {
            let t = k as f64 / 48000.0;
            (1..=6).map(|h| amp / h as f64 * (2.0 * PI * f * h as f64 * t).sin()).sum::<f64>() as f32
        }).collect()
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
    fn noise(n: usize, amp: f64) -> Vec<f32> {
        let mut nz = vintage::dsp::Noise::new(11);
        (0..n).map(|_| (nz.next() * amp) as f32).collect()
    }

    fn level(set: &[(usize, f32)], f: f64) -> f64 {
        let p = with(defaults(), set);
        let x: Vec<f32> = (0..48000).map(|k| (0.1 * (2.0 * PI * f * k as f64 / 48000.0).sin()) as f32).collect();
        let y = go(&p, &x);
        db(rms(&y[24000..]))
    }
    #[test]
    fn tone_switch_darkens() {
        let b = level(&[(TONESW, 0.0), (BODY, 0.0)], 4000.0);
        let d = level(&[(TONESW, 2.0), (BODY, 0.0)], 4000.0);
        println!("4 kHz: bright {b:.1} dB, dark {d:.1} dB");
        assert!(b - d > 10.0);
    }
    #[test]
    fn bridge_is_thinner_than_neck() {
        let n = level(&[(PICKUP, 0.0), (BODY, 0.0)], 150.0);
        let b = level(&[(PICKUP, 2.0), (BODY, 0.0)], 150.0);
        println!("150 Hz: neck {n:.1}, bridge {b:.1}");
        assert!(n - b > 3.0);
    }
    #[test]
    fn body_rings() {
        let a = level(&[(BODY, 0.0)], 110.0);
        let b = level(&[(BODY, 1.0)], 110.0);
        println!("110 Hz: body 0 {a:.1}, body 1 {b:.1}");
        assert!(b - a > 2.0);
    }

}
