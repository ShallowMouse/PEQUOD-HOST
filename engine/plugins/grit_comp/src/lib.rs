//! Grit Comp — saturating "belter" compressor for that 50s/early-60s vocal
//! edge, now on the shared Studio Two framework.
//!
//! v1.9: EDGE can follow the pitch of the source. In FOLLOW mode the edge band
//! sits on a chosen harmonic of the note being sung or played (tracked with
//! the shared YIN pitch follower), so the grit forms on the same partial as the
//! melody moves. Two parameters appended at the end, so saved projects load as before.
//!
//! Same plugin ID, parameter IDs and parameter order as the standalone
//! versions, so existing projects keep their settings.

use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
use vintage::dsp::Smooth;
use vintage::nih_plug_egui::EguiState;
use vintage::params::*;
use vintage::ui::{self, ActiveWhen, Item, MeterKind, Panel, Preset};
use vintage::pitch::Pitch;
use vintage::{VintageDsp, VintageParams};

mod engine;
use engine::{Processor, Settings};

const W: u32 = 640;
const H: u32 = 670;

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum BreakupMode {
    #[name = "Level"]
    Level,
    #[name = "Gain Reduction"]
    GainReduction,
}

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum EdgeTrack {
    #[name = "Fixed"]
    Fixed,
    #[name = "Follow pitch"]
    Follow,
}

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Makeup {
    #[name = "Manual"]
    Manual,
    #[name = "Auto"]
    Auto,
}

#[derive(Params)]
pub struct GritParams {
    #[persist = "editor-state-v4"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "grit"]
    grit: FloatParam,
    #[id = "edge"]
    edge: FloatParam,
    #[id = "edgefreq"]
    edge_freq: FloatParam,
    #[id = "bias"]
    bias: FloatParam,
    #[id = "comp"]
    compress: FloatParam,
    #[id = "lowcut"]
    low_cut: FloatParam,
    #[id = "highcut"]
    high_cut: FloatParam,
    #[id = "mix"]
    mix: FloatParam,
    #[id = "output"]
    output: FloatParam,
    #[id = "onset"]
    onset: FloatParam,
    #[id = "mode"]
    mode: EnumParam<BreakupMode>,
    #[id = "ratio"]
    ratio: FloatParam,
    #[id = "knee"]
    knee: FloatParam,
    #[id = "attack"]
    attack: FloatParam,
    #[id = "release"]
    release: FloatParam,
    #[id = "opto"]
    opto: FloatParam,
    #[id = "schpf"]
    sc_hpf: FloatParam,
    #[id = "makeup"]
    makeup: FloatParam,
    #[id = "automakeup"]
    auto_makeup: EnumParam<Makeup>,
    // added in 1.9 (appended)
    #[id = "edgefollow"]
    edge_follow: EnumParam<EdgeTrack>,
    #[id = "edgeharm"]
    edge_harm: IntParam,
}

impl Default for GritParams {
    fn default() -> Self {
        let d = Settings::default();
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            grit: db("Grit", d.grit_db as f32, 0.0, 30.0),
            edge: db("Edge", d.edge_db as f32, 0.0, 12.0),
            edge_freq: hz("Edge Freq", d.edge_hz as f32, 800.0, 5000.0),
            bias: pct("Tube Bias", d.bias as f32),
            // same 0..1 parameter as before (old sessions keep their setting),
            // now shown as the threshold it sets
            compress: FloatParam::new("Threshold", d.compress as f32, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_value_to_string(Arc::new(|v| format!("{:.1} dB", -36.0 * v)))
                .with_string_to_value(Arc::new(|s| {
                    s.trim().trim_end_matches("dB").trim().parse::<f32>().ok().map(|db| (-db / 36.0).clamp(0.0, 1.0))
                })),
            low_cut: hz("Low Cut", d.low_cut_hz as f32, 20.0, 400.0),
            high_cut: hz("High Cut", d.high_cut_hz as f32, 2000.0, 20000.0),
            mix: pct("Mix", d.mix as f32),
            output: db("Output", 0.0, -24.0, 12.0),
            onset: db("GR Onset", d.gr_onset_db as f32, 0.5, 12.0),
            mode: EnumParam::new("Breakup Mode", BreakupMode::Level),
            ratio: FloatParam::new("Ratio", d.ratio as f32, FloatRange::Skewed { min: 1.0, max: 20.0, factor: FloatRange::skew_factor(-1.5) })
                .with_step_size(0.1)
                .with_value_to_string(Arc::new(|v| format!("{v:.1}:1")))
                .with_string_to_value(Arc::new(|s| s.trim().trim_end_matches(":1").trim().parse().ok())),
            knee: db("Knee", d.knee_db as f32, 0.0, 18.0),
            attack: ms("Attack", d.attack_ms as f32, 0.2, 100.0),
            release: ms("Release", d.release_ms as f32, 20.0, 2000.0),
            opto: pct("Opto", d.opto as f32),
            sc_hpf: hz("Sidechain HPF", d.sc_hpf_hz as f32, 20.0, 500.0),
            makeup: db("Make-up", d.makeup_db as f32, -12.0, 24.0),
            auto_makeup: EnumParam::new("Auto Make-up", Makeup::Auto),
            edge_follow: EnumParam::new("Edge Follow", EdgeTrack::Fixed),
            edge_harm: IntParam::new("Edge Harmonic", 6, IntRange::Linear { min: 1, max: 16 }),
        }
    }
}

