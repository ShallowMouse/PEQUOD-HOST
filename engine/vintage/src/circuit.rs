//! Nonlinear circuits, solved sample by sample: modified nodal analysis with
//! trapezoidal capacitor companions and Newton-Raphson for the semiconductors
//! (diodes, bipolar transistors as Ebers-Moll, JFETs as the square law,
//! op-amps and OTAs with their output limits). Built from the schematic's own
//! part values, so a pedal or a channel behaves like the circuit: its bias,
//! its clipping, its loading and its frequency response all come out of the
//! same solve.
//!
//! The circuits here are small (a stage or a pedal: up to a couple of dozen
//! nodes) and are solved densely. Node 0 is ground.

use crate::amp::net::Taper;

/// Thermal voltage at room temperature.
pub const VT: f64 = 0.02585;

#[derive(Clone, Copy, Debug)]
pub struct BjtModel {
    pub is: f64,
    pub bf: f64,
    pub br: f64,
    /// Emission coefficient (germanium parts run a little above 1).
    pub n: f64,
    /// Collector-emitter leakage (germanium: microamps; silicon: none).
    pub leak: f64,
}

/// Small-signal silicon NPN (BC108/BC109/2N5088 class).
pub const BC108: BjtModel = BjtModel { is: 1.8e-14, bf: 300.0, br: 5.0, n: 1.0, leak: 0.0 };
/// Low-noise high-gain silicon NPN (2N5088 / BC549C).
pub const N5088: BjtModel = BjtModel { is: 2.0e-14, bf: 600.0, br: 5.0, n: 1.0, leak: 0.0 };
/// High-gain low-noise silicon NPN (BC184L / BC184C).
pub const BC184: BjtModel = BjtModel { is: 2.0e-14, bf: 500.0, br: 5.0, n: 1.0, leak: 0.0 };
/// General-purpose silicon NPN (2N3904 / BC547).
pub const N3904: BjtModel = BjtModel { is: 6.7e-15, bf: 260.0, br: 4.0, n: 1.0, leak: 0.0 };
/// Germanium PNP (AC128 / NKT275 class): low gain spread here set mid-range, leaky.
pub const AC128: BjtModel = BjtModel { is: 2.0e-7, bf: 90.0, br: 2.0, n: 1.25, leak: 100e-6 };
/// Germanium PNP with low leakage (a selected NKT275).
pub const NKT275: BjtModel = BjtModel { is: 1.5e-7, bf: 80.0, br: 2.0, n: 1.2, leak: 60e-6 };

#[derive(Clone, Copy, Debug)]
pub struct DiodeModel {
    pub is: f64,
    pub n: f64,
}

/// 1N914 / 1N4148 silicon signal diode.
pub const D1N914: DiodeModel = DiodeModel { is: 2.52e-9, n: 1.752 };
/// 1N34A germanium point-contact diode.
pub const D1N34: DiodeModel = DiodeModel { is: 2.0e-7, n: 1.3 };
/// Red LED.
pub const LED_RED: DiodeModel = DiodeModel { is: 1.0e-19, n: 1.9 };

#[derive(Clone, Copy, Debug)]
pub struct JfetModel {
    /// Pinch-off voltage (negative for N-channel).
    pub vp: f64,
    pub idss: f64,
    pub lambda: f64,
}

/// 2N5952 (the MXR Phase 90's FETs), a matched-ish part.
pub const J2N5952: JfetModel = JfetModel { vp: -1.6, idss: 4.0e-3, lambda: 0.01 };
/// J201 (low pinch-off).
pub const J201: JfetModel = JfetModel { vp: -0.8, idss: 0.6e-3, lambda: 0.01 };

#[derive(Clone, Debug)]
enum El {
    R { a: usize, b: usize, g: f64 },
    C { a: usize, b: usize, c: f64, k: usize },
    L { a: usize, b: usize, l: f64, k: usize },
    /// Ideal capacitor across which only the charge state matters; see C.
    Pot { a: usize, w: usize, b: usize, r: f64, taper: Taper, ctl: usize },
    Rheo { a: usize, b: usize, r: f64, taper: Taper, ctl: usize, min: f64 },
    /// Externally set resistance (photocells, FET channels driven from outside).
    Var { a: usize, b: usize, slot: usize },
    /// Source with series resistance: `src` indexes the per-sample inputs; DC sources use fixed values.
    Src { node: usize, rs: f64, src: usize },
    Dc { node: usize, rs: f64, v: f64 },
    Diode { a: usize, k: usize, m: DiodeModel },
    Bjt { c: usize, b: usize, e: usize, pnp: bool, m: BjtModel },
    Jfet { d: usize, g: usize, s: usize, m: JfetModel },
    /// Op-amp: output through `rout` toward a limited open-loop gain stage.
    OpAmp { p: usize, n: usize, o: usize, gain: f64, lo: f64, hi: f64, rout: f64 },
    /// OTA (CA3080): output current = iabc * tanh((v+ - v-) / 2Vt), iabc from a slot.
    Ota { p: usize, n: usize, o: usize, slot: usize },
    /// Linear op-amp (an ideal amplifier of `gain` behind `rout`): for stages that never clip, it
    /// costs nothing at run time.
    LinAmp { p: usize, n: usize, o: usize, gain: f64, rout: f64 },
    /// OTA (CA3080 / LM13700 half) with its bias pin in the circuit: the current into `abc`
    /// (a diode-connected junction to `vee`) sets the transconductance.
    OtaPin { p: usize, n: usize, o: usize, abc: usize, vee: usize },
}

/// A circuit description. Add parts with the builder methods, then `build`.
#[derive(Clone, Debug, Default)]
pub struct Circuit {
    nodes: usize,
    els: Vec<El>,
    caps: usize,
    inds: usize,
    srcs: usize,
    ctls: usize,
    vars: usize,
    outs: Vec<usize>,
}

