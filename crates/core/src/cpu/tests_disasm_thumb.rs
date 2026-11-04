// SPDX-License-Identifier: Apache-2.0
//! The Thumb disassembler. Every expected string is written by hand from
//! `docs/hw/disassembly.md` (notation) and `docs/hw/thumb.md` / `docs/hw/armv5te.md`
//! (encodings): each halfword is assembled from its bit fields in the comments of the rows, never
//! taken from a disassembler or from running the implementation.
//!
//! A table row is `(halfword, next halfword, address, arm9, expected text)`. The harness also
//! checks that the row decodes to the `ThumbKind` the table is about, so a row cannot sit under
//! the wrong format.

use super::disassemble_thumb;
use super::thumb_decode::{ThumbKind, decode};
use proptest::prelude::*;

/// Address of the instruction under test unless a row says otherwise.
const A: u32 = 0x1000;

/// `(halfword, next, address, arm9, expected text)`.
type Row = (u16, Option<u16>, u32, bool, &'static str);
/// `(halfword, next, address, arm9, expected branch target)`.
type TargetRow = (u16, Option<u16>, u32, bool, Option<u32>);

// ---------------------------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------------------------

fn render(halfword: u16, next: Option<u16>, address: u32, arm9: bool) -> String {
    if arm9 {
        disassemble_thumb::<true>(halfword, next, address).to_string()
    } else {
        disassemble_thumb::<false>(halfword, next, address).to_string()
    }
}

fn target_of(halfword: u16, next: Option<u16>, address: u32, arm9: bool) -> Option<u32> {
    if arm9 {
        disassemble_thumb::<true>(halfword, next, address).branch_target()
    } else {
        disassemble_thumb::<false>(halfword, next, address).branch_target()
    }
}

fn kind_of(halfword: u16, arm9: bool) -> ThumbKind {
    if arm9 {
        decode::<true>(halfword)
    } else {
        decode::<false>(halfword)
    }
}

/// Runs every row and returns one line per row whose text or decoded kind differs.
fn table_failures(kind: ThumbKind, rows: &[Row]) -> Vec<String> {
    let mut failures = Vec::new();
    for &(halfword, next, address, arm9, expected) in rows {
        let got = render(halfword, next, address, arm9);
        let got_kind = kind_of(halfword, arm9);
        if got != expected || got_kind != kind {
            failures.push(format!(
                "{halfword:#06x} next={next:?} @{address:#x} arm9={arm9}: \
                 got {got:?} ({got_kind:?}), expected {expected:?} ({kind:?})"
            ));
        }
    }
    failures
}

/// Runs every row of branch targets and returns one line per row that differs.
fn target_failures(rows: &[TargetRow]) -> Vec<String> {
    let mut failures = Vec::new();
    for &(halfword, next, address, arm9, expected) in rows {
        let got = target_of(halfword, next, address, arm9);
        if got != expected {
            failures.push(format!(
                "{halfword:#06x} next={next:?} @{address:#x} arm9={arm9}: \
                 got {got:x?}, expected {expected:x?}"
            ));
        }
    }
    failures
}

/// Whether the first token of `text` is a mnemonic the decoder's `kind` may print. The BL pieces
/// are the parenthesised forms `(bl ...` and `(blx ...`.
fn mnemonic_matches_kind(kind: ThumbKind, text: &str) -> bool {
    let token = text.split(' ').next().unwrap_or("");
    match kind {
        ThumbKind::Unhandled => token == "undefined",
        ThumbKind::ShiftImmediate => matches!(token, "lsl" | "lsr" | "asr"),
        ThumbKind::AddSub => matches!(token, "add" | "sub"),
        ThumbKind::Immediate => matches!(token, "mov" | "cmp" | "add" | "sub"),
        ThumbKind::Alu => matches!(
            token,
            "and"
                | "eor"
                | "lsl"
                | "lsr"
                | "asr"
                | "adc"
                | "sbc"
                | "ror"
                | "tst"
                | "neg"
                | "cmp"
                | "cmn"
                | "orr"
                | "mul"
                | "bic"
                | "mvn"
        ),
        ThumbKind::HiRegister => matches!(token, "add" | "cmp" | "mov"),
        ThumbKind::BranchExchange => token == "bx",
        ThumbKind::BranchLinkExchange => token == "blx",
        ThumbKind::LoadPc => token == "ldr",
        ThumbKind::TransferRegister | ThumbKind::TransferImmediate => {
            matches!(token, "str" | "strb" | "ldr" | "ldrb")
        }
        ThumbKind::TransferSigned => matches!(token, "strh" | "ldsb" | "ldrh" | "ldsh"),
        ThumbKind::TransferHalfword => matches!(token, "strh" | "ldrh"),
        ThumbKind::TransferSp => matches!(token, "str" | "ldr"),
        ThumbKind::LoadAddress => token == "add",
        ThumbKind::AdjustSp => matches!(token, "add" | "sub"),
        ThumbKind::PushPop => matches!(token, "push" | "pop"),
        ThumbKind::BlockTransfer => matches!(token, "stmia" | "ldmia"),
        ThumbKind::ConditionalBranch => matches!(
            token,
            "beq"
                | "bne"
                | "bcs"
                | "bcc"
                | "bmi"
                | "bpl"
                | "bvs"
                | "bvc"
                | "bhi"
                | "bls"
                | "bge"
                | "blt"
                | "bgt"
                | "ble"
                | "bal"
        ),
        ThumbKind::Swi => token == "swi",
        ThumbKind::Branch => token == "b",
        ThumbKind::LongBranchPrefix => matches!(token, "bl" | "blx" | "(bl"),
        ThumbKind::LongBranchSuffix => token == "(bl",
        ThumbKind::Breakpoint => token == "bkpt",
        ThumbKind::LongBranchExchangeSuffix => token == "(blx",
    }
}

/// The text printed on `ARM9` for the halfword matches the decoder's kind for that processor.
fn class_failure(halfword: u16, next: Option<u16>, address: u32, arm9: bool) -> Option<String> {
    let text = render(halfword, next, address, arm9);
    let kind = kind_of(halfword, arm9);
    if mnemonic_matches_kind(kind, &text) {
        None
    } else {
        Some(format!("{halfword:#06x} {kind:?}: {text:?}"))
    }
}

/// The exact first token the note gives `halfword`, for the formats where one bit field picks
/// the mnemonic, unpacked here from the bits and not from the implementation.
fn expected_token(halfword: u16, arm9: bool) -> Option<&'static str> {
    const SHIFTS: [&str; 3] = ["lsl", "lsr", "asr"];
    const IMMEDIATES: [&str; 4] = ["mov", "cmp", "add", "sub"];
    const ALU: [&str; 16] = [
        "and", "eor", "lsl", "lsr", "asr", "adc", "sbc", "ror", "tst", "neg", "cmp", "cmn", "orr",
        "mul", "bic", "mvn",
    ];
    const HI: [&str; 3] = ["add", "cmp", "mov"];
    const REGISTER: [&str; 4] = ["str", "strb", "ldr", "ldrb"];
    const SIGNED: [&str; 4] = ["strh", "ldsb", "ldrh", "ldsh"];
    let h = usize::from(halfword);
    let bit = |n: usize| h >> n & 1 == 1;
    match kind_of(halfword, arm9) {
        ThumbKind::ShiftImmediate => Some(SHIFTS[h >> 11 & 3]),
        ThumbKind::AddSub => Some(if bit(9) { "sub" } else { "add" }),
        ThumbKind::Immediate => Some(IMMEDIATES[h >> 11 & 3]),
        ThumbKind::Alu => Some(ALU[h >> 6 & 0xF]),
        ThumbKind::HiRegister => Some(HI[h >> 8 & 3]),
        ThumbKind::TransferRegister => Some(REGISTER[h >> 10 & 3]),
        ThumbKind::TransferSigned => Some(SIGNED[h >> 10 & 3]),
        ThumbKind::TransferImmediate => Some(REGISTER[(h >> 11 & 1) << 1 | h >> 12 & 1]),
        ThumbKind::TransferHalfword => Some(if bit(11) { "ldrh" } else { "strh" }),
        ThumbKind::TransferSp => Some(if bit(11) { "ldr" } else { "str" }),
        ThumbKind::PushPop => Some(if bit(11) { "pop" } else { "push" }),
        ThumbKind::BlockTransfer => Some(if bit(11) { "ldmia" } else { "stmia" }),
        ThumbKind::AdjustSp => Some(if bit(7) { "sub" } else { "add" }),
        _ => None,
    }
}

