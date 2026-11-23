// SPDX-License-Identifier: Apache-2.0
//! Where the emu loop sends each frame's audio (#47).

use super::resample::{Resampler, trim_for_fill};
use super::ring::AudioRing;
use std::sync::Arc;

/// Takes the audio of one machine frame: interleaved stereo `i16` at the machine's output rate.
pub trait AudioSink {
    /// Handles one frame's samples. Never called with an empty slice.
    fn push(&mut self, samples: &[i16]);
}

/// A sink that drops everything, for a run with no sound device.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoAudio;

impl AudioSink for NoAudio {
    fn push(&mut self, _samples: &[i16]) {}
}

/// Resamples a frame's audio to the device's rate and stores it in the ring the device callback
/// reads. Before each frame it sets the resampler's trim from how far the ring is from its target
/// ([`trim_for_fill`]), so the two clocks stay together without a click.
#[derive(Debug)]
pub struct RingSink {
    ring: Arc<AudioRing>,
    resampler: Resampler,
    target: usize,
    scratch: Vec<i16>,
}

impl RingSink {
    /// A sink for `ring`, drained at `device_hz`, kept near `target` pairs. It starts the ring
    /// with `target` pairs of silence, so the device's first callbacks have something to play
    /// while the first frames are made.
    pub fn new(ring: Arc<AudioRing>, device_hz: u32, target: usize) -> Self {
        ring.push(&vec![0; target * 2]);
        Self {
            ring,
            resampler: Resampler::for_machine(device_hz),
            target,
            scratch: Vec::new(),
        }
    }
}

impl AudioSink for RingSink {
    fn push(&mut self, samples: &[i16]) {
        self.resampler
            .set_trim_ppm(trim_for_fill(self.ring.fill(), self.target));
        self.scratch.clear();
        self.resampler.process(samples, &mut self.scratch);
        self.ring.push(&self.scratch);
    }
}

#[cfg(test)]
mod tests;
