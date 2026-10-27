//! Plugin plumbing shared by every effect in the suite: the parameter link,
//! saved editor values, meters, and an export macro that writes the
//! nih-plug `Plugin`/`Vst3Plugin` boilerplate.

use crate::amp::ir::IrShared;
use crate::link::{GuiLink, Resolver};
use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};

/// Implemented by each plugin's `#[derive(Params)]` struct.
pub trait VintageParams: Params + Default + 'static {
    fn editor_state(&self) -> Arc<EguiState>;
    /// `#[persist]` field holding editor-set values (see `link`).
    fn saved(&self) -> &Arc<RwLock<Vec<f32>>>;
    /// The parameters the DSP and panel use, in a fixed order.
    fn linked(&self) -> Vec<ParamPtr>;
    /// `#[persist]` field holding a loaded impulse-response file path, for
    /// plugins with an IR loader.
    fn ir_path(&self) -> Option<Arc<RwLock<String>>> {
        None
    }
}

/// Implemented by each plugin's DSP.
pub trait VintageDsp: Send + 'static {
    /// Number of meter values the DSP reports (GR, levels, rotor phase, ...).
    const METERS: usize;
    fn new(sample_rate: f32, channels: usize) -> Self;
    fn reset(&mut self);
    fn latency(&self) -> u32 {
        0
    }
    /// Plain (display-unit) values of every linked parameter, once per block.
    fn set_params(&mut self, plain: &[f32]);
    /// Process a block in place. Write meter values (>= 0) into `meters`;
    /// the engine keeps the maximum until the editor reads them.
    fn process(&mut self, chans: &mut [&mut [f32]], meters: &mut [f32]);
    /// Plugins with an IR loader keep this to pick up newly loaded files.
    fn attach_ir(&mut self, _ir: Arc<IrShared>) {}
    /// MIDI for instruments, delivered between sub-blocks at the right sample.
    fn midi(&mut self, _ev: Midi) {}
    /// Host tempo (BPM), song position in quarter notes, and play state, once per block.
    fn transport(&mut self, _tempo: Option<f64>, _beats: Option<f64>, _playing: bool) {}
}

/// MIDI events passed to instrument DSPs (velocity / value / bend in 0..1, -1..1).
#[derive(Clone, Copy, Debug)]
pub enum Midi {
    On { note: u8, vel: f32 },
    Off { note: u8 },
    Cc { cc: u8, value: f32 },
    Bend { value: f32 },
    AllOff,
}

/// A MIDI event at a sample offset within the block.
#[derive(Clone, Copy, Debug)]
pub struct TimedMidi {
    pub at: u32,
    pub ev: Midi,
}

/// Meter values shared with the editor. Index `METERS` and above are unused.
pub type Meters = Arc<Vec<AtomicF32>>;

pub struct Engine<D: VintageDsp> {
    pub link: Arc<GuiLink>,
    pub meters: Meters,
    pub ir: Arc<IrShared>,
    resolver: Option<Resolver>,
    dsp: Option<D>,
    sample_rate: f32,
    ptrs: Vec<ParamPtr>,
    norms: Vec<f32>,
    plain: Vec<f32>,
    block_meters: Vec<f32>,
    pub midi_buf: Vec<TimedMidi>,
}

fn norms_of(ptrs: &[ParamPtr], modulated: bool, out: &mut [f32]) {
    for (o, p) in out.iter_mut().zip(ptrs) {
        // SAFETY: the pointers come from the plugin's params Arc, which lives
        // as long as the plugin (and therefore this engine).
        *o = unsafe {
            if modulated {
                p.modulated_normalized_value()
            } else {
                p.unmodulated_normalized_value()
            }
        };
    }
}

impl<D: VintageDsp> Engine<D> {
    pub fn new<P: VintageParams>(params: &P) -> Self {
        let ptrs = params.linked();
        let mut norms = vec![0.0; ptrs.len()];
        norms_of(&ptrs, false, &mut norms);
        Self {
            link: Arc::new(GuiLink::new(&norms)),
            meters: Arc::new((0..D::METERS.max(1)).map(|_| AtomicF32::new(0.0)).collect()),
            ir: Arc::new(IrShared::default()),
            resolver: None,
            dsp: None,
            sample_rate: 48000.0,
            plain: vec![0.0; ptrs.len()],
            norms,
            ptrs,
            block_meters: vec![0.0; D::METERS.max(1)],
            midi_buf: Vec::with_capacity(1024),
        }
    }

