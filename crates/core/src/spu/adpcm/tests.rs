// SPDX-License-Identifier: Apache-2.0
//! The IMA-ADPCM decode step. Expected values are worked out by hand from the formula in
//! `docs/hw/spu.md` ("IMA-ADPCM"), never from running the code.
//!
//! With `Index` 0 the step size is 7: `7 / 8 = 0`, `7 / 4 = 1`, `7 / 2 = 3` and `7 / 1 = 7`, so the
//! four low bits of a value add `0`, `1`, `3` and `7` and combine: 1 adds 1, 2 adds 3, 4 adds 7.
//! With `Index` 8 the size is 16: `2`, `4`, `8` and `16`.

use super::*;
use proptest::prelude::*;

/// One decode step case: the `(pcm, index, data)` in and the `(pcm, index)` out.
type Case = ((i32, u8, u8), (i32, u8));

/// The (pcm, index) each `(pcm, index, data)` should step to, and those it does not.
fn step_failures(cases: &[Case]) -> Vec<Case> {
    cases
        .iter()
        .map(|&(input, _)| (input, step(input.0, input.1, input.2)))
        .zip(cases)
        .filter(|(got, want)| got != *want)
        .map(|(got, _)| got)
        .collect()
}

#[test]
fn given_index_0_when_each_magnitude_bit_is_decoded_then_the_value_moves_by_the_hand_worked_diff() {
    // Index moves by -1 (data & 7 < 4) clamped to 0, or +2, +4, +6, +8 for 4, 5, 6, 7.
    let cases = [
        ((0, 0, 0b0000), (0, 0)),
        ((0, 0, 0b0001), (1, 0)),
        ((0, 0, 0b0010), (3, 0)),
        ((0, 0, 0b0011), (4, 0)),
        ((0, 0, 0b0100), (7, 2)),
        ((0, 0, 0b0111), (11, 8)),
    ];
    assert_eq!(step_failures(&cases), Vec::new());
}

#[test]
fn given_the_sign_bit_when_decoded_then_the_diff_is_subtracted_and_the_index_moves_the_same() {
    // Index 8 is a step of 16: 0b1100 subtracts 2 + 16 = 18 and moves the index by +2.
    let fixed = [
        ((100, 0, 0b1000), (100, 0)),
        ((100, 0, 0b1001), (99, 0)),
        ((100, 0, 0b1111), (89, 8)),
        ((0, 8, 0b1100), (-18, 10)),
    ];
    assert_eq!(step_failures(&fixed), Vec::new());
}

#[test]
fn given_a_diff_that_overflows_when_added_then_the_value_stops_at_7fff() {
    let cases = [
        ((0x7FFE, 0, 0b0111), (0x7FFF, 8)),
        ((0x7FFF, 10, 0b0111), (0x7FFF, 18)),
    ];
    assert_eq!(step_failures(&cases), Vec::new());
}

#[test]
fn given_a_diff_that_underflows_when_subtracted_then_the_value_stops_at_minus_7fff() {
    let cases = [
        ((-0x7FFE, 0, 0b1111), (-0x7FFF, 8)),
        ((-0x7FFF, 0, 0b1000), (-0x7FFF, 0)),
    ];
    assert_eq!(step_failures(&cases), Vec::new());
}

#[test]
fn given_a_start_of_minus_8000_when_added_to_then_it_is_not_clipped_and_when_subtracted_it_is_lifted()
 {
    // The add side only caps at 7FFFh; the subtract side floors at -7FFFh (GBATEK).
    let cases = [
        ((-0x8000, 0, 0b0000), (-0x8000, 0)),
        ((-0x8000, 0, 0b0001), (-0x7FFF, 0)),
        ((-0x8000, 0, 0b1000), (-0x7FFF, 0)),
    ];
    assert_eq!(step_failures(&cases), Vec::new());
}

#[test]
fn given_the_index_at_each_end_when_the_data_pushes_it_out_then_it_clamps_to_0_and_88() {
    let cases = [
        ((0, 0, 0b0000), (0, 0)),
        ((0, 88, 0b0111), (0x7FFF, 88)),
        ((0, 87, 0b0111), (0x7FFF, 88)),
    ];
    assert_eq!(step_failures(&cases), Vec::new());
}

