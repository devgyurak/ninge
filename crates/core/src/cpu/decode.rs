// SPDX-License-Identifier: Apache-2.0
//! The ARM instruction decoder: a table built at compile time, indexed by instruction bits
//! 27–20 and 7–4 (`docs/hw/arm-data-processing.md`, "Encoding").
//!
//! Decided in #78.

/// Which handler an ARM word runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Not (yet) executed: the step only advances the PC.
    // TODO(hw-unverified): only the 011/bit 4 space traps as undefined; GBATEK says other unused
    // opcodes need not (`docs/hw/arm-exceptions-modes.md`, "Unverified").
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
    /// LDRD and STRD (ARM9; `docs/hw/armv5te.md`, "LDRD and STRD").
    DoubleTransfer,
    /// SWP and SWPB.
    Swap,
    /// LDM and STM.
    BlockTransfer,
    /// B and BL (`docs/hw/arm-branch-pipeline.md`).
    Branch,
    /// BX.
    BranchExchange,
    /// SMLAxy, SMLAWy, SMULWy, SMLALxy and SMULxy (ARM9; `docs/hw/armv5te.md`, "Signed halfword
    /// multiplies").
    SignedHalfwordMultiply,
    /// QADD, QSUB, QDADD and QDSUB (ARM9; `docs/hw/armv5te.md`, "Saturating arithmetic").
    SaturatingArithmetic,
    /// CLZ (ARM9; `docs/hw/armv5te.md`, "CLZ").
    CountLeadingZeros,
    /// BLX with a register (ARM9; `docs/hw/armv5te.md`, "BLX").
    BranchLinkExchange,
    /// BLX with an immediate, in the unconditional space (ARM9; `docs/hw/armv5te.md`, "BLX").
    BranchLinkExchangeImmediate,
    /// BKPT (ARM9; `docs/hw/armv5te.md`, "BKPT"): a prefetch abort.
    // TODO(hw-unverified): a condition other than AL makes it conditional here
    // (`docs/hw/armv5te.md`, "Unverified").
    Breakpoint,
    /// PLD, a cache hint that does nothing (ARM9; `docs/hw/armv5te.md`, "PLD").
    Preload,
    /// SWI (`docs/hw/arm-exceptions-modes.md`).
    Swi,
    /// The undefined-instruction space: bits 27–25 = 011 with bit 4 = 1.
    Undefined,
    /// CDP, MCR, MRC, LDC, STC and MCRR/MRRC: what they do depends on the core and on the
    /// coprocessor number in bits 11–8, which the table does not index.
    Coprocessor,
}

const TABLE_SIZE: usize = 1 << 12;

/// The ARM7's table (ARMv4T) and the ARM9's (ARMv5TE), and the ARM9's table for the words whose
/// condition field is 1111 (`docs/hw/armv5te.md`, "Unconditional space").
const TABLE_V4: [Kind; TABLE_SIZE] = build_table(false);
const TABLE_V5: [Kind; TABLE_SIZE] = build_table(true);
const TABLE_UNCONDITIONAL: [Kind; TABLE_SIZE] = build_unconditional_table();

/// The table index of an instruction word: bits 27–20, then bits 7–4.
const fn index(word: u32) -> usize {
    (((word >> 20) & 0xFF) << 4 | ((word >> 4) & 0xF)) as usize
}

