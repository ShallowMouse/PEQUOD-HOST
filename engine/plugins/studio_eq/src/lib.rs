//! Studio EQ — vintage broadcast/studio channel equaliser.
//!
//! Broad, musical bands in the spirit of 60s desk EQ: a bass shelf, a wide
//! presence bell, and a treble shelf, plus low/high cut filters and a valve
//! output stage that can add a touch of colour.

use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
use vintage::dsp::*;
use vintage::nih_plug_egui::EguiState;
use vintage::params::*;
use vintage::ui::{self, Preset, Item, Panel};
use vintage::{VintageDsp, VintageParams};

const W: u32 = 640;
const H: u32 = 432;

#[derive(Params)]
pub struct EqParams {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "bass"]
    bass: FloatParam,
    #[id = "bassf"]
    bass_freq: FloatParam,
    #[id = "pres"]
    presence: FloatParam,
    #[id = "presf"]
    presence_freq: FloatParam,
    #[id = "treble"]
    treble: FloatParam,
    #[id = "treblef"]
    treble_freq: FloatParam,
    #[id = "lowcut"]
    low_cut: FloatParam,
    #[id = "highcut"]
    high_cut: FloatParam,
    #[id = "drive"]
    drive: FloatParam,
    #[id = "output"]
    output: FloatParam,
}

impl Default for EqParams {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            bass: db("Bass", 0.0, -12.0, 12.0),
            bass_freq: hz("Bass Freq", 110.0, 40.0, 400.0),
            presence: db("Presence", 0.0, -10.0, 10.0),
            presence_freq: hz("Presence Freq", 2800.0, 800.0, 8000.0),
            treble: db("Treble", 0.0, -12.0, 12.0),
            treble_freq: hz("Treble Freq", 10000.0, 4000.0, 16000.0),
            low_cut: hz("Low Cut", 20.0, 20.0, 400.0),
            high_cut: hz("High Cut", 20000.0, 3000.0, 20000.0),
            drive: pct("Valve Drive", 0.15),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

const BASS: usize = 0;
const BASS_F: usize = 1;
const PRES: usize = 2;
const PRES_F: usize = 3;
const TREBLE: usize = 4;
const TREBLE_F: usize = 5;
const LOW_CUT: usize = 6;
const HIGH_CUT: usize = 7;
const DRIVE: usize = 8;
const OUTPUT: usize = 9;

impl VintageParams for EqParams {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.bass.as_ptr(),
            self.bass_freq.as_ptr(),
            self.presence.as_ptr(),
            self.presence_freq.as_ptr(),
            self.treble.as_ptr(),
            self.treble_freq.as_ptr(),
            self.low_cut.as_ptr(),
            self.high_cut.as_ptr(),
            self.drive.as_ptr(),
            self.output.as_ptr(),
        ]
    }
}

#[derive(Clone)]
struct Chan {
    bands: [Biquad; 3],
    lc: [Biquad; 2],
    hc: [Biquad; 2],
    os: Oversampler,
    dc: DcBlock,
}

pub struct EqDsp {
    fs: f64,
    ch: Vec<Chan>,
    drive: Smooth,
    out: Smooth,
    last: Vec<f32>,
}

impl VintageDsp for EqDsp {
    const METERS: usize = 0;

    fn new(sample_rate: f32, channels: usize) -> Self {
        let fs = sample_rate as f64;
        let c = Chan {
            bands: [Biquad::identity(); 3],
            lc: [Biquad::identity(); 2],
            hc: [Biquad::identity(); 2],
            os: Oversampler::new(),
            dc: DcBlock::new(fs),
        };
        Self { fs, ch: vec![c; channels.max(1)], drive: Smooth::new(fs, 30.0, 0.15), out: Smooth::new(fs, 30.0, 1.0), last: Vec::new() }
    }

    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }

    fn reset(&mut self) {
        for c in &mut self.ch {
            c.bands.iter_mut().chain(c.lc.iter_mut()).chain(c.hc.iter_mut()).for_each(|b| b.reset());
            c.os.reset();
            c.dc.reset();
        }
    }

    fn set_params(&mut self, v: &[f32]) {
        self.drive.set(v[DRIVE] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
        if self.last.as_slice() == &v[..8] {
            return;
        }
        self.last = v[..8].to_vec();
        let f = |i: usize| v[i] as f64;
        let fs = self.fs;
        for c in &mut self.ch {
            c.bands[0].set_low_shelf(fs, f(BASS_F), 0.6, f(BASS));
            // wide bell, slightly narrower the more it's boosted (proportional Q)
            let q = 0.5 + 0.06 * f(PRES).abs();
            c.bands[1].set_peak(fs, f(PRES_F), q, f(PRES));
            c.bands[2].set_high_shelf(fs, f(TREBLE_F), 0.6, f(TREBLE));
            // filters fully out of the way at their end stops
            if f(LOW_CUT) <= 20.5 {
                c.lc = [Biquad::identity(); 2];
            } else {
                c.lc[0].set_highpass(fs, f(LOW_CUT), 0.54);
                c.lc[1].set_highpass(fs, f(LOW_CUT), 1.31);
            }
            if f(HIGH_CUT) >= 19900.0 {
                c.hc = [Biquad::identity(); 2];
            } else {
                c.hc[0].set_lowpass(fs, f(HIGH_CUT), 0.54);
                c.hc[1].set_lowpass(fs, f(HIGH_CUT), 1.31);
            }
        }
    }

    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len().min(self.ch.len());
        for k in 0..n {
            let dr = self.drive.next();
            let o = self.out.next();
            let g = 1.0 + 4.0 * dr;
            for c in 0..nch {
                let st = &mut self.ch[c];
                let mut y = chans[c][k] as f64;
                for b in st.lc.iter_mut().chain(st.bands.iter_mut()).chain(st.hc.iter_mut()) {
                    y = b.process(y);
                }
                y = st.os.process(y, |v| tube(v * g, 0.2) / g);
                y = st.dc.process(y);
                chans[c][k] = (y * o) as f32;
            }
        }
    }
}

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Flat", values: &[] },
        Preset { name: "Vocal Presence", values: &[(PRES, 4.0), (PRES_F, 3200.0), (LOW_CUT, 90.0), (TREBLE, 2.0)] },
        Preset { name: "Bass Weight", values: &[(BASS, 5.0), (BASS_F, 80.0), (PRES, -2.0), (PRES_F, 900.0)] },
        Preset { name: "Bright Acoustic", values: &[(TREBLE, 5.0), (TREBLE_F, 9000.0), (BASS, -2.0), (LOW_CUT, 70.0)] },
        Preset { name: "Telephone", values: &[(LOW_CUT, 350.0), (HIGH_CUT, 3500.0), (PRES, 6.0), (PRES_F, 1600.0), (DRIVE, 0.5)] },
        Preset { name: "Warm & Dark", values: &[(BASS, 2.0), (TREBLE, -5.0), (TREBLE_F, 7000.0), (DRIVE, 0.35)] },
    ]
}