impl Circuit {
    pub fn new() -> Self {
        Self::default()
    }
    fn n(&mut self, ns: &[usize]) {
        for &x in ns {
            self.nodes = self.nodes.max(x);
        }
    }
    pub fn r(mut self, a: usize, b: usize, r: f64) -> Self {
        self.n(&[a, b]);
        self.els.push(El::R { a, b, g: 1.0 / r.max(1e-3) });
        self
    }
    pub fn c(mut self, a: usize, b: usize, c: f64) -> Self {
        self.n(&[a, b]);
        self.els.push(El::C { a, b, c, k: self.caps });
        self.caps += 1;
        self
    }
    /// Inductor (a guitar pickup's coil).
    pub fn l(mut self, a: usize, b: usize, l: f64) -> Self {
        self.n(&[a, b]);
        self.els.push(El::L { a, b, l, k: self.inds });
        self.inds += 1;
        self
    }
    /// Three-terminal pot (`a` CCW end, `w` wiper, `b` CW end), on control `ctl` (0..1).
    pub fn pot(mut self, a: usize, w: usize, b: usize, r: f64, taper: Taper, ctl: usize) -> Self {
        self.n(&[a, w, b]);
        self.els.push(El::Pot { a, w, b, r, taper, ctl });
        self.ctls = self.ctls.max(ctl + 1);
        self
    }
    /// Variable resistor: `r * taper(pos)` (never below `min`).
    pub fn rheo(mut self, a: usize, b: usize, r: f64, taper: Taper, ctl: usize, min: f64) -> Self {
        self.n(&[a, b]);
        self.els.push(El::Rheo { a, b, r, taper, ctl, min });
        self.ctls = self.ctls.max(ctl + 1);
        self
    }
    /// A resistance set from outside every sample (`set_var`).
    pub fn var(mut self, a: usize, b: usize) -> Self {
        self.n(&[a, b]);
        self.els.push(El::Var { a, b, slot: self.vars });
        self.vars += 1;
        self
    }
    /// The signal input: a voltage source with series resistance (the next input index).
    pub fn input(mut self, node: usize, rs: f64) -> Self {
        self.n(&[node]);
        self.els.push(El::Src { node, rs: rs.max(1e-3), src: self.srcs });
        self.srcs += 1;
        self
    }
    /// A DC supply rail with its source resistance.
    pub fn dc(mut self, node: usize, v: f64, rs: f64) -> Self {
        self.n(&[node]);
        self.els.push(El::Dc { node, rs: rs.max(1e-3), v });
        self
    }
    pub fn diode(mut self, a: usize, k: usize, m: DiodeModel) -> Self {
        self.n(&[a, k]);
        self.els.push(El::Diode { a, k, m });
        self
    }
    pub fn npn(mut self, c: usize, b: usize, e: usize, m: BjtModel) -> Self {
        self.n(&[c, b, e]);
        self.els.push(El::Bjt { c, b, e, pnp: false, m });
        self
    }
    pub fn pnp(mut self, c: usize, b: usize, e: usize, m: BjtModel) -> Self {
        self.n(&[c, b, e]);
        self.els.push(El::Bjt { c, b, e, pnp: true, m });
        self
    }
    pub fn jfet(mut self, d: usize, g: usize, s: usize, m: JfetModel) -> Self {
        self.n(&[d, g, s]);
        self.els.push(El::Jfet { d, g, s, m });
        self
    }
    /// Op-amp with open-loop gain `gain`, output swinging between `lo` and `hi`.
    pub fn opamp(mut self, p: usize, n: usize, o: usize, gain: f64, lo: f64, hi: f64) -> Self {
        self.n(&[p, n, o]);
        self.els.push(El::OpAmp { p, n, o, gain, lo, hi, rout: 50.0 });
        self
    }
    /// OTA whose bias current comes from `set_var` slot.
    pub fn ota(mut self, p: usize, n: usize, o: usize) -> Self {
        self.n(&[p, n, o]);
        self.els.push(El::Ota { p, n, o, slot: self.vars });
        self.vars += 1;
        self
    }
    /// Op-amp that stays linear (no rails): open-loop `gain` (TL06x-class: 2e5) behind 75 ohms.
    pub fn linamp(mut self, p: usize, n: usize, o: usize, gain: f64) -> Self {
        self.n(&[p, n, o]);
        self.els.push(El::LinAmp { p, n, o, gain, rout: 75.0 });
        self
    }
    /// OTA whose bias current comes from the circuit: whatever flows into `abc` (which sits one
    /// junction drop above `vee`) is the amplifier bias, and the output current is that times
    /// tanh((v+ - v-) / 2Vt).
    pub fn ota_pin(mut self, p: usize, n: usize, o: usize, abc: usize, vee: usize) -> Self {
        self.n(&[p, n, o, abc, vee]);
        self.els.push(El::OtaPin { p, n, o, abc, vee });
        self
    }
    pub fn output(mut self, node: usize) -> Self {
        self.n(&[node]);
        self.outs.push(node);
        self
    }
    pub fn build(self, fs: f64) -> Solver {
        Solver::new(self, fs)
    }
}

/// exp(x) to about 1e-10 relative: 2^(x log2 e), split into an integer power and a polynomial.
#[inline]
fn fexp(x: f64) -> f64 {
    if x < -700.0 {
        return 0.0;
    }
    let t = x * std::f64::consts::LOG2_E;
    let ip = t.round();
    let f = (t - ip) * std::f64::consts::LN_2;
    // e^f for |f| <= ln2 / 2: Taylor to f^9 / 9!
    let p = 1.0 + f * (1.0 + f * (1.0 / 2.0 + f * (1.0 / 6.0 + f * (1.0 / 24.0 + f * (1.0 / 120.0 + f * (1.0 / 720.0 + f * (1.0 / 5040.0 + f * (1.0 / 40320.0 + f * (1.0 / 362880.0)))))))));
    let bits = ((ip as i64 + 1023) as u64) << 52;
    p * f64::from_bits(bits)
}

#[inline]
fn lexp(x: f64) -> (f64, f64) {
    // exp with a linear continuation past 40 (keeps Newton finite)
    if x < 40.0 {
        let e = fexp(x);
        (e, e)
    } else {
        let e = fexp(40.0);
        (e * (1.0 + x - 40.0), e)
    }
}

#[derive(Clone, Copy)]
struct Dev {
    el: usize,
    /// a second diode across the first, the other way round (anti-parallel pair)
    pair: Option<usize>,
    p: usize,
    i: usize,
    np: usize,
    ni: usize,
}

/// Square-law JFET for vds >= 0: (id, d id/d vgs, d id/d vds).
fn jfet_iv(m: JfetModel, vgs: f64, vds: f64) -> (f64, f64, f64) {
    let vov = vgs - m.vp;
    let beta = m.idss / (m.vp * m.vp);
    if vov <= 0.0 {
        (0.0, 0.0, 1e-12)
    } else if vds < vov {
        let l = 1.0 + m.lambda * vds;
        let core = 2.0 * vov * vds - vds * vds;
        (beta * core * l, beta * 2.0 * vds * l, beta * (2.0 * vov - 2.0 * vds) * l + beta * core * m.lambda)
    } else {
        let l = 1.0 + m.lambda * vds;
        (beta * vov * vov * l, 2.0 * beta * vov * l, beta * vov * vov * m.lambda)
    }
}

