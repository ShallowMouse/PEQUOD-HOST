//! Chorus Echo: tape echo with three heads, spring reverb and chorus, after the Roland RE-501 Chorus Echo whose spring is Jack Antonoff's favourite outboard reverb. The dual outputs (one side chorused) give the wide, three-dimensional sound.

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
use vintage::fx::Bbd;
use vintage::verb::Diffuser;

pub const W: u32 = 640;
pub const H: u32 = 432;


#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Heads { #[name = "1"] H1, #[name = "2"] H2, #[name = "3"] H3, #[name = "1+2"] H12, #[name = "2+3"] H23, #[name = "1+2+3"] H123, #[name = "Reverb only"] Rev }
#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum OnOff { #[name = "Off"] Off, #[name = "On"] On }
// which heads play for each mode
const MODES: [[bool; 3]; 7] = [[true, false, false], [false, true, false], [false, false, true], [true, true, false], [false, true, true], [true, true, true], [false, false, false]];


#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "mode"]
    pub mode: EnumParam<Heads>,
    #[id = "rate"]
    pub rate_: FloatParam,
    #[id = "intensity"]
    pub intensity: FloatParam,
    #[id = "echo"]
    pub echo: FloatParam,
    #[id = "reverb"]
    pub reverb: FloatParam,
    #[id = "chorus"]
    pub chorus: EnumParam<OnOff>,
    #[id = "bass"]
    pub bass: FloatParam,
    #[id = "treble"]
    pub treble: FloatParam,
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
            mode: EnumParam::new("Heads", Heads::H12),
            rate_: ms("Repeat Rate", 120.0, 50.0, 300.0),
            intensity: pct("Intensity", 0.35),
            echo: pct("Echo Volume", 0.5),
            reverb: pct("Reverb Volume", 0.35),
            chorus: EnumParam::new("Chorus", OnOff::On),
            bass: db("Bass", 0.0, -10.0, 10.0),
            treble: db("Treble", 0.0, -10.0, 10.0),
            mix: pct("Mix", 0.4),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

