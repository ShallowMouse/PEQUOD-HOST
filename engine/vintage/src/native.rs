//! The PEQUOD engine: every plugin's DSP, parameters and panel layout behind
//! a plain interface, so PEQUOD Host can run them natively (no VST3 wrapper,
//! no scanning, panels drawn by the host). The VST3 builds are unaffected.
//!
//! State is saved in exactly the format the VST3 versions use (nih-plug's
//! `PluginState` JSON), so a rack saved with a VST3 PEQUOD plugin loads into
//! the native one and back.

use crate::plugin::{Midi, TimedMidi, VintageDsp, VintageParams};
use crate::ui::{Item, MeterKind, Panel};
use nih_plug::prelude::{AtomicF32, ParamPtr, PluginState};
use nih_plug::wrapper::state::ParamValue;
use std::cell::UnsafeCell;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::sync::atomic::Ordering;

/// One plugin as the engine lists it.
pub struct Descriptor {
    pub name: &'static str,
    pub vendor: &'static str,
    pub class_id: [u8; 16],
    pub instrument: bool,
    /// VST3 sub-categories, e.g. ["Fx", "Delay"].
    pub categories: &'static [&'static str],
    pub make: fn() -> Box<dyn Instance>,
    pub panel: fn() -> Panel,
}

/// A parameter as the host sees it.
#[derive(Clone, Debug)]
pub struct ParamInfo {
    pub id: String,
    pub name: String,
    pub unit: String,
    pub default: f32,
    /// Steps for switches and selectors (0 = continuous).
    pub steps: u32,
}

/// A running plugin.
///
/// The `&self` methods may be called from any thread at any time (they only
/// touch atomics and the immutable parameter definitions). The `unsafe` ones
/// are the audio side: call them from one thread at a time, and never
/// `prepare`/`reset` while `process` is running (the host's usual contract).
pub trait Instance: Send + Sync {
    fn params(&self) -> &[ParamInfo];
    fn meter_count(&self) -> usize;
    fn set_param(&self, i: usize, norm: f32);
    fn get_param(&self, i: usize) -> f32;
    fn param_text(&self, i: usize, norm: f32) -> String;
    fn param_from_text(&self, i: usize, text: &str) -> Option<f32>;
    /// Plain value (as in a preset) to normalized.
    fn normalize(&self, i: usize, plain: f32) -> f32;
    /// Meter values since the last call (the maximum of each), then cleared.
    fn take_meters(&self, out: &mut [f32]) -> usize;
    fn save_state(&self) -> String;
    fn load_state(&self, data: &[u8]) -> bool;
    /// # Safety
    /// Audio side; see the trait docs.
    unsafe fn prepare(&self, sample_rate: f32, channels: usize, max_block: usize) -> u32;
    /// # Safety
    /// Audio side.
    unsafe fn reset(&self);
    /// Queue a MIDI event for the next `process`, `at` samples into it.
    /// # Safety
    /// Audio side.
    unsafe fn midi(&self, at: u32, ev: Midi);
    /// # Safety
    /// Audio side.
    unsafe fn process(&self, chans: &mut [&mut [f32]], tempo: Option<f64>, beats: Option<f64>, playing: bool);
}

struct Ptr(ParamPtr);
// SAFETY: the pointers point into the boxed params, which their owner never
// moves or mutates (parameter values live in the atomics below).
unsafe impl Send for Ptr {}
unsafe impl Sync for Ptr {}

struct Audio<D: VintageDsp> {
    dsp: Option<D>,
    plain: Vec<f32>,
    block: Vec<f32>,
    midi: Vec<TimedMidi>,
}

pub struct NativeImpl<P: VintageParams, D: VintageDsp> {
    _params: Box<P>,
    ptrs: Vec<Ptr>,
    info: Vec<ParamInfo>,
    norms: Vec<AtomicF32>,
    meters: Vec<AtomicF32>,
    audio: UnsafeCell<Audio<D>>,
}

// SAFETY: see `Instance`: the audio cell is only touched through the unsafe
// audio-side methods, whose callers guarantee exclusive access.
unsafe impl<P: VintageParams, D: VintageDsp> Sync for NativeImpl<P, D> {}
unsafe impl<P: VintageParams, D: VintageDsp> Send for NativeImpl<P, D> {}