/// From this many device ports on, reusing the factored Jacobian beats refactoring every step.
const CHORD_PORTS: usize = 12;

/// Saturation current of an OTA's bias-pin junction.
const OTA_IS: f64 = 1.0e-14;

/// The most device ports (junction voltages and the like) one circuit may have.
const MP: usize = 24;

/// A built circuit, run one sample at a time.
///
/// Internally: the linear part (resistors, pots, the capacitors' and inductors'
/// trapezoidal companions, the sources) is folded into matrices once per control
/// change, so a sample costs a few small matrix-vector products over the
/// reactive states and the device ports, plus Newton over the device ports only.
#[derive(Clone)]
pub struct Solver {
    c: Circuit,
    n: usize,
    fs: f64,
    ctl: Vec<f64>,
    var: Vec<f64>,
    lin_dirty: bool,
    lin_dc: bool,
    supply: f64,
    // reactive states: (a, b, g, is_inductor)
    states: Vec<(usize, usize, f64, bool)>,
    // inputs: sources (node, g, index), then one column for all DC supplies
    srcs: Vec<(usize, f64, usize)>,
    dcs: Vec<(usize, f64, f64)>,
    nx: usize,
    x: Vec<f64>,
    // devices
    devs: Vec<Dev>,
    np: usize,
    ni: usize,
    pmap: Vec<Vec<(usize, f64)>>,
    imap: Vec<Vec<(usize, f64)>>,
    jlim: Vec<Option<(f64, f64, f64)>>,
    /// per-port convergence scale (1/volts): a port whose voltage is amplified (an op-amp's inputs) converges tighter
    pscale: [f64; MP],
    p: [f64; MP],
    p0: [f64; MP],
    p_prev: [f64; MP],
    p_pp: [f64; MP],
    icur: [f64; MP],
    dmat: Box<[[f64; MP]; MP]>,
    // folded matrices
    glin: Vec<f64>,
    ginv: Vec<f64>,
    work: Vec<f64>,
    wx: Vec<f64>,
    mmat: Vec<f64>,
    pmat: Vec<f64>,
    kmat: Box<[[f64; MP]; MP]>,
    bx: Vec<f64>,
    bi: Vec<f64>,
    vb: Vec<f64>,
    jred: Box<[[f64; MP]; MP]>,
    fres: [f64; MP],
    // the factored Jacobian, reused across iterations and samples while it keeps Newton contracting
    lu: Box<[[f64; MP]; MP]>,
    piv: [usize; MP],
    lu_ok: bool,
    /// Jacobian factorizations so far (for tests and tuning).
    pub factors: usize,
    /// Reuse the Jacobian while it keeps contracting (false: plain Newton, for checking).
    pub chord: bool,
    /// Newton iterations used on the last sample (for tests and tuning).
    pub iters: usize,
    pub max_iter: usize,
    /// How many times the linear part was refactored (for tests).
    pub refactors: usize,
    pub tol: f64,
    /// Convergence: the largest port-voltage residual (volts) accepted.
    pub ftol: f64,
}

impl Solver {
    fn new(c: Circuit, fs: f64) -> Self {
        let n = c.nodes;
        // the devices' ports: controlling voltages (p) and the currents they put into nodes (i)
        let mut devs = Vec::new();
        let (mut pmap, mut imap): (Vec<Vec<(usize, f64)>>, Vec<Vec<(usize, f64)>>) = (vec![], vec![]);
        let pair = |a: usize, b: usize| {
            let mut v = vec![];
            if a > 0 { v.push((a, 1.0)); }
            if b > 0 { v.push((b, -1.0)); }
            v
        };
        let one = |a: usize| if a > 0 { vec![(a, 1.0)] } else { vec![] };
        let mut taken = vec![false; c.els.len()];
        for (ix, el) in c.els.iter().enumerate() {
            if taken[ix] {
                continue;
            }
            let (p0, i0) = (pmap.len(), imap.len());
            let mut partner = None;
            match *el {
                El::Diode { a, k, .. } => {
                    partner = c.els.iter().enumerate().skip(ix + 1).position(|(j, e)| !taken[j] && matches!(*e, El::Diode { a: a2, k: k2, .. } if a2 == k && k2 == a)).map(|q| q + ix + 1);
                    if let Some(j) = partner {
                        taken[j] = true;
                    }
                    pmap.push(pair(a, k));
                    imap.push(pair(a, k));
                }
                El::Bjt { c: cc, b, e, .. } => {
                    pmap.push(pair(b, e));
                    pmap.push(pair(b, cc));
                    imap.push(pair(cc, e));
                    imap.push(pair(b, e));
                }
                El::Jfet { d, g, s: so, .. } => {
                    pmap.push(pair(g, so));
                    pmap.push(pair(d, so));
                    imap.push(pair(d, so));
                }
                El::OpAmp { p, n: nn, o, .. } => {
                    pmap.push(pair(p, nn));
                    pmap.push(one(o));
                    imap.push(one(o));
                }
                El::Ota { p, n: nn, o, .. } => {
                    pmap.push(pair(p, nn));
                    imap.push(one(o));
                }
                El::OtaPin { p, n: nn, o, abc, vee } => {
                    pmap.push(pair(p, nn));
                    pmap.push(pair(abc, vee));
                    imap.push(one(o));
                    imap.push(pair(abc, vee));
                }
                _ => continue,
            }
            devs.push(Dev { el: ix, pair: partner, p: p0, i: i0, np: pmap.len() - p0, ni: imap.len() - i0 });
        }
        let (np, ni) = (pmap.len(), imap.len());
        assert!(np <= MP, "circuit has {np} device ports; the solver takes up to {MP}");
        let mut jlim: Vec<Option<(f64, f64, f64)>> = vec![None; np];
        for d in &devs {
            let (sg, is, nn) = match c.els[d.el] {
                // sign 0: limit both ways (anti-parallel pair)
                El::Diode { m, .. } => (if d.pair.is_some() { 0.0 } else { 1.0 }, m.is, m.n),
                El::Bjt { pnp, m, .. } => (if pnp { -1.0 } else { 1.0 }, m.is, m.n),
                El::OtaPin { .. } => {
                    let vcrit = VT * (VT / (std::f64::consts::SQRT_2 * OTA_IS)).ln();
                    jlim[d.p + 1] = Some((1.0, VT, vcrit));
                    continue;
                }
                _ => continue,
            };
            let nvt = nn * VT;
            let vcrit = nvt * (nvt / (std::f64::consts::SQRT_2 * is)).ln();
            for q in 0..d.np {
                jlim[d.p + q] = Some((sg, nvt, vcrit));
            }
        }
        let states: Vec<(usize, usize, f64, bool)> = c
            .els
            .iter()
            .filter_map(|e| match *e {
                El::C { a, b, c, .. } => Some((a, b, 2.0 * fs * c, false)),
                El::L { a, b, l, .. } => Some((a, b, 1.0 / (2.0 * fs * l), true)),
                _ => None,
            })
            .collect();
        let srcs: Vec<(usize, f64, usize)> = c.els.iter().filter_map(|e| if let El::Src { node, rs, src } = *e { Some((node, 1.0 / rs, src)) } else { None }).collect();
        let dcs: Vec<(usize, f64, f64)> = c.els.iter().filter_map(|e| if let El::Dc { node, rs, v } = *e { Some((node, 1.0 / rs, v)) } else { None }).collect();
        let pscale = [1.0; MP];
        for d in &devs {
            let _ = d;
        }
        let ns = states.len();
        let nx = ns + c.srcs + 1;
        let mut s = Self {
            n,
            fs,
            ctl: vec![0.5; c.ctls],
            var: vec![1e6; c.vars],
            lin_dirty: true,
            lin_dc: false,
            supply: 1.0,
            states,
            srcs,
            dcs,
            nx,
            x: vec![0.0; nx],
            devs,
            np,
            ni,
            pmap,
            imap,
            jlim,
            pscale,
            p: [0.0; MP],
            p0: [0.0; MP],
            p_prev: [0.0; MP],
            p_pp: [0.0; MP],
            icur: [0.0; MP],
            dmat: Box::new([[0.0; MP]; MP]),
            glin: vec![0.0; n * n],
            ginv: vec![0.0; n * n],
            work: vec![0.0; n * n],
            wx: vec![0.0; n * nx],
            mmat: vec![0.0; n * ni],
            pmat: vec![0.0; np * nx],
            kmat: Box::new([[0.0; MP]; MP]),
            bx: vec![0.0; ns * nx],
            bi: vec![0.0; ns * ni],
            vb: vec![0.0; ns],
            jred: Box::new([[0.0; MP]; MP]),
            fres: [0.0; MP],
            lu: Box::new([[0.0; MP]; MP]),
            piv: [0; MP],
            lu_ok: false,
            factors: 0,
            chord: np >= CHORD_PORTS && std::env::var("PQ_NO_CHORD").is_err(),
            iters: 0,
            max_iter: 30,
            refactors: 0,
            tol: 1e-4,
            ftol: 1e-6,
            c,
        };
        s.dc_op();
        s.max_iter = 20;
        s
    }

