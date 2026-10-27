//! Valve (vacuum tube) models and the circuit stages built from them.
//!
//! Plate/screen currents use Norman Koren's phenomenological equations with
//! parameter fits from the Guitarix DK-method amp tools (which also give the
//! grid-current terms Gco/Gcf). Every stage is solved from its real
//! component values: supply voltage, plate and cathode resistors, bypass and
//! coupling capacitors, grid leaks and stoppers.
//!
//! All inter-stage signals are AC voltages in volts (deviation from the DC
//! operating point), so coupling capacitors start settled.

/// Koren model parameters.
#[derive(Clone, Copy, Debug)]
pub struct Valve {
    pub mu: f64,
    pub ex: f64,
    pub kp: f64,
    pub kvb: f64,
    pub kg1: f64,
    /// Screen-grid constant (pentodes only).
    pub kg2: f64,
    /// Grid conduction onset (V) and coefficient (A/V^1.5).
    pub gco: f64,
    pub gcf: f64,
}

// Parameter fits from guitarix/trunk/tools/ampsim/DK/circ.py
pub const ECC83: Valve = Valve { mu: 100.0, ex: 1.4, kp: 600.0, kvb: 300.0, kg1: 1060.0, kg2: 0.0, gco: -0.2, gcf: 1e-5 };
pub const V7025: Valve = Valve { mu: 102.5, ex: 1.41, kp: 813.8, kvb: 44.9, kg1: 1598.8, kg2: 0.0, gco: -0.2, gcf: 1e-5 };
pub const ECC81: Valve = Valve { mu: 60.0, ex: 1.35, kp: 300.0, kvb: 300.0, kg1: 460.0, kg2: 0.0, gco: -0.5, gcf: 12e-5 };
pub const EL84: Valve = Valve { mu: 21.3, ex: 1.24, kp: 111.1, kvb: 17.9, kg1: 401.7, kg2: 4500.0, gco: -0.2, gcf: 1e-5 };
pub const EL34: Valve = Valve { mu: 12.3, ex: 1.17, kp: 61.1, kvb: 29.9, kg1: 353.9, kg2: 4500.0, gco: -0.2, gcf: 1e-5 };
pub const L6L6: Valve = Valve { mu: 8.7, ex: 1.35, kp: 48.0, kvb: 12.0, kg1: 1460.0, kg2: 4500.0, gco: -0.2, gcf: 1e-5 };
pub const V6V6: Valve = Valve { mu: 12.67, ex: 1.198, kp: 38.07, kvb: 30.2, kg1: 915.0, kg2: 4500.0, gco: 0.0, gcf: 12e-5 };

#[inline]
fn softplus(x: f64) -> f64 {
    if x > 30.0 {
        x
    } else {
        x.exp().ln_1p()
    }
}

impl Valve {
    /// Triode plate current (A).
    #[inline]
    pub fn ip(&self, vgk: f64, vpk: f64) -> f64 {
        if vpk <= 0.0 {
            return 0.0;
        }
        let e1 = vpk / self.kp * softplus(self.kp * (1.0 / self.mu + vgk / (self.kvb + vpk * vpk).sqrt()));
        if e1 <= 0.0 {
            0.0
        } else {
            e1.powf(self.ex) / self.kg1
        }
    }

    /// Pentode/beam-tetrode plate current (A).
    #[inline]
    pub fn ip_pentode(&self, vgk: f64, vg2k: f64, vpk: f64) -> f64 {
        if vg2k <= 1.0 || vpk <= 0.0 {
            return 0.0;
        }
        let e1 = vg2k / self.kp * softplus(self.kp * (1.0 / self.mu + vgk / vg2k));
        if e1 <= 0.0 {
            0.0
        } else {
            e1.powf(self.ex) / self.kg1 * (vpk / self.kvb).atan()
        }
    }

    /// Screen current (A).
    #[inline]
    pub fn ig2(&self, vgk: f64, vg2k: f64) -> f64 {
        let x = vg2k / self.mu + vgk;
        if x <= 0.0 || self.kg2 <= 0.0 {
            0.0
        } else {
            x.powf(self.ex) / self.kg2
        }
    }

    /// Control-grid current (A) once the grid goes positive.
    #[inline]
    pub fn ig(&self, vgk: f64) -> f64 {
        if vgk > self.gco {
            let d = vgk - self.gco;
            self.gcf * d * d.sqrt()
        } else {
            0.0
        }
    }

