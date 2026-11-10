// SPDX-License-Identifier: Apache-2.0
//! Tests of `decode`.

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
