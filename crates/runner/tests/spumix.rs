// SPDX-License-Identifier: Apache-2.0
//! The mixer, observed through our own `spumix` test ROM (`testroms/spumix/`). Four channels each loop
//! one constant PCM8 byte (`40h`, `20h`, `10h`, `08h`: PCM16 `4000h`, `2000h`, `1000h`, `0800h`), so the
//! output is a steady level that the registers decide, and the ARM7 steps through nine settings with a
//! busy wait between them (about 200 output samples each; see the ROM's header for the settings).
//!
//! The expected levels are worked out by hand from `docs/hw/spu.md` ("Volume and panning", "The
//! mixer", "The gates"): channel 0 is full left (`4000h`), channel 1 full right (`2000h`), channel 2
//! centred (`0800h` on each side at master 127), channel 3 on the left (`0800h`). With master volume
//! 127 and the bias `200h` a channel's value comes out unchanged (to a multiple of 64) on its side.
//! The nine plateaus below are the sums the ROM's settings give, side by side as `(left, right)`:
//!
//! | step | setting | left | right |
//! |---|---|---|---|
//! | A | channel 0 | `4000h` | 0 |
//! | B | + channel 1 | `4000h` | `2000h` |
//! | C | + channel 2 | `4800h` | `2800h` |
//! | D | master volume 64 | `2400h` | `1400h` |
//! | E | + channel 3 | `2800h` | `1400h` |
//! | F | master 127, channel 1 not mixed | `5000h` | `0800h` |
//! | G | left output from channel 3 | `0800h` | `0800h` |
//! | H | all mixed, bias `3FFh`: both clip | `7FC0h` | `7FC0h` |
//! | I | master enable off, bias `240h` | `1000h` | `1000h` |

use ninge_runner::golden::{
    assert_audio_deterministic, assert_audio_round_trip, assert_deterministic, assert_frame_hash,
    rom_audio,
};

const FRAMES: u64 = 5;
/// A plateau is at least this many output samples; the one-sample change between two settings and
/// the few between two register writes are shorter.
const MIN_PLATEAU: usize = 50;

/// The runs of at least [`MIN_PLATEAU`] equal `(left, right)` pairs, in order, as `(left, right)`.
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

#[test]
fn given_the_spumix_rom_when_5_frames_run_then_the_nine_settings_give_the_hand_worked_levels() {
    let expected: Vec<(i16, i16)> = vec![
        (0x4000, 0),
        (0x4000, 0x2000),
        (0x4800, 0x2800),
        (0x2400, 0x1400),
        (0x2800, 0x1400),
        (0x5000, 0x0800),
        (0x0800, 0x0800),
        (0x7FC0, 0x7FC0),
        (0x1000, 0x1000),
    ];
    assert_eq!(plateaus(&rom_audio("spumix", FRAMES)), expected);
}

#[test]
fn given_the_spumix_rom_when_5_frames_run_then_the_screens_and_audio_match_the_golden() {
    assert_frame_hash("spumix", FRAMES);
}

#[test]
fn given_the_spumix_rom_when_run_twice_then_the_frames_states_and_audio_agree() {
    assert_deterministic("spumix", FRAMES);
    assert_audio_deterministic("spumix", FRAMES);
}

#[test]
fn given_the_spumix_rom_when_a_savestate_is_resumed_then_the_audio_matches_the_uninterrupted_run() {
    assert_audio_round_trip("spumix", 2, 3);
}
