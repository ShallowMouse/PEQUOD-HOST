//! Studio Spaces — the rooms the Beatles recorded in.
//!
//! Each space is built from its geometry: the room's dimensions, where the
//! band stood and where the room microphone is. Early reflections come from
//! image sources off each wall; the walls' absorption is solved so the room
//! decays at its measured (or best-estimate) reverberation times, and the
//! diffuse tail follows at the level acoustics theory gives for that mic
//! distance and polar pattern. The output is what a room mic in that space
//! would hear; MIX blends it with the close-miked (dry) signal.

pub mod room;

use nih_plug::prelude::*;
use room::*;
use std::sync::{Arc, RwLock};
use vintage::dsp::*;
use vintage::nih_plug_egui::EguiState;
use vintage::params::*;
use vintage::ui::{self, Item, Panel, Plan, Preset};
use vintage::{VintageDsp, VintageParams};

const W: u32 = 640;
const H: u32 = 432;

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Space {
    #[name = "EMI Studio Two"]
    Two,
    #[name = "EMI Studio One"]
    One,
    #[name = "EMI Studio Three"]
    Three,
    #[name = "Trident, Soho"]
    Trident,
    #[name = "Olympic, Barnes"]
    Olympic,
    #[name = "Apple, Savile Row"]
    Apple,
    #[name = "Twickenham Stage One"]
    Twickenham,
    #[name = "Apple Rooftop"]
    Rooftop,
}

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum MicPattern {
    #[name = "Omni"]
    Omni,
    #[name = "Cardioid"]
    Cardioid,
    #[name = "Figure-8"]
    Figure8,
}

#[derive(Params)]
pub struct SpaceParams {
    #[persist = "editor-state-v2"]
    editor_state: Arc<EguiState>,
    #[persist = "gui-values"]
    saved: Arc<RwLock<Vec<f32>>>,
    #[id = "space"]
    space: EnumParam<Space>,
    #[id = "distance"]
    distance: FloatParam,
    #[id = "pattern"]
    pattern: EnumParam<MicPattern>,
    #[id = "screens"]
    screens: FloatParam,
    #[id = "spacing"]
    spacing: FloatParam,
    #[id = "lowcut"]
    low_cut: FloatParam,
    #[id = "mix"]
    mix: FloatParam,
    #[id = "output"]
    output: FloatParam,
}

impl Default for SpaceParams {
    fn default() -> Self {
        Self {
            editor_state: ui::editor_state((W, H)),
            saved: Arc::new(RwLock::new(Vec::new())),
            space: EnumParam::new("Space", Space::Two),
            distance: FloatParam::new("Mic Distance", 3.0, FloatRange::Skewed { min: 0.5, max: 25.0, factor: FloatRange::skew_factor(-1.5) })
                .with_step_size(0.1)
                .with_unit(" m"),
            pattern: EnumParam::new("Mic Pattern", MicPattern::Cardioid),
            screens: pct("Screens", 0.0),
            spacing: FloatParam::new("Mic Spacing", 0.0, FloatRange::Linear { min: 0.0, max: 3.0 })
                .with_step_size(0.05)
                .with_unit(" m"),
            low_cut: hz("Room Low Cut", 20.0, 20.0, 500.0),
            mix: pct("Mix", 0.35),
            output: db("Output", 0.0, -18.0, 12.0),
        }
    }
}

const SPACE: usize = 0;
const DISTANCE: usize = 1;
const PATTERN: usize = 2;
const SCREENS: usize = 3;
const SPACING: usize = 4;
const LOWCUT: usize = 5;
const MIX: usize = 6;
const OUTPUT: usize = 7;

impl VintageParams for SpaceParams {
    fn editor_state(&self) -> Arc<EguiState> {
        self.editor_state.clone()
    }
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>> {
        &self.saved
    }
    fn linked(&self) -> Vec<ParamPtr> {
        vec![
            self.space.as_ptr(),
            self.distance.as_ptr(),
            self.pattern.as_ptr(),
            self.screens.as_ptr(),
            self.spacing.as_ptr(),
            self.low_cut.as_ptr(),
            self.mix.as_ptr(),
            self.output.as_ptr(),
        ]
    }
}

