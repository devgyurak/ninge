// SPDX-License-Identifier: Apache-2.0
//! The ARM7TDMI multiplier's carry-save model, on the pure functions in `multiply_carry.rs`.
//! Expected values come from `docs/hw/arm-multiply.md`, "ARM7TDMI multiply carry": the four
//! worked examples of US 5,557,563 for the datapath, and hand-worked operands for the C rule.

use crate::cpu::multiply_carry::{carry_save, iterations, multiply_carry};
use proptest::prelude::*;

/// The 33-bit width of the active region of the save and carry vectors.
const ACTIVE_MASK: u64 = (1 << 33) - 1;

/// P3's four examples: multiplier, iterations, save low bits (8m + 1 of them), and the low 32
/// bits of the carry vector. The multiplicand is 1 (the patent's example titles are a typo).
const P3_EXAMPLES: [(u32, u32, u64, u64); 4] = [
    (0x1, 1, 0x1FC, 0x4),
    (0x100, 2, 0x1_FC80, 0x480),
    (0x1_0000, 3, 0x1FC_8000, 0x4_8000),
    (0x100_0000, 4, 0x1_FC80_0000, 0x480_0000),
];

/// The examples whose iteration count, save low bits, active save or low carry differ from P3.
fn p3_example_failures() -> Vec<String> {
    P3_EXAMPLES
        .iter()
        .flat_map(|&(multiplier, iterations, save_low, carry_low)| {
            [false, true].map(|signed| {
                let state = carry_save(multiplier, 1, 0, signed);
                let low_bits = 8 * iterations + 1;
                let got = (
                    state.iterations,
                    state.save & ((1 << low_bits) - 1),
                    (state.save >> low_bits) & ACTIVE_MASK,
                    state.carry & 0xFFFF_FFFF,
                );
                // The 64-bit vector holds only 64 - low_bits bits of the 33-bit active save.
                let active = 0x0_FFFF_FFFF & (u64::MAX >> low_bits);
                let want = (iterations, save_low, active, carry_low);
                (got != want).then(|| format!("{multiplier:#x} signed={signed}: {got:x?}"))
            })
        })
        .flatten()
        .collect()
}

#[test]
fn given_p3_worked_examples_when_the_core_runs_then_save_and_carry_match_the_printed_buses() {
    assert_eq!(p3_example_failures(), Vec::<String>::new());
}

/// (multiplier, signed, iterations) from the TRM `m` rule.
const ITERATION_CASES: [(u32, bool, u32); 10] = [
    (0, false, 1),
    (0xFF, false, 1),
    (0x100, false, 2),
    (0xFFFF, false, 2),
    (0x1_0000, false, 3),
    (0x100_0000, false, 4),
    (0xFFFF_FFFF, true, 1),
    (0xFFFF_FF00, true, 1),
    (0xFFFF_0000, true, 2),
    (0xFFFF_FFFF, false, 4),
];

fn iteration_failures() -> Vec<String> {
    ITERATION_CASES
        .iter()
        .filter(|&&(multiplier, signed, want)| iterations(multiplier, signed) != want)
        .map(|&(multiplier, signed, want)| format!("{multiplier:#x} signed={signed}: want {want}"))
        .collect()
}

#[test]
fn given_multipliers_when_counting_iterations_then_leading_zeros_and_for_signed_ones_end_early() {
    assert_eq!(iteration_failures(), Vec::<String>::new());
}

/// (multiplier, multiplicand, accumulate, signed, long, C), worked by hand from the model. The
/// long m = 4 row takes bit 63 of the carry vector (it is 1, while bit 31 is 0); the long m = 1
/// row takes bit 31 (0, while bit 63 is 1).
const CARRY_CASES: [(u32, u32, u64, bool, bool, bool); 5] = [
    (
        0xDEAD_BEEF,
        0xCAFE_F00D,
        0x1234_5678_90AB_CDEF,
        false,
        true,
        true,
    ),
    (
        0xDEAD_BEEF,
        0xCAFE_F00D,
        0x1234_5678_90AB_CDEF,
        true,
        false,
        false,
    ),
    (0xFF, 0x7FFF_FFFF, 0x8000_0000, true, true, false),
    (0x81, 0xFFFF_FFFE, 0, true, false, true),
    (0xFF_FF81, 0x1234, 0, true, false, false),
];

fn carry_failures() -> Vec<String> {
    CARRY_CASES
        .iter()
        .filter(|&&(multiplier, multiplicand, acc, signed, long, want)| {
            multiply_carry(multiplier, multiplicand, acc, signed, long) != want
        })
        .map(|&(multiplier, multiplicand, ..)| format!("{multiplier:#x} x {multiplicand:#x}"))
        .collect()
}

#[test]
fn given_hand_worked_operands_when_taking_c_then_long_m4_uses_bit_63_and_the_rest_bit_31() {
    assert_eq!(carry_failures(), Vec::<String>::new());
}

/// The plain result the carry-save pair must add up to, wrapped to the instruction's width.
fn plain(multiplier: u32, multiplicand: u32, acc: u64, signed: bool, long: bool) -> u64 {
    let widen = |v: u32| match signed {
        true => i64::from(v as i32) as u64,
        false => u64::from(v),
    };
    let product = widen(multiplier)
        .wrapping_mul(widen(multiplicand))
        .wrapping_add(acc);
    match long {
        true => product,
        false => product & 0xFFFF_FFFF,
    }
}

/// An operand that needs 1 to 4 iterations: a signed value shifted down by 0, 8, 16 or 24.
fn operand() -> impl Strategy<Value = u32> {
    (any::<i32>(), 0u32..4).prop_map(|(value, bytes)| (value >> (8 * bytes)) as u32)
}

/// What `save + carry + R[0]` adds up to, and the plain result, both wrapped to the instruction's
/// width. The accumulate of a 32-bit form is a register, so only its low word is used.
fn sum_and_plain(
    multiplier: u32,
    multiplicand: u32,
    acc: u64,
    signed: bool,
    long: bool,
) -> (u64, u64) {
    let (acc, mask) = match long {
        true => (acc, u64::MAX),
        false => (acc & 0xFFFF_FFFF, 0xFFFF_FFFF),
    };
    let state = carry_save(multiplier, multiplicand, acc, signed);
    let sum = state
        .save
        .wrapping_add(state.carry)
        .wrapping_add(u64::from(multiplier & 1));
    (
        sum & mask,
        plain(multiplier, multiplicand, acc, signed, long),
    )
}

proptest! {
    #[test]
    fn given_any_operands_when_the_core_runs_then_save_plus_carry_plus_r0_is_the_plain_result(
        multiplier in operand(),
        multiplicand in any::<u32>(),
        acc in any::<u64>(),
        signed in any::<bool>(),
        long in any::<bool>(),
    ) {
        let (sum, want) = sum_and_plain(multiplier, multiplicand, acc, signed, long);
        prop_assert_eq!(sum, want);
    }
}
