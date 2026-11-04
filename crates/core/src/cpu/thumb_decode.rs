// SPDX-License-Identifier: Apache-2.0
//! The Thumb instruction decoder: a table built at compile time, indexed by instruction bits
//! 15–6 (`docs/hw/thumb.md`, "Formats"). Ten bits are enough to tell every format apart,
//! including the condition of a conditional branch and the H1 and H2 bits of format 5.
//!
//! Decided in #78.

/// Which handler a Thumb halfword runs. The variants follow the numbered formats of
/// `docs/hw/thumb.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ThumbKind {
    /// Not executed: the step only advances the PC. The ARMv5 encodings (the BLX forms, BKPT)
    /// and the unused part of the `1011` space.
    // TODO(hw-unverified): what ARMv4T hardware does with these is not stated; a plain step is
    // a guess (`docs/hw/thumb.md`, "Unverified").
    // TODO(phase1): the ARMv5 forms (BLX register and immediate, BKPT) arrive in #14.
    Unhandled,
    /// Format 1: LSL, LSR, ASR by an immediate.
    ShiftImmediate,
    /// Format 2: ADD and SUB of a register or a 3-bit immediate.
    AddSub,
    /// Format 3: MOV, CMP, ADD, SUB with an 8-bit immediate.
    Immediate,
    /// Format 4: the sixteen register ALU operations.
    Alu,
    /// Format 5: ADD, CMP, MOV on the high registers.
    HiRegister,
    /// Format 5: BX.
    // TODO(phase1): with H1 = 1 this is BLX (register) on the ARM9 (#14); ARMv4T runs it as BX.
    BranchExchange,
    /// Format 6: LDR Rd, [PC, #imm].
    LoadPc,
    /// Format 7: STR, STRB, LDR, LDRB with a register offset.
    TransferRegister,
    /// Format 8: STRH, LDRH, LDSB, LDSH with a register offset.
    TransferSigned,
    /// Format 9: STR, LDR, STRB, LDRB with an immediate offset.
    TransferImmediate,
    /// Format 10: STRH and LDRH with an immediate offset.
    TransferHalfword,
    /// Format 11: STR and LDR relative to SP.
    TransferSp,
    /// Format 12: ADD Rd, PC or SP, #imm.
    LoadAddress,
    /// Format 13: ADD SP, #imm and SUB SP, #imm.
    AdjustSp,
    /// Format 14: PUSH and POP.
    PushPop,
    /// Format 15: STMIA and LDMIA.
    BlockTransfer,
    /// Format 16: B with a condition (0000–1110; 1110 always holds).
    ConditionalBranch,
    /// Format 17: SWI (the condition field of format 16 reads 1111).
    Swi,
    /// Format 18: B.
    Branch,
    /// Format 19: the first halfword of BL.
    LongBranchPrefix,
    /// Format 19: the second halfword of BL.
    LongBranchSuffix,
}

const TABLE_SIZE: usize = 1 << 10;

const TABLE: [ThumbKind; TABLE_SIZE] = build_table();

/// The table index of a halfword: its bits 15–6.
const fn index(halfword: u16) -> usize {
    (halfword >> 6) as usize
}