// ----------------------------------------------------------------- spaces --

const FT: f64 = 0.3048;

/// The spaces, in selector order. Sources for each are in the panel text
/// and README; estimates are marked there.
pub fn spaces() -> [RoomSpec; 8] {
    let walls = [1.0, 1.0, 1.0, 1.0, 0.45, 1.2];
    [
        // EMI Studio Two: 60 x 40 x 23 ft. 1960s: mid RT 0.4-0.65 s depending
        // on how many of the hinged screens were out; bass held near 1 s by
        // the 1950s panel absorbers (IoA paper, P. Dix).
        RoomSpec {
            dims: [60.0 * FT, 40.0 * FT, 23.0 * FT],
            rt_live: [1.0, 0.65, 0.55],
            rt_damped: [0.8, 0.40, 0.33],
            bias: walls,
            src: [5.5, 6.1, 1.2],
            mic_height: 2.0,
            open_air: None,
        },
        // EMI Studio One: 93 x 56 x 40 ft, mid RT ~2.35 s after the 1960s
        // panelling (2.05 s in the late 1950s, falling below 200 Hz).
        RoomSpec {
            dims: [93.0 * FT, 56.0 * FT, 40.0 * FT],
            rt_live: [2.0, 2.35, 1.7],
            rt_damped: [1.75, 2.05, 1.5],
            bias: walls,
            src: [8.0, 8.5, 1.2],
            mic_height: 3.0,
            open_air: None,
        },
        // EMI Studio Three: 39 x 31 x 19 ft, mid RT ~0.65 s in 1960-70 with
        // Cabot's quilt (low/high values estimated from the survey's trend).
        RoomSpec {
            dims: [39.0 * FT, 31.0 * FT, 19.0 * FT],
            rt_live: [0.8, 0.65, 0.45],
            rt_damped: [0.6, 0.45, 0.32],
            bias: walls,
            src: [3.5, 4.7, 1.2],
            mic_height: 2.0,
            open_air: None,
        },
        // Trident, 17 St Anne's Court: basement studio ~40 ft long and ~20 ft
        // high (double height, control room looking down). Width and decay
        // times are estimates for a treated late-60s pop room.
        RoomSpec {
            dims: [40.0 * FT, 25.0 * FT, 20.0 * FT],
            rt_live: [0.7, 0.55, 0.42],
            rt_damped: [0.55, 0.40, 0.30],
            bias: walls,
            src: [3.5, 3.8, 1.2],
            mic_height: 2.0,
            open_air: None,
        },
        // Olympic Studio One, Barnes: built in a ~40 x 30 x 22 ft shell;
        // hard vinyl floor, absorptive ceiling tiles, angled wood slats,
        // "very clean, no secondary echo". Decay times estimated.
        RoomSpec {
            dims: [40.0 * FT, 30.0 * FT, 22.0 * FT],
            rt_live: [0.75, 0.70, 0.55],
            rt_damped: [0.6, 0.5, 0.4],
            bias: [0.9, 0.9, 0.8, 0.8, 0.3, 1.6],
            src: [3.5, 4.6, 1.2],
            mic_height: 2.0,
            open_air: None,
        },
        // Apple, 3 Savile Row basement, January 1969: a low-ceilinged basement
        // with little treatment. Dimensions and decay are estimates.
        RoomSpec {
            dims: [10.0, 6.0, 3.0],
            rt_live: [0.75, 0.5, 0.45],
            rt_damped: [0.55, 0.38, 0.32],
            bias: [1.0, 1.0, 1.0, 1.0, 0.5, 1.0],
            src: [2.8, 3.0, 1.2],
            mic_height: 1.8,
            open_air: None,
        },
        // Twickenham Film Studios: the 165 x 75 ft stage; height and decay
        // estimated (a big film stage with absorbent walls).
        RoomSpec {
            dims: [165.0 * FT, 75.0 * FT, 40.0 * FT],
            rt_live: [1.8, 1.4, 1.0],
            rt_damped: [1.4, 1.0, 0.75],
            bias: walls,
            src: [15.0, 11.4, 1.2],
            mic_height: 2.5,
            open_air: None,
        },
        // The roof of 3 Savile Row, 30 January 1969: open sky, the roof
        // surface under the band, low parapets and chimney stacks, and the
        // building fronts across Savile Row. Geometry estimated.
        RoomSpec {
            dims: [24.0, 14.0, 30.0],
            rt_live: [0.8, 1.0, 0.6],
            rt_damped: [0.8, 1.0, 0.6],
            bias: walls,
            src: [5.0, 7.0, 1.2],
            mic_height: 1.6,
            open_air: Some(OpenAir {
                beta: [[0.35, 0.3, 0.25], [0.6, 0.55, 0.45], [0.2, 0.15, 0.1], [0.2, 0.15, 0.1], [0.9, 0.88, 0.8], [0.0, 0.0, 0.0]],
                tail_energy: 0.0005,
                tail_rt: 1.0,
            }),
        },
    ]
}

