//! Passive linear networks (tone stacks, volume and bright circuits, mixing
//! resistors) solved by modified nodal analysis with trapezoidal capacitor
//! companions — i.e. the exact circuit, discretised with the bilinear
//! transform. Pots are real three-terminal pots with their taper; the
//! matrix is refactored only when a control moves.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Taper {
    Linear,
    /// Log/"audio" taper (10% resistance at half rotation).
    Audio,
}

impl Taper {
    #[inline]
    pub fn apply(self, pos: f64) -> f64 {
        let p = pos.clamp(0.0, 1.0);
        match self {
            Taper::Linear => p,
            Taper::Audio => (81f64.powf(p) - 1.0) / 80.0,
        }
    }
}

#[derive(Clone, Debug)]
enum Elem {
    R { a: usize, b: usize, r: f64 },
    C { a: usize, b: usize, c: f64 },
    /// Three-terminal pot: `a` = CCW end, `w` = wiper, `b` = CW end.
    /// Rotation 1.0 puts the wiper at `b`.
    Pot { a: usize, w: usize, b: usize, r: f64, taper: Taper, ctl: usize },
    /// Variable resistor (wiper tied to one end): resistance = r * taper(pos).
    Rheo { a: usize, b: usize, r: f64, taper: Taper, ctl: usize },
}

/// Describe a circuit. Node 0 is ground; nodes are numbered from 1.
#[derive(Clone, Debug, Default)]
pub struct Circuit {
    nodes: usize,
    elems: Vec<Elem>,
    /// Voltage sources with series resistance: (node, resistance).
    inputs: Vec<(usize, f64)>,
    outputs: Vec<usize>,
    controls: usize,
}

const R_MIN: f64 = 2.0;

impl Circuit {
    pub fn new() -> Self {
        Self::default()
    }
    fn touch(&mut self, n: usize) {
        self.nodes = self.nodes.max(n);
    }
    pub fn r(mut self, a: usize, b: usize, r: f64) -> Self {
        self.touch(a.max(b));
        self.elems.push(Elem::R { a, b, r });
        self
    }
    pub fn c(mut self, a: usize, b: usize, c: f64) -> Self {
        self.touch(a.max(b));
        self.elems.push(Elem::C { a, b, c });
        self
    }
    pub fn pot(mut self, a: usize, w: usize, b: usize, r: f64, taper: Taper, ctl: usize) -> Self {
        self.touch(a.max(b).max(w));
        self.controls = self.controls.max(ctl + 1);
        self.elems.push(Elem::Pot { a, w, b, r, taper, ctl });
        self
    }
    pub fn rheo(mut self, a: usize, b: usize, r: f64, taper: Taper, ctl: usize) -> Self {
        self.touch(a.max(b));
        self.controls = self.controls.max(ctl + 1);
        self.elems.push(Elem::Rheo { a, b, r, taper, ctl });
        self
    }
    /// Signal source (Thevenin: voltage behind `rs`) into `node`.
    pub fn input(mut self, node: usize, rs: f64) -> Self {
        self.touch(node);
        self.inputs.push((node, rs.max(R_MIN)));
        self
    }
    pub fn output(mut self, node: usize) -> Self {
        self.touch(node);
        self.outputs.push(node);
        self
    }

    pub fn build(self, fs: f64) -> Net {
        Net::new(self, fs)
    }
}

#[derive(Clone)]
struct Cap {
    a: usize,
    b: usize,
    g: f64,
}

/// A discretised circuit, ready to run. Internally the nodal solution is
/// folded into a small state-space system over the capacitor states, so a
/// sample costs (caps + inputs) x (caps + outputs) multiply-adds; the
/// matrices are rebuilt (without allocating) when a control moves.
#[derive(Clone)]
pub struct Net {
    c: Circuit,
    n: usize,
    caps: Vec<Cap>,
    ctl: Vec<f64>,
    dirty: bool,
    // scratch for refactoring
    m: Vec<f64>,
    work: Vec<f64>,
    inv: Vec<f64>,
    // state space: j' = sa j + sb u ; y = sc j + sd u
    j: Vec<f64>,
    j_next: Vec<f64>,
    sa: Vec<f64>,
    sb: Vec<f64>,
    sc: Vec<f64>,
    sd: Vec<f64>,
}