    pub fn initialize<P: VintageParams>(&mut self, params: &P, channels: usize, sample_rate: f32) -> u32 {
        self.sample_rate = sample_rate;
        let mut dsp = D::new(sample_rate, channels);
        let latency = dsp.latency();
        self.ir.sample_rate.store(sample_rate, Ordering::Relaxed);
        dsp.attach_ir(self.ir.clone());
        self.dsp = Some(dsp);
        // (re)load the saved impulse response at this sample rate
        if let Some(path) = params.ir_path() {
            let p = path.read().map(|p| p.clone()).unwrap_or_default();
            if !p.is_empty() {
                self.ir.load_file(std::path::Path::new(&p));
            }
        }

        let mut current = vec![0.0; self.ptrs.len()];
        norms_of(&self.ptrs, false, &mut current);
        let mut r = Resolver::new(&self.link, &current);
        for (i, v) in current.iter().enumerate() {
            self.link.set_effective(i, *v);
        }
        // re-apply editor edits the host never stored
        if let Ok(saved) = params.saved().read() {
            if saved.len() == self.ptrs.len() {
                for (i, v) in saved.iter().enumerate() {
                    let v = v.clamp(0.0, 1.0);
                    if (v - current[i]).abs() > 1e-6 {
                        r.force(&self.link, i, v);
                    }
                }
            }
        }
        self.resolver = Some(r);
        self.update_plain();
        if let Some(d) = self.dsp.as_mut() {
            d.set_params(&self.plain);
            d.reset();
        }
        latency
    }

    fn update_plain(&mut self) {
        if let Some(r) = self.resolver.as_ref() {
            for (i, n) in r.effective().iter().enumerate() {
                self.plain[i] = unsafe { self.ptrs[i].preview_plain(*n) };
            }
        }
    }

    pub fn reset(&mut self) {
        if let Some(d) = self.dsp.as_mut() {
            d.reset();
        }
    }

    pub fn transport(&mut self, tempo: Option<f64>, beats: Option<f64>, playing: bool) {
        if let Some(d) = self.dsp.as_mut() {
            d.transport(tempo, beats, playing);
        }
    }

    pub fn process<P: VintageParams>(&mut self, params: &P, buffer: &mut Buffer) {
        let (Some(_), Some(_)) = (self.dsp.as_ref(), self.resolver.as_ref()) else {
            return;
        };
        norms_of(&self.ptrs, true, &mut self.norms);
        let block = buffer.samples() as u32;
        let host_changed = self
            .resolver
            .as_mut()
            .unwrap()
            .resolve(&self.link, &self.norms, block, self.sample_rate);
        if host_changed {
            if let Ok(mut w) = params.saved().try_write() {
                if w.len() == self.norms.len() {
                    w.copy_from_slice(self.resolver.as_ref().unwrap().effective());
                }
            }
        }
        self.update_plain();

        let dsp = self.dsp.as_mut().unwrap();
        dsp.set_params(&self.plain);
        self.block_meters.iter_mut().for_each(|m| *m = 0.0);
        if self.midi_buf.is_empty() {
            dsp.process(buffer.as_slice(), &mut self.block_meters);
        } else {
            // split the block at each event so notes land on the right sample
            let n = buffer.samples();
            let chans = buffer.as_slice();
            let mut start = 0usize;
            let mut k = 0usize;
            while start < n || k < self.midi_buf.len() {
                while k < self.midi_buf.len() && (self.midi_buf[k].at as usize) <= start {
                    dsp.midi(self.midi_buf[k].ev);
                    k += 1;
                }
                if start >= n {
                    break;
                }
                let end = if k < self.midi_buf.len() { (self.midi_buf[k].at as usize).clamp(start + 1, n) } else { n };
                if chans.len() >= 2 {
                    let (l, r) = chans.split_at_mut(1);
                    let mut seg: [&mut [f32]; 2] = [&mut l[0][start..end], &mut r[0][start..end]];
                    dsp.process(&mut seg, &mut self.block_meters);
                } else if chans.len() == 1 {
                    let mut seg: [&mut [f32]; 1] = [&mut chans[0][start..end]];
                    dsp.process(&mut seg, &mut self.block_meters);
                }
                start = end;
            }
            self.midi_buf.clear();
        }

        if params.editor_state().is_open() {
            for (m, v) in self.meters.iter().zip(&self.block_meters) {
                let old = m.load(Ordering::Relaxed);
                m.store(old.max(*v), Ordering::Relaxed);
            }
        }
    }
}

