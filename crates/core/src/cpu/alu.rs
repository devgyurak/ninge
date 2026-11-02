// SPDX-License-Identifier: Apache-2.0
//! The ALU of the data-processing instructions (`docs/hw/arm-data-processing.md`).

/// Opcode field values, bits 24–21.
pub(crate) const TST: u32 = 8;
pub(crate) const CMN: u32 = 11;

/// A result and the C and V flags it leaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Alu {
    pub(crate) result: u32,
    pub(crate) carry: bool,
    pub(crate) overflow: bool,
}

/// True for TST, TEQ, CMP and CMN, which set flags but write no register.
pub(crate) fn is_compare(opcode: u32) -> bool {
    (TST..=CMN).contains(&opcode)
}

/// `a + b + carry_in`, with the unsigned carry-out and the signed overflow.
fn add(a: u32, b: u32, carry_in: bool) -> Alu {
    let (sum, c1) = a.overflowing_add(b);
    let (result, c2) = sum.overflowing_add(u32::from(carry_in));
    Alu {
        result,
        carry: c1 || c2,
        // Overflow when both operands have the same sign and the result's differs.
        overflow: ((a ^ result) & (b ^ result)) >> 31 == 1,
    }
}

/// `a - b - 1 + carry_in`, computed as `a + !b + carry_in`: the carry-out is "not borrow".
fn sub(a: u32, b: u32, carry_in: bool) -> Alu {
    add(a, !b, carry_in)
}

/// Runs `opcode` on `a` (Rn) and `b` (operand 2). `shifter_carry` is what a logical operation
/// leaves in C; `carry` and `overflow` are the current flags.
pub(crate) fn execute(
    opcode: u32,
    a: u32,
    b: u32,
    shifter_carry: bool,
    carry: bool,
    overflow: bool,
) -> Alu {
    let logical = |result| Alu {
        result,
        carry: shifter_carry,
        overflow,
    };
    match opcode & 0xF {
        0 | 8 => logical(a & b),
        1 | 9 => logical(a ^ b),
        2 | 10 => sub(a, b, true),
        3 => sub(b, a, true),
        4 | 11 => add(a, b, false),
        5 => add(a, b, carry),
        6 => sub(a, b, carry),
        7 => sub(b, a, carry),
        12 => logical(a | b),
        13 => logical(b),
        14 => logical(a & !b),
        _ => logical(!b),
    }
}