/// Classifies the encoding space of one table index (bit `k` of the index is bit `k + 6` of the
/// halfword).
const fn classify(index: usize) -> ThumbKind {
    let top5 = index >> 5; // bits 15–11
    let bit10 = index >> 4 & 1;
    let bit9 = index >> 3 & 1;
    match top5 {
        // bits 15–13 = 000: bits 12–11 = 11 is the add/subtract format
        0b00000..=0b00010 => ThumbKind::ShiftImmediate,
        0b00011 => ThumbKind::AddSub,
        0b00100..=0b00111 => ThumbKind::Immediate,
        // bits 15–10 = 010000 is the ALU format, 010001 the high-register format
        0b01000 => match (bit10, index >> 2 & 3, index >> 1 & 1) {
            (0, _, _) => ThumbKind::Alu,
            // Op 3 is BX.
            // TODO(hw-unverified): with H1 = 1 ARMv4T still runs it as BX, which only the suite
            // shows (`docs/hw/thumb.md`, "Unverified").
            (_, 3, _) => ThumbKind::BranchExchange,
            _ => ThumbKind::HiRegister,
        },
        0b01001 => ThumbKind::LoadPc,
        // bits 15–12 = 0101: bit 9 picks the sign-extended format
        0b01010 | 0b01011 => match bit9 {
            1 => ThumbKind::TransferSigned,
            _ => ThumbKind::TransferRegister,
        },
        0b01100..=0b01111 => ThumbKind::TransferImmediate,
        0b10000 | 0b10001 => ThumbKind::TransferHalfword,
        0b10010 | 0b10011 => ThumbKind::TransferSp,
        0b10100 | 0b10101 => ThumbKind::LoadAddress,
        // bits 15–12 = 1011: bits 15–8 = 10110000 is the SP adjustment; bits 10–9 = 10 is
        // PUSH and POP
        0b10110 | 0b10111 => match (index >> 2 == 0b1011_0000, bit10, bit9) {
            (true, _, _) => ThumbKind::AdjustSp,
            (_, 1, 0) => ThumbKind::PushPop,
            _ => ThumbKind::Unhandled,
        },
        0b11000 | 0b11001 => ThumbKind::BlockTransfer,
        // bits 15–12 = 1101: the condition is bits 11–8; 1111 is SWI.
        // TODO(hw-unverified): 1110 is taken as "always", which only the ARMv4T suite shows; the
        // ARM9 may take the Undefined exception instead
        // (`docs/hw/thumb.md`, "Unverified").
        0b11010 | 0b11011 => match index >> 2 & 0xF {
            0xF => ThumbKind::Swi,
            _ => ThumbKind::ConditionalBranch,
        },
        0b11100 => ThumbKind::Branch,
        // bits 15–11 = 11101 is the BLX suffix on ARMv5 (#14)
        0b11101 => ThumbKind::Unhandled,
        0b11110 => ThumbKind::LongBranchPrefix,
        _ => ThumbKind::LongBranchSuffix,
    }
}

const fn build_table() -> [ThumbKind; TABLE_SIZE] {
    let mut table = [ThumbKind::Unhandled; TABLE_SIZE];
    let mut i = 0;
    while i < TABLE_SIZE {
        table[i] = classify(i);
        i += 1;
    }
    table
}