    #[inline]
    pub fn ig_deriv(&self, vgk: f64) -> f64 {
        if vgk > self.gco {
            1.5 * self.gcf * (vgk - self.gco).sqrt()
        } else {
            0.0
        }
    }
}

// ------------------------------------------------------------------ table --

/// Plate current vs. grid-cathode voltage along a stage's load line.
#[derive(Clone)]
pub struct LoadLineTable {
    lo: f64,
    inv_step: f64,
    ip: Vec<f64>,
    /// Slope of each segment (A/V).
    gm: Vec<f64>,
}

impl LoadLineTable {
    /// `vp_of(ip)` gives the plate-cathode voltage for a plate current.
    fn build(valve: &Valve, lo: f64, hi: f64, n: usize, vp_of: impl Fn(f64) -> f64, ip_max: f64) -> Self {
        let step = (hi - lo) / (n - 1) as f64;
        let mut ip = Vec::with_capacity(n);
        for i in 0..n {
            let vgk = lo + step * i as f64;
            // solve ip = valve.ip(vgk, vp_of(ip)) by bisection (monotonic)
            let (mut a, mut b) = (0.0, ip_max);
            for _ in 0..60 {
                let m = 0.5 * (a + b);
                if valve.ip(vgk, vp_of(m)) > m {
                    a = m;
                } else {
                    b = m;
                }
            }
            ip.push(0.5 * (a + b));
        }
        let mut gm: Vec<f64> = ip.windows(2).map(|w| (w[1] - w[0]) / step).collect();
        gm.push(*gm.last().unwrap());
        Self { lo, inv_step: 1.0 / step, ip, gm }
    }

    /// (current, d current / d vgk)
    #[inline]
    pub fn eval(&self, vgk: f64) -> (f64, f64) {
        let x = (vgk - self.lo) * self.inv_step;
        let last = self.ip.len() - 1;
        let i = if x <= 0.0 { 0 } else if x >= last as f64 { last - 1 } else { x as usize };
        let g = self.gm[i];
        let i_out = self.ip[i] + g * (x - i as f64) / self.inv_step;
        // beyond the table: hold the end slopes, never below zero current
        if x <= 0.0 {
            (self.ip[0] + g.max(0.0) * x / self.inv_step, g.max(0.0))
        } else {
            (i_out, g)
        }
    }
}

// -------------------------------------------------------------- coupling --

/// The network in front of a grid: a Thevenin source (previous plate or a
/// tone network) through a coupling capacitor into the grid leak, with the
/// grid conducting when driven positive. Grid conduction charges the
/// coupling cap and shifts the bias: the "blocking" behaviour of a pushed
/// valve amp. A grid stopper plus Miller capacitance roll off the top.
#[derive(Clone)]
pub struct GridInput {
    rs: f64,
    cc: f64,
    rg: f64,
    k: f64,
    vc: f64,
    i_prev: f64,
    miller_a: f64,
    miller_z: f64,
}

impl GridInput {
    /// `rs`: source resistance (incl. grid stopper), `cc`: coupling cap
    /// (0 = DC coupled), `rg`: grid leak, `c_miller`: effective input
    /// capacitance (Cgk + Cgp*(1+gain)).
    pub fn new(fs: f64, rs: f64, cc: f64, rg: f64, c_miller: f64) -> Self {
        let k = if cc > 0.0 { 1.0 / (2.0 * fs * cc) } else { 0.0 };
        let r_eff = 1.0 / (1.0 / rs.max(1.0) + 1.0 / rg);
        let fc = 1.0 / (2.0 * std::f64::consts::PI * r_eff * c_miller.max(1e-13));
        let miller_a = (-2.0 * std::f64::consts::PI * (fc / fs).min(0.49)).exp();
        Self { rs: rs.max(1.0), cc, rg, k, vc: 0.0, i_prev: 0.0, miller_a, miller_z: 0.0 }
    }

    pub fn reset(&mut self) {
        self.vc = 0.0;
        self.i_prev = 0.0;
        self.miller_z = 0.0;
    }

