//! Ricky 330: voices a guitar DI toward Paul Weller's Rickenbacker 330, the semi-hollow he played almost
//! throughout The Jam (Rotosound strings, into a cranked Vox AC30). It takes out the character of your own
//! pickups, then adds the Rickenbacker's: the bright, ringing "toaster"-era high-output pickups with their
//! sharp resonance, the three-way selector and the fifth knob that blends the two pickups in the middle
//! position, the tone pots, and the short ring of the thin hollow body. TWELVE adds the octave courses of a
//! 330/12 (the lower four courses an octave up, the top two in unison), for the chime of mod's 60s records.
//! Curves estimated from published descriptions and measurements of these pickups.

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
pub enum Source { #[name = "Single coils"] Single, #[name = "Humbuckers"] Humbucker, #[name = "Already a Rickenbacker"] Flat }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Pickup { #[name = "Neck"] Neck, #[name = "Both"] Both, #[name = "Bridge"] Bridge }

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
    #[id = "blend"]
    pub blend: FloatParam,
    #[id = "chime"]
    pub chime: FloatParam,
    #[id = "tone"]
    pub tone: FloatParam,
    #[id = "body"]
    pub body: FloatParam,
    #[id = "twelve"]
    pub twelve: FloatParam,
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
            blend: pct("Blend", 0.5),
            chime: pct("Chime", 0.6),
            tone: pct("Tone", 1.0),
            body: pct("Body", 0.35),
            twelve: pct("Twelve", 0.0),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const SOURCE: usize = 0;
pub const PICKUP: usize = 1;
pub const BLEND: usize = 2;
pub const CHIME: usize = 3;
pub const TONE: usize = 4;
pub const BODY: usize = 5;
pub const TWELVE: usize = 6;
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
            self.source.as_ptr(),
            self.pickup.as_ptr(),
            self.blend.as_ptr(),
            self.chime.as_ptr(),
            self.tone.as_ptr(),
            self.body.as_ptr(),
            self.twelve.as_ptr(),
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

struct Ch {
    undo: [Biquad; 2],
    neck: [Biquad; 4],
    bridge: [Biquad; 4],
    post: [Biquad; 4],
    ring: [Biquad; 2],
    lo: Biquad,
    hi: Biquad,
    oct: PitchShift,
    uni: PitchShift,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    ch: [Ch; 2],
    key: (usize, i32, i32),
    blend: Smooth,
    body: Smooth,
    twelve: Smooth,
    out: Smooth,
}

impl Dsp {
    fn design(&mut self) {
        let fs = self.fs;
        let src = (self.v[SOURCE].round() as usize).min(2);
        let chime = self.v[CHIME] as f64;
        let tone = self.v[TONE] as f64;
        let mut undo = [Biquad::identity(); 2];
        match src {
            0 => {
                undo[0].set_peak(fs, 3600.0, 1.2, -2.5);
                undo[1].set_low_shelf(fs, 220.0, 0.7, 1.0);
            }
            1 => {
                undo[0].set_peak(fs, 4300.0, 0.9, 3.0);
                undo[1].set_peak(fs, 1300.0, 0.9, -2.5);
            }
            _ => {}
        }
        // the neck pickup: round, with its resonance low in the presence range
        let mut neck = [Biquad::identity(); 4];
        neck[0].set_low_shelf(fs, 250.0, 0.7, 2.5);
        neck[1].set_peak(fs, 2800.0, 1.5, 3.0);
        neck[2].set_high_shelf(fs, 4500.0, 0.7, -4.0);
        neck[3].set_peak(fs, 700.0, 0.8, 1.0);
        // the bridge pickup: thin, honky, with the sharp high-output resonance that makes the jangle
        let mut bridge = [Biquad::identity(); 4];
        bridge[0].set_low_shelf(fs, 380.0, 0.7, -5.5);
        bridge[1].set_peak(fs, 1200.0, 0.9, 2.0);
        bridge[2].set_peak(fs, 4200.0, 1.8, 5.5);
        bridge[3].set_high_shelf(fs, 8000.0, 0.7, -2.0);
        // chime (the metallic ring), the tone pots, the thin hollow body's air
        let mut post = [Biquad::identity(); 4];
        post[0].set_peak(fs, 3300.0, 1.2, 5.0 * chime);
        post[1].set_peak(fs, 5600.0, 4.0, 3.0 * chime);
        post[2].set_peak(fs, 420.0, 0.9, -3.0 * chime);
        post[3].set_lowpass(fs, 1800.0 * (11.0f64).powf(tone).min(10.5), 0.65);
        let mut ring = [Biquad::identity(); 2];
        ring[0].set_bandpass(fs, 140.0, 6.0);
        ring[1].set_bandpass(fs, 290.0, 5.0);
        for c in self.ch.iter_mut() {
            c.undo = undo;
            c.neck = neck;
            c.bridge = bridge;
            c.post = post;
            c.ring = ring;
            c.lo.set_lowpass(fs, 700.0, 0.7);
            c.hi.set_highpass(fs, 700.0, 0.7);
        }
    }
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let ch = || Ch {
            undo: [Biquad::identity(); 2], neck: [Biquad::identity(); 4], bridge: [Biquad::identity(); 4],
            post: [Biquad::identity(); 4], ring: [Biquad::identity(); 2], lo: Biquad::identity(), hi: Biquad::identity(),
            oct: PitchShift::new(fs, 28.0), uni: PitchShift::new(fs, 34.0),
        };
        let mut d = Self {
            fs, v, ch: [ch(), ch()], key: (9, -1, -1),
            blend: Smooth::new(fs, 20.0, 0.5), body: Smooth::new(fs, 20.0, 0.35), twelve: Smooth::new(fs, 20.0, 0.0), out: Smooth::new(fs, 20.0, 1.0),
        };
        d.design();
        d
    }
    fn reset(&mut self) {
        for c in self.ch.iter_mut() {
            c.undo.iter_mut().chain(c.neck.iter_mut()).chain(c.bridge.iter_mut()).chain(c.post.iter_mut()).chain(c.ring.iter_mut()).for_each(|b| b.reset());
            c.lo.reset();
            c.hi.reset();
            c.oct.reset();
            c.uni.reset();
        }
        self.blend.snap(self.blend_target());
        self.body.snap(self.v[BODY] as f64);
        self.twelve.snap(self.v[TWELVE] as f64);
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = (v[SOURCE].round() as usize, (v[CHIME] * 200.0) as i32, (v[TONE] * 200.0) as i32);
        if k != self.key {
            self.key = k;
            self.design();
        }
        self.blend.set(self.blend_target());
        self.body.set(v[BODY] as f64);
        self.twelve.set(v[TWELVE] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let detune = 2f64.powf(4.0 / 1200.0);
        for k in 0..n {
            let b = self.blend.next();
            let body = self.body.next();
            let tw = self.twelve.next();
            let o = self.out.next();
            for (ci, chan) in chans.iter_mut().enumerate() {
                let c = &mut self.ch[ci.min(1)];
                let x = chan[k] as f64;
                // the twelve-string's extra courses: the low four an octave up, the top two in unison
                let x = if tw > 1e-4 {
                    let oct = c.oct.process(c.lo.process(x), 2.0);
                    let uni = c.uni.process(c.hi.process(x), detune);
                    x + tw * (0.75 * oct + 0.45 * uni)
                } else {
                    x
                };
                let x = cascade(&mut c.undo, x);
                let nk = if b < 0.999 { cascade(&mut c.neck, x) } else { 0.0 };
                let br = if b > 0.001 { cascade(&mut c.bridge, x) } else { 0.0 };
                // both on: the two pickups summed, with the interference that scoops the middle
                let y = (1.0 - b) * nk + b * br;
                let y = cascade(&mut c.post, y);
                let ring = c.ring[0].process(x) * 1.2 + c.ring[1].process(x) * 0.7;
                chan[k] = ((y + body * ring) * o) as f32;
            }
        }
    }
}

impl Dsp {
    /// 0 = neck alone, 1 = bridge alone; in the middle position, the blend knob.
    fn blend_target(&self) -> f64 {
        match (self.v[PICKUP].round() as usize).min(2) {
            0 => 0.0,
            2 => 1.0,
            _ => (self.v[BLEND] as f64).clamp(0.02, 0.98),
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Going Underground (bridge, ringing)", values: &[] },
        Preset { name: "In the City (both, full chime)", values: &[(PICKUP, 1.0), (BLEND, 0.6), (CHIME, 0.85)] },
        Preset { name: "Eton Rifles (bridge, metallic)", values: &[(PICKUP, 2.0), (CHIME, 1.0), (BODY, 0.2)] },
        Preset { name: "Town Called Malice (both, rhythm)", values: &[(PICKUP, 1.0), (BLEND, 0.45), (CHIME, 0.5), (TONE, 0.8)] },
        Preset { name: "Neck, Mellow", values: &[(PICKUP, 0.0), (CHIME, 0.3), (TONE, 0.6), (BODY, 0.5)] },
        Preset { name: "Sixties Twelve-String Chime", values: &[(PICKUP, 1.0), (BLEND, 0.7), (CHIME, 0.7), (TWELVE, 0.8)] },
        Preset { name: "Twelve-String, Bridge Jangle", values: &[(PICKUP, 2.0), (CHIME, 0.8), (TWELVE, 1.0)] },
        Preset { name: "From a Les Paul", values: &[(SOURCE, 1.0), (PICKUP, 2.0), (CHIME, 0.75)] },
        Preset { name: "Already a Rickenbacker (twelve only)", values: &[(SOURCE, 2.0), (CHIME, 0.0), (BODY, 0.0), (TWELVE, 0.8)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "RICKY 330",
        subtitle: ("A RICKENBACKER 330 FROM YOUR DI", "TOASTER JANGLE · BLEND KNOB · TWELVE-STRING COURSES"),
        accent: ui::MOSS,
        size: (W, H),
        items: vec![
            Item::RadioList { p: SOURCE, x: 40.0, y: 100.0, w: 170.0, row_h: 22.0, names: &["Single coils", "Humbuckers", "Already a Rickenbacker"], tip: "What you are actually playing, so its own pickup character can be taken out first." },
            Item::Text { x: 125.0, y: 196.0, text: "YOUR GUITAR", size: 11.0, align: Align2::CENTER_CENTER },
            Item::Selector { p: PICKUP, x: 300.0, y: 150.0, label: "PICKUP", positions: &["NECK", "BOTH", "BRIDGE"], tip: "The three-way selector: neck (round), both (scooped, ringing) or bridge (the thin, cutting jangle of Going Underground)." },
            Item::Knob { p: BLEND, x: 420.0, y: 150.0, r: 24.0, label: "BLEND", tip: "The Rickenbacker's fifth knob: in the middle position, how much of each pickup (left neck, right bridge).", active: Some(ActiveWhen { p: PICKUP, min: 1.0, max: 1.0 }) },
            Item::Knob { p: TONE, x: 530.0, y: 150.0, r: 24.0, label: "TONE", tip: "The tone pots: turn down to roll off the top.", active: None },
            Item::Divider { y: 236.0 },
            Item::Knob { p: CHIME, x: 110.0, y: 310.0, r: 28.0, label: "CHIME", tip: "The ring: the high-output pickups' sharp resonance and the metallic top, with the low mids scooped.", active: None },
            Item::Knob { p: BODY, x: 250.0, y: 310.0, r: 28.0, label: "BODY", tip: "The short ring of the thin semi-hollow body (around 140 and 290 Hz).", active: None },
            Item::Knob { p: TWELVE, x: 390.0, y: 310.0, r: 28.0, label: "TWELVE", tip: "Adds a 330/12's extra strings: the lower four courses an octave up, the top two doubled in unison.", active: None },
            Item::Knob { p: OUTPUT, x: 530.0, y: 310.0, r: 28.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Ricky 330", class_id: *b"TJamRicky330PQD1", vendor: "PEQUOD The Jam",
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
    #[test]
    fn bridge_is_thinner_than_neck() {
        let n = level(&[(PICKUP, 0.0), (BODY, 0.0)], 150.0);
        let b = level(&[(PICKUP, 2.0), (BODY, 0.0)], 150.0);
        println!("150 Hz: neck {n:.1}, bridge {b:.1}");
        assert!(n - b > 5.0);
    }
    #[test]
    fn chime_rings() {
        let a = level(&[(CHIME, 0.0), (BODY, 0.0)], 3300.0);
        let b = level(&[(CHIME, 1.0), (BODY, 0.0)], 3300.0);
        println!("3.3 kHz: chime 0 {a:.1}, chime 1 {b:.1}");
        assert!(b - a > 3.5);
    }
    #[test]
    fn tone_darkens() {
        let a = level(&[(TONE, 1.0), (BODY, 0.0)], 5000.0);
        let b = level(&[(TONE, 0.0), (BODY, 0.0)], 5000.0);
        println!("5 kHz: tone up {a:.1}, down {b:.1}");
        assert!(a - b > 8.0);
    }
    #[test]
    fn twelve_adds_the_octave() {
        let x = sine(196.0, 0.1, 48000);
        let a = go(&with(defaults(), &[(TWELVE, 0.0), (BODY, 0.0)]), &x);
        let b = go(&with(defaults(), &[(TWELVE, 1.0), (BODY, 0.0)]), &x);
        // short windows: the shifter's grains spread the octave a little, so look at it the way the ear does
        let short = |y: &[f32]| (0..30).map(|w| goertzel(&y[12000 + w * 1024..12000 + (w + 1) * 1024], 392.0)).sum::<f64>() / 30.0;
        let (oa, ob) = (short(&a), short(&b));
        println!("392 Hz: twelve 0 {:.1} dB, twelve 1 {:.1} dB", db(oa), db(ob));
        assert!(db(ob) - db(oa) > 20.0);
    }
    #[test]
    fn blend_moves_between_pickups() {
        let n = level(&[(PICKUP, 1.0), (BLEND, 0.0), (BODY, 0.0)], 150.0);
        let b = level(&[(PICKUP, 1.0), (BLEND, 1.0), (BODY, 0.0)], 150.0);
        println!("both, 150 Hz: blend neck {n:.1}, blend bridge {b:.1}");
        assert!(n - b > 4.0);
    }

}
