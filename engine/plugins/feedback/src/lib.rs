//! Feedback — guitar/amp feedback machine.
//!
//! Real feedback happens when the amp's sound shakes the guitar string,
//! which feeds the pickup, which feeds the amp... The loop locks onto the
//! note being played or one of its harmonics, and blooms into a sustained
//! howl a moment after the note is struck.
//!
//! Here, the pitch of the incoming note is tracked, and a saturating
//! resonator loop (a string-like delay tuned to that pitch or a chosen
//! harmonic, with the amp's drive and tone inside the loop) is fed from the
//! input. While a note is held, the loop's level blooms in over BLOOM
//! seconds. HOLD keeps it singing after the note stops.

use nih_plug::prelude::*;
use std::sync::{Arc, RwLock};
use vintage::dsp::*;
use vintage::pitch::Pitch;
use vintage::nih_plug_egui::EguiState;
use vintage::params::*;
use vintage::ui::{self, Preset, Item, MeterKind, Panel};
use vintage::{VintageDsp, VintageParams};

const W: u32 = 640;
const H: u32 = 432;

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Harmonic {
    #[name = "Fundamental"]
    Fundamental,
    #[name = "Octave"]
    Octave,
    #[name = "Octave + 5th"]
    Twelfth,
    #[name = "Two Octaves"]
    TwoOctaves,
}

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Hold {
    #[name = "Off"]
    Off,
    #[name = "On"]
    On,
}

#[derive(Params)]
pub struct FeedbackParams {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "amount"]
    amount: FloatParam,
    #[id = "bloom"]
    bloom: FloatParam,
    #[id = "harmonic"]
    harmonic: EnumParam<Harmonic>,
    #[id = "drive"]
    drive: FloatParam,
    #[id = "tone"]
    tone: FloatParam,
    #[id = "threshold"]
    threshold: FloatParam,
    #[id = "mix"]
    mix: FloatParam,
    #[id = "output"]
    output: FloatParam,
    #[id = "hold"]
    hold: EnumParam<Hold>,
}

impl Default for FeedbackParams {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            amount: pct("Feedback", 0.6),
            bloom: secs("Bloom", 1.2, 0.05, 6.0),
            harmonic: EnumParam::new("Harmonic", Harmonic::Octave),
            drive: db("Amp Drive", 9.0, 0.0, 30.0),
            tone: hz("Tone", 3500.0, 800.0, 10000.0),
            threshold: db("Threshold", -40.0, -70.0, -10.0),
            mix: pct("Mix", 0.5),
            output: db("Output", 0.0, -18.0, 6.0),
            hold: EnumParam::new("Hold", Hold::Off),
        }
    }
}

const AMOUNT: usize = 0;
const BLOOM: usize = 1;
const HARMONIC: usize = 2;
const DRIVE: usize = 3;
const TONE: usize = 4;
const THRESHOLD: usize = 5;
const MIX: usize = 6;
const OUTPUT: usize = 7;
const HOLD: usize = 8;

impl VintageParams for FeedbackParams {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.amount.as_ptr(),
            self.bloom.as_ptr(),
            self.harmonic.as_ptr(),
            self.drive.as_ptr(),
            self.tone.as_ptr(),
            self.threshold.as_ptr(),
            self.mix.as_ptr(),
            self.output.as_ptr(),
            self.hold.as_ptr(),
        ]
    }
}

// ------------------------------------------------------------ the machine --

pub struct FeedbackDsp {
    fs: f64,
    pitch: Pitch,
    string: DelayLine,
    loop_lp: OnePole,
    loop_hp: OnePole,
    dc: DcBlock,
    amp_os: [Oversampler; 2],
    amp_dc: [DcBlock; 2],
    amp_tone: [Biquad; 2],
    env: Envelope,
    bloom: f64,
    bloom_up: f64,
    bloom_down: f64,
    period: Smooth,
    thr: f64,
    hold: bool,
    harmonic: f64,
    amount: Smooth,
    drive: Smooth,
    mix: Smooth,
    out: Smooth,
    drift: Wander,
    note_on: bool,
}

impl FeedbackDsp {
    fn loop_gain(amount: f64) -> f64 {
        // below ~55% the loop decays (long sustain); above it self-sustains,
        // held in check by the saturation in the loop (the extra headroom
        // makes up for the tone filters inside the loop)
        0.92 + 0.22 * amount
    }
}

impl VintageDsp for FeedbackDsp {
    const METERS: usize = 2;