    /// Change the source resistance (e.g. a volume pot's wiper moved).
    pub fn set_source(&mut self, fs: f64, rs: f64, c_miller: f64) {
        self.rs = rs.max(1.0);
        let r_eff = 1.0 / (1.0 / self.rs + 1.0 / self.rg);
        let fc = 1.0 / (2.0 * std::f64::consts::PI * r_eff * c_miller.max(1e-13));
        self.miller_a = (-2.0 * std::f64::consts::PI * (fc / fs).min(0.49)).exp();
    }

    /// Returns the grid voltage relative to its leak return. `ig(v)` gives
    /// the grid current and its derivative for a grid voltage `v`.
    #[inline]
    pub fn process(&mut self, vs: f64, ig: impl Fn(f64) -> (f64, f64)) -> f64 {
        let src = vs - self.vc - self.k * self.i_prev;
        let rt = self.rs + self.k;
        // solve (src - v)/rt = v/rg + ig(v); no grid current = linear divider
        let mut v = src * self.rg / (self.rg + rt);
        let mut iters = if ig(v).0 > 0.0 { 5 } else { 0 };
        while iters > 0 {
            iters -= 1;
            let (i_g, di_g) = ig(v);
            let f = (src - v) / rt - v / self.rg - i_g;
            let df = -1.0 / rt - 1.0 / self.rg - di_g;
            let dv = f / df;
            v -= dv;
            if dv.abs() < 1e-7 {
                break;
            }
        }
        let i = (src - v) * (1.0 / rt);
        if self.cc > 0.0 {
            self.vc += self.k * (i + self.i_prev);
            self.i_prev = i;
        }
        // Miller / stopper roll-off
        self.miller_z = v + self.miller_a * (self.miller_z - v);
        self.miller_z
    }
}

// ----------------------------------------------------------- triode stage --

/// Common-cathode triode gain stage.
#[derive(Clone)]
pub struct TriodeStage {
    pub valve: Valve,
    table: LoadLineTable,
    rk: f64,
    inv_rk: f64,
    gck: f64,
    vk: f64,
    ick: f64,
    ip0: f64,
    vk0: f64,
    #[allow(dead_code)]
    ra_ac: f64,
    /// Small-signal plate resistance at the operating point (ohm).
    pub rp: f64,
    /// Plate resistor (ohm).
    pub ra: f64,
    /// DC plate voltage at idle.
    pub vp0: f64,
}

impl TriodeStage {
    /// `vb`: supply, `ra`: plate resistor, `rk`: cathode resistor,
    /// `ck`: cathode bypass (0 = unbypassed), `r_load`: AC load on the plate
    /// (next grid leak / network; `f64::INFINITY` = open circuit).
    pub fn new(fs: f64, valve: Valve, vb: f64, ra: f64, rk: f64, ck: f64, r_load: f64) -> Self {
        // DC operating point: vk = rk*ip, vgk = -vk, vpk = vb - ra*ip - vk
        let (mut a, mut b) = (0.0, vb / ra);
        for _ in 0..80 {
            let m = 0.5 * (a + b);
            let vk = rk * m;
            if valve.ip(-vk, vb - ra * m - vk) > m {
                a = m;
            } else {
                b = m;
            }
        }
        let ip0 = 0.5 * (a + b);
        let vk0 = rk * ip0;
        let vp0 = vb - ra * ip0;
        let ra_ac = if r_load.is_finite() { ra * r_load / (ra + r_load) } else { ra };
        let vpk_of = move |ip: f64| (vp0 - ra_ac * (ip - ip0) - vk0).max(0.0);
        let table = LoadLineTable::build(&valve, -(vb / valve.mu) * 1.6 - 4.0, 4.0, 4096, vpk_of, vb / ra_ac * 1.5 + 0.02);
        let dv = 0.5;
        let rp = dv / (valve.ip(-vk0, vp0 - vk0 + dv) - valve.ip(-vk0, vp0 - vk0 - dv)).max(1e-12) * 2.0;
        Self {
            valve,
            table,
            rk,
            inv_rk: if rk > 0.0 { 1.0 / rk } else { 0.0 },
            gck: if ck > 0.0 { 2.0 * fs * ck } else { 0.0 },
            vk: vk0,
            ick: 0.0,
            ip0,
            vk0,
            ra_ac,
            rp,
            ra,
            vp0,
        }
    }

    pub fn reset(&mut self) {
        self.vk = self.vk0;
        self.ick = 0.0;
    }

    /// Source resistance that goes with the Thevenin voltage returned by
    /// [`process`](Self::process). The valve's own plate resistance is in
    /// the current it computes along the stage's load line.
    pub fn r_out(&self) -> f64 {
        self.ra
    }