// linked order (matches the original plugin's saved-state layout)
const GRIT: usize = 0;
const EDGE: usize = 1;
const EDGE_F: usize = 2;
const BIAS: usize = 3;
const COMP: usize = 4;
const LOW: usize = 5;
const HIGH: usize = 6;
const MIX: usize = 7;
const OUTPUT: usize = 8;
const ONSET: usize = 9;
const MODE: usize = 10;
// added in 1.3 (appended, so older saved values still line up)
const RATIO: usize = 11;
const KNEE: usize = 12;
const ATTACK: usize = 13;
const RELEASE: usize = 14;
const OPTO: usize = 15;
const SCHPF: usize = 16;
const MAKEUP: usize = 17;
const AUTO: usize = 18;
// added in 1.9
const FOLLOW: usize = 19;
const HARM: usize = 20;

impl VintageParams for GritParams {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.grit.as_ptr(),
            self.edge.as_ptr(),
            self.edge_freq.as_ptr(),
            self.bias.as_ptr(),
            self.compress.as_ptr(),
            self.low_cut.as_ptr(),
            self.high_cut.as_ptr(),
            self.mix.as_ptr(),
            self.output.as_ptr(),
            self.onset.as_ptr(),
            self.mode.as_ptr(),
            self.ratio.as_ptr(),
            self.knee.as_ptr(),
            self.attack.as_ptr(),
            self.release.as_ptr(),
            self.opto.as_ptr(),
            self.sc_hpf.as_ptr(),
            self.makeup.as_ptr(),
            self.auto_makeup.as_ptr(),
            self.edge_follow.as_ptr(),
            self.edge_harm.as_ptr(),
        ]
    }
}

pub struct GritDsp {
    proc: Processor,
    s: Settings,
    // per-sample smoothing of the continuous controls
    grit: Smooth,
    bias: Smooth,
    comp: Smooth,
    mix: Smooth,
    out: Smooth,
    // EDGE FOLLOW
    fs: f64,
    pitch: Pitch,
    follow: bool,
    harm: f64,
    knob_hz: f64,
    edge_now: f64,
    since_pitch: f64,
    k: usize,
    locked: bool,
    refresh: usize,
}

/// Where the edge band goes for a tracked fundamental: that harmonic, kept inside a sensible range.
pub fn follow_hz(f0: f64, harmonic: f64) -> f64 {
    (f0 * harmonic).clamp(300.0, 8000.0)
}

impl VintageDsp for GritDsp {
    const METERS: usize = 3;

    fn new(sample_rate: f32, channels: usize) -> Self {
        let fs = sample_rate as f64;
        let s = Settings::default();
        Self {
            proc: Processor::new(fs, channels),
            grit: Smooth::new(fs, 20.0, s.grit_db),
            bias: Smooth::new(fs, 20.0, s.bias),
            comp: Smooth::new(fs, 20.0, s.compress),
            mix: Smooth::new(fs, 20.0, s.mix),
            out: Smooth::new(fs, 20.0, s.output_db),
            fs,
            pitch: Pitch::new(fs),
            follow: false,
            harm: 6.0,
            knob_hz: s.edge_hz,
            edge_now: s.edge_hz,
            since_pitch: 10.0,
            k: 0,
            locked: false,
            refresh: 0,
            s,
        }
    }