    fn new(sample_rate: f32, _channels: usize) -> Self {
        let fs = sample_rate as f64;
        let mut loop_hp = OnePole::default();
        loop_hp.set_lowpass(fs, 70.0);
        Self {
            fs,
            pitch: Pitch::new(fs),
            string: DelayLine::new((fs / 40.0) as usize),
            loop_lp: OnePole::default(),
            loop_hp,
            dc: DcBlock::new(fs),
            amp_os: [Oversampler::new(), Oversampler::new()],
            amp_dc: [DcBlock::new(fs), DcBlock::new(fs)],
            amp_tone: [Biquad::identity(); 2],
            env: Envelope::new(fs, 2.0, 120.0),
            bloom: 0.0,
            bloom_up: coeff(fs, 400.0),
            bloom_down: coeff(fs, 250.0),
            period: Smooth::new(fs, 25.0, fs / 220.0),
            thr: db_to_gain(-40.0),
            hold: false,
            harmonic: 2.0,
            amount: Smooth::new(fs, 50.0, 0.6),
            drive: Smooth::new(fs, 30.0, 3.0),
            mix: Smooth::new(fs, 20.0, 0.5),
            out: Smooth::new(fs, 20.0, 1.0),
            drift: Wander::new(fs, 0.4, 23),
            note_on: false,
        }
    }

    fn latency(&self) -> u32 {
        OS_LATENCY as u32
    }

    fn reset(&mut self) {
        self.pitch.reset();
        self.string.reset();
        self.loop_lp.reset();
        self.loop_hp.reset();
        self.dc.reset();
        self.amp_os.iter_mut().for_each(|o| o.reset());
        self.amp_dc.iter_mut().for_each(|d| d.reset());
        self.amp_tone.iter_mut().for_each(|b| b.reset());
        self.bloom = 0.0;
    }

