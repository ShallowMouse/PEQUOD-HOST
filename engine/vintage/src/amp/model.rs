//! Complete amplifiers assembled from the circuit blocks: preamp, phase
//! inverter, power stage, feedback loop, output transformer and speaker.
//!
//! Signals are in volts. A full-scale (0 dBFS) input is taken as 1 V peak
//! from the guitar, which is about what a hot single-coil hitting hard
//! delivers.

use super::net::{self, Circuit, Net, StackValues, Taper};
use super::output::{OutputTransformer, SpeakerLoad, SpeakerSpec, TransformerSpec};
use super::tube::*;

/// Front-panel settings, 0..1 like the real knobs (switches 0/1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Controls {
    pub volume: f64,
    pub treble: f64,
    pub bass: f64,
    pub middle: f64,
    pub cut: f64,
    pub bright: bool,
    /// AC30 only: early (82 ohm) vs late (50 ohm) output cathode resistor.
    pub alt: bool,
}

impl Default for Controls {
    fn default() -> Self {
        Self { volume: 0.5, treble: 0.5, bass: 0.5, middle: 0.5, cut: 0.0, bright: false, alt: false }
    }
}

// ------------------------------------------------------------------ specs --

#[derive(Clone, Copy, Debug)]
pub struct FenderPreSpec {
    pub v1: Valve,
    pub vb: f64,
    pub stack: StackValues,
    /// Fixed middle (no middle pot): rotation the middle resistor sits at.
    pub fixed_mid: bool,
    pub treble_taper: Taper,
    pub bass_taper: Taper,
    /// Bright cap across the volume pot (F); `bright_switch` = has a switch.
    pub bright_cap: f64,
    pub bright_switch: bool,
    /// Second stage cathode resistor (shared resistors given as their per-
    /// valve equivalent) and bypass.
    pub v2_rk: f64,
    pub v2_ck: f64,
    /// Coupling cap from the second stage into the 220k mixing resistors.
    pub mix_cap: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct VoxPreSpec {
    pub vb: f64,
    /// Input stage plate and (per-valve equivalent) cathode.
    pub v1_ra: f64,
    pub v1_rk: f64,
    pub v1_ck: f64,
    /// Top Boost gain stage and cathode follower.
    pub tb_ra: f64,
    pub tb_rk: f64,
    pub tb_ck: f64,
    pub cf_rk: f64,
    pub stack: StackValues,
    /// Normal channel: V1 -> volume -> mixer, no Top Boost stage or tone network.
    pub normal: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct HybridPreSpec {
    /// Transistor stage gains and their clipping rails (volts, +/-).
    pub gain1: f64,
    pub rails1: (f64, f64),
    pub gain2: f64,
    pub rails2: (f64, f64),
    pub stack: StackValues,
}

#[derive(Clone, Copy, Debug)]
pub enum PreSpec {
    Fender(FenderPreSpec),
    Vox(VoxPreSpec),
    Hybrid(HybridPreSpec),
}

#[derive(Clone, Copy, Debug)]
pub struct PiSpec {
    pub valve: Valve,
    pub vb: f64,
    pub ra1: f64,
    pub ra2: f64,
    pub rtail: f64,
    pub rbias: f64,
    /// Vox "Cut": series cap between the output-valve grids and the pot (ohm).
    pub cut: Option<(f64, f64)>,
}

#[derive(Clone, Copy, Debug)]
pub struct AmpSpec {
    pub pre: PreSpec,
    pub pi: PiSpec,
    pub power: PowerSpec,
    /// Alternative output cathode resistor (the AC30's 82 ohm) for `Controls::alt`.
    pub alt_rk: Option<f64>,
    /// Feedback fraction of the secondary voltage into the phase inverter.
    pub nfb: f64,
    pub ot: TransformerSpec,
    pub speaker: SpeakerSpec,
    /// How close the power stage is to a current source (see SpeakerLoad).
    pub current_drive: f64,
}

// --------------------------------------------------------------- preamps --

#[inline]
fn rail_clip(x: f64, pos: f64, neg: f64) -> f64 {
    // transistor stage running out of headroom: soft, asymmetric
    if x >= 0.0 {
        pos * (x / pos).tanh()
    } else {
        -neg * (-x / neg).tanh()
    }
}

#[derive(Clone)]
struct FenderPre {
    s: FenderPreSpec,
    fs: f64,
    input: GridInput,
    v1: TriodeStage,
    stack: Net,
    g2: GridInput,
    v2: TriodeStage,
    mix: Net,
}

const VOL: usize = 3;
const BRIGHT: usize = 4;
const CUT: usize = 0;

impl FenderPre {
    fn new(fs: f64, s: FenderPreSpec) -> Self {
        use net::stack_nodes::*;
        // V1 -> tone stack at its plate -> volume pot (+ bright cap) -> V2
        let vol = NEXT;
        let bright = NEXT + 1;
        let stack = net::tone_stack(s.stack, s.bass_taper, s.treble_taper)
            .input(IN, 100e3)
            .pot(0, vol, OUT, 1e6, Taper::Audio, VOL)
            .c(OUT, bright, s.bright_cap)
            .rheo(bright, vol, 1e12, Taper::Linear, BRIGHT)
            .output(vol)
            .build(fs);
        // V2 -> coupling cap -> 220k mixing resistor; the other channel's
        // 220k returns to its (low-impedance) plate; 1M PI grid return
        let mix = Circuit::new()
            .input(1, 100e3)
            .c(1, 2, s.mix_cap)
            .r(2, 3, 220e3)
            .r(3, 0, 220e3 + 40e3)
            .r(3, 0, 1e6)
            .output(3)
            .build(fs);
        Self {
            s,
            fs,
            input: GridInput::new(fs, 68e3, 0.0, 1e6, 120e-12),
            v1: TriodeStage::new(fs, s.v1, s.vb, 100e3, 1.5e3, 25e-6, 150e3),
            stack,
            g2: GridInput::new(fs, 250e3, 0.0, 1e9, 120e-12),
            v2: TriodeStage::new(fs, ECC83, s.vb, 100e3, s.v2_rk, s.v2_ck, 220e3),
            mix,
        }
    }

    fn set(&mut self, c: &Controls) {
        self.stack.set(net::TREBLE, c.treble);
        self.stack.set(net::BASS, c.bass);
        self.stack.set(net::MIDDLE, if self.s.fixed_mid { 1.0 } else { c.middle });
        self.stack.set(VOL, c.volume);
        let on = !self.s.bright_switch || c.bright;
        self.stack.set(BRIGHT, if on { 0.0 } else { 1.0 });
        // wiper source resistance seen by V2's grid (with the stack's output
        // impedance, ~ the treble pot, on top)
        let t = Taper::Audio.apply(c.volume);
        let rs = 1e6 * t * (1.0 - t) + 60e3 * t + 2e3;
        self.g2.set_source(self.fs, rs, 120e-12);
    }

    fn reset(&mut self) {
        self.input.reset();
        self.v1.reset();
        self.stack.reset();
        self.g2.reset();
        self.v2.reset();
        self.mix.reset();
    }

    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let v1 = &self.v1;
        let g = self.input.process(x, |v| v1.grid_current(v));
        let p1 = self.v1.process(g);
        let w = self.stack.run(p1);
        let v2 = &self.v2;
        let g = self.g2.process(w, |v| v2.grid_current(v));
        let p2 = self.v2.process(g);
        self.mix.run(p2)
    }
}

#[derive(Clone)]
struct VoxPre {
    fs: f64,
    input: GridInput,
    v1: TriodeStage,
    vol: Net,
    tb_in: GridInput,
    tb: TriodeStage,
    cf: CathodeFollower,
    stack: Net,
    /// Normal-channel path (V1 plate -> 0.047 -> 1M volume -> 220k mixing resistor).
    normal: Option<Net>,
}

impl VoxPre {
    fn new(fs: f64, s: VoxPreSpec) -> Self {
        use net::stack_nodes::*;
        // V1 plate -> 0.047 -> Brilliant volume (1M log, the Top Boost
        // stage's grid return) -> TB gain stage -> cathode follower ->
        // treble/bass network -> 220k mixing resistor -> phase inverter grid
        let vol = Circuit::new()
            .input(1, s.v1_ra)
            .c(1, 2, 0.047e-6)
            .pot(0, 3, 2, 1e6, Taper::Audio, 0)
            .output(3)
            .build(fs);
        let tb = TriodeStage::new(fs, ECC83, s.vb, s.tb_ra, s.tb_rk, s.tb_ck, f64::INFINITY);
        let cf = CathodeFollower::new(ECC83, s.vb, s.cf_rk, tb.vp0, s.tb_ra);
        let mix_node = NEXT;
        let stack = net::tone_stack(s.stack, Taper::Audio, Taper::Audio)
            .input(IN, cf.r_out)
            .r(OUT, mix_node, 220e3)
            // the other two channels' 220k mixing resistors, returning
            // through their volume controls
            .r(mix_node, 0, 110e3 + 50e3)
            .r(mix_node, 0, 1e6)
            .output(mix_node)
            .build(fs);
        let normal = if s.normal {
            Some(
                Circuit::new()
                    .input(1, s.v1_ra)
                    .c(1, 2, 0.047e-6)
                    .pot(0, 3, 2, 1e6, Taper::Audio, 0)
                    .r(3, 4, 220e3)
                    // the Brilliant and Vib-Trem channels' mixing resistors
                    .r(4, 0, 110e3 + 50e3)
                    .r(4, 0, 1e6)
                    .output(4)
                    .build(fs),
            )
        } else {
            None
        };
        Self {
            fs,
            input: GridInput::new(fs, 68e3, 0.0, 1e6, 120e-12),
            v1: TriodeStage::new(fs, ECC83, s.vb, s.v1_ra, s.v1_rk, s.v1_ck, 1e6),
            normal,
            vol,
            tb_in: GridInput::new(fs, 250e3, 0.0, 1e9, 120e-12),
            tb,
            cf,
            stack,
        }
    }

