//! The instruments on tape-replay keyboards (the Mellotron and the Chamberlin): each key's tape
//! holds a recording of a player, or a section, playing that note. These are synthesised, not
//! sampled: a table of `Sound`s, each a small ensemble of players (their own detune and
//! vibrato, some an octave down), a waveform for the instrument's source (bowed saw, reed
//! pulse, clarinet square, flute), its body or vowel formants, a lowpass that can open as the
//! note speaks (brass), breath and chiff. The keyboards add what the machine does: the tape
//! start, wow and flutter, hiss, the eight-second limit.

use crate::dsp::*;
use std::f64::consts::TAU;

#[derive(Clone, Copy)]
pub enum Wave {
    /// Bowed strings, brass, voices.
    Saw,
    /// Odd harmonics only: the clarinet.
    Square,
    /// A narrow pulse (duty 0..0.5): reeds - oboe, saxophone.
    Pulse(f64),
    /// Nearly sine, a little 2nd and 3rd, breath on top.
    Flute,
    /// Singers: each a glottal pulse with its own pitch drift, jitter and vibrato, through the
    /// vowel's five formants for their voice part. The `players` and `formants` of the Sound
    /// aren't used: each Section has its own.
    Voices(&'static [Section]),
}

/// A voice part of a choir: its singers, and the five formants of the vowel as that part
/// sings it (Hz, bandwidth Hz, dB).
#[derive(Clone, Copy)]
pub struct Section {
    pub singers: &'static [Player],
    pub formants: [(f64, f64, f64); 5],
    /// Aspiration: air through the folds, riding on each pulse.
    pub breath: f64,
}

/// One period of glottal flow (Rosenberg): opens over 40 % of the cycle, closes over 16 %,
/// then shut. Smooth, with the falling spectrum of a voice rather than a buzzy saw.
#[inline]
fn glottal(ph: f64) -> f64 {
    const OPEN: f64 = 0.40;
    const CLOSE: f64 = 0.16;
    if ph < OPEN {
        0.5 * (1.0 - (std::f64::consts::PI * ph / OPEN).cos())
    } else if ph < OPEN + CLOSE {
        (0.5 * std::f64::consts::PI * (ph - OPEN) / CLOSE).cos()
    } else {
        0.0
    }
}

/// One player: detune (cents), vibrato rate (Hz) and depth (cents), octave.
#[derive(Clone, Copy)]
pub struct Player(pub f64, pub f64, pub f64, pub i8);

#[derive(Clone, Copy)]
pub struct Sound {
    pub name: &'static str,
    pub players: &'static [Player],
    pub wave: Wave,
    /// Body / vowel resonances: (Hz, Q, dB); 0 dB leaves one out.
    pub formants: [(f64, f64, f64); 3],
    /// Lowpass once the note has spoken, and where it starts (brass opens up).
    pub lp: f64,
    pub lp_start: f64,
    pub open_s: f64,
    /// The player's own attack (bow, breath), seconds.
    pub attack: f64,
    /// Vibrato comes in after this long, over `vib_fade`.
    pub vib_onset: f64,
    pub vib_fade: f64,
    pub breath: f64,
    pub chiff: f64,
    pub gain: f64,
}

pub const MAX_PLAYERS: usize = 16;
const MAX_SECTIONS: usize = 4;

#[inline]
fn blep(t: f64, dt: f64) -> f64 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

#[inline]
fn pulse(ph: f64, dt: f64, w: f64) -> f64 {
    let mut p = if ph < w { 1.0 } else { -1.0 };
    p += blep(ph, dt);
    p -= blep((ph - w + 1.0).fract(), dt);
    p - (2.0 * w - 1.0)
}

/// One key's recording, as it plays.
#[derive(Clone, Copy)]
pub struct TapeVoice {
    pub age: usize,
    pub f: f64,
    ph: [f64; MAX_PLAYERS],
    vph: [f64; MAX_PLAYERS],
    form: [Biquad; 3],
    lp: [f64; 2],
    lp_a: f64,
    breath: Biquad,
    sound: usize,
    // singers (Wave::Voices)
    drift: [f64; MAX_PLAYERS],
    jitter: [f64; MAX_PLAYERS],
    shimmer: [f64; MAX_PLAYERS],
    onset: [f64; MAX_PLAYERS],
    bank: [[Biquad; 5]; MAX_SECTIONS],
    radiate: [f64; MAX_SECTIONS],
}

impl Default for TapeVoice {
    fn default() -> Self {
        Self {
            age: 0, f: 440.0, ph: [0.0; MAX_PLAYERS], vph: [0.0; MAX_PLAYERS], form: [Biquad::identity(); 3], lp: [0.0; 2], lp_a: 1.0,
            breath: Biquad::identity(), sound: 0,
            drift: [0.0; MAX_PLAYERS], jitter: [0.0; MAX_PLAYERS], shimmer: [1.0; MAX_PLAYERS], onset: [0.0; MAX_PLAYERS],
            bank: [[Biquad::identity(); 5]; MAX_SECTIONS], radiate: [0.0; MAX_SECTIONS],
        }
    }
}

impl TapeVoice {
    /// A key goes down: `sound` is an index into the instrument's table, `seed` makes this
    /// take its own (players start mid-vibrato, at their own phase).
    pub fn start(&mut self, sounds: &[Sound], sound: usize, f: f64, fs: f64, seed: u32) {
        let s = &sounds[sound.min(sounds.len() - 1)];
        let mut nz = Noise::new(seed | 1);
        *self = TapeVoice::default();
        self.sound = sound.min(sounds.len() - 1);
        self.f = f;
        for k in 0..MAX_PLAYERS {
            self.ph[k] = 0.5 + 0.5 * nz.next();
            self.vph[k] = 0.5 + 0.5 * nz.next();
        }
        for (b, (hz, q, g)) in self.form.iter_mut().zip(s.formants.iter()) {
            if *g != 0.0 {
                b.set_peak(fs, hz.min(0.45 * fs), *q, *g);
            }
        }
        if let Wave::Voices(sections) = s.wave {
            for (bank, sec) in self.bank.iter_mut().zip(sections.iter()) {
                for (b, (hz, bw, _)) in bank.iter_mut().zip(sec.formants.iter()) {
                    let hz = hz.min(0.45 * fs);
                    b.set_bandpass(fs, hz, hz / bw.max(10.0));
                }
            }
            // the singers come in a little apart, and each starts at their own pitch
            for k in 0..MAX_PLAYERS {
                self.onset[k] = 0.09 * (0.5 + 0.5 * nz.next());
                self.drift[k] = 6.0 * nz.next();
            }
        }
        if matches!(s.wave, Wave::Flute) || s.breath > 0.0 {
            self.breath.set_bandpass(fs, (2.0 * f).clamp(300.0, 0.4 * fs), 2.0);
        }
        self.lp_a = 1.0 - (-TAU * s.lp_start.min(0.45 * fs) / fs).exp();
    }