/// Why the first token of the text differs from `expected_token`, if it does.
fn exact_token_failure(halfword: u16, address: u32, arm9: bool) -> Option<String> {
    let expected = expected_token(halfword, arm9)?;
    let text = render(halfword, None, address, arm9);
    let token = text.split(' ').next().unwrap_or("");
    if token == expected {
        None
    } else {
        Some(format!(
            "{halfword:#06x}: expected {expected:?}, got {text:?}"
        ))
    }
}

/// Text on the ARM7 and the ARM9 is the same whenever both decode the halfword alike and the
/// kind does not depend on the next halfword.
fn cpu_text_agrees(halfword: u16, next: Option<u16>, address: u32) -> bool {
    let same_kind = decode::<false>(halfword) == decode::<true>(halfword);
    let is_prefix = decode::<true>(halfword) == ThumbKind::LongBranchPrefix;
    if same_kind && !is_prefix {
        render(halfword, next, address, false) == render(halfword, next, address, true)
    } else {
        true
    }
}

/// Text is ASCII with no upper-case letter (`docs/hw/disassembly.md`, "Common notation").
fn text_is_lowercase_ascii(halfword: u16, next: Option<u16>, address: u32, arm9: bool) -> bool {
    let text = render(halfword, next, address, arm9);
    !text.is_empty() && text.is_ascii() && !text.chars().any(|c| c.is_ascii_uppercase())
}

/// Formats 16 and 18 target `address + 4 + sext(offset) * 2`, computed here independently.
fn expected_branch_target(halfword: u16, address: u32) -> Option<u32> {
    if halfword >> 12 == 0xD && (halfword >> 8) & 0xF != 0xF {
        let offset = i32::from(halfword as u8 as i8);
        Some(address.wrapping_add(4).wrapping_add((offset * 2) as u32))
    } else if halfword >> 11 == 0b11100 {
        let offset = (i32::from(halfword & 0x7FF) << 21) >> 21;
        Some(address.wrapping_add(4).wrapping_add((offset * 2) as u32))
    } else {
        None
    }
}

/// With no next halfword the target is `expected_branch_target` on both processors (`None` for
/// everything that is not format 16 or 18, a partial BL prefix included).
fn branch_target_follows_formula(halfword: u16, address: u32) -> bool {
    let want = expected_branch_target(halfword, address);
    target_of(halfword, None, address, false) == want
        && target_of(halfword, None, address, true) == want
}

/// A complete BL (or, on the ARM9, BLX) pair prints and targets
/// `address + 4 + (sext(off11) << 12) + (suffix off11 << 1)`, with bits 1-0 cleared for BLX.
fn complete_pair_matches(prefix_off: u16, suffix_off: u16, address: u32, blx: bool) -> bool {
    let prefix = 0xF000 | prefix_off;
    let suffix = if blx { 0xE800 } else { 0xF800 } | suffix_off;
    let high = ((i32::from(prefix_off) << 21) >> 9) as u32;
    let raw = address
        .wrapping_add(4)
        .wrapping_add(high)
        .wrapping_add(u32::from(suffix_off) << 1);
    let target = if blx { raw & !3 } else { raw };
    let text = format!("{} {target:#010x}", if blx { "blx" } else { "bl" });
    let arm9_ok = render(prefix, Some(suffix), address, true) == text
        && target_of(prefix, Some(suffix), address, true) == Some(target);
    // The ARM7 has no BLX suffix, so only the plain pair is checked there.
    let arm7_ok = blx
        || (render(prefix, Some(suffix), address, false) == text
            && target_of(prefix, Some(suffix), address, false) == Some(target));
    arm9_ok && arm7_ok
}

