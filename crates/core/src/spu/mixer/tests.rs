// SPDX-License-Identifier: Apache-2.0
//! The mixer arithmetic, one field at a time. Expected values come from GBATEK's width table as
//! `docs/hw/spu.md` ("Volume and panning", "The mixer") gives it, worked out by hand.
//!
//! The reference case: PCM16 `4000h`, volume 127 (read as 128), divider 0, panning 0 (all left).
//! `4000h << 4` times 128 times 128 is `4000h * 16 * 4000h`, and `>> 10` leaves `4000h * 256`: the
//! sample in 1/256 units, `40_0000h`.

use super::*;

/// The reference sample: a quarter of the PCM16 range.
const S: i32 = 0x4000;
/// The reference left value in 1/256 units.
const FULL_LEFT: i64 = 0x40_0000;

/// The rows `(name, got, want)` that differ.
fn differing(rows: &[(&'static str, i64, i64)]) -> Vec<(&'static str, i64, i64)> {
    rows.iter().filter(|r| r.1 != r.2).copied().collect()
}

#[test]
fn given_the_reference_sample_when_volume_127_and_pan_0_then_the_left_value_is_the_sample_times_256()
 {
    assert_eq!(channel(S, 0, 127, 0), (FULL_LEFT, 0));
}

#[test]
fn given_the_volume_register_when_126_127_64_and_0_then_it_scales_by_n_over_128_with_127_as_128() {
    let rows = [
        ("126", channel(S, 0, 126, 0).0, FULL_LEFT / 128 * 126),
        ("127", channel(S, 0, 127, 0).0, FULL_LEFT),
        ("64", channel(S, 0, 64, 0).0, FULL_LEFT / 2),
        ("0", channel(S, 0, 0, 0).0, 0),
    ];
    assert_eq!(differing(&rows), Vec::new());
}

#[test]
fn given_the_divider_selector_when_0_to_3_then_the_value_is_divided_by_1_2_4_and_16() {
    let rows = [
        ("div1", channel(S, 0, 127, 0).0, FULL_LEFT),
        ("div2", channel(S, 1, 127, 0).0, FULL_LEFT / 2),
        ("div4", channel(S, 2, 127, 0).0, FULL_LEFT / 4),
        ("div16", channel(S, 3, 127, 0).0, FULL_LEFT / 16),
    ];
    assert_eq!(differing(&rows), Vec::new());
}

#[test]
fn given_the_panning_when_0_64_and_127_then_the_sides_split_n_and_128_minus_n() {
    let rows = [
        ("0 left", channel(S, 0, 127, 0).0, FULL_LEFT),
        ("0 right", channel(S, 0, 127, 0).1, 0),
        ("64 left", channel(S, 0, 127, 64).0, FULL_LEFT / 2),
        ("64 right", channel(S, 0, 127, 64).1, FULL_LEFT / 2),
        ("127 left", channel(S, 0, 127, 127).0, 0),
        ("127 right", channel(S, 0, 127, 127).1, FULL_LEFT),
        ("32 left", channel(S, 0, 127, 32).0, FULL_LEFT / 4 * 3),
        ("32 right", channel(S, 0, 127, 32).1, FULL_LEFT / 4),
    ];
    assert_eq!(differing(&rows), Vec::new());
}

#[test]
fn given_a_small_negative_and_positive_sample_when_scaled_then_it_rounds_toward_minus_infinity() {
    // sample 1 at volume 1 and panning 1: 16 * 1 * 127 = 2032, >> 10 = 1; sample -1: -2032 >> 10 = -2.
    assert_eq!((channel(1, 0, 1, 1).0, channel(-1, 0, 1, 1).0), (1, -2));
}

#[test]
fn given_the_master_volume_when_127_64_and_0_then_it_scales_by_n_over_128_over_64() {
    // 40_0000h * 128 >> 21 = 256 (a quarter of the 10-bit range's 1024).
    let rows = [
        ("127", master(FULL_LEFT, 127), 256),
        ("64", master(FULL_LEFT, 64), 128),
        ("0", master(FULL_LEFT, 0), 0),
        ("126", master(FULL_LEFT, 126), 252),
    ];
    assert_eq!(differing(&rows), Vec::new());
}

#[test]
fn given_a_tiny_negative_sum_when_the_master_drops_the_fraction_then_it_rounds_down() {
    assert_eq!(master(-1, 127), -1);
}

#[test]
fn given_a_signal_and_a_bias_when_added_then_the_level_is_clipped_to_0_and_3ff() {
    let rows = [
        ("zero", i64::from(level(0, 0x200)), 0x200),
        ("sum", i64::from(level(100, 0x200)), 0x264),
        ("low clip", i64::from(level(-0x300, 0x200)), 0),
        ("high clip", i64::from(level(0x300, 0x200)), 0x3FF),
        ("bias 0", i64::from(level(5, 0)), 5),
        ("bias max", i64::from(level(0, 0x3FF)), 0x3FF),
    ];
    assert_eq!(differing(&rows), Vec::new());
}

#[test]
fn given_the_levels_when_converted_then_the_medium_is_zero_and_the_range_is_minus_8000_to_7fc0() {
    assert_eq!(
        [to_i16(0), to_i16(0x200), to_i16(0x3FF), to_i16(0x201)],
        [-0x8000, 0, 0x7FC0, 64]
    );
}