/// Classifies the encoding space of one table index.
const fn classify(index: usize, arm9: bool) -> Kind {
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
        // (`docs/hw/arm-exceptions-modes.md`, "Undefined instruction")
        3 if reg_form => Kind::SingleTransferRegister,
        3 => Kind::Undefined,
        // bits 27–25 = 100
        4 => Kind::BlockTransfer,
        // bits 27–25 = 101
        5 => Kind::Branch,
        // bits 27–25 = 001
        1 => match (psr_space, opcode & 1) {
            (false, _) => Kind::DataImmediate,
            (true, 1) => Kind::MsrImmediate,
            _ => Kind::Unhandled,
        },
        // bits 27–25 = 000
        0 => match (psr_space, multiply_space, reg_form) {
            // SWP and the halfword transfers sit inside the PSR space, so this comes first.
            (_, true, _) => multiply_space_kind(high, low, arm9),
            (false, false, true) => Kind::DataImmediateShift,
            (false, false, false) => Kind::DataRegisterShift,
            // BX is bits 27–20 = 0x12 with bits 7–4 = 0001, a TEQ-shaped word in the PSR space
            // (`docs/hw/arm-branch-pipeline.md`, "Encodings").
            // TODO(hw-unverified): bits 19–8 are not checked, so a word with other values there
            // still runs as BX (`docs/hw/arm-branch-pipeline.md`, "Unverified").
            (true, _, _) => match (low, opcode & 1) {
                (1, 1) if high == 0x12 => Kind::BranchExchange,
                // TODO(hw-unverified): the should-be-one fields (bits 19–8) of BLX register, CLZ
                // and BKPT's immediate field are not checked (`docs/hw/armv5te.md`, "Unverified").
                (3, 1) if arm9 && high == 0x12 => Kind::BranchLinkExchange,
                (7, 1) if arm9 && high == 0x12 => Kind::Breakpoint,
                (1, 1) if arm9 && high == 0x16 => Kind::CountLeadingZeros,
                // bit 7 set and bit 4 clear in the PSR space
                (_, _) if arm9 && low & 0b1001 == 0b1000 => Kind::SignedHalfwordMultiply,
                // bits 7–4 = 0101 in the PSR space
                (5, _) if arm9 => Kind::SaturatingArithmetic,
                (0, 0) => Kind::Mrs,
                (0, _) => Kind::MsrRegister,
                _ => Kind::Unhandled,
            },
        },
        // bits 27–25 = 110: LDC, STC, and MCRR/MRRC
        6 => Kind::Coprocessor,
        // bits 27–25 = 111: SWI with bit 24 set, CDP, MCR and MRC without
        _ => match high >> 4 & 1 {
            1 => Kind::Swi,
            _ => Kind::Coprocessor,
        },
    }
}

/// Classifies a word with bits 27–25 = 000 and bits 7 and 4 set: a multiply is exactly
/// bits 27–24 = 0000 with bits 7–4 = 1001 (`docs/hw/arm-multiply.md`, "Encoding"); SWP is
/// bits 27–23 = 00010 and 21–20 = 00 with bits 7–4 = 1001; a halfword transfer has bits 6–5
/// not 00 (`docs/hw/arm-load-store.md`, "Encodings").
const fn multiply_space_kind(high: usize, low: usize, arm9: bool) -> Kind {
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
        (_, 1) => Kind::HalfwordTransfer,
        (_, 2 | 3) if load => Kind::HalfwordTransfer,
        // A store with SH = 10 or 11 is LDRD or STRD (ARM9 only).
        (_, 2 | 3) if arm9 => Kind::DoubleTransfer,
        _ => Kind::Unhandled,
    }
}

const fn build_table(arm9: bool) -> [Kind; TABLE_SIZE] {
    let mut table = [Kind::Unhandled; TABLE_SIZE];
    let mut i = 0;
    while i < TABLE_SIZE {
        table[i] = classify(i, arm9);
        i += 1;
    }
    table
}

/// Classifies the encoding space of one table index for a word with condition 1111, which the
/// ARM9 does not treat as "never" (`docs/hw/armv5te.md`, "Unconditional space").
const fn unconditional_kind(index: usize) -> Kind {
    let high = index >> 4; // bits 27–20
    let low = index & 0xF; // bits 7–4
    // PLD is a byte load (P = 1, B = 1, W = 0, L = 1; U is free) with the PC-like Rd field 1111.
    // TODO(hw-unverified): the Rd field (bits 15–12) is not checked
    // (`docs/hw/armv5te.md`, "Unverified").
    let pld_bits = high & 0x17 == 0x15;
    match high >> 5 {
        // bits 27–25 = 101: BLX with an immediate
        5 => Kind::BranchLinkExchangeImmediate,
        // bits 27–25 = 010: PLD with an immediate offset
        2 if pld_bits => Kind::Preload,
        // bits 27–25 = 011: PLD with a register offset, which has bit 4 clear
        3 if pld_bits && low & 1 == 0 => Kind::Preload,
        // TODO(hw-unverified): the other words of the 1111 space are stepped past; the real core
        // may take Undefined (`docs/hw/armv5te.md`, "Unverified").
        _ => Kind::Unhandled,
    }
}

