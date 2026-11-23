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
