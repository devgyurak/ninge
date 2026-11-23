// SPDX-License-Identifier: Apache-2.0
//! IMA-ADPCM playback, observed through our own `spuadpcm` test ROM (`testroms/spuadpcm/`). The ARM7
//! copies a header word and seven words of 4-bit values into RAM and starts two channels on them at
//! TMR `FE00h`, one channel sample for each output sample (2,048 master cycles): channel 0 loops
//! (PNT 1, LEN 7) on the left at full volume, channel 1 plays the 56 samples once on the right at a
//! quarter volume (divider 2).
//!
//! The expected samples come from an ADPCM decoder written here from the formula and the tables of
//! `docs/hw/spu.md` ("IMA-ADPCM"), and from the mixer steps of "Volume and panning" and "The mixer";
//! the code below shares nothing with `ninge-core`. For channel 0 (volume 127, left, master 127) a
//! decoded value `s` is a level of `s >> 6`; for channel 1 (divider 2, volume 127, right) it is
//! `s >> 8`; a level is 64 `i16` steps. Which output sample first carries each channel depends on where
//! its start write falls on the 2,048-cycle grid, so the test lets `find_offsets` pick the phase.

use ninge_runner::golden::{
    assert_audio_deterministic, assert_audio_round_trip, assert_deterministic, assert_frame_hash,
    find_offsets, rom_audio,
};

const FRAMES: u64 = 4;
const PAIRS: usize = 300;

/// The data words of the ROM: the header (value 0, index 20) and seven words of 4-bit values.
const WORDS: [u32; 8] = [
    0x0014_0000,
    0x7531_7531,
    0xFDB9_FDB9,
    0x3333_3333,
    0xBBBB_BBBB,
    0x5555_5555,
    0xDDDD_DDDD,
    0x1234_5678,
];

/// The step sizes by table index, from GBATEK's table as `docs/hw/spu.md` lists it.
const STEPS: [i64; 89] = [
    0x0007, 0x0008, 0x0009, 0x000A, 0x000B, 0x000C, 0x000D, 0x000E, 0x0010, 0x0011, 0x0013, 0x0015,
    0x0017, 0x0019, 0x001C, 0x001F, 0x0022, 0x0025, 0x0029, 0x002D, 0x0032, 0x0037, 0x003C, 0x0042,
    0x0049, 0x0050, 0x0058, 0x0061, 0x006B, 0x0076, 0x0082, 0x008F, 0x009D, 0x00AD, 0x00BE, 0x00D1,
    0x00E6, 0x00FD, 0x0117, 0x0133, 0x0151, 0x0173, 0x0198, 0x01C1, 0x01EE, 0x0220, 0x0256, 0x0292,
    0x02D4, 0x031C, 0x036C, 0x03C3, 0x0424, 0x048E, 0x0502, 0x0583, 0x0610, 0x06AB, 0x0756, 0x0812,
    0x08E0, 0x09C3, 0x0ABD, 0x0BD0, 0x0CFF, 0x0E4C, 0x0FBA, 0x114C, 0x1307, 0x14EE, 0x1706, 0x1954,
    0x1BDC, 0x1EA5, 0x21B6, 0x2515, 0x28CA, 0x2CDF, 0x315B, 0x364B, 0x3BB9, 0x41B2, 0x4844, 0x4F7E,
    0x5771, 0x602F, 0x69CE, 0x7462, 0x7FFF,
];
const INDEX_MOVES: [i64; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

/// The 56 decoded PCM16 values, one for each 4-bit value, low nibble first, from the header's start.
fn decoded() -> Vec<i64> {
    let mut pcm = i64::from(WORDS[0] as u16 as i16);
    let mut index = i64::from((WORDS[0] >> 16) & 0x7F);
    let nibbles = WORDS[1..]
        .iter()
        .flat_map(|word| (0..8).map(move |n| ((word >> (4 * n)) & 0xF) as usize));
    nibbles
        .map(|nibble| {
            let step = STEPS[index as usize];
            let mut diff = step / 8;
            if nibble & 1 != 0 {
                diff += step / 4;
            }
            if nibble & 2 != 0 {
                diff += step / 2;
            }
            if nibble & 4 != 0 {
                diff += step;
            }
            pcm = if nibble & 8 == 0 {
                (pcm + diff).min(0x7FFF)
            } else {
                (pcm - diff).max(-0x7FFF)
            };
            index = (index + INDEX_MOVES[nibble & 7]).clamp(0, 88);
            pcm
        })
        .collect()
}

/// The expected left and right `i16` at output sample `m` for the channels' first sounding output
/// samples `starts`, given the decoded values.
fn expected(values: &[i64], m: i64, starts: [i64; 2]) -> (i64, i64) {
    let len = values.len() as i64;
    let left = if m >= starts[0] {
        (values[((m - starts[0]) % len) as usize] >> 6) * 64
    } else {
        0
    };
    let played = m - starts[1];
    let right = if (0..len).contains(&played) {
        (values[played as usize] >> 8) * 64
    } else {
        0
    };
    (left, right)
}

fn matches(audio: &[i16], values: &[i64], starts: [i64; 2]) -> bool {
    audio.chunks(2).take(PAIRS).enumerate().all(|(m, pair)| {
        let (left, right) = expected(values, m as i64, starts);
        (i64::from(pair[0]), i64::from(pair[1])) == (left, right)
    })
}

#[test]
fn given_the_adpcm_data_when_decoded_by_the_model_then_it_has_56_values_that_move_up_and_down() {
    // A guard on the model: the first values by hand. Header 0, index 20 (step 50); nibble 1 (the
    // low nibble of 7531h's low byte) adds 50 / 8 + 50 / 4 = 6 + 12 = 18, index 19.
    let values = decoded();
    assert_eq!((values.len(), values[0]), (56, 18));
}

#[test]
fn given_the_spuadpcm_rom_when_4_frames_run_then_the_audio_is_the_decoded_loop_and_one_shot() {
    let audio = rom_audio("spuadpcm", FRAMES);
    let values = decoded();
    let phase = find_offsets(0..4, |starts| matches(&audio, &values, starts));
    assert!(phase.is_some(), "no phase of the model fits the audio");
}

#[test]
fn given_the_spuadpcm_rom_when_4_frames_run_then_the_screens_and_audio_match_the_golden() {
    assert_frame_hash("spuadpcm", FRAMES);
}

#[test]
fn given_the_spuadpcm_rom_when_run_twice_then_the_frames_states_and_audio_agree() {
    assert_deterministic("spuadpcm", FRAMES);
    assert_audio_deterministic("spuadpcm", FRAMES);
}

#[test]
fn given_the_spuadpcm_rom_when_a_savestate_is_resumed_then_the_decoder_state_carries_over() {
    assert_audio_round_trip("spuadpcm", 2, 2);
}