    fn set(&mut self, c: &Controls) {
        self.vol.set(0, c.volume);
        if let Some(n) = self.normal.as_mut() {
            n.set(0, c.volume);
        }
        self.stack.set(net::TREBLE, c.treble);
        self.stack.set(net::BASS, c.bass);
        self.stack.set(net::MIDDLE, 1.0);
        let t = Taper::Audio.apply(c.volume);
        let rs = 1e6 * t * (1.0 - t) + 180e3 * t + 1e3;
        self.tb_in.set_source(self.fs, rs, 120e-12);
    }

    fn reset(&mut self) {
        self.input.reset();
        self.v1.reset();
        self.vol.reset();
        self.tb_in.reset();
        self.tb.reset();
        self.cf.reset();
        self.stack.reset();
        if let Some(n) = self.normal.as_mut() {
            n.reset();
        }
    }

    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let v1 = &self.v1;
        let g = self.input.process(x, |v| v1.grid_current(v));
        let p1 = self.v1.process(g);
        if let Some(n) = self.normal.as_mut() {
            return n.run(p1);
        }
        let w = self.vol.run(p1);
        let tb = &self.tb;
        let g = self.tb_in.process(w, |v| tb.grid_current(v));
        let p2 = self.tb.process(g);
        let k = self.cf.process(p2);
        self.stack.run(k)
    }
}