    /// Bias point grid-cathode voltage (for grid-current calculations).
    pub fn vgk_bias(&self) -> f64 {
        -self.vk0
    }

    /// Present cathode voltage deviation from idle.
    pub fn vk_ac(&self) -> f64 {
        self.vk - self.vk0
    }

    /// Grid current for a grid AC voltage (uses the present cathode voltage).
    #[inline]
    pub fn grid_current(&self, vg_ac: f64) -> (f64, f64) {
        let vgk = vg_ac - self.vk;
        (self.valve.ig(vgk), self.valve.ig_deriv(vgk))
    }

    /// AC grid voltage in; out: the plate signal as a Thevenin voltage
    /// behind the plate resistor (Norton plate current x Ra). Feed it to the
    /// next network with source resistance [`r_out`](Self::r_out).
    #[inline]
    pub fn process(&mut self, vg: f64) -> f64 {
        let vk_prev = self.vk;
        let mut vk = vk_prev;
        if self.rk > 0.0 {
            for _ in 0..4 {
                let (ip, gm) = self.table.eval(vg - vk);
                let ic = self.gck * (vk - vk_prev) - self.ick;
                let f = ip - vk * self.inv_rk - ic;
                let df = -gm - self.inv_rk - self.gck;
                let d = f / df;
                vk -= d;
                if d.abs() < 1e-6 {
                    break;
                }
            }
            self.ick = self.gck * (vk - vk_prev) - self.ick;
            self.vk = vk;
        }
        let (ip, _) = self.table.eval(vg - vk);
        -self.ra * (ip - self.ip0)
    }
}

// -------------------------------------------------------- cathode follower --

/// Cathode follower: plate at the supply, output taken at the cathode. The
/// grid is DC-coupled to the previous plate; when the grid is driven above
/// the cathode it draws current through the previous stage's plate resistor,
/// which flattens one side of the waveform (part of the Top Boost / tweed
/// "fizz"). The static transfer vk(vg) is tabulated from the Koren model.
#[derive(Clone)]
pub struct CathodeFollower {
    valve: Valve,
    lo: f64,
    step: f64,
    vk_tab: Vec<f64>,
    vg0: f64,
    vk0: f64,
    r_src: f64,
    /// Output resistance (~1/gm in parallel with the cathode resistor).
    pub r_out: f64,
}

impl CathodeFollower {
    /// `vg0`: DC grid voltage (previous plate), `r_src`: resistance feeding
    /// the grid (previous plate resistor).
    pub fn new(valve: Valve, vb: f64, rk: f64, vg0: f64, r_src: f64) -> Self {
        let solve = |vg: f64| {
            // vk/rk = ip(vg - vk, vb - vk), monotonic in vk: bisection
            let (mut a, mut b) = (0.0, vb);
            for _ in 0..70 {
                let m = 0.5 * (a + b);
                if valve.ip(vg - m, vb - m) > m / rk {
                    a = m;
                } else {
                    b = m;
                }
            }
            0.5 * (a + b)
        };
        let lo = vg0 - 150.0;
        let hi = vg0 + 150.0;
        let n = 3001;
        let step = (hi - lo) / (n - 1) as f64;
        let vk_tab: Vec<f64> = (0..n).map(|i| solve(lo + step * i as f64)).collect();
        let vk0 = solve(vg0);
        let h = 0.05;
        let gain = (solve(vg0 + h) - solve(vg0 - h)) / (2.0 * h);
        // follower gain g = gm*rk/(1+gm*rk) -> output resistance rk*(1-g)
        let r_out = rk * (1.0 - gain).max(1e-4);
        let mut cf = Self { valve, lo, step, vk_tab, vg0, vk0, r_src, r_out };
        // followers often idle right at the edge of grid conduction: settle
        // the DC point with the grid current flowing through r_src
        cf.vk0 = 0.0;
        cf.vk0 = cf.process(0.0);
        cf
    }

    #[inline]
    fn vk_of(&self, vg: f64) -> f64 {
        let x = ((vg - self.lo) / self.step).clamp(0.0, (self.vk_tab.len() - 2) as f64);
        let i = x as usize;
        let t = x - i as f64;
        self.vk_tab[i] + t * (self.vk_tab[i + 1] - self.vk_tab[i])
    }

