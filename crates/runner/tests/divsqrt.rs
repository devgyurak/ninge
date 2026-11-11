// SPDX-License-Identifier: Apache-2.0
//! The hardware divider and square root, observed through our own `divsqrt` test ROM
//! (`testroms/divsqrt/`). The ARM9 records what it reads in main RAM at 0x0200_4000; the slot
//! layout is at the top of `arm9.s`. Expected values come from `docs/hw/divsqrt.md` ("Behavior",
//! "Edge cases"), worked out here, never from running the ROM:
//!
//! - 100 / 7 = 14 remainder 2.
//! - -7 / 2 = -3 remainder -1 (truncation toward zero, the remainder takes the numerator's sign).
//! - Mode 0 by zero: the remainder is the numerator; the quotient is -1 for a non-negative
//!   numerator, and mode 0 inverts the upper half of the sign-extended quotient, so it is
//!   `0000_0000_FFFF_FFFFh`. The flag (DIVCNT bit 14) is set because the full 64-bit denominator
//!   is zero.
//! - `80000000h / FFFFFFFFh` in mode 0: the quotient is `-MAX`, `FFFF_FFFF_8000_0000h` sign
//!   extended, upper half inverted: `0000_0000_8000_0000h`.
//! - 2^32 / 3 = 1,431,655,765 = 55555555h remainder 1 (4,294,967,295 is divisible by 3).
//! - 2^40 / (2^32 + 1) = 255 remainder 2^40 - 255 x (2^32 + 1) = 4,294,967,041 = FFFF_FF01h.
//! - Mode 2 by zero with numerator -5: the quotient is +1 and the remainder is the numerator.
//! - `-2^63 / -1` in mode 2: the quotient stays `-2^63` and the remainder is 0.
//! - -2^32 / -3 in mode 1 is +1,431,655,765 with remainder -1.
//! - sqrt(1,000,000) = 1000; sqrt(2^32) = 65,536; sqrt(2^64 - 1) = FFFF_FFFFh; sqrt(99) = 9.

use ninge_core::Machine;
use ninge_runner::{load_rom_file, testrom_path};

const RECORD: u32 = 0x0200_4000;

/// The ARM9 finishes in a fraction of a frame.
const FRAMES: u32 = 2;

