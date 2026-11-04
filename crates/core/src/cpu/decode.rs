// SPDX-License-Identifier: Apache-2.0
//! The ARM instruction decoder: a table built at compile time, indexed by instruction bits
//! 27–20 and 7–4 (`docs/hw/arm-data-processing.md`, "Encoding").
//!
//! Decided in #78.

/// Which handler an ARM word runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Not (yet) executed: the step only advances the PC.
    // TODO(phase1): the remaining groups arrive in #11–#13; LDRD and STRD, which share the
    // halfword-transfer space, and PLD arrive in #14.
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
    /// MUL or MLA.
    Multiply,
    /// UMULL, UMLAL, SMULL or SMLAL.
    MultiplyLong,
    /// LDR, STR, LDRB, STRB and the T forms with a 12-bit immediate offset.
    SingleTransferImmediate,
    /// The same with a register offset shifted by an immediate.
    SingleTransferRegister,
    /// LDRH, STRH, LDRSB and LDRSH.
    HalfwordTransfer,
    /// SWP and SWPB.
    Swap,
    /// LDM and STM.
    BlockTransfer,
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
        // bits 27–25 = 010
        2 => Kind::SingleTransferImmediate,
        // bits 27–25 = 011; bit 4 = 1 is the undefined space
        3 if reg_form => Kind::SingleTransferRegister,
        // bits 27–25 = 100
        4 => Kind::BlockTransfer,
        // bits 27–25 = 001
        1 => match (psr_space, opcode & 1) {
            (false, _) => Kind::DataImmediate,
            (true, 1) => Kind::MsrImmediate,
            _ => Kind::Unhandled,
        },
        // bits 27–25 = 000
        0 => match (psr_space, multiply_space, reg_form) {
            // SWP and the halfword transfers sit inside the PSR space, so this comes first.
            (_, true, _) => multiply_space_kind(high, low),
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

/// Classifies a word with bits 27–25 = 000 and bits 7 and 4 set: a multiply is exactly
/// bits 27–24 = 0000 with bits 7–4 = 1001 (`docs/hw/arm-multiply.md`, "Encoding"); SWP is
/// bits 27–23 = 00010 and 21–20 = 00 with bits 7–4 = 1001; a halfword transfer has bits 6–5
/// not 00 (`docs/hw/arm-load-store.md`, "Encodings").
const fn multiply_space_kind(high: usize, low: usize) -> Kind {
    let sh = (low >> 1) & 0b11; // bits 6–5
    let load = high & 1 == 1; // bit 20
    match (low, sh) {
        (0b1001, _) => match (high >> 4 == 0, high & 0xBB == 0x10, (high >> 2) & 0b11) {
            // Bit 23 is L; bits 23–22 = 01 is UMAAL (ARMv6), which stays unhandled.
            (true, _, 0) => Kind::Multiply,
            (true, _, 2 | 3) => Kind::MultiplyLong,
            (_, true, _) => Kind::Swap,
            _ => Kind::Unhandled,
        },
        // A store with SH = 10 or 11 is LDRD or STRD (#14).
        (_, 1) => Kind::HalfwordTransfer,
        (_, 2 | 3) if load => Kind::HalfwordTransfer,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Words and the kind each must decode to (`docs/hw/arm-multiply.md`, "Encoding";
    /// `docs/hw/arm-load-store.md`, "Encodings").
    const CASES: [(u32, Kind); 27] = [
        (0xE000_0090, Kind::Multiply),                // MUL
        (0xE010_0090, Kind::Multiply),                // MULS
        (0xE020_0090, Kind::Multiply),                // MLA
        (0xE030_0090, Kind::Multiply),                // MLAS
        (0xE080_0090, Kind::MultiplyLong),            // UMULL
        (0xE0A0_0090, Kind::MultiplyLong),            // UMLAL
        (0xE0C0_0090, Kind::MultiplyLong),            // SMULL
        (0xE0F0_0090, Kind::MultiplyLong),            // SMLALS
        (0xE040_0090, Kind::Unhandled),               // bits 23-22 = 01, A = 0
        (0xE060_0090, Kind::Unhandled),               // bits 23-22 = 01, A = 1
        (0xE100_0090, Kind::Swap),                    // SWP
        (0xE140_0090, Kind::Swap),                    // SWPB
        (0xE000_00B0, Kind::HalfwordTransfer),        // STRH
        (0xE010_00B0, Kind::HalfwordTransfer),        // LDRH
        (0xE010_00D0, Kind::HalfwordTransfer),        // LDRSB
        (0xE010_00F0, Kind::HalfwordTransfer),        // LDRSH
        (0xE180_0090, Kind::Unhandled),               // bits 27-24 = 0001, not SWP
        (0xE200_0090, Kind::DataImmediate),           // bits 27-24 = 0010 is data processing
        (0xE1C0_00D0, Kind::Unhandled),               // LDRD: L = 0 with SH = 10 (#14)
        (0xE1C0_00F0, Kind::Unhandled),               // STRD: L = 0 with SH = 11 (#14)
        (0xE890_0003, Kind::BlockTransfer),           // LDMIA
        (0xE880_0003, Kind::BlockTransfer),           // STMIA
        (0xE590_1004, Kind::SingleTransferImmediate), // LDR with an immediate offset
        (0xE5C0_1004, Kind::SingleTransferImmediate), // STRB with an immediate offset
        (0xE790_1002, Kind::SingleTransferRegister),  // LDR with a register offset
        (0xE780_1102, Kind::SingleTransferRegister),  // STR, register offset shifted by 2
        (0xE790_1012, Kind::Unhandled), // bits 27-25 = 011 with bit 4 = 1 is undefined
    ];

    fn classification_failures() -> Vec<(u32, Kind, Kind)> {
        CASES
            .iter()
            .map(|&(word, want)| (word, want, decode(word)))
            .filter(|(_, want, got)| want != got)
            .collect()
    }

    #[test]
    fn given_multiply_transfer_and_swap_words_when_decoded_then_each_word_gets_its_kind() {
        assert_eq!(classification_failures(), vec![]);
    }
}