    pub fn reset(&mut self) {}

    #[inline]
    pub fn process(&mut self, vg_ac: f64) -> f64 {
        let vin = self.vg0 + vg_ac;
        let mut vg = vin;
        let mut vk = self.vk_of(vg);
        // grid conduction pulls the driving plate down: vg = vin - ig*r_src
        for _ in 0..3 {
            let x = vg - vk;
            let f = vg - vin + self.valve.ig(x) * self.r_src;
            if f.abs() < 1e-6 {
                break;
            }
            let df = 1.0 + self.valve.ig_deriv(x) * self.r_src;
            vg -= f / df;
            vk = self.vk_of(vg);
        }
        vk - self.vk0
    }
}

// ------------------------------------------------------- phase inverter --

/// Long-tailed-pair phase inverter. Input on grid 1, grid 2 takes the
/// negative-feedback signal (0 if the amp has none).
#[derive(Clone)]
pub struct LongTailPair {
    table: LoadLineTable,
    rtail: f64,
    ip0: f64,
    u: f64,
    ra1: f64,
    ra2: f64,
    pub r_out: f64,
}

impl LongTailPair {
    pub fn new(valve: Valve, vb: f64, ra1: f64, ra2: f64, rtail: f64, rbias: f64) -> Self {
        // operating point: each side ~ same current; the cathodes sit at
        // rtail*(2ip)+rbias*2ip, grids at rtail*2ip -> vgk = -2*ip*rbias
        let ra = 0.5 * (ra1 + ra2);
        let (mut a, mut b) = (0.0, vb / (ra + 2.0 * rtail));
        for _ in 0..80 {
            let m = 0.5 * (a + b);
            let vk = 2.0 * m * (rtail + rbias);
            if valve.ip(-2.0 * m * rbias, vb - ra * m - vk) > m {
                a = m;
            } else {
                b = m;
            }
        }
        let ip0 = 0.5 * (a + b);
        let vk0 = 2.0 * ip0 * (rtail + rbias);
        let vp0 = vb - ra * ip0;
        let table = LoadLineTable::build(&valve, -12.0, 4.0, 4096, move |ip| (vp0 - ra * (ip - ip0) - vk0).max(0.0), vb / ra);
        let dv = 0.5;
        let vgk0 = -2.0 * ip0 * rbias;
        let rp = 2.0 * dv / (valve.ip(vgk0, vp0 - vk0 + dv) - valve.ip(vgk0, vp0 - vk0 - dv)).max(1e-12);
        // shift the table so it is indexed by grid voltage relative to bias
        let mut t = table;
        t.lo -= vgk0;
        let _ = rp;
        Self { table: t, rtail, ip0, u: 0.0, ra1, ra2, r_out: ra }
    }

    pub fn reset(&mut self) {
        self.u = 0.0;
    }

    /// Returns the two plate signals as Thevenin voltages behind the plate
    /// resistors (inverted side first: grid 1's own plate).
    #[inline]
    pub fn process(&mut self, g1: f64, g2: f64) -> (f64, f64) {
        let mut u = self.u;
        for _ in 0..4 {
            let (i1, d1) = self.table.eval(g1 - u);
            let (i2, d2) = self.table.eval(g2 - u);
            let f = (i1 + i2 - 2.0 * self.ip0) * self.rtail - u;
            let df = -(d1 + d2) * self.rtail - 1.0;
            let d = f / df;
            u -= d;
            if d.abs() < 1e-6 {
                break;
            }
        }
        self.u = u;
        let (i1, _) = self.table.eval(g1 - u);
        let (i2, _) = self.table.eval(g2 - u);
        (-self.ra1 * (i1 - self.ip0), -self.ra2 * (i2 - self.ip0))
    }
}

// ------------------------------------------------------------ power amp --

#[derive(Clone, Copy, Debug)]
pub enum PowerBias {
    /// Shared cathode resistor and bypass cap (Vox AC30 style).
    Cathode { rk: f64, ck: f64 },
    /// Fixed negative grid bias, set for this idle current per valve (A).
    Fixed { idle_ma: f64 },
}