    pub fn set_ctl(&mut self, ctl: usize, pos: f64) {
        let p = pos.clamp(0.0, 1.0);
        if p != self.ctl[ctl] {
            self.ctl[ctl] = p;
            self.lin_dirty = true;
        }
    }
    /// Set an externally controlled resistance (ohms) or OTA bias current (amps).
    pub fn set_var(&mut self, slot: usize, x: f64) {
        if x != self.var[slot] {
            self.var[slot] = x;
            // OTA bias currents don't touch the linear part
            if self.c.els.iter().any(|e| matches!(*e, El::Var { slot: s2, .. } if s2 == slot)) {
                self.lin_dirty = true;
            }
        }
    }
    /// A node's voltage on the last sample.
    pub fn node(&self, node: usize) -> f64 {
        if node == 0 {
            return 0.0;
        }
        let (nx, k) = (self.nx, self.ni);
        let r = node - 1;
        let mut acc = 0.0;
        for (w, x) in self.wx[r * nx..(r + 1) * nx].iter().zip(&self.x) {
            acc += w * x;
        }
        for (m, i) in self.mmat[r * k..(r + 1) * k].iter().zip(&self.icur[..k]) {
            acc -= m * i;
        }
        acc
    }
    /// The device port voltages on the last sample (for tests).
    pub fn ports(&self) -> Vec<f64> {
        self.p[..self.np].to_vec()
    }
    /// The largest residual of the last sample's equations at port voltages `p` (for tests).
    pub fn residual_at(&mut self, p: &[f64]) -> f64 {
        let saved = self.p;
        self.p[..p.len()].copy_from_slice(p);
        self.eval_devices();
        let mut res = 0.0f64;
        for r in 0..self.np {
            let mut f = self.p[r] - self.p0[r];
            for l in 0..self.ni {
                f += self.kmat[r][l] * self.icur[l];
            }
            res = res.max(f.abs());
        }
        self.p = saved;
        self.eval_devices();
        res
    }
    /// (nodes, device ports, reactive states): for tests and tuning.
    pub fn size(&self) -> (usize, usize, usize) {
        (self.n, self.np, self.states.len())
    }
    pub fn out(&self, k: usize) -> f64 {
        self.node(self.c.outs[k])
    }

    /// Find the operating point: capacitors open, the supplies brought up in steps.
    pub fn dc_op(&mut self) {
        let ns = self.states.len();
        self.x.iter_mut().for_each(|v| *v = 0.0);
        self.p.iter_mut().for_each(|x| *x = 0.0);
        let inputs = vec![0.0; self.c.srcs];
        let saved = self.max_iter;
        self.max_iter = 60;
        for step in 1..=20 {
            self.newton(&inputs, step as f64 / 20.0, true);
        }
        self.max_iter = saved;
        // the reactive states at the operating point: capacitors carry no current, inductors the DC
        let mut st = vec![0.0; ns];
        for (j, &(a, b, g, ind)) in self.states.iter().enumerate() {
            let vb = self.node(a) - self.node(b);
            let _ = g;
            st[j] = if ind { 1.0e3 * vb } else { -g * vb };
        }
        self.x[..ns].copy_from_slice(&st);
        // and the matrices for running (so node() reads the operating point too)
        self.refactor(false);
        self.p_prev.copy_from_slice(&self.p);
        self.p_pp.copy_from_slice(&self.p);
    }

    /// Back to the operating point.
    pub fn reset(&mut self) {
        self.dc_op();
    }

