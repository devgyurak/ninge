// SPDX-License-Identifier: Apache-2.0
//! The sound device (#47, plan §4.3): a thin adapter over `cpal`, and the pure parts around it.
//!
//! The device's callback runs on `cpal`'s own audio thread. It pops stereo pairs from the
//! [`AudioRing`], converts them to the device's sample format and spreads them over its channels
//! ([`render`]); it takes no lock and allocates nothing. What can be tested without a device is
//! pure: the format conversion ([`OutputSample`]), the channel mapping ([`channel_sample`]), the
//! callback body ([`render`]) and whether to open a device at all ([`use_audio`]). [`Device`]
//! only wires them to `cpal`, so CI, which has no sound device, runs the "no device" path.

use super::ring::AudioRing;
use super::ring_capacity;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample, Stream, StreamConfig};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Pairs the callback pops at a time: the scratch it fills lives on the stack, so a callback of
/// any size is served without allocating.
const CHUNK_PAIRS: usize = 512;

/// A sample type the device can ask for, made from the ring's `i16`.
pub trait OutputSample: Copy {
    /// The sample for `sample`, silence (0) included.
    fn from_i16(sample: i16) -> Self;
}

impl OutputSample for i16 {
    fn from_i16(sample: i16) -> Self {
        sample
    }
}

impl OutputSample for f32 {
    /// `-32768` is -1.0 and `32767` is just under 1.0.
    fn from_i16(sample: i16) -> Self {
        f32::from(sample) / 32_768.0
    }
}

impl OutputSample for u16 {
    /// Unsigned audio centers on 32768: the sign bit flips.
    fn from_i16(sample: i16) -> Self {
        (sample as u16) ^ 0x8000
    }
}

/// The sample channel `channel` of a device with `channels` channels plays for a pair: one
/// channel gets the average of left and right, two get left and right, and more get left and
/// right on the first two and silence on the rest.
pub fn channel_sample(pair: [i16; 2], channel: usize, channels: usize) -> i16 {
    match (channels, channel) {
        (1, _) => {
            let mean = (i32::from(pair[0]) + i32::from(pair[1])) / 2;
            i16::try_from(mean).expect("the mean of two i16 is an i16")
        }
        (_, 0) => pair[0],
        (_, 1) => pair[1],
        _ => 0,
    }
}

/// The callback's body: fills `out`, `channels` interleaved samples per frame, from `ring`.
/// What the ring does not hold is silence and counts as an underrun ([`AudioRing::pop_into`]).
pub fn render<T: OutputSample>(ring: &AudioRing, channels: usize, out: &mut [T]) {
    let channels = channels.max(1);
    let mut scratch = [0_i16; CHUNK_PAIRS * 2];
    for chunk in out.chunks_mut(channels * CHUNK_PAIRS) {
        let frames = chunk.len().div_ceil(channels);
        let pairs = &mut scratch[..frames * 2];
        ring.pop_into(pairs);
        for (frame, pair) in chunk.chunks_mut(channels).zip(pairs.as_chunks::<2>().0) {
            for (channel, slot) in frame.iter_mut().enumerate() {
                *slot = T::from_i16(channel_sample(*pair, channel, channels));
            }
        }
    }
}

/// Opens a device through `open` when sound is wanted and there is a ROM to hear; `None` means
/// frame pacing. A run with `--no-audio` or with no ROM never calls `open`, and a failed `open`
/// is the same as no device: the emulator runs on without sound.
pub fn use_audio<T, E>(
    enabled: bool,
    has_rom: bool,
    open: impl FnOnce() -> Result<T, E>,
) -> Option<T> {
    (enabled && has_rom).then(open)?.ok()
}

/// Why a device could not be opened.
#[derive(Debug)]
pub enum OpenError {
    /// The host has no default output device.
    NoDevice,
    /// The device wants a sample format this adapter does not make.
    Unsupported(SampleFormat),
    /// `cpal` refused.
    Cpal(cpal::Error),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDevice => write!(f, "no sound output device"),
            Self::Unsupported(format) => write!(f, "unsupported sample format {format:?}"),
            Self::Cpal(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for OpenError {}

impl From<cpal::Error> for OpenError {
    fn from(error: cpal::Error) -> Self {
        Self::Cpal(error)
    }
}

/// An open, playing output stream and what the emu thread needs to feed it.
pub struct Device {
    /// Playing while it lives; dropping it stops the sound.
    _stream: Stream,
    /// The device's rate, in pairs a second.
    pub rate: u32,
    /// The ring the callback reads.
    pub ring: Arc<AudioRing>,
    /// Set by the stream's error callback: the device is gone or broken.
    pub failed: Arc<AtomicBool>,
}

impl Device {
    /// Opens the default output device at its default configuration and starts it.
    pub fn open() -> Result<Self, OpenError> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or(OpenError::NoDevice)?;
        let supported = device.default_output_config()?;
        let config = supported.config();
        let rate = config.sample_rate;
        let ring = Arc::new(AudioRing::new(ring_capacity(rate)));
        let failed = Arc::new(AtomicBool::new(false));
        let (reads, flags) = (Arc::clone(&ring), Arc::clone(&failed));
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, config, reads, flags),
            SampleFormat::I16 => build::<i16>(&device, config, reads, flags),
            SampleFormat::U16 => build::<u16>(&device, config, reads, flags),
            other => Err(OpenError::Unsupported(other)),
        }?;
        stream.play()?;
        Ok(Self {
            _stream: stream,
            rate,
            ring,
            failed,
        })
    }
}

/// A stream of `T` samples that plays `ring` and sets `failed` on an error.
fn build<T: SizedSample + OutputSample + 'static>(
    device: &cpal::Device,
    config: StreamConfig,
    ring: Arc<AudioRing>,
    failed: Arc<AtomicBool>,
) -> Result<Stream, OpenError> {
    let channels = usize::from(config.channels);
    Ok(device.build_output_stream(
        config,
        move |out: &mut [T], _| render(&ring, channels, out),
        move |_| failed.store(true, Ordering::Release),
        None,
    )?)
}

#[cfg(test)]
mod tests;