#[derive(Clone)]
struct HybridPre {
    s: HybridPreSpec,
    hp: crate::dsp::Biquad,
    lp: crate::dsp::Biquad,
    stack: Net,
}

impl HybridPre {
    fn new(fs: f64, s: HybridPreSpec) -> Self {
        use net::stack_nodes::*;
        let mut hp = crate::dsp::Biquad::identity();
        hp.set_highpass(fs, 25.0, 0.6);
        let mut lp = crate::dsp::Biquad::identity();
        lp.set_lowpass(fs, 14000.0, 0.6);
        let stack = net::tone_stack(s.stack, Taper::Audio, Taper::Audio)
            .input(IN, 4.7e3)
            .pot(0, NEXT, OUT, 1e6, Taper::Audio, VOL)
            .output(NEXT)
            .build(fs);
        Self { s, hp, lp, stack }
    }

    fn set(&mut self, c: &Controls) {
        self.stack.set(net::TREBLE, c.treble);
        self.stack.set(net::BASS, c.bass);
        self.stack.set(net::MIDDLE, 1.0);
        self.stack.set(VOL, c.volume);
    }

    fn reset(&mut self) {
        self.hp.reset();
        self.lp.reset();
        self.stack.reset();
    }

    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let s = self.s;
        let a = rail_clip(self.hp.process(x) * s.gain1, s.rails1.0, s.rails1.1);
        let a = self.lp.process(a);
        let w = self.stack.run(a);
        // the second stage inverts, like the valve it replaces
        -rail_clip(w * s.gain2, s.rails2.0, s.rails2.1)
    }
}