    /// One sample: `inputs` are the source voltages; returns the first output.
    pub fn step(&mut self, inputs: &[f64]) -> f64 {
        // start Newton from a linear prediction of the device voltages
        for j in 0..self.np {
            self.p[j] = 2.0 * self.p_prev[j] - self.p_pp[j];
        }
        self.newton(inputs, 1.0, false);
        std::mem::swap(&mut self.p_pp, &mut self.p_prev);
        self.p_prev.copy_from_slice(&self.p);
        // branch voltages of the reactive parts, then their trapezoidal states for the next sample
        let (ns, nx, k) = (self.states.len(), self.nx, self.ni);
        for j in 0..ns {
            let mut acc = 0.0;
            for (b, x) in self.bx[j * nx..(j + 1) * nx].iter().zip(&self.x) {
                acc += b * x;
            }
            for (b, i) in self.bi[j * k..(j + 1) * k].iter().zip(&self.icur[..k]) {
                acc -= b * i;
            }
            self.vb[j] = acc;
        }
        let out = if self.c.outs.is_empty() { 0.0 } else { self.node(self.c.outs[0]) };
        for (j, &(_, _, g, ind)) in self.states.iter().enumerate() {
            let s = self.x[j];
            self.x[j] = if ind { s + 2.0 * g * self.vb[j] } else { -(2.0 * g * self.vb[j] + s) };
        }
        out
    }

    fn newton(&mut self, inputs: &[f64], supply: f64, dc: bool) {
        let (m, nx) = (self.np, self.nx);
        let ns = self.states.len();
        self.supply = supply;
        if self.lin_dirty || self.lin_dc != dc {
            self.refactor(dc);
        }
        // the input part of x: sources, then the supplies
        for (j, &(_, _, src)) in self.srcs.iter().enumerate() {
            let _ = j;
            self.x[ns + src] = if dc { 0.0 } else { inputs.get(src).copied().unwrap_or(0.0) };
        }
        self.x[nx - 1] = supply;
        if dc {
            self.x[..ns].iter_mut().for_each(|v| *v = 0.0);
        }
        // p0: the device voltages with no device current
        for j in 0..m {
            let mut acc = 0.0;
            for (pm, x) in self.pmat[j * nx..(j + 1) * nx].iter().zip(&self.x) {
                acc += pm * x;
            }
            self.p0[j] = acc;
        }
        self.iters = 0;
        if m == 0 {
            return;
        }
        macro_rules! sized {
            ($($n:literal)*) => {
                match m { $($n => self.iterate::<$n>(),)* _ => self.iterate::<MP>() }
            };
        }
        let start = self.p;
        let mut converged = sized!(1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23);
        if !converged && self.chord {
            // the reused Jacobian lost its way: plain Newton from the same start, with more room
            self.p = start;
            self.chord = false;
            let mi = self.max_iter;
            self.max_iter = 2 * mi;
            converged = sized!(1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23);
            self.max_iter = mi;
            self.chord = true;
        }
        if !converged {
            // didn't settle: keep the last good voltages rather than a wild guess
            if !dc {
                self.p.copy_from_slice(&self.p_prev);
            }
            self.eval_devices();
        }
    }

    /// J = I + K D at the present device conductances, factored into `lu`.
    fn factor<const M: usize>(&mut self) -> bool {
        // each current depends on at most two neighbouring ports: (first port, d/dfirst, d/dsecond)
        let mut dl = [(0usize, 0.0f64, 0.0f64); MP];
        for d in &self.devs {
            for l in d.i..d.i + d.ni {
                let row = &self.dmat[l];
                dl[l] = (d.p, row[d.p], if d.np > 1 { row[d.p + 1] } else { 0.0 });
            }
        }
        let k = self.ni;
        for r in 0..M {
            let krow: &[f64; MP] = &self.kmat[r];
            let jrow: &mut [f64; MP] = &mut self.lu[r];
            *jrow = [0.0; MP];
            for (l, &(c, d0, d1)) in dl[..k].iter().enumerate() {
                let kv = krow[l];
                if c + 1 < MP {
                    jrow[c] += kv * d0;
                    jrow[c + 1] += kv * d1;
                }
            }
            jrow[r] += 1.0;
        }
        self.factors += 1;
        self.lu_ok = lu_factor::<M>(&mut self.lu, &mut self.piv);
        self.lu_ok
    }

    /// Newton over the device ports, sized at compile time (ports past `np` are padding and stay at zero).
    /// Convergence is judged on the true residual at the present voltages, so the factored Jacobian
    /// can be reused (chord steps) for as long as it keeps the residual falling fast; it is
    /// refactored the moment it doesn't. The answer is the same either way, only the work differs.
    fn iterate<const M: usize>(&mut self) -> bool {
        let m = self.np;
        let mut prev = f64::INFINITY;
        // iterations since the Jacobian was factored (large: it came from an earlier sample)
        let mut age = usize::MAX / 2;
        for it in 0..self.max_iter {
            self.eval_devices();
            // F = p - p0 + K i
            let mut res = 0.0f64;
            for r in 0..M {
                let krow: &[f64; MP] = &self.kmat[r];
                let mut f = self.p[r] - self.p0[r];
                for l in 0..M {
                    f += krow[l] * self.icur[l];
                }
                self.fres[r] = -f;
                res = res.max(f.abs() * self.pscale[r]);
            }
            self.iters = it;
            if res < self.ftol {
                return true;
            }
            if !res.is_finite() {
                return false;
            }
            if !self.chord || !self.lu_ok || (age > 0 && res > 0.25 * prev) {
                if !self.factor::<M>() {
                    return false;
                }
                age = 0;
            }
            lu_solve::<M>(&self.lu, &self.piv, &mut self.fres);
            let mut big = 0.0f64;
            for j in 0..M {
                big = big.max(self.fres[j].abs());
            }
            let mut limited = false;
            // SPICE's pnjlim on every junction: forward steps past the critical voltage are taken logarithmically
            for j in 0..m {
                let old = self.p[j];
                let mut new = old + self.fres[j];
                if let Some((sg, nvt, vcrit)) = self.jlim[j] {
                    let sg = if sg == 0.0 { if new >= 0.0 { 1.0 } else { -1.0 } } else { sg };
                    let (vo, vn) = (sg * old, sg * new);
                    if vn > vcrit && (vn - vo).abs() > 2.0 * nvt {
                        let lim = if vo > 0.0 {
                            let arg = 1.0 + (vn - vo) / nvt;
                            if arg > 0.0 { vo + nvt * arg.ln() } else { vcrit }
                        } else {
                            nvt * (vn / nvt).ln()
                        };
                        new = sg * lim;
                        limited = true;
                    }
                }
                self.fres[j] = new - old;
                self.p[j] = new;
            }
            if age == 0 && big < self.tol && !limited {
                // a Newton step this small (Jacobian fresh at this point) leaves a far smaller
                // error: take it, and bring the currents to the new voltages to first order
                for d in &self.devs {
                    for l in d.i..d.i + d.ni {
                        let mut acc = 0.0;
                        for c in d.p..d.p + d.np {
                            acc += self.dmat[l][c] * self.fres[c];
                        }
                        self.icur[l] += acc;
                    }
                }
                self.iters = it + 1;
                return true;
            }
            age += 1;
            prev = res;
        }
        false
    }