const fn build_unconditional_table() -> [Kind; TABLE_SIZE] {
    let mut table = [Kind::Unhandled; TABLE_SIZE];
    let mut i = 0;
    while i < TABLE_SIZE {
        table[i] = unconditional_kind(i);
        i += 1;
    }
    table
}

/// The handler kind of an ARM instruction word on the ARM9 (`ARM9`) or the ARM7.
pub(crate) fn decode<const ARM9: bool>(word: u32) -> Kind {
    match (ARM9, word >> 28 == 0xF) {
        (true, true) => TABLE_UNCONDITIONAL[index(word)],
        (true, false) => TABLE_V5[index(word)],
        (false, _) => TABLE_V4[index(word)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Words and the kind each must decode to (`docs/hw/arm-multiply.md`, "Encoding";
    /// `docs/hw/arm-load-store.md`, "Encodings").
    const CASES: [(u32, Kind); 49] = [
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
        (0xE1C0_00D0, Kind::Unhandled),               // LDRD: L = 0 with SH = 10 (ARM9 only)
        (0xE1C0_00F0, Kind::Unhandled),               // STRD: L = 0 with SH = 11 (ARM9 only)
        (0xE890_0003, Kind::BlockTransfer),           // LDMIA
        (0xE880_0003, Kind::BlockTransfer),           // STMIA
        (0xE590_1004, Kind::SingleTransferImmediate), // LDR with an immediate offset
        (0xE5C0_1004, Kind::SingleTransferImmediate), // STRB with an immediate offset
        (0xE790_1002, Kind::SingleTransferRegister),  // LDR with a register offset
        (0xE780_1102, Kind::SingleTransferRegister),  // STR, register offset shifted by 2
        // Words inside the PSR space that the halfword and MSR/MRS checks must tell apart.
        (0xE100_00B0, Kind::HalfwordTransfer), // STRH, P = 1, U = 0, no writeback
        (0xE10F_0000, Kind::Mrs),              // MRS
        (0xE129_F000, Kind::MsrRegister),      // MSR CPSR_fc, R0
        // Branches (`docs/hw/arm-branch-pipeline.md`, "Encodings"): bits 27-25 = 101, and BX
        // is bits 27-20 = 0x12 with bits 7-4 = 0001, inside the PSR-transfer space.
        (0xEA00_0000, Kind::Branch),         // B
        (0xEB00_0000, Kind::Branch),         // BL
        (0x0A00_0000, Kind::Branch),         // BEQ
        (0xE12F_FF11, Kind::BranchExchange), // BX R1
        (0xE12F_FF10, Kind::BranchExchange), // BX R0 with bits 3-0 = 0 (Rn = R0)
        (0xE12F_FF31, Kind::Unhandled),      // BLX register is the ARM9's, not BX
        (0xE16F_0F11, Kind::Unhandled),      // CLZ is the ARM9's, not BX
        // Exceptions (`docs/hw/arm-exceptions-modes.md`): SWI is bits 27-24 = 1111; the
        // undefined space is bits 27-25 = 011 with bit 4 = 1; the coprocessor instructions are
        // bits 27-25 = 110 and bits 27-24 = 1110.
        (0xEF00_0000, Kind::Swi),         // SWI 0
        (0xEFFF_FFFF, Kind::Swi),         // SWI with every comment bit set
        (0xE790_1012, Kind::Undefined),   // bits 27-25 = 011 with bit 4 = 1
        (0xE600_0010, Kind::Undefined),   // the lowest word of the space
        (0xE7F0_00F0, Kind::Undefined),   // the word GBATEK sets aside for user use
        (0xE7FF_FFFF, Kind::Undefined),   // the highest word of the space
        (0xEE00_0000, Kind::Coprocessor), // CDP
        (0xEE00_0010, Kind::Coprocessor), // MCR
        (0xEE10_0010, Kind::Coprocessor), // MRC
        (0xED90_0000, Kind::Coprocessor), // LDC
        (0xED80_0000, Kind::Coprocessor), // STC
        (0xEC40_0000, Kind::Coprocessor), // MCRR
        (0xEC50_0000, Kind::Coprocessor), // MRRC
    ];

    /// Words and the kind each must decode to on the ARM9 (`docs/hw/armv5te.md`).
    const ARM9_CASES: [(u32, Kind); 30] = [
        (0xE12F_FF31, Kind::BranchLinkExchange),          // BLX R1
        (0xE12F_FF11, Kind::BranchExchange),              // BX R1 is unchanged
        (0xE16F_2F11, Kind::CountLeadingZeros),           // CLZ R2, R1
        (0xE100_0050, Kind::SaturatingArithmetic),        // QADD
        (0xE120_0050, Kind::SaturatingArithmetic),        // QSUB
        (0xE140_0050, Kind::SaturatingArithmetic),        // QDADD
        (0xE160_0050, Kind::SaturatingArithmetic),        // QDSUB
        (0xE100_0080, Kind::SignedHalfwordMultiply),      // SMLABB
        (0xE100_00E0, Kind::SignedHalfwordMultiply),      // SMLATT
        (0xE120_0080, Kind::SignedHalfwordMultiply),      // SMLAWB
        (0xE120_00A0, Kind::SignedHalfwordMultiply),      // SMULWB
        (0xE140_0080, Kind::SignedHalfwordMultiply),      // SMLALBB
        (0xE160_0080, Kind::SignedHalfwordMultiply),      // SMULBB
        (0xE1C0_20D8, Kind::DoubleTransfer),              // LDRD
        (0xE1C0_20F8, Kind::DoubleTransfer),              // STRD
        (0xE1D0_20D8, Kind::HalfwordTransfer),            // LDRSB is unchanged
        (0xE1C0_20B8, Kind::HalfwordTransfer),            // STRH is unchanged
        (0xF5D0_F004, Kind::Preload),                     // PLD [R0, #4]
        (0xF550_F004, Kind::Preload),                     // PLD [R0, #-4]
        (0xF7D0_F001, Kind::Preload),                     // PLD [R0, R1]
        (0xF7D0_F011, Kind::Unhandled),                   // register form with bit 4 set
        (0xF5C0_F004, Kind::Unhandled),                   // bits 22 and 20 not both set: not PLD
        (0xFA00_0000, Kind::BranchLinkExchangeImmediate), // BLX +0
        (0xFB00_0000, Kind::BranchLinkExchangeImmediate), // BLX +2
        (0xFAFF_FFFE, Kind::BranchLinkExchangeImmediate), // BLX -8
        (0xF000_0000, Kind::Unhandled),                   // another word in the 1111 space
        (0xE000_0090, Kind::Multiply),                    // MUL is unchanged
        (0xE12F_FF1E, Kind::BranchExchange),              // BX LR is unchanged
        (0xE120_0070, Kind::Breakpoint),                  // BKPT #0
        (0xE12F_FF7F, Kind::Breakpoint),                  // BKPT with every immediate bit set
    ];

    fn arm9_classification_failures() -> Vec<(u32, Kind, Kind)> {
        ARM9_CASES
            .iter()
            .map(|&(word, want)| (word, want, decode::<true>(word)))
            .filter(|(_, want, got)| want != got)
            .collect()
    }

    #[test]
    fn given_arm9_encodings_when_decoded_then_each_gets_its_kind() {
        assert_eq!(arm9_classification_failures(), vec![]);
    }

    fn classification_failures() -> Vec<(u32, Kind, Kind)> {
        CASES
            .iter()
            .map(|&(word, want)| (word, want, decode::<false>(word)))
            .filter(|(_, want, got)| want != got)
            .collect()
    }

    #[test]
    fn given_multiply_transfer_and_swap_words_when_decoded_then_each_word_gets_its_kind() {
        assert_eq!(classification_failures(), vec![]);
    }
}