#[derive(Clone)]
enum Pre {
    Fender(FenderPre),
    Vox(VoxPre),
    Hybrid(HybridPre),
}

// ------------------------------------------------------------------- amp --

/// A complete amplifier at the (oversampled) rate it was built for.
#[derive(Clone)]
pub struct Amp {
    spec: AmpSpec,
    fs: f64,
    pre: Pre,
    pi_in: GridInput,
    pi: LongTailPair,
    cut: Option<Net>,
    cut_comp: f64,
    power: PushPull,
    power_alt: Option<PushPull>,
    speaker: SpeakerLoad,
    ot: OutputTransformer,
    fb: f64,
    controls: Controls,
    /// Secondary voltage at full rated power (peak), for scaling.
    pub v_full_secondary: f64,
}

impl Amp {
    pub fn new(fs: f64, spec: AmpSpec) -> Self {
        let pre = match spec.pre {
            PreSpec::Fender(s) => Pre::Fender(FenderPre::new(fs, s)),
            PreSpec::Vox(s) => Pre::Vox(VoxPre::new(fs, s)),
            PreSpec::Hybrid(s) => Pre::Hybrid(HybridPre::new(fs, s)),
        };
        let p = spec.pi;
        let pi = LongTailPair::new(p.valve, p.vb, p.ra1, p.ra2, p.rtail, p.rbias);
        let pi_in = match spec.pre {
            // the mixing network already ends at the PI grid (its 1M return
            // is part of it)
            PreSpec::Fender(_) => GridInput::new(fs, 80e3, 0.0, 1e12, 60e-12),
            // so does the Vox channel mixer
            PreSpec::Vox(_) => GridInput::new(fs, 150e3, 0.0, 1e12, 60e-12),
            PreSpec::Hybrid(_) => GridInput::new(fs, 2e3, 0.047e-6, 1e6, 60e-12),
        };
        let ra = 0.5 * (p.ra1 + p.ra2);
        let rg = spec.power.rg;
        let (cut, r_drive, cut_comp) = match p.cut {
            Some((c, r)) => {
                let n = Circuit::new()
                    .input(1, p.ra1)
                    .input(2, p.ra2)
                    .r(1, 0, rg)
                    .r(2, 0, rg)
                    .c(1, 3, c)
                    .rheo(3, 2, r, Taper::Linear, CUT)
                    .output(1)
                    .output(2)
                    .build(fs);
                let rd = ra * rg / (ra + rg);
                (Some(n), rd, (rd + spec.power.r_stop + rg) / rg)
            }
            None => (None, ra, 1.0),
        };
        let power = PushPull::new(fs, spec.power, r_drive);
        let power_alt = spec.alt_rk.map(|rk| {
            let mut ps = spec.power;
            if let PowerBias::Cathode { ck, .. } = ps.bias {
                ps.bias = PowerBias::Cathode { rk, ck };
            }
            PushPull::new(fs, ps, r_drive)
        });
        let v_full_secondary = spec.ot.v_full * spec.ot.ratio;
        let mut a = Self {
            speaker: SpeakerLoad::new(fs, spec.speaker, spec.current_drive),
            ot: OutputTransformer::new(fs, spec.ot),
            spec,
            fs,
            pre,
            pi_in,
            pi,
            cut,
            cut_comp,
            power,
            power_alt,
            fb: 0.0,
            controls: Controls::default(),
            v_full_secondary,
        };
        a.set_controls(Controls::default());
        a
    }

