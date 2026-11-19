//! Iron Room: the corrugated-iron trick from The Eton Rifles. Vic Coppersmith-Heaven lined the walls and floor
//! around Paul Weller's Vox AC30 with about thirty sheets of corrugated iron, for grit and a sharp, metallic
//! tone bounced back into the microphones: an AKG D12 right on the speaker and a Neumann U67 further back for
//! the room. Put it after an amp and cabinet. The sheets give a cluster of hard early reflections, a flutter
//! between the parallel sheets, the ring of the metal itself, and a buzz when the amp is loud enough to rattle
//! them. Reflection times and the sheets' ring estimated from the room as described.

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

#[derive(Params)]
pub struct P {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "iron"]
    pub iron: FloatParam,
    #[id = "ring"]
    pub ring: FloatParam,
    #[id = "rattle"]
    pub rattle: FloatParam,
    #[id = "distance"]
    pub distance: FloatParam,
    #[id = "room"]
    pub room: FloatParam,
    #[id = "output"]
    pub output: FloatParam,
}

impl Default for P {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            iron: pct("Iron", 0.7),
            ring: pct("Ring", 0.5),
            rattle: pct("Rattle", 0.3),
            distance: FloatParam::new("U67 Distance", 2.0, FloatRange::Linear { min: 0.5, max: 4.0 }).with_step_size(0.05).with_unit(" m"),
            room: pct("D12 / U67", 0.4),
            output: db("Output", 0.0, -24.0, 12.0),
        }
    }
}

pub const IRON: usize = 0;
pub const RING: usize = 1;
pub const RATTLE: usize = 2;
pub const DISTANCE: usize = 3;
pub const ROOM: usize = 4;
pub const OUTPUT: usize = 5;
pub const NPARAMS: usize = 6;