    /// The next sample of the recording, with the tape running at `speed` (1 = right pitch).
    #[inline]
    pub fn next(&mut self, sounds: &[Sound], speed: f64, fs: f64, noise: &mut Noise) -> f64 {
        let s = &sounds[self.sound];
        let t = self.age as f64 / fs;
        self.age += 1;
        let f = self.f * speed;
        let vib_amt = if s.vib_fade > 0.0 { ((t - s.vib_onset) / s.vib_fade).clamp(0.0, 1.0) } else if t >= s.vib_onset { 1.0 } else { 0.0 };
        let mut x = 0.0;
        if let Wave::Voices(sections) = s.wave {
            let mut k = 0;
            for (si, sec) in sections.iter().enumerate().take(MAX_SECTIONS) {
                let mut flow = 0.0;
                for Player(det, vr, vd, oct) in sec.singers.iter() {
                    if k >= MAX_PLAYERS {
                        break;
                    }
                    // each singer: a slow wander in pitch, vibrato that settles in, a tiny
                    // cycle-to-cycle jitter and shimmer
                    self.drift[k] += 0.004 * noise.next() - 0.00002 * self.drift[k];
                    let vib = vd / 1200.0 * (TAU * self.vph[k]).sin() * (0.6 + 0.4 * vib_amt);
                    let cents = det + self.drift[k] * 1.2 + self.jitter[k];
                    let dt = (f * 2f64.powf(*oct as f64 + cents / 1200.0 + vib) / fs).min(0.45);
                    let ph = self.ph[k] + dt;
                    if ph >= 1.0 {
                        self.jitter[k] = 4.0 * noise.next();
                        self.shimmer[k] = 1.0 + 0.06 * noise.next();
                    }
                    self.ph[k] = ph.fract();
                    self.vph[k] = (self.vph[k] + vr * (1.0 + 0.03 * self.drift[k].tanh()) / fs).fract();
                    let come_in = ((t - self.onset[k]) / s.attack.max(0.001)).clamp(0.0, 1.0);
                    flow += glottal(self.ph[k]) * self.shimmer[k] * come_in;
                    k += 1;
                }
                // air through the folds, louder as they open
                let air = noise.next() * sec.breath * (0.1 + flow / sec.singers.len().max(1) as f64);
                // the lips radiate the change in flow (a gentle rise in the treble)
                let src = flow + air;
                let rad = 12.0 * (src - 0.94 * self.radiate[si]);
                self.radiate[si] = src;
                let mut y = 0.0;
                for (b, (_, _, db)) in self.bank[si].iter_mut().zip(sec.formants.iter()) {
                    y += b.process(rad) * db_to_gain(*db);
                }
                x += y;
            }
            if self.age % 32 == 1 {
                self.lp_a = 1.0 - (-TAU * s.lp.min(0.45 * fs) / fs).exp();
            }
            self.lp[0] += self.lp_a * (x - self.lp[0]);
            self.lp[1] += self.lp_a * (self.lp[0] - self.lp[1]);
            // low notes carry less: keep the keyboard even (about +6 dB at the bottom, -4 at the top)
            let even = (440.0 / self.f).sqrt().clamp(0.6, 2.0);
            return self.lp[1] * s.gain * even;
        }
        match s.wave {
            Wave::Flute => {
                let Player(_, vr, vd, _) = s.players[0];
                let vib = vd / 1200.0 * (TAU * self.vph[0]).sin() * vib_amt;
                let dt = f * 2f64.powf(vib) / fs;
                let p = TAU * self.ph[0];
                self.ph[0] = (self.ph[0] + dt).fract();
                self.vph[0] = (self.vph[0] + vr / fs).fract();
                x = p.sin() + 0.22 * (2.0 * p).sin() + 0.07 * (3.0 * p).sin();
            }
            w => {
                for (j, Player(det, vr, vd, oct)) in s.players.iter().enumerate().take(MAX_PLAYERS) {
                    let vib = vd / 1200.0 * (TAU * self.vph[j]).sin() * vib_amt;
                    let dt = (f * 2f64.powf(*oct as f64 + det / 1200.0 + vib) / fs).min(0.45);
                    let ph = self.ph[j];
                    x += match w {
                        Wave::Saw => 2.0 * ph - 1.0 - blep(ph, dt),
                        Wave::Square => pulse(ph, dt, 0.5),
                        Wave::Pulse(d) => pulse(ph, dt, d),
                        Wave::Flute | Wave::Voices(_) => 0.0,
                    };
                    self.ph[j] = (ph + dt).fract();
                    self.vph[j] = (self.vph[j] + vr / fs).fract();
                }
            }
        }
        // breath (and the chiff as a wind note speaks)
        if s.breath > 0.0 || s.chiff > 0.0 {
            let ch = 1.0 + s.chiff * (-t / 0.03).exp();
            x += self.breath.process(noise.next()) * s.breath * ch * 4.0;
        }
        // the lowpass, opening from lp_start to lp as the note speaks
        if self.age % 32 == 1 {
            let k = if s.open_s > 0.0 { (t / s.open_s).min(1.0) } else { 1.0 };
            let fc = s.lp_start + (s.lp - s.lp_start) * (1.0 - (1.0 - k) * (1.0 - k));
            self.lp_a = 1.0 - (-TAU * fc.min(0.45 * fs) / fs).exp();
        }
        self.lp[0] += self.lp_a * (x - self.lp[0]);
        self.lp[1] += self.lp_a * (self.lp[0] - self.lp[1]);
        let y = cascade(&mut self.form, self.lp[1]);
        y * s.gain * (t / s.attack.max(0.001)).min(1.0)
    }
}

const fn p(det: f64, rate: f64, depth: f64) -> Player {
    Player(det, rate, depth, 0)
}
const fn p8(det: f64, rate: f64, depth: f64) -> Player {
    Player(det, rate, depth, -1)
}
const NONE: (f64, f64, f64) = (1000.0, 1.0, 0.0);

/// The sounds, as instruments go: the keyboards pick their own sets from these.
pub mod sounds {
    use super::*;