/// Generate the plugin struct and its nih-plug trait impls.
///
/// ```ignore
/// vintage::vintage_plugin!(
///     Doubler, DoublerParams, DoublerDsp, panel,
///     name: "Doubler", class_id: *b"Doubler_PEQUOD01", vendor: "PEQUOD Beatles",
///     categories: [Fx, Modulation]
/// );
/// ```
/// Add `instrument,` before `categories` for a MIDI instrument (no audio input).
#[macro_export]
macro_rules! vintage_plugin {
    ($ty:ident, $params:ty, $dsp:ty, $panel:path,
     name: $name:literal, class_id: $id:expr, vendor: $vendor:literal,
     instrument, categories: [$($cat:ident),*]) => {
        $crate::vintage_plugin!(@impl $ty, $params, $dsp, $panel, $name, $id, $vendor, true, [$($cat),*]);
    };
    ($ty:ident, $params:ty, $dsp:ty, $panel:path,
     name: $name:literal, class_id: $id:expr, vendor: $vendor:literal,
     categories: [$($cat:ident),*]) => {
        $crate::vintage_plugin!(@impl $ty, $params, $dsp, $panel, $name, $id, $vendor, false, [$($cat),*]);
    };
    (@impl $ty:ident, $params:ty, $dsp:ty, $panel:path, $name:literal, $id:expr, $vendor:literal, $inst:literal, [$($cat:ident),*]) => {
        pub struct $ty {
            params: ::std::sync::Arc<$params>,
            engine: $crate::plugin::Engine<$dsp>,
        }

        impl Default for $ty {
            fn default() -> Self {
                let params = ::std::sync::Arc::new(<$params>::default());
                let engine = $crate::plugin::Engine::new(&*params);
                Self { params, engine }
            }
        }

        impl $crate::nih_plug::prelude::Plugin for $ty {
            const NAME: &'static str = $name;
            const VENDOR: &'static str = $vendor;
            const URL: &'static str = "";
            const EMAIL: &'static str = "";
            const VERSION: &'static str = env!("CARGO_PKG_VERSION");
            const AUDIO_IO_LAYOUTS: &'static [$crate::nih_plug::prelude::AudioIOLayout] = if $inst {
                &[$crate::nih_plug::prelude::AudioIOLayout {
                    main_input_channels: None,
                    main_output_channels: ::std::num::NonZeroU32::new(2),
                    ..$crate::nih_plug::prelude::AudioIOLayout::const_default()
                }]
            } else {
                &[
                    $crate::nih_plug::prelude::AudioIOLayout {
                        main_input_channels: ::std::num::NonZeroU32::new(2),
                        main_output_channels: ::std::num::NonZeroU32::new(2),
                        ..$crate::nih_plug::prelude::AudioIOLayout::const_default()
                    },
                    $crate::nih_plug::prelude::AudioIOLayout {
                        main_input_channels: ::std::num::NonZeroU32::new(1),
                        main_output_channels: ::std::num::NonZeroU32::new(1),
                        ..$crate::nih_plug::prelude::AudioIOLayout::const_default()
                    },
                ]
            };
            const MIDI_INPUT: $crate::nih_plug::prelude::MidiConfig = if $inst {
                $crate::nih_plug::prelude::MidiConfig::MidiCCs
            } else {
                $crate::nih_plug::prelude::MidiConfig::None
            };
            const SAMPLE_ACCURATE_AUTOMATION: bool = false;
            type SysExMessage = ();
            type BackgroundTask = ();

            fn params(&self) -> ::std::sync::Arc<dyn $crate::nih_plug::prelude::Params> {
                self.params.clone()
            }

            fn editor(
                &mut self,
                _e: $crate::nih_plug::prelude::AsyncExecutor<Self>,
            ) -> Option<Box<dyn $crate::nih_plug::prelude::Editor>> {
                $crate::ui::create_editor(
                    self.params.clone(),
                    self.engine.link.clone(),
                    self.engine.meters.clone(),
                    self.engine.ir.clone(),
                    $panel(),
                )
            }

            fn initialize(
                &mut self,
                layout: &$crate::nih_plug::prelude::AudioIOLayout,
                config: &$crate::nih_plug::prelude::BufferConfig,
                context: &mut impl $crate::nih_plug::prelude::InitContext<Self>,
            ) -> bool {
                let ch = layout.main_output_channels.map(|c| c.get() as usize).unwrap_or(2);
                let latency = self.engine.initialize(&*self.params, ch, config.sample_rate);
                context.set_latency_samples(latency);
                true
            }

            fn reset(&mut self) {
                self.engine.reset();
            }

            fn process(
                &mut self,
                buffer: &mut $crate::nih_plug::prelude::Buffer,
                _aux: &mut $crate::nih_plug::prelude::AuxiliaryBuffers,
                ctx: &mut impl $crate::nih_plug::prelude::ProcessContext<Self>,
            ) -> $crate::nih_plug::prelude::ProcessStatus {
                if $inst {
                    use $crate::nih_plug::prelude::NoteEvent;
                    use $crate::plugin::{Midi, TimedMidi};
                    for c in buffer.as_slice().iter_mut() {
                        c.fill(0.0);
                    }
                    while let Some(ev) = ctx.next_event() {
                        let m = match ev {
                            NoteEvent::NoteOn { timing, note, velocity, .. } => {
                                if velocity <= 0.0 { Some((timing, Midi::Off { note })) } else { Some((timing, Midi::On { note, vel: velocity })) }
                            }
                            NoteEvent::NoteOff { timing, note, .. } => Some((timing, Midi::Off { note })),
                            NoteEvent::MidiCC { timing, cc, value, .. } => {
                                if cc == 120 || cc == 123 { Some((timing, Midi::AllOff)) } else { Some((timing, Midi::Cc { cc, value })) }
                            }
                            NoteEvent::MidiPitchBend { timing, value, .. } => Some((timing, Midi::Bend { value: value * 2.0 - 1.0 })),
                            _ => None,
                        };
                        if let Some((at, ev)) = m {
                            if self.engine.midi_buf.len() < self.engine.midi_buf.capacity() {
                                self.engine.midi_buf.push(TimedMidi { at, ev });
                            }
                        }
                    }
                }
                {
                    let t = ctx.transport();
                    self.engine.transport(t.tempo, t.pos_beats(), t.playing);
                }
                self.engine.process(&*self.params, buffer);
                $crate::nih_plug::prelude::ProcessStatus::Normal
            }
        }

        impl $crate::nih_plug::prelude::Vst3Plugin for $ty {
            const VST3_CLASS_ID: [u8; 16] = $id;
            const VST3_SUBCATEGORIES: &'static [$crate::nih_plug::prelude::Vst3SubCategory] =
                &[$($crate::nih_plug::prelude::Vst3SubCategory::$cat),*];
        }

        $crate::export_vst3!($ty);

        /// This plugin for the built-in PEQUOD engine (PEQUOD Host).
        pub fn pequod_native() -> $crate::native::Descriptor {
            $crate::native::Descriptor {
                name: $name,
                vendor: $vendor,
                class_id: $id,
                instrument: $inst,
                categories: &[$(stringify!($cat)),*],
                make: || ::std::boxed::Box::new($crate::native::NativeImpl::<$params, $dsp>::new()),
                panel: $panel,
            }
        }

        #[cfg(test)]
        mod vintage_generated_tests {
            #[allow(unused_imports)]
            use super::*;
            #[test]
            fn presets_are_valid() {
                $crate::plugin::check_presets(&<$params>::default(), &$panel());
            }
        }
    };
}

