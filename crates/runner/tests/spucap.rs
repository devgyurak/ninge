// SPDX-License-Identifier: Apache-2.0
//! Sound capture, observed through our own `spucap` test ROM (`testroms/spucap/`). Channel 0 loops the
//! PCM8 byte `40h` (`4000h`) on the left, channel 2 loops `70h` (`7000h`) on the left once a phase
//! turns it on, and channel 1 plays the capture buffer back on the right at volume 64 (half). Capture
//! 0 shares channel 1's timer, `FE00h`, which is one sample for each output sample, so the speakers
//! carry what the capture wrote. The ROM stops and restarts the capture in eight phases with about
//! 200 output samples each (the ROM's header lists them).
//!
//! The expected levels are worked out by hand from `docs/hw/sound-capture.md` and `docs/hw/spu.md`,
//! then corroborated by a small model below that shares nothing with `ninge-core`. With master volume
//! 127 and the bias at the medium level, the left output is the left sum divided by 64 and floored,
//! clipped to the 10-bit range, times 64; the right output is channel 1's PCM16 value `s` at volume
//! 64 (half), which is `s / 128` floored, times 64.
//!
//! | phase | what the ROM does | left | right |
//! |---|---|---|---|
//! | A | nothing captured, the buffer is zero | `4000h` | 0 |
//! | B | capture the mixer as PCM16 (`4000h`) | `4000h` | `2000h` |
//! | C | channel 2 on: the sum `0B000h` clips to `7FFFh` in the capture, and to `7FC0h` at the speaker | `7FC0h` | `3FC0h` |
//! | D | capture channel 0 alone: `4000h`, not clipped, no channel 2 | `7FC0h` | `2000h` |
//! | E | channel 0 plus channel 1 (bits 0 and 1): channel 1 joins channel 0 and leaves the right | `7FC0h` | 0 |
//! | F | capture the mixer as PCM8, channel 1 plays PCM8: `7FFFh` clipped is the byte `7Fh` | `7FC0h` | `3F80h` |
//! | G | channel 2 off, one-shot PCM8 capture of `4000h` (byte `40h`) | `4000h` | `2000h` |
//! | H | channel 0 off: the one-shot ended and holds its bytes | 0 | `2000h` |
//!
//! Phase E rests on how this emulator models the addition for the mixer; `docs/hw/sound-capture.md`
//! lists it as unverified, so that plateau is the model's, not a measurement of hardware.

use ninge_runner::golden::{
    assert_audio_deterministic, assert_audio_round_trip, assert_deterministic, assert_frame_hash,
    rom_audio,
};

const FRAMES: u64 = 4;
/// A plateau is at least this many output samples; the settling of a capture buffer and the few
/// between two register writes are shorter.
const MIN_PLATEAU: usize = 50;

/// The runs of at least [`MIN_PLATEAU`] equal `(left, right)` pairs, in order.
fn plateaus(audio: &[i16]) -> Vec<(i16, i16)> {
    let pairs: Vec<(i16, i16)> = audio.chunks(2).map(|pair| (pair[0], pair[1])).collect();
    let mut runs: Vec<((i16, i16), usize)> = Vec::new();
    for pair in pairs {
        match runs.last_mut() {
            Some((seen, count)) if *seen == pair => *count += 1,
            _ => runs.push((pair, 1)),
        }
    }
    runs.into_iter()
        .filter(|&(_, count)| count >= MIN_PLATEAU)
        .map(|(pair, _)| pair)
        .collect()
}

/// The left output for a left sum, as the model derives it.
fn left_output(sum: i64) -> i16 {
    ((sum >> 6).clamp(-0x200, 0x1FF) * 64) as i16
}

/// The right output for channel 1's PCM16 value at volume 64, as the model derives it.
fn right_output(value: i64) -> i16 {
    ((value >> 7) * 64) as i16
}

/// What a PCM16 capture of `sum` holds: the sum clipped to 16 bits.
fn pcm16_capture(sum: i64) -> i64 {
    sum.clamp(-0x8000, 0x7FFF)
}

/// What channel 1 plays from a PCM8 capture of `sum`: the clipped sum's top byte, as a PCM16 value.
fn pcm8_replay(sum: i64) -> i64 {
    (pcm16_capture(sum) >> 8) << 8
}

/// The eight phases as the model derives them from the ROM's settings.
fn model() -> Vec<(i16, i16)> {
    let (ch0, ch2) = (0x4000, 0x7000);
    vec![
        (left_output(ch0), 0),
        (left_output(ch0), right_output(pcm16_capture(ch0))),
        (
            left_output(ch0 + ch2),
            right_output(pcm16_capture(ch0 + ch2)),
        ),
        (left_output(ch0 + ch2), right_output(ch0)),
        // Channel 1 joins channel 0, so it is on the left, not the right, and the left is clipped
        // whatever it adds.
        (left_output(ch0 + ch2 + 0x3FFF), 0),
        (left_output(ch0 + ch2), right_output(pcm8_replay(ch0 + ch2))),
        (left_output(ch0), right_output(pcm8_replay(ch0))),
        (0, right_output(pcm8_replay(ch0))),
    ]
}

#[test]
fn given_the_spucap_rom_when_4_frames_run_then_the_eight_phases_give_the_hand_worked_levels() {
    let expected: Vec<(i16, i16)> = vec![
        (0x4000, 0),
        (0x4000, 0x2000),
        (0x7FC0, 0x3FC0),
        (0x7FC0, 0x2000),
        (0x7FC0, 0),
        (0x7FC0, 0x3F80),
        (0x4000, 0x2000),
        (0, 0x2000),
    ];
    assert_eq!(plateaus(&rom_audio("spucap", FRAMES)), expected);
}

#[test]
fn given_an_independent_model_when_it_derives_the_phases_then_it_agrees_with_the_hand_table() {
    let hand: Vec<(i16, i16)> = vec![
        (0x4000, 0),
        (0x4000, 0x2000),
        (0x7FC0, 0x3FC0),
        (0x7FC0, 0x2000),
        (0x7FC0, 0),
        (0x7FC0, 0x3F80),
        (0x4000, 0x2000),
        (0, 0x2000),
    ];
    assert_eq!(model(), hand);
}

#[test]
fn given_the_spucap_rom_when_4_frames_run_then_the_audio_matches_the_model() {
    assert_eq!(plateaus(&rom_audio("spucap", FRAMES)), model());
}

#[test]
fn given_the_spucap_rom_when_4_frames_run_then_the_screens_and_audio_match_the_golden() {
    assert_frame_hash("spucap", FRAMES);
}

#[test]
fn given_the_spucap_rom_when_run_twice_then_the_frames_states_and_audio_agree() {
    assert_deterministic("spucap", FRAMES);
    assert_audio_deterministic("spucap", FRAMES);
}

#[test]
fn given_the_spucap_rom_when_a_savestate_is_resumed_then_the_audio_matches_the_uninterrupted_run() {
    assert_audio_round_trip("spucap", 2, 2);
}
