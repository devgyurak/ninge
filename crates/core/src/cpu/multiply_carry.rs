// SPDX-License-Identifier: Apache-2.0
//! The ARM7TDMI multiplier's carry-save datapath and the C flag an S multiply leaves
//! (`docs/hw/arm-multiply.md`, "ARM7TDMI multiply carry"; #81, decided in #80).
//!
//! The multiplier retires eight multiplier bits per iteration in four radix-4 Booth rows, keeps
//! the running sum as a save vector and a carry vector, and stops after one to four iterations.
//! The ARM7TDMI sets C from the barrel shifter's carry-out while the final cycles bring the
//! carry vector back through it, which is a bit of that vector.
// TODO(hw-unverified): the link from the shifter to C is oracle evidence only (the pinned
// suite), m = 1 and 2 are untested there, the long-form m < 4 slice is inferred from the shift
// amounts of US 5,557,563 Table 1, the patents' embodiment being the ARM7TDMI's multiplier and
// the off-by-one reading of the P1/P2 table are inferences, and SIGNED = 1 for MUL, MLA and Thumb MUL is inferred from
// the TRM `m` rule (`docs/hw/arm-multiply.md`, "Unverified").

/// Bits of the multiplier consumed per iteration (four rows of two bits).
const BITS_PER_ITERATION: u32 = 8;
/// Booth rows per iteration.
const ROWS_PER_ITERATION: u32 = 4;
/// Width of the active region of the save and carry vectors, in bits.
const ACTIVE_BITS: u32 = 33;
const ACTIVE_MASK: u64 = (1 << ACTIVE_BITS) - 1;
/// The multiplicand and multiplier are widened to 33 bits, and the multiple to 34.
const OPERAND_MASK: u64 = (1 << ACTIVE_BITS) - 1;
const MULTIPLE_MASK: u64 = (1 << 34) - 1;
/// The bit of the carry vector C is taken from: the word's top bit, or the long result's.
const CARRY_BIT_WORD: u32 = 31;
const CARRY_BIT_LONG: u32 = 63;

/// The save and carry vectors after the iterations the multiplier needs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CarrySave {
    /// The save vector (the running sum), modulo 2^64.
    pub(crate) save: u64,
    /// The carry vector, modulo 2^64.
    pub(crate) carry: u64,
    /// The number of iterations, 1 to 4 (the `m` of the TRM).
    pub(crate) iterations: u32,
}

/// The TRM's `m`: 1 if bits 31–8 of the multiplier are all zero (or, when `signed`, all one),
/// 2 for bits 31–16, 3 for bits 31–24, otherwise 4.
pub(crate) fn iterations(multiplier: u32, signed: bool) -> u32 {
    (1..4)
        .find(|&m| {
            let top = multiplier >> (BITS_PER_ITERATION * m);
            top == 0 || (signed && top == u32::MAX >> (BITS_PER_ITERATION * m))
        })
        .unwrap_or(4)
}

/// The digit of a radix-4 Booth recoding for the three multiplier bits `bits` (`R[2k+2:2k]`).
fn booth_digit(bits: u64) -> i32 {
    match bits & 7 {
        0 | 7 => 0,
        1 | 2 => 1,
        3 => 2,
        4 => -2,
        _ => -1,
    }
}

/// One multiple of the multiplicand as the row adds it: `X[33:0]` and the row's own `XC` bit.
/// `d` is the 33-bit multiplicand `D[32:0]`.
fn multiple(digit: i32, d: u64) -> (u64, u64) {
    let inverse = !d & OPERAND_MASK;
    let top = |v: u64| (v >> 32 & 1) << 33;
    match digit {
        -2 => (top(inverse) | (inverse & 0xFFFF_FFFF) << 1 | 1, 1),
        -1 => (top(inverse) | inverse, 1),
        0 => (0, 0),
        1 => (top(d) | d, 0),
        _ => (top(d) | (d & 0xFFFF_FFFF) << 1, 0),
    }
}