pub const PLANS: &[Plan] = &[
    Plan { length: 18.29, width: 12.19, src: (5.5, 6.1), open_air: false, far_wall: "" },
    Plan { length: 28.35, width: 17.07, src: (8.0, 8.5), open_air: false, far_wall: "" },
    Plan { length: 11.89, width: 9.45, src: (3.5, 4.7), open_air: false, far_wall: "" },
    Plan { length: 12.19, width: 7.62, src: (3.5, 3.8), open_air: false, far_wall: "" },
    Plan { length: 12.19, width: 9.14, src: (3.5, 4.6), open_air: false, far_wall: "" },
    Plan { length: 10.0, width: 6.0, src: (2.8, 3.0), open_air: false, far_wall: "" },
    Plan { length: 50.29, width: 22.86, src: (15.0, 11.4), open_air: false, far_wall: "" },
    Plan { length: 24.0, width: 14.0, src: (5.0, 7.0), open_air: true, far_wall: "Savile Row  " },
];

const NAMES: &[&str] = &[
    "EMI Studio Two",
    "EMI Studio One",
    "EMI Studio Three",
    "Trident, Soho",
    "Olympic, Barnes",
    "Apple, Savile Row",
    "Twickenham Stage",
    "Apple Rooftop",
];

const ABOUT: &[&str] = &[
    "Abbey Road's Studio Two, 60 x 40 x 23 ft: most Beatles records were made here. Short, warm decay: 0.65 s, down to 0.4 s with the hinged screens out.",
    "Abbey Road's orchestral hall, 93 x 56 x 40 ft, about 2.3 s: the orchestra on A Day in the Life, All You Need Is Love's broadcast.",
    "Abbey Road's smallest room, 39 x 31 x 19 ft, 0.65 s, quilted for pop work in the 1960s.",
    "Trident, St Anne's Court (1968): a double-height basement room, control room looking down. Hey Jude, Dear Prudence. Width and decay estimated.",
    "Olympic Studio One, Barnes: hard floor, absorbent ceiling, angled wood slats; clean, no flutter. Baby You're a Rich Man, early Something. Decay estimated.",
    "The Apple basement studio, January 1969: low ceiling, little treatment, close boxy reflections. Size and decay estimated.",
    "Twickenham Film Studios' 165 x 75 ft stage, where the Get Back sessions began. Height and decay estimated.",
    "The roof of 3 Savile Row, 30 January 1969: open air, the roof under the band and the buildings across the street. Geometry estimated.",
];

// -------------------------------------------------------------------- dsp --

pub struct SpaceDsp {
    fs: f64,
    specs: [RoomSpec; 8],
    space: usize,
    pending: Option<usize>,
    fade: f64,
    placement: Placement,
    target: Placement,
    built: Option<Placement>,
    line: Vec<f64>,
    pos: usize,
    taps: [Vec<Tap>; 2],
    filters: [[OrderFilter; MAX_ORDER + 1]; 2],
    tail: Tail,
    wet_scale: f64,
    width: f64,
    low_cut: [Biquad; 2],
    low_cut_hz: f64,
    mix: Smooth,
    out: Smooth,
}

