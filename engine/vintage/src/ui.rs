//! Shared vintage faceplate editor. Each plugin describes its panel as a
//! list of `Item`s; this module draws it and handles all interaction.

use crate::amp::ir::IrShared;
use crate::link::{GuiLink, HOLD_SECS};
use crate::plugin::{Meters, VintageParams};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, epaint::PathStroke, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Pos2, Rect,
    Sense, Shape, Stroke, StrokeKind, Vec2,
};
use nih_plug_egui::{create_egui_editor, EguiState};
use std::f32::consts::PI;
use std::sync::atomic::Ordering;
use std::sync::Arc;

// ---------------------------------------------------------------- palette --

pub const WOOD: Color32 = Color32::from_rgb(0x3a, 0x24, 0x16);
pub const WOOD_DARK: Color32 = Color32::from_rgb(0x24, 0x16, 0x0d);
pub const PLATE: Color32 = Color32::from_rgb(0xe8, 0xdd, 0xc4);
pub const PLATE_EDGE: Color32 = Color32::from_rgb(0xb9, 0xa9, 0x86);
pub const INK: Color32 = Color32::from_rgb(0x2b, 0x21, 0x18);
pub const INK_SOFT: Color32 = Color32::from_rgb(0x6b, 0x5a, 0x48);
pub const CREAM: Color32 = Color32::from_rgb(0xf6, 0xee, 0xdc);
pub const BAKELITE: Color32 = Color32::from_rgb(0x1c, 0x17, 0x14);
pub const BAKELITE_HI: Color32 = Color32::from_rgb(0x33, 0x2b, 0x25);
pub const METER_FACE: Color32 = Color32::from_rgb(0xf3, 0xe2, 0xa6);

/// Accent colours, one per plugin, so the suite reads as a family.
pub const OXBLOOD: Color32 = Color32::from_rgb(0x8e, 0x2a, 0x22);
pub const BOTTLE: Color32 = Color32::from_rgb(0x2f, 0x5a, 0x3e);
pub const NAVY: Color32 = Color32::from_rgb(0x24, 0x3b, 0x5e);
pub const MUSTARD: Color32 = Color32::from_rgb(0xa8, 0x7a, 0x1c);
pub const PLUM: Color32 = Color32::from_rgb(0x5e, 0x2c, 0x52);
pub const TEAL: Color32 = Color32::from_rgb(0x1f, 0x5f, 0x63);
pub const RUST: Color32 = Color32::from_rgb(0xa3, 0x4a, 0x1f);
pub const SLATE: Color32 = Color32::from_rgb(0x44, 0x4b, 0x52);
pub const INDIGO: Color32 = Color32::from_rgb(0x3b, 0x33, 0x6b);
// PEQUOD category colours (accent per kind of plugin)
pub const TANGERINE: Color32 = Color32::from_rgb(0xc2, 0x61, 0x1b); // drive, fuzz & boost
pub const OLIVE: Color32 = Color32::from_rgb(0x66, 0x66, 0x1c); // expanders, gates & levellers
pub const WALNUT: Color32 = Color32::from_rgb(0x6b, 0x4a, 0x2e); // tape & saturation
pub const STEEL: Color32 = Color32::from_rgb(0x46, 0x59, 0x6b); // consoles, preamps & strips
pub const COBALT: Color32 = Color32::from_rgb(0x2c, 0x4a, 0x9a); // EQ & filters
pub const OCEAN: Color32 = Color32::from_rgb(0x1e, 0x5a, 0x84); // delays & echoes
pub const PINE: Color32 = Color32::from_rgb(0x1f, 0x5e, 0x4a); // rooms, chambers & spaces
pub const VIOLET: Color32 = Color32::from_rgb(0x5c, 0x2e, 0x7a); // modulation
pub const MOSS: Color32 = Color32::from_rgb(0x3f, 0x6b, 0x2a); // synths, keys & instruments
pub const ROSE: Color32 = Color32::from_rgb(0x8c, 0x2f, 0x62); // lo-fi & character
pub const GRAPHITE: Color32 = Color32::from_rgb(0x2b, 0x2f, 0x33); // utilities

// ------------------------------------------------------------------ panel --

/// Standard panel size for the suite (the bottom strip holds the preset bar).
pub const PANEL: (u32, u32) = (640, 432);

pub struct Panel {
    pub title: &'static str,
    pub subtitle: (&'static str, &'static str),
    pub accent: Color32,
    pub size: (u32, u32),
    pub items: Vec<Item>,
    pub presets: Vec<Preset>,
}

/// A named starting point. Parameters not listed go back to their defaults.
/// Values are in plain units (dB, Hz, ms, 0..1 for percentages, variant
/// index for switches/selectors).
pub struct Preset {
    pub name: &'static str,
    pub values: &'static [(usize, f32)],
}

/// Floor plan of one room for [`Item::RoomPlan`]: length (along the
/// source-mic axis) and width in metres, source position in metres, and
/// whether it's open air (drawn without walls on three sides).
#[derive(Clone, Copy)]
pub struct Plan {
    pub length: f32,
    pub width: f32,
    pub src: (f32, f32),
    pub open_air: bool,
    /// Short note drawn along the far wall (e.g. "control room window").
    pub far_wall: &'static str,
}

/// Where the microphone sits for a given distance: straight down the room
/// from the source, kept 0.3 m clear of the far wall. Shared by the plan
/// drawing and the DSP so they agree.
pub fn mic_position(plan_len: f32, src_x: f32, distance: f32) -> f32 {
    (src_x + distance).min(plan_len - 0.3).max(src_x + 0.05)
}

/// A knob is shown faded (it still works) unless param `p`'s normalized
/// value lies within `min..=max`.
#[derive(Clone, Copy)]
pub struct ActiveWhen {
    pub p: usize,
    pub min: f32,
    pub max: f32,
}

#[derive(Clone, Copy)]
pub enum MeterKind {
    /// value = gain reduction in dB (>= 0); scale 0..20 dB
    GainReduction,
    /// value = linear peak level; scale -20..+3 "VU" with 0 VU = -18 dBFS
    Vu,
}

pub enum Item {
    Knob {
        p: usize,
        x: f32,
        y: f32,
        r: f32,
        label: &'static str,
        tip: &'static str,
        active: Option<ActiveWhen>,
    },
    /// Two-position lever for a 2-step parameter.
    Switch {
        p: usize,
        x: f32,
        y: f32,
        labels: [&'static str; 2],
        tip: &'static str,
    },
    /// Chicken-head rotary selector for a stepped parameter.
    Selector {
        p: usize,
        x: f32,
        y: f32,
        label: &'static str,
        positions: &'static [&'static str],
        tip: &'static str,
    },
    Meter {
        m: usize,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        kind: MeterKind,
        caption: &'static str,
    },
    /// Indicator lamp; brightness = (meter - lo) / (hi - lo).
    Lamp {
        m: usize,
        x: f32,
        y: f32,
        label: &'static str,
        lo: f32,
        hi: f32,
    },
    /// Top view of a rotating speaker: meters `horn` and `drum` are phases 0..1.
    Rotor {
        horn: usize,
        drum: usize,
        x: f32,
        y: f32,
        r: f32,
    },
    Divider {
        y: f32,
    },
    Text {
        x: f32,
        y: f32,
        text: &'static str,
        size: f32,
        align: Align2,
    },
    /// A column of radio buttons for a stepped parameter (one row per step).
    RadioList {
        p: usize,
        x: f32,
        y: f32,
        w: f32,
        row_h: f32,
        names: &'static [&'static str],
        tip: &'static str,
    },
    /// Text that follows a stepped parameter: `texts[step]`, wrapped to `w`.
    Caption {
        p: usize,
        x: f32,
        y: f32,
        w: f32,
        size: f32,
        texts: &'static [&'static str],
    },
    /// Top-down plan of a room with the source and microphone(s), driven by
    /// a room selector, a distance parameter (metres) and a mic-spacing
    /// parameter (metres).
    RoomPlan {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        room: usize,
        distance: usize,
        spacing: usize,
        plans: &'static [Plan],
    },
    /// Polar pattern of a microphone with the source direction. Meter
    /// `pattern` reports the pattern in use (1 omni, 2 cardioid, 3 figure-8);
    /// param `angle` is the source angle in degrees.
    Polar {
        x: f32,
        y: f32,
        r: f32,
        pattern: usize,
        angle: usize,
    },
    /// Impulse-response loader: button, file name, clear. `cab` is the cab
    /// selector parameter and `ir_pos` its "IR" position (variant index), so
    /// the panel can say when the IR is selected but nothing is loaded.
    IrLoader {
        x: f32,
        y: f32,
        w: f32,
        cab: usize,
        ir_pos: usize,
    },
    /// Tuner display: meter `freq` = detected Hz (0 = none), meter `level` =
    /// input peak, param `reference` = A4 in Hz.
    Tuner {
        freq: usize,
        level: usize,
        reference: usize,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
}