#[derive(Clone, Copy, Debug)]
pub struct PowerSpec {
    pub valve: Valve,
    /// Valves per side of the push-pull pair.
    pub per_side: usize,
    pub b_plus: f64,
    /// Screen supply as a fraction of the plate supply.
    pub screen_ratio: f64,
    /// Ultra-linear screen tap (fraction of the primary half-winding; 0 =
    /// plain pentode/beam operation).
    pub ultralinear: f64,
    /// Plate-to-plate load (ohm).
    pub raa: f64,
    pub bias: PowerBias,
    /// Supply source resistance (rectifier + transformer) and first filter cap.
    pub r_supply: f64,
    pub c_supply: f64,
    /// Grid coupling cap, grid leak, grid stopper.
    pub cc: f64,
    pub rg: f64,
    pub r_stop: f64,
}

/// Koren's pentode equations are homogeneous in the screen voltage:
/// Ip = Vg2^Ex * G(Vgk/Vg2) * atan(Vpk/Kvb) and Ig2 = Vg2^Ex * H(Vgk/Vg2),
/// so G and H are tabulated once and Vg2^Ex is the only power per sample.
#[derive(Clone)]
struct PentodeTable {
    lo: f64,
    inv_step: f64,
    g: Vec<f64>,
    h: Vec<f64>,
}

impl PentodeTable {
    fn new(v: &Valve) -> Self {
        let (lo, hi, n) = (-3.0, 1.0, 8192);
        let step = (hi - lo) / (n - 1) as f64;
        let mut g = Vec::with_capacity(n);
        let mut h = Vec::with_capacity(n);
        for i in 0..n {
            let r = lo + step * i as f64;
            let e = softplus(v.kp * (1.0 / v.mu + r)) / v.kp;
            g.push(if e > 0.0 { e.powf(v.ex) / v.kg1 } else { 0.0 });
            let x = 1.0 / v.mu + r;
            h.push(if x > 0.0 && v.kg2 > 0.0 { x.powf(v.ex) / v.kg2 } else { 0.0 });
        }
        Self { lo, inv_step: 1.0 / step, g, h }
    }

    #[inline]
    fn eval(&self, r: f64) -> (f64, f64) {
        let x = ((r - self.lo) * self.inv_step).max(0.0);
        let last = self.g.len() - 1;
        let i = (x as usize).min(last - 1);
        let t = x - i as f64;
        (self.g[i] + t * (self.g[i + 1] - self.g[i]), self.h[i] + t * (self.h[i + 1] - self.h[i]))
    }
}

/// atan for x >= 0, |error| < 1e-5 rad.
#[inline]
fn atan_pos(x: f64) -> f64 {
    let (y, flip) = if x > 1.0 { (1.0 / x, true) } else { (x, false) };
    let y2 = y * y;
    let a = y * (0.999_866_0 + y2 * (-0.330_299_5 + y2 * (0.180_141_0 + y2 * (-0.085_133_0 + y2 * 0.020_835_1))));
    if flip {
        std::f64::consts::FRAC_PI_2 - a
    } else {
        a
    }
}

/// Push-pull pentode/beam power stage with bias, supply sag and the
/// interaction of the plates through the output transformer.
#[derive(Clone)]
pub struct PushPull {
    spec: PowerSpec,
    table: PentodeTable,
    v_source: f64,
    grids: [GridInput; 2],
    vbias: f64,
    vk: f64,
    vb: f64,
    vp: [f64; 2],
    i: [f64; 2],
    i0: f64,
    dt: f64,
    pub vk0: f64,
    /// Grid conduction of the output valves (A), for metering.
    pub grid_current: f64,
}