    /// Build G, invert it, and fold everything through it.
    fn refactor(&mut self, dc: bool) {
        self.refactors += 1;
        let n = self.n;
        let (m, k, nx) = (self.np, self.ni, self.nx);
        let ns = self.states.len();
        // G
        self.glin.iter_mut().for_each(|x| *x = 0.0);
        {
            let glin = &mut self.glin;
            let mut g2 = |a: usize, b: usize, g: f64| {
                if a > 0 { glin[(a - 1) * n + a - 1] += g; }
                if b > 0 { glin[(b - 1) * n + b - 1] += g; }
                if a > 0 && b > 0 {
                    glin[(a - 1) * n + b - 1] -= g;
                    glin[(b - 1) * n + a - 1] -= g;
                }
            };
            for el in &self.c.els {
                match *el {
                    El::R { a, b, g } => g2(a, b, g),
                    El::C { a, b, c, .. } => g2(a, b, if dc { 1e-12 } else { 2.0 * self.fs * c }),
                    El::L { a, b, l, .. } => g2(a, b, if dc { 1.0e3 } else { 1.0 / (2.0 * self.fs * l) }),
                    El::Pot { a, w, b, r, taper, ctl } => {
                        // rotation 1.0 puts the wiper at `b`
                        let p = taper.apply(self.ctl[ctl]);
                        g2(a, w, 1.0 / (r * p).max(2.0));
                        g2(w, b, 1.0 / (r * (1.0 - p)).max(2.0));
                    }
                    El::Rheo { a, b, r, taper, ctl, min } => g2(a, b, 1.0 / (r * taper.apply(self.ctl[ctl])).max(min).max(1.0)),
                    El::Var { a, b, slot } => g2(a, b, 1.0 / self.var[slot].max(1.0)),

                    El::Src { node, rs, .. } | El::Dc { node, rs, .. } => g2(node, 0, 1.0 / rs),
                    _ => {}
                }
            }
        }
        for el in &self.c.els {
            if let El::LinAmp { p, n: nn, o, gain, rout } = *el {
                // Norton: gain (v+ - v-) / rout into the output, rout to ground
                self.glin[(o - 1) * n + o - 1] += 1.0 / rout;
                if p > 0 {
                    self.glin[(o - 1) * n + p - 1] -= gain / rout;
                }
                if nn > 0 {
                    self.glin[(o - 1) * n + nn - 1] += gain / rout;
                }
            }
        }
        // gmin: 1 G-ohm from every node to ground, so nodes reached only through devices stay solvable
        for i in 0..n {
            self.glin[i * n + i] += 1e-9;
        }
        invert(&self.glin, &mut self.work, &mut self.ginv, n);
        // W = -G^-1 H : columns are the injections of each state and input
        let col = |j: usize| -> Vec<(usize, f64)> {
            if j < ns {
                let (a, b, _, _) = self.states[j];
                let mut v = vec![];
                if a > 0 { v.push((a, 1.0)); }
                if b > 0 { v.push((b, -1.0)); }
                v
            } else if j < nx - 1 {
                self.srcs.iter().filter(|s| s.2 == j - ns).map(|&(node, g, _)| (node, -g)).collect()
            } else {
                self.dcs.iter().map(|&(node, g, v)| (node, -g * v)).collect()
            }
        };
        for j in 0..nx {
            let inj = col(j);
            for r in 0..n {
                let mut acc = 0.0;
                for &(node, c) in &inj {
                    acc -= c * self.ginv[r * n + node - 1];
                }
                self.wx[r * nx + j] = acc;
            }
        }
        // M = G^-1 N
        for r in 0..n {
            for l in 0..k {
                let mut acc = 0.0;
                for &(node, c) in &self.imap[l] {
                    acc += c * self.ginv[r * n + node - 1];
                }
                self.mmat[r * k + l] = acc;
            }
        }
        // P = E W, K = E M
        for j in 0..m {
            for q in 0..nx {
                let mut acc = 0.0;
                for &(node, c) in &self.pmap[j] {
                    acc += c * self.wx[(node - 1) * nx + q];
                }
                self.pmat[j * nx + q] = acc;
            }
            for l in 0..k {
                let mut acc = 0.0;
                for &(node, c) in &self.pmap[j] {
                    acc += c * self.mmat[(node - 1) * k + l];
                }
                self.kmat[j][l] = acc;
            }
        }
        // the reactive branches: Bx = Eb W, Bi = Eb M
        for (j, &(a, b, _, _)) in self.states.iter().enumerate() {
            for q in 0..nx {
                let va = if a > 0 { self.wx[(a - 1) * nx + q] } else { 0.0 };
                let vbb = if b > 0 { self.wx[(b - 1) * nx + q] } else { 0.0 };
                self.bx[j * nx + q] = va - vbb;
            }
            for l in 0..k {
                let va = if a > 0 { self.mmat[(a - 1) * k + l] } else { 0.0 };
                let vbb = if b > 0 { self.mmat[(b - 1) * k + l] } else { 0.0 };
                self.bi[j * k + l] = va - vbb;
            }
        }
        self.lin_dirty = false;
        self.lin_dc = dc;
        self.lu_ok = false;
    }