#[test]
fn given_a_header_index_past_88_when_decoded_then_the_step_size_is_zero_and_the_index_clamps() {
    // Entries 89-127 are 0 (GBATEK's reproduction formula): no diff, and the index lands on 88
    // at most.
    let cases = [((50, 127, 0b0111), (50, 88)), ((50, 89, 0b0000), (50, 88))];
    assert_eq!(step_failures(&cases), Vec::new());
}

#[test]
fn given_the_table_when_read_at_its_ends_and_a_few_entries_then_it_is_the_listed_one() {
    assert_eq!(
        [
            STEP_TABLE[0],
            STEP_TABLE[8],
            STEP_TABLE[44],
            STEP_TABLE[88],
            STEP_TABLE[89],
            STEP_TABLE[127]
        ],
        [0x0007, 0x0010, 0x01EE, 0x7FFF, 0, 0]
    );
}

#[test]
fn given_a_header_word_when_split_then_the_value_is_signed_and_the_index_is_bits_16_to_22() {
    assert_eq!(
        [
            header(0x0000_8000),
            header(0x0058_7FFF),
            header(0xFFFF_FFFF)
        ],
        [(-0x8000, 0), (0x7FFF, 88), (-1, 127)]
    );
}

/// Decodes the data nibbles `data` (bytes, low nibble first) of a sound with the given loop start
/// and returns the PCM16 value after each of `ticks` decodes, wrapping at `wrap` data samples.
fn decode_run(header_word: u32, data: &[u8], loop_start: u64, wrap: u64, ticks: u64) -> Vec<i32> {
    let mut state = Adpcm::from_header(header_word);
    (0..ticks)
        .map(|tick| {
            let at = if tick < wrap {
                tick
            } else {
                loop_start + (tick - loop_start) % (wrap - loop_start)
            };
            state.decode(at, loop_start, data[(at / 2) as usize]);
            state.pcm()
        })
        .collect()
}

#[test]
fn given_a_four_byte_sound_when_decoded_then_the_values_follow_the_hand_computed_sequence() {
    // Header: pcm 0, index 0. Bytes 21h, 43h: nibbles 1, 2, 3, 4.
    // 1: +1 -> 1, index 0 (-1 clamped); 2: +3 -> 4, index 0; 3: +4 -> 8, index 0;
    // 4: +7 -> 15, index 2.
    assert_eq!(decode_run(0, &[0x21, 0x43], 0, 4, 4), vec![1, 4, 8, 15]);
}

#[test]
fn given_a_loop_when_it_wraps_then_the_state_at_the_loop_start_is_restored_each_pass() {
    // Loop starts at data sample 2 (after the nibbles 1, 2). The state there is pcm 4, index 0
    // (see the sequence above). Samples 2 and 3 decode to 8 and 15 on every pass.
    let values = decode_run(0, &[0x21, 0x43], 2, 4, 8);
    assert_eq!(values, vec![1, 4, 8, 15, 8, 15, 8, 15]);
}

#[test]
fn given_a_loop_starting_at_the_first_sample_when_it_wraps_then_the_header_state_is_restored() {
    let values = decode_run(0, &[0x21, 0x43], 0, 4, 8);
    assert_eq!(values, vec![1, 4, 8, 15, 1, 4, 8, 15]);
}

#[test]
fn given_a_decoder_state_with_a_wide_index_when_checked_then_it_is_not_well_formed() {
    let bad = Adpcm {
        index: 200,
        ..Adpcm::default()
    };
    assert_eq!(
        (Adpcm::default().well_formed(), bad.well_formed()),
        (true, false)
    );
}

proptest! {
    #[test]
    fn given_any_state_and_value_when_stepped_then_the_pcm_and_index_stay_in_range(
        pcm in -0x8000i32..=0x7FFF, index in 0u8..=127, data in 0u8..=255,
    ) {
        let (next_pcm, next_index) = step(pcm, index, data);
        prop_assert!((-0x8000..=0x7FFF).contains(&next_pcm));
        prop_assert!(next_index <= 88);
    }

    #[test]
    fn given_any_value_when_the_sign_bit_flips_then_the_index_moves_the_same_and_the_diff_mirrors(
        pcm in -0x7000i32..=0x7000, index in 0u8..=40, data in 0u8..8,
    ) {
        let (up, index_up) = step(pcm, index, data);
        let (down, index_down) = step(pcm, index, data | 8);
        prop_assert_eq!(index_up, index_down);
        prop_assert_eq!(up - pcm, pcm - down);
    }
}
