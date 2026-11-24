// SPDX-License-Identifier: Apache-2.0
//! Where the emu loop sends each frame's audio (#47).

use super::prefill;
use super::resample::{Resampler, trim_for_fill};
use super::ring::AudioRing;
use std::sync::Arc;

/// Takes the audio of one machine frame: interleaved stereo `i16` at the machine's output rate.
pub trait AudioSink {
    /// A frame is about to run. The emulation takes a good part of a frame's time, in which the
    /// device keeps playing, so a sink that wants the ring's level reads it here.
    fn frame_started(&mut self) {}

    /// The run goes on after a pause, in which nothing was pushed and the device played what was
    /// left. Called before the first frame after it.
    fn resumed(&mut self) {}

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
/// reads. It sets the resampler's trim from how far the ring was from its target when the frame
/// started ([`trim_for_fill`]; read at the end of the frame the ring would be a frame's worth
/// emptier and the trim would lean slow), so the two clocks stay together without a click.
#[derive(Debug)]
pub struct RingSink {
    ring: Arc<AudioRing>,
    resampler: Resampler,
    target: usize,
    /// The ring's fill when the frame started; the target before the first.
    level: usize,
    scratch: Vec<i16>,
}

impl RingSink {
    /// A sink for `ring`, drained at `device_hz`, kept near `target` pairs.
    pub fn new(ring: Arc<AudioRing>, device_hz: u32, target: usize) -> Self {
        Self {
            ring,
            resampler: Resampler::for_machine(device_hz),
            target,
            level: target,
            scratch: Vec::new(),
        }
    }
}

impl AudioSink for RingSink {
    fn frame_started(&mut self) {
        self.level = self.ring.fill();
    }

    /// Fills the ring back up to its target with silence, as before the first frame of a run
    /// ([`prefill`]): the device has played it empty, and the frames that follow would otherwise
    /// arrive at a device with nothing in hand. The level read at this frame's start
    /// ([`AudioSink::frame_started`]) was the drained ring's, so it is put at the target: the
    /// next trim must not take the pause for a clock that drifted.
    fn resumed(&mut self) {
        prefill(&self.ring, self.target.saturating_sub(self.ring.fill()));
        self.level = self.target;
    }

    fn push(&mut self, samples: &[i16]) {
        self.resampler
            .set_trim_ppm(trim_for_fill(self.level, self.target));
        self.scratch.clear();
        self.resampler.process(samples, &mut self.scratch);
        self.ring.push(&self.scratch);
    }
}

#[cfg(test)]
mod tests;