    /// Device currents `icur` and their derivatives `dmat` (k x m) at the port voltages `p`.
    fn eval_devices(&mut self) {
        // every device writes its whole block of D, so no clearing is needed
        for d in &self.devs {
            let (po, io) = (d.p, d.i);
            let p = &self.p;
            let dm = &mut self.dmat;
            let ic = &mut self.icur;
            match self.c.els[d.el] {
                El::Diode { m: md, .. } => {
                    let nvt = md.n * VT;
                    let (e, de) = lexp(p[po] / nvt);
                    let mut i = md.is * (e - 1.0) + 1e-12 * p[po];
                    let mut g = md.is * de / nvt + 1e-12;
                    if let Some(j) = d.pair {
                        if let El::Diode { m: m2, .. } = self.c.els[j] {
                            let nvt2 = m2.n * VT;
                            let (e2, de2) = lexp(-p[po] / nvt2);
                            i -= m2.is * (e2 - 1.0);
                            g += m2.is * de2 / nvt2;
                        }
                    }
                    ic[io] = i;
                    dm[io][po] = g;
                }
                El::Bjt { pnp, m: md, .. } => {
                    let s = if pnp { -1.0 } else { 1.0 };
                    let nvt = md.n * VT;
                    let vbe = s * p[po];
                    let vbc = s * p[po + 1];
                    let (ef, def) = lexp(vbe / nvt);
                    let (er, der) = lexp(vbc / nvt);
                    let vce = vbe - vbc;
                    let leak = if vce > 0.0 { md.leak * vce / (vce + 0.1) } else { 0.0 };
                    let dleak = if vce > 0.0 { md.leak * 0.1 / (vce + 0.1).powi(2) } else { 0.0 };
                    let icd = md.is * (ef - er) - md.is / md.br * (er - 1.0) + leak;
                    let ibd = md.is / md.bf * (ef - 1.0) + md.is / md.br * (er - 1.0);
                    let gf = md.is * def / nvt;
                    let gr = md.is * der / nvt;
                    // node-frame currents I = s * i; dI/dp = s * di/dv * s = di/dv
                    ic[io] = s * icd;
                    ic[io + 1] = s * ibd;
                    dm[io][po] = gf + dleak;
                    dm[io][po + 1] = -gr - gr / md.br - dleak;
                    dm[io + 1][po] = gf / md.bf;
                    dm[io + 1][po + 1] = gr / md.br;
                }
                El::Jfet { m: md, .. } => {
                    let (vgs, vds) = (p[po], p[po + 1]);
                    // symmetric channel: for vds < 0 the source and drain swap
                    let (id, gm, gds) = if vds >= 0.0 {
                        jfet_iv(md, vgs, vds)
                    } else {
                        let (i2, gm2, gds2) = jfet_iv(md, vgs - vds, -vds);
                        // id = -i2(vgd, -vds); d/dvgs = -gm2; d/dvds = gm2 + gds2
                        (-i2, -gm2, gm2 + gds2)
                    };
                    ic[io] = id;
                    dm[io][po] = gm;
                    dm[io][po + 1] = gds.max(1e-12);
                }
                El::OpAmp { gain, lo, hi, rout, .. } => {
                    let sup = self.supply.max(1e-3);
                    let mid = 0.5 * (hi + lo) * sup;
                    let half = 0.5 * (hi - lo) * sup;
                    let t = (gain * p[po] / half).tanh();
                    let vo = mid + half * t;
                    let dvo = gain * (1.0 - t * t);
                    ic[io] = (p[po + 1] - vo) / rout;
                    dm[io][po] = -dvo / rout;
                    dm[io][po + 1] = 1.0 / rout;
                }
                El::Ota { slot, .. } => {
                    let iabc = self.var[slot].max(0.0);
                    let t = (p[po] / (2.0 * VT)).tanh();
                    ic[io] = -iabc * t;
                    dm[io][po] = -iabc * (1.0 - t * t) / (2.0 * VT);
                }
                El::OtaPin { .. } => {
                    let (e, de) = lexp(p[po + 1] / VT);
                    let iabc = OTA_IS * (e - 1.0) + 1e-12 * p[po + 1];
                    let giabc = OTA_IS * de / VT + 1e-12;
                    let t = (p[po] / (2.0 * VT)).tanh();
                    let ib = iabc.max(0.0);
                    ic[io] = -ib * t;
                    dm[io][po] = -ib * (1.0 - t * t) / (2.0 * VT);
                    dm[io][po + 1] = if iabc > 0.0 { -t * giabc } else { 0.0 };
                    ic[io + 1] = iabc;
                    dm[io + 1][po] = 0.0;
                    dm[io + 1][po + 1] = giabc;
                }
                _ => {}
            }
        }
    }

}