    pub fn set_controls(&mut self, c: Controls) {
        match &mut self.pre {
            Pre::Fender(p) => p.set(&c),
            Pre::Vox(p) => p.set(&c),
            Pre::Hybrid(p) => p.set(&c),
        }
        if let Some(n) = self.cut.as_mut() {
            // knob up = less resistance in series with the cap = more cut
            n.set(CUT, 1.0 - c.cut);
        }
        if c.alt != self.controls.alt {
            // switching the cathode resistor: start the new bias settled
            if let Some(p) = self.power_alt.as_mut() {
                p.reset();
            }
            self.power.reset();
        }
        self.controls = c;
    }

    pub fn reset(&mut self) {
        match &mut self.pre {
            Pre::Fender(p) => p.reset(),
            Pre::Vox(p) => p.reset(),
            Pre::Hybrid(p) => p.reset(),
        }
        self.pi_in.reset();
        self.pi.reset();
        if let Some(n) = self.cut.as_mut() {
            n.reset();
        }
        self.power.reset();
        if let Some(p) = self.power_alt.as_mut() {
            p.reset();
        }
        self.speaker.reset();
        self.ot.reset();
        self.fb = 0.0;
    }

    fn power(&self) -> &PushPull {
        match (&self.power_alt, self.controls.alt) {
            (Some(p), true) => p,
            _ => &self.power,
        }
    }

    /// Present B+ sag in volts (for metering).
    pub fn sag(&self) -> f64 {
        self.spec.power.b_plus - self.power().supply()
    }

    /// Output-valve grid current (A), i.e. how hard the power stage is driven.
    pub fn power_grid_current(&self) -> f64 {
        self.power().grid_current
    }