    pub const VIOLINS: Sound = Sound {
        name: "3 Violins",
        players: &[p(-6.0, 5.3, 11.0), p(1.0, 5.8, 9.0), p(5.0, 6.2, 13.0)],
        wave: Wave::Saw,
        formants: [(480.0, 1.2, 5.0), (1300.0, 1.4, 3.0), (2900.0, 1.6, 4.0)],
        lp: 5000.0, lp_start: 5000.0, open_s: 0.0,
        attack: 0.12, vib_onset: 0.0, vib_fade: 0.4, breath: 0.0, chiff: 0.0, gain: 0.3,
    };
    pub const FLUTE: Sound = Sound {
        name: "Flute",
        players: &[p(0.0, 4.8, 8.0)],
        wave: Wave::Flute,
        formants: [NONE; 3],
        lp: 6000.0, lp_start: 6000.0, open_s: 0.0,
        attack: 0.06, vib_onset: 0.5, vib_fade: 0.4, breath: 0.06, chiff: 6.0, gain: 0.55,
    };
    // The vowel "aah", part by part (the classic singing formant tables: bass, tenor, alto,
    // soprano - frequency, bandwidth, level), and the singers in each.
    const BASS_AH: [(f64, f64, f64); 5] = [(600.0, 60.0, 0.0), (1040.0, 70.0, -7.0), (2250.0, 110.0, -9.0), (2450.0, 120.0, -9.0), (2750.0, 130.0, -20.0)];
    const TENOR_AH: [(f64, f64, f64); 5] = [(650.0, 80.0, 0.0), (1080.0, 90.0, -6.0), (2650.0, 120.0, -7.0), (2900.0, 130.0, -8.0), (3250.0, 140.0, -22.0)];
    const ALTO_AH: [(f64, f64, f64); 5] = [(800.0, 80.0, 0.0), (1150.0, 90.0, -4.0), (2800.0, 120.0, -20.0), (3500.0, 130.0, -36.0), (4950.0, 140.0, -60.0)];
    const SOPRANO_AH: [(f64, f64, f64); 5] = [(800.0, 80.0, 0.0), (1150.0, 90.0, -6.0), (2900.0, 120.0, -32.0), (3900.0, 130.0, -20.0), (4950.0, 140.0, -50.0)];