pub fn editor_state(panel_size: (u32, u32)) -> Arc<EguiState> {
    EguiState::from_size(panel_size.0, panel_size.1)
}

// -------------------------------------------------------------- ui state --

struct UiState {
    init: bool,
    last_param: Vec<f32>,
    last_edit: Vec<f64>,
    edited: bool,
    meter_disp: Vec<f32>,
    preset_open: bool,
    preset_sel: Option<usize>,
    preset_dirty: bool,
    tuner_seen: f64,
    tuner_level: f32,
}

struct Ctl<'a> {
    setter: &'a ParamSetter<'a>,
    link: &'a GuiLink,
    ptrs: &'a [ParamPtr],
    now: f64,
    accent: Color32,
}

impl Ctl<'_> {
    fn begin(&self, i: usize) {
        unsafe { self.setter.raw_context.raw_begin_set_parameter(self.ptrs[i]) }
    }
    fn end(&self, i: usize) {
        unsafe { self.setter.raw_context.raw_end_set_parameter(self.ptrs[i]) }
    }
    /// Tell the host AND apply directly (see `link`).
    fn set(&self, state: &mut UiState, i: usize, norm: f32) {
        unsafe { self.setter.raw_context.raw_set_parameter_normalized(self.ptrs[i], norm) }
        self.link.gui_set(i, norm);
        state.last_edit[i] = self.now;
        state.edited = true;
    }
    fn value(&self, i: usize) -> f32 {
        self.link.effective(i)
    }
    fn text(&self, i: usize, norm: f32) -> String {
        unsafe { self.ptrs[i].normalized_value_to_string(norm, true) }
    }
    fn default(&self, i: usize) -> f32 {
        unsafe { self.ptrs[i].default_normalized_value() }
    }
    fn steps(&self, i: usize) -> usize {
        unsafe { self.ptrs[i].step_count() }.unwrap_or(1).max(1)
    }
}

// ----------------------------------------------------------------- editor --