impl Net {
    fn new(c: Circuit, fs: f64) -> Self {
        let n = c.nodes;
        let caps: Vec<Cap> = c
            .elems
            .iter()
            .filter_map(|e| match e {
                Elem::C { a, b, c } => Some(Cap { a: *a, b: *b, g: 2.0 * fs * c }),
                _ => None,
            })
            .collect();
        let (nc, ni, no) = (caps.len(), c.inputs.len(), c.outputs.len());
        let mut net = Self {
            n,
            ctl: vec![0.5; c.controls],
            dirty: true,
            m: vec![0.0; n * n],
            work: vec![0.0; n * n],
            inv: vec![0.0; n * n],
            j: vec![0.0; nc],
            j_next: vec![0.0; nc],
            sa: vec![0.0; nc * nc],
            sb: vec![0.0; nc * ni],
            sc: vec![0.0; no * nc],
            sd: vec![0.0; no * ni],
            caps,
            c,
        };
        net.refactor();
        net
    }

    pub fn reset(&mut self) {
        self.j.iter_mut().for_each(|v| *v = 0.0);
    }

    /// Set a control (pot rotation 0..1). Takes effect on the next sample.
    pub fn set(&mut self, ctl: usize, pos: f64) {
        let pos = pos.clamp(0.0, 1.0);
        if (self.ctl[ctl] - pos).abs() > 1e-9 {
            self.ctl[ctl] = pos;
            self.dirty = true;
        }
    }

    fn refactor(&mut self) {
        let n = self.n;
        let m = &mut self.m;
        m.iter_mut().for_each(|v| *v = 0.0);
        let mut stamp = |a: usize, b: usize, g: f64| {
            if a > 0 {
                m[(a - 1) * n + a - 1] += g;
            }
            if b > 0 {
                m[(b - 1) * n + b - 1] += g;
            }
            if a > 0 && b > 0 {
                m[(a - 1) * n + b - 1] -= g;
                m[(b - 1) * n + a - 1] -= g;
            }
        };
        for e in &self.c.elems {
            match *e {
                Elem::R { a, b, r } => stamp(a, b, 1.0 / r.max(R_MIN)),
                Elem::C { .. } => {}
                Elem::Pot { a, w, b, r, taper, ctl } => {
                    let t = taper.apply(self.ctl[ctl]);
                    stamp(a, w, 1.0 / (r * t).max(R_MIN));
                    stamp(w, b, 1.0 / (r * (1.0 - t)).max(R_MIN));
                }
                Elem::Rheo { a, b, r, taper, ctl } => {
                    let t = taper.apply(self.ctl[ctl]);
                    stamp(a, b, 1.0 / (r * t).max(R_MIN));
                }
            }
        }
        for cap in &self.caps {
            stamp(cap.a, cap.b, cap.g);
        }
        for &(node, rs) in &self.c.inputs {
            stamp(node, 0, 1.0 / rs);
        }
        // tiny leak keeps floating nodes solvable
        for i in 0..n {
            self.m[i * n + i] += 1e-12;
        }
        invert_into(&self.m, &mut self.work, &mut self.inv, n);

        // node voltage response to one unit of each source
        let inv = &self.inv;
        let at = |row: usize, col: usize| if row == 0 || col == 0 { 0.0 } else { inv[(row - 1) * n + col - 1] };
        // v[row] per unit J_k (injected +a, -b) and per unit input u_i
        let vj = |row: usize, k: &Cap| at(row, k.a) - at(row, k.b);
        let vu = |row: usize, i: usize| {
            let (node, rs) = self.c.inputs[i];
            at(row, node) / rs
        };
        let (nc, ni) = (self.caps.len(), self.c.inputs.len());
        for (k, ck) in self.caps.iter().enumerate() {
            for (jdx, cj) in self.caps.iter().enumerate() {
                let w = vj(ck.a, cj) - vj(ck.b, cj);
                self.sa[k * nc + jdx] = 2.0 * ck.g * w - if k == jdx { 1.0 } else { 0.0 };
            }
            for i in 0..ni {
                let w = vu(ck.a, i) - vu(ck.b, i);
                self.sb[k * ni + i] = 2.0 * ck.g * w;
            }
        }
        for (o, &node) in self.c.outputs.iter().enumerate() {
            for (jdx, cj) in self.caps.iter().enumerate() {
                self.sc[o * nc + jdx] = vj(node, cj);
            }
            for i in 0..ni {
                self.sd[o * ni + i] = vu(node, i);
            }
        }
        self.dirty = false;
    }

