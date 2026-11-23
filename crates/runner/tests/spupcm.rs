// SPDX-License-Identifier: Apache-2.0
//! PCM8 and PCM16 playback, observed through our own `spupcm` test ROM (`testroms/spupcm/`). The ARM7
//! makes a PCM8 ramp and a PCM16 ramp in RAM and starts three channels at TMR `FE00h`, which is one
//! channel sample for each output sample (2,048 master cycles): channel 0 loops the PCM8 ramp at
//! volume 64 on the left, channel 1 loops the PCM16 ramp at volume 127 on the right, channel 2 plays
//! the first 16 bytes of the ramp once with Hold at volume 64, centred.
//!
//! The expected samples are worked out here from `docs/hw/spu.md` ("Position", "Volume and panning",
//! "The mixer"), never read back from the emulator, and the code below shares nothing with
//! `ninge-core`. With master volume 127 and the bias at the medium level, a PCM8 byte `b` of channel
//! 0 (volume 64, left) is a level of `2 * b`, channel 2 (volume 64, centre) adds its byte `b2` to
//! both sides, and channel 1's PCM16 value `s` is a level of `s / 64` on the right; a level is 64 `i16`
//! steps. Where the first output sample of each channel falls depends on where its start write falls
//! on the 2,048-cycle grid, so the test lets `find_offsets` pick the phase and then compares every
//! sample.

use ninge_runner::golden::{
    assert_audio_deterministic, assert_audio_round_trip, assert_deterministic, assert_frame_hash,
    find_offsets, rom_audio,
};

const FRAMES: u64 = 4;
/// Output pairs compared: more than three loops of the longest channel.
const PAIRS: usize = 2_000;

/// Channel 0's byte `i` samples after its start: the ramp `4 * i - 128`, looping every 64.
fn byte0(i: i64) -> i64 {
    4 * (i % 64) - 128
}

/// Channel 1's PCM16 value `i` samples after its start: `800h * j - 4000h`, looping every 16.
fn value1(i: i64) -> i64 {
    0x800 * (i % 16) - 0x4000
}

/// Channel 2's byte `i` samples after its start: the first 16 bytes of the ramp, then the last of
/// them held (Hold set, byte 15 is -68).
fn byte2(i: i64) -> i64 {
    4 * i.min(15) - 128
}

/// A channel's value `i` samples in, 0 before it has started.
fn when_on(i: i64, value: impl Fn(i64) -> i64) -> i64 {
    if i >= 0 { value(i) } else { 0 }
}

/// The expected left and right `i16` at output sample `m` for the channels' first sounding output
/// samples `starts`.
fn expected(m: i64, starts: [i64; 3]) -> (i64, i64) {
    let b0 = when_on(m - starts[0], byte0);
    let s1 = when_on(m - starts[1], value1);
    let b2 = when_on(m - starts[2], byte2);
    (64 * (2 * b0 + b2), s1 + 64 * b2)
}

/// Whether the first [`PAIRS`] pairs of `audio` are the expected ones for `starts`.
fn matches(audio: &[i16], starts: [i64; 3]) -> bool {
    audio.chunks(2).take(PAIRS).enumerate().all(|(m, pair)| {
        let (left, right) = expected(m as i64, starts);
        (i64::from(pair[0]), i64::from(pair[1])) == (left, right)
    })
}

#[test]
fn given_the_spupcm_rom_when_4_frames_run_then_the_audio_is_the_three_channels_ramps_mixed() {
    let audio = rom_audio("spupcm", FRAMES);
    let phase = find_offsets(0..4, |starts| matches(&audio, starts));
    assert!(phase.is_some(), "no phase of the model fits the audio");
}

#[test]
fn given_the_spupcm_rom_when_4_frames_run_then_the_audio_has_one_pair_per_2048_cycles() {
    // 547 pairs in each frame; see `docs/hw/spu.md`, "Output samples".
    assert_eq!(rom_audio("spupcm", FRAMES).len(), 4 * 2 * 547);
}

#[test]
fn given_the_spupcm_rom_when_4_frames_run_then_the_screens_and_audio_match_the_golden() {
    assert_frame_hash("spupcm", FRAMES);
}

#[test]
fn given_the_spupcm_rom_when_run_twice_then_the_frames_states_and_audio_agree() {
    assert_deterministic("spupcm", FRAMES);
    assert_audio_deterministic("spupcm", FRAMES);
}

#[test]
fn given_the_spupcm_rom_when_a_savestate_is_resumed_then_the_audio_matches_the_uninterrupted_run() {
    assert_audio_round_trip("spupcm", 2, 2);
}