    fn set_params(&mut self, v: &[f32]) {
        self.amount.set(v[AMOUNT] as f64);
        // bloom time = time to reach ~90% (3 time constants)
        self.bloom_up = coeff(self.fs, v[BLOOM] as f64 * 1000.0 / 2.3);
        self.harmonic = match v[HARMONIC].round() as i32 {
            0 => 1.0,
            1 => 2.0,
            2 => 3.0,
            _ => 4.0,
        };
        self.drive.set(db_to_gain(v[DRIVE] as f64));
        self.loop_lp.set_lowpass(self.fs, v[TONE] as f64 * 1.5);
        for b in &mut self.amp_tone {
            b.set_lowpass(self.fs, v[TONE] as f64, 0.8);
        }
        self.thr = db_to_gain(v[THRESHOLD] as f64);
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
        self.hold = v[HOLD] > 0.5;
    }

    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let stereo = chans.len() >= 2;
        let mut peak = 0.0f64;
        let mut fb_level = 0.0f64;
        for k in 0..n {
            let (xl, xr) = if stereo { (chans[0][k] as f64, chans[1][k] as f64) } else { (chans[0][k] as f64, chans[0][k] as f64) };
            let mono = 0.5 * (xl + xr);
            let amount = self.amount.next();
            let drive = self.drive.next();
            let mix = self.mix.next();
            let o = self.out.next();

            // note detection + pitch
            self.pitch.push(mono);
            let level = self.env.process(mono);
            let playing = level > self.thr;
            if playing && !self.note_on {
                self.note_on = true;
                self.pitch.new_note();
            } else if !playing && level < self.thr * 0.5 {
                self.note_on = false;
            }
            // keep following the string's pitch (bends, vibrato) as long as
            // it can still be heard at all, not just while it's above the
            // threshold: a bent string has usually faded by then
            let audible = level > (self.thr * 0.03).max(db_to_gain(-80.0));
            if let Some(f) = self.pitch.freq {
                if audible {
                    // the loop rings at the chosen harmonic of the played note
                    let p = self.fs / (f * self.harmonic);
                    self.period.set(p.clamp(8.0, self.string.max_delay() - 4.0));
                }
            }

            // bloom: grows while a note is held; HOLD keeps it going
            let target = if self.note_on || self.hold { 1.0 } else { 0.0 };
            let c = if target > self.bloom { self.bloom_up } else { self.bloom_down };
            self.bloom = target + c * (self.bloom - target);

            // string/pickup/amp loop: tuned delay with saturation and tone inside
            let p = self.period.next() * (1.0 + 0.0015 * self.drift.next());
            let back = self.string.read(p);
            let shaped = self.loop_lp.lp(back);
            let shaped = shaped - self.loop_hp.lp(shaped);
            let excite = mono * 0.6 * self.bloom;
            let g = Self::loop_gain(amount) * (0.85 + 0.15 * self.bloom);
            let y = (2.0 * (excite + g * shaped)).tanh() * 0.5;
            let y = self.dc.process(y);
            self.string.write(y);
            let fb = y * self.bloom * 2.0;
            fb_level = fb_level.max(fb.abs() * self.bloom);

            // the amp: guitar plus the feedback, overdriven together;
            // MIX blends the clean guitar against the amp + feedback
            let mut outs = [0.0f64; 2];
            for (c, x) in [xl, xr].into_iter().enumerate() {
                let sig = x + fb;
                let a = self.amp_os[c].process(sig, |v| tube(v * drive * 0.5, 0.2) / (drive * 0.5).sqrt());
                let a = self.amp_tone[c].process(self.amp_dc[c].process(a));
                outs[c] = (x * (1.0 - mix) + a * mix) * o;
            }
            if stereo {
                chans[0][k] = outs[0] as f32;
                chans[1][k] = outs[1] as f32;
            } else {
                chans[0][k] = (0.5 * (outs[0] + outs[1])) as f32;
            }
            peak = peak.max(outs[0].abs()).max(outs[1].abs());
        }
        meters[0] = peak as f32;
        meters[1] = fb_level as f32;
    }
}

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Feel Fine Intro", values: &[(AMOUNT, 0.75), (BLOOM, 0.6), (HARMONIC, 0.0), (DRIVE, 12.0)] },
        Preset { name: "Octave Howl", values: &[(AMOUNT, 0.8), (BLOOM, 1.2), (HARMONIC, 1.0), (DRIVE, 15.0)] },
        Preset { name: "Screaming Twelfth", values: &[(AMOUNT, 0.9), (BLOOM, 0.9), (HARMONIC, 2.0), (DRIVE, 20.0), (TONE, 5000.0)] },
        Preset { name: "Gentle Sustain", values: &[(AMOUNT, 0.4), (BLOOM, 2.0), (HARMONIC, 1.0), (DRIVE, 6.0), (MIX, 0.4)] },
        Preset { name: "Infinite Hold", values: &[(AMOUNT, 0.85), (BLOOM, 1.0), (HOLD, 1.0), (DRIVE, 14.0)] },
        Preset { name: "Two-Octave Shriek", values: &[(AMOUNT, 0.95), (BLOOM, 0.5), (HARMONIC, 3.0), (DRIVE, 22.0), (TONE, 6500.0), (OUTPUT, -4.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "FEEDBACK",
        subtitle: ("GUITAR & AMPLIFIER", "FEEDBACK MACHINE"),
        accent: ui::INDIGO,
        size: (W, H),
        items: vec![
            Item::Knob { p: AMOUNT, x: 130.0, y: 172.0, r: 50.0, label: "FEEDBACK",
                tip: "How hard the amp drives the string. Below about half-way notes sustain and swell; above it they howl on indefinitely.", active: None },
            Item::Knob { p: BLOOM, x: 510.0, y: 172.0, r: 50.0, label: "BLOOM",
                tip: "How long a held note takes to bloom into feedback.", active: None },
            Item::Meter { m: 0, x: 222.0, y: 92.0, w: 196.0, h: 136.0, kind: MeterKind::Vu, caption: "OUTPUT" },
            Item::Lamp { m: 1, x: 236.0, y: 254.0, label: "FEEDING BACK", lo: 0.03, hi: 0.4 },
            Item::Switch { p: HOLD, x: 424.0, y: 254.0, labels: ["OFF", "HOLD"],
                tip: "ON: the feedback keeps singing after you stop playing (until you switch it off)." },
            Item::Divider { y: 284.0 },
            Item::Selector { p: HARMONIC, x: 90.0, y: 340.0, label: "HARMONIC", positions: &["1", "8va", "12th", "15ma"],
                tip: "Which harmonic of the played note the feedback locks onto: the note itself, an octave up, an octave and a fifth, or two octaves up." },
            Item::Knob { p: DRIVE, x: 190.0, y: 332.0, r: 22.0, label: "AMP DRIVE", tip: "Overdrive of the amp the guitar and feedback go through.", active: None },
            Item::Knob { p: TONE, x: 280.0, y: 332.0, r: 22.0, label: "TONE", tip: "Amp tone: lower = rounder, darker feedback.", active: None },
            Item::Knob { p: THRESHOLD, x: 370.0, y: 332.0, r: 22.0, label: "THRESHOLD", tip: "How loud a note must be to start feeding back.", active: None },
            Item::Knob { p: MIX, x: 460.0, y: 332.0, r: 22.0, label: "MIX", tip: "Clean guitar vs. amp + feedback.", active: None },
            Item::Knob { p: OUTPUT, x: 550.0, y: 332.0, r: 22.0, label: "OUTPUT", tip: "Final level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, FeedbackParams, FeedbackDsp, panel,
    name: "Feedback", class_id: *b"S2FeedbackDIY001", vendor: "PEQUOD Beatles",
    categories: [Fx, Distortion]
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn yin_tracks_guitar_notes() {
        let fs = 48000.0;
        for f0 in [82.4, 110.0, 196.0, 329.6, 659.3] {
            let mut p = Pitch::new(fs);
            for k in 0..(fs * 0.3) as usize {
                // note with harmonics, like a plucked string
                let t = k as f64 / fs;
                let x = (2.0 * PI * f0 * t).sin() + 0.5 * (4.0 * PI * f0 * t).sin() + 0.3 * (6.0 * PI * f0 * t).sin();
                p.push(0.3 * x);
            }
            let f = p.freq.expect("pitch");
            println!("{f0} Hz -> {f:.1} Hz");
            assert!((f / f0 - 1.0).abs() < 0.02, "{f0} -> {f}");
        }
    }

    fn render(params: &[f32], secs: f64, note_secs: f64, f0: f64) -> Vec<f32> {
        let fs = 48000.0;
        let mut d = FeedbackDsp::new(fs as f32, 1);
        d.set_params(params);
        let n = (fs * secs) as usize;
        let mut x: Vec<f32> = (0..n)
            .map(|k| {
                let t = k as f64 / fs;
                if t < note_secs {
                    // decaying plucked note
                    (0.3 * (-t * 1.5).exp() * ((2.0 * PI * f0 * t).sin() + 0.4 * (4.0 * PI * f0 * t).sin())) as f32
                } else {
                    0.0
                }
            })
            .collect();
        let mut m = [0.0f32; 2];
        for chunk in x.chunks_mut(512) {
            d.process(&mut [chunk], &mut m);
        }
        x
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
    }

    const P: [f32; 9] = [0.85, 0.8, 1.0, 9.0, 3500.0, -50.0, 1.0, 0.0, 0.0];

    #[test]
    fn held_note_blooms_into_sustained_feedback() {
        // plucked note decays on its own; with the machine it should keep going
        let x = render(&P, 3.0, 3.0, 196.0);
        let early = rms(&x[4800..14400]);
        let late = rms(&x[120000..144000]);
        println!("early {early:.3} late {late:.3}");
        assert!(late > early * 0.6, "feedback should sustain the note");
        assert!(x.iter().all(|v| v.is_finite() && v.abs() < 4.0));
    }

    #[test]
    fn stops_after_note_unless_hold() {
        let x = render(&P, 4.0, 1.5, 196.0);
        let tail = rms(&x[168000..192000]);
        let mut held = P;
        held[HOLD] = 1.0;
        let y = render(&held, 4.0, 1.5, 196.0);
        let tail_held = rms(&y[168000..192000]);
        println!("tail without hold {tail:.4}, with hold {tail_held:.4}");
        assert!(tail < 0.01);
        assert!(tail_held > tail * 10.0);
    }

    #[test]
    fn locks_onto_the_octave() {
        // with HARMONIC = octave, the sustained tone should be dominated by 2*f0
        let f0 = 196.0;
        let x = render(&P, 3.0, 3.0, f0);
        let seg = &x[100000..140000];
        let fs = 48000.0;
        let mag = |f: f64| {
            let (mut re, mut im) = (0.0, 0.0);
            for (k, v) in seg.iter().enumerate() {
                let ph = 2.0 * PI * f * k as f64 / fs;
                re += *v as f64 * ph.cos();
                im += *v as f64 * ph.sin();
            }
            (re * re + im * im).sqrt()
        };
        let (fund, oct) = (mag(f0), mag(2.0 * f0));
        println!("fundamental {fund:.1}, octave {oct:.1}");
        assert!(oct > fund);
    }

    /// Render a held note that bends up `semis` between 1.0 and 1.5 s, with a
    /// little noise and a natural decay, and return the output.
    fn render_bend(params: &[f32], f0: f64, semis: f64) -> Vec<f32> {
        render_bend_decay(params, f0, semis, 0.9)
    }

    fn render_bend_decay(params: &[f32], f0: f64, semis: f64, decay: f64) -> Vec<f32> {
        let fs = 48000.0;
        let mut d = FeedbackDsp::new(fs as f32, 1);
        d.set_params(params);
        let n = (fs * 3.0) as usize;
        let mut noise = Noise::new(99);
        let mut ph = 0.0f64;
        let mut x: Vec<f32> = (0..n)
            .map(|k| {
                let t = k as f64 / fs;
                let b = ((t - 1.0) / 0.5).clamp(0.0, 1.0) * semis;
                let f = f0 * 2f64.powf(b / 12.0);
                ph += f / fs;
                let tone = (2.0 * PI * ph).sin() + 0.4 * (4.0 * PI * ph).sin() + 0.2 * (6.0 * PI * ph).sin();
                (0.3 * (-t * decay).exp() * tone + 0.0005 * noise.next()) as f32
            })
            .collect();
        let mut m = [0.0f32; 2];
        for chunk in x.chunks_mut(256) {
            d.process(&mut [chunk], &mut m);
        }
        x
    }

    /// Strongest frequency in `seg` between lo and hi Hz (1 Hz steps).
    fn peak_freq(seg: &[f32], lo: f64, hi: f64) -> f64 {
        let fs = 48000.0;
        let mut best = (0.0, lo);
        let mut f = lo;
        while f <= hi {
            let (mut re, mut im) = (0.0, 0.0);
            for (k, v) in seg.iter().enumerate() {
                let ph = 2.0 * PI * f * k as f64 / fs;
                re += *v as f64 * ph.cos();
                im += *v as f64 * ph.sin();
            }
            let m = re * re + im * im;
            if m > best.0 {
                best = (m, f);
            }
            f += 1.0;
        }
        best.1
    }

    #[test]
    fn feedback_follows_string_bends() {
        let f0 = 196.0; // G
        let x = render_bend(&P, f0, 2.0);
        // HARMONIC = octave: before the bend ~392 Hz, after a whole-tone bend ~440 Hz
        let before = peak_freq(&x[30000..44000], 350.0, 480.0);
        let after = peak_freq(&x[110000..140000], 350.0, 480.0);
        println!("feedback pitch before bend {before:.0} Hz, after {after:.0} Hz (expect ~392 -> ~440)");
        assert!((before - 392.0).abs() < 8.0);
        assert!((after - 440.0).abs() < 8.0, "feedback did not follow the bend");
    }

    #[test]
    fn bends_tracked_after_note_decays_below_threshold() {
        // realistic: string has faded under the threshold by the time it's bent,
        // feedback kept alive by HOLD
        let mut p = P;
        p[THRESHOLD] = -30.0;
        p[HOLD] = 1.0;
        let x = render_bend_decay(&p, 196.0, 2.0, 2.4);
        let before = peak_freq(&x[30000..44000], 350.0, 480.0);
        let after = peak_freq(&x[110000..140000], 350.0, 480.0);
        println!("quiet string: before {before:.0} Hz, after {after:.0} Hz (expect ~392 -> ~440)");
        assert!((after - 440.0).abs() < 8.0, "feedback did not follow the bend");
    }

    #[test]
    fn bend_and_release_returns_to_pitch() {
        let fs = 48000.0;
        let mut d = FeedbackDsp::new(fs as f32, 1);
        let mut p = P;
        p[HOLD] = 1.0;
        d.set_params(&p);
        let n = (fs * 4.0) as usize;
        let mut ph = 0.0f64;
        let mut x: Vec<f32> = (0..n)
            .map(|k| {
                let t = k as f64 / fs;
                // up a whole tone 1.0-1.5 s, back down 2.0-2.5 s
                let b = 2.0 * (((t - 1.0) / 0.5).clamp(0.0, 1.0) - ((t - 2.0) / 0.5).clamp(0.0, 1.0));
                ph += 196.0 * 2f64.powf(b / 12.0) / fs;
                (0.3 * (-t * 1.6).exp() * ((2.0 * PI * ph).sin() + 0.4 * (4.0 * PI * ph).sin())) as f32
            })
            .collect();
        let mut m = [0.0f32; 2];
        for chunk in x.chunks_mut(256) {
            d.process(&mut [chunk], &mut m);
        }
        let up = peak_freq(&x[80000..94000], 350.0, 480.0);
        let back = peak_freq(&x[140000..170000], 350.0, 480.0);
        println!("bent {up:.0} Hz, released {back:.0} Hz (expect ~440 then ~392)");
        assert!((up - 440.0).abs() < 8.0 && (back - 392.0).abs() < 8.0);
    }
}