/// Format 6 prints the word-aligned PC read plus the offset, both in the text and the comment.
fn pc_load_matches(rd: u16, word: u16, address: u32, arm9: bool) -> bool {
    let halfword = 0x4800 | (rd << 8) | word;
    let read = (address.wrapping_add(4) & !3).wrapping_add(u32::from(word) * 4);
    let want = format!(
        "ldr r{rd}, [pc, #{:#x}] ; {read:#010x}",
        u32::from(word) * 4
    );
    render(halfword, None, address, arm9) == want
}

// ---------------------------------------------------------------------------------------------
// One table per ThumbKind
// ---------------------------------------------------------------------------------------------

/// `undefined 0x` and the halfword, four lowercase hex digits (`disassembly.md`, "The decoder
/// decides"). 0xB100 is outside the 1011 formats; 0xBE00 (BKPT) and 0xE800 (BLX suffix) are
/// ARMv5-only.
const UNHANDLED_ROWS: &[Row] = &[
    (0xB100, None, A, false, "undefined 0xb100"),
    (0xB100, None, A, true, "undefined 0xb100"),
    (0xB1FF, None, A, true, "undefined 0xb1ff"),
    (0xB200, None, A, false, "undefined 0xb200"),
    (0xB600, None, A, true, "undefined 0xb600"),
    (0xBF00, None, A, true, "undefined 0xbf00"),
    (0xBE00, None, A, false, "undefined 0xbe00"),
    (0xBEFF, None, A, false, "undefined 0xbeff"),
    (0xE800, None, A, false, "undefined 0xe800"),
    (0xE800, Some(0xF800), A, false, "undefined 0xe800"),
    (0xEFFF, None, A, false, "undefined 0xefff"),
];

/// Format 1: 000 op(2) off5 rs rd.
const SHIFT_ROWS: &[Row] = &[
    // lsl off=0 rs=1 rd=0
    (0x0008, None, A, false, "lsl r0, r1, #0"),
    // lsl off=5 rs=3 rd=2
    (0x015A, None, A, true, "lsl r2, r3, #5"),
    // lsl off=31 rs=0 rd=0
    (0x07C0, None, A, false, "lsl r0, r0, #31"),
    // lsr off=0 means 32
    (0x0808, None, A, false, "lsr r0, r1, #32"),
    (0x0808, None, A, true, "lsr r0, r1, #32"),
    // lsr off=31 rs=5 rd=4
    (0x0FEC, None, A, false, "lsr r4, r5, #31"),
    // asr off=0 means 32
    (0x1008, None, A, false, "asr r0, r1, #32"),
    (0x1008, None, A, true, "asr r0, r1, #32"),
    // asr off=1 rs=6 rd=7
    (0x1077, None, A, true, "asr r7, r6, #1"),
];

/// Format 2: 00011 I Op rn/imm3 rs rd.
const ADD_SUB_ROWS: &[Row] = &[
    (0x1888, None, A, false, "add r0, r1, r2"),
    (0x1B63, None, A, true, "sub r3, r4, r5"),
    (0x1CC8, None, A, false, "add r0, r1, #0x3"),
    (0x1FDA, None, A, true, "sub r2, r3, #0x7"),
];

/// Format 3: 001 op(2) rd imm8; op 0 mov, 1 cmp, 2 add, 3 sub.
const IMMEDIATE_ROWS: &[Row] = &[
    (0x2001, None, A, false, "mov r0, #0x1"),
    (0x2000, None, A, true, "mov r0, #0x0"),
    (0x29FF, None, A, false, "cmp r1, #0xff"),
    (0x3780, None, A, true, "add r7, #0x80"),
    (0x3A10, None, A, false, "sub r2, #0x10"),
];

/// Format 4: 010000 op(4) rs rd, with rs = 1 and rd = 0 unless noted.
const ALU_ROWS: &[Row] = &[
    (0x4008, None, A, false, "and r0, r1"),
    (0x4048, None, A, true, "eor r0, r1"),
    (0x4088, None, A, false, "lsl r0, r1"),
    (0x40C8, None, A, true, "lsr r0, r1"),
    (0x4108, None, A, false, "asr r0, r1"),
    (0x4148, None, A, true, "adc r0, r1"),
    (0x4188, None, A, false, "sbc r0, r1"),
    (0x41C8, None, A, true, "ror r0, r1"),
    (0x4208, None, A, false, "tst r0, r1"),
    (0x4248, None, A, true, "neg r0, r1"),
    (0x4288, None, A, false, "cmp r0, r1"),
    (0x42C8, None, A, true, "cmn r0, r1"),
    (0x4308, None, A, false, "orr r0, r1"),
    (0x4348, None, A, true, "mul r0, r1"),
    (0x4388, None, A, false, "bic r0, r1"),
    (0x43C8, None, A, true, "mvn r0, r1"),
    // and, rs=6 rd=7
    (0x4037, None, A, false, "and r7, r6"),
];

/// Format 5, add/cmp/mov: 010001 op(2) H1 H2 rs rd; H1 and H2 add 8 to rd and rs.
const HI_REGISTER_ROWS: &[Row] = &[
    // add, H2=1: rs = r8
    (0x4440, None, A, false, "add r0, r8"),
    // add, H1=1: rd = r8
    (0x4480, None, A, true, "add r8, r0"),
    // add, H2=1 rs=7: pc
    (0x4479, None, A, false, "add r1, pc"),
    // cmp, H2=1 rs=1: r9
    (0x4549, None, A, true, "cmp r1, r9"),
    // mov, no high bits
    (0x4608, None, A, false, "mov r0, r1"),
    // mov, H1=1 rd=5: sp
    (0x4685, None, A, true, "mov sp, r0"),
    // mov, H2=1 rs=6: lr
    (0x4670, None, A, false, "mov r0, lr"),
    // mov, H1=1 H2=1 rs=6 rd=7: pc, lr
    (0x46F7, None, A, true, "mov pc, lr"),
];

