// SPDX-License-Identifier: Apache-2.0
//! The barrel shifter (`docs/hw/arm-barrel-shifter.md`): a value, a shift type and an amount in,
//! a result and the carry-out out.

/// Shift type, bits 6–5 of a shifted-register operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShiftKind {
    Lsl,
    Lsr,
    Asr,
    Ror,
}

impl ShiftKind {
    /// Decodes the two shift-type bits.
    pub(crate) fn from_bits(bits: u32) -> Self {
        match bits & 3 {
            0 => Self::Lsl,
            1 => Self::Lsr,
            2 => Self::Asr,
            _ => Self::Ror,
        }
    }
}

/// Shifts by a 5-bit immediate, where an amount of 0 has its own meaning for LSR, ASR and ROR
/// (`docs/hw/arm-barrel-shifter.md`, "Behavior"). `carry` is the current C flag.
pub(crate) fn shift_immediate(
    kind: ShiftKind,
    value: u32,
    amount: u32,
    carry: bool,
) -> (u32, bool) {
    match (kind, amount) {
        (ShiftKind::Lsl, 0) => (value, carry),
        (ShiftKind::Lsr | ShiftKind::Asr, 0) => shift_register(kind, value, 32, carry),
        // ROR #0 is RRX: rotate right by one through the carry flag.
        (ShiftKind::Ror, 0) => ((u32::from(carry) << 31) | (value >> 1), value & 1 == 1),
        _ => shift_register(kind, value, amount, carry),
    }
}

/// Shifts by an amount taken from a register (only the low byte counts; the caller masks it).
/// An amount of 0 changes nothing for every type.
pub(crate) fn shift_register(kind: ShiftKind, value: u32, amount: u32, carry: bool) -> (u32, bool) {
    match (amount, kind) {
        (0, _) => (value, carry),
        (1..=31, ShiftKind::Lsl) => (value << amount, (value >> (32 - amount)) & 1 == 1),
        (32, ShiftKind::Lsl) => (0, value & 1 == 1),
        (_, ShiftKind::Lsl) => (0, false),
        (1..=31, ShiftKind::Lsr) => (value >> amount, (value >> (amount - 1)) & 1 == 1),
        (32, ShiftKind::Lsr) => (0, value >> 31 == 1),
        (_, ShiftKind::Lsr) => (0, false),
        // The cast reinterprets the bits so `>>` is arithmetic.
        (1..=31, ShiftKind::Asr) => (
            ((value as i32) >> amount) as u32,
            (value >> (amount - 1)) & 1 == 1,
        ),
        (_, ShiftKind::Asr) => (((value as i32) >> 31) as u32, value >> 31 == 1),
        (_, ShiftKind::Ror) => {
            let result = value.rotate_right(amount & 31);
            (result, result >> 31 == 1)
        }
    }
}

/// The rotated-immediate operand: an 8-bit value rotated right by twice the 4-bit field. A
/// rotation of zero keeps the carry flag; otherwise the carry-out is bit 31 of the result.
pub(crate) fn rotated_immediate(imm8: u32, rotate_field: u32, carry: bool) -> (u32, bool) {
    let rotation = (rotate_field & 0xF) * 2;
    let value = (imm8 & 0xFF).rotate_right(rotation);
    match rotation {
        0 => (value, carry),
        _ => (value, value >> 31 == 1),
    }
}
