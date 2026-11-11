// SPDX-License-Identifier: Apache-2.0
//! The divider and square-root unit, from `docs/hw/divsqrt.md`.

use super::*;

/// A unit after a division of `numer` by `denom` in `mode`, started at time 0.
fn divided(mode: u32, numer: u64, denom: u64) -> DivSqrt {
    let mut unit = DivSqrt::new();
    unit.write(0, Width::Word, mode, 0);
    unit.write(NUMER, Width::Word, numer as u32, 0);
    unit.write(NUMER + 4, Width::Word, (numer >> 32) as u32, 0);
    unit.write(DENOM, Width::Word, denom as u32, 0);
    unit.write(DENOM + 4, Width::Word, (denom >> 32) as u32, 0);
    unit
}

fn rooted(mode: u32, param: u64) -> DivSqrt {
    let mut unit = DivSqrt::new();
    unit.write(SQRTCNT, Width::Word, mode, 0);
    unit.write(SQRT_PARAM, Width::Word, param as u32, 0);
    unit.write(SQRT_PARAM + 4, Width::Word, (param >> 32) as u32, 0);
    unit
}

fn peek_u64(unit: &DivSqrt, offset: u32, now: u64) -> u64 {
    u64::from(unit.peek(offset, Width::Word, now))
        | u64::from(unit.peek(offset + 4, Width::Word, now)) << 32
}

#[test]
fn given_a_new_unit_when_read_then_every_register_is_zero_and_nothing_is_busy() {
    let unit = DivSqrt::new();
    assert_eq!(
        (
            unit.peek(0, Width::Word, 0),
            peek_u64(&unit, RESULT, 0),
            peek_u64(&unit, REMAINDER, 0),
            unit.peek(SQRTCNT, Width::Word, 0),
            unit.peek(SQRT_RESULT, Width::Word, 0),
        ),
        (0, 0, 0, 0, 0)
    );
}

#[test]
fn given_the_divide_function_when_run_over_the_signs_then_it_truncates_toward_zero() {
    let cases: [(u64, u64, u64, u64); 4] = [
        (7, 2, 3, 1),
        (-7_i64 as u64, 2, -3_i64 as u64, -1_i64 as u64),
        (7, -2_i64 as u64, -3_i64 as u64, 1),
        (-7_i64 as u64, -2_i64 as u64, 3, -1_i64 as u64),
    ];
    let wrong: Vec<_> = cases
        .iter()
        .filter(|&&(n, d, q, r)| divide(2, n, d) != (q, r))
        .collect();
    assert_eq!(wrong, Vec::<&(u64, u64, u64, u64)>::new());
}

#[test]
fn given_mode_0_with_the_most_negative_over_minus_one_when_divided_then_the_quotient_is_positive_64_bit()
 {
    assert_eq!(
        divide(0, 0x8000_0000, 0xFFFF_FFFF),
        (0x0000_0000_8000_0000, 0)
    );
}

#[test]
fn given_mode_0_with_a_zero_denominator_when_divided_then_the_upper_quotient_follows_the_numerator_sign()
 {
    assert_eq!(
        (divide(0, 5, 0), divide(0, -5_i64 as u64, 0)),
        (
            (0x0000_0000_FFFF_FFFF, 5),
            (0xFFFF_FFFF_0000_0001, -5_i64 as u64)
        )
    );
}

#[test]
fn given_mode_2_with_the_most_negative_over_minus_one_when_divided_then_it_wraps_with_remainder_0()
{
    assert_eq!(
        divide(2, i64::MIN as u64, -1_i64 as u64),
        (i64::MIN as u64, 0)
    );
}

#[test]
fn given_mode_1_when_the_denominator_has_upper_bits_then_only_its_low_half_divides() {
    // `divsqrt.md`, "Division": mode 1 takes the low 32 bits of the denominator, signed.
    assert_eq!(divide(1, 100, 0x7777_7777_0000_0007), (14, 2));
}

#[test]
fn given_a_denominator_with_only_upper_bits_set_when_divided_in_mode_0_then_the_flag_is_clear() {
    // The flag looks at the full 64 bits; the division itself used the zero low half.
    let unit = divided(0, 5, 1 << 32);
    assert_eq!(unit.divcnt(1_000), 0);
}

#[test]
fn given_a_zero_denominator_when_divided_then_divcnt_bit_14_is_set_and_it_clears_on_the_next_division()
 {
    let mut unit = divided(2, 5, 0);
    let flagged = unit.divcnt(1_000);
    unit.write(DENOM, Width::Word, 1, 0);
    assert_eq!((flagged, unit.divcnt(1_000)), (0x4002, 0x0002));
}

#[test]
fn given_a_division_started_at_time_0_when_read_around_its_end_then_busy_lasts_36_master_cycles_in_mode_0()
 {
    // 18 bus cycles x 2 (`divsqrt.md`, "Duration").
    let unit = divided(0, 100, 7);
    assert_eq!(
        (unit.divcnt(0), unit.divcnt(35), unit.divcnt(36)),
        (BUSY_BIT, BUSY_BIT, 0)
    );
}