    /// Guitar volts in, speaker volts out.
    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let pre = match &mut self.pre {
            Pre::Fender(p) => p.process(x),
            Pre::Vox(p) => p.process(x),
            Pre::Hybrid(p) => p.process(x),
        };
        let g1 = self.pi_in.process(pre, |_| (0.0, 0.0));
        let (a, b) = self.pi.process(g1, -self.spec.nfb * self.fb);
        let (d1, d2) = match self.cut.as_mut() {
            Some(n) => {
                let mut o = [0.0; 2];
                n.process(&[a, b], &mut o);
                (o[0] * self.cut_comp, o[1] * self.cut_comp)
            }
            None => (a, b),
        };
        let power = match (&mut self.power_alt, self.controls.alt) {
            (Some(p), true) => p,
            _ => &mut self.power,
        };
        let primary = power.process(d1, d2);
        let loaded = self.speaker.process(primary);
        let sec = self.ot.process(loaded);
        self.fb = sec;
        sec
    }

    pub fn sample_rate(&self) -> f64 {
        self.fs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amp::specs;
    use std::f64::consts::PI;

    fn run(amp: &mut Amp, f: f64, level: f64, secs: f64) -> (f64, f64) {
        // returns (fundamental amplitude, THD) of the steady-state output
        let fs = amp.sample_rate();
        let n = (fs * secs) as usize;
        let mut y = Vec::with_capacity(n);
        for k in 0..n {
            y.push(amp.process(level * (2.0 * PI * f * k as f64 / fs).sin()));
        }
        let tail = &y[n / 2..];
        let mut h = [0.0; 8];
        for (m, hm) in h.iter_mut().enumerate() {
            let (mut re, mut im) = (0.0, 0.0);
            for (k, v) in tail.iter().enumerate() {
                let ph = 2.0 * PI * f * (m + 1) as f64 * (k + n / 2) as f64 / fs;
                re += v * ph.sin();
                im += v * ph.cos();
            }
            *hm = 2.0 * (re * re + im * im).sqrt() / tail.len() as f64;
        }
        let thd = (h[1..].iter().map(|v| v * v).sum::<f64>()).sqrt() / h[0].max(1e-12);
        (h[0], thd)
    }

    #[test]
    fn every_amp_is_stable_and_breaks_up_with_volume() {
        let fs = 192000.0;
        for (name, spec) in specs::all() {
            let mut thds = vec![];
            for vol in [0.1, 0.5, 1.0] {
                let mut amp = Amp::new(fs, spec);
                amp.set_controls(Controls { volume: vol, ..Controls::default() });
                let (a, thd) = run(&mut amp, 220.0, 0.1, 0.5);
                assert!(a.is_finite() && a < amp.v_full_secondary * 3.0, "{name}: runaway {a}");
                println!(
                    "{name:>14} vol {vol:.1}: out {:6.2} V ({:5.1}% of full), THD {:5.1}%, sag {:4.1} V",
                    a,
                    100.0 * a / amp.v_full_secondary,
                    thd * 100.0,
                    amp.sag()
                );
                thds.push(thd);
            }
            assert!(thds[2] > thds[0], "{name}: more volume should mean more distortion");
            assert!(thds[0] < 0.05, "{name}: should be clean at low volume ({:.1}%)", thds[0] * 100.0);
        }
    }

    #[test]
    fn cpu_cost() {
        let fs = 192000.0;
        for (name, spec) in specs::all() {
            let mut a = Amp::new(fs, spec);
            a.set_controls(Controls { volume: 0.7, ..Controls::default() });
            let n = 192000;
            let t = std::time::Instant::now();
            let mut acc = 0.0;
            for k in 0..n {
                acc += a.process(0.3 * (k as f64 * 0.01).sin());
            }
            let ns = t.elapsed().as_nanos() as f64 / n as f64;
            // one second of 48 kHz audio at 4x oversampling = 192k samples
            println!("{name:>14}: {ns:.0} ns/sample -> {:.1}% of one core at 48 kHz ({acc:.0})", ns * 192000.0 / 1e7);
        }
    }

    #[test]
    fn feedback_lowers_gain() {
        let fs = 192000.0;
        let (_, spec) = specs::all().into_iter().find(|(n, _)| *n == "Deluxe Reverb").unwrap();
        let mut a = Amp::new(fs, spec);
        a.set_controls(Controls { volume: 0.15, ..Controls::default() });
        let (with, _) = run(&mut a, 1000.0, 0.05, 0.3);
        let mut open = spec;
        open.nfb = 0.0;
        let mut b = Amp::new(fs, open);
        b.set_controls(Controls { volume: 0.15, ..Controls::default() });
        let (without, _) = run(&mut b, 1000.0, 0.05, 0.3);
        let db = 20.0 * (without / with).log10();
        println!("feedback: {db:.1} dB");
        assert!((2.0..20.0).contains(&db));
    }
}

#[cfg(test)]
mod probe {
    use super::*;
    use crate::amp::specs;
    use std::f64::consts::PI;