    const MEN_A: &[Player] = &[p(-9.0, 5.1, 22.0), p(-2.0, 5.6, 18.0), p(6.0, 5.3, 25.0), p(11.0, 5.9, 20.0)];
    const MEN_B: &[Player] = &[p(-12.0, 5.4, 20.0), p(-5.0, 5.0, 24.0), p(3.0, 5.8, 19.0), p(8.0, 5.2, 23.0)];
    const WOMEN_A: &[Player] = &[p(-8.0, 5.7, 24.0), p(-1.0, 6.1, 20.0), p(5.0, 5.5, 27.0), p(10.0, 5.9, 22.0)];
    const WOMEN_B: &[Player] = &[p(-11.0, 5.9, 21.0), p(-4.0, 5.4, 26.0), p(2.0, 6.2, 19.0), p(7.0, 5.6, 24.0)];

    pub const CHOIR: Sound = Sound {
        name: "Mixed Choir",
        players: &[],
        wave: Wave::Voices(&[
            Section { singers: &[p(-9.0, 5.1, 20.0), p(4.0, 5.6, 18.0)], formants: TENOR_AH, breath: 0.05 },
            Section { singers: &[p(-3.0, 5.8, 22.0), p(8.0, 5.4, 19.0)], formants: ALTO_AH, breath: 0.05 },
        ]),
        formants: [NONE; 3],
        lp: 6000.0, lp_start: 6000.0, open_s: 0.0,
        attack: 0.15, vib_onset: 0.25, vib_fade: 0.6, breath: 0.0, chiff: 0.0, gain: 2.2,
    };
    pub const CELLO: Sound = Sound {
        name: "Cello",
        players: &[p(0.0, 5.4, 14.0)],
        wave: Wave::Saw,
        formants: [(220.0, 1.5, 6.0), (700.0, 1.5, 3.0), (1500.0, 2.0, 3.0)],
        lp: 3500.0, lp_start: 3500.0, open_s: 0.0,
        attack: 0.15, vib_onset: 0.25, vib_fade: 0.4, breath: 0.0, chiff: 0.0, gain: 0.5,
    };
    pub const STRING_SECTION: Sound = Sound {
        name: "String Section",
        players: &[
            p(-8.0, 5.3, 10.0), p(-2.0, 5.9, 9.0), p(3.0, 6.2, 12.0), p(7.0, 5.5, 11.0),
            p(-4.0, 5.6, 10.0), p(5.0, 6.0, 9.0), p8(-3.0, 5.4, 12.0), p8(4.0, 5.8, 11.0),
        ],
        wave: Wave::Saw,
        formants: [(420.0, 1.0, 4.0), (1250.0, 1.3, 3.0), (2800.0, 1.5, 3.0)],
        lp: 4500.0, lp_start: 4500.0, open_s: 0.0,
        attack: 0.18, vib_onset: 0.0, vib_fade: 0.5, breath: 0.0, chiff: 0.0, gain: 0.17,
    };
    pub const BRASS: Sound = Sound {
        name: "3 Trumpets",
        players: &[p(-5.0, 5.5, 6.0), p(0.0, 6.0, 5.0), p(5.0, 5.2, 6.0)],
        wave: Wave::Saw,
        formants: [(1200.0, 1.5, 6.0), (2500.0, 2.0, 4.0), NONE],
        lp: 5000.0, lp_start: 700.0, open_s: 0.09,
        attack: 0.04, vib_onset: 0.6, vib_fade: 0.5, breath: 0.0, chiff: 0.0, gain: 0.25,
    };
    pub const TROMBONE: Sound = Sound {
        name: "Trombone",
        players: &[p(0.0, 5.0, 6.0)],
        wave: Wave::Saw,
        formants: [(500.0, 1.4, 5.0), (1150.0, 1.6, 4.0), (2400.0, 2.0, 2.0)],
        lp: 2600.0, lp_start: 500.0, open_s: 0.12,
        attack: 0.05, vib_onset: 0.7, vib_fade: 0.5, breath: 0.0, chiff: 0.0, gain: 0.5,
    };
    pub const SAX: Sound = Sound {
        name: "Alto Sax",
        players: &[p(0.0, 5.3, 14.0)],
        wave: Wave::Saw,
        formants: [(550.0, 2.0, 6.0), (1600.0, 2.5, 5.0), (2700.0, 3.0, 3.0)],
        lp: 4000.0, lp_start: 1500.0, open_s: 0.05,
        attack: 0.04, vib_onset: 0.3, vib_fade: 0.4, breath: 0.03, chiff: 0.0, gain: 0.45,
    };
    pub const OBOE: Sound = Sound {
        name: "Oboe",
        players: &[p(0.0, 5.6, 10.0)],
        wave: Wave::Pulse(0.14),
        formants: [(1050.0, 3.0, 9.0), (1400.0, 3.0, 6.0), (3000.0, 3.0, 4.0)],
        lp: 5000.0, lp_start: 5000.0, open_s: 0.0,
        attack: 0.05, vib_onset: 0.25, vib_fade: 0.4, breath: 0.01, chiff: 0.0, gain: 0.45,
    };
    pub const CLARINET: Sound = Sound {
        name: "Clarinet",
        players: &[p(0.0, 5.0, 3.0)],
        wave: Wave::Square,
        formants: [(1500.0, 2.0, 3.0), NONE, NONE],
        lp: 2500.0, lp_start: 2500.0, open_s: 0.0,
        attack: 0.05, vib_onset: 0.4, vib_fade: 0.5, breath: 0.02, chiff: 0.0, gain: 0.5,
    };
    pub const MALE_CHOIR: Sound = Sound {
        name: "Male Choir",
        players: &[],
        wave: Wave::Voices(&[
            Section { singers: MEN_A, formants: TENOR_AH, breath: 0.05 },
            Section { singers: MEN_B, formants: BASS_AH, breath: 0.05 },
        ]),
        formants: [NONE; 3],
        lp: 5000.0, lp_start: 5000.0, open_s: 0.0,
        attack: 0.2, vib_onset: 0.3, vib_fade: 0.6, breath: 0.0, chiff: 0.0, gain: 1.75,
    };
    pub const FEMALE_CHOIR: Sound = Sound {
        name: "Female Choir",
        players: &[],
        wave: Wave::Voices(&[
            Section { singers: WOMEN_A, formants: SOPRANO_AH, breath: 0.06 },
            Section { singers: WOMEN_B, formants: ALTO_AH, breath: 0.06 },
        ]),
        formants: [NONE; 3],
        lp: 7000.0, lp_start: 7000.0, open_s: 0.0,
        attack: 0.18, vib_onset: 0.3, vib_fade: 0.6, breath: 0.0, chiff: 0.0, gain: 1.45,
    };
    pub const EIGHT_CHOIR: Sound = Sound {
        name: "8 Voice Choir",
        players: &[],
        wave: Wave::Voices(&[
            Section { singers: &[p(-10.0, 5.1, 20.0), p(5.0, 5.6, 22.0)], formants: BASS_AH, breath: 0.05 },
            Section { singers: &[p(-4.0, 5.4, 24.0), p(9.0, 5.0, 19.0)], formants: TENOR_AH, breath: 0.05 },
            Section { singers: &[p(-7.0, 5.8, 23.0), p(3.0, 6.1, 20.0)], formants: ALTO_AH, breath: 0.06 },
            Section { singers: &[p(-2.0, 5.5, 25.0), p(11.0, 5.9, 21.0)], formants: SOPRANO_AH, breath: 0.06 },
        ]),
        formants: [NONE; 3],
        lp: 6500.0, lp_start: 6500.0, open_s: 0.0,
        attack: 0.3, vib_onset: 0.3, vib_fade: 0.8, breath: 0.0, chiff: 0.0, gain: 1.7,
    };