impl VintageParams for P {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.iron.as_ptr(),
            self.ring.as_ptr(),
            self.rattle.as_ptr(),
            self.distance.as_ptr(),
            self.room.as_ptr(),
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

/// Early reflections off the sheets: (time ms, gain) for the close mic and for the room mic (left, right).
const NEAR: [(f64, f64); 6] = [(0.9, 0.55), (1.6, 0.45), (2.4, 0.4), (3.3, 0.3), (4.1, 0.25), (5.2, 0.2)];
const FAR: [[(f64, f64); 8]; 2] = [
    [(2.1, 0.7), (3.4, 0.6), (4.9, 0.55), (6.3, 0.5), (7.7, 0.42), (9.4, 0.36), (11.8, 0.3), (14.1, 0.25)],
    [(2.6, 0.7), (3.9, 0.6), (5.3, 0.55), (6.9, 0.5), (8.4, 0.42), (10.2, 0.36), (12.5, 0.3), (15.0, 0.25)],
];
/// The modes the sheets ring at (Hz, ring time constant in seconds).
const MODES: [(f64, f64); 6] = [(1850.0, 0.05), (2600.0, 0.045), (3150.0, 0.04), (4650.0, 0.035), (6200.0, 0.03), (7900.0, 0.025)];

pub struct Dsp {
    fs: f64,
    v: [f32; NPARAMS],
    line: DelayLine,
    flutter: [DelayLine; 2],
    flutter_lp: [OnePole; 2],
    modes: [[Biquad; 6]; 2],
    rattle_bp: [Biquad; 2],
    env: Envelope,
    d12: [Biquad; 4],
    u67: [[Biquad; 3]; 2],
    iron: Smooth,
    ring: Smooth,
    rattle: Smooth,
    dist: Smooth,
    room: Smooth,
    out: Smooth,
}

impl VintageDsp for Dsp {
    const METERS: usize = 0;
    fn new(sample_rate: f32, _ch: usize) -> Self {
        let fs = sample_rate as f64;
        let mut v = [0.0; NPARAMS];
        v.copy_from_slice(&defaults());
        let mut modes = [[Biquad::identity(); 6]; 2];
        for (c, m) in modes.iter_mut().enumerate() {
            for (i, (f, tau)) in MODES.iter().enumerate() {
                // the two room-mic sides hear the sheets a little differently; Q from the ring time
                let f = f * (1.0 + 0.012 * c as f64);
                m[i].set_bandpass(fs, f, std::f64::consts::PI * f * tau);
            }
        }
        let mut d12 = [Biquad::identity(); 4];
        // AKG D12 on the speaker: a big low bump, a lift in the upper mids, the top rolled off
        d12[0].set_peak(fs, 110.0, 0.8, 4.0);
        d12[1].set_peak(fs, 1800.0, 0.7, -1.5);
        d12[2].set_peak(fs, 4000.0, 1.0, 3.0);
        d12[3].set_lowpass(fs, 8500.0, 0.7);
        let mut u67 = [[Biquad::identity(); 3]; 2];
        for u in u67.iter_mut() {
            // U67 (cardioid): open and even, a gentle presence rise, its low-cut at the 40 Hz step
            u[0].set_highpass(fs, 40.0, 0.7);
            u[1].set_peak(fs, 9000.0, 0.8, 2.0);
            u[2].set_high_shelf(fs, 12000.0, 0.7, -2.0);
        }
        let mut rattle_bp = [Biquad::identity(); 2];
        rattle_bp[0].set_bandpass(fs, 2400.0, 1.2);
        rattle_bp[1].set_bandpass(fs, 2700.0, 1.2);
        let mut flutter_lp = [OnePole::default(), OnePole::default()];
        flutter_lp.iter_mut().for_each(|p| p.set_lowpass(fs, 7000.0));
        Self {
            fs, v,
            line: DelayLine::new((fs * 0.05) as usize),
            flutter: [DelayLine::new((fs * 0.02) as usize), DelayLine::new((fs * 0.02) as usize)],
            flutter_lp, modes, rattle_bp,
            env: Envelope::new(fs, 2.0, 120.0),
            d12, u67,
            iron: Smooth::new(fs, 30.0, 0.7), ring: Smooth::new(fs, 30.0, 0.5), rattle: Smooth::new(fs, 30.0, 0.3),
            dist: Smooth::new(fs, 80.0, 2.0), room: Smooth::new(fs, 30.0, 0.4), out: Smooth::new(fs, 20.0, 1.0),
        }
    }
    fn reset(&mut self) {
        self.line.reset();
        self.flutter.iter_mut().for_each(|l| l.reset());
        self.flutter_lp.iter_mut().for_each(|p| p.reset());
        self.modes.iter_mut().flatten().chain(self.rattle_bp.iter_mut()).chain(self.d12.iter_mut()).for_each(|b| b.reset());
        self.u67.iter_mut().flatten().for_each(|b| b.reset());
        self.env.value = 0.0;
        let v = self.v;
        self.iron.snap(v[IRON] as f64);
        self.ring.snap(v[RING] as f64);
        self.rattle.snap(v[RATTLE] as f64);
        self.dist.snap(v[DISTANCE] as f64);
        self.room.snap(v[ROOM] as f64);
        self.out.snap(db_to_gain(v[OUTPUT] as f64));
    }
    fn set_params(&mut self, v: &[f32]) {
        self.v.copy_from_slice(&v[..NPARAMS]);
        self.iron.set(v[IRON] as f64);
        self.ring.set(v[RING] as f64);
        self.rattle.set(v[RATTLE] as f64);
        self.dist.set(v[DISTANCE] as f64);
        self.room.set(v[ROOM] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }
    fn process(&mut self, chans: &mut [&mut [f32]], _meters: &mut [f32]) {
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nc = chans.len().max(1);
        let ms = self.fs * 0.001;
        // flutter between the parallel sheets: about 1.6 m apart
        let flutter_d = [0.0093 * self.fs, 0.0101 * self.fs];
        for k in 0..n {
            let iron = self.iron.next();
            let ring = self.ring.next();
            let rat = self.rattle.next();
            let dist = self.dist.next();
            let room = self.room.next();
            let o = self.out.next();
            let x = chans.iter().map(|c| c[k] as f64).sum::<f64>() / nc as f64;
            self.line.write(x);
            let e = self.env.process(x);
            // close mic: the speaker, with the nearest sheets just behind it
            let mut near = 0.0;
            for (t, g) in NEAR {
                near += g * self.line.read(t * ms);
            }
            let close = cascade(&mut self.d12, x + iron * 0.35 * near);
            // room mic: the direct sound arrives later by the distance, then the sheets all round
            let direct_d = (dist / 343.0) * self.fs;
            let mut ys = [0.0; 2];
            for c in 0..2 {
                let direct = self.line.read(direct_d.max(1.0)) / dist.max(0.5);
                let mut refl = 0.0;
                for (t, g) in FAR[c] {
                    refl += g * self.line.read(direct_d + t * ms * (0.6 + 0.4 * dist / 2.0));
                }
                refl *= iron;
                // the flutter: a short, metallic repeat between parallel sheets
                let fl = self.flutter[c].read(flutter_d[c]);
                let fl_in = refl * 0.5 + self.flutter_lp[c].lp(fl) * (0.55 * iron);
                self.flutter[c].write(fl_in);
                // the iron rings at its own modes
                let mut metal = 0.0;
                for m in self.modes[c].iter_mut() {
                    metal += m.process(refl + fl);
                }
                // loud enough, and the sheets buzz against each other
                let loud = ((e - 0.05) / 0.25).clamp(0.0, 1.0);
                let b = self.rattle_bp[c].process(refl);
                let buzz = ((b * 25.0).clamp(-1.0, 1.0) * 0.06 - b * 0.15) * loud * rat * iron;
                let r = direct + 0.6 * refl + 0.5 * fl + ring * 6.0 * metal * iron + buzz;
                ys[c] = cascade(&mut self.u67[c], r);
            }
            for (ci, ch) in chans.iter_mut().enumerate() {
                let c = ci.min(1);
                ch[k] = (((1.0 - room) * close + room * ys[c] * 1.2) * o) as f32;
            }
        }
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "The Eton Rifles (30 sheets)", values: &[] },
        Preset { name: "Close D12 Only, a Little Iron", values: &[(IRON, 0.5), (ROOM, 0.0)] },
        Preset { name: "U67 in the Iron Room", values: &[(IRON, 0.8), (ROOM, 0.85), (DISTANCE, 2.5)] },
        Preset { name: "Rattling Sheets (loud)", values: &[(IRON, 1.0), (RING, 0.7), (RATTLE, 0.9), (ROOM, 0.5)] },
        Preset { name: "Sharp and Metallic", values: &[(IRON, 0.9), (RING, 0.9), (RATTLE, 0.2), (ROOM, 0.5), (DISTANCE, 1.2)] },
        Preset { name: "Few Sheets, Subtle Grit", values: &[(IRON, 0.3), (RING, 0.3), (RATTLE, 0.2), (ROOM, 0.3)] },
        Preset { name: "Back of the Room", values: &[(IRON, 0.7), (ROOM, 0.7), (DISTANCE, 4.0)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "IRON ROOM",
        subtitle: ("THE ETON RIFLES GUITAR ROOM", "CORRUGATED IRON · D12 CLOSE · U67 BACK"),
        accent: ui::PINE,
        size: (W, H),
        items: vec![
            Item::Text { x: 320.0, y: 100.0, text: "PUT IT AFTER THE AMP AND CABINET", size: 11.0, align: Align2::CENTER_CENTER },
            Item::Knob { p: IRON, x: 140.0, y: 170.0, r: 30.0, label: "IRON", tip: "How many sheets of corrugated iron line the walls and floor round the amp (about thirty on The Eton Rifles).", active: None },
            Item::Knob { p: RING, x: 320.0, y: 170.0, r: 30.0, label: "RING", tip: "The metal's own ring: the sharp, metallic tone.", active: None },
            Item::Knob { p: RATTLE, x: 500.0, y: 170.0, r: 30.0, label: "RATTLE", tip: "The sheets buzzing against each other when the amp is loud.", active: None },
            Item::Divider { y: 236.0 },
            Item::Knob { p: DISTANCE, x: 140.0, y: 310.0, r: 28.0, label: "U67 DISTANCE", tip: "How far back the Neumann U67 room mic is.", active: None },
            Item::Knob { p: ROOM, x: 320.0, y: 310.0, r: 28.0, label: "D12 / U67", tip: "The AKG D12 on the speaker (left) against the U67 in the room (right).", active: None },
            Item::Knob { p: OUTPUT, x: 500.0, y: 310.0, r: 28.0, label: "OUTPUT", tip: "Output level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, P, Dsp, panel,
    name: "Iron Room", class_id: *b"TJamIronRoomPQD1", vendor: "PEQUOD The Jam",
    categories: [Fx, Reverb]
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

    fn impulse(p: &[f32], amp: f32) -> Vec<f32> {
        let mut x = vec![0.0f32; 24000];
        x[100] = amp;
        go(p, &x)
    }
    #[test]
    fn iron_adds_reflections() {
        let a = impulse(&with(defaults(), &[(IRON, 0.0), (ROOM, 1.0)]), 0.5);
        let b = impulse(&with(defaults(), &[(IRON, 1.0), (ROOM, 1.0)]), 0.5);
        // after the direct sound has passed (2 m: 5.8 ms)
        let (ea, eb) = (rms(&a[500..2500]), rms(&b[500..2500]));
        println!("reflections: iron 0 {:.1} dB, iron 1 {:.1} dB", db(ea), db(eb));
        assert!(db(eb) - db(ea) > 10.0);
    }
    #[test]
    fn the_metal_rings() {
        let a = impulse(&with(defaults(), &[(RING, 0.0), (ROOM, 1.0)]), 0.5);
        let b = impulse(&with(defaults(), &[(RING, 1.0), (ROOM, 1.0)]), 0.5);
        let (ta, tb) = (rms(&a[2400..4800]), rms(&b[2400..4800]));
        println!("50-100 ms tail: ring 0 {:.1} dB, ring 1 {:.1} dB", db(ta), db(tb));
        assert!(db(tb) - db(ta) > 6.0);
    }
    #[test]
    fn rattles_only_when_loud() {
        let quiet = sine(220.0, 0.02, 48000);
        let loud = sine(220.0, 0.5, 48000);
        let off = |x: &[f32]| go(&with(defaults(), &[(RATTLE, 0.0), (ROOM, 1.0)]), x);
        let on = |x: &[f32]| go(&with(defaults(), &[(RATTLE, 1.0), (ROOM, 1.0)]), x);
        let diff = |a: &[f32], b: &[f32]| rms(&a[24000..].iter().zip(&b[24000..]).map(|(p, q)| p - q).collect::<Vec<f32>>()) / rms(&a[24000..]);
        let dq = diff(&off(&quiet), &on(&quiet));
        let dl = diff(&off(&loud), &on(&loud));
        println!("rattle changes: quiet {:.1} dB, loud {:.1} dB", db(dq), db(dl));
        assert!(dq < 1e-6 && db(dl) > -40.0);
    }

}
