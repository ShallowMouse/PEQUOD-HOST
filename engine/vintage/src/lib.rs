//! Shared framework for the Studio Two suite of vintage-style plugins.

pub mod amp;
pub mod dsp;
pub mod link;
pub mod native;
pub mod pitch;
pub mod plugin;
pub mod ui;

pub use nih_plug;
pub use nih_plug_egui;
pub use realfft;
pub use plugin::{Engine, Midi, VintageDsp, VintageParams};

/// Helpers for declaring parameters consistently across the suite.
pub mod params {
    use nih_plug::prelude::*;

    pub fn db(name: &str, default: f32, min: f32, max: f32) -> FloatParam {
        FloatParam::new(name, default, FloatRange::Linear { min, max })
            .with_step_size(0.1)
            .with_unit(" dB")
    }

    pub fn hz(name: &str, default: f32, min: f32, max: f32) -> FloatParam {
        FloatParam::new(name, default, FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(-2.0) })
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(1))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz())
    }

    pub fn ms(name: &str, default: f32, min: f32, max: f32) -> FloatParam {
        FloatParam::new(name, default, FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(-1.0) })
            .with_step_size(0.1)
            .with_unit(" ms")
    }

    pub fn secs(name: &str, default: f32, min: f32, max: f32) -> FloatParam {
        FloatParam::new(name, default, FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(-1.0) })
            .with_step_size(0.01)
            .with_unit(" s")
    }

    pub fn pct(name: &str, default: f32) -> FloatParam {
        FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
            .with_unit("%")
            .with_value_to_string(formatters::v2s_f32_percentage(0))
            .with_string_to_value(formatters::s2v_f32_percentage())
    }

    pub fn rate(name: &str, default: f32, min: f32, max: f32) -> FloatParam {
        FloatParam::new(name, default, FloatRange::Skewed { min, max, factor: FloatRange::skew_factor(-1.0) })
            .with_step_size(0.01)
            .with_unit(" Hz")
    }
}
pub mod verb;
pub mod fx;