#[test]
fn given_a_division_started_at_time_0_when_read_around_its_end_then_busy_lasts_68_master_cycles_in_modes_1_and_2()
 {
    let (one, two) = (divided(1, 100, 7), divided(2, 100, 7));
    assert_eq!(
        (
            one.divcnt(67),
            one.divcnt(68),
            two.divcnt(67) & BUSY_BIT,
            two.divcnt(68) & BUSY_BIT
        ),
        (BUSY_BIT | 1, 1, BUSY_BIT, 0)
    );
}

#[test]
fn given_a_busy_division_when_an_operand_is_written_again_then_the_count_restarts() {
    let mut unit = divided(0, 100, 7);
    unit.write(DENOM, Width::Word, 9, 30);
    assert_eq!((unit.divcnt(65), unit.divcnt(66)), (BUSY_BIT, 0));
}

#[test]
fn given_a_square_root_started_at_time_0_when_read_around_its_end_then_busy_lasts_26_master_cycles()
{
    // 13 bus cycles x 2, in either mode.
    let (zero, one) = (rooted(0, 100), rooted(1, 100));
    assert_eq!(
        (
            zero.sqrtcnt(25),
            zero.sqrtcnt(26),
            one.sqrtcnt(25),
            one.sqrtcnt(26)
        ),
        (BUSY_BIT, 0, BUSY_BIT | 1, 1)
    );
}

#[test]
fn given_the_square_root_function_when_run_over_edges_then_it_is_the_floor() {
    let cases: [(u8, u64, u32); 6] = [
        (0, 0, 0),
        (0, 99, 9),
        (0, 0xFFFF_FFFF, 65_535),
        (0, 0xFFFF_FFFF_0000_0064, 10),
        (1, 1 << 32, 65_536),
        (1, u64::MAX, u32::MAX),
    ];
    let wrong: Vec<_> = cases
        .iter()
        .filter(|&&(mode, param, root)| square_root(mode, param) != root)
        .collect();
    assert_eq!(wrong, Vec::<&(u8, u64, u32)>::new());
}

#[test]
fn given_byte_writes_to_the_numerator_when_the_last_one_lands_then_the_quotient_uses_all_of_them() {
    let mut unit = DivSqrt::new();
    unit.write(DENOM, Width::Byte, 2, 0);
    unit.write(NUMER, Width::Byte, 0x10, 0);
    unit.write(NUMER + 1, Width::Byte, 0x01, 0);
    assert_eq!(unit.peek(RESULT, Width::Word, 99), 0x88);
}

#[test]
fn given_a_write_to_a_result_register_when_read_back_then_it_is_unchanged_and_nothing_started() {
    let mut unit = divided(0, 100, 7);
    unit.write(RESULT, Width::Word, 0x1234, 1_000);
    unit.write(REMAINDER, Width::Word, 0x1234, 1_000);
    unit.write(SQRT_RESULT, Width::Word, 0x1234, 1_000);
    assert_eq!(
        (
            unit.peek(RESULT, Width::Word, 1_000),
            unit.peek(REMAINDER, Width::Word, 1_000),
            unit.peek(0, Width::Word, 1_001) & BUSY_BIT,
        ),
        (14, 2, 0)
    );
}

#[test]
fn given_a_write_of_ffff_to_divcnt_when_read_back_then_only_the_mode_is_kept() {
    let mut unit = DivSqrt::new();
    unit.write(0, Width::Word, 0xFFFF_FFFF, 0);
    assert_eq!(unit.divcnt(1_000), 0x4003);
}

#[test]
fn given_a_write_in_the_holes_when_read_then_it_reads_zero_and_starts_nothing() {
    let mut unit = DivSqrt::new();
    unit.write(0x04, Width::Word, 0xFFFF_FFFF, 0);
    unit.write(0x3C + 4, Width::Byte, 0xFF, 0);
    assert_eq!(
        (
            unit.peek(0x04, Width::Word, 0),
            unit.peek(0, Width::Word, 0)
        ),
        (0, 0)
    );
}

#[test]
fn given_a_busy_end_far_beyond_the_longest_operation_when_checked_then_the_state_is_corrupt() {
    let mut corrupt = DivSqrt::new();
    corrupt.div_busy_until = 1_000_000;
    assert_eq!(
        (DivSqrt::new().is_well_formed(0), corrupt.is_well_formed(0)),
        (true, false)
    );
}

#[test]
fn given_the_debug_accessors_when_a_division_and_a_root_ran_then_they_return_the_results() {
    let mut unit = divided(0, 100, 7);
    unit.write(SQRTCNT, Width::Word, 0, 0);
    unit.write(SQRT_PARAM, Width::Word, 81, 0);
    assert_eq!((unit.division_result(), unit.sqrt_result()), ((14, 2), 9));
}
