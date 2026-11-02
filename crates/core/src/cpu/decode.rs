// SPDX-License-Identifier: Apache-2.0
//! The ARM instruction decoder: a table built at compile time, indexed by instruction bits
//! 27–20 and 7–4 (`docs/hw/arm-data-processing.md`, "Encoding").
//!
//! Decided in #78.

/// Which handler an ARM word runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Not (yet) executed: the step only advances the PC.
    // TODO(phase1): the remaining groups arrive in #9–#13.
    Unhandled,
    /// Data processing, rotated-immediate operand.
    DataImmediate,
    /// Data processing, register operand shifted by an immediate.
    DataImmediateShift,
    /// Data processing, register operand shifted by a register.
    DataRegisterShift,
    /// MRS.
    Mrs,
    /// MSR with an immediate.
    MsrImmediate,
    /// MSR with a register.
    MsrRegister,
}

const TABLE_SIZE: usize = 1 << 12;

const TABLE: [Kind; TABLE_SIZE] = build_table();

/// The table index of an instruction word: bits 27–20, then bits 7–4.
const fn index(word: u32) -> usize {
    (((word >> 20) & 0xFF) << 4 | ((word >> 4) & 0xF)) as usize
}

/// Classifies the encoding space of one table index.
const fn classify(index: usize) -> Kind {
    let high = index >> 4; // bits 27–20
    let low = index & 0xF; // bits 7–4
    let opcode = (high >> 1) & 0xF;
    let set_flags = high & 1 == 1;
    // TST/TEQ/CMP/CMN without S is the PSR-transfer space, not a data-processing instruction.
    let psr_space = !set_flags && opcode >= 8 && opcode <= 11;
    let reg_form = low & 1 == 0; // bit 4 = 0: shift by immediate
    let multiply_space = low & 0b1001 == 0b1001; // bit 7 and bit 4 set
    match high >> 5 {
        // bits 27–25 = 001
        1 => match (psr_space, opcode & 1) {
            (false, _) => Kind::DataImmediate,
            (true, 1) => Kind::MsrImmediate,
            _ => Kind::Unhandled,
        },
        // bits 27–25 = 000
        0 => match (psr_space, multiply_space, reg_form) {
            (false, true, _) => Kind::Unhandled,
            (false, false, true) => Kind::DataImmediateShift,
            (false, false, false) => Kind::DataRegisterShift,
            (true, _, _) => match (low, opcode & 1) {
                (0, 0) => Kind::Mrs,
                (0, _) => Kind::MsrRegister,
                _ => Kind::Unhandled,
            },
        },
        _ => Kind::Unhandled,
    }
}

const fn build_table() -> [Kind; TABLE_SIZE] {
    let mut table = [Kind::Unhandled; TABLE_SIZE];
    let mut i = 0;
    while i < TABLE_SIZE {
        table[i] = classify(i);
        i += 1;
    }
    table
}

/// The handler kind of an ARM instruction word.
pub(crate) fn decode(word: u32) -> Kind {
    TABLE[index(word)]
}
