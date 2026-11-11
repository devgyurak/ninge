// SPDX-License-Identifier: Apache-2.0
//! Tests of `thumb_decode`.

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
    (0x4788, ThumbKind::BranchExchange),    // BX with H1 = 1: BLX on the ARM9
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
    (0xBE00, ThumbKind::Unhandled),         // BKPT is the ARM9's
    (0xC001, ThumbKind::BlockTransfer),     // STMIA r0!, {r0}
    (0xC801, ThumbKind::BlockTransfer),     // LDMIA r0!, {r0}
    (0xD000, ThumbKind::ConditionalBranch), // BEQ
    (0xDD00, ThumbKind::ConditionalBranch), // BLE, the last real condition
    (0xDE00, ThumbKind::ConditionalBranch), // condition 1110, always taken
    (0xDEFF, ThumbKind::ConditionalBranch), // condition 1110, every offset bit set
    (0xDF00, ThumbKind::Swi),               // SWI 0
    (0xDFFF, ThumbKind::Swi),               // SWI 255
    (0xE000, ThumbKind::Branch),            // B
    (0xE800, ThumbKind::Unhandled),         // the BLX suffix is the ARM9's
    (0xF000, ThumbKind::LongBranchPrefix),  // BL prefix
    (0xF7FF, ThumbKind::LongBranchPrefix),  // BL prefix, the last one
];

fn classification_failures() -> Vec<(u16, ThumbKind, ThumbKind)> {
    CASES
        .iter()
        .map(|&(halfword, want)| (halfword, want, decode::<false>(halfword)))
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
        .filter(|&h| decode::<false>(h) != ThumbKind::LongBranchSuffix)
        .collect();
    assert_eq!(wrong, Vec::<u16>::new());
}
