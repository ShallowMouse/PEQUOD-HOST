//! Foxton Bass: voices a bass DI toward Bruce Foxton's sound in The Jam - his Rickenbacker 4001 (the metallic,
//! clanking treble of In the City and Going Underground) or the Fender Precision he turned to for the rounder,
//! Motown-inspired parts (Town Called Malice). Played hard with a pick on bright Rotosound roundwounds, into a
//! 100-watt Marshall valve bass head. The 4001's Rick-O-Sound output can split the pickups: the neck to a
//! bassy amp on the left, the bridge to a bright one on the right. On the records the bass was taken DI and
//! from the amp together. Curves estimated from published descriptions and measurements of these basses.

#![allow(clippy::excessive_precision)]
use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
#[allow(unused_imports)]
use vintage::dsp::*;
#[allow(unused_imports)]
use vintage::fx::*;
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
pub enum Bass { #[name = "Rickenbacker 4001"] Rick, #[name = "Precision"] Precision }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Pickup { #[name = "Neck"] Neck, #[name = "Both"] Both, #[name = "Bridge"] Bridge }

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "bass"]
    pub bass: EnumParam<Bass>,
    #[id = "pickup"]
    pub pickup: EnumParam<Pickup>,
    #[id = "rounds"]
    pub rounds: FloatParam,
    #[id = "pick"]
    pub pick: FloatParam,
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "amp"]
    pub amp: FloatParam,
    #[id = "stereo"]
    pub stereo: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            bass: EnumParam::new("Bass", Bass::Rick),
            pickup: EnumParam::new("Pickup", Pickup::Both),
            rounds: pct("Roundwounds", 0.7),
            pick: pct("Pick", 0.6),
            drive: pct("Amp Drive", 0.35),
            amp: pct("DI / Amp", 0.5),
            stereo: pct("Rick-O-Sound", 0.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const BASS: usize = 0;
pub const PICKUP: usize = 1;
pub const ROUNDS: usize = 2;
pub const PICK: usize = 3;
pub const DRIVE: usize = 4;
pub const AMP: usize = 5;
pub const STEREO: usize = 6;
pub const OUTPUT: usize = 7;
pub const NPARAMS: usize = 8;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.bass.as_ptr(),
            self.pickup.as_ptr(),
            self.rounds.as_ptr(),
            self.pick.as_ptr(),
            self.drive.as_ptr(),
            self.amp.as_ptr(),
            self.stereo.as_ptr(),
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
    neck: [Biquad; 4],
    bridge: [Biquad; 4],
    strings: [Biquad; 2],
    pick_hp: Biquad,
    fast: Envelope,
    slow: Envelope,
    // two amps for Rick-O-Sound (or one, the same for both)
    amp_pre: [Biquad; 2],
    amp_cab: [[Biquad; 3]; 2],
    dc: [DcBlock; 2],
    key: (usize, i32),
    mix: Smooth,
    pick: Smooth,
    drive: Smooth,
    amp: Smooth,
    stereo: Smooth,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let rick = (self.v[BASS].round() as usize) == 0;
        let r = self.v[ROUNDS] as f64;
        let mut neck = [Biquad::identity(); 4];
        let mut bridge = [Biquad::identity(); 4];
        if rick {
            // 4001 neck: deep, with the scooped low mids
            neck[0].set_low_shelf(fs, 120.0, 0.7, 3.0);
            neck[1].set_peak(fs, 550.0, 0.8, -3.0);
            neck[2].set_peak(fs, 2500.0, 1.2, 2.0);
            neck[3].set_highpass(fs, 32.0, 0.7);
            // 4001 bridge: thin, honky clank and the metallic treble
            bridge[0].set_low_shelf(fs, 220.0, 0.7, -7.0);
            bridge[1].set_peak(fs, 1100.0, 1.0, 4.0);
            bridge[2].set_peak(fs, 2800.0, 1.5, 5.5);
            bridge[3].set_highpass(fs, 55.0, 0.7);
        } else {
            // Precision: one split-coil pickup, round lows and a mid growl, no extreme top
            neck[0].set_peak(fs, 90.0, 0.9, 3.0);
            neck[1].set_peak(fs, 800.0, 1.0, 2.5);
            neck[2].set_lowpass(fs, 4500.0, 0.7);
            neck[3].set_highpass(fs, 35.0, 0.7);
            bridge = neck;
        }
        self.neck = neck;
        self.bridge = bridge;
        // roundwounds: the zing of new Rotosounds (flats with it at zero)
        self.strings[0].set_high_shelf(fs, 2500.0, 0.7, -6.0 + 12.0 * r);
        self.strings[1].set_peak(fs, 5000.0, 2.0, 4.0 * r);
        self.pick_hp.set_highpass(fs, 900.0, 0.7);
        // the amps: the bassy side darker, the bright side thinner
        self.amp_pre[0].set_low_shelf(fs, 100.0, 0.7, 3.0);
        self.amp_pre[1].set_peak(fs, 1500.0, 0.8, 3.0);
        for (i, c) in self.amp_cab.iter_mut().enumerate() {
            c[0].set_highpass(fs, if i == 0 { 40.0 } else { 70.0 }, 0.7);
            c[1].set_peak(fs, 110.0, 1.0, if i == 0 { 3.0 } else { 1.0 });
            c[2].set_lowpass(fs, if i == 0 { 2600.0 } else { 4800.0 }, 0.7);
        }
    }
    fn mix_target(&self) -> f64 {
        if (self.v[BASS].round() as usize) == 1 {
            return 0.0; // (the Precision's one pickup is on the neck path)
        }
        match (self.v[PICKUP].round() as usize).min(2) {
            0 => 0.0,
            1 => 0.5,
            _ => 1.0,
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut d = Self {
            fs, v,
            neck: [Biquad::identity(); 4], bridge: [Biquad::identity(); 4], strings: [Biquad::identity(); 2], pick_hp: Biquad::identity(),
            fast: Envelope::new(fs, 0.5, 25.0), slow: Envelope::new(fs, 25.0, 250.0),
            amp_pre: [Biquad::identity(); 2], amp_cab: [[Biquad::identity(); 3]; 2], dc: [DcBlock::new(fs), DcBlock::new(fs)],
            key: (9, -1),
            mix: Smooth::new(fs, 30.0, 0.5), pick: Smooth::new(fs, 20.0, 0.6), drive: Smooth::new(fs, 20.0, 0.35),
            amp: Smooth::new(fs, 20.0, 0.5), stereo: Smooth::new(fs, 30.0, 0.0), out: Smooth::new(fs, 20.0, 1.0),
        };
        d.design();
        d
    }
    fn reset(&mut self) {
        self.neck.iter_mut().chain(self.bridge.iter_mut()).chain(self.strings.iter_mut()).chain(self.amp_pre.iter_mut()).for_each(|b| b.reset());
        self.amp_cab.iter_mut().flatten().for_each(|b| b.reset());
        self.pick_hp.reset();
        self.fast.value = 0.0;
        self.slow.value = 0.0;
        self.dc.iter_mut().for_each(|d| d.reset());
        self.mix.snap(self.mix_target());
        self.pick.snap(self.v[PICK] as f64);
        self.drive.snap(self.v[DRIVE] as f64);
        self.amp.snap(self.v[AMP] as f64);
        self.stereo.snap(self.v[STEREO] as f64);
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = (v[BASS].round() as usize, (v[ROUNDS] * 200.0) as i32);
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.mix.set(self.mix_target());
        self.pick.set(v[PICK] as f64);
        self.drive.set(v[DRIVE] as f64);
        self.amp.set(v[AMP] as f64);
        self.stereo.set(if (v[BASS].round() as usize) == 0 { v[STEREO] as f64 } else { 0.0 });
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nc = chans.len().max(1);
        for k in 0..n {
            let m = self.mix.next();
            let pk = self.pick.next();
            let dr = self.drive.next();
            let am = self.amp.next();
            let st = self.stereo.next();
            let o = self.out.next();
            // a bass is one source: the channels summed
            let x = chans.iter().map(|c| c[k] as f64).sum::<f64>() / nc as f64;
            // the pick: the attack's edge lifted on each new note
            let f = self.fast.process(x);
            let s = self.slow.process(x);
            let edge = ((f / (s + 1e-6)) - 1.0).clamp(0.0, 3.0);
            let x = x * (1.0 + pk * 0.5 * edge) + pk * 0.8 * edge * self.pick_hp.process(x);
            let x = cascade(&mut self.strings, x);
            let nk = cascade(&mut self.neck, x);
            let br = cascade(&mut self.bridge, x);
            let mono = (1.0 - m) * nk + m * br;
            // Rick-O-Sound: each pickup to its own amp, else both amps get the selected sound
            let src = [mono + st * (nk - mono), mono + st * (br - mono)];
            let g = 1.0 + dr * 14.0;
            let mut out = [0.0; 2];
            for i in 0..2 {
                let a = self.amp_pre[i].process(src[i]);
                let a = tube(a * g, 0.25) / g.powf(0.6);
                let a = self.dc[i].process(cascade(&mut self.amp_cab[i], a));
                // the DI and the miked amp, blended as on the records
                out[i] = ((1.0 - am) * src[i] + am * a * 1.4) * o;
            }
            let (l, r) = if st > 1e-3 { (out[0], out[1]) } else { let v = 0.5 * (out[0] + out[1]); (v, v) };
            for (ci, ch) in chans.iter_mut().enumerate() {
                ch[k] = (if ci == 0 { l } else { r }) as f32;
            }
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "In the City (Rick, both, clanky)", values: &[] },
        Preset { name: "Going Underground (Rick, bridge bite)", values: &[(PICKUP, 2.0), (ROUNDS, 0.85), (PICK, 0.75), (DRIVE, 0.45)] },
        Preset { name: "Town Called Malice (Precision, Motown)", values: &[(BASS, 1.0), (ROUNDS, 0.45), (PICK, 0.35), (DRIVE, 0.2), (AMP, 0.4)] },
        Preset { name: "Start! (Rick, neck, melodic)", values: &[(PICKUP, 0.0), (ROUNDS, 0.6), (PICK, 0.5), (DRIVE, 0.3)] },
        Preset { name: "Eton Rifles (DI and amp)", values: &[(PICKUP, 1.0), (ROUNDS, 0.7), (PICK, 0.7), (DRIVE, 0.5), (AMP, 0.55)] },
        Preset { name: "Rick-O-Sound Stereo Split", values: &[(PICKUP, 1.0), (STEREO, 1.0), (AMP, 0.7)] },
        Preset { name: "Cranked Marshall Bass", values: &[(PICKUP, 1.0), (DRIVE, 0.85), (AMP, 0.8)] },
        Preset { name: "Clean DI Only", values: &[(DRIVE, 0.0), (AMP, 0.0)] },
        Preset { name: "Flatwounds, Fingers (Sixties Mod)", values: &[(BASS, 1.0), (ROUNDS, 0.0), (PICK, 0.0), (DRIVE, 0.25), (AMP, 0.5)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "FOXTON BASS",
        subtitle: ("BRUCE FOXTON'S BASSES & MARSHALL", "4001 OR PRECISION · ROTOSOUNDS · PICK · RICK-O-SOUND"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::Selector { p: BASS, x: 110.0, y: 150.0, label: "BASS", positions: &["4001", "PRECISION"], tip: "The Rickenbacker 4001 (metallic, clanking treble) or the Fender Precision (round, Motown)." },
            Item::Selector { p: PICKUP, x: 250.0, y: 150.0, label: "PICKUP", positions: &["NECK", "BOTH", "BRIDGE"], tip: "The 4001's pickups: neck (deep), both, or bridge (thin, honky clank). The Precision has one." },
            Item::Knob { p: ROUNDS, x: 390.0, y: 150.0, r: 26.0, label: "ROUNDWOUNDS", tip: "From dead flatwounds (left) to new, zingy Rotosound roundwounds (right).", active: None },
            Item::Knob { p: PICK, x: 530.0, y: 150.0, r: 26.0, label: "PICK", tip: "The plectrum: the click and edge on each new note.", active: None },
            Item::Divider { y: 236.0 },
            Item::Knob { p: DRIVE, x: 110.0, y: 310.0, r: 28.0, label: "AMP DRIVE", tip: "How hard the 100-watt Marshall valve bass head is pushed.", active: None },
            Item::Knob { p: AMP, x: 250.0, y: 310.0, r: 28.0, label: "DI / AMP", tip: "The DI (left) and the miked amp (right), taken together on the records.", active: None },
            Item::Knob { p: STEREO, x: 390.0, y: 310.0, r: 28.0, label: "RICK-O-SOUND", tip: "The 4001's stereo output: the neck pickup to a bassy amp on the left, the bridge to a bright one on the right.", active: Some(ActiveWhen { p: BASS, min: 0.0, max: 0.0 }) },
            Item::Knob { p: OUTPUT, x: 530.0, y: 310.0, r: 28.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Foxton Bass", class_id: *b"TJamFoxtonBassP1", vendor: "PEQUOD The Jam",
    categories: [Fx, Filter]
);

#[cfg(test)]
#[allow(dead_code)]
pub mod tests {
    use super::*;
    use std::f64::consts::PI;

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
    pub fn sine(f: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n).map(|k| (amp * (2.0 * PI * f * k as f64 / 48000.0).sin()) as f32).collect()
    }
    pub fn noise(n: usize, amp: f64) -> Vec<f32> {
        let mut nz = vintage::dsp::Noise::new(11);
        (0..n).map(|_| (nz.next() * amp) as f32).collect()
    }
    /// Both channels out for a mono input.
    pub fn go2(p: &[f32], x: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut input = vec![x.to_vec(), x.to_vec()];
        run(p, &mut input);
        let r = input.pop().unwrap();
        (input.pop().unwrap(), r)
    }
    pub fn go(p: &[f32], x: &[f32]) -> Vec<f32> {
        go2(p, x).0
    }
    /// Steady-state level of a sine at f through the settings, in dB.
    pub fn level(set: &[(usize, f32)], f: f64) -> f64 {
        let p = with(defaults(), set);
        let y = go(&p, &sine(f, 0.1, 48000));
        db(rms(&y[24000..]))
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
            let l: Vec<f32> = (0..n).map(|k| if (k / 6000) % 2 == 0 { (nz.next() * 0.5) as f32 } else { 0.0 }).collect();
            let mut input = vec![l.clone(), l];
            run(&p, &mut input);
            assert!(input.iter().flatten().all(|v| v.is_finite() && v.abs() < 8.0), "unbounded output for {:?}", p);
        }
    }

    #[test]
    fn bridge_is_thinner_and_brighter() {
        let n = level(&[(PICKUP, 0.0), (AMP, 0.0)], 80.0);
        let b = level(&[(PICKUP, 2.0), (AMP, 0.0)], 80.0);
        let nt = level(&[(PICKUP, 0.0), (AMP, 0.0)], 2800.0);
        let bt = level(&[(PICKUP, 2.0), (AMP, 0.0)], 2800.0);
        println!("80 Hz: neck {n:.1} bridge {b:.1}; 2.8 kHz: neck {nt:.1} bridge {bt:.1}");
        assert!(n - b > 5.0 && bt - nt > 3.0);
    }
    #[test]
    fn precision_is_rounder_than_the_rick() {
        let r = level(&[(BASS, 0.0), (PICKUP, 1.0), (AMP, 0.0)], 2800.0);
        let p = level(&[(BASS, 1.0), (AMP, 0.0)], 2800.0);
        println!("2.8 kHz: Rick {r:.1}, Precision {p:.1}");
        assert!(r - p > 4.0);
    }
    #[test]
    fn roundwounds_zing() {
        let f = level(&[(ROUNDS, 0.0), (AMP, 0.0)], 5000.0);
        let r = level(&[(ROUNDS, 1.0), (AMP, 0.0)], 5000.0);
        println!("5 kHz: flats {f:.1}, rounds {r:.1}");
        assert!(r - f > 10.0);
    }
    #[test]
    fn rick_o_sound_splits_the_pickups() {
        let p = with(defaults(), &[(STEREO, 1.0), (AMP, 0.0)]);
        let (l, r) = go2(&p, &sine(70.0, 0.1, 48000));
        println!("70 Hz split: left {:.1}, right {:.1}", db(rms(&l[24000..])), db(rms(&r[24000..])));
        assert!(db(rms(&l[24000..])) - db(rms(&r[24000..])) > 5.0);
    }
    #[test]
    fn pick_sharpens_the_attack() {
        // plucks: a decaying 110 Hz tone with a bright start
        let x: Vec<f32> = (0..48000).map(|k| {
            let t = (k % 12000) as f64 / 48000.0;
            ((2.0 * PI * 110.0 * t).sin() * 0.3 * (-t * 6.0).exp()) as f32
        }).collect();
        let a = go(&with(defaults(), &[(PICK, 0.0), (AMP, 0.0)]), &x);
        let b = go(&with(defaults(), &[(PICK, 1.0), (AMP, 0.0)]), &x);
        let att = |y: &[f32]| rms(&y[12000..12480]);
        let tail = |y: &[f32]| rms(&y[18000..20000]);
        let (ra, rb) = (db(att(&a)) - db(tail(&a)), db(att(&b)) - db(tail(&b)));
        println!("attack over tail: pick 0 {ra:.1} dB, pick 1 {rb:.1} dB");
        assert!(rb - ra > 1.0);
    }

}
