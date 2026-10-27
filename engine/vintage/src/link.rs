//! Direct editor -> audio parameter link (works in hosts such as VSTHost
//! that never echo a plugin's own GUI edits back to the audio thread).
//! Whichever side changed a value most recently wins; in well-behaved hosts
//! both sides agree and nothing changes.

use nih_plug::prelude::AtomicF32;
use std::sync::atomic::{AtomicU32, Ordering};

/// Seconds after an editor edit during which host-side changes to the same
/// parameter are treated as (possibly lagging) echoes and ignored.
pub const HOLD_SECS: f32 = 0.35;

pub struct GuiLink {
    gui_norm: Vec<AtomicF32>,
    gui_seq: Vec<AtomicU32>,
    effective: Vec<AtomicF32>,
}

impl GuiLink {
    pub fn new(init: &[f32]) -> Self {
        Self {
            gui_norm: init.iter().map(|v| AtomicF32::new(*v)).collect(),
            gui_seq: init.iter().map(|_| AtomicU32::new(0)).collect(),
            effective: init.iter().map(|v| AtomicF32::new(*v)).collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.effective.len()
    }

    pub fn is_empty(&self) -> bool {
        self.effective.is_empty()
    }

    pub fn gui_set(&self, i: usize, norm: f32) {
        self.gui_norm[i].store(norm, Ordering::Relaxed);
        self.effective[i].store(norm, Ordering::Relaxed);
        self.gui_seq[i].fetch_add(1, Ordering::Release);
    }

    pub fn effective(&self, i: usize) -> f32 {
        self.effective[i].load(Ordering::Relaxed)
    }

    pub fn set_effective(&self, i: usize, norm: f32) {
        self.effective[i].store(norm, Ordering::Relaxed);
    }

    pub fn effective_all(&self) -> Vec<f32> {
        (0..self.len()).map(|i| self.effective(i)).collect()
    }
}

/// Audio-thread side. Pre-allocated; `resolve` does not allocate.
pub struct Resolver {
    last_param: Vec<f32>,
    last_seq: Vec<u32>,
    hold: Vec<u32>,
    eff: Vec<f32>,
}

impl Resolver {
    pub fn new(link: &GuiLink, params_norm: &[f32]) -> Self {
        Self {
            last_param: params_norm.to_vec(),
            last_seq: (0..params_norm.len()).map(|i| link.gui_seq[i].load(Ordering::Acquire)).collect(),
            hold: vec![0; params_norm.len()],
            eff: params_norm.to_vec(),
        }
    }

    pub fn force(&mut self, link: &GuiLink, i: usize, norm: f32) {
        self.eff[i] = norm;
        link.set_effective(i, norm);
    }

    pub fn effective(&self) -> &[f32] {
        &self.eff
    }

    /// Returns whether a host-side change was adopted this block.
    pub fn resolve(&mut self, link: &GuiLink, params_norm: &[f32], block_len: u32, sample_rate: f32) -> bool {
        let hold_len = (HOLD_SECS * sample_rate) as u32;
        let mut host_changed = false;
        for i in 0..self.eff.len() {
            if params_norm[i] != self.last_param[i] {
                self.last_param[i] = params_norm[i];
                if self.hold[i] == 0 {
                    self.eff[i] = params_norm[i];
                    link.set_effective(i, params_norm[i]);
                    host_changed = true;
                }
            }
            let seq = link.gui_seq[i].load(Ordering::Acquire);
            if seq != self.last_seq[i] {
                self.last_seq[i] = seq;
                self.eff[i] = link.gui_norm[i].load(Ordering::Relaxed);
                self.hold[i] = hold_len;
            }
            self.hold[i] = self.hold[i].saturating_sub(block_len);
        }
        host_changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SR: f32 = 48000.0;

    #[test]
    fn gui_edit_sticks_without_echo() {
        let link = GuiLink::new(&[0.5; 3]);
        let mut r = Resolver::new(&link, &[0.5; 3]);
        link.gui_set(0, 0.9);
        for _ in 0..100 {
            r.resolve(&link, &[0.5; 3], 512, SR);
            assert_eq!(r.effective()[0], 0.9);
        }
    }

    #[test]
    fn automation_wins_after_hold() {
        let link = GuiLink::new(&[0.5; 3]);
        let mut r = Resolver::new(&link, &[0.5; 3]);
        link.gui_set(2, 0.8);
        for _ in 0..100 {
            r.resolve(&link, &[0.5; 3], 512, SR);
        }
        assert!(r.resolve(&link, &[0.5, 0.5, 0.1], 512, SR));
        assert_eq!(r.effective()[2], 0.1);
    }

    #[test]
    fn lagging_echo_ignored() {
        let link = GuiLink::new(&[0.5; 1]);
        let mut r = Resolver::new(&link, &[0.5; 1]);
        link.gui_set(0, 0.6);
        r.resolve(&link, &[0.5], 512, SR);
        link.gui_set(0, 0.7);
        r.resolve(&link, &[0.6], 512, SR);
        assert_eq!(r.effective()[0], 0.7);
    }
}