impl PushPull {
    pub fn new(fs: f64, spec: PowerSpec, r_drive: f64) -> Self {
        let v = spec.valve;
        let n = spec.per_side as f64;
        let vb = spec.b_plus;
        let vg2 = vb * spec.screen_ratio;
        let (vbias, vk0, i0) = match spec.bias {
            PowerBias::Cathode { rk, .. } => {
                // all valves share rk: vk = rk * total current
                let (mut a, mut b) = (0.0, 0.2 * n * 2.0);
                for _ in 0..80 {
                    let m = 0.5 * (a + b); // total current
                    let vk = rk * m;
                    let per = v.ip_pentode(-vk, vg2 - vk, vb - vk) + v.ig2(-vk, vg2 - vk);
                    if per * 2.0 * n > m {
                        a = m;
                    } else {
                        b = m;
                    }
                }
                let it = 0.5 * (a + b);
                let vk = rk * it;
                (0.0, vk, v.ip_pentode(-vk, vg2 - vk, vb - vk))
            }
            PowerBias::Fixed { idle_ma } => {
                let target = idle_ma * 1e-3;
                let (mut a, mut b) = (-150.0, 0.0);
                for _ in 0..80 {
                    let m = 0.5 * (a + b);
                    if v.ip_pentode(m, vg2, vb) > target {
                        b = m;
                    } else {
                        a = m;
                    }
                }
                (0.5 * (a + b), 0.0, target)
            }
        };
        // grid-cathode + Miller capacitance of the paralleled output valves
        let c_miller = 15e-12 * n;
        let grid = GridInput::new(fs, r_drive + spec.r_stop, spec.cc, spec.rg, c_miller);
        // `b_plus` is the idle (loaded) plate supply; the rectifier's
        // open-circuit voltage sits above it by the idle current's drop
        let idle_total = 2.0 * n * (i0 + v.ig2(vbias - vk0, vg2 - vk0));
        let v_source = vb + idle_total * spec.r_supply;
        Self {
            table: PentodeTable::new(&v),
            v_source,
            spec,
            grids: [grid.clone(), grid],
            vbias,
            vk: vk0,
            vb,
            vp: [vb; 2],
            i: [i0 * n; 2],
            i0,
            dt: 1.0 / fs,
            vk0,
            grid_current: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.grids.iter_mut().for_each(|g| g.reset());
        self.vk = self.vk0;
        self.vb = self.spec.b_plus;
        self.vp = [self.vb; 2];
        let n = self.spec.per_side as f64;
        self.i = [self.i0 * n; 2];
    }

    /// Idle plate current per valve (A).
    pub fn idle_current(&self) -> f64 {
        self.i0
    }

    /// Present supply voltage (sags under load).
    pub fn supply(&self) -> f64 {
        self.vb
    }

    /// Drive voltages for the two sides in; plate-to-plate primary voltage out.
    #[inline]
    pub fn process(&mut self, d1: f64, d2: f64) -> f64 {
        let s = self.spec;
        let v = s.valve;
        let n = s.per_side as f64;
        let vk = self.vk;
        let vbias = self.vbias;
        let g = [
            self.grids[0].process(d1, |x| (v.ig(x + vbias - vk) * n, v.ig_deriv(x + vbias - vk) * n)),
            self.grids[1].process(d2, |x| (v.ig(x + vbias - vk) * n, v.ig_deriv(x + vbias - vk) * n)),
        ];
        self.grid_current = (v.ig(g[0] + vbias - vk) + v.ig(g[1] + vbias - vk)) * n;
        // Koren pentode: Ip = F(vgk, vg2) * atan(vpk / kvb). F is fixed for
        // this sample; only the plate term couples the two sides.
        let mut f = [0.0; 2];
        let mut ig2 = 0.0;
        let mut vg2k = [0.0; 2];
        for (side, s2) in vg2k.iter_mut().enumerate() {
            let vg2 = self.vb * s.screen_ratio + s.ultralinear * (self.vp[side] - self.vb);
            *s2 = (vg2 - vk).max(1.0);
        }
        let p0 = vg2k[0].powf(v.ex) * n;
        let p1 = if s.ultralinear != 0.0 { vg2k[1].powf(v.ex) * n } else { p0 };
        for side in 0..2 {
            let vgk = g[side] + vbias - vk;
            let (gg, hh) = self.table.eval(vgk / vg2k[side]);
            let p = if side == 0 { p0 } else { p1 };
            f[side] = p * gg;
            ig2 += p * hh;
        }
        // Newton on i = F*atan((vp - vk)/kvb), vp = vb -/+ q*(i0 - i1)
        let q = s.raa / 4.0;
        let inv_kvb = 1.0 / v.kvb;
        let mut i = self.i;
        for _ in 0..4 {
            let diff = i[0] - i[1];
            let vp = [self.vb - q * diff, self.vb + q * diff];
            let mut r = [0.0; 2];
            let mut gp = [0.0; 2];
            for side in 0..2 {
                let x = ((vp[side] - vk) * inv_kvb).max(0.0);
                r[side] = i[side] - f[side] * atan_pos(x);
                gp[side] = if vp[side] > vk { f[side] * inv_kvb / (1.0 + x * x) } else { 0.0 };
            }
            // Jacobian: d vp0/d i0 = -q, d vp0/d i1 = +q, d vp1/d i0 = +q, d vp1/d i1 = -q
            let j00 = 1.0 + gp[0] * q;
            let j01 = -gp[0] * q;
            let j10 = -gp[1] * q;
            let j11 = 1.0 + gp[1] * q;
            let det = j00 * j11 - j01 * j10;
            let d0 = (r[0] * j11 - r[1] * j01) / det;
            let d1 = (j00 * r[1] - j10 * r[0]) / det;
            i[0] = (i[0] - d0).max(0.0);
            i[1] = (i[1] - d1).max(0.0);
            if d0.abs() + d1.abs() < 1e-7 {
                break;
            }
        }
        self.i = i;
        let diff = i[0] - i[1];
        self.vp = [self.vb - q * diff, self.vb + q * diff];
        let total = i[0] + i[1] + ig2;
        // cathode bias: shared resistor + bypass cap
        if let PowerBias::Cathode { rk, ck } = s.bias {
            self.vk += self.dt * (total - self.vk / rk) / ck;
        }
        // supply sag: rectifier/transformer resistance into the filter cap
        self.vb += self.dt * ((self.v_source - self.vb) / s.r_supply - total) / s.c_supply;
        (s.raa / 2.0) * diff
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecc83_operating_point_is_realistic() {
        // classic 100k / 1.5k / 250 V stage: ~1.0-1.5 V bias, ~0.8-1.2 mA, gain ~ 55-65
        let st = TriodeStage::new(96000.0, ECC83, 250.0, 100e3, 1.5e3, 25e-6, 1e6);
        let bias = -st.vgk_bias();
        println!("bias {:.2} V, Ia {:.2} mA, Vp {:.0} V, rp {:.0} k", bias, st.ip0 * 1e3, st.vp0, st.rp / 1e3);
        assert!((0.8..2.0).contains(&bias));
        assert!((0.5..1.6).contains(&(st.ip0 * 1e3)));
        let mut s = st.clone();
        // small-signal gain at 1 kHz-ish (bypassed): drive a small sine
        let mut peak: f64 = 0.0;
        for k in 0..96000 {
            let x = 0.01 * (k as f64 * 2.0 * std::f64::consts::PI * 1000.0 / 96000.0).sin();
            let y = s.process(x);
            if k > 48000 {
                peak = peak.max(y.abs());
            }
        }
        // loaded by the 1M grid leak of the next stage
        let gain = peak / 0.01 * 1e6 / (1e6 + 100e3);
        println!("gain {gain:.1}");
        assert!((45.0..75.0).contains(&gain));
    }

    #[test]
    fn grid_current_clamps_positive_side() {
        let mut st = TriodeStage::new(96000.0, ECC83, 250.0, 100e3, 1.5e3, 25e-6, 1e6);
        let mut gi = GridInput::new(96000.0, 68e3, 22e-9, 1e6, 100e-12);
        let mut maxg: f64 = -1e9;
        for k in 0..96000 {
            let x = 20.0 * (k as f64 * 2.0 * std::f64::consts::PI * 200.0 / 96000.0).sin();
            let vg = gi.process(x, |v| st.grid_current(v));
            let _ = st.process(vg);
            if k > 48000 {
                maxg = maxg.max(vg - st.vk_ac() + st.vgk_bias());
            }
        }
        println!("max vgk {maxg:.2} V");
        // grid conduction holds vgk to a few volts despite a 20 V drive
        assert!(maxg < 5.0);
    }

    #[test]
    fn el84_cathode_biased_quad_idles_sensibly() {
        let spec = PowerSpec {
            valve: EL84,
            per_side: 2,
            b_plus: 320.0,
            screen_ratio: 0.97,
            raa: 4000.0,
            bias: PowerBias::Cathode { rk: 50.0, ck: 250e-6 },
            ultralinear: 0.0,
            r_supply: 150.0,
            c_supply: 32e-6,
            cc: 0.1e-6,
            rg: 220e3,
            r_stop: 1.8e3,
        };
        let pp = PushPull::new(96000.0, spec, 40e3);
        println!("AC30-style idle: vk {:.1} V, {:.1} mA per valve", pp.vk0, pp.i0 * 1e3);
        assert!((7.0..16.0).contains(&pp.vk0));
        assert!((25.0..60.0).contains(&(pp.i0 * 1e3)));
    }
}