impl SpaceDsp {
    fn rebuild(&mut self) {
        let spec = self.specs[self.space];
        let p = self.placement;
        let ac = acoustics(&spec, p.screens);
        let plan = PLANS[self.space];
        let mx = ui::mic_position(plan.length, plan.src.0, p.distance as f32) as f64;
        let half = 0.5 * p.spacing;
        let mut shelves = [[0.0; 2]; MAX_ORDER + 1];
        let mut t3 = (0.0, 0.0);
        let mut e_er = 0.0;
        for ch in 0..2 {
            let y = spec.src[1] + if ch == 0 { -half } else { half };
            let mic = [mx, y.clamp(0.3, spec.dims[1] - 0.3), spec.mic_height];
            image_taps(&spec, &ac, mic, p.pattern, self.fs, &mut self.taps[ch], &mut shelves);
            for o in 0..=MAX_ORDER {
                self.filters[ch][o].set(self.fs, if o == 0 { 0.0 } else { shelves[o][0] }, shelves[o][1]);
            }
            for t in &self.taps[ch] {
                e_er += 0.5 * t.gain * t.gain;
                if t.order == MAX_ORDER {
                    t3.0 += t.delay;
                    t3.1 += 1.0;
                }
            }
        }
        // the diffuse tail takes over around the third-order reflections
        let t0 = if t3.1 > 0.0 { t3.0 / t3.1 / self.fs } else { 0.05 };
        let (energy, rt) = match spec.open_air {
            Some(o) => (o.tail_energy * if p.pattern == Pattern::Omni { 1.0 } else { 1.0 / 3.0 }, [o.tail_rt * 0.8, o.tail_rt, o.tail_rt * 0.6]),
            None => (tail_energy(&ac, p.pattern, t0), ac.rt),
        };
        self.tail.configure(ac.mean_free_path, rt, t0, energy);
        // what the room mic hears, scaled to a steady level whatever the
        // distance (the balance of direct to room changes, not the loudness)
        self.wet_scale = 1.0 / (e_er + energy).sqrt();
        self.width = (p.spacing / 1.5).min(1.0);
        self.built = Some(p);
    }
}

fn pattern_of(v: f32) -> Pattern {
    match v.round() as i32 {
        0 => Pattern::Omni,
        2 => Pattern::Figure8,
        _ => Pattern::Cardioid,
    }
}

impl VintageDsp for SpaceDsp {
    const METERS: usize = 1;

    fn new(sample_rate: f32, _channels: usize) -> Self {
        let fs = sample_rate as f64;
        let placement = Placement { distance: 3.0, spacing: 0.0, pattern: Pattern::Cardioid, screens: 0.0 };
        let mut d = Self {
            fs,
            specs: spaces(),
            space: 0,
            pending: None,
            fade: 1.0,
            placement,
            target: placement,
            built: None,
            line: vec![0.0; 2 * ((ER_WINDOW * fs) as usize + 8)],
            pos: 0,
            taps: [Vec::with_capacity(512), Vec::with_capacity(512)],
            filters: Default::default(),
            tail: Tail::new(fs),
            wet_scale: 1.0,
            width: 0.0,
            low_cut: [Biquad::identity(), Biquad::identity()],
            low_cut_hz: 0.0,
            mix: Smooth::new(fs, 20.0, 0.35),
            out: Smooth::new(fs, 20.0, 1.0),
        };
        d.rebuild();
        d
    }

    fn reset(&mut self) {
        self.line.iter_mut().for_each(|v| *v = 0.0);
        self.tail.clear();
        for ch in &mut self.filters {
            ch.iter_mut().for_each(|f| f.reset());
        }
        self.low_cut.iter_mut().for_each(|b| b.reset());
    }

