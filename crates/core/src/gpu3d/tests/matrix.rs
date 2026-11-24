// SPDX-License-Identifier: Apache-2.0
//! The matrix arithmetic and the shapes of the matrix commands as pure functions. Expected values
//! come from `docs/hw/geometry-engine.md` ("Matrices") and GBATEK's "Matrix Types" and "Maths
//! Basics" (full-precision sums, one shift right by 12 at the end), worked out by hand.

use crate::gpu3d::matrix::{IDENTITY, Matrix, ONE, multiply, shape};
use proptest::prelude::*;

/// A matrix whose elements are `0, 1, 2, ...` times `step`.
fn counting(step: i32) -> Matrix {
    std::array::from_fn(|i| i as i32 * step)
}

/// The same product with 128-bit arithmetic, as a reference for the proptest.
fn reference(a: &Matrix, b: &Matrix) -> Matrix {
    std::array::from_fn(|index| {
        let (y, x) = (index / 4, index % 4);
        let sum: i128 = (0..4)
            .map(|k| i128::from(a[4 * y + k]) * i128::from(b[4 * k + x]))
            .sum();
        (sum >> 12) as i32
    })
}

#[test]
fn given_the_identity_when_it_multiplies_a_matrix_then_the_matrix_is_unchanged() {
    let m = counting(0x123);
    assert_eq!((multiply(&IDENTITY, &m), multiply(&m, &IDENTITY)), (m, m));
}

#[test]
fn given_two_scalings_when_multiplied_then_the_product_keeps_the_12_bit_fraction() {
    // 1.5 x 1.5 = 2.25: 1800h x 1800h = 240_0000h, shifted right by 12 is 2400h.
    let mut a = IDENTITY;
    a[0] = 0x1800;
    assert_eq!(multiply(&a, &a)[0], 0x2400);
}

/// Row 0 holds 800h, 800h, 0, 0; the rest is 0.
fn half_row() -> Matrix {
    std::array::from_fn(|i| [0x800, 0x800][i.min(2) % 2] * i32::from(i < 2))
}

/// Column 0 holds 1, 1, 0, 0; the rest is 0.
fn ones_column() -> Matrix {
    std::array::from_fn(|i| i32::from(i == 0 || i == 4))
}

#[test]
fn given_a_row_and_a_column_when_they_are_multiplied_then_the_sum_is_shifted_once_at_the_end() {
    // Row 0 of a is 800h, 800h, 0, 0 and column 0 of b is 1, 1, 0, 0: each product is 800h and
    // shifted alone would be 0; summed first they are 1000h, which shifts to 1.
    assert_eq!(multiply(&half_row(), &ones_column())[0], 1);
}

#[test]
fn given_a_negative_product_when_shifted_then_it_rounds_toward_negative_infinity() {
    let mut a = IDENTITY;
    a[0] = -1;
    let mut b = IDENTITY;
    b[0] = 1;
    // -1 x 1 = -1, and -1 >> 12 is -1 (arithmetic shift), not 0.
    assert_eq!(multiply(&a, &b)[0], -1);
}

#[test]
fn given_a_product_that_overflows_32_bits_when_multiplied_then_it_wraps() {
    let mut a = IDENTITY;
    a[0] = 0x4000_0000;
    // 4000_0000h x 4.0 (4000h) = 1_0000_0000_0000h in 64 bits; >> 12 = 1_0000_0000h; low 32 bits 0.
    let mut b = IDENTITY;
    b[0] = 0x4000;
    assert_eq!(multiply(&a, &b)[0], 0);
}

/// The 16 elements `0..16` as the parameter words of the 4x4 commands.
fn words(count: usize) -> Vec<u32> {
    (1..=count as u32).collect()
}

#[test]
fn given_sixteen_parameters_when_shaped_as_4x4_then_they_are_the_elements_in_order() {
    let want: Matrix = std::array::from_fn(|i| i as i32 + 1);
    assert_eq!(
        (shape(0x16, &words(16)), shape(0x18, &words(16))),
        (want, want)
    );
}

#[test]
fn given_twelve_parameters_when_shaped_as_4x3_then_the_fourth_column_is_zero_zero_zero_one() {
    let want: Matrix = [
        1, 2, 3, 0, //
        4, 5, 6, 0, //
        7, 8, 9, 0, //
        10, 11, 12, ONE,
    ];
    assert_eq!(
        (shape(0x17, &words(12)), shape(0x19, &words(12))),
        (want, want)
    );
}

#[test]
fn given_nine_parameters_when_shaped_as_3x3_then_the_rest_is_the_identity() {
    let want: Matrix = [
        1, 2, 3, 0, //
        4, 5, 6, 0, //
        7, 8, 9, 0, //
        0, 0, 0, ONE,
    ];
    assert_eq!(shape(0x1A, &words(9)), want);
}

#[test]
fn given_three_parameters_when_shaped_as_a_scale_then_they_are_the_diagonal() {
    let want: Matrix = [
        1, 0, 0, 0, //
        0, 2, 0, 0, //
        0, 0, 3, 0, //
        0, 0, 0, ONE,
    ];
    assert_eq!(shape(0x1B, &words(3)), want);
}

#[test]
fn given_three_parameters_when_shaped_as_a_translation_then_they_are_the_bottom_row() {
    let want: Matrix = [
        ONE, 0, 0, 0, //
        0, ONE, 0, 0, //
        0, 0, ONE, 0, //
        1, 2, 3, ONE,
    ];
    assert_eq!(shape(0x1C, &words(3)), want);
}

fn any_matrix() -> impl Strategy<Value = Matrix> {
    proptest::array::uniform16(any::<i32>())
}

/// Elements small enough that the sum of four products fits 64 bits, where the reference and the
/// accumulator agree.
fn bounded_matrix() -> impl Strategy<Value = Matrix> {
    proptest::array::uniform16(-(1 << 30)..(1 << 30))
}

proptest! {
    #[test]
    fn given_bounded_matrices_when_multiplied_then_the_result_matches_the_128_bit_reference(
        a in bounded_matrix(),
        b in bounded_matrix(),
    ) {
        prop_assert_eq!(multiply(&a, &b), reference(&a, &b));
    }

    #[test]
    fn given_any_matrices_when_multiplied_then_it_never_panics(a in any_matrix(), b in any_matrix()) {
        prop_assert_eq!(multiply(&a, &b).len(), 16);
    }

    #[test]
    fn given_any_matrix_when_multiplied_by_the_identity_then_it_is_unchanged(m in any_matrix()) {
        prop_assert_eq!(multiply(&IDENTITY, &m), m);
        prop_assert_eq!(multiply(&m, &IDENTITY), m);
    }
}
