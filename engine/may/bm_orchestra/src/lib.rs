//! Guitar Orchestra: Brian May's guitar choirs (Killer Queen, The Prophet's Song, Bohemian Rhapsody's solo) built from one line. It follows the note you play and adds up to three more guitars a diatonic interval away in your key and scale (thirds, fourths, fifths, sixths, octaves, above or below), each a little late and a little out of tune like real overdubs, panned across the stereo field.

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
use vintage::fx::PitchShift;
use vintage::pitch::Pitch;

pub const W: u32 = 640;
pub const H: u32 = 468;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Key { C, #[name = "C#"] Cs, D, #[name = "Eb"] Eb, E, F, #[name = "F#"] Fs, G, #[name = "Ab"] Ab, A, #[name = "Bb"] Bb, B }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Scale { #[name = "Major"] Major, #[name = "Minor"] Minor, #[name = "Harmonic minor"] Harm, #[name = "Dorian"] Dorian, #[name = "Mixolydian"] Mixo }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Interval {
    #[name = "Off"] Off, #[name = "Double (unison)"] Uni, #[name = "3rd up"] T3u, #[name = "4th up"] T4u, #[name = "5th up"] T5u, #[name = "6th up"] T6u,
    #[name = "Octave up"] Ou, #[name = "3rd down"] T3d, #[name = "4th down"] T4d, #[name = "5th down"] T5d, #[name = "6th down"] T6d, #[name = "Octave down"] Od
}


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "key"]
    pub key: EnumParam<Key>,
    #[id = "scale"]
    pub scale: EnumParam<Scale>,
    #[id = "v1"]
    pub v1: EnumParam<Interval>,
    #[id = "v2"]
    pub v2: EnumParam<Interval>,
    #[id = "v3"]
    pub v3: EnumParam<Interval>,
    #[id = "voices"]
    pub voices: FloatParam,
    #[id = "loose"]
    pub loose: FloatParam,
    #[id = "dry"]
    pub dry: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            key: EnumParam::new("Key", Key::A),
            scale: EnumParam::new("Scale", Scale::Major),
            v1: EnumParam::new("Voice 1", Interval::T3u),
            v2: EnumParam::new("Voice 2", Interval::T5u),
            v3: EnumParam::new("Voice 3", Interval::Off),
            voices: db("Voices Level", -3.0, -24.0, 6.0),
            loose: pct("Looseness", 0.4),
            dry: db("Dry Level", 0.0, -60.0, 6.0),
            output: db("Output", -3.0, -24.0, 12.0),
        }
    }
}

pub const KEY: usize = 0;
pub const SCALE: usize = 1;
pub const V1: usize = 2;
pub const V2: usize = 3;
pub const V3: usize = 4;
pub const VOICES: usize = 5;
pub const LOOSE: usize = 6;
pub const DRY: usize = 7;
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
            self.key.as_ptr(),
            self.scale.as_ptr(),
            self.v1.as_ptr(),
            self.v2.as_ptr(),
            self.v3.as_ptr(),
            self.voices.as_ptr(),
            self.loose.as_ptr(),
            self.dry.as_ptr(),
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

pub const SCALES: [[i32; 7]; 5] = [
    [0, 2, 4, 5, 7, 9, 11],
    [0, 2, 3, 5, 7, 8, 10],
    [0, 2, 3, 5, 7, 8, 11],
    [0, 2, 3, 5, 7, 9, 10],
    [0, 2, 4, 5, 7, 9, 10],
];
/// Scale steps for each interval choice (index 0 = off).
const STEPS: [i32; 12] = [0, 0, 2, 3, 4, 5, 7, -2, -3, -4, -5, -7];

/// Semitones from `note` (MIDI, rounded) to the note `steps` scale steps away in key/scale.
pub fn diatonic_shift(note: i32, key: i32, scale: usize, steps: i32) -> i32 {
    let s = &SCALES[scale.min(4)];
    let rel = (note - key).rem_euclid(12);
    let oct = (note - key).div_euclid(12);
    // the scale degree at or just below the played note (a chromatic note keeps its offset)
    let mut deg = 0;
    for (i, v) in s.iter().enumerate() {
        if *v <= rel {
            deg = i as i32;
        }
    }
    let off = rel - s[deg as usize];
    let t = deg + steps;
    let t_oct = t.div_euclid(7);
    let t_deg = t.rem_euclid(7) as usize;
    let target = key + 12 * (oct + t_oct) + s[t_deg] + off;
    target - note
}