    fn set_params(&mut self, v: &[f32]) {
        let space = (v[SPACE].round() as usize).min(7);
        if space != self.space && self.pending != Some(space) {
            self.pending = Some(space);
        }
        self.target = Placement {
            distance: v[DISTANCE] as f64,
            spacing: v[SPACING] as f64,
            pattern: pattern_of(v[PATTERN]),
            screens: v[SCREENS] as f64,
        };
        let lc = v[LOWCUT] as f64;
        if (lc - self.low_cut_hz).abs() > 0.01 {
            self.low_cut_hz = lc;
            for b in &mut self.low_cut {
                if lc <= 20.5 {
                    *b = Biquad::identity();
                } else {
                    b.set_highpass(self.fs, lc, 0.707);
                }
            }
        }
        self.mix.set(v[MIX] as f64);
        self.out.set(db_to_gain(v[OUTPUT] as f64));
    }

    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]) {
        // glide the placement, rebuild when it has moved
        let t = self.target;
        let p = &mut self.placement;
        p.distance += (t.distance - p.distance) * 0.35;
        p.spacing += (t.spacing - p.spacing) * 0.35;
        p.screens += (t.screens - p.screens) * 0.35;
        if (p.distance - t.distance).abs() < 0.005 {
            p.distance = t.distance;
        }
        p.pattern = t.pattern;
        let changed = match self.built {
            Some(b) => {
                (b.distance - p.distance).abs() > 0.002 * b.distance.max(0.5)
                    || (b.spacing - p.spacing).abs() > 0.005
                    || (b.screens - p.screens).abs() > 0.003
                    || b.pattern != p.pattern
            }
            None => true,
        };
        if changed && self.pending.is_none() {
            self.rebuild();
        }

        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        let nch = chans.len();
        let len = self.line.len() / 2;
        let fade_step = 1.0 / (0.012 * self.fs);
        let mut peak: f64 = 0.0;
        for k in 0..n {
            // switching spaces: fade the room out, swap, fade back in
            if let Some(next) = self.pending {
                self.fade -= fade_step;
                if self.fade <= 0.0 {
                    self.fade = 0.0;
                    self.space = next;
                    self.pending = None;
                    self.reset();
                    self.rebuild();
                }
            } else if self.fade < 1.0 {
                self.fade = (self.fade + fade_step).min(1.0);
            }

            let mut x = 0.0;
            for c in chans.iter() {
                x += c[k] as f64;
            }
            x /= nch.max(1) as f64;
            // mirrored write: reads never need a wrap-around
            self.line[self.pos] = x;
            self.line[self.pos + len] = x;
            let base = self.pos + len;

            let mut er = [0.0; 2];
            for ch in 0..2 {
                let mut by_order = [0.0; MAX_ORDER + 1];
                for tap in &self.taps[ch] {
                    let d = tap.delay;
                    let i = d as usize;
                    let f = d - i as f64;
                    let a = self.line[base - i];
                    let b = self.line[base - i - 1];
                    by_order[tap.order] += tap.gain * (a + f * (b - a));
                }
                for (o, v) in by_order.iter().enumerate() {
                    er[ch] += self.filters[ch][o].process(*v);
                }
            }
            self.pos = (self.pos + 1) % len;
            let (tl, tr) = self.tail.process(x);
            let mid = 0.5 * (tl + tr);
            let side = 0.5 * (tl - tr) * self.width;
            let wet = [
                self.low_cut[0].process((er[0] + mid + side) * self.wet_scale),
                self.low_cut[1].process((er[1] + mid - side) * self.wet_scale),
            ];
            let m = self.mix.next();
            let g = self.out.next();
            let fade = self.fade;
            for (ci, c) in chans.iter_mut().enumerate() {
                let w = if nch == 1 { 0.5 * (wet[0] + wet[1]) } else { wet[ci.min(1)] };
                let y = (c[k] as f64 * (1.0 - m) + w * m * fade) * g;
                peak = peak.max(y.abs());
                c[k] = y as f32;
            }
        }
        meters[0] = peak as f32;
    }
}

