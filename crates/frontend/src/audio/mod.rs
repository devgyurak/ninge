// SPDX-License-Identifier: Apache-2.0
//! Audio between the machine and the output device (#47, plan §4.3).
//!
//! The machine makes one stereo sample every `OUTPUT_SAMPLE_CYCLES` master cycles, 32,728.498 Hz
//! (`docs/hw/spu.md`, "Output samples"); the device runs at its own rate. The emu thread turns
//! the one into the other with a [`resample::Resampler`] and hands the result to the device
//! callback through an [`ring::AudioRing`]. How full that ring is then tells the pacer when the
//! next frame is due.

pub mod device;
pub mod resample;
pub mod ring;
pub mod sink;

use crate::pacing::{AudioPacer, FRAME_TIME, Level, Pacing};
use ring::AudioRing;
use sink::{AudioSink, NoAudio, RingSink};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

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

/// How a [`Pacing`] reads the ring: boxed so audio and frame pacing are one type.
pub type Fill = Box<dyn Fn() -> Level + Send>;

/// Where the emu loop's audio goes: the ring when a device is open, nowhere otherwise.
#[derive(Debug)]
pub enum Sink {
    /// Resample into the device's ring.
    Ring(RingSink),
    /// Drop it.
    None(NoAudio),
}

impl AudioSink for Sink {
    fn resumed(&mut self) {
        match self {
            Self::Ring(sink) => sink.resumed(),
            Self::None(sink) => sink.resumed(),
        }
    }

    fn push(&mut self, samples: &[i16]) {
        match self {
            Self::Ring(sink) => sink.push(samples),
            Self::None(sink) => sink.push(samples),
        }
    }
}

/// Puts `target` pairs of silence in `ring`, so a device's first callbacks have something to play
/// while the first frames are made. Done before the device starts.
pub fn prefill(ring: &AudioRing, target: usize) {
    ring.push(&vec![0; target * 2]);
}

/// What a run needs of an open device: its ring, its failed flag and its rate in pairs a second.
pub type Output = (Arc<AudioRing>, Arc<AtomicBool>, u32);

/// The pacing and the sink of a run: following the device's ring when there is one, else the
/// machine's frame period with nowhere for audio to go.
pub fn paced(output: Option<Output>, now: Instant) -> (Pacing<Fill>, Sink) {
    match output {
        Some((ring, failed, rate)) => {
            let target = target_pairs(rate);
            let reads = Arc::clone(&ring);
            let fill: Fill = Box::new(move || Level {
                fill: reads.fill(),
                pulled: reads.pulled(),
            });
            (
                Pacing::audio(AudioPacer::new(fill, target, rate), failed, now),
                Sink::Ring(RingSink::new(ring, rate, target)),
            )
        }
        None => (Pacing::frame(now), Sink::None(NoAudio)),
    }
}

#[cfg(test)]
mod tests;