struct Voice {
    ps: PitchShift,
    dl: DelayLine,
    delay: f64,
    detune: f64,
    pan: (f64, f64),
    ratio: Smooth,
}

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    pitch: Pitch,
    voices: Vec<Voice>,
    note: Option<i32>,
    dry: Smooth,
    vl: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let pans = [-0.8, 0.8, 0.0];
        let voices = (0..3).map(|i| {
            let th = (pans[i] + 1.0) * std::f64::consts::FRAC_PI_4;
            Voice { ps: PitchShift::new(fs, 36.0), dl: DelayLine::new((0.06 * fs) as usize), delay: 0.0, detune: 0.0, pan: (th.cos(), th.sin()), ratio: Smooth::new(fs, 12.0, 1.0) }
        }).collect();
        Self { fs, v, pitch: Pitch::new(fs), voices, note: None, dry: Smooth::new(fs, 20.0, 1.0), vl: Smooth::new(fs, 20.0, 0.7), out: Smooth::new(fs, 20.0, 1.0) }
    }
    fn reset(&mut self) {
        self.pitch.reset();
        for v in self.voices.iter_mut() {
            v.ps.reset();
            v.dl.reset();
            v.ratio.snap(1.0);
        }
        self.note = None;
        self.dry.snap(db_to_gain(self.v[DRY] as f64));
        self.vl.snap(db_to_gain(self.v[VOICES] as f64));
        self.out.snap(db_to_gain(self.v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let loose = v[LOOSE] as f64;
        // each overdub is a little late and a little off: fixed per voice, scaled by LOOSE
        let lag = [11.0, 19.0, 27.0];
        let cents = [6.0, -7.0, 3.0];
        for (i, vo) in self.voices.iter_mut().enumerate() {
            vo.delay = (2.0 + lag[i] * loose) * 0.001 * self.fs;
            vo.detune = 2f64.powf(cents[i] * loose / 1200.0);
        }
        self.dry.set(db_to_gain(v[DRY] as f64));
        self.vl.set(db_to_gain(v[VOICES] as f64));
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().max(1);
        let key = self.v[KEY].round() as i32;
        let scale = self.v[SCALE].round() as usize;
        let ivs = [self.v[V1].round() as usize, self.v[V2].round() as usize, self.v[V3].round() as usize];
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch as f64;
            self.pitch.push(x);
            if let Some(f) = self.pitch.freq {
                self.note = Some((69.0 + 12.0 * (f / 440.0).log2()).round() as i32);
            }
            let dry = self.dry.next();
            let vl = self.vl.next();
            let o = self.out.next();
            let (mut l, mut r) = (0.0, 0.0);
            for (i, vo) in self.voices.iter_mut().enumerate() {
                let iv = ivs[i].min(11);
                if iv == 0 {
                    continue;
                }
                let semis = match self.note {
                    Some(nt) => diatonic_shift(nt, key, scale, STEPS[iv]),
                    None => 0,
                };
                let target = 2f64.powf(semis as f64 / 12.0) * vo.detune;
                vo.ratio.set(target);
                let ratio = vo.ratio.next();
                let y = vo.ps.process(x, ratio);
                vo.dl.write(y);
                let y = vo.dl.read(vo.delay.max(1.0)) * vl;
                l += y * vo.pan.0;
                r += y * vo.pan.1;
            }
            if nch >= 2 {
                chans[0][k] = ((chans[0][k] as f64 * dry + l) * o) as f32;
                chans[1][k] = ((chans[1][k] as f64 * dry + r) * o) as f32;
            } else {
                chans[0][k] = ((chans[0][k] as f64 * dry + 0.5 * (l + r)) * o) as f32;
            }
        }
        meters[0] = if self.note.is_some() && self.pitch.freq.is_some() { 1.0 } else { 0.0 };
    }
}


// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Two-Part (3rd up)", values: &[(V2, 0.0)] },
        Preset { name: "Three-Part (3rd + 5th up)", values: &[] },
        Preset { name: "Killer Queen Choir (3rd + 5th + octave)", values: &[(V3, 6.0)] },
        Preset { name: "Harmony Below (3rd + 6th down)", values: &[(V1, 7.0), (V2, 10.0)] },
        Preset { name: "Double-Tracked Lead (unison)", values: &[(V1, 1.0), (V2, 0.0), (LOOSE, 0.6)] },
        Preset { name: "Octaves (up and down)", values: &[(V1, 6.0), (V2, 11.0)] },
        Preset { name: "Minor Key Harmonies (Am)", values: &[(KEY, 9.0), (SCALE, 1.0)] },
        Preset { name: "Voices Only (for a send)", values: &[(DRY, -60.0), (VOICES, 0.0)] },
        Preset { name: "Tight Studio Overdubs", values: &[(LOOSE, 0.15)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "GUITAR ORCHESTRA",
        subtitle: ("HARMONY GUITARS FROM ONE LINE", "DIATONIC · IN KEY · UP TO 3 PARTS"),
        accent: ui::INDIGO,
        size: (W, H),
        items: vec![

            Item::Selector { p: KEY, x: 90.0, y: 160.0, label: "KEY", positions: &["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"], tip: "The key the harmonies stay in." },
            Item::RadioList { p: SCALE, x: 170.0, y: 96.0, w: 130.0, row_h: 20.0, names: &["Major", "Minor", "Harmonic minor", "Dorian", "Mixolydian"], tip: "The scale the harmonies stay in." },
            Item::Lamp { m: 0, x: 75.0, y: 250.0, label: "NOTE", lo: 0.0, hi: 1.0 },
            Item::Text { x: 470.0, y: 96.0, text: "VOICES (L · R · CENTRE)", size: 11.0, align: Align2::CENTER_CENTER },
            Item::RadioList { p: V1, x: 330.0, y: 110.0, w: 92.0, row_h: 14.0, names: &["Off", "Double", "3rd up", "4th up", "5th up", "6th up", "Oct up", "3rd dn", "4th dn", "5th dn", "6th dn", "Oct dn"], tip: "Voice 1 (left): its interval in the scale." },
            Item::RadioList { p: V2, x: 426.0, y: 110.0, w: 92.0, row_h: 14.0, names: &["Off", "Double", "3rd up", "4th up", "5th up", "6th up", "Oct up", "3rd dn", "4th dn", "5th dn", "6th dn", "Oct dn"], tip: "Voice 2 (right)." },
            Item::RadioList { p: V3, x: 522.0, y: 110.0, w: 92.0, row_h: 14.0, names: &["Off", "Double", "3rd up", "4th up", "5th up", "6th up", "Oct up", "3rd dn", "4th dn", "5th dn", "6th dn", "Oct dn"], tip: "Voice 3 (centre)." },
            Item::Divider { y: 290.0 },
            Item::Knob { p: VOICES, x: 80.0, y: 355.0, r: 22.0, label: "VOICES", tip: "Level of the added guitars.", active: None },
            Item::Knob { p: LOOSE, x: 200.0, y: 355.0, r: 22.0, label: "LOOSENESS", tip: "How much each added part drifts like a real overdub: up to about 30 ms late and 7 cents out.", active: None },
            Item::Knob { p: DRY, x: 320.0, y: 355.0, r: 22.0, label: "DRY", tip: "Level of your own part.", active: None },
            Item::Knob { p: OUTPUT, x: 540.0, y: 355.0, r: 22.0, label: "OUTPUT", tip: "Output level. Works on single-note lines (it follows one note at a time).", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Guitar Orchestra", class_id: *b"BMyGuitarOrchPQ1", vendor: "PEQUOD Brian May",
    categories: [Fx, PitchShift]
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

    #[test]
    fn diatonic_maths() {
        // C major: E + 3rd = G (3 semis), F + 3rd = A (4), B + 5th = F (6), A - 3rd = F (-4)
        assert_eq!(diatonic_shift(64, 0, 0, 2), 3);
        assert_eq!(diatonic_shift(65, 0, 0, 2), 4);
        assert_eq!(diatonic_shift(71, 0, 0, 4), 6);
        assert_eq!(diatonic_shift(69, 0, 0, -2), -4);
        assert_eq!(diatonic_shift(60, 0, 0, 7), 12);
        // A minor: C + 3rd = E (4), A + 3rd = C (3)
        assert_eq!(diatonic_shift(60, 9, 1, 2), 4);
        assert_eq!(diatonic_shift(69, 9, 1, 2), 3);
    }
    #[test]
    fn adds_a_diatonic_third() {
        // key C major, play E4: the 3rd above is G4 (392 Hz); play F4: A4 (440 Hz)
        for (f, want, not) in [(329.63, 392.0, 415.3), (349.23, 440.0, 415.3)] {
            let p = with(defaults(), &[(KEY, 0.0), (SCALE, 0.0), (V1, 2.0), (V2, 0.0), (V3, 0.0), (DRY, -60.0), (VOICES, 0.0)]);
            let (l, _r) = go2(&p, &sine_amp(f, 0.3, 48000));
            let a = goertzel(&l[24000..], want);
            let b = goertzel(&l[24000..], not);
            println!("played {f:.0} Hz: {want:.0} Hz {:.1} dB vs {not:.0} Hz {:.1} dB", db(a), db(b));
            assert!(db(a) > db(b) + 12.0);
        }
    }

}
