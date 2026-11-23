// SPDX-License-Identifier: Apache-2.0
//! Audio between the machine and the output device (#47, plan §4.3).
//!
//! The machine makes one stereo sample every `OUTPUT_SAMPLE_CYCLES` master cycles, 32,728.498 Hz
//! (`docs/hw/spu.md`, "Output samples"); the device runs at its own rate. The emu thread turns
//! the one into the other with a [`resample::Resampler`] and hands the result to the device
//! callback through an [`ring::AudioRing`]. How full that ring is then tells the pacer when the
//! next frame is due.

pub mod resample;
pub mod ring;
pub mod sink;

use crate::pacing::FRAME_TIME;

/// Frames of audio the ring is kept at: one for the emu thread's jitter, one for the device's
/// own buffer and one to spare, about 50 ms. Less underruns on a busy machine; more is delay
/// between a button and its sound.
pub const TARGET_FRAMES: u32 = 3;

/// The pairs the ring is kept near at a device rate of `device_hz`: [`TARGET_FRAMES`] frames.
pub fn target_pairs(device_hz: u32) -> usize {
    let nanos = FRAME_TIME.as_nanos() * u128::from(TARGET_FRAMES);
    usize::try_from(nanos * u128::from(device_hz) / 1_000_000_000).expect("a target fits usize")
}

/// The ring's capacity in pairs at a device rate of `device_hz`: a quarter of a second, far
/// above the target, so a late wake of the pacer does not overflow it.
pub fn ring_capacity(device_hz: u32) -> usize {
    usize::try_from(device_hz / 4).expect("a capacity fits usize")
}

#[cfg(test)]
mod tests;