    /// The Chamberlin's voices: a few singers, closer to the microphone than the Mellotron's choirs.
    pub const CH_FEMALE_VOICE: Sound = Sound {
        name: "Female Voice",
        players: &[],
        wave: Wave::Voices(&[Section { singers: &[p(-4.0, 5.8, 26.0), p(5.0, 5.5, 22.0)], formants: SOPRANO_AH, breath: 0.05 }]),
        formants: [NONE; 3],
        lp: 8000.0, lp_start: 8000.0, open_s: 0.0,
        attack: 0.12, vib_onset: 0.25, vib_fade: 0.5, breath: 0.0, chiff: 0.0, gain: 2.4,
    };
    pub const CH_MALE_VOICE: Sound = Sound {
        name: "Male Voice",
        players: &[],
        wave: Wave::Voices(&[Section { singers: &[p(-5.0, 5.2, 22.0), p(4.0, 5.6, 20.0)], formants: TENOR_AH, breath: 0.05 }]),
        formants: [NONE; 3],
        lp: 6000.0, lp_start: 6000.0, open_s: 0.0,
        attack: 0.14, vib_onset: 0.25, vib_fade: 0.5, breath: 0.0, chiff: 0.0, gain: 2.6,
    };
}

#[cfg(test)]
mod tests {
    use super::sounds::*;
    use super::*;