/// Runs the multiplier for `multiplier` × `multiplicand` + `accumulate`, `signed` or unsigned.
/// `accumulate` is 0 without an accumulate, the register Rn for MLA (zero-extended), and
/// RdHi:RdLo for the long forms.
pub(crate) fn carry_save(
    multiplier: u32,
    multiplicand: u32,
    accumulate: u64,
    signed: bool,
) -> CarrySave {
    let m = iterations(multiplier, signed);
    let sign = |v: u32| u64::from(signed && v >> 31 == 1);
    // R[32] and D[32] are copies of the sign when signed, else 0 (US 5,583,804).
    let r = u64::from(multiplier) | sign(multiplier) << 32;
    let d = u64::from(multiplicand) | sign(multiplicand) << 32;
    let d34 = d | (d >> 32 & 1) << 33;
    // The accumulate value is sign-extended past bit 63 when signed.
    let extension = u128::from(signed && accumulate >> 63 == 1) * (!0u128 << 64);
    let acc = u128::from(accumulate) | extension;
    let initial_save = acc as u64 & MULTIPLE_MASK; // truncation: the low 34 bits
    let initial_carry = match r & 1 {
        1 => !d34 & MULTIPLE_MASK,
        _ => 0,
    };
    // Bit 0 of the initialisers stays in the low region; bits 33..1 are the active region.
    let mut low_save = initial_save & 1;
    let mut low_carry = initial_carry & 1;
    let mut save = initial_save >> 1;
    let mut carry = initial_carry >> 1;
    // The accumulate bits above the initialiser enter two per row.
    let high = (acc >> 34) as u64 & 0xFFFF_FFFF; // truncation: 32 bits, two per row
    for k in 0..ROWS_PER_ITERATION * m {
        let (x, xc) = multiple(booth_digit(r >> (2 * k)), d);
        let ah = high >> (2 * k) & 3;
        let sum = (save ^ carry ^ x) & ACTIVE_MASK;
        let carries = ((save & carry) | (save & x) | (carry & x)) & ACTIVE_MASK;
        // The top two bits sum the accumulate bit with the complemented top carry and top
        // multiple bit; the top carry becomes the complement of the second accumulate bit.
        let top = (ah & 1) + (!carry >> 32 & 1) + (!x >> 33 & 1);
        low_save |= (sum & 3) << (1 + 2 * k);
        low_carry |= (xc | (carries & 1) << 1) << (1 + 2 * k);
        save = (sum >> 2 & 0x7FFF_FFFF) | (top & 1) << 31 | (top >> 1) << 32;
        carry = (carries >> 1 & 0xFFFF_FFFF) | (!ah >> 1 & 1) << 32;
    }
    let low_bits = BITS_PER_ITERATION * m + 1;
    // Past the low region: the active save, and the accumulate bits that never entered.
    let entered = low_bits + ACTIVE_BITS;
    let rest = (acc >> entered) << entered;
    let total_save = u128::from(low_save) + (u128::from(save) << low_bits) + rest;
    // The active carry is a 33-bit signed value (its bit 32 repeats upward).
    // truncation: the 33-bit value sign-extended through i64
    let carry_signed = ((carry << 31) as i64) >> 31;
    let total_carry = low_carry.wrapping_add((carry_signed as u64) << low_bits);
    CarrySave {
        save: total_save as u64, // truncation: modulo 2^64
        carry: total_carry,
        iterations: m,
    }
}

/// The C flag after an S multiply on the ARM7TDMI: bit 31 of the carry vector, or bit 63 for
/// the long forms (`long`) that needed all four iterations.
pub(crate) fn multiply_carry(
    multiplier: u32,
    multiplicand: u32,
    accumulate: u64,
    signed: bool,
    long: bool,
) -> bool {
    let state = carry_save(multiplier, multiplicand, accumulate, signed);
    let bit = match long && state.iterations == 4 {
        true => CARRY_BIT_LONG,
        false => CARRY_BIT_WORD,
    };
    state.carry >> bit & 1 == 1
}
