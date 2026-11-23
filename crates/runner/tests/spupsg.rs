// SPDX-License-Identifier: Apache-2.0
//! PSG and noise, observed through our own `spupsg` test ROM (`testroms/spupsg/`). The ARM7 starts three
//! channels at TMR `FE00h` (one position or step for each output sample, 2,048 master cycles), all at
//! volume 32: channel 8, PSG duty 3, on the left; channel 9, PSG duty 6, on the right; channel 14,
//! noise, centred.
//!
//! The expected samples come from `docs/hw/spu.md` ("PSG", "Noise", "Volume and panning", "The
//! mixer"): the duty table drawn there, the noise register worked from `7FFFh`, and the mixer
//! steps. The code below shares nothing with `ninge-core`. HIGH and LOW are `+7FFFh` and `-7FFFh`; at
//! volume 32 a sample is `s * 64` mixer units on a channel's own side (`s * 32` centred) and the
//! master stage keeps `units * 128 >> 21`; a level is 64 `i16` steps. Which output sample first
//! carries each channel depends on where its start write falls on the 2,048-cycle grid, so the test
//! lets `find_offsets` pick the phase.

use ninge_runner::golden::{
    assert_audio_deterministic, assert_audio_round_trip, assert_deterministic, assert_frame_hash,
    find_offsets, rom_audio,
};

const FRAMES: u64 = 4;
const PAIRS: usize = 600;
const HIGH: i64 = 0x7FFF;
const LOW: i64 = -0x7FFF;

/// The PSG level `i` samples after the start for `duty`: LOW for `7 - duty` samples, then HIGH.
fn psg(i: i64, duty: i64) -> i64 {
    if i % 8 >= 7 - duty { HIGH } else { LOW }
}

/// The noise register's outputs for the first `count` steps from `7FFFh`: a 1 shifted out is LOW
/// and toggles `6000h`, otherwise HIGH.
fn noise(count: usize) -> Vec<i64> {
    let mut x: u32 = 0x7FFF;
    (0..count)
        .map(|_| {
            let carry = x & 1 != 0;
            x >>= 1;
            if carry {
                x ^= 0x6000;
            }
            if carry { LOW } else { HIGH }
        })
        .collect()
}

/// One side's `i16` from the mixer units: the master stage, the bias and the 64-step scale.
fn side(units: i64) -> i64 {
    let level = ((units * 128) >> 21) + 0x200;
    (level.clamp(0, 0x3FF) - 0x200) * 64
}

/// The expected left and right `i16` at output sample `m` for the channels' first sounding output
/// samples `starts`, given the noise outputs.
fn expected(noise: &[i64], m: i64, starts: [i64; 3]) -> (i64, i64) {
    let wave8 = if m >= starts[0] {
        psg(m - starts[0], 3)
    } else {
        0
    };
    let wave9 = if m >= starts[1] {
        psg(m - starts[1], 6)
    } else {
        0
    };
    let hiss = if m >= starts[2] {
        noise[(m - starts[2]) as usize]
    } else {
        0
    };
    (side(wave8 * 64 + hiss * 32), side(wave9 * 64 + hiss * 32))
}

fn matches(audio: &[i16], noise: &[i64], starts: [i64; 3]) -> bool {
    audio.chunks(2).take(PAIRS).enumerate().all(|(m, pair)| {
        let (left, right) = expected(noise, m as i64, starts);
        (i64::from(pair[0]), i64::from(pair[1])) == (left, right)
    })
}

#[test]
fn given_the_noise_model_when_stepped_from_7fff_then_the_first_outputs_are_low_by_the_formula() {
    // 7FFFh -> 5FFFh -> 4FFFh -> 47FFh -> 43FFh: a 1 is shifted out each time, so four LOWs; the
    // register 43FFh then shifts out a 1 again, and so on until a 0 reaches bit 0.
    assert_eq!(&noise(4)[..], &[LOW, LOW, LOW, LOW]);
}

#[test]
fn given_the_spupsg_rom_when_4_frames_run_then_the_audio_is_two_duty_waves_and_noise_mixed() {
    let audio = rom_audio("spupsg", FRAMES);
    let steps = noise(PAIRS + 2);
    let phase = find_offsets(0..4, |starts| matches(&audio, &steps, starts));
    assert!(phase.is_some(), "no phase of the model fits the audio");
}

#[test]
fn given_the_spupsg_rom_when_4_frames_run_then_the_screens_and_audio_match_the_golden() {
    assert_frame_hash("spupsg", FRAMES);
}

#[test]
fn given_the_spupsg_rom_when_run_twice_then_the_frames_states_and_audio_agree() {
    assert_deterministic("spupsg", FRAMES);
    assert_audio_deterministic("spupsg", FRAMES);
}

#[test]
fn given_the_spupsg_rom_when_a_savestate_is_resumed_then_the_noise_register_carries_over() {
    assert_audio_round_trip("spupsg", 2, 2);
}