pub fn create_editor<P: VintageParams>(
    params: Arc<P>,
    link: Arc<GuiLink>,
    meters: Meters,
    ir: Arc<IrShared>,
    panel: Panel,
) -> Option<Box<dyn Editor>> {
    let n = link.len();
    let state = UiState {
        init: false,
        last_param: vec![0.0; n],
        last_edit: vec![f64::NEG_INFINITY; n],
        edited: false,
        meter_disp: vec![0.0; meters.len()],
        preset_open: false,
        preset_sel: None,
        preset_dirty: false,
        tuner_seen: f64::NEG_INFINITY,
        tuner_level: 0.0,
    };
    let ptrs = params.linked();
    create_egui_editor(params.editor_state(), state, |_, _| {}, move |ctx, setter, state| {
        let now = ctx.input(|i| i.time);
        let dt = ctx.input(|i| i.stable_dt).clamp(0.001, 0.1);
        ir.collect_garbage();

        // host-side changes (automation, host UI, preset load)
        for i in 0..n {
            let pn = unsafe { ptrs[i].unmodulated_normalized_value() };
            if !state.init {
                state.last_param[i] = pn;
            } else if pn != state.last_param[i] {
                state.last_param[i] = pn;
                if now - state.last_edit[i] > HOLD_SECS as f64 {
                    link.set_effective(i, pn);
                }
            }
        }
        state.init = true;
        state.edited = false;

        // meters: read + reset the max-hold values, then apply ballistics
        let mut raw = vec![0.0f32; meters.len()];
        for (k, m) in meters.iter().enumerate() {
            raw[k] = m.load(Ordering::Relaxed);
            m.store(0.0, Ordering::Relaxed);
        }
        for item in &panel.items {
            match item {
                Item::Meter { m, kind, .. } => {
                    let target = match kind {
                        MeterKind::GainReduction => raw[*m],
                        MeterKind::Vu => vu_db(raw[*m]),
                    };
                    let up = matches!(kind, MeterKind::GainReduction) == (target > state.meter_disp[*m]);
                    let tau = match kind {
                        MeterKind::GainReduction => if target > state.meter_disp[*m] { 0.03 } else { 0.35 },
                        MeterKind::Vu => 0.3,
                    };
                    let _ = up;
                    let k = 1.0 - (-dt / tau).exp();
                    state.meter_disp[*m] += (target - state.meter_disp[*m]) * k;
                }
                Item::Lamp { m, lo, hi, .. } => {
                    let target = ((raw[*m] - lo) / (hi - lo)).clamp(0.0, 1.0);
                    let tau = if target > state.meter_disp[*m] { 0.02 } else { 0.25 };
                    let k = 1.0 - (-dt / tau).exp();
                    state.meter_disp[*m] += (target - state.meter_disp[*m]) * k;
                }
                Item::Tuner { freq, level, .. } => {
                    let f = raw[*freq];
                    state.tuner_level = raw[*level].max(state.tuner_level * 0.9);
                    if f > 0.0 {
                        let cur = state.meter_disp[*freq];
                        // jump straight to a new note, glide within the same one
                        if cur <= 0.0 || (f / cur - 1.0).abs() > 0.03 {
                            state.meter_disp[*freq] = f;
                        } else {
                            let k = 1.0 - (-dt / 0.08).exp();
                            state.meter_disp[*freq] = cur * (f / cur).powf(k);
                        }
                        state.tuner_seen = now;
                    } else if now - state.tuner_seen > 1.5 {
                        state.meter_disp[*freq] = 0.0;
                    }
                }
                Item::Polar { pattern, .. } => {
                    if raw[*pattern] > 0.0 {
                        state.meter_disp[*pattern] = raw[*pattern];
                    }
                }
                Item::Rotor { horn, drum, .. } => {
                    // phases: take the latest value when one arrived
                    for idx in [*horn, *drum] {
                        if raw[idx] > 0.0 {
                            state.meter_disp[idx] = raw[idx];
                        }
                    }
                }
                _ => {}
            }
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(WOOD))
            .show(ctx, |ui| {
                let full = ui.max_rect();
                let p = ui.painter().clone();
                let o = full.min.to_vec2();
                let at = |x: f32, y: f32| Pos2::new(x, y) + o;
                let (w, h) = (panel.size.0 as f32, panel.size.1 as f32);

                faceplate(&p, full, at, w, h, &panel);

                let ctl = Ctl { setter, link: &link, ptrs: &ptrs, now, accent: panel.accent };
                for item in &panel.items {
                    match item {
                        Item::Knob { p: i, x, y, r, label, tip, active } => {
                            let dim = active
                                .map(|a| {
                                    let v = ctl.value(a.p);
                                    !(v >= a.min - 1e-4 && v <= a.max + 1e-4)
                                })
                                .unwrap_or(false);
                            knob(ui, &p, &ctl, state, *i, at(*x, *y), *r, label, tip, dim);
                        }
                        Item::Switch { p: i, x, y, labels, tip } => {
                            switch(ui, &p, &ctl, state, *i, at(*x, *y), labels, tip);
                        }
                        Item::Selector { p: i, x, y, label, positions, tip } => {
                            selector(ui, &p, &ctl, state, *i, at(*x, *y), label, positions, tip);
                        }
                        Item::Meter { m, x, y, w, h, kind, caption } => {
                            meter(&p, Rect::from_min_size(at(*x, *y), Vec2::new(*w, *h)), *kind, state.meter_disp[*m], caption, panel.accent);
                        }
                        Item::Lamp { m, x, y, label, .. } => {
                            lamp(&p, at(*x, *y), state.meter_disp[*m]);
                            p.text(at(*x + 14.0, *y), Align2::LEFT_CENTER, *label, FontId::proportional(12.0), INK);
                        }
                        Item::Rotor { horn, drum, x, y, r } => {
                            rotor(&p, at(*x, *y), *r, state.meter_disp[*horn], state.meter_disp[*drum], panel.accent);
                        }
                        Item::Divider { y } => {
                            p.line_segment([at(40.0, *y), at(w - 40.0, *y)], Stroke::new(1.0_f32, PLATE_EDGE));
                        }
                        Item::Text { x, y, text, size, align } => {
                            p.text(at(*x, *y), *align, *text, FontId::proportional(*size), INK);
                        }
                        Item::RadioList { p: i, x, y, w, row_h, names, tip } => {
                            radio_list(ui, &p, &ctl, state, *i, at(*x, *y), *w, *row_h, names, tip);
                        }
                        Item::Caption { p: i, x, y, w, size, texts } => {
                            let steps = ctl.steps(*i) as f32;
                            let k = ((ctl.value(*i) * steps).round() as usize).min(texts.len().saturating_sub(1));
                            if let Some(t) = texts.get(k) {
                                let galley = p.layout(t.to_string(), FontId::proportional(*size), INK, *w);
                                p.galley(at(*x, *y), galley, INK);
                            }
                        }
                        Item::RoomPlan { x, y, w, h, room, distance, spacing, plans } => {
                            let steps = ctl.steps(*room) as f32;
                            let k = ((ctl.value(*room) * steps).round() as usize).min(plans.len() - 1);
                            let d = unsafe { ptrs[*distance].preview_plain(link.effective(*distance)) };
                            let sp = unsafe { ptrs[*spacing].preview_plain(link.effective(*spacing)) };
                            room_plan(&p, Rect::from_min_size(at(*x, *y), Vec2::new(*w, *h)), &plans[k], d, sp, panel.accent);
                        }
                        Item::Polar { x, y, r, pattern, angle } => {
                            let a = unsafe { ptrs[*angle].preview_plain(link.effective(*angle)) };
                            polar_plot(&p, at(*x, *y), *r, state.meter_disp[*pattern], a, panel.accent);
                        }
                        Item::IrLoader { x, y, w, cab, ir_pos } => {
                            let steps = ctl.steps(*cab) as f32;
                            let selected = (ctl.value(*cab) * steps).round() as usize == *ir_pos;
                            ir_loader(ui, &p, &ir, params.ir_path(), at(*x, *y), *w, selected, panel.accent);
                        }
                        Item::Tuner { freq, reference, x, y, w, h, .. } => {
                            let a4 = unsafe { ptrs[*reference].preview_plain(link.effective(*reference)) };
                            tuner(&p, Rect::from_min_size(at(*x, *y), Vec2::new(*w, *h)), state.meter_disp[*freq], a4, state.tuner_level, panel.accent);
                        }
                    }
                }

                p.text(at(38.0, h - 36.0), Align2::LEFT_CENTER, "[PEQUOD]", FontId::monospace(12.0), Color32::from_rgb(0xa8, 0x82, 0x3a));
                if !panel.presets.is_empty() {
                    let applied = preset_bar(ui, &p, &ctl, state, &panel, at(w / 2.0, h - 36.0));
                    if state.edited && !applied {
                        state.preset_dirty = true;
                    }
                }
            });

        if state.edited {
            if let Ok(mut wr) = params.saved().write() {
                *wr = link.effective_all();
            }
        }
        ctx.request_repaint();
    })
}