    fn latency(&self) -> u32 {
        engine::LATENCY as u32
    }

    fn reset(&mut self) {
        self.proc.reset();
        self.pitch.reset();
        self.edge_now = self.knob_hz;
        self.since_pitch = 10.0;
        self.s.edge_hz = self.edge_now;
        self.proc.update(&self.s);
    }

    fn set_params(&mut self, v: &[f32]) {
        let f = |i: usize| v[i] as f64;
        self.s.edge_db = f(EDGE);
        self.knob_hz = f(EDGE_F);
        self.follow = v.len() > FOLLOW && v[FOLLOW] > 0.5;
        self.harm = if v.len() > HARM { f(HARM).max(1.0) } else { 6.0 };
        self.s.edge_hz = if self.follow { self.edge_now } else { self.knob_hz };
        self.s.low_cut_hz = f(LOW);
        self.s.high_cut_hz = f(HIGH);
        self.s.gr_onset_db = f(ONSET);
        self.s.gr_mode = v[MODE] > 0.5;
        self.s.ratio = f(RATIO);
        self.s.knee_db = f(KNEE);
        self.s.attack_ms = f(ATTACK);
        self.s.release_ms = f(RELEASE);
        self.s.opto = f(OPTO);
        self.s.sc_hpf_hz = f(SCHPF);
        self.s.makeup_db = f(MAKEUP);
        self.s.auto_makeup = v[AUTO] > 0.5;
        self.grit.set(f(GRIT));
        self.bias.set(f(BIAS));
        self.comp.set(f(COMP));
        self.mix.set(f(MIX));
        self.out.set(f(OUTPUT));
        self.proc.update(&self.s);
    }

    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(8);
        let mut frame = [0.0f32; 8];
        let mut gr = 0.0f64;
        // EDGE FOLLOW: glide the edge band (in octaves) toward the tracked harmonic every 32 samples
        let glide = (-32.0 / (self.fs * 0.03)).exp();
        let hold = 0.5;
        for k in 0..n {
            if self.follow {
                let mut x = 0.0f64;
                for c in 0..nch {
                    x += chans[c][k] as f64;
                }
                self.pitch.push(x / nch.max(1) as f64);
                // look for the pitch afresh several times a second, so a new note (sung legato or
                // picked) is never followed as a sub-harmonic of the one before
                self.refresh += 1;
                if self.refresh as f64 >= 0.15 * self.fs {
                    self.refresh = 0;
                    self.pitch.new_note();
                }
                self.since_pitch += 1.0 / self.fs;
                if self.k == 0 {
                    let target = match self.pitch.freq {
                        Some(f0) => {
                            self.since_pitch = 0.0;
                            self.locked = true;
                            follow_hz(f0, self.harm)
                        }
                        // between notes and on consonants: hold, then drift back to the knob
                        None if self.since_pitch < hold => self.edge_now,
                        None => {
                            self.locked = false;
                            self.knob_hz
                        }
                    };
                    let l = target.ln() + glide * (self.edge_now.ln() - target.ln());
                    self.edge_now = l.exp();
                    // only re-tune the filters when the band has moved audibly (0.2%)
                    if (self.edge_now / self.s.edge_hz - 1.0).abs() > 0.002 {
                        self.s.edge_hz = self.edge_now;
                        self.proc.update(&self.s);
                    }
                }
                self.k = (self.k + 1) % 32;
            } else {
                self.locked = false;
                if (self.edge_now - self.knob_hz).abs() > 1e-6 {
                    self.edge_now = self.knob_hz;
                }
            }
            self.s.grit_db = self.grit.next();
            self.s.bias = self.bias.next();
            self.s.compress = self.comp.next();
            self.s.mix = self.mix.next();
            self.s.output_db = self.out.next();
            for c in 0..nch {
                frame[c] = chans[c][k];
            }
            self.proc.process_frame(&mut frame[..nch], &self.s);
            for c in 0..nch {
                chans[c][k] = frame[c];
            }
            gr = gr.max(-self.proc.gain_reduction_db());
        }
        meters[0] = gr as f32;
        meters[1] = self.proc.take_sat_peak() as f32;
        meters[2] = if self.follow && self.locked { 1.0 } else { 0.0 };
    }
}

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Stand By Me Belter", values: &[(GRIT, 14.0), (EDGE, 6.0), (EDGE_F, 2000.0), (COMP, 0.45)] },
        Preset { name: "Rockabilly Shout", values: &[(GRIT, 20.0), (EDGE, 8.0), (EDGE_F, 2600.0), (BIAS, 0.45), (COMP, 0.55), (LOW, 160.0), (HIGH, 7500.0)] },
        Preset { name: "Gentle Valve Warmth", values: &[(GRIT, 6.0), (EDGE, 3.0), (BIAS, 0.5), (COMP, 0.3), (HIGH, 14000.0)] },
        Preset { name: "AM Radio Squash", values: &[(GRIT, 22.0), (EDGE, 9.0), (EDGE_F, 1800.0), (COMP, 0.8), (LOW, 300.0), (HIGH, 4500.0), (OUTPUT, -2.0)] },
        Preset { name: "GR: Grit When Pushed", values: &[(MODE, 1.0), (GRIT, 16.0), (COMP, 0.35), (ONSET, 3.0)] },
        Preset { name: "GR: Gentle Comp, Early Grit", values: &[(MODE, 1.0), (GRIT, 12.0), (COMP, 0.22), (ONSET, 1.0), (EDGE, 5.0)] },
        Preset { name: "Parallel Grit", values: &[(GRIT, 24.0), (EDGE, 8.0), (COMP, 0.6), (MIX, 0.4)] },
        Preset { name: "Fast Peak Catcher", values: &[(GRIT, 10.0), (COMP, 0.35), (RATIO, 8.0), (KNEE, 2.0), (ATTACK, 1.0), (RELEASE, 60.0), (OPTO, 0.1)] },
        Preset { name: "Slow Opto Levelling", values: &[(GRIT, 12.0), (COMP, 0.5), (RATIO, 3.0), (KNEE, 12.0), (ATTACK, 20.0), (RELEASE, 300.0), (OPTO, 0.8)] },
        Preset { name: "Follow the Voice (6th harmonic)", values: &[(GRIT, 16.0), (EDGE, 8.0), (COMP, 0.5), (FOLLOW, 1.0), (HARM, 6.0)] },
        Preset { name: "Follow, Low Edge (3rd harmonic)", values: &[(GRIT, 16.0), (EDGE, 7.0), (COMP, 0.5), (FOLLOW, 1.0), (HARM, 3.0), (LOW, 80.0)] },
        Preset { name: "Follow, Bright Bite (10th harmonic)", values: &[(GRIT, 18.0), (EDGE, 9.0), (COMP, 0.55), (FOLLOW, 1.0), (HARM, 10.0)] },
        Preset { name: "Follow on Guitar (4th harmonic)", values: &[(GRIT, 20.0), (EDGE, 8.0), (COMP, 0.4), (FOLLOW, 1.0), (HARM, 4.0), (LOW, 90.0), (HIGH, 7000.0)] },
        Preset { name: "Brick-Wall Shout", values: &[(GRIT, 20.0), (EDGE, 7.0), (COMP, 0.7), (RATIO, 20.0), (KNEE, 0.0), (ATTACK, 0.5), (RELEASE, 120.0), (OPTO, 0.2), (SCHPF, 150.0)] },
    ]
}