/// Format 5, BX: 010001 11 H1 H2 rs 000. On the ARM7 H1 = 1 is still BX.
const BRANCH_EXCHANGE_ROWS: &[Row] = &[
    (0x4708, None, A, false, "bx r1"),
    (0x4708, None, A, true, "bx r1"),
    // H2=1 rs=6: lr
    (0x4770, None, A, false, "bx lr"),
    (0x4770, None, A, true, "bx lr"),
    // ARM7 with H1=1 prints bx (the ARM9 reads it as BLX)
    (0x4788, None, A, false, "bx r1"),
    (0x47C0, None, A, false, "bx r8"),
    (0x47F0, None, A, false, "bx lr"),
];

/// Format 5 with H1 = 1 on the ARM9 is BLX with a register.
const BRANCH_LINK_EXCHANGE_ROWS: &[Row] = &[
    (0x4788, None, A, true, "blx r1"),
    // H2=1 rs=0: r8
    (0x47C0, None, A, true, "blx r8"),
    // H2=1 rs=6: lr
    (0x47F0, None, A, true, "blx lr"),
];

/// Format 6: 01001 rd word8. The comment is the word-aligned PC plus word8 * 4.
const LOAD_PC_ROWS: &[Row] = &[
    // word8=4 at aligned addresses: (0x02000104 + 4) + 0x10
    (
        0x4804,
        None,
        0x0200_0104,
        false,
        "ldr r0, [pc, #0x10] ; 0x02000118",
    ),
    // bit 1 of the address set: PC = 0x0200010A reads as 0x02000108
    (
        0x4804,
        None,
        0x0200_0106,
        true,
        "ldr r0, [pc, #0x10] ; 0x02000118",
    ),
    // PC = 0x02000106 reads as 0x02000104
    (
        0x4804,
        None,
        0x0200_0102,
        false,
        "ldr r0, [pc, #0x10] ; 0x02000114",
    ),
    (
        0x4804,
        None,
        0x0200_0100,
        true,
        "ldr r0, [pc, #0x10] ; 0x02000114",
    ),
    // word8=255 rd=7
    (
        0x4FFF,
        None,
        0x1000,
        false,
        "ldr r7, [pc, #0x3fc] ; 0x00001400",
    ),
    // word8=0: the offset is dropped, the comment stays; PC 0x1006 reads as 0x1004
    (0x4900, None, 0x1002, true, "ldr r1, [pc] ; 0x00001004"),
    (0x4900, None, 0x1000, false, "ldr r1, [pc] ; 0x00001004"),
];

/// Format 7: 0101 L B 0 ro rb rd, with ro = 2 rb = 1 rd = 0.
const TRANSFER_REGISTER_ROWS: &[Row] = &[
    (0x5088, None, A, false, "str r0, [r1, r2]"),
    (0x5488, None, A, true, "strb r0, [r1, r2]"),
    (0x5888, None, A, false, "ldr r0, [r1, r2]"),
    (0x5C88, None, A, true, "ldrb r0, [r1, r2]"),
];

/// Format 8: 0101 H S 1 ro rb rd, with ro = 2 rb = 1 rd = 0.
const TRANSFER_SIGNED_ROWS: &[Row] = &[
    (0x5288, None, A, false, "strh r0, [r1, r2]"),
    (0x5688, None, A, true, "ldsb r0, [r1, r2]"),
    (0x5A88, None, A, false, "ldrh r0, [r1, r2]"),
    (0x5E88, None, A, true, "ldsh r0, [r1, r2]"),
];

/// Format 9: 011 B L off5 rb rd. Word offsets are scaled by 4, byte offsets are not.
const TRANSFER_IMMEDIATE_ROWS: &[Row] = &[
    // str off=1 -> #0x4
    (0x6048, None, A, false, "str r0, [r1, #0x4]"),
    // ldr off=31 rb=3 rd=2 -> #0x7c
    (0x6FDA, None, A, true, "ldr r2, [r3, #0x7c]"),
    // zero offset
    (0x6808, None, A, false, "ldr r0, [r1]"),
    // strb off=31 -> #0x1f
    (0x77C8, None, A, true, "strb r0, [r1, #0x1f]"),
    // ldrb off=1 rb=5 rd=4
    (0x7868, None, A, false, "ldrb r0, [r5, #0x1]"),
    (0x7008, None, A, true, "strb r0, [r1]"),
];

/// Format 10: 1000 L off5 rb rd. Offsets are scaled by 2.
const TRANSFER_HALFWORD_ROWS: &[Row] = &[
    (0x8048, None, A, false, "strh r0, [r1, #0x2]"),
    (0x8FDA, None, A, true, "ldrh r2, [r3, #0x3e]"),
    (0x8A35, None, A, false, "ldrh r5, [r6, #0x10]"),
    (0x8008, None, A, true, "strh r0, [r1]"),
];

/// Format 11: 1001 L rd word8, offset word8 * 4.
const TRANSFER_SP_ROWS: &[Row] = &[
    (0x9001, None, A, false, "str r0, [sp, #0x4]"),
    (0x9BFF, None, A, true, "ldr r3, [sp, #0x3fc]"),
    (0x9800, None, A, false, "ldr r0, [sp]"),
];

/// Format 12: 1010 SP rd word8, offset word8 * 4.
const LOAD_ADDRESS_ROWS: &[Row] = &[
    (0xA004, None, A, false, "add r0, pc, #0x10"),
    (0xA9FF, None, A, true, "add r1, sp, #0x3fc"),
    (0xAA00, None, A, false, "add r2, sp, #0x0"),
    // not a memory offset: the zero operand stays
    (0xA000, None, A, true, "add r0, pc, #0x0"),
];

/// Format 13: 10110000 S sword7, offset sword7 * 4.
const ADJUST_SP_ROWS: &[Row] = &[
    (0xB002, None, A, false, "add sp, #0x8"),
    (0xB0FF, None, A, true, "sub sp, #0x1fc"),
    (0xB000, None, A, false, "add sp, #0x0"),
];