#[allow(clippy::too_many_arguments)]
fn radio_list(
    ui: &mut egui::Ui,
    p: &egui::Painter,
    ctl: &Ctl,
    state: &mut UiState,
    i: usize,
    c: Pos2,
    w: f32,
    row_h: f32,
    names: &[&str],
    tip: &str,
) {
    let steps = ctl.steps(i);
    let current = ((ctl.value(i) * steps as f32).round() as usize).min(steps);
    for (k, name) in names.iter().enumerate() {
        let r = Rect::from_min_size(c + Vec2::new(0.0, k as f32 * row_h), Vec2::new(w, row_h - 3.0));
        let resp = ui
            .interact(r, Id::new(("radio", i, k)), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(tip);
        if resp.clicked() && k != current {
            ctl.begin(i);
            ctl.set(state, i, k as f32 / steps.max(1) as f32);
            ctl.end(i);
        }
        let on = k == current;
        if on {
            p.rect_filled(r, CornerRadius::same(4), ctl.accent);
        } else if resp.hovered() {
            p.rect_filled(r, CornerRadius::same(4), lerp_color(PLATE, PLATE_EDGE, 0.35));
        }
        let lc = r.left_center() + Vec2::new(12.0, 0.0);
        p.circle_stroke(lc, 4.5, Stroke::new(1.2_f32, if on { CREAM } else { INK_SOFT }));
        if on {
            p.circle_filled(lc, 2.6, CREAM);
        }
        p.text(lc + Vec2::new(12.0, 0.0), Align2::LEFT_CENTER, *name, FontId::proportional(12.5), if on { CREAM } else { INK });
    }
}

fn polar_plot(p: &egui::Painter, c: Pos2, r: f32, pattern: f32, angle_deg: f32, accent: Color32) {
    let k = match pattern.round() as i32 {
        1 => 0.0,
        3 => 1.0,
        _ => 0.5,
    };
    for f in [0.25f32, 0.5, 0.75, 1.0] {
        p.circle_stroke(c, r * f, Stroke::new(if f == 1.0 { 1.2_f32 } else { 0.7 }, PLATE_EDGE));
    }
    p.line_segment([c - Vec2::new(r, 0.0), c + Vec2::new(r, 0.0)], Stroke::new(0.7_f32, PLATE_EDGE));
    p.line_segment([c - Vec2::new(0.0, r), c + Vec2::new(0.0, r)], Stroke::new(0.7_f32, PLATE_EDGE));
    // pattern: 0 deg (front) points up
    let n = 120;
    let mut front = Vec::new();
    let mut back = Vec::new();
    for i in 0..=n {
        let th = i as f32 / n as f32 * 2.0 * PI;
        let g = (1.0 - k) + k * th.cos();
        let pt = Pos2::new(c.x + r * g.abs() * th.sin(), c.y - r * g.abs() * th.cos());
        if g >= 0.0 { front.push(pt) } else { back.push(pt) }
        if g < 0.0 && !front.is_empty() && back.len() == 1 {
            // keep lobes as separate lines
        }
    }
    p.add(Shape::line(front, PathStroke::new(2.2_f32, accent)));
    if back.len() > 1 {
        p.add(Shape::line(back, PathStroke::new(1.6_f32, lerp_color(accent, PLATE, 0.45))));
    }
    // the source
    let a = angle_deg.to_radians();
    let s = Pos2::new(c.x + (r + 14.0) * a.sin(), c.y - (r + 14.0) * a.cos());
    p.line_segment([c, s], Stroke::new(1.0_f32, INK_SOFT));
    p.circle_filled(s, 6.0, INK);
    p.text(s + Vec2::new(10.0, 0.0), Align2::LEFT_CENTER, format!("source {angle_deg:.0}°"), FontId::proportional(9.5), INK_SOFT);
    p.circle_filled(c, 3.0, INK);
    let name = match pattern.round() as i32 {
        1 => "OMNI",
        3 => "FIGURE-8",
        _ => "CARDIOID",
    };
    p.text(c + Vec2::new(0.0, r + 12.0), Align2::CENTER_CENTER, name, FontId::proportional(10.0), INK);
}

fn room_plan(p: &egui::Painter, rect: Rect, plan: &Plan, distance: f32, spacing: f32, accent: Color32) {
    p.rect_filled(rect, CornerRadius::same(6), Color32::from_rgb(0xdc, 0xd0, 0xb3));
    let pad = 16.0;
    let inner = rect.shrink(pad);
    let scale = (inner.width() / plan.length).min(inner.height() / plan.width);
    let size = Vec2::new(plan.length * scale, plan.width * scale);
    let room = Rect::from_center_size(inner.center(), size);
    let to = |x: f32, y: f32| Pos2::new(room.left() + x * scale, room.top() + y * scale);
    let wall = Stroke::new(3.0_f32, INK);
    if plan.open_air {
        p.rect_filled(room, CornerRadius::ZERO, Color32::from_rgb(0xc9, 0xbc, 0x9c));
        // the building across the street, beyond the far end of the roof
        p.line_segment([room.right_top(), room.right_bottom()], Stroke::new(1.5_f32, INK_SOFT));
        for k in 0..6 {
            let y = room.top() + size.y * (k as f32 + 0.5) / 6.0;
            p.line_segment([Pos2::new(room.right() + 4.0, y - 4.0), Pos2::new(room.right() + 4.0, y + 4.0)], Stroke::new(3.0_f32, INK_SOFT));
        }
    } else {
        p.rect_filled(room, CornerRadius::ZERO, Color32::from_rgb(0xe9, 0xdf, 0xc8));
        p.rect_stroke(room, CornerRadius::ZERO, wall, StrokeKind::Outside);
    }
    if !plan.far_wall.is_empty() {
        let tp = Pos2::new(room.right() - 4.0, room.center().y);
        p.text(tp, Align2::RIGHT_CENTER, plan.far_wall, FontId::proportional(9.5), INK_SOFT);
    }
    // scale bar: 5 m
    let bar = 5.0 * scale;
    let b0 = Pos2::new(rect.left() + 10.0, rect.bottom() - 7.0);
    p.line_segment([b0, b0 + Vec2::new(bar, 0.0)], Stroke::new(1.5_f32, INK_SOFT));
    p.text(b0 + Vec2::new(bar + 4.0, 0.0), Align2::LEFT_CENTER, "5 m", FontId::proportional(9.5), INK_SOFT);
    // dimensions
    p.text(Pos2::new(rect.right() - 8.0, rect.bottom() - 7.0), Align2::RIGHT_CENTER,
        format!("{:.1} x {:.1} m", plan.length, plan.width), FontId::proportional(9.5), INK_SOFT);

    let s = to(plan.src.0, plan.src.1);
    let mx = mic_position(plan.length, plan.src.0, distance);
    let half = 0.5 * spacing;
    let mics: Vec<Pos2> = if half > 0.02 {
        vec![to(mx, plan.src.1 - half), to(mx, plan.src.1 + half)]
    } else {
        vec![to(mx, plan.src.1)]
    };
    for m in &mics {
        p.line_segment([s, *m], Stroke::new(1.0_f32, lerp_color(INK_SOFT, PLATE, 0.4)));
    }
    p.circle_filled(s, 7.0, accent);
    p.text(s, Align2::CENTER_CENTER, "S", FontId::proportional(9.0), CREAM);
    for m in &mics {
        p.circle_filled(*m, 5.5, INK);
        p.circle_filled(*m, 2.0, CREAM);
    }
    p.text(s + Vec2::new(0.0, 14.0), Align2::CENTER_CENTER, "band", FontId::proportional(9.5), INK_SOFT);
    p.text(mics[0] + Vec2::new(0.0, -12.0), Align2::CENTER_CENTER, format!("mic {distance:.1} m"), FontId::proportional(9.5), INK);
}

#[allow(clippy::too_many_arguments)]
fn ir_loader(
    ui: &mut egui::Ui,
    p: &egui::Painter,
    ir: &IrShared,
    path: Option<Arc<std::sync::RwLock<String>>>,
    c: Pos2,
    w: f32,
    selected: bool,
    accent: Color32,
) {
    // [ LOAD IR... ]  name.wav (120 ms)  [x]
    let btn = Rect::from_min_size(c, Vec2::new(84.0, 24.0));
    let resp = ui.interact(btn, Id::new("ir_load"), Sense::click()).on_hover_text(
        "Load a cabinet impulse response (WAV, any sample rate; the first 250 ms is used). \
         Select IR on the CAB switch to hear it.",
    );
    let hot = resp.hovered();
    p.rect_filled(btn, CornerRadius::same(4), if hot { lerp_color(accent, CREAM, 0.15) } else { accent });
    p.text(btn.center(), Align2::CENTER_CENTER, "LOAD IR...", FontId::proportional(12.0), CREAM);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    if resp.clicked() {
        if let Some(file) = pick_wav() {
            if ir.load_file(&file) {
                if let Some(path) = path.as_ref() {
                    if let Ok(mut w) = path.write() {
                        *w = file.to_string_lossy().to_string();
                    }
                }
            }
        }
    }
    let status = ir.status.read().map(|s| s.clone()).unwrap_or_default();
    let loaded = ir.loaded.load(Ordering::Relaxed) || !status.is_empty() && !status.starts_with("IR not");
    let text = if status.is_empty() {
        if selected { "no IR loaded: using the built-in cabinet".to_string() } else { "no IR loaded".to_string() }
    } else {
        status.clone()
    };
    let tx = c.x + 96.0;
    let max_w = w - 96.0 - 30.0;
    let mut shown = text.clone();
    while shown.chars().count() > 4
        && p.layout_no_wrap(shown.clone(), FontId::proportional(12.0), INK).size().x > max_w
    {
        let cut: String = shown.chars().take(shown.chars().count() - 2).collect();
        shown = format!("{}…", cut.trim_end_matches('…'));
    }
    p.text(Pos2::new(tx, c.y + 12.0), Align2::LEFT_CENTER, shown, FontId::proportional(12.0), if loaded { INK } else { INK_SOFT });
    if loaded {
        let xr = Rect::from_center_size(Pos2::new(c.x + w - 10.0, c.y + 12.0), Vec2::splat(18.0));
        let r = ui.interact(xr, Id::new("ir_clear"), Sense::click()).on_hover_text("Unload the IR");
        p.circle_stroke(xr.center(), 8.0, Stroke::new(1.2_f32, if r.hovered() { INK } else { INK_SOFT }));
        let d = 3.5;
        for s in [-1.0, 1.0] {
            p.line_segment([xr.center() + Vec2::new(-d, -d * s), xr.center() + Vec2::new(d, d * s)], Stroke::new(1.4_f32, INK_SOFT));
        }
        if r.clicked() {
            ir.unload();
            if let Some(path) = path.as_ref() {
                if let Ok(mut w) = path.write() {
                    w.clear();
                }
            }
        }
    }
}

#[cfg(windows)]
fn pick_wav() -> Option<std::path::PathBuf> {
    rfd::FileDialog::new().set_title("Load cabinet impulse response").add_filter("WAV audio", &["wav", "WAV"]).pick_file()
}

#[cfg(not(windows))]
fn pick_wav() -> Option<std::path::PathBuf> {
    // the file dialog is wired up for the Windows build; elsewhere a path
    // can be given in this environment variable
    std::env::var_os("STUDIO_TWO_IR").map(std::path::PathBuf::from)
}

fn vu_db(peak: f32) -> f32 {
    // 0 VU = -18 dBFS; display range -20..+3
    (20.0 * peak.max(1e-6).log10() + 18.0).clamp(-25.0, 4.0)
}

// -------------------------------------------------------------- drawing --

fn faceplate(p: &egui::Painter, full: Rect, at: impl Fn(f32, f32) -> Pos2, w: f32, h: f32, panel: &Panel) {
    let mut i = 0;
    while (i as f32) * 29.0 < h {
        let y = full.top() + 6.0 + i as f32 * 29.0;
        p.line_segment([Pos2::new(full.left(), y), Pos2::new(full.right(), y + 3.0)], Stroke::new(1.0_f32, WOOD_DARK));
        i += 1;
    }
    let plate = Rect::from_min_max(at(14.0, 14.0), at(w - 14.0, h - 14.0));
    p.rect_filled(plate.translate(Vec2::new(0.0, 3.0)), CornerRadius::same(10), Color32::from_black_alpha(90));
    p.rect_filled(plate, CornerRadius::same(10), PLATE);
    p.rect_stroke(plate, CornerRadius::same(10), Stroke::new(2.0_f32, PLATE_EDGE), StrokeKind::Inside);

    let band = Rect::from_min_max(at(14.0, 14.0), at(w - 14.0, 70.0));
    p.rect_filled(band, CornerRadius { nw: 10, ne: 10, sw: 0, se: 0 }, panel.accent);
    p.text(at(44.0, 42.0), Align2::LEFT_CENTER, panel.title, FontId::proportional(30.0), CREAM);
    p.text(at(w - 44.0, 36.0), Align2::RIGHT_CENTER, panel.subtitle.0, FontId::proportional(12.0), CREAM);
    p.text(at(w - 44.0, 52.0), Align2::RIGHT_CENTER, panel.subtitle.1, FontId::proportional(12.0), CREAM);

    for (x, y) in [(28.0, 84.0), (w - 28.0, 84.0), (28.0, h - 28.0), (w - 28.0, h - 28.0)] {
        let c = at(x, y);
        p.circle_filled(c, 6.0, PLATE_EDGE);
        p.circle_filled(c, 4.8, Color32::from_rgb(0xcf, 0xc4, 0xab));
        p.line_segment([c - Vec2::new(3.5, 1.0), c + Vec2::new(3.5, 1.0)], Stroke::new(1.4_f32, INK_SOFT));
    }
}

fn angle(v: f32) -> f32 {
    (-135.0 + 270.0 * v.clamp(0.0, 1.0)) * PI / 180.0
}

fn polar(c: Pos2, r: f32, a: f32) -> Pos2 {
    Pos2::new(c.x + r * a.sin(), c.y - r * a.cos())
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

#[allow(clippy::too_many_arguments)]
fn knob(
    ui: &mut egui::Ui,
    p: &egui::Painter,
    ctl: &Ctl,
    state: &mut UiState,
    i: usize,
    c: Pos2,
    r: f32,
    label: &str,
    tip: &str,
    dim: bool,
) {
    let big = r > 30.0;
    let rect = Rect::from_center_size(c, Vec2::splat(r * 2.0 + 16.0));
    let id = Id::new(("knob", i));
    let resp = ui
        .interact(rect, id, Sense::click_and_drag())
        .on_hover_cursor(CursorIcon::ResizeVertical)
        .on_hover_text(format!("{tip}\n\nDrag up/down (Shift = fine), scroll, double-click to reset."));

    let current = ctl.value(i);
    if resp.drag_started() {
        ctl.begin(i);
        ui.data_mut(|d| d.insert_temp(id, current));
    }
    if resp.dragged() {
        let fine = ui.input(|inp| inp.modifiers.shift);
        let sens = if fine { 0.0012 } else { 0.005 };
        let start: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(current);
        let v = (start - resp.drag_delta().y * sens).clamp(0.0, 1.0);
        ui.data_mut(|d| d.insert_temp(id, v));
        ctl.set(state, i, v);
    }
    if resp.drag_stopped() {
        ctl.end(i);
    }
    if resp.double_clicked() {
        ctl.begin(i);
        ctl.set(state, i, ctl.default(i));
        ctl.end(i);
    }
    if resp.hovered() && !resp.dragged() {
        let scroll = ui.input(|inp| if inp.raw_scroll_delta.y != 0.0 { inp.raw_scroll_delta.y } else { inp.smooth_scroll_delta.y });
        if scroll != 0.0 {
            let fine = ui.input(|inp| inp.modifiers.shift);
            let v = (current + scroll * if fine { 0.0003 } else { 0.001 }).clamp(0.0, 1.0);
            ctl.begin(i);
            ctl.set(state, i, v);
            ctl.end(i);
        }
    }

    let fade = |col: Color32| if dim { lerp_color(col, PLATE, 0.6) } else { col };
    let (ink, ink_soft, accent, bake, bake_hi, cream) =
        (fade(INK), fade(INK_SOFT), fade(ctl.accent), fade(BAKELITE), fade(BAKELITE_HI), fade(CREAM));
    let v = ctl.value(i);

    let ticks = if big { 11 } else { 7 };
    for t in 0..ticks {
        let a = angle(t as f32 / (ticks - 1) as f32);
        let long = t == 0 || t == ticks - 1 || (big && t == ticks / 2);
        p.line_segment(
            [polar(c, r + 5.0, a), polar(c, r + if long { 12.0 } else { 9.0 }, a)],
            Stroke::new(if long { 2.0_f32 } else { 1.2_f32 }, ink),
        );
    }
    let a0 = angle(0.0);
    let a1 = angle(v);
    if v > 0.001 {
        let arc: Vec<Pos2> = (0..=40).map(|k| polar(c, r + 2.5, a0 + (a1 - a0) * k as f32 / 40.0)).collect();
        p.add(Shape::line(arc, PathStroke::new(3.0_f32, accent)));
    }
    p.circle_filled(c + Vec2::new(0.0, 3.0), r, Color32::from_black_alpha(if dim { 25 } else { 80 }));
    p.circle_filled(c, r, bake);
    p.circle_filled(c, r * 0.82, bake_hi);
    p.circle_stroke(c, r * 0.82, Stroke::new(1.0_f32, bake));
    for k in 0..24 {
        let a = k as f32 * PI / 12.0 + a1;
        p.line_segment([polar(c, r * 0.86, a), polar(c, r * 0.98, a)], Stroke::new(1.0_f32, bake_hi));
    }
    p.line_segment([polar(c, r * 0.2, a1), polar(c, r * 0.78, a1)], Stroke::new(if big { 4.0_f32 } else { 3.0_f32 }, cream));
    if resp.hovered() || resp.dragged() {
        p.circle_stroke(c, r + 1.0, Stroke::new(1.5_f32, ctl.accent));
    }
    let (lf, vf) = if big { (16.0, 13.0) } else { (11.0, 11.0) };
    p.text(c - Vec2::new(0.0, r + if big { 22.0 } else { 20.0 }), Align2::CENTER_CENTER, label, FontId::proportional(lf), ink);
    p.text(c + Vec2::new(0.0, r + if big { 22.0 } else { 18.0 }), Align2::CENTER_CENTER, ctl.text(i, v), FontId::proportional(vf), ink_soft);
}

#[allow(clippy::too_many_arguments)]
fn switch(ui: &mut egui::Ui, p: &egui::Painter, ctl: &Ctl, state: &mut UiState, i: usize, c: Pos2, labels: &[&str; 2], tip: &str) {
    let on = ctl.value(i) > 0.5;
    let pill = Rect::from_center_size(c, Vec2::new(34.0, 16.0));
    let hit = Rect::from_min_max(Pos2::new(c.x - 62.0, c.y - 11.0), Pos2::new(c.x + 62.0, c.y + 11.0));
    let resp = ui
        .interact(hit, Id::new(("switch", i)), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(format!("{tip}\n\nClick to switch."));
    if resp.clicked() {
        ctl.begin(i);
        ctl.set(state, i, if on { 0.0 } else { 1.0 });
        ctl.end(i);
    }
    let on = ctl.value(i) > 0.5;
    let (lc, rc) = if on { (INK_SOFT, INK) } else { (INK, INK_SOFT) };
    p.text(Pos2::new(pill.left() - 6.0, c.y), Align2::RIGHT_CENTER, labels[0], FontId::proportional(11.0), lc);
    p.text(Pos2::new(pill.right() + 6.0, c.y), Align2::LEFT_CENTER, labels[1], FontId::proportional(11.0), rc);
    p.rect_filled(pill, CornerRadius::same(8), BAKELITE);
    if on {
        p.rect_filled(pill.shrink(2.0), CornerRadius::same(6), ctl.accent);
    }
    let kc = Pos2::new(if on { pill.right() - 8.0 } else { pill.left() + 8.0 }, c.y);
    p.circle_filled(kc + Vec2::new(0.0, 1.0), 6.5, Color32::from_black_alpha(90));
    p.circle_filled(kc, 6.0, CREAM);
    if resp.hovered() {
        p.rect_stroke(pill.expand(2.0), CornerRadius::same(9), Stroke::new(1.2_f32, ctl.accent), StrokeKind::Outside);
    }
}

#[allow(clippy::too_many_arguments)]
fn selector(
    ui: &mut egui::Ui,
    p: &egui::Painter,
    ctl: &Ctl,
    state: &mut UiState,
    i: usize,
    c: Pos2,
    label: &str,
    positions: &[&str],
    tip: &str,
) {
    let steps = ctl.steps(i);
    let n = steps + 1;
    let r = 20.0;
    let idx_of = |v: f32| ((v * steps as f32).round() as usize).min(steps);
    let current = idx_of(ctl.value(i));
    // positions spread over a 180° arc above the knob
    let pos_angle = |k: usize| {
        let span = if n <= 3 { 100.0 } else if n == 4 { 150.0 } else { 200.0 };
        ((-span / 2.0) + span * k as f32 / (n - 1).max(1) as f32) * PI / 180.0
    };

    let rect = Rect::from_center_size(c, Vec2::splat(r * 2.0 + 18.0));
    let id = Id::new(("selector", i));
    let resp = ui
        .interact(rect, id, Sense::click_and_drag())
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(format!("{tip}\n\nClick to step, drag or scroll to choose."));
    let set_idx = |state: &mut UiState, k: usize| {
        ctl.begin(i);
        ctl.set(state, i, k as f32 / steps as f32);
        ctl.end(i);
    };
    if resp.clicked() {
        set_idx(state, (current + 1) % n);
    }
    if resp.drag_started() {
        ui.data_mut(|d| d.insert_temp(id, 0.0f32));
    }
    if resp.dragged() {
        let mut acc: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(0.0);
        acc += resp.drag_delta().x - resp.drag_delta().y;
        if acc.abs() > 18.0 {
            let k = if acc > 0.0 { (current + 1).min(steps) } else { current.saturating_sub(1) };
            if k != current {
                set_idx(state, k);
            }
            acc = 0.0;
        }
        ui.data_mut(|d| d.insert_temp(id, acc));
    }
    if resp.hovered() && !resp.dragged() {
        let scroll = ui.input(|inp| inp.raw_scroll_delta.y);
        if scroll != 0.0 {
            let k = if scroll > 0.0 { (current + 1).min(steps) } else { current.saturating_sub(1) };
            if k != current {
                set_idx(state, k);
            }
        }
    }
    let current = idx_of(ctl.value(i));

    // position labels
    for k in 0..n {
        let a = pos_angle(k);
        let tick_in = polar(c, r + 4.0, a);
        let tick_out = polar(c, r + 9.0, a);
        p.line_segment([tick_in, tick_out], Stroke::new(1.5_f32, INK));
        if let Some(t) = positions.get(k) {
            let col = if k == current { ctl.accent } else { INK_SOFT };
            // side labels hang outward so they clear the knob
            let (lp, align) = if n >= 4 && a.sin() > 0.6 {
                (polar(c, r + 12.0, a), Align2::LEFT_CENTER)
            } else if n >= 4 && a.sin() < -0.6 {
                (polar(c, r + 12.0, a), Align2::RIGHT_CENTER)
            } else {
                (polar(c, r + 20.0, a), Align2::CENTER_CENTER)
            };
            p.text(lp, align, *t, FontId::proportional(10.0), col);
        }
    }
    // chicken-head knob
    let a = pos_angle(current);
    p.circle_filled(c + Vec2::new(0.0, 3.0), r, Color32::from_black_alpha(80));
    p.circle_filled(c, r * 0.9, BAKELITE);
    let tip_pt = polar(c, r * 1.05, a);
    let back = polar(c, r * 0.65, a + PI);
    let side1 = polar(c, r * 0.45, a + PI / 2.0);
    let side2 = polar(c, r * 0.45, a - PI / 2.0);
    p.add(Shape::convex_polygon(vec![tip_pt, side1, back, side2], BAKELITE_HI, Stroke::new(1.0_f32, BAKELITE)));
    p.line_segment([c, polar(c, r * 0.95, a)], Stroke::new(2.5_f32, CREAM));
    if resp.hovered() {
        p.circle_stroke(c, r + 1.0, Stroke::new(1.5_f32, ctl.accent));
    }
    p.text(c + Vec2::new(0.0, r + 13.0), Align2::CENTER_CENTER, label, FontId::proportional(11.0), INK);
}

fn meter(p: &egui::Painter, rect: Rect, kind: MeterKind, value: f32, caption: &str, accent: Color32) {
    p.rect_filled(rect.expand(5.0), CornerRadius::same(6), BAKELITE);
    p.rect_filled(rect, CornerRadius::same(3), METER_FACE);
    let pivot = Pos2::new(rect.center().x, rect.bottom() + 28.0);
    let radius = rect.height() + 2.0;
    let sweep = 40.0f32;

    // map a scale value to an angle (rad); returns (angle) for value in units of the meter
    let map = |v: f32| -> f32 {
        let t = match kind {
            MeterKind::GainReduction => 1.0 - (v.clamp(0.0, 20.0) / 20.0).powf(0.6), // right = 0 dB GR
            MeterKind::Vu => {
                // -20..+3, roughly logarithmic like a real VU
                let lin = 10f32.powf(v.clamp(-20.0, 3.0) / 20.0);
                (lin - 0.1) / (10f32.powf(0.15) - 0.1)
            }
        };
        (-sweep + 2.0 * sweep * t.clamp(0.0, 1.0)) * PI / 180.0
    };
    let (red_from, red_to, marks): (f32, f32, &[(f32, &str)]) = match kind {
        MeterKind::GainReduction => (10.0, 20.0, &[(0.0, "0"), (1.0, ""), (2.0, "2"), (3.0, ""), (5.0, "5"), (7.0, ""), (10.0, "10"), (15.0, ""), (20.0, "20")]),
        MeterKind::Vu => (0.0, 3.0, &[(-20.0, "20"), (-10.0, "10"), (-7.0, "7"), (-5.0, "5"), (-3.0, "3"), (-2.0, ""), (-1.0, ""), (0.0, "0"), (1.0, ""), (2.0, ""), (3.0, "+3")]),
    };
    let red: Vec<Pos2> = (0..=20).map(|k| polar(pivot, radius - 22.0, map(red_from + (red_to - red_from) * k as f32 / 20.0))).collect();
    p.add(Shape::line(red, PathStroke::new(4.0_f32, accent)));
    let (lo, hi) = match kind {
        MeterKind::GainReduction => (0.0, 20.0),
        MeterKind::Vu => (-20.0, 3.0),
    };
    let arc: Vec<Pos2> = (0..=40).map(|k| polar(pivot, radius - 26.0, map(lo + (hi - lo) * k as f32 / 40.0))).collect();
    p.add(Shape::line(arc, PathStroke::new(1.2_f32, INK)));
    for (v, txt) in marks {
        let a = map(*v);
        let long = !txt.is_empty();
        p.line_segment([polar(pivot, radius - 26.0, a), polar(pivot, radius - if long { 36.0 } else { 32.0 }, a)], Stroke::new(1.2_f32, INK));
        if long {
            p.text(polar(pivot, radius - 14.0, a), Align2::CENTER_CENTER, *txt, FontId::proportional(11.0), INK);
        }
    }
    let unit = match kind {
        MeterKind::GainReduction => "dB",
        MeterKind::Vu => "VU",
    };
    p.text(Pos2::new(rect.center().x, rect.bottom() - 34.0), Align2::CENTER_CENTER, caption, FontId::proportional(10.0), INK_SOFT);
    p.text(Pos2::new(rect.center().x, rect.bottom() - 21.0), Align2::CENTER_CENTER, unit, FontId::proportional(10.0), INK_SOFT);
    let clip = p.with_clip_rect(rect);
    clip.line_segment([pivot, polar(pivot, radius - 8.0, map(value))], Stroke::new(2.0_f32, Color32::from_rgb(0x1a, 0x12, 0x0c)));
    p.rect_filled(Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 10.0), rect.max), CornerRadius { nw: 0, ne: 0, sw: 3, se: 3 }, BAKELITE_HI);
}

fn lamp(p: &egui::Painter, c: Pos2, level: f32) {
    let off = Color32::from_rgb(0x5a, 0x2a, 0x18);
    let on = Color32::from_rgb(0xff, 0x8a, 0x2a);
    if level > 0.05 {
        for (r, a) in [(16.0, 40.0), (12.0, 70.0)] {
            p.circle_filled(c, r, Color32::from_rgba_unmultiplied(0xff, 0x8a, 0x2a, (a * level) as u8));
        }
    }
    p.circle_filled(c, 8.0, BAKELITE);
    p.circle_filled(c, 6.0, lerp_color(off, on, level));
    p.circle_filled(c - Vec2::new(2.0, 2.0), 1.8, Color32::from_white_alpha((60.0 + 120.0 * level) as u8));
}

fn rotor(p: &egui::Painter, c: Pos2, r: f32, horn_phase: f32, drum_phase: f32, accent: Color32) {
    // cabinet window
    let rect = Rect::from_center_size(c, Vec2::new(r * 2.0 + 24.0, r * 2.0 + 24.0));
    p.rect_filled(rect.expand(5.0), CornerRadius::same(6), BAKELITE);
    p.rect_filled(rect, CornerRadius::same(4), Color32::from_rgb(0x2a, 0x20, 0x1a));
    // grille slats
    let mut x = rect.left() + 6.0;
    while x < rect.right() - 3.0 {
        p.line_segment([Pos2::new(x, rect.top() + 3.0), Pos2::new(x, rect.bottom() - 3.0)], Stroke::new(1.0_f32, Color32::from_rgb(0x3a, 0x2e, 0x26)));
        x += 7.0;
    }
    // drum (lower rotor): a rotating baffle, drawn as a wide arc
    let da = drum_phase * 2.0 * PI;
    let drum: Vec<Pos2> = (0..=24).map(|k| polar(c, r * 0.95, da - 1.1 + 2.2 * k as f32 / 24.0)).collect();
    p.circle_stroke(c, r * 0.95, Stroke::new(1.0_f32, Color32::from_rgb(0x55, 0x47, 0x3c)));
    p.add(Shape::line(drum, PathStroke::new(6.0_f32, lerp_color(accent, CREAM, 0.25))));
    // horn (upper rotor): double cone, one end open
    let ha = horn_phase * 2.0 * PI;
    let tip = polar(c, r * 0.8, ha);
    let back = polar(c, r * 0.8, ha + PI);
    let w1 = polar(tip, r * 0.22, ha + PI / 2.0);
    let w2 = polar(tip, r * 0.22, ha - PI / 2.0);
    p.add(Shape::convex_polygon(vec![c, w1, w2], CREAM, Stroke::new(1.0_f32, BAKELITE)));
    p.line_segment([c, back], Stroke::new(4.0_f32, Color32::from_rgb(0x9a, 0x8c, 0x76)));
    p.circle_filled(c, r * 0.12, BAKELITE_HI);
    p.text(Pos2::new(c.x, rect.bottom() + 16.0), Align2::CENTER_CENTER, "ROTOR", FontId::proportional(10.0), INK_SOFT);
}

// ---------------------------------------------------------------- presets --

/// Draws the preset strip; returns true if a preset was applied this frame.
fn preset_bar(ui: &mut egui::Ui, p: &egui::Painter, ctl: &Ctl, state: &mut UiState, panel: &Panel, c: Pos2) -> bool {
    let n = panel.presets.len();
    let name_rect = Rect::from_center_size(c, Vec2::new(220.0, 20.0));
    let prev_rect = Rect::from_center_size(c - Vec2::new(128.0, 0.0), Vec2::new(22.0, 20.0));
    let next_rect = Rect::from_center_size(c + Vec2::new(128.0, 0.0), Vec2::new(22.0, 20.0));
    p.text(prev_rect.left_center() - Vec2::new(12.0, 0.0), Align2::RIGHT_CENTER, "PRESET", FontId::proportional(11.0), INK_SOFT);

    let mut chosen: Option<usize> = None;
    let cur = state.preset_sel;
    let prev = ui.interact(prev_rect, Id::new("preset-prev"), Sense::click()).on_hover_text("Previous preset");
    let next = ui.interact(next_rect, Id::new("preset-next"), Sense::click()).on_hover_text("Next preset");
    let name = ui
        .interact(name_rect, Id::new("preset-name"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text("Click for the preset list");
    if prev.clicked() {
        chosen = Some(cur.map(|i| (i + n - 1) % n).unwrap_or(n - 1));
    }
    if next.clicked() {
        chosen = Some(cur.map(|i| (i + 1) % n).unwrap_or(0));
    }
    if name.clicked() {
        state.preset_open = !state.preset_open;
    }

    for (r, hovered, up) in [(prev_rect, prev.hovered(), false), (next_rect, next.hovered(), true)] {
        p.rect_filled(r, CornerRadius::same(4), if hovered { ctl.accent } else { BAKELITE });
        let cx = r.center();
        let pts = if up {
            vec![cx + Vec2::new(-3.0, -5.0), cx + Vec2::new(4.0, 0.0), cx + Vec2::new(-3.0, 5.0)]
        } else {
            vec![cx + Vec2::new(3.0, -5.0), cx + Vec2::new(-4.0, 0.0), cx + Vec2::new(3.0, 5.0)]
        };
        p.add(Shape::convex_polygon(pts, CREAM, Stroke::NONE));
    }
    p.rect_filled(name_rect, CornerRadius::same(4), BAKELITE);
    p.rect_stroke(name_rect, CornerRadius::same(4), Stroke::new(1.0_f32, if name.hovered() || state.preset_open { ctl.accent } else { BAKELITE_HI }), StrokeKind::Inside);
    let label = match cur {
        Some(i) => format!("{}{}", panel.presets[i].name, if state.preset_dirty { " *" } else { "" }),
        None => "— choose a preset —".to_string(),
    };
    p.text(name_rect.center(), Align2::CENTER_CENTER, label, FontId::proportional(12.0), METER_FACE);

    if state.preset_open {
        // list opens upwards over the panel
        let item_h = 22.0;
        let list_h = item_h * n as f32 + 8.0;
        let top = name_rect.top() - list_h - 4.0;
        let list_rect = Rect::from_min_size(Pos2::new(name_rect.left(), top), Vec2::new(name_rect.width(), list_h));
        let resp = egui::Area::new(Id::new("preset-list"))
            .order(egui::Order::Foreground)
            .fixed_pos(list_rect.min)
            .show(ui.ctx(), |ui| {
                let pp = ui.painter();
                pp.rect_filled(list_rect.translate(Vec2::new(0.0, 3.0)), CornerRadius::same(6), Color32::from_black_alpha(90));
                pp.rect_filled(list_rect, CornerRadius::same(6), BAKELITE);
                for (i, pr) in panel.presets.iter().enumerate() {
                    let r = Rect::from_min_size(Pos2::new(list_rect.left() + 4.0, list_rect.top() + 4.0 + i as f32 * item_h), Vec2::new(list_rect.width() - 8.0, item_h));
                    let it = ui.interact(r, Id::new(("preset-item", i)), Sense::click());
                    let sel = cur == Some(i);
                    if it.hovered() || sel {
                        pp.rect_filled(r, CornerRadius::same(4), if it.hovered() { ctl.accent } else { BAKELITE_HI });
                    }
                    pp.text(r.left_center() + Vec2::new(10.0, 0.0), Align2::LEFT_CENTER, pr.name, FontId::proportional(12.0), if it.hovered() { CREAM } else { METER_FACE });
                    if it.clicked() {
                        chosen = Some(i);
                    }
                }
            });
        // click anywhere else closes it
        if chosen.is_none() && ui.input(|i| i.pointer.any_click()) && !resp.response.hovered() && !name.hovered() {
            let inside = ui.input(|i| i.pointer.interact_pos()).map(|pos| list_rect.contains(pos)).unwrap_or(false);
            if !inside {
                state.preset_open = false;
            }
        }
    }

    if let Some(i) = chosen {
        let preset = &panel.presets[i];
        for k in 0..ctl.ptrs.len() {
            let plain = preset
                .values
                .iter()
                .find(|(idx, _)| *idx == k)
                .map(|(_, v)| *v)
                .unwrap_or_else(|| unsafe { ctl.ptrs[k].default_plain_value() });
            let norm = unsafe { ctl.ptrs[k].preview_normalized(plain) };
            ctl.begin(k);
            ctl.set(state, k, norm);
            ctl.end(k);
        }
        state.preset_sel = Some(i);
        state.preset_dirty = false;
        state.preset_open = false;
        return true;
    }
    false
}

// ------------------------------------------------------------------ tuner --

const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// Note name, octave and cents offset for a frequency.
pub fn note_of(freq: f32, a4: f32) -> (&'static str, i32, f32) {
    let n = 12.0 * (freq / a4).log2() + 69.0;
    let nearest = n.round();
    let cents = (n - nearest) * 100.0;
    let idx = (nearest as i32).rem_euclid(12) as usize;
    (NOTE_NAMES[idx], nearest as i32 / 12 - 1, cents)
}

fn tuner(p: &egui::Painter, rect: Rect, freq: f32, a4: f32, level: f32, accent: Color32) {
    p.rect_filled(rect.expand(5.0), CornerRadius::same(8), BAKELITE);
    p.rect_filled(rect, CornerRadius::same(5), Color32::from_rgb(0x14, 0x12, 0x10));
    let glow = Color32::from_rgb(0xff, 0xb2, 0x4a);
    let dim = Color32::from_rgb(0x4a, 0x3a, 0x2a);
    let green = Color32::from_rgb(0x7c, 0xe0, 0x7a);

    // cents scale across the top
    let scale_y = rect.top() + 26.0;
    let x_of = |cents: f32| rect.center().x + cents.clamp(-50.0, 50.0) / 50.0 * (rect.width() * 0.42);
    for c in (-50..=50).step_by(10) {
        let x = x_of(c as f32);
        let long = c % 50 == 0 || c == 0;
        p.line_segment([Pos2::new(x, scale_y), Pos2::new(x, scale_y + if long { 12.0 } else { 7.0 })], Stroke::new(1.2_f32, dim));
    }
    p.text(Pos2::new(x_of(-50.0), scale_y - 10.0), Align2::CENTER_CENTER, "-50", FontId::proportional(10.0), dim);
    p.text(Pos2::new(x_of(50.0), scale_y - 10.0), Align2::CENTER_CENTER, "+50", FontId::proportional(10.0), dim);
    p.text(Pos2::new(x_of(0.0), scale_y - 10.0), Align2::CENTER_CENTER, "0", FontId::proportional(10.0), dim);

    if freq > 0.0 {
        let (name, oct, cents) = note_of(freq, a4);
        let in_tune = cents.abs() < 3.0;
        let col = if in_tune { green } else { glow };
        // needle
        let x = x_of(cents);
        p.line_segment([Pos2::new(x, scale_y - 4.0), Pos2::new(x, scale_y + 18.0)], Stroke::new(3.0_f32, col));
        // flat / sharp arrows light up
        let arrow_y = rect.center().y + 8.0;
        let left = Pos2::new(rect.left() + 34.0, arrow_y);
        let right = Pos2::new(rect.right() - 34.0, arrow_y);
        let flat_on = cents < -3.0;
        let sharp_on = cents > 3.0;
        p.add(Shape::convex_polygon(vec![left + Vec2::new(10.0, -12.0), left + Vec2::new(-10.0, 0.0), left + Vec2::new(10.0, 12.0)], if sharp_on { glow } else { dim }, Stroke::NONE));
        p.add(Shape::convex_polygon(vec![right + Vec2::new(-10.0, -12.0), right + Vec2::new(10.0, 0.0), right + Vec2::new(-10.0, 12.0)], if flat_on { glow } else { dim }, Stroke::NONE));
        p.text(Pos2::new(rect.center().x - 6.0, arrow_y), Align2::CENTER_CENTER, name, FontId::proportional(54.0), col);
        p.text(Pos2::new(rect.center().x + 34.0, arrow_y + 14.0), Align2::LEFT_CENTER, oct.to_string(), FontId::proportional(16.0), col);
        p.text(Pos2::new(rect.center().x, rect.bottom() - 16.0), Align2::CENTER_CENTER, format!("{freq:.1} Hz   {cents:+.0} cents"), FontId::proportional(12.0), dim.gamma_multiply(2.2));
        if in_tune {
            p.circle_filled(Pos2::new(rect.right() - 16.0, rect.top() + 14.0), 5.0, green);
        }
    } else {
        let msg = if level < 0.002 { "PLAY A NOTE" } else { "LISTENING..." };
        p.text(rect.center(), Align2::CENTER_CENTER, msg, FontId::proportional(18.0), dim);
    }
    let _ = accent;
}