/// Gauss-Jordan inverse of `m` (n x n) into `inv`.
fn invert(m: &[f64], a: &mut [f64], inv: &mut [f64], n: usize) {
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

/// Gaussian elimination with partial pivoting; solution in `b`.
/// LU-factor `a` in place with partial pivoting (row swaps recorded in `piv`).
fn lu_factor<const M: usize>(a: &mut [[f64; MP]; MP], piv: &mut [usize; MP]) -> bool {
    for col in 0..M {
        let mut p = col;
        let mut best = a[col][col].abs();
        for r in col + 1..M {
            let x = a[r][col].abs();
            if x > best {
                best = x;
                p = r;
            }
        }
        if !(best > 1e-30) {
            return false;
        }
        piv[col] = p;
        if p != col {
            a.swap(col, p);
        }
        let (top, rest) = a.split_at_mut(col + 1);
        let prow = &top[col];
        let inv = 1.0 / prow[col];
        for row in rest[..M - col - 1].iter_mut() {
            let f = row[col] * inv;
            row[col] = f;
            for (x, q) in row[col + 1..M].iter_mut().zip(&prow[col + 1..M]) {
                *x -= f * q;
            }
        }
    }
    true
}

/// Solve with factors from [`lu_factor`], in place.
fn lu_solve<const M: usize>(a: &[[f64; MP]; MP], piv: &[usize; MP], b: &mut [f64; MP]) {
    for col in 0..M {
        b.swap(col, piv[col]);
    }
    for r in 1..M {
        let mut acc = b[r];
        for c in 0..r {
            acc -= a[r][c] * b[c];
        }
        b[r] = acc;
    }
    for r in (0..M).rev() {
        let mut acc = b[r];
        for c in r + 1..M {
            acc -= a[r][c] * b[c];
        }
        b[r] = acc / a[r][r];
    }
}

/// Solve `a x = b` in place (x into b) over the first `M` rows and columns.
fn solve_fixed<const M: usize>(a: &mut [[f64; MP]; MP], b: &mut [f64; MP]) -> bool {
    for col in 0..M {
        let mut piv = col;
        let mut best = a[col][col].abs();
        for r in col + 1..M {
            let x = a[r][col].abs();
            if x > best {
                best = x;
                piv = r;
            }
        }
        if best < 1e-30 {
            return false;
        }
        if piv != col {
            a.swap(col, piv);
            b.swap(col, piv);
        }
        let (top, rest) = a.split_at_mut(col + 1);
        let prow = &top[col];
        let inv = 1.0 / prow[col];
        let bc = b[col];
        for (ri, row) in rest[..M - col - 1].iter_mut().enumerate() {
            let f = row[col] * inv;
            for (x, p) in row[col + 1..M].iter_mut().zip(&prow[col + 1..M]) {
                *x -= f * p;
            }
            b[col + 1 + ri] -= f * bc;
        }
    }
    for col in (0..M).rev() {
        let mut acc = b[col];
        for c in col + 1..M {
            acc -= a[col][c] * b[c];
        }
        b[col] = acc / a[col][col];
    }
    true
}

#[allow(dead_code)]
fn solve(a: &mut [f64], b: &mut [f64], n: usize) -> bool {
    let a = &mut a[..n * n];
    let b = &mut b[..n];
    for col in 0..n {
        let mut piv = col;
        let mut best = a[col * n + col].abs();
        for r in col + 1..n {
            let x = a[r * n + col].abs();
            if x > best {
                best = x;
                piv = r;
            }
        }
        if best < 1e-30 {
            return false;
        }
        if piv != col {
            for k in col..n {
                a.swap(col * n + k, piv * n + k);
            }
            b.swap(col, piv);
        }
        let (top, rest) = a.split_at_mut((col + 1) * n);
        let prow = &top[col * n + col..col * n + n];
        let inv = 1.0 / prow[0];
        let bc = b[col];
        for (ri, row) in rest.chunks_exact_mut(n).enumerate() {
            let f = row[col] * inv;
            if f != 0.0 {
                for (x, p) in row[col..].iter_mut().zip(prow) {
                    *x -= f * p;
                }
                b[col + 1 + ri] -= f * bc;
            }
        }
    }
    for col in (0..n).rev() {
        let row = &a[col * n..col * n + n];
        let mut acc = b[col];
        for (x, bk) in row[col + 1..].iter().zip(&b[col + 1..]) {
            acc -= x * bk;
        }
        b[col] = acc / row[col];
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lu_matches_solve() {
        let mut a = [[0.0; MP]; MP];
        let mut seed = 12345u64;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        for r in 0..5 {
            for c in 0..5 {
                a[r][c] = rnd();
            }
        }
        let mut b = [0.0; MP];
        for r in 0..5 {
            b[r] = rnd();
        }
        let (mut a1, mut b1) = (a, b);
        assert!(solve_fixed::<5>(&mut a1, &mut b1));
        let (mut a2, mut b2, mut piv) = (a, b, [0usize; MP]);
        assert!(lu_factor::<5>(&mut a2, &mut piv));
        lu_solve::<5>(&a2, &piv, &mut b2);
        for r in 0..5 {
            assert!((b1[r] - b2[r]).abs() < 1e-9, "{r}: {} vs {}", b1[r], b2[r]);
        }
    }

    #[test]
    fn linamp_inverting() {
        // 10k in, 100k feedback: -10
        let mut s = Circuit::new().input(1, 1.0).r(1, 2, 10e3).r(3, 2, 100e3).linamp(0, 2, 3, 2e5).output(3).build(48000.0);
        let y = s.step(&[0.1]);
        assert!((y + 1.0).abs() < 1e-3, "y {y}");
    }

    #[test]
    fn fast_exp() {
        for k in -300..300 {
            let x = k as f64 * 0.137;
            let (a, b) = (fexp(x), x.exp());
            assert!(((a - b) / b).abs() < 1e-9, "x {x}: {a} vs {b}");
        }
    }

    #[test]
    fn divider() {
        let mut s = Circuit::new().dc(1, 9.0, 1.0).r(1, 2, 10e3).r(2, 0, 10e3).output(2).build(48000.0);
        assert!((s.step(&[]) - 4.5).abs() < 1e-3);
    }

    #[test]
    fn diode_clipper_limits() {
        // 1 V in through 2.2k into back-to-back 1N914s
        let mut s = Circuit::new().input(1, 2200.0).diode(1, 0, D1N914).diode(0, 1, D1N914).output(1).build(48000.0);
        let mut peak = 0.0f64;
        for k in 0..4800 {
            let x = 3.0 * (k as f64 * 2.0 * std::f64::consts::PI * 440.0 / 48000.0).sin();
            peak = peak.max(s.step(&[x]).abs());
        }
        assert!(peak > 0.45 && peak < 0.8, "peak {peak}");
    }

    #[test]
    fn common_emitter_bias() {
        // 9 V, 10k collector, 1k emitter, base divider 47k/10k: Vb ~1.58, Ve ~0.95, Ic ~0.95 mA, Vc ~ 9.5 - ... ~ -> about 0.4..0.6 of supply
        let mut s = Circuit::new()
            .dc(1, 9.0, 1.0)
            .r(1, 2, 47e3)
            .r(2, 0, 10e3)
            .r(1, 3, 4.7e3)
            .r(4, 0, 1e3)
            .npn(3, 2, 4, N3904)
            .output(3)
            .build(48000.0);
        let vc = s.step(&[]);
        let ve = s.node(4);
        assert!(ve > 0.75 && ve < 1.1, "ve {ve}");
        assert!(vc > 3.5 && vc < 5.8, "vc {vc}");
    }

    #[test]
    fn rl_lowpass_corner() {
        // 1 k + 1 H into 1 k: corner at 2 k / (2 pi) ~ 318 Hz; at 3.18 kHz about -20 dB
        let fs = 192000.0;
        let mut s = Circuit::new().input(1, 1000.0).l(1, 2, 1.0).r(2, 0, 1000.0).output(2).build(fs);
        let mut peak = 0.0f64;
        for k in 0..(fs as usize / 10) {
            let x = (k as f64 * 2.0 * std::f64::consts::PI * 3180.0 / fs).sin();
            let y = s.step(&[x]);
            if k > fs as usize / 20 {
                peak = peak.max(y.abs());
            }
        }
        // ideal: 0.5 * 1/sqrt(1+100) = 0.0498
        assert!((peak - 0.0498).abs() < 0.003, "peak {peak}");
    }

    #[test]
    fn opamp_inverting_gain() {
        // -10x inverting amp on +-15 V
        let mut s = Circuit::new().input(1, 1.0).r(1, 2, 10e3).r(2, 3, 100e3).opamp(0, 2, 3, 2e5, -13.5, 13.5).output(3).build(48000.0);
        let y = s.step(&[0.1]);
        assert!((y + 1.0).abs() < 0.01, "y {y}");
        let y = s.step(&[5.0]);
        assert!(y < -13.0 && y > -13.6, "y {y}");
    }
}