    /// Run one sample: one voltage per input, returns the output node voltages.
    #[inline]
    pub fn process(&mut self, inputs: &[f64], out: &mut [f64]) {
        if self.dirty {
            self.refactor();
        }
        let (nc, ni) = (self.j.len(), inputs.len());
        for (o, y) in out.iter_mut().enumerate() {
            let mut acc = 0.0;
            for (a, b) in self.sc[o * nc..(o + 1) * nc].iter().zip(&self.j) {
                acc += a * b;
            }
            for (a, b) in self.sd[o * ni..(o + 1) * ni].iter().zip(inputs) {
                acc += a * b;
            }
            *y = acc;
        }
        for k in 0..nc {
            let mut acc = 0.0;
            for (a, b) in self.sa[k * nc..(k + 1) * nc].iter().zip(&self.j) {
                acc += a * b;
            }
            for (a, b) in self.sb[k * ni..(k + 1) * ni].iter().zip(inputs) {
                acc += a * b;
            }
            self.j_next[k] = acc;
        }
        std::mem::swap(&mut self.j, &mut self.j_next);
    }

    /// Single input, single output convenience.
    #[inline]
    pub fn run(&mut self, x: f64) -> f64 {
        let mut o = [0.0];
        self.process(&[x], &mut o);
        o[0]
    }
}