// ------------------------------------------------------------------ panel --

fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "Studio Two: Drum Room Mic", values: &[(SPACE, 0.0), (DISTANCE, 3.5), (PATTERN, 1.0), (MIX, 0.3)] },
        Preset { name: "Studio Two: Screens Out (Dry)", values: &[(SPACE, 0.0), (DISTANCE, 2.0), (SCREENS, 1.0), (MIX, 0.25)] },
        Preset { name: "Studio Two: Ambient Figure-8", values: &[(SPACE, 0.0), (DISTANCE, 6.0), (PATTERN, 2.0), (MIX, 0.4), (LOWCUT, 80.0)] },
        Preset { name: "Studio One: Orchestra", values: &[(SPACE, 1.0), (DISTANCE, 8.0), (PATTERN, 0.0), (SPACING, 1.5), (MIX, 0.5)] },
        Preset { name: "Studio One: Hall Ambience", values: &[(SPACE, 1.0), (DISTANCE, 15.0), (PATTERN, 0.0), (SPACING, 2.0), (MIX, 0.35), (LOWCUT, 100.0)] },
        Preset { name: "Studio Three: Tight Room", values: &[(SPACE, 2.0), (DISTANCE, 2.0), (MIX, 0.3)] },
        Preset { name: "Trident: Piano Room", values: &[(SPACE, 3.0), (DISTANCE, 2.5), (PATTERN, 1.0), (MIX, 0.35)] },
        Preset { name: "Olympic: Clean Room", values: &[(SPACE, 4.0), (DISTANCE, 3.0), (PATTERN, 1.0), (SPACING, 0.8), (MIX, 0.35)] },
        Preset { name: "Apple Basement: Live Band", values: &[(SPACE, 5.0), (DISTANCE, 2.5), (PATTERN, 0.0), (MIX, 0.4)] },
        Preset { name: "Twickenham: Stage Rehearsal", values: &[(SPACE, 6.0), (DISTANCE, 8.0), (PATTERN, 0.0), (MIX, 0.5)] },
        Preset { name: "Rooftop Concert", values: &[(SPACE, 7.0), (DISTANCE, 4.0), (PATTERN, 1.0), (MIX, 0.6)] },
    ]
}

fn panel() -> Panel {
    Panel {
        title: "STUDIO SPACES",
        subtitle: ("ROOMS THE BEATLES RECORDED IN", "ABBEY ROAD · TRIDENT · OLYMPIC · APPLE"),
        accent: ui::PINE,
        size: (W, H),
        items: vec![
            Item::RadioList { p: SPACE, x: 34.0, y: 84.0, w: 178.0, row_h: 25.0, names: NAMES,
                tip: "Choose the room. Each is built from its dimensions and decay times." },
            Item::RoomPlan { x: 226.0, y: 84.0, w: 380.0, h: 146.0, room: SPACE, distance: DISTANCE, spacing: SPACING, plans: PLANS },
            Item::Caption { p: SPACE, x: 226.0, y: 236.0, w: 380.0, size: 11.5, texts: ABOUT },
            Item::Divider { y: 284.0 },
            Item::Knob { p: DISTANCE, x: 62.0, y: 337.0, r: 21.0, label: "MIC DISTANCE",
                tip: "How far the room mic is from the band. Close = more direct sound; past a few metres the room takes over.", active: None },
            Item::Selector { p: PATTERN, x: 150.0, y: 337.0, label: "PATTERN", positions: &["OMNI", "CARD", "FIG-8"],
                tip: "Microphone pattern. Cardioid (U47/U67) and figure-8 (ribbon) pick up a third of the diffuse room an omni does; figure-8's back lobe hears the far wall in reverse polarity." },
            Item::Knob { p: SCREENS, x: 238.0, y: 337.0, r: 21.0, label: "SCREENS",
                tip: "Acoustic screens and drapes out into the room. In Studio Two this is the real range: 0.65 s with the screens folded back to 0.4 s with them out.", active: None },
            Item::Knob { p: SPACING, x: 326.0, y: 337.0, r: 21.0, label: "MIC SPACING",
                tip: "0 = one room mic (mono, as the 1960s mostly were). Up = a spaced stereo pair.", active: None },
            Item::Knob { p: LOWCUT, x: 414.0, y: 337.0, r: 21.0, label: "ROOM LOW CUT", tip: "High-pass on the room signal only.", active: None },
            Item::Knob { p: MIX, x: 502.0, y: 337.0, r: 21.0, label: "MIX",
                tip: "Close (dry) signal vs. the room mic. The room mic's direct sound arrives later by its distance, as in a real room.", active: None },
            Item::Knob { p: OUTPUT, x: 590.0, y: 337.0, r: 21.0, label: "OUTPUT", tip: "Final level.", active: None },
        ],
        presets: presets(),
    }
}

