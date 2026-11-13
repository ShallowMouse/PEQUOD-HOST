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

pub const MAX_PLAYERS: usize = 8;

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
}

impl Default for TapeVoice {
    fn default() -> Self {
        Self { age: 0, f: 440.0, ph: [0.0; MAX_PLAYERS], vph: [0.0; MAX_PLAYERS], form: [Biquad::identity(); 3], lp: [0.0; 2], lp_a: 1.0, breath: Biquad::identity(), sound: 0 }
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
                        Wave::Flute => 0.0,
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
    pub const CHOIR: Sound = Sound {
        name: "Mixed Choir",
        players: &[p(-9.0, 5.1, 14.0), p(-3.0, 5.6, 12.0), p(4.0, 5.9, 16.0), p(10.0, 5.4, 13.0)],
        wave: Wave::Saw,
        formants: [(720.0, 4.0, 12.0), (1150.0, 5.0, 9.0), (2600.0, 6.0, 7.0)],
        lp: 3500.0, lp_start: 3500.0, open_s: 0.0,
        attack: 0.15, vib_onset: 0.0, vib_fade: 0.0, breath: 0.011, chiff: 0.0, gain: 0.2,
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
        players: &[p(-10.0, 5.0, 15.0), p(-3.0, 5.4, 13.0), p(4.0, 5.7, 16.0), p(9.0, 5.2, 14.0)],
        wave: Wave::Saw,
        formants: [(650.0, 5.0, 12.0), (1080.0, 6.0, 9.0), (2450.0, 7.0, 6.0)],
        lp: 3000.0, lp_start: 3000.0, open_s: 0.0,
        attack: 0.2, vib_onset: 0.0, vib_fade: 0.0, breath: 0.012, chiff: 0.0, gain: 0.2,
    };
    pub const FEMALE_CHOIR: Sound = Sound {
        name: "Female Choir",
        players: &[p(-8.0, 5.6, 15.0), p(-2.0, 5.9, 12.0), p(3.0, 6.2, 16.0), p(8.0, 5.5, 13.0)],
        wave: Wave::Saw,
        formants: [(850.0, 5.0, 12.0), (1250.0, 6.0, 9.0), (2850.0, 7.0, 7.0)],
        lp: 4500.0, lp_start: 4500.0, open_s: 0.0,
        attack: 0.18, vib_onset: 0.0, vib_fade: 0.0, breath: 0.012, chiff: 0.0, gain: 0.23,
    };
    pub const EIGHT_CHOIR: Sound = Sound {
        name: "8 Voice Choir",
        players: &[
            p(-12.0, 5.0, 14.0), p(-5.0, 5.5, 12.0), p(2.0, 5.8, 16.0), p(9.0, 5.3, 13.0),
            p8(-9.0, 5.1, 13.0), p8(-2.0, 5.6, 15.0), p8(5.0, 5.9, 12.0), p8(12.0, 5.4, 14.0),
        ],
        wave: Wave::Saw,
        formants: [(560.0, 4.0, 10.0), (900.0, 5.0, 7.0), (2500.0, 6.0, 5.0)],
        lp: 3200.0, lp_start: 3200.0, open_s: 0.0,
        attack: 0.3, vib_onset: 0.0, vib_fade: 0.0, breath: 0.012, chiff: 0.0, gain: 0.19,
    };
}

#[cfg(test)]
mod tests {
    use super::sounds::*;
    use super::*;

    #[test]
    fn every_sound_is_finite_and_audible() {
        let all = [VIOLINS, FLUTE, CHOIR, CELLO, STRING_SECTION, BRASS, TROMBONE, SAX, OBOE, CLARINET, MALE_CHOIR, FEMALE_CHOIR, EIGHT_CHOIR];
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
                assert!(r > 0.01 && r < 1.5, "{} at {f}: rms {r}", s.name);
            }
        }
    }
}