impl<P: VintageParams, D: VintageDsp> NativeImpl<P, D> {
    pub fn new() -> Self {
        let params = Box::new(P::default());
        let ptrs: Vec<ParamPtr> = params.linked();
        // ids from the parameter map, matched by pointer
        let map = params.param_map();
        let info: Vec<ParamInfo> = ptrs
            .iter()
            .map(|p| {
                let id = map.iter().find(|(_, q, _)| q == p).map(|(id, _, _)| id.clone()).unwrap_or_default();
                unsafe {
                    ParamInfo {
                        id,
                        name: p.name().to_string(),
                        unit: p.unit().trim().to_string(),
                        default: p.default_normalized_value(),
                        steps: p.step_count().map(|s| s as u32).unwrap_or(0),
                    }
                }
            })
            .collect();
        let norms = info.iter().map(|i| AtomicF32::new(i.default)).collect();
        let n = ptrs.len();
        Self {
            _params: params,
            ptrs: ptrs.into_iter().map(Ptr).collect(),
            info,
            norms,
            meters: (0..D::METERS.max(1)).map(|_| AtomicF32::new(0.0)).collect(),
            audio: UnsafeCell::new(Audio { dsp: None, plain: vec![0.0; n], block: vec![0.0; D::METERS.max(1)], midi: Vec::with_capacity(512) }),
        }
    }

    fn update_plain(&self, a: &mut Audio<D>) {
        for (i, p) in self.ptrs.iter().enumerate() {
            a.plain[i] = unsafe { p.0.preview_plain(self.norms[i].load(Ordering::Relaxed)) };
        }
    }
}

impl<P: VintageParams, D: VintageDsp> Default for NativeImpl<P, D> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P: VintageParams, D: VintageDsp> Instance for NativeImpl<P, D> {
    fn params(&self) -> &[ParamInfo] {
        &self.info
    }
    fn meter_count(&self) -> usize {
        D::METERS
    }
    fn set_param(&self, i: usize, norm: f32) {
        if let Some(v) = self.norms.get(i) {
            // stepped parameters land exactly on a step
            let n = norm.clamp(0.0, 1.0);
            let n = match self.info[i].steps {
                0 => n,
                s => (n * s as f32).round() / s as f32,
            };
            v.store(n, Ordering::Relaxed);
        }
    }
    fn get_param(&self, i: usize) -> f32 {
        self.norms.get(i).map(|v| v.load(Ordering::Relaxed)).unwrap_or(0.0)
    }
    fn param_text(&self, i: usize, norm: f32) -> String {
        self.ptrs.get(i).map(|p| unsafe { p.0.normalized_value_to_string(norm, true) }).unwrap_or_default()
    }
    fn param_from_text(&self, i: usize, text: &str) -> Option<f32> {
        self.ptrs.get(i).and_then(|p| unsafe { p.0.string_to_normalized_value(text) })
    }
    fn normalize(&self, i: usize, plain: f32) -> f32 {
        self.ptrs.get(i).map(|p| unsafe { p.0.preview_normalized(plain) }).unwrap_or(0.0)
    }
    fn take_meters(&self, out: &mut [f32]) -> usize {
        let n = out.len().min(D::METERS);
        for (o, m) in out.iter_mut().zip(&self.meters).take(n) {
            *o = m.swap(0.0, Ordering::Relaxed);
        }
        n
    }
    fn save_state(&self) -> String {
        let mut params = BTreeMap::new();
        for (i, p) in self.ptrs.iter().enumerate() {
            if self.info[i].id.is_empty() {
                continue;
            }
            let norm = self.norms[i].load(Ordering::Relaxed);
            let v = unsafe {
                match p.0 {
                    ParamPtr::FloatParam(_) => ParamValue::F32(p.0.preview_plain(norm)),
                    ParamPtr::IntParam(_) => ParamValue::I32(p.0.preview_plain(norm).round() as i32),
                    ParamPtr::BoolParam(_) => ParamValue::Bool(norm > 0.5),
                    ParamPtr::EnumParam(_) => ParamValue::I32(p.0.preview_plain(norm).round() as i32),
                }
            };
            params.insert(self.info[i].id.clone(), v);
        }
        let st = PluginState { version: env!("CARGO_PKG_VERSION").to_string(), params, fields: BTreeMap::new() };
        serde_json::to_string(&st).unwrap_or_default()
    }
    fn load_state(&self, data: &[u8]) -> bool {
        let Ok(st) = serde_json::from_slice::<PluginState>(data) else {
            return false;
        };
        for (i, p) in self.ptrs.iter().enumerate() {
            let Some(v) = st.params.get(&self.info[i].id) else { continue };
            let norm = unsafe {
                match v {
                    ParamValue::F32(x) => p.0.preview_normalized(*x),
                    ParamValue::I32(x) => p.0.preview_normalized(*x as f32),
                    ParamValue::Bool(b) => {
                        if *b {
                            1.0
                        } else {
                            0.0
                        }
                    }
                    // enums saved by their stable id: match the display text
                    ParamValue::String(s) => match p.0.string_to_normalized_value(s) {
                        Some(n) => n,
                        None => continue,
                    },
                }
            };
            self.norms[i].store(norm.clamp(0.0, 1.0), Ordering::Relaxed);
        }
        true
    }
    unsafe fn prepare(&self, sample_rate: f32, channels: usize, _max_block: usize) -> u32 {
        let a = &mut *self.audio.get();
        let mut d = D::new(sample_rate, channels.max(1));
        self.update_plain(a);
        d.set_params(&a.plain);
        d.reset();
        let lat = d.latency();
        a.dsp = Some(d);
        a.midi.clear();
        lat
    }
    unsafe fn reset(&self) {
        let a = &mut *self.audio.get();
        if let Some(d) = a.dsp.as_mut() {
            d.reset();
        }
        a.midi.clear();
    }
    unsafe fn midi(&self, at: u32, ev: Midi) {
        let a = &mut *self.audio.get();
        if a.midi.len() < a.midi.capacity() {
            a.midi.push(TimedMidi { at, ev });
        }
    }
    unsafe fn process(&self, chans: &mut [&mut [f32]], tempo: Option<f64>, beats: Option<f64>, playing: bool) {
        let a = &mut *self.audio.get();
        if a.dsp.is_none() {
            return;
        }
        self.update_plain(a);
        let Audio { dsp, plain, block, midi } = a;
        let d = dsp.as_mut().unwrap();
        d.transport(tempo, beats, playing);
        d.set_params(plain);
        block.iter_mut().for_each(|m| *m = 0.0);
        let n = chans.first().map(|c| c.len()).unwrap_or(0);
        if midi.is_empty() {
            d.process(chans, block);
        } else {
            midi.sort_by_key(|m| m.at);
            let mut start = 0usize;
            let mut k = 0usize;
            while start < n || k < midi.len() {
                while k < midi.len() && (midi[k].at as usize) <= start {
                    d.midi(midi[k].ev);
                    k += 1;
                }
                if start >= n {
                    break;
                }
                let end = if k < midi.len() { (midi[k].at as usize).clamp(start + 1, n) } else { n };
                if chans.len() >= 2 {
                    let (l, r) = chans.split_at_mut(1);
                    let mut seg: [&mut [f32]; 2] = [&mut l[0][start..end], &mut r[0][start..end]];
                    d.process(&mut seg, block);
                } else if chans.len() == 1 {
                    let mut seg: [&mut [f32]; 1] = [&mut chans[0][start..end]];
                    d.process(&mut seg, block);
                }
                start = end;
            }
            midi.clear();
        }
        for (m, v) in self.meters.iter().zip(block.iter()) {
            let old = m.load(Ordering::Relaxed);
            if *v > old || old.is_nan() {
                m.store(*v, Ordering::Relaxed);
            }
        }
    }
}