vintage::vintage_plugin!(
    Plugin, SpaceParams, SpaceDsp, panel,
    name: "Studio Spaces", class_id: *b"S2SpacesDIY00001", vendor: "PEQUOD Beatles",
    categories: [Fx, Reverb]
);

#[cfg(test)]
mod tests {
    use super::*;

    fn params(space: f32, dist: f32, pattern: f32, screens: f32) -> Vec<f32> {
        vec![space, dist, pattern, screens, 0.0, 20.0, 1.0, 0.0]
    }

    /// Impulse response of the wet path.
    fn ir(space: f32, dist: f32, pattern: f32, screens: f32, secs: f64) -> Vec<f64> {
        let fs = 48000.0;
        let mut d = SpaceDsp::new(fs as f32, 1);
        let p = params(space, dist, pattern, screens);
        d.set_params(&p);
        // let the switch + placement settle
        let mut z = vec![0.0f32; 4096];
        for _ in 0..20 {
            d.set_params(&p);
            d.process(&mut [&mut z[..]], &mut [0.0]);
            z.iter_mut().for_each(|v| *v = 0.0);
        }
        d.reset();
        let n = (fs * secs) as usize;
        let mut x = vec![0.0f32; n];
        x[0] = 1.0;
        for c in x.chunks_mut(512) {
            d.process(&mut [c], &mut [0.0]);
        }
        x.into_iter().map(|v| v as f64).collect()
    }

    /// T30 from the Schroeder backward integral (-5 to -35 dB) of a band.
    fn t30(h: &[f64], fs: f64, f0: f64) -> f64 {
        let mut bp = Biquad::identity();
        bp.set_bandpass(fs, f0, 1.4);
        let y: Vec<f64> = h.iter().map(|v| bp.process(*v)).collect();
        let mut e: Vec<f64> = y.iter().map(|v| v * v).collect();
        for i in (0..e.len() - 1).rev() {
            e[i] += e[i + 1];
        }
        let e0 = e[0];
        let db = |i: usize| 10.0 * (e[i] / e0).max(1e-30).log10();
        let i5 = (0..e.len()).find(|&i| db(i) < -5.0).unwrap();
        let i35 = (0..e.len()).find(|&i| db(i) < -35.0).unwrap_or(e.len() - 1);
        (i35 - i5) as f64 / fs * 2.0
    }

    #[test]
    fn rooms_decay_at_their_reverberation_times() {
        let fs = 48000.0;
        let specs = spaces();
        for (k, name) in NAMES.iter().enumerate().take(7) {
            let s = specs[k];
            // far mic so the decay is the room's, not the direct sound's
            let h = ir(k as f32, 12.0, 0.0, 0.0, s.rt_live[1] * 2.5 + 0.5);
            let mid = t30(&h, fs, 800.0);
            let lo = t30(&h, fs, 150.0);
            let hi = t30(&h, fs, 4000.0);
            println!("{name:>18}: T30 150 Hz {lo:.2} s (target {:.2}), 800 Hz {mid:.2} s ({:.2}), 4 kHz {hi:.2} s ({:.2})", s.rt_live[0], s.rt_live[1], s.rt_live[2]);
            assert!((mid / s.rt_live[1] - 1.0).abs() < 0.2, "{name}: mid {mid} vs {}", s.rt_live[1]);
            assert!((lo / s.rt_live[0] - 1.0).abs() < 0.3, "{name}: low {lo} vs {}", s.rt_live[0]);
            assert!((hi / s.rt_live[2] - 1.0).abs() < 0.3, "{name}: high {hi} vs {}", s.rt_live[2]);
        }
    }