fn invert_into(m: &[f64], a: &mut [f64], inv: &mut [f64], n: usize) {
    a.copy_from_slice(m);
    inv.iter_mut().for_each(|v| *v = 0.0);
    for i in 0..n {
        inv[i * n + i] = 1.0;
    }
    for col in 0..n {
        let mut piv = col;
        for r in col + 1..n {
            if a[r * n + col].abs() > a[piv * n + col].abs() {
                piv = r;
            }
        }
        if piv != col {
            for k in 0..n {
                a.swap(col * n + k, piv * n + k);
                inv.swap(col * n + k, piv * n + k);
            }
        }
        let d = a[col * n + col];
        let d = if d.abs() < 1e-300 { 1e-300 } else { d };
        for k in 0..n {
            a[col * n + k] /= d;
            inv[col * n + k] /= d;
        }
        for r in 0..n {
            if r != col {
                let f = a[r * n + col];
                if f != 0.0 {
                    for k in 0..n {
                        a[r * n + k] -= f * a[col * n + k];
                        inv[r * n + k] -= f * inv[col * n + k];
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------ tone stacks --

/// Component values of a treble/bass/middle stack in Yeh's notation:
/// R1 treble pot, R2 bass pot, R3 middle pot (or fixed resistor), R4 slope
/// resistor, C1 treble cap, C2 bass cap, C3 middle cap.
#[derive(Clone, Copy, Debug)]
pub struct StackValues {
    pub r1: f64,
    pub r2: f64,
    pub r3: f64,
    pub r4: f64,
    pub c1: f64,
    pub c2: f64,
    pub c3: f64,
}

/// Node numbers of the stack built by [`tone_stack`].
pub mod stack_nodes {
    pub const IN: usize = 1;
    pub const OUT: usize = 3;
    /// First free node number for circuitry added after the stack.
    pub const NEXT: usize = 8;
}

/// Controls used by [`tone_stack`].
pub const TREBLE: usize = 0;
pub const BASS: usize = 1;
pub const MIDDLE: usize = 2;

/// The Fender/Marshall-style passive stack (also Guitarix/Yeh's model of
/// the Vox Top Boost network). `mid_fixed` = Some(pos) bolts the middle
/// pot at that rotation (amps with no middle control).
///
/// Node map: 1 in, 2 treble pot top, 3 treble wiper (out), 4 treble pot
/// bottom / bass pot top, 5 slope junction, 6 middle pot top, 7 middle wiper.
pub fn tone_stack(v: StackValues, bass_taper: Taper, treble_taper: Taper) -> Circuit {
    Circuit::new()
        .c(1, 2, v.c1)
        // treble pot: rotation 1 = wiper at the top
        .pot(4, 3, 2, v.r1, treble_taper, TREBLE)
        .r(1, 5, v.r4)
        .c(5, 4, v.c2)
        // bass rheostat between the treble pot bottom and the middle pot top
        .rheo(4, 6, v.r2, bass_taper, BASS)
        .c(5, 7, v.c3)
        // middle pot: rotation 1 = full resistance below the wiper
        .pot(0, 7, 6, v.r3, Taper::Linear, MIDDLE)
}

pub const AB763_DELUXE: StackValues = StackValues { r1: 250e3, r2: 250e3, r3: 6.8e3, r4: 100e3, c1: 250e-12, c2: 0.1e-6, c3: 0.047e-6 };
pub const AB763_TWIN: StackValues = StackValues { r1: 250e3, r2: 250e3, r3: 10e3, r4: 100e3, c1: 250e-12, c2: 0.1e-6, c3: 0.047e-6 };
pub const AA270_TWIN: StackValues = StackValues { r1: 250e3, r2: 250e3, r3: 10e3, r4: 100e3, c1: 120e-12, c2: 0.1e-6, c3: 0.047e-6 };
pub const BASSMAN_6G6: StackValues = StackValues { r1: 250e3, r2: 1e6, r3: 10e3, r4: 100e3, c1: 250e-12, c2: 0.1e-6, c3: 0.047e-6 };
pub const VOX_TB: StackValues = StackValues { r1: 1e6, r2: 1e6, r3: 10e3, r4: 100e3, c1: 50e-12, c2: 22e-9, c3: 22e-9 };

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// Yeh's closed form (as used in Guitarix tonestack.dsp), analogue H(s).
    #[allow(non_snake_case)]
    fn yeh(v: StackValues, t: f64, m: f64, l: f64, s: (f64, f64)) -> (f64, f64) {
        let StackValues { r1: R1, r2: R2, r3: R3, r4: R4, c1: C1, c2: C2, c3: C3 } = v;
        #[allow(non_snake_case)]
        let b1 = t * C1 * R1 + m * C3 * R3 + l * (C1 * R2 + C2 * R2) + (C1 * R3 + C2 * R3);
        let b2 = t * (C1 * C2 * R1 * R4 + C1 * C3 * R1 * R4) - m * m * (C1 * C3 * R3 * R3 + C2 * C3 * R3 * R3)
            + m * (C1 * C3 * R1 * R3 + C1 * C3 * R3 * R3 + C2 * C3 * R3 * R3)
            + l * (C1 * C2 * R1 * R2 + C1 * C2 * R2 * R4 + C1 * C3 * R2 * R4)
            + l * m * (C1 * C3 * R2 * R3 + C2 * C3 * R2 * R3)
            + (C1 * C2 * R1 * R3 + C1 * C2 * R3 * R4 + C1 * C3 * R3 * R4);
        let b3 = l * m * (C1 * C2 * C3 * R1 * R2 * R3 + C1 * C2 * C3 * R2 * R3 * R4)
            - m * m * (C1 * C2 * C3 * R1 * R3 * R3 + C1 * C2 * C3 * R3 * R3 * R4)
            + m * (C1 * C2 * C3 * R1 * R3 * R3 + C1 * C2 * C3 * R3 * R3 * R4)
            + t * C1 * C2 * C3 * R1 * R3 * R4
            - t * m * C1 * C2 * C3 * R1 * R3 * R4
            + t * l * C1 * C2 * C3 * R1 * R2 * R4;
        let a1 = (C1 * R1 + C1 * R3 + C2 * R3 + C2 * R4 + C3 * R4) + m * C3 * R3 + l * (C1 * R2 + C2 * R2);
        let a2 = m * (C1 * C3 * R1 * R3 - C2 * C3 * R3 * R4 + C1 * C3 * R3 * R3 + C2 * C3 * R3 * R3)
            + l * m * (C1 * C3 * R2 * R3 + C2 * C3 * R2 * R3)
            - m * m * (C1 * C3 * R3 * R3 + C2 * C3 * R3 * R3)
            + l * (C1 * C2 * R2 * R4 + C1 * C2 * R1 * R2 + C1 * C3 * R2 * R4 + C2 * C3 * R2 * R4)
            + (C1 * C2 * R1 * R4 + C1 * C3 * R1 * R4 + C1 * C2 * R3 * R4 + C1 * C2 * R1 * R3 + C1 * C3 * R3 * R4 + C2 * C3 * R3 * R4);
        let a3 = l * m * (C1 * C2 * C3 * R1 * R2 * R3 + C1 * C2 * C3 * R2 * R3 * R4)
            - m * m * (C1 * C2 * C3 * R1 * R3 * R3 + C1 * C2 * C3 * R3 * R3 * R4)
            + m * (C1 * C2 * C3 * R3 * R3 * R4 + C1 * C2 * C3 * R1 * R3 * R3 - C1 * C2 * C3 * R1 * R3 * R4)
            + l * C1 * C2 * C3 * R1 * R2 * R4
            + C1 * C2 * C3 * R1 * R3 * R4;
        // complex polynomial evaluation at s = (0, w)
        let w = s.1;
        let num = (-b2 * w * w, b1 * w - b3 * w * w * w);
        let den = (1.0 - a2 * w * w, a1 * w - a3 * w * w * w);
        let d2 = den.0 * den.0 + den.1 * den.1;
        ((num.0 * den.0 + num.1 * den.1) / d2, (num.1 * den.0 - num.0 * den.1) / d2)
    }

    fn measure(net: &mut Net, fs: f64, f: f64) -> f64 {
        net.reset();
        let n = (fs * 0.5) as usize;
        let (mut re, mut im, mut cnt) = (0.0, 0.0, 0.0);
        for k in 0..n {
            let ph = 2.0 * PI * f * k as f64 / fs;
            let y = net.run(ph.sin());
            if k > n / 2 {
                re += y * ph.sin();
                im += y * ph.cos();
                cnt += 1.0;
            }
        }
        2.0 * (re * re + im * im).sqrt() / cnt
    }

    #[test]
    fn matches_yeh_closed_form() {
        let fs = 96000.0;
        for v in [AB763_DELUXE, AB763_TWIN, BASSMAN_6G6, VOX_TB] {
            for (t, m, l) in [(0.5, 0.5, 0.5), (0.9, 0.2, 0.1), (0.1, 0.9, 0.95)] {
                let mut net = tone_stack(v, Taper::Linear, Taper::Linear).input(1, 1.0).output(3).build(fs);
                net.set(TREBLE, t);
                net.set(BASS, l);
                net.set(MIDDLE, m);
                for f in [40.0, 120.0, 400.0, 1000.0, 3000.0, 8000.0] {
                    let wd = 2.0 * fs * (PI * f / fs).tan(); // bilinear prewarp
                    let (re, im) = yeh(v, t, m, l, (0.0, wd));
                    let want = (re * re + im * im).sqrt();
                    let got = measure(&mut net, fs, f);
                    let err_db = 20.0 * (got / want).log10();
                    assert!(err_db.abs() < 0.05, "{v:?} t{t} m{m} l{l} {f} Hz: {err_db:.3} dB");
                }
            }
        }
    }

    #[test]
    fn audio_taper_is_ten_percent_at_half() {
        assert!((Taper::Audio.apply(0.5) - 0.1).abs() < 1e-12);
        assert_eq!(Taper::Audio.apply(0.0), 0.0);
        assert!((Taper::Audio.apply(1.0) - 1.0).abs() < 1e-12);
    }
}