/// Every preset must refer to real parameters, with values inside their
/// ranges (a value that clamps would silently do something else).
pub fn check_presets<P: VintageParams>(params: &P, panel: &crate::ui::Panel) {
    let ptrs = params.linked();
    for pr in &panel.presets {
        for (i, v) in pr.values {
            assert!(*i < ptrs.len(), "preset '{}': parameter index {} out of range", pr.name, i);
            let norm = unsafe { ptrs[*i].preview_normalized(*v) };
            let back = unsafe { ptrs[*i].preview_plain(norm) };
            let name = unsafe { ptrs[*i].name() }.to_string();
            assert!((0.0..=1.0).contains(&norm), "preset '{}': {} = {} outside range", pr.name, name, v);
            assert!((back - v).abs() <= v.abs() * 0.01 + 0.06, "preset '{}': {} = {} clamps to {}", pr.name, name, v, back);
        }
    }
}

/// The VST3 entry points. Left out when the plugins are built into the PEQUOD
/// engine (feature `engine`), where 200 copies of them would collide.
#[cfg(not(feature = "engine"))]
#[macro_export]
macro_rules! export_vst3 {
    ($ty:ident) => {
        $crate::nih_plug::nih_export_vst3!($ty);
    };
}

#[cfg(feature = "engine")]
#[macro_export]
macro_rules! export_vst3 {
    ($ty:ident) => {};
}