    fn amp_of(mut f: impl FnMut(f64) -> f64, fs: f64, level: f64) -> f64 {
        let n = (fs * 0.3) as usize;
        let mut pk: f64 = 0.0;
        for k in 0..n {
            let y = f(level * (2.0 * PI * 1000.0 * k as f64 / fs).sin());
            if k > n / 2 {
                pk = pk.max(y.abs());
            }
        }
        pk / level
    }

    #[test]
    fn stage_gains() {
        let fs = 192000.0;
        let a = specs::ac30();
        if let PreSpec::Vox(s) = a.pre {
            let mut p = VoxPre::new(fs, s);
            p.set(&Controls { volume: 1.0, ..Default::default() });
            println!("V1 {:.1}", amp_of(|x| { let g = p.input.process(x, |_| (0.0, 0.0)); p.v1.process(g) * 1e6 / (1e6 + 220e3) }, fs, 0.01));
            let mut p = VoxPre::new(fs, s);
            p.set(&Controls { volume: 1.0, ..Default::default() });
            println!("TB {:.1}", amp_of(|x| { let g = p.tb_in.process(x, |_| (0.0, 0.0)); p.tb.process(g) }, fs, 0.01));
            println!("CF {:.2}  r_out {:.0} vg0 {:.1}", amp_of(|x| p.cf.process(x), fs, 0.1), p.cf.r_out, p.tb.vp0);
            for x in [-1.0, -0.1, 0.0, 0.1, 1.0] { println!("  cf({x}) = {:.4}", p.cf.process(x)); }
            println!("stack {:.3}", amp_of(|x| p.stack.run(x), fs, 1.0));
            let mut p = VoxPre::new(fs, s);
            p.set(&Controls { volume: 1.0, ..Default::default() });
            println!("pre total @vol1 {:.1}", amp_of(|x| p.process(x), fs, 0.001));
        }
        let pi = a.pi;
        let mut l = LongTailPair::new(pi.valve, pi.vb, pi.ra1, pi.ra2, pi.rtail, pi.rbias);
        println!("PI {:.1} (thevenin)", amp_of(|x| l.process(x, 0.0).0, fs, 0.01));
        let mut pp = PushPull::new(fs, a.power, 69e3);
        println!("power p-p primary per grid volt (diff) {:.1}", amp_of(|x| pp.process(x, -x), fs, 0.5));
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use crate::amp::specs;
    fn t(name: &str, mut f: impl FnMut(f64) -> f64) {
        let n = 400000;
        let st = std::time::Instant::now();
        let mut acc = 0.0;
        for k in 0..n {
            acc += f(0.3 * (k as f64 * 0.01).sin());
        }
        println!("{name:>12}: {:.0} ns ({acc:.1})", st.elapsed().as_nanos() as f64 / n as f64);
    }
    #[test]
    fn parts() {
        let fs = 192000.0;
        let a = specs::deluxe_reverb();
        let PreSpec::Fender(s) = a.pre else { panic!() };
        let mut p = FenderPre::new(fs, s);
        p.set(&Controls { volume: 0.7, ..Default::default() });
        t("fender pre", |x| p.process(x));
        let mut st = p.stack.clone();
        t("stack net", |x| st.run(x));
        let mut v1 = p.v1.clone();
        t("triode", |x| v1.process(x));
        let mut gi = p.input.clone();
        let v1 = p.v1.clone();
        t("grid in", |x| gi.process(x * 10.0, |v| v1.grid_current(v)));
        let pi = a.pi;
        let mut l = LongTailPair::new(pi.valve, pi.vb, pi.ra1, pi.ra2, pi.rtail, pi.rbias);
        t("ltp", |x| l.process(x, 0.0).0);
        let mut pp = PushPull::new(fs, a.power, 69e3);
        t("pushpull", |x| pp.process(x * 30.0, -x * 30.0));
        let mut ot = crate::amp::output::OutputTransformer::new(fs, a.ot);
        t("ot", |x| ot.process(x * 100.0));
    }
}