// ------------------------------------------------------------- panel JSON --

pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn strs(v: &[&str]) -> String {
    format!("[{}]", v.iter().map(|s| esc(s)).collect::<Vec<_>>().join(","))
}

fn num(x: f32) -> String {
    if x.is_finite() {
        format!("{x}")
    } else {
        "0".to_string()
    }
}

/// The panel as JSON for the host to draw: title, colours, size, every item and the presets.
pub fn panel_json(p: &Panel) -> String {
    let mut items = Vec::new();
    for it in &p.items {
        let j = match it {
            Item::Knob { p, x, y, r, label, tip, active } => {
                let a = active.map(|a| format!(",\"active\":[{},{},{}]", a.p, num(a.min), num(a.max))).unwrap_or_default();
                format!("{{\"type\":\"knob\",\"p\":{p},\"x\":{},\"y\":{},\"r\":{},\"label\":{},\"tip\":{}{a}}}", num(*x), num(*y), num(*r), esc(label), esc(tip))
            }
            Item::Switch { p, x, y, labels, tip } => {
                format!("{{\"type\":\"switch\",\"p\":{p},\"x\":{},\"y\":{},\"labels\":{},\"tip\":{}}}", num(*x), num(*y), strs(labels), esc(tip))
            }
            Item::Selector { p, x, y, label, positions, tip } => format!(
                "{{\"type\":\"selector\",\"p\":{p},\"x\":{},\"y\":{},\"label\":{},\"positions\":{},\"tip\":{}}}",
                num(*x),
                num(*y),
                esc(label),
                strs(positions),
                esc(tip)
            ),
            Item::Meter { m, x, y, w, h, kind, caption } => {
                let k = match kind {
                    MeterKind::GainReduction => "gr",
                    MeterKind::Vu => "vu",
                };
                format!(
                    "{{\"type\":\"meter\",\"m\":{m},\"x\":{},\"y\":{},\"w\":{},\"h\":{},\"kind\":\"{k}\",\"caption\":{}}}",
                    num(*x),
                    num(*y),
                    num(*w),
                    num(*h),
                    esc(caption)
                )
            }
            Item::Lamp { m, x, y, label, lo, hi } => {
                format!("{{\"type\":\"lamp\",\"m\":{m},\"x\":{},\"y\":{},\"label\":{},\"lo\":{},\"hi\":{}}}", num(*x), num(*y), esc(label), num(*lo), num(*hi))
            }
            Item::Rotor { horn, drum, x, y, r } => {
                format!("{{\"type\":\"rotor\",\"horn\":{horn},\"drum\":{drum},\"x\":{},\"y\":{},\"r\":{}}}", num(*x), num(*y), num(*r))
            }
            Item::Divider { y } => format!("{{\"type\":\"divider\",\"y\":{}}}", num(*y)),
            Item::Text { x, y, text, size, .. } => format!("{{\"type\":\"text\",\"x\":{},\"y\":{},\"text\":{},\"size\":{}}}", num(*x), num(*y), esc(text), num(*size)),
            Item::RadioList { p, x, y, w, row_h, names, tip } => format!(
                "{{\"type\":\"radio\",\"p\":{p},\"x\":{},\"y\":{},\"w\":{},\"row_h\":{},\"names\":{},\"tip\":{}}}",
                num(*x),
                num(*y),
                num(*w),
                num(*row_h),
                strs(names),
                esc(tip)
            ),
            Item::Caption { p, x, y, w, size, texts } => {
                format!("{{\"type\":\"caption\",\"p\":{p},\"x\":{},\"y\":{},\"w\":{},\"size\":{},\"texts\":{}}}", num(*x), num(*y), num(*w), num(*size), strs(texts))
            }
            Item::RoomPlan { x, y, w, h, room, distance, spacing, .. } => format!(
                "{{\"type\":\"roomplan\",\"x\":{},\"y\":{},\"w\":{},\"h\":{},\"room\":{room},\"distance\":{distance},\"spacing\":{spacing}}}",
                num(*x),
                num(*y),
                num(*w),
                num(*h)
            ),
            Item::Polar { x, y, r, pattern, angle } => {
                format!("{{\"type\":\"polar\",\"x\":{},\"y\":{},\"r\":{},\"pattern\":{pattern},\"angle\":{angle}}}", num(*x), num(*y), num(*r))
            }
            Item::IrLoader { x, y, w, .. } => format!("{{\"type\":\"irloader\",\"x\":{},\"y\":{},\"w\":{}}}", num(*x), num(*y), num(*w)),
            Item::Tuner { freq, level, reference, x, y, w, h } => format!(
                "{{\"type\":\"tuner\",\"freq\":{freq},\"level\":{level},\"reference\":{reference},\"x\":{},\"y\":{},\"w\":{},\"h\":{}}}",
                num(*x),
                num(*y),
                num(*w),
                num(*h)
            ),
        };
        items.push(j);
    }
    let presets: Vec<String> = p
        .presets
        .iter()
        .map(|pr| format!("{{\"name\":{},\"values\":[{}]}}", esc(pr.name), pr.values.iter().map(|(i, v)| format!("[{i},{}]", num(*v))).collect::<Vec<_>>().join(",")))
        .collect();
    let a = p.accent;
    format!(
        "{{\"title\":{},\"subtitle\":[{},{}],\"accent\":\"#{:02x}{:02x}{:02x}\",\"w\":{},\"h\":{},\"items\":[{}],\"presets\":[{}]}}",
        esc(p.title),
        esc(p.subtitle.0),
        esc(p.subtitle.1),
        a.r(),
        a.g(),
        a.b(),
        p.size.0,
        p.size.1,
        items.join(","),
        presets.join(",")
    )
}

/// The plugin and its parameters as JSON, for the host's plugin list.
pub fn info_json(d: &Descriptor) -> String {
    let inst = (d.make)();
    let params: Vec<String> = inst
        .params()
        .iter()
        .map(|p| format!("{{\"id\":{},\"name\":{},\"unit\":{},\"default\":{},\"steps\":{}}}", esc(&p.id), esc(&p.name), esc(&p.unit), num(p.default), p.steps))
        .collect();
    let id: String = d.class_id.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{{\"name\":{},\"vendor\":{},\"id\":\"{id}\",\"instrument\":{},\"categories\":{},\"meters\":{},\"params\":[{}]}}",
        esc(d.name),
        esc(d.vendor),
        d.instrument,
        esc(&d.categories.join("|")),
        inst.meter_count(),
        params.join(",")
    )
}