fn panel() -> Panel {
    let big = 36.0;
    Panel {
        title: "STUDIO EQ",
        subtitle: ("CHANNEL EQUALISER", "BASS · PRESENCE · TREBLE"),
        accent: ui::COBALT,
        size: (W, H),
        items: vec![
            Item::Knob { p: BASS, x: 140.0, y: 150.0, r: big, label: "BASS", tip: "Low shelf boost/cut.", active: None },
            Item::Knob { p: PRESENCE_IDX, x: 320.0, y: 150.0, r: big, label: "PRESENCE", tip: "Wide mid bell for bringing a voice or guitar forward (or tucking it back).", active: None },
            Item::Knob { p: TREBLE, x: 500.0, y: 150.0, r: big, label: "TREBLE", tip: "High shelf boost/cut.", active: None },
            Item::Knob { p: BASS_F, x: 140.0, y: 242.0, r: 14.0, label: "", tip: "Bass shelf frequency.", active: None },
            Item::Knob { p: PRES_F, x: 320.0, y: 242.0, r: 14.0, label: "", tip: "Presence centre frequency.", active: None },
            Item::Knob { p: TREBLE_F, x: 500.0, y: 242.0, r: 14.0, label: "", tip: "Treble shelf frequency.", active: None },
            Item::Divider { y: 284.0 },
            Item::Knob { p: LOW_CUT, x: 140.0, y: 332.0, r: 22.0, label: "LOW CUT", tip: "12 dB/oct high-pass. Fully left = off.", active: None },
            Item::Knob { p: HIGH_CUT, x: 260.0, y: 332.0, r: 22.0, label: "HIGH CUT", tip: "12 dB/oct low-pass. Fully right = off.", active: None },
            Item::Knob { p: DRIVE, x: 380.0, y: 332.0, r: 22.0, label: "VALVE DRIVE", tip: "Colour from the valve output stage.", active: None },
            Item::Knob { p: OUTPUT, x: 500.0, y: 332.0, r: 22.0, label: "OUTPUT", tip: "Final level.", active: None },
        ],
        presets: presets(),
    }
}

const PRESENCE_IDX: usize = PRES;

vintage::vintage_plugin!(
    Plugin, EqParams, EqDsp, panel,
    name: "Studio EQ", class_id: *b"S2StudioEQDIY001", vendor: "PEQUOD Beatles",
    categories: [Fx, Eq]
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn gain_at(freq: f64, v: &[f32]) -> f64 {
        let fs = 48000.0;
        let mut d = EqDsp::new(fs as f32, 1);
        d.set_params(v);
        let n = 48000;
        let a = 0.01;
        let mut x: Vec<f32> = (0..n).map(|k| (a * (2.0 * PI * freq * k as f64 / fs).sin()) as f32).collect();
        d.process(&mut [&mut x[..]], &mut []);
        let tail = &x[n / 2..];
        let rms = (tail.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / tail.len() as f64).sqrt();
        gain_to_db(rms / (a / 2f64.sqrt()))
    }

    const FLAT: [f32; 10] = [0.0, 110.0, 0.0, 2800.0, 0.0, 10000.0, 20.0, 20000.0, 0.0, 0.0];

    #[test]
    fn flat_when_centred() {
        for f in [50.0, 1000.0, 8000.0] {
            let g = gain_at(f, &FLAT);
            assert!(g.abs() < 0.3, "{f} Hz: {g} dB");
        }
    }

    #[test]
    fn bands_do_what_they_say() {
        let mut v = FLAT;
        v[BASS] = 8.0;
        v[PRES] = 6.0;
        v[TREBLE] = -8.0;
        let lo = gain_at(40.0, &v);
        let mid = gain_at(2800.0, &v);
        let hi = gain_at(16000.0, &v);
        println!("bass {lo:.1} presence {mid:.1} treble {hi:.1}");
        assert!(lo > 6.0 && mid > 4.5 && hi < -6.0);
    }
}