/// Format 14: 1011 L 10 R rlist. 0xB4 pushes, 0xBC pops, R = bit 8 adds lr or pc.
const PUSH_POP_ROWS: &[Row] = &[
    // rlist = r0-r3, R=1
    (0xB50F, None, A, false, "push {r0-r3, lr}"),
    // a run of two stays single
    (0xB430, None, A, true, "push {r4, r5}"),
    (0xB407, None, A, false, "push {r0-r2}"),
    (0xBDFF, None, A, true, "pop {r0-r7, pc}"),
    (0xBC02, None, A, false, "pop {r1}"),
    (0xB400, None, A, true, "push {}"),
    (0xB500, None, A, false, "push {lr}"),
    (0xBD00, None, A, true, "pop {pc}"),
    // r7 and lr are not consecutive registers
    (0xB580, None, A, false, "push {r7, lr}"),
    (0xB5C0, None, A, true, "push {r6, r7, lr}"),
    (0xB5E0, None, A, false, "push {r5-r7, lr}"),
    (0xB405, None, A, true, "push {r0, r2}"),
    // bits 0 1 3 4 5
    (0xB43B, None, A, false, "push {r0, r1, r3-r5}"),
];

/// Format 15: 1100 L rb rlist.
const BLOCK_TRANSFER_ROWS: &[Row] = &[
    (0xC006, None, A, false, "stmia r0!, {r1, r2}"),
    (0xCB07, None, A, true, "ldmia r3!, {r0-r2}"),
    (0xC700, None, A, false, "stmia r7!, {}"),
    (0xC9FF, None, A, true, "ldmia r1!, {r0-r7}"),
];

/// Format 16: 1101 cond soff8, offset 0x10 at 0x1000: 0x1000 + 4 + 0x20 = 0x1024. The
/// remaining rows reach backward and forward.
const CONDITIONAL_BRANCH_ROWS: &[Row] = &[
    (0xD010, None, A, false, "beq 0x00001024"),
    (0xD110, None, A, true, "bne 0x00001024"),
    (0xD210, None, A, false, "bcs 0x00001024"),
    (0xD310, None, A, true, "bcc 0x00001024"),
    (0xD410, None, A, false, "bmi 0x00001024"),
    (0xD510, None, A, true, "bpl 0x00001024"),
    (0xD610, None, A, false, "bvs 0x00001024"),
    (0xD710, None, A, true, "bvc 0x00001024"),
    (0xD810, None, A, false, "bhi 0x00001024"),
    (0xD910, None, A, true, "bls 0x00001024"),
    (0xDA10, None, A, false, "bge 0x00001024"),
    (0xDB10, None, A, true, "blt 0x00001024"),
    (0xDC10, None, A, false, "bgt 0x00001024"),
    (0xDD10, None, A, true, "ble 0x00001024"),
    // condition 1110 is written bal
    (0xDE10, None, A, false, "bal 0x00001024"),
    (0xDE10, None, A, true, "bal 0x00001024"),
    // offset 0: the target is the PC read
    (0xD000, None, A, false, "beq 0x00001004"),
    // +127 * 2 = 254
    (0xD07F, None, A, true, "beq 0x00001102"),
    // -2 * 2: 0x1004 - 4
    (0xD0FE, None, A, false, "beq 0x00001000"),
    // -128 * 2: 0x1004 - 0x100
    (0xD180, None, A, true, "bne 0x00000f04"),
];

/// Format 17: 11011111 value8.
const SWI_ROWS: &[Row] = &[
    (0xDF00, None, A, false, "swi 0x0"),
    (0xDF06, None, A, true, "swi 0x6"),
    (0xDF0B, None, A, false, "swi 0xb"),
    (0xDFFF, None, A, true, "swi 0xff"),
];

/// Format 18: 11100 off11, target address + 4 + sext(off11) * 2.
const BRANCH_ROWS: &[Row] = &[
    (0xE000, None, A, false, "b 0x00001004"),
    (0xE010, None, A, true, "b 0x00001024"),
    // off = 0x3FF: +1023 * 2 = 0x7FE
    (0xE3FF, None, A, false, "b 0x00001802"),
    // off = 0x7FF = -1: -2
    (0xE7FF, None, A, true, "b 0x00001002"),
    // off = 0x400 = -1024: -0x800
    (0xE400, None, A, false, "b 0x00000804"),
    // off = 0x7FE = -2: a branch to itself
    (0xE7FE, None, 0x2000, true, "b 0x00002000"),
];

/// The first halfword of BL, in a mix of complete and partial forms (the three cases have their
/// own tables below).
const PREFIX_ROWS: &[Row] = &[
    (0xF000, Some(0xF800), A, false, "bl 0x00001004"),
    (0xF001, Some(0xF800), A, true, "bl 0x00002004"),
    (0xF000, Some(0xE800), A, true, "blx 0x00001004"),
    (0xF000, None, A, false, "(bl prefix lr=0x00001004)"),
    (0xF000, Some(0x4770), A, true, "(bl prefix lr=0x00001004)"),
];

/// The second halfword of BL: `next` never changes it.
const SUFFIX_ROWS: &[Row] = &[
    (0xF800, None, A, false, "(bl suffix)"),
    (0xF800, None, A, true, "(bl suffix)"),
    (0xFFFF, Some(0xF000), A, false, "(bl suffix)"),
    (0xF801, Some(0xF800), A, true, "(bl suffix)"),
];

/// BKPT: 10111110 imm8, ARM9 only.
const BREAKPOINT_ROWS: &[Row] = &[
    (0xBE00, None, A, true, "bkpt 0x0"),
    (0xBE12, None, A, true, "bkpt 0x12"),
    (0xBEFF, None, A, true, "bkpt 0xff"),
];

/// The second halfword of BLX: 11101 off11, ARM9 only.
const EXCHANGE_SUFFIX_ROWS: &[Row] = &[
    (0xE800, None, A, true, "(blx suffix)"),
    (0xEFFF, None, A, true, "(blx suffix)"),
    (0xE801, Some(0xF800), A, true, "(blx suffix)"),
];