/// A mismatch: (slot, name, expected, got).
type Mismatch = (u32, &'static str, u32, u32);

/// Expected record: (slot, name, expected value).
type Row = (u32, &'static str, u32);

fn mismatches(rows: &[Row]) -> Vec<Mismatch> {
    let rom = load_rom_file(&testrom_path("divsqrt")).expect("divsqrt ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("divsqrt ROM has a valid header");
    (0..FRAMES).for_each(|_| machine.run_frame());
    rows.iter()
        .map(|&(slot, name, want)| {
            (
                slot,
                name,
                want,
                machine.inspect().arm9_peek32(RECORD + slot * 4),
            )
        })
        .filter(|&(_, _, want, got)| want != got)
        .collect()
}

#[test]
fn given_divsqrt_rom_when_run_then_the_arm9_finishes_and_writes_its_sentinel() {
    let rows: &[Row] = &[(120, "progress", 1), (121, "sentinel", 0x4449_5637)];
    assert_eq!(mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_a_32_bit_division_when_it_completes_then_quotient_and_remainder_are_exact() {
    let rows: &[Row] = &[
        (2, "100/7 quotient", 14),
        (3, "100/7 quotient high", 0),
        (4, "100/7 remainder", 2),
        (5, "100/7 remainder high", 0),
    ];
    assert_eq!(mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_a_negative_numerator_when_divided_in_mode_0_then_it_truncates_and_sign_extends() {
    let rows: &[Row] = &[
        (10, "-7/2 quotient", 0xFFFF_FFFD),
        (11, "-7/2 quotient high", 0xFFFF_FFFF),
        (12, "-7/2 remainder", 0xFFFF_FFFF),
        (13, "-7/2 remainder high", 0xFFFF_FFFF),
    ];
    assert_eq!(mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_garbage_in_the_upper_halves_when_divided_in_mode_0_then_only_the_low_halves_count() {
    // `divsqrt.md`, "Edge cases": mode 0 ignores the upper 32 bits (the flag aside).
    let rows: &[Row] = &[
        (74, "quotient", 0xFFFF_FFFD),
        (75, "quotient high", 0xFFFF_FFFF),
        (76, "remainder", 0xFFFF_FFFF),
        (77, "remainder high", 0xFFFF_FFFF),
        (73, "DIVCNT", 0),
    ];
    assert_eq!(mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_a_zero_denominator_in_mode_0_when_divided_then_the_flag_is_set_and_the_upper_quotient_is_inverted()
 {
    let rows: &[Row] = &[
        (17, "DIVCNT", 0x4000),
        (18, "quotient", 0xFFFF_FFFF),
        (19, "quotient high", 0),
        (20, "remainder", 5),
        (21, "remainder high", 0),
    ];
    assert_eq!(mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_the_most_negative_number_over_minus_one_in_mode_0_when_divided_then_the_result_is_positive_64_bit()
 {
    let rows: &[Row] = &[
        (25, "DIVCNT", 0),
        (26, "quotient", 0x8000_0000),
        (27, "quotient high", 0),
        (28, "remainder", 0),
        (29, "remainder high", 0),
    ];
    assert_eq!(mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_a_64_bit_numerator_in_mode_1_when_divided_by_3_then_it_is_55555555h_remainder_1() {
    let rows: &[Row] = &[
        (33, "DIVCNT", 1),
        (34, "quotient", 0x5555_5555),
        (35, "quotient high", 0),
        (36, "remainder", 1),
        (37, "remainder high", 0),
    ];
    assert_eq!(mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_mode_3_when_dividing_then_it_acts_as_mode_1() {
    let rows: &[Row] = &[
        (66, "quotient", 0x5555_5555),
        (67, "quotient high", 0),
        (68, "remainder", 1),
        (69, "remainder high", 0),
    ];
    assert_eq!(mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_two_negative_operands_in_mode_1_when_divided_then_the_quotient_is_positive_and_the_remainder_negative()
 {
    let rows: &[Row] = &[
        (82, "quotient", 0x5555_5555),
        (83, "quotient high", 0),
        (84, "remainder", 0xFFFF_FFFF),
        (85, "remainder high", 0xFFFF_FFFF),
    ];
    assert_eq!(mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_a_64_bit_denominator_in_mode_2_when_divided_then_the_quotient_is_255_remainder_ffffff01h()
{
    let rows: &[Row] = &[
        (41, "DIVCNT", 2),
        (42, "quotient", 255),
        (43, "quotient high", 0),
        (44, "remainder", 0xFFFF_FF01),
        (45, "remainder high", 0),
    ];
    assert_eq!(mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_a_zero_denominator_in_mode_2_with_a_negative_numerator_when_divided_then_the_quotient_is_plus_one()
 {
    let rows: &[Row] = &[
        (49, "DIVCNT", 0x4002),
        (50, "quotient", 1),
        (51, "quotient high", 0),
        (52, "remainder", 0xFFFF_FFFB),
        (53, "remainder high", 0xFFFF_FFFF),
    ];
    assert_eq!(mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_the_most_negative_64_bit_number_over_minus_one_in_mode_2_when_divided_then_the_quotient_stays_negative()
 {
    let rows: &[Row] = &[
        (57, "DIVCNT", 2),
        (58, "quotient", 0),
        (59, "quotient high", 0x8000_0000),
        (60, "remainder", 0),
        (61, "remainder high", 0),
    ];
    assert_eq!(mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_a_division_when_the_last_operand_is_written_then_busy_is_set_until_it_completes() {
    // `divsqrt.md`, "Duration": the read right after the write sees the busy bit, the read after
    // the wait does not (the DIVCNT slots above carry no bit 15).
    let rows: &[Row] = &[
        (0, "mode 0 busy", 0x8000),
        (32, "mode 1 busy", 0x8000),
        (40, "mode 2 busy", 0x8000),
    ];
    assert_eq!(mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_square_roots_when_they_complete_then_the_result_is_the_floor_of_the_root() {
    let rows: &[Row] = &[
        (98, "sqrt 1000000", 1000),
        (102, "sqrt 2^32", 65_536),
        (106, "sqrt 2^64-1", 0xFFFF_FFFF),
        (110, "sqrt of the low half only", 10),
        (114, "sqrt 99", 9),
    ];
    assert_eq!(mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_a_square_root_when_the_parameter_is_written_then_busy_is_set_until_it_completes() {
    let rows: &[Row] = &[
        (96, "busy right after", 0x8000),
        (97, "SQRTCNT after the wait, mode 0", 0),
        (101, "SQRTCNT after the wait, mode 1", 1),
    ];
    assert_eq!(mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_a_write_to_a_result_register_when_read_back_then_it_changes_nothing_and_starts_nothing() {
    // The last division's quotient (op 10) is still there, and the unit is not busy.
    let rows: &[Row] = &[
        (118, "DIV_RESULT after a write", 0x5555_5555),
        (119, "busy after the write", 0),
    ];
    assert_eq!(mismatches(rows), Vec::<Mismatch>::new());
}