fn panel() -> Panel {
    let gr_only = Some(ActiveWhen { p: MODE, min: 1.0, max: 1.0 });
    Panel {
        title: "GRIT COMP",
        subtitle: ("VOCAL SATURATION", "& LEVELLING AMPLIFIER"),
        accent: ui::MUSTARD,
        size: (W, H),
        items: vec![
            Item::Knob { p: GRIT, x: 130.0, y: 172.0, r: 50.0, label: "GRIT",
                tip: "How hard the tube is driven.\nLEVEL mode: loud notes break up sooner; quiet singing stays clean.\nGR mode: the drive reached about 6 dB above the GR Onset.", active: None },
            Item::Knob { p: COMP, x: 510.0, y: 172.0, r: 50.0, label: "THRESHOLD",
                tip: "Where compression starts: 0 dB (off) down to -36 dB. Turn it down for more squash.", active: None },
            Item::Meter { m: 0, x: 222.0, y: 92.0, w: 196.0, h: 136.0, kind: MeterKind::GainReduction, caption: "GAIN REDUCTION" },
            Item::Lamp { m: 1, x: 236.0, y: 264.0, label: "BREAKUP", lo: 0.6, hi: 1.8 },
            Item::Switch { p: MODE, x: 380.0, y: 264.0, labels: ["LEVEL", "GR"],
                tip: "Breakup mode\n\nLEVEL: loud notes break up (tube before the compressor).\nGR: the harder the compressor squashes, the more it breaks up (compressor before the tube). GR ONSET sets how much gain reduction it takes to start breaking up; below that it stays clean." },
            Item::Divider { y: 284.0 },
            Item::Knob { p: RATIO, x: 64.0, y: 340.0, r: 22.0, label: "RATIO", tip: "1:1 (none) to 20:1 (limiting). The original fixed setting was 4:1.", active: None },
            Item::Knob { p: KNEE, x: 148.0, y: 340.0, r: 22.0, label: "KNEE", tip: "0 dB = hard knee; wider = compression eases in gradually. Original: 6 dB.", active: None },
            Item::Knob { p: ATTACK, x: 232.0, y: 340.0, r: 22.0, label: "ATTACK", tip: "How fast it grabs. Short catches consonants and peaks; longer lets the front of each word through. Original: 8 ms.", active: None },
            Item::Knob { p: RELEASE, x: 316.0, y: 340.0, r: 22.0, label: "RELEASE", tip: "How fast it lets go. Short = more energy and pumping; long = smoother. Original: 80 ms.", active: None },
            Item::Knob { p: OPTO, x: 400.0, y: 340.0, r: 22.0, label: "OPTO",
                tip: "Blend of a slow, opto-like recovery (7.5x the attack, 15x the release) with the fast detector. Up = smoother, more program-dependent levelling. Original: 40%.", active: None },
            Item::Knob { p: SCHPF, x: 484.0, y: 340.0, r: 22.0, label: "SC HPF", tip: "High-pass on the compressor's detector only, so low end and plosives don't pump it. 20 Hz = off.", active: None },
            Item::Knob { p: MAKEUP, x: 568.0, y: 340.0, r: 22.0, label: "MAKE-UP", tip: "Gain after the compressor (on top of AUTO if that's on).", active: None },
            Item::Switch { p: AUTO, x: 540.0, y: 264.0, labels: ["MANUAL", "AUTO"],
                tip: "AUTO adds half of the gain the threshold and ratio take away, as the original did. MAN leaves it all to the MAKE-UP knob." },
            Item::Divider { y: 396.0 },
            Item::Knob { p: EDGE, x: 64.0, y: 448.0, r: 22.0, label: "EDGE", tip: "Upper-mid push into the tube (removed again after it), so the grit forms where the vocal edge lives.", active: None },
            Item::Knob { p: EDGE_F, x: 137.0, y: 448.0, r: 22.0, label: "EDGE FREQ", tip: "Centre of the edge band. In FOLLOW mode this is where the band rests when no pitch is found.", active: None },
            Item::Knob { p: BIAS, x: 210.0, y: 448.0, r: 22.0, label: "TUBE BIAS", tip: "Asymmetry: more = warmer, more 2nd-harmonic character.", active: None },
            Item::Knob { p: LOW, x: 283.0, y: 448.0, r: 22.0, label: "LOW CUT", tip: "High-pass before the tube, keeps the low end from muddying the grit.", active: None },
            Item::Knob { p: HIGH, x: 356.0, y: 448.0, r: 22.0, label: "HIGH CUT", tip: "Rolls off the top for that darker period sound.", active: None },
            Item::Knob { p: MIX, x: 429.0, y: 448.0, r: 22.0, label: "MIX", tip: "Blend with the dry vocal (latency compensated).", active: None },
            Item::Knob { p: ONSET, x: 502.0, y: 448.0, r: 22.0, label: "GR ONSET",
                tip: "GR mode only: how much gain reduction before the tube starts to break up. Lower = grit with gentler compression.", active: gr_only },
            Item::Knob { p: OUTPUT, x: 575.0, y: 448.0, r: 22.0, label: "OUTPUT", tip: "Final level.", active: None },
            Item::Divider { y: 504.0 },
            Item::Text { x: 120.0, y: 528.0, text: "EDGE FOLLOW", size: 12.0, align: vintage::nih_plug_egui::egui::Align2::CENTER_CENTER },
            Item::Switch { p: FOLLOW, x: 120.0, y: 560.0, labels: ["FIXED", "FOLLOW"],
                tip: "FIXED: the edge band stays on EDGE FREQ. FOLLOW: it tracks the pitch of the voice or instrument and sits on the chosen harmonic, so the grit forms on the same partial as the melody moves." },
            Item::Knob { p: HARM, x: 290.0, y: 556.0, r: 22.0, label: "HARMONIC",
                tip: "FOLLOW mode: which harmonic of the note the edge band sits on (1 = the fundamental). For a voice around 200 Hz, 6 puts it near 1.2 kHz, 10 near 2 kHz.", active: Some(ActiveWhen { p: FOLLOW, min: 1.0, max: 1.0 }) },
            Item::Lamp { m: 2, x: 400.0, y: 545.0, label: "PITCH LOCK", lo: 0.0, hi: 1.0 },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, GritParams, GritDsp, panel,
    name: "Grit Comp", class_id: *b"GritCompDIYVox01", vendor: "PEQUOD",
    categories: [Fx, Dynamics, Distortion]
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processes_and_meters() {
        let mut d = GritDsp::new(48000.0, 1);
        let defaults = [14.0, 6.0, 2000.0, 0.3, 0.45, 110.0, 9000.0, 1.0, 0.0, 2.0, 0.0, 4.0, 6.0, 8.0, 80.0, 0.4, 20.0, 0.0, 1.0];
        d.set_params(&defaults);
        let mut x: Vec<f32> = (0..48000).map(|k| 0.5 * (k as f32 * 0.09).sin()).collect();
        let mut m = [0.0f32; 3];
        d.process(&mut [&mut x[..]], &mut m);
        assert!(x.iter().all(|v| v.is_finite()));
        assert!(m[0] > 1.0, "compressor should be working on a loud note: {}", m[0]);
        assert!(m[1] > 0.6, "tube should be breaking up: {}", m[1]);
    }

    #[test]
    fn edge_follows_the_note() {
        let mut v = vec![14.0f32, 6.0, 2000.0, 0.3, 0.45, 110.0, 9000.0, 1.0, 0.0, 2.0, 0.0, 4.0, 6.0, 8.0, 80.0, 0.4, 20.0, 0.0, 1.0, 1.0, 8.0];
        let mut d = GritDsp::new(48000.0, 1);
        d.set_params(&v);
        d.reset();
        let run = |d: &mut GritDsp, f: f64| -> f32 {
            let mut m = [0.0f32; 3];
            let mut x: Vec<f32> = (0..24000).map(|k| (0.3 * (2.0 * std::f64::consts::PI * f * k as f64 / 48000.0).sin()) as f32).collect();
            for c in x.chunks_mut(512) {
                d.process(&mut [c], &mut m);
            }
            m[2]
        };
        run(&mut d, 220.0);
        let a = d.edge_now;
        let lock = run(&mut d, 330.0);
        let b = d.edge_now;
        println!("edge at 220 Hz x8: {a:.0} Hz; at 330 Hz x8: {b:.0} Hz; lock {lock}");
        assert!((a - 1760.0).abs() < 30.0 && (b - 2640.0).abs() < 40.0 && lock > 0.5);
        // FIXED: back on the knob
        v[19] = 0.0;
        d.set_params(&v);
        run(&mut d, 220.0);
        assert!((d.s.edge_hz - 2000.0).abs() < 1e-6);
    }
}
