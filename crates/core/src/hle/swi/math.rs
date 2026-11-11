// SPDX-License-Identifier: Apache-2.0
//! Div and Sqrt (`docs/hw/hle-swi.md`, "The copy, math and decompression SWIs"): pure functions
//! on integers, with no float and no memory access.

/// Div: the quotient truncated toward zero, the remainder with the numerator's sign, and the
/// absolute value of the quotient.
// TODO(hw-unverified): a zero denominator gives +1 (-1 for a negative numerator) with the
// numerator as the remainder, and `-80000000h / -1` wraps (`hle-swi.md`, "Unverified").
pub(super) fn divide(numerator: i32, denominator: i32) -> (i32, i32, u32) {
    match denominator {
        0 => {
            let quotient = match numerator < 0 {
                true => -1,
                false => 1,
            };
            (quotient, numerator, 1)
        }
        _ => {
            let quotient = numerator.wrapping_div(denominator);
            (
                quotient,
                numerator.wrapping_rem(denominator),
                quotient.unsigned_abs(),
            )
        }
    }
}

/// Sqrt: the largest `r` with `r * r <= value`, by the bit-pair method (two bits of the value per
/// step, integers only).
pub(super) fn sqrt(value: u32) -> u32 {
    let mut remainder = u64::from(value);
    let mut result = 0u64;
    // The highest power of four not above the value; 0 for a value of 0.
    let mut bit = 1u64 << 30;
    while bit > remainder {
        bit >>= 2;
    }
    while bit != 0 {
        match remainder >= result + bit {
            true => {
                remainder -= result + bit;
                result = (result >> 1) + bit;
            }
            false => result >>= 1,
        }
        bit >>= 2;
    }
    // At most FFFFh: the square root of a 32-bit value.
    result as u32
}