#[test]
fn given_unhandled_halfwords_when_disassembled_then_text_is_undefined_with_four_hex_digits() {
    assert_eq!(
        table_failures(ThumbKind::Unhandled, UNHANDLED_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_shift_immediate_halfwords_when_disassembled_then_lsl_lsr_asr_print_amount_with_32_for_zero()
 {
    assert_eq!(
        table_failures(ThumbKind::ShiftImmediate, SHIFT_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_add_sub_halfwords_when_disassembled_then_register_and_three_bit_immediate_forms_print() {
    assert_eq!(
        table_failures(ThumbKind::AddSub, ADD_SUB_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_immediate_halfwords_when_disassembled_then_mov_cmp_add_sub_print_hex_immediate() {
    assert_eq!(
        table_failures(ThumbKind::Immediate, IMMEDIATE_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_all_sixteen_alu_operations_when_disassembled_then_each_prints_its_mnemonic() {
    assert_eq!(
        table_failures(ThumbKind::Alu, ALU_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_high_register_halfwords_when_disassembled_then_sp_lr_pc_and_r8_up_are_named() {
    assert_eq!(
        table_failures(ThumbKind::HiRegister, HI_REGISTER_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_bx_halfwords_when_disassembled_then_bx_prints_even_on_arm7_with_h1_set() {
    assert_eq!(
        table_failures(ThumbKind::BranchExchange, BRANCH_EXCHANGE_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_arm9_bx_with_h1_set_when_disassembled_then_blx_register_prints() {
    assert_eq!(
        table_failures(ThumbKind::BranchLinkExchange, BRANCH_LINK_EXCHANGE_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_pc_relative_load_halfwords_when_disassembled_then_comment_shows_the_word_aligned_read_address()
 {
    assert_eq!(
        table_failures(ThumbKind::LoadPc, LOAD_PC_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_register_offset_transfers_when_disassembled_then_str_strb_ldr_ldrb_print_base_and_offset_registers()
 {
    assert_eq!(
        table_failures(ThumbKind::TransferRegister, TRANSFER_REGISTER_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_signed_transfers_when_disassembled_then_strh_ldsb_ldrh_ldsh_print_in_encoding_order() {
    assert_eq!(
        table_failures(ThumbKind::TransferSigned, TRANSFER_SIGNED_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_immediate_offset_transfers_when_disassembled_then_offset_is_in_bytes_and_zero_is_omitted()
{
    assert_eq!(
        table_failures(ThumbKind::TransferImmediate, TRANSFER_IMMEDIATE_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_halfword_immediate_transfers_when_disassembled_then_offset_is_scaled_by_two() {
    assert_eq!(
        table_failures(ThumbKind::TransferHalfword, TRANSFER_HALFWORD_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_sp_relative_transfers_when_disassembled_then_offset_is_scaled_by_four() {
    assert_eq!(
        table_failures(ThumbKind::TransferSp, TRANSFER_SP_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_load_address_halfwords_when_disassembled_then_add_with_pc_or_sp_prints_scaled_offset() {
    assert_eq!(
        table_failures(ThumbKind::LoadAddress, LOAD_ADDRESS_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_sp_adjustments_when_disassembled_then_add_and_sub_print_scaled_offset() {
    assert_eq!(
        table_failures(ThumbKind::AdjustSp, ADJUST_SP_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_push_pop_halfwords_when_disassembled_then_lists_use_ranges_of_three_and_name_lr_pc() {
    assert_eq!(
        table_failures(ThumbKind::PushPop, PUSH_POP_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_block_transfer_halfwords_when_disassembled_then_stmia_ldmia_print_base_writeback_and_list()
{
    assert_eq!(
        table_failures(ThumbKind::BlockTransfer, BLOCK_TRANSFER_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_conditional_branches_when_disassembled_then_every_condition_prints_with_its_target() {
    assert_eq!(
        table_failures(ThumbKind::ConditionalBranch, CONDITIONAL_BRANCH_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_swi_halfwords_when_disassembled_then_number_prints_as_bare_hex() {
    assert_eq!(
        table_failures(ThumbKind::Swi, SWI_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_unconditional_branches_when_disassembled_then_target_uses_the_sign_extended_offset() {
    assert_eq!(
        table_failures(ThumbKind::Branch, BRANCH_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_bl_prefix_halfwords_when_disassembled_then_complete_and_partial_forms_print() {
    assert_eq!(
        table_failures(ThumbKind::LongBranchPrefix, PREFIX_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_bl_suffix_halfwords_when_disassembled_then_bl_suffix_prints_whatever_follows() {
    assert_eq!(
        table_failures(ThumbKind::LongBranchSuffix, SUFFIX_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_arm9_bkpt_halfwords_when_disassembled_then_bkpt_prints_the_number_in_hex() {
    assert_eq!(
        table_failures(ThumbKind::Breakpoint, BREAKPOINT_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_arm9_blx_suffix_halfwords_when_disassembled_then_blx_suffix_prints() {
    assert_eq!(
        table_failures(ThumbKind::LongBranchExchangeSuffix, EXCHANGE_SUFFIX_ROWS),
        Vec::<String>::new()
    );
}

// ---------------------------------------------------------------------------------------------
// The BL pair: three cases
// ---------------------------------------------------------------------------------------------

/// Complete pair: target = address + 4 + (sext(off11) << 12) + (suffix off11 << 1).
const BL_COMPLETE_ROWS: &[Row] = &[
    // both offsets 0
    (0xF000, Some(0xF800), A, false, "bl 0x00001004"),
    // suffix off 1: +2
    (0xF000, Some(0xF801), A, true, "bl 0x00001006"),
    // prefix off 1: +0x1000
    (0xF001, Some(0xF800), A, false, "bl 0x00002004"),
    // suffix off 0x7FF: +0xFFE
    (0xF000, Some(0xFFFF), A, true, "bl 0x00002002"),
    // prefix off 0x7FF = -1: -0x1000
    (0xF7FF, Some(0xF800), A, false, "bl 0x00000004"),
    (0xF7FF, Some(0xFFFF), A, true, "bl 0x00001002"),
    // prefix off 0x400 = -1024: -0x400000
    (0xF400, Some(0xF800), 0x0050_0000, false, "bl 0x00100004"),
    // wraps past the top of the address space
    (0xF000, Some(0xF800), 0xFFFF_FFFE, true, "bl 0x00000002"),
    // wraps below zero
    (0xF7FF, Some(0xF800), 0, false, "bl 0xfffff004"),
];

/// ARM9 prefix + BLX suffix: `blx`, bits 1-0 of the target cleared.
const BLX_COMPLETE_ROWS: &[Row] = &[
    (0xF000, Some(0xE800), A, true, "blx 0x00001004"),
    // suffix off 1: 0x1006 -> 0x1004
    (0xF000, Some(0xE801), A, true, "blx 0x00001004"),
    // suffix off 2: 0x1008
    (0xF000, Some(0xE802), A, true, "blx 0x00001008"),
    // suffix off 3: 0x100A -> 0x1008
    (0xF000, Some(0xE803), A, true, "blx 0x00001008"),
    // address with bit 1 set: 0x1002 + 4 = 0x1006 -> 0x1004
    (0xF000, Some(0xE800), 0x1002, true, "blx 0x00001004"),
    // prefix off -1: -0x1000
    (0xF7FF, Some(0xE800), A, true, "blx 0x00000004"),
    // wraps past the top: 2 -> 0
    (0xF000, Some(0xE800), 0xFFFF_FFFE, true, "blx 0x00000000"),
];

/// A prefix the ARM7 cannot complete: the BLX suffix is `Unhandled` there.
const BLX_ON_ARM7_ROWS: &[Row] = &[
    (0xF000, Some(0xE800), A, false, "(bl prefix lr=0x00001004)"),
    (0xF001, Some(0xE803), A, false, "(bl prefix lr=0x00002004)"),
];

/// A suffix prints the same whatever came before.
const SUFFIX_ALONE_ROWS: &[Row] = &[
    (0xF800, None, A, false, "(bl suffix)"),
    (0xF800, None, A, true, "(bl suffix)"),
    (0xFFFF, None, A, false, "(bl suffix)"),
    (0xFFFF, Some(0xF800), A, true, "(bl suffix)"),
];

/// The BLX suffix: `(blx suffix)` on the ARM9; on the ARM7 it is an `Unhandled` word.
const BLX_SUFFIX_ROWS: &[Row] = &[
    (0xE800, None, A, true, "(blx suffix)"),
    (0xEFFF, None, A, true, "(blx suffix)"),
    (0xE800, None, A, false, "undefined 0xe800"),
    (0xEFFF, None, A, false, "undefined 0xefff"),
];

/// A prefix with no usable next halfword prints LR = address + 4 + (sext(off11) << 12).
const PREFIX_ALONE_ROWS: &[Row] = &[
    (0xF000, None, A, false, "(bl prefix lr=0x00001004)"),
    (0xF000, None, A, true, "(bl prefix lr=0x00001004)"),
    (0xF001, None, A, true, "(bl prefix lr=0x00002004)"),
    // next is not a suffix: a Thumb BX, a format 18 branch, another prefix
    (0xF000, Some(0x4770), A, false, "(bl prefix lr=0x00001004)"),
    (0xF000, Some(0xE000), A, true, "(bl prefix lr=0x00001004)"),
    (0xF000, Some(0xF000), A, true, "(bl prefix lr=0x00001004)"),
    // negative prefix offset: -0x1000
    (0xF7FF, None, A, false, "(bl prefix lr=0x00000004)"),
    // wraps below zero and past the top
    (0xF7FF, None, 0, true, "(bl prefix lr=0xfffff004)"),
    (
        0xF001,
        None,
        0xFFFF_FFFE,
        false,
        "(bl prefix lr=0x00001002)",
    ),
];

#[test]
fn given_a_prefix_followed_by_a_bl_suffix_when_disassembled_then_bl_prints_on_the_prefix_line() {
    assert_eq!(
        table_failures(ThumbKind::LongBranchPrefix, BL_COMPLETE_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_an_arm9_prefix_followed_by_a_blx_suffix_when_disassembled_then_blx_prints_with_bits_1_0_cleared()
 {
    assert_eq!(
        table_failures(ThumbKind::LongBranchPrefix, BLX_COMPLETE_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_an_arm7_prefix_followed_by_a_blx_suffix_when_disassembled_then_the_partial_prefix_prints()
{
    assert_eq!(
        table_failures(ThumbKind::LongBranchPrefix, BLX_ON_ARM7_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_a_bl_suffix_alone_when_disassembled_then_bl_suffix_prints_whatever_follows() {
    assert_eq!(
        table_failures(ThumbKind::LongBranchSuffix, SUFFIX_ALONE_ROWS),
        Vec::<String>::new()
    );
}

#[test]
fn given_a_blx_suffix_when_disassembled_then_arm9_prints_blx_suffix_and_arm7_prints_undefined() {
    let arm9_failures = table_failures(ThumbKind::LongBranchExchangeSuffix, &BLX_SUFFIX_ROWS[..2]);
    let arm7_failures = table_failures(ThumbKind::Unhandled, &BLX_SUFFIX_ROWS[2..]);
    assert_eq!(
        (arm9_failures, arm7_failures),
        (Vec::<String>::new(), Vec::<String>::new())
    );
}

#[test]
fn given_a_prefix_with_no_usable_next_halfword_when_disassembled_then_partial_form_prints_lr() {
    assert_eq!(
        table_failures(ThumbKind::LongBranchPrefix, PREFIX_ALONE_ROWS),
        Vec::<String>::new()
    );
}

// ---------------------------------------------------------------------------------------------
// Boundary cases
// ---------------------------------------------------------------------------------------------

/// Branches at the top and bottom of the address space wrap at 32 bits.
const WRAP_ROWS: &[Row] = &[
    // 0xFFFFFFFE + 4 wraps to 2
    (0xD000, None, 0xFFFF_FFFE, false, "beq 0x00000002"),
    (0xE000, None, 0xFFFF_FFFE, true, "b 0x00000002"),
    // 0xFFFFFFF0 + 4 + 0x20 wraps to 0x14
    (0xD010, None, 0xFFFF_FFF0, false, "beq 0x00000014"),
    // 2 - 4 wraps back to the top
    (0xD0FE, None, 0xFFFF_FFFE, true, "beq 0xfffffffe"),
    // 0 + 4 - 4 stays at zero
    (0xD0FE, None, 0, false, "beq 0x00000000"),
    // 0 + 4 - 256 wraps below zero
    (0xD180, None, 0, true, "bne 0xffffff04"),
    // off = 0x7FC = -4: 4 - 8
    (0xE7FC, None, 0, false, "b 0xfffffffc"),
];

#[test]
fn given_branches_at_the_ends_of_the_address_space_when_disassembled_then_targets_wrap_at_32_bits()
{
    let conditional: Vec<Row> = WRAP_ROWS
        .iter()
        .copied()
        .filter(|row| row.0 >> 12 == 0xD)
        .collect();
    let plain: Vec<Row> = WRAP_ROWS
        .iter()
        .copied()
        .filter(|row| row.0 >> 12 == 0xE)
        .collect();
    assert_eq!(
        (
            table_failures(ThumbKind::ConditionalBranch, &conditional),
            table_failures(ThumbKind::Branch, &plain)
        ),
        (Vec::<String>::new(), Vec::<String>::new())
    );
}

#[test]
fn given_pc_relative_load_at_an_address_with_bit_1_set_when_disassembled_then_pc_reads_word_aligned()
 {
    assert!(pc_load_matches(0, 4, 0x0200_0106, false));
    assert!(pc_load_matches(0, 4, 0x0200_0106, true));
}

const TARGET_ROWS: &[TargetRow] = &[
    // format 16 and 18: Some
    (0xD010, None, A, false, Some(0x1024)),
    (0xDE10, None, A, true, Some(0x1024)),
    (0xE010, None, A, false, Some(0x1024)),
    (0xE7FF, None, A, true, Some(0x1002)),
    // a completed pair, on the prefix line
    (0xF000, Some(0xF800), A, false, Some(0x1004)),
    (0xF000, Some(0xF800), A, true, Some(0x1004)),
    (0xF000, Some(0xE803), A, true, Some(0x1008)),
    // a partial prefix, suffix lines
    (0xF000, None, A, false, None),
    (0xF000, Some(0x4770), A, true, None),
    (0xF000, Some(0xE800), A, false, None),
    (0xF800, None, A, false, None),
    (0xF800, Some(0xF800), A, true, None),
    (0xE800, None, A, true, None),
    (0xE800, None, A, false, None),
    // registers and other instructions
    (0x4770, None, A, false, None),
    (0x4788, None, A, true, None),
    (0xDF00, None, A, false, None),
    (0x1888, None, A, true, None),
    (0x4804, None, A, false, None),
    (0xBD00, None, A, true, None),
    (0x46F7, None, A, false, None),
    (0xB100, None, A, false, None),
    (0xBE00, None, A, true, None),
];

#[test]
fn given_branch_and_non_branch_halfwords_when_disassembled_then_branch_target_is_some_only_for_b_and_complete_bl()
 {
    assert_eq!(target_failures(TARGET_ROWS), Vec::<String>::new());
}

// ---------------------------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------------------------

proptest::proptest! {
    #[test]
    fn given_any_input_when_disassembled_on_arm7_then_mnemonic_class_matches_the_decoded_kind(
        halfword in any::<u16>(),
        next in any::<Option<u16>>(),
        address in any::<u32>(),
    ) {
        prop_assert_eq!(class_failure(halfword, next, address, false), None);
    }

    #[test]
    fn given_any_input_when_disassembled_on_arm9_then_mnemonic_class_matches_the_decoded_kind(
        halfword in any::<u16>(),
        next in any::<Option<u16>>(),
        address in any::<u32>(),
    ) {
        prop_assert_eq!(class_failure(halfword, next, address, true), None);
    }

    #[test]
    fn given_any_halfword_when_disassembled_then_the_first_token_is_the_one_its_bits_name(
        halfword in any::<u16>(),
        address in any::<u32>(),
    ) {
        prop_assert_eq!(exact_token_failure(halfword, address, false), None);
        prop_assert_eq!(exact_token_failure(halfword, address, true), None);
    }

    #[test]
    fn given_any_input_when_disassembled_then_text_is_lowercase_ascii_on_both_cpus(
        halfword in any::<u16>(),
        next in any::<Option<u16>>(),
        address in any::<u32>(),
    ) {
        prop_assert!(text_is_lowercase_ascii(halfword, next, address, false));
        prop_assert!(text_is_lowercase_ascii(halfword, next, address, true));
    }

    #[test]
    fn given_a_halfword_both_cpus_decode_alike_when_disassembled_then_text_is_identical(
        halfword in any::<u16>(),
        next in any::<Option<u16>>(),
        address in any::<u32>(),
    ) {
        prop_assert!(cpu_text_agrees(halfword, next, address));
    }

    #[test]
    fn given_any_halfword_when_disassembled_then_branch_target_follows_the_offset_formula(
        halfword in any::<u16>(),
        address in any::<u32>(),
    ) {
        prop_assert!(branch_target_follows_formula(halfword, address));
    }

    #[test]
    fn given_any_bl_pair_when_disassembled_then_target_sums_both_offsets(
        prefix_off in 0u16..0x800,
        suffix_off in 0u16..0x800,
        address in any::<u32>(),
    ) {
        prop_assert!(complete_pair_matches(prefix_off, suffix_off, address, false));
    }

    #[test]
    fn given_any_blx_pair_when_disassembled_on_arm9_then_target_clears_bits_1_0(
        prefix_off in 0u16..0x800,
        suffix_off in 0u16..0x800,
        address in any::<u32>(),
    ) {
        prop_assert!(complete_pair_matches(prefix_off, suffix_off, address, true));
    }

    #[test]
    fn given_any_pc_relative_load_when_disassembled_then_comment_is_the_aligned_pc_plus_offset(
        rd in 0u16..8,
        word in 1u16..256,
        address in any::<u32>(),
    ) {
        prop_assert!(pc_load_matches(rd, word, address, false));
        prop_assert!(pc_load_matches(rd, word, address, true));
    }
}