    #[test]
    fn screens_shorten_studio_two() {
        let fs = 48000.0;
        let live = t30(&ir(0.0, 10.0, 0.0, 0.0, 2.0), fs, 800.0);
        let damped = t30(&ir(0.0, 10.0, 0.0, 1.0, 2.0), fs, 800.0);
        println!("Studio Two: {live:.2} s screens back, {damped:.2} s screens out");
        assert!(damped < live * 0.8);
    }

    #[test]
    fn direct_to_reverberant_follows_distance() {
        // near the critical distance the direct and room energies are equal
        let fs = 48000.0;
        let spec = spaces()[0];
        let ac = acoustics(&spec, 0.0);
        let rc = 0.057 * (ac.volume / ac.rt[1]).sqrt();
        let ratio = |dist: f32| {
            let h = ir(0.0, dist, 0.0, 0.0, 2.0);
            let arrival = (dist as f64 / C * fs) as usize;
            let w = (0.001 * fs) as usize;
            let direct: f64 = h[arrival.saturating_sub(w)..arrival + w].iter().map(|v| v * v).sum();
            let rest: f64 = h[arrival + w..].iter().map(|v| v * v).sum();
            10.0 * (direct / rest).log10()
        };
        let near = ratio(1.0);
        let far = ratio(8.0);
        println!("critical distance {rc:.2} m; D/R at 1 m {near:.1} dB, at 8 m {far:.1} dB");
        // theory: 20 log(rc/r); the walls' early reflections add a little room
        assert!(near > far + 8.0);
        assert!(near > 0.0 && far < 0.0);
    }

    #[test]
    fn rooftop_is_mostly_dry() {
        let h = ir(7.0, 4.0, 1.0, 0.0, 1.5);
        let fs = 48000.0;
        let arrival = (4.0 / C * fs) as usize;
        let early: f64 = h[..arrival + 200].iter().map(|v| v * v).sum();
        let late: f64 = h[(0.12 * fs) as usize..].iter().map(|v| v * v).sum();
        let db = 10.0 * (late / early).log10();
        println!("rooftop late/early {db:.1} dB");
        assert!(db < -20.0);
    }

    #[test]
    fn stays_finite_and_switches_spaces() {
        let fs = 48000.0;
        let mut d = SpaceDsp::new(fs as f32, 2);
        for space in 0..8 {
            d.set_params(&[space as f32, 3.0, 1.0, 0.5, 1.0, 80.0, 0.5, 0.0]);
            let mut l: Vec<f32> = (0..9600).map(|k| ((k as f64 * 0.05).sin() * 0.5) as f32).collect();
            let mut r = l.clone();
            d.process(&mut [&mut l[..], &mut r[..]], &mut [0.0]);
            assert!(l.iter().chain(r.iter()).all(|v| v.is_finite() && v.abs() < 4.0));
        }
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    #[test]
    fn cpu() {
        let fs = 48000.0;
        for space in [0.0f32, 1.0, 6.0] {
            let mut d = SpaceDsp::new(fs as f32, 2);
            d.set_params(&[space, 6.0, 1.0, 0.0, 1.0, 20.0, 0.5, 0.0]);
            let mut l = vec![0.1f32; 48000];
            let mut r = l.clone();
            let t = std::time::Instant::now();
            d.process(&mut [&mut l[..], &mut r[..]], &mut [0.0]);
            println!("space {space}: {:.1}% of a core, taps {}", t.elapsed().as_secs_f64() * 100.0, d.taps[0].len() + d.taps[1].len());
        }
    }
}