/// The handler kind of a Thumb halfword.
pub(crate) fn decode(halfword: u16) -> ThumbKind {
    TABLE[index(halfword)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Halfwords and the kind each must decode to (`docs/hw/thumb.md`, "Formats").
    const CASES: [(u16, ThumbKind); 52] = [
        (0x0000, ThumbKind::ShiftImmediate),    // LSL r0, r0, #0
        (0x0808, ThumbKind::ShiftImmediate),    // LSR
        (0x1008, ThumbKind::ShiftImmediate),    // ASR
        (0x1888, ThumbKind::AddSub),            // ADD r0, r1, r2
        (0x1E48, ThumbKind::AddSub),            // SUB r0, r1, #1
        (0x2001, ThumbKind::Immediate),         // MOV r0, #1
        (0x2801, ThumbKind::Immediate),         // CMP r0, #1
        (0x3001, ThumbKind::Immediate),         // ADD r0, #1
        (0x3801, ThumbKind::Immediate),         // SUB r0, #1
        (0x4008, ThumbKind::Alu),               // AND r0, r1
        (0x43C8, ThumbKind::Alu),               // MVN r0, r1
        (0x4408, ThumbKind::HiRegister),        // ADD r0, r1 (H1 = H2 = 0)
        (0x4488, ThumbKind::HiRegister),        // ADD r8, r1
        (0x4508, ThumbKind::HiRegister),        // CMP r0, r1
        (0x4608, ThumbKind::HiRegister),        // MOV r0, r1
        (0x4708, ThumbKind::BranchExchange),    // BX r1
        (0x4740, ThumbKind::BranchExchange),    // BX r8
        (0x4788, ThumbKind::BranchExchange),    // BX with H1 = 1: BLX on ARMv5 (#14)
        (0x4800, ThumbKind::LoadPc),            // LDR r0, [PC, #0]
        (0x5088, ThumbKind::TransferRegister),  // STR r0, [r1, r2]
        (0x5C88, ThumbKind::TransferRegister),  // LDRB r0, [r1, r2]
        (0x5288, ThumbKind::TransferSigned),    // STRH r0, [r1, r2]
        (0x5688, ThumbKind::TransferSigned),    // LDSB
        (0x5E88, ThumbKind::TransferSigned),    // LDSH
        (0x6008, ThumbKind::TransferImmediate), // STR r0, [r1, #0]
        (0x7808, ThumbKind::TransferImmediate), // LDRB r0, [r1, #0]
        (0x8008, ThumbKind::TransferHalfword),  // STRH r0, [r1, #0]
        (0x8808, ThumbKind::TransferHalfword),  // LDRH r0, [r1, #0]
        (0x9000, ThumbKind::TransferSp),        // STR r0, [SP, #0]
        (0x9800, ThumbKind::TransferSp),        // LDR r0, [SP, #0]
        (0xA000, ThumbKind::LoadAddress),       // ADD r0, PC, #0
        (0xA800, ThumbKind::LoadAddress),       // ADD r0, SP, #0
        (0xB000, ThumbKind::AdjustSp),          // ADD SP, #0
        (0xB080, ThumbKind::AdjustSp),          // SUB SP, #0
        (0xB401, ThumbKind::PushPop),           // PUSH {r0}
        (0xB500, ThumbKind::PushPop),           // PUSH {LR}
        (0xBC01, ThumbKind::PushPop),           // POP {r0}
        (0xBD00, ThumbKind::PushPop),           // POP {PC}
        (0xB100, ThumbKind::Unhandled),         // the unused part of the 1011 space
        (0xBE00, ThumbKind::Unhandled),         // BKPT (ARMv5, #14)
        (0xC001, ThumbKind::BlockTransfer),     // STMIA r0!, {r0}
        (0xC801, ThumbKind::BlockTransfer),     // LDMIA r0!, {r0}
        (0xD000, ThumbKind::ConditionalBranch), // BEQ
        (0xDD00, ThumbKind::ConditionalBranch), // BLE, the last real condition
        (0xDE00, ThumbKind::ConditionalBranch), // condition 1110, always taken
        (0xDEFF, ThumbKind::ConditionalBranch), // condition 1110, every offset bit set
        (0xDF00, ThumbKind::Swi),               // SWI 0
        (0xDFFF, ThumbKind::Swi),               // SWI 255
        (0xE000, ThumbKind::Branch),            // B
        (0xE800, ThumbKind::Unhandled),         // the BLX suffix (ARMv5, #14)
        (0xF000, ThumbKind::LongBranchPrefix),  // BL prefix
        (0xF7FF, ThumbKind::LongBranchPrefix),  // BL prefix, the last one
    ];

    fn classification_failures() -> Vec<(u16, ThumbKind, ThumbKind)> {
        CASES
            .iter()
            .map(|&(halfword, want)| (halfword, want, decode(halfword)))
            .filter(|(_, want, got)| want != got)
            .collect()
    }

    #[test]
    fn given_a_halfword_of_each_format_when_decoded_then_each_gets_its_kind() {
        assert_eq!(classification_failures(), vec![]);
    }

    #[test]
    fn given_the_bl_suffix_range_when_decoded_then_every_halfword_is_a_suffix() {
        let wrong: Vec<u16> = (0xF800..=0xFFFF)
            .filter(|&h| decode(h) != ThumbKind::LongBranchSuffix)
            .collect();
        assert_eq!(wrong, Vec::<u16>::new());
    }
}