pub const MODE: usize = 0;
pub const RATE: usize = 1;
pub const INTENSITY: usize = 2;
pub const ECHO: usize = 3;
pub const REVERB: usize = 4;
pub const CHORUS: usize = 5;
pub const BASS: usize = 6;
pub const TREBLE: usize = 7;
pub const MIX: usize = 8;
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
            self.mode.as_ptr(),
            self.rate_.as_ptr(),
            self.intensity.as_ptr(),
            self.echo.as_ptr(),
            self.reverb.as_ptr(),
            self.chorus.as_ptr(),
            self.bass.as_ptr(),
            self.treble.as_ptr(),
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
    tape: DelayLine,
    rec: OnePole,
    play_lp: OnePole,
    fb: f64,
    wow: Wander,
    flutter: Lfo,
    spring_ap: [Diffuser; 4],
    spring: [DelayLine; 2],
    sp_lp: OnePole,
    sp_fb: f64,
    bbd: Bbd,
    clfo: Lfo,
    eq: [[Biquad; 2]; 2],
    eq_key: (f64, f64),
    v: [f32; NPARAMS],
    rate: Smooth,
    mix: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 1;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut rec = OnePole::default();
        rec.set_lowpass(fs, 8000.0);
        let mut play_lp = OnePole::default();
        play_lp.set_lowpass(fs, 5000.0);
        let mut sp_lp = OnePole::default();
        sp_lp.set_lowpass(fs, 4500.0);
        let mut bbd = Bbd::new(fs, 20.0);
        bbd.set_bandwidth(9000.0);
        Self {
            fs,
            tape: DelayLine::new((fs * 1.0) as usize),
            rec,
            play_lp,
            fb: 0.0,
            wow: Wander::new(fs, 0.8, 7),
            flutter: Lfo::default(),
            spring_ap: [Diffuser::new(fs, 0.9, 0.65), Diffuser::new(fs, 1.7, 0.65), Diffuser::new(fs, 2.9, 0.6), Diffuser::new(fs, 4.3, 0.6)],
            spring: [DelayLine::new((fs * 0.06) as usize), DelayLine::new((fs * 0.06) as usize)],
            sp_lp,
            sp_fb: 0.0,
            bbd,
            clfo: Lfo::default(),
            eq: [[Biquad::identity(); 2]; 2],
            eq_key: (99.0, 99.0),
            v,
            rate: Smooth::new(fs, 150.0, 120.0),
            mix: Smooth::new(fs, 20.0, 0.4),
            out: Smooth::new(fs, 20.0, 1.0),
        }
    }
    fn reset(&mut self) {
        self.tape.reset();
        self.spring.iter_mut().for_each(|s| s.reset());
        self.spring_ap.iter_mut().for_each(|s| s.reset());
        self.bbd.reset();
        self.fb = 0.0;
        self.sp_fb = 0.0;
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        let k = (v[BASS] as f64, v[TREBLE] as f64);
        if k != self.eq_key {
            for c in &mut self.eq {
                c[0].set_low_shelf(self.fs, 150.0, 0.7, k.0);
                c[1].set_high_shelf(self.fs, 3000.0, 0.7, k.1);
            }
            self.eq_key = k;
        }
        self.rate.set(v[RATE] as f64);
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let fs = self.fs;
        let mode = (self.v[MODE].round() as usize).min(6);
        let heads = MODES[mode];
        let inten = self.v[INTENSITY] as f64 * 1.05;
        let echo = self.v[ECHO] as f64;
        let rev = self.v[REVERB] as f64;
        let chorus = self.v[CHORUS] > 0.5;
        let sp_len = [37.0 * 0.001 * fs, 43.0 * 0.001 * fs];
        let mut peak = 0.0f64;
        for k in 0..n {
            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            // tape: record head, three playback heads at 1x, 2x, 3x the repeat rate
            let d1 = self.rate.next() * 0.001 * fs * (1.0 + 0.003 * self.wow.next() + 0.0008 * self.flutter.next(fs, 6.0));
            let rec = (self.rec.lp(x + self.fb * inten) * 1.5).tanh() / 1.5;
            self.tape.write(rec);
            let mut e = 0.0;
            let mut cnt = 0.0f64;
            for (h, on) in heads.iter().enumerate() {
                if *on {
                    e += self.tape.read(d1 * (h + 1) as f64);
                    cnt += 1.0;
                }
            }
            let e = if cnt > 0.0 { self.play_lp.lp(e / cnt.sqrt()) } else { 0.0 };
            self.fb = e;
            // spring: dispersive allpasses into two springs
            let mut s = x * 0.6 + e * 0.4 + self.sp_fb * 0.72;
            for ap in self.spring_ap.iter_mut() {
                s = ap.process(s);
            }
            let a = self.spring[0].read(sp_len[0]);
            let b = self.spring[1].read(sp_len[1]);
            self.spring[0].write(s);
            self.spring[1].write(s);
            self.sp_fb = self.sp_lp.lp(0.5 * (a + b));
            let wet = e * echo + 0.5 * (a + b) * rev * 1.4;
            // dual outputs: left plain, right through the chorus
            let wl = cascade(&mut self.eq[0], wet);
            let wr0 = if chorus {
                let m = self.clfo.next(fs, 0.6);
                self.bbd.process(wet, (4.0 + 2.0 * m) * 0.001 * fs)
            } else {
                wet
            };
            let wr = cascade(&mut self.eq[1], wr0);
            let mx = self.mix.next();
            let o = self.out.next();
            for (ci, c) in chans.iter_mut().enumerate() {
                let w = if nch == 1 { 0.5 * (wl + wr) } else if ci == 0 { wl } else { wr };
                let y = (c[k] as f64 * (1.0 - mx) + w * mx) * o;
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
        Preset { name: "Spring Only (the favourite)", values: &[(MODE, 6.0), (REVERB, 0.6), (ECHO, 0.0), (MIX, 0.35)] },
        Preset { name: "Wide Vocal Echo + Spring", values: &[(MODE, 3.0), (RATE, 110.0), (INTENSITY, 0.3), (ECHO, 0.45), (REVERB, 0.35), (MIX, 0.35)] },
        Preset { name: "Vibraphone Through the Echo", values: &[(MODE, 5.0), (RATE, 150.0), (INTENSITY, 0.45), (ECHO, 0.6), (REVERB, 0.3), (MIX, 0.45)] },
        Preset { name: "Slapback", values: &[(MODE, 0.0), (RATE, 90.0), (INTENSITY, 0.1), (ECHO, 0.6), (REVERB, 0.1), (CHORUS, 0.0)] },
        Preset { name: "Big Room Alternative", values: &[(MODE, 4.0), (RATE, 70.0), (INTENSITY, 0.55), (ECHO, 0.35), (REVERB, 0.6), (MIX, 0.4)] },
        Preset { name: "Runaway Self-Oscillation", values: &[(MODE, 5.0), (RATE, 200.0), (INTENSITY, 0.98), (ECHO, 0.7), (MIX, 0.45)] },
        Preset { name: "Dark Drums Echo", values: &[(MODE, 2.0), (RATE, 180.0), (INTENSITY, 0.35), (TREBLE, -6.0), (ECHO, 0.5), (REVERB, 0.25)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "CHORUS ECHO",
        subtitle: ("AFTER THE ROLAND RE-501", "TAPE ECHO · SPRING · CHORUS"),
        accent: ui::OCEAN,
        size: (W, H),
        items: vec![

            Item::Selector { p: MODE, x: 90.0, y: 160.0, label: "MODE", positions: &["1", "2", "3", "1+2", "2+3", "ALL", "REV"], tip: "Which playback heads sound (at 1x, 2x and 3x the repeat rate). REV: spring reverb only." },
            Item::Knob { p: RATE, x: 220.0, y: 160.0, r: 38.0, label: "REPEAT RATE", tip: "Tape speed: the gap between repeats.", active: None },
            Item::Knob { p: INTENSITY, x: 340.0, y: 160.0, r: 38.0, label: "INTENSITY", tip: "Repeats. Near the top it runs away into self-oscillation.", active: None },
            Item::Knob { p: ECHO, x: 460.0, y: 160.0, r: 30.0, label: "ECHO VOLUME", tip: "Echo level.", active: None },
            Item::Knob { p: REVERB, x: 565.0, y: 160.0, r: 30.0, label: "REVERB VOLUME", tip: "The spring reverb.", active: None },
            Item::Divider { y: 264.0 },
            Item::Switch { p: CHORUS, x: 80.0, y: 330.0, labels: ["OFF", "CHORUS"], tip: "Chorus on the second output: one side plain, one side chorused, for width." },
            Item::Knob { p: BASS, x: 200.0, y: 330.0, r: 22.0, label: "BASS", tip: "Bass on the effect.", active: None },
            Item::Knob { p: TREBLE, x: 290.0, y: 330.0, r: 22.0, label: "TREBLE", tip: "Treble on the effect.", active: None },
            Item::Knob { p: MIX, x: 420.0, y: 330.0, r: 22.0, label: "MIX", tip: "Dry vs effect.", active: None },
            Item::Knob { p: OUTPUT, x: 550.0, y: 330.0, r: 22.0, label: "OUTPUT", tip: "Output level.", active: None },

        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Chorus Echo", class_id: *b"JAChorusEchoPQD1", vendor: "PEQUOD Antonoff",
    categories: [Fx, Delay]
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
    fn heads_land_at_multiples() {
        let p = with(defaults(), &[(MODE, 2.0), (RATE, 100.0), (INTENSITY, 0.0), (REVERB, 0.0), (CHORUS, 0.0), (MIX, 1.0)]);
        let mut x = vec![0.0f32; 96000];
        for v in x.iter_mut().skip(48000).take(48) {
            *v = 0.4;
        }
        let mut input = vec![x];
        run(&p, &mut input);
        let pk = input[0].iter().enumerate().skip(48200).max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap()).unwrap().0 as f64 / 48.0 - 1000.0;
        println!("head 3 at {pk:.1} ms (expect ~300)");
        assert!((pk - 300.0).abs() < 8.0);
    }

}