    #[test]
    fn every_sound_is_finite_and_audible() {
        let all = [CHOIR, MALE_CHOIR, FEMALE_CHOIR, EIGHT_CHOIR, CH_FEMALE_VOICE, CH_MALE_VOICE, VIOLINS, FLUTE, CELLO, STRING_SECTION, BRASS, TROMBONE, SAX, OBOE, CLARINET];
        let mut nz = Noise::new(5);
        for (i, s) in all.iter().enumerate() {
            for f in [98.0, 440.0, 1397.0] {
                let mut v = TapeVoice::default();
                v.start(&all, i, f, 48000.0, 7);
                let mut e = 0.0;
                for _ in 0..24000 {
                    let y = v.next(&all, 1.0, 48000.0, &mut nz);
                    assert!(y.is_finite() && y.abs() < 6.0, "{} at {f}: {y}", s.name);
                    e += y * y;
                }
                let r = (e / 24000.0).sqrt();
                println!("{:16} {f:6}: rms {r:.4}", s.name);
                assert!(r > 0.01 && r < 1.5, "{} at {f}: rms {r}", s.name);
            }
        }
    }
}

#[cfg(test)]
mod listen {
    use super::sounds::*;
    use super::*;

    /// (Development: `CHOIR_WAVS=dir cargo test -p vintage choir_wavs -- --ignored` renders each
    /// choir as a short phrase of chords, to listen to.)
    #[test]
    #[ignore]
    fn choir_wavs() {
        let dir = std::env::var("CHOIR_WAVS").unwrap_or_else(|_| "/tmp".into());
        let all = [CHOIR, MALE_CHOIR, FEMALE_CHOIR, EIGHT_CHOIR];
        let fs = 48000.0;
        for (i, s) in all.iter().enumerate() {
            let chords: [&[f64]; 3] = [&[48.0, 55.0, 60.0, 64.0], &[45.0, 52.0, 57.0, 60.0], &[50.0, 57.0, 62.0, 65.0]];
            let mut out: Vec<f32> = Vec::new();
            let mut nz = Noise::new(9);
            for (c, chord) in chords.iter().enumerate() {
                let mut vs: Vec<TapeVoice> = chord
                    .iter()
                    .enumerate()
                    .map(|(k, n)| {
                        let mut v = TapeVoice::default();
                        v.start(&all, i, 440.0 * 2f64.powf((n - 69.0) / 12.0), fs, 11 + (c * 7 + k) as u32);
                        v
                    })
                    .collect();
                for t in 0..(2.5 * fs) as usize {
                    let fade = ((2.5 * fs - t as f64) / (0.15 * fs)).min(1.0);
                    let y: f64 = vs.iter_mut().map(|v| v.next(&all, 1.0, fs, &mut nz)).sum::<f64>() * 0.25 * fade;
                    out.push(y as f32);
                }
            }
            let path = format!("{dir}/{}.wav", s.name.replace(' ', "_"));
            let mut b: Vec<u8> = Vec::new();
            let n = out.len() as u32;
            b.extend_from_slice(b"RIFF");
            b.extend_from_slice(&(36 + n * 2).to_le_bytes());
            b.extend_from_slice(b"WAVEfmt ");
            b.extend_from_slice(&16u32.to_le_bytes());
            b.extend_from_slice(&1u16.to_le_bytes());
            b.extend_from_slice(&1u16.to_le_bytes());
            b.extend_from_slice(&48000u32.to_le_bytes());
            b.extend_from_slice(&96000u32.to_le_bytes());
            b.extend_from_slice(&2u16.to_le_bytes());
            b.extend_from_slice(&16u16.to_le_bytes());
            b.extend_from_slice(b"data");
            b.extend_from_slice(&(n * 2).to_le_bytes());
            for x in out {
                b.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
            }
            std::fs::write(path, b).unwrap();
        }
    }
}
