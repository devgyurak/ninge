// SPDX-License-Identifier: Apache-2.0
//! The ARM disassembler. Every expected string is written by hand from `docs/hw/disassembly.md`
//! (the notation) and the encodings in `arm-*.md`, `armv5te.md` and `cp15.md`; none comes from
//! running the implementation. The words are hand-encoded: the comment on a table says which
//! fields it varies.
//!
//! Loops live in the harness functions below; a test body only asserts that the list of
//! failures is empty, so every wrong row shows at once.

use super::decode::{Kind, decode};
use super::disassemble_arm;

/// The address of the instruction under test unless a row says otherwise. Chosen so that the
/// +8 pipeline offset is visible in a branch target and a PC-relative comment.
const BASE: u32 = 0x0200_0100;

/// A row run at `BASE` on both processors: the word and its text.
type Row = (u32, &'static str);

/// Which processors a row applies to.
#[derive(Clone, Copy)]
enum Cpu {
    Both,
    Arm9,
    Arm7,
}

/// A row with its own address and processor: (word, address, cpu, text).
type RowAt = (u32, u32, Cpu, &'static str);

/// A row whose text differs between processors: (word, ARM9 text, ARM7 text).
type PairRow = (u32, &'static str, &'static str);

// ------------------------------------------------------------------------------ harness

fn render(arm9: bool, word: u32, address: u32) -> String {
    if arm9 {
        disassemble_arm::<true>(word, address).to_string()
    } else {
        disassemble_arm::<false>(word, address).to_string()
    }
}

fn kind_of(arm9: bool, word: u32) -> Kind {
    if arm9 {
        decode::<true>(word)
    } else {
        decode::<false>(word)
    }
}

fn target_of(arm9: bool, word: u32, address: u32) -> Option<u32> {
    if arm9 {
        disassemble_arm::<true>(word, address).branch_target()
    } else {
        disassemble_arm::<false>(word, address).branch_target()
    }
}

/// Checks one row on one processor; a wrong text, or a decoder kind other than `kind`, is
/// recorded in `out`.
fn check_row(
    kind: Option<Kind>,
    arm9: bool,
    word: u32,
    address: u32,
    expected: &str,
    out: &mut Vec<String>,
) {
    let got = render(arm9, word, address);
    if got != expected {
        out.push(format!(
            "{word:#010x} arm9={arm9} at {address:#010x}: expected {expected:?}, got {got:?}"
        ));
    }
    if let Some(want) = kind {
        let actual = kind_of(arm9, word);
        if actual != want {
            out.push(format!(
                "{word:#010x} arm9={arm9}: the table expects kind {want:?}, the decoder says {actual:?}"
            ));
        }
    }
}

/// Runs rows that carry their own address and processor. `kind` ties the table to a decoder
/// kind, so a row that drifts into another encoding space is reported.
fn run_at(kind: Option<Kind>, rows: &[RowAt]) -> Vec<String> {
    let mut out = Vec::new();
    for &(word, address, cpu, expected) in rows {
        if matches!(cpu, Cpu::Both | Cpu::Arm9) {
            check_row(kind, true, word, address, expected, &mut out);
        }
        if matches!(cpu, Cpu::Both | Cpu::Arm7) {
            check_row(kind, false, word, address, expected, &mut out);
        }
    }
    out
}

/// Rows at `BASE`, run on the ARM9 and the ARM7.
fn run_both(kind: Kind, rows: &[Row]) -> Vec<String> {
    let mut out = Vec::new();
    for &(word, expected) in rows {
        check_row(Some(kind), true, word, BASE, expected, &mut out);
        check_row(Some(kind), false, word, BASE, expected, &mut out);
    }
    out
}

/// Rows at `BASE`, run on the ARM9 only (ARMv5TE additions).
fn run_arm9(kind: Kind, rows: &[Row]) -> Vec<String> {
    let mut out = Vec::new();
    for &(word, expected) in rows {
        check_row(Some(kind), true, word, BASE, expected, &mut out);
    }
    out
}

/// Rows whose text differs between the processors.
fn run_pairs(rows: &[PairRow]) -> Vec<String> {
    let mut out = Vec::new();
    for &(word, arm9_text, arm7_text) in rows {
        check_row(None, true, word, BASE, arm9_text, &mut out);
        check_row(None, false, word, BASE, arm7_text, &mut out);
    }
    out
}

/// Checks `branch_target()` for rows of (word, address, cpu, expected target).
fn run_targets(rows: &[(u32, u32, Cpu, Option<u32>)]) -> Vec<String> {
    let mut out = Vec::new();
    for &(word, address, cpu, expected) in rows {
        let arm9_run = matches!(cpu, Cpu::Both | Cpu::Arm9);
        let arm7_run = matches!(cpu, Cpu::Both | Cpu::Arm7);
        for (run, arm9) in [(arm9_run, true), (arm7_run, false)] {
            let got = target_of(arm9, word, address);
            if run && got != expected {
                out.push(format!(
                    "{word:#010x} arm9={arm9} at {address:#010x}: expected {expected:x?}, got {got:x?}"
                ));
            }
        }
    }
    out
}

const CONDS: [&str; 16] = [
    "", "eq", "ne", "cs", "cc", "mi", "pl", "vs", "vc", "hi", "ls", "ge", "lt", "gt", "le", "nv",
];

/// `base`, then an optional condition, then one of `suffixes` (the empty string for "no
/// letter").
fn base_cond_suffix(token: &str, bases: &[&str], suffixes: &[&str]) -> bool {
    bases.iter().any(|base| {
        token.strip_prefix(*base).is_some_and(|rest| {
            CONDS.iter().any(|cond| {
                rest.strip_prefix(*cond)
                    .is_some_and(|tail| suffixes.contains(&tail))
            })
        })
    })
}

const DATA_OPS: [&str; 16] = [
    "and", "eor", "sub", "rsb", "add", "adc", "sbc", "rsc", "tst", "teq", "cmp", "cmn", "orr",
    "mov", "bic", "mvn",
];

/// The signed-halfword-multiply mnemonics: the x/y letters come BEFORE the condition.
const HALFWORD_MULTIPLIES: [&str; 16] = [
    "smlabb", "smlabt", "smlatb", "smlatt", "smulbb", "smulbt", "smultb", "smultt", "smlalbb",
    "smlalbt", "smlaltb", "smlaltt", "smlawb", "smlawt", "smulwb", "smulwt",
];

/// Whether the first token of `text` is a mnemonic of `kind` (`docs/hw/disassembly.md`, the
/// table under "ARM"): a base, an optional condition, then one of the kind's letters.
fn mnemonic_matches_kind(kind: Kind, text: &str) -> bool {
    let token = text.split_whitespace().next().unwrap_or("");
    match kind {
        Kind::Unhandled | Kind::Undefined => token == "undefined",
        Kind::DataImmediate | Kind::DataImmediateShift | Kind::DataRegisterShift => {
            base_cond_suffix(token, &DATA_OPS, &["", "s"])
        }
        Kind::Mrs => base_cond_suffix(token, &["mrs"], &[""]),
        Kind::MsrImmediate | Kind::MsrRegister => base_cond_suffix(token, &["msr"], &[""]),
        Kind::Multiply => base_cond_suffix(token, &["mul", "mla"], &["", "s"]),
        Kind::MultiplyLong => {
            base_cond_suffix(token, &["umull", "umlal", "smull", "smlal"], &["", "s"])
        }
        Kind::SingleTransferImmediate | Kind::SingleTransferRegister => {
            base_cond_suffix(token, &["ldr", "str"], &["", "b", "t", "bt"])
        }
        Kind::HalfwordTransfer => base_cond_suffix(token, &["ldr", "str"], &["h", "sb", "sh"]),
        Kind::DoubleTransfer => base_cond_suffix(token, &["ldr", "str"], &["d"]),
        Kind::Swap => base_cond_suffix(token, &["swp"], &["", "b"]),
        Kind::BlockTransfer => base_cond_suffix(token, &["ldm", "stm"], &["ia", "ib", "da", "db"]),
        Kind::Branch => base_cond_suffix(token, &["b", "bl"], &[""]),
        Kind::BranchExchange => base_cond_suffix(token, &["bx"], &[""]),
        Kind::SignedHalfwordMultiply => base_cond_suffix(token, &HALFWORD_MULTIPLIES, &[""]),
        Kind::SaturatingArithmetic => {
            base_cond_suffix(token, &["qadd", "qsub", "qdadd", "qdsub"], &[""])
        }
        Kind::CountLeadingZeros => base_cond_suffix(token, &["clz"], &[""]),
        Kind::BranchLinkExchange => base_cond_suffix(token, &["blx"], &[""]),
        // No condition: the unconditional space has none to write.
        Kind::BranchLinkExchangeImmediate => token == "blx",
        Kind::Breakpoint => base_cond_suffix(token, &["bkpt"], &[""]),
        Kind::Preload => token == "pld",
        Kind::Swi => base_cond_suffix(token, &["swi"], &[""]),
        Kind::Coprocessor => base_cond_suffix(
            token,
            &["cdp", "mcr", "mrc", "ldc", "stc", "mcrr", "mrrc"],
            &["", "l"],
        ),
    }
}

/// Why the text of `word` is wrong for the decoder's kind, if it is.
fn mnemonic_failure<const ARM9: bool>(word: u32, address: u32) -> Option<String> {
    let kind = decode::<ARM9>(word);
    let text = disassemble_arm::<ARM9>(word, address).to_string();
    let undefined_kind = matches!(kind, Kind::Unhandled | Kind::Undefined);
    let expected_undefined = format!("undefined {word:#010x}");
    let class_ok = mnemonic_matches_kind(kind, &text);
    let text_ok = !undefined_kind || text == expected_undefined;
    let no_stray_undefined = undefined_kind || !text.starts_with("undefined");
    if class_ok && text_ok && no_stray_undefined {
        None
    } else {
        Some(format!("{word:#010x} {kind:?}: {text:?}"))
    }
}

/// Why `branch_target()` is wrong for `word`, if it is. The expected value is computed here from
/// the offset field, independently of the implementation.
fn branch_target_failure<const ARM9: bool>(word: u32, address: u32) -> Option<String> {
    let kind = decode::<ARM9>(word);
    let raw = word & 0x00FF_FFFF;
    let signed = if raw & 0x0080_0000 != 0 {
        raw | 0xFF00_0000
    } else {
        raw
    };
    let target = address.wrapping_add(8).wrapping_add(signed << 2);
    let expected = match kind {
        Kind::Branch => Some(target),
        // H, bit 24, adds a halfword (`docs/hw/armv5te.md`, "BLX").
        Kind::BranchLinkExchangeImmediate => Some(target.wrapping_add((word >> 24 & 1) * 2)),
        _ => None,
    };
    let got = disassemble_arm::<ARM9>(word, address).branch_target();
    if got == expected {
        None
    } else {
        Some(format!(
            "{word:#010x} at {address:#010x} {kind:?}: expected {expected:x?}, got {got:x?}"
        ))
    }
}

// ------------------------------------------------------------------ Kind::Unhandled

const UNHANDLED: &[Row] = &[
    // TST/CMP without S and an immediate: the ARMv6 move-immediate space, not executed here.
    (0x0300_0000, "undefined 0x03000000"),
    (0xE300_0000, "undefined 0xe3000000"),
    (0xE340_0000, "undefined 0xe3400000"),
    // UMAAL-shaped (bits 27-20 = 0000 0100, bits 7-4 = 1001): the ARMv6 space stays unhandled.
    (0xE040_0090, "undefined 0xe0400090"),
    (0x0040_0090, "undefined 0x00400090"),
    // bits 27-25 = 000, bit 7 and bit 4 set, but not a multiply, a swap or a halfword form.
    (0xE1A0_0090, "undefined 0xe1a00090"),
    // PSR space with bits 7-4 = 0010 and 0110.
    (0xE100_0020, "undefined 0xe1000020"),
    (0xE100_0060, "undefined 0xe1000060"),
];

const UNHANDLED_ARM9: &[Row] = &[
    // The ARM9's cond = 1111 space is only BLX immediate and PLD
    // (`docs/hw/armv5te.md`, "Unconditional space").
    (0xF3A0_0000, "undefined 0xf3a00000"),
    (0xF000_0000, "undefined 0xf0000000"),
    (0xFF00_0001, "undefined 0xff000001"),
    (0xF120_0070, "undefined 0xf1200070"),
    (0xF7D1_F012, "undefined 0xf7d1f012"),
    (0xF890_0002, "undefined 0xf8900002"),
];

const UNHANDLED_ARM7: &[Row] = &[
    // ARMv5TE words are not executed on the ARM7 (`docs/hw/armv5te.md`, first table).
    (0xE16F_2F11, "undefined 0xe16f2f11"),
    (0xE12F_FF31, "undefined 0xe12fff31"),
    (0xE102_0051, "undefined 0xe1020051"),
    (0xE1C1_00D0, "undefined 0xe1c100d0"),
    (0xE1C1_00F0, "undefined 0xe1c100f0"),
    (0xE120_0070, "undefined 0xe1200070"),
    (0xE100_3281, "undefined 0xe1003281"),
];

#[test]
fn given_unhandled_words_when_disassembled_then_each_prints_undefined_with_the_whole_word() {
    let failures = run_both(Kind::Unhandled, UNHANDLED);
    assert_eq!(failures, Vec::<String>::new());
}

#[test]
fn given_cond_1111_words_on_the_arm9_when_disassembled_then_each_prints_undefined() {
    let failures = run_arm9(Kind::Unhandled, UNHANDLED_ARM9);
    assert_eq!(failures, Vec::<String>::new());
}

#[test]
fn given_armv5te_words_on_the_arm7_when_disassembled_then_each_prints_undefined() {
    let rows: Vec<RowAt> = UNHANDLED_ARM7
        .iter()
        .map(|&(word, text)| (word, BASE, Cpu::Arm7, text))
        .collect();
    let failures = run_at(Some(Kind::Unhandled), &rows);
    assert_eq!(failures, Vec::<String>::new());
}

// ------------------------------------------------------------------ Kind::Undefined

const UNDEFINED: &[Row] = &[
    // bits 27-25 = 011 with bit 4 = 1 (`docs/hw/arm-exceptions-modes.md`, "Undefined instruction").
    (0xE7F0_00F0, "undefined 0xe7f000f0"),
    (0x0780_0010, "undefined 0x07800010"),
    (0xE610_0010, "undefined 0xe6100010"),
    (0xE7FF_FFFF, "undefined 0xe7ffffff"),
];

#[test]
fn given_the_undefined_instruction_space_when_disassembled_then_each_prints_undefined_with_the_whole_word()
 {
    let failures = run_both(Kind::Undefined, UNDEFINED);
    assert_eq!(failures, Vec::<String>::new());
}

#[test]
fn given_cond_1111_undefined_space_when_disassembled_then_both_processors_print_undefined() {
    let rows = [
        (0xF7F0_00F0, BASE, Cpu::Arm7, "undefined 0xf7f000f0"),
        (0xF7D1_F012, BASE, Cpu::Arm7, "undefined 0xf7d1f012"),
    ];
    let failures = run_at(Some(Kind::Undefined), &rows);
    assert_eq!(failures, Vec::<String>::new());
}

// ------------------------------------------------------------------ data processing

const DATA_IMMEDIATE: &[Row] = &[
    // Opcode 0..15, rotate 0, S = 0 for the first ten, implied S for the compares.
    (0xE201_00FF, "and r0, r1, #0xff"),
    (0xE233_2001, "eors r2, r3, #0x1"),
    (0xE241_0004, "sub r0, r1, #0x4"),
    (0xE261_0000, "rsb r0, r1, #0x0"),
    (0xE281_0001, "add r0, r1, #0x1"),
    (0xE2A5_4002, "adc r4, r5, #0x2"),
    (0xE2C0_0003, "sbc r0, r0, #0x3"),
    (0xE2E2_1004, "rsc r1, r2, #0x4"),
    (0xE311_0001, "tst r1, #0x1"),
    (0xE332_0002, "teq r2, #0x2"),
    (0xE353_0003, "cmp r3, #0x3"),
    (0xE374_0004, "cmn r4, #0x4"),
    (0xE381_0008, "orr r0, r1, #0x8"),
    (0xE3A0_0000, "mov r0, #0x0"),
    (0xE3C1_000F, "bic r0, r1, #0xf"),
    (0xE3E0_0000, "mvn r0, #0x0"),
    // S suffix after the condition.
    (0xE3B0_0001, "movs r0, #0x1"),
    (0xE291_0001, "adds r0, r1, #0x1"),
    (0x0291_0001, "addeqs r0, r1, #0x1"),
    (0x13B0_0001, "movnes r0, #0x1"),
    // The compares have no s and no Rd.
    (0x1350_0001, "cmpne r0, #0x1"),
    // Rotated immediates are shown after the rotation.
    (0xE3A0_04FF, "mov r0, #0xff000000"),
    (0xE3A0_0103, "mov r0, #0xc0000000"),
    (0xE3A0_0F0F, "mov r0, #0x3c"),
    (0xE3A0_0AFF, "mov r0, #0xff000"),
    // Register names r13 and r14.
    (0xE3A0_D001, "mov sp, #0x1"),
    (0xE3A0_E000, "mov lr, #0x0"),
    // One word per condition, mov r0, #1.
    (0x03A0_0001, "moveq r0, #0x1"),
    (0x13A0_0001, "movne r0, #0x1"),
    (0x23A0_0001, "movcs r0, #0x1"),
    (0x33A0_0001, "movcc r0, #0x1"),
    (0x43A0_0001, "movmi r0, #0x1"),
    (0x53A0_0001, "movpl r0, #0x1"),
    (0x63A0_0001, "movvs r0, #0x1"),
    (0x73A0_0001, "movvc r0, #0x1"),
    (0x83A0_0001, "movhi r0, #0x1"),
    (0x93A0_0001, "movls r0, #0x1"),
    (0xA3A0_0001, "movge r0, #0x1"),
    (0xB3A0_0001, "movlt r0, #0x1"),
    (0xC3A0_0001, "movgt r0, #0x1"),
    (0xD3A0_0001, "movle r0, #0x1"),
    (0xE3A0_0001, "mov r0, #0x1"),
];

#[test]
fn given_immediate_data_processing_words_when_disassembled_then_each_prints_its_text() {
    let failures = run_both(Kind::DataImmediate, DATA_IMMEDIATE);
    assert_eq!(failures, Vec::<String>::new());
}

#[test]
fn given_an_arm7_immediate_word_with_cond_1111_when_disassembled_then_the_condition_reads_nv() {
    let rows = [(0xF3A0_0001, BASE, Cpu::Arm7, "movnv r0, #0x1")];
    let failures = run_at(Some(Kind::DataImmediate), &rows);
    assert_eq!(failures, Vec::<String>::new());
}

const DATA_IMMEDIATE_SHIFT: &[Row] = &[
    (0xE1A0_0001, "mov r0, r1"),
    (0xE1B0_0001, "movs r0, r1"),
    (0xE1A0_0000, "mov r0, r0"),
    // Shift types with a non-zero amount.
    (0xE1A0_0101, "mov r0, r1, lsl #2"),
    (0xE1A0_0F81, "mov r0, r1, lsl #31"),
    (0xE1A0_00A1, "mov r0, r1, lsr #1"),
    (0xE1A0_0FA1, "mov r0, r1, lsr #31"),
    (0xE1A0_0241, "mov r0, r1, asr #4"),
    (0xE1A0_01E1, "mov r0, r1, ror #3"),
    // #0: lsr and asr read as #32, ror as rrx, lsl writes nothing
    // (`docs/hw/arm-barrel-shifter.md`).
    (0xE1A0_0021, "mov r0, r1, lsr #32"),
    (0xE1A0_0041, "mov r0, r1, asr #32"),
    (0xE1A0_0061, "mov r0, r1, rrx"),
    (0xE081_0002, "add r0, r1, r2"),
    (0xE081_0102, "add r0, r1, r2, lsl #2"),
    (0xE081_0062, "add r0, r1, r2, rrx"),
    // Every opcode with three register operands.
    (0xE001_0002, "and r0, r1, r2"),
    (0xE021_0002, "eor r0, r1, r2"),
    (0xE041_0002, "sub r0, r1, r2"),
    (0xE061_0002, "rsb r0, r1, r2"),
    (0xE0A1_0002, "adc r0, r1, r2"),
    (0xE0C1_0002, "sbc r0, r1, r2"),
    (0xE0E1_0002, "rsc r0, r1, r2"),
    (0xE111_0002, "tst r1, r2"),
    (0xE131_0002, "teq r1, r2"),
    (0xE151_0002, "cmp r1, r2"),
    (0xE171_0002, "cmn r1, r2"),
    (0xE181_0002, "orr r0, r1, r2"),
    (0xE1A0_0002, "mov r0, r2"),
    (0xE1C1_0002, "bic r0, r1, r2"),
    (0xE1E0_0002, "mvn r0, r2"),
    (0xE110_0081, "tst r0, r1, lsl #1"),
    // S and condition.
    (0xE091_0002, "adds r0, r1, r2"),
    (0x0091_0002, "addeqs r0, r1, r2"),
    (0x11A0_0101, "movne r0, r1, lsl #2"),
    // Destination or source pc, sp, lr.
    (0xE1A0_F00E, "mov pc, lr"),
    (0xE1B0_F00E, "movs pc, lr"),
    (0xE1A0_D00C, "mov sp, r12"),
    (0xE1A0_C00D, "mov r12, sp"),
    (0xE1A0_E00F, "mov lr, pc"),
];

#[test]
fn given_register_data_processing_words_when_disassembled_then_each_prints_its_text() {
    let failures = run_both(Kind::DataImmediateShift, DATA_IMMEDIATE_SHIFT);
    assert_eq!(failures, Vec::<String>::new());
}

const DATA_REGISTER_SHIFT: &[Row] = &[
    (0xE1A0_0211, "mov r0, r1, lsl r2"),
    (0xE1A0_0231, "mov r0, r1, lsr r2"),
    (0xE1A0_0251, "mov r0, r1, asr r2"),
    (0xE1A0_0271, "mov r0, r1, ror r2"),
    (0xE1B0_0211, "movs r0, r1, lsl r2"),
    (0xE081_0312, "add r0, r1, r2, lsl r3"),
    (0xE1C1_0312, "bic r0, r1, r2, lsl r3"),
    (0xE150_0251, "cmp r0, r1, asr r2"),
    (0xE110_0251, "tst r0, r1, asr r2"),
    (0x0081_0312, "addeq r0, r1, r2, lsl r3"),
    (0x0091_0312, "addeqs r0, r1, r2, lsl r3"),
];

#[test]
fn given_register_shifted_register_words_when_disassembled_then_each_prints_its_text() {
    let failures = run_both(Kind::DataRegisterShift, DATA_REGISTER_SHIFT);
    assert_eq!(failures, Vec::<String>::new());
}

// ------------------------------------------------------------------ PSR transfer

const MRS: &[Row] = &[
    (0xE10F_0000, "mrs r0, cpsr"),
    (0xE14F_0000, "mrs r0, spsr"),
    (0x010F_1000, "mrseq r1, cpsr"),
    (0xE14F_E000, "mrs lr, spsr"),
    (0xE10F_C000, "mrs r12, cpsr"),
];

#[test]
fn given_mrs_words_when_disassembled_then_each_names_the_psr() {
    let failures = run_both(Kind::Mrs, MRS);
    assert_eq!(failures, Vec::<String>::new());
}

const MSR_REGISTER: &[Row] = &[
    (0xE129_F000, "msr cpsr_fc, r0"),
    (0xE12F_F001, "msr cpsr_fsxc, r1"),
    // Field mask 0 writes no letter (`docs/hw/disassembly.md`, "MSR").
    (0xE120_F000, "msr cpsr_, r0"),
    (0xE160_F000, "msr spsr_, r0"),
    (0xE168_F002, "msr spsr_f, r2"),
    (0xE161_F003, "msr spsr_c, r3"),
    (0xE16F_F004, "msr spsr_fsxc, r4"),
    // Letters in the order f s x c.
    (0xE126_F000, "msr cpsr_sx, r0"),
    (0xE124_F000, "msr cpsr_s, r0"),
    (0xE122_F00E, "msr cpsr_x, lr"),
    (0x1121_F001, "msrne cpsr_c, r1"),
];

#[test]
fn given_msr_register_words_when_disassembled_then_the_field_letters_follow_the_mask() {
    let failures = run_both(Kind::MsrRegister, MSR_REGISTER);
    assert_eq!(failures, Vec::<String>::new());
}

const MSR_IMMEDIATE: &[Row] = &[
    (0xE368_F20F, "msr spsr_f, #0xf0000000"),
    (0xE328_F20F, "msr cpsr_f, #0xf0000000"),
    (0xE321_F0D3, "msr cpsr_c, #0xd3"),
    (0xE329_F001, "msr cpsr_fc, #0x1"),
    (0xE320_F000, "msr cpsr_, #0x0"),
    (0x1328_F20F, "msrne cpsr_f, #0xf0000000"),
];

#[test]
fn given_msr_immediate_words_when_disassembled_then_the_value_is_shown_after_rotation() {
    let failures = run_both(Kind::MsrImmediate, MSR_IMMEDIATE);
    assert_eq!(failures, Vec::<String>::new());
}

// ------------------------------------------------------------------ multiplies and swap

const MULTIPLY: &[Row] = &[
    // mul rd, rm, rs and mla rd, rm, rs, rn.
    (0xE000_0291, "mul r0, r1, r2"),
    (0xE013_0594, "muls r3, r4, r5"),
    (0xE020_3291, "mla r0, r1, r2, r3"),
    (0xE030_3291, "mlas r0, r1, r2, r3"),
    (0x0000_0291, "muleq r0, r1, r2"),
    (0x0030_3291, "mlaeqs r0, r1, r2, r3"),
    (0x1010_0291, "mulnes r0, r1, r2"),
    (0xE00C_0E9D, "mul r12, sp, lr"),
];

#[test]
fn given_mul_and_mla_words_when_disassembled_then_operands_follow_rd_rm_rs_rn() {
    let failures = run_both(Kind::Multiply, MULTIPLY);
    assert_eq!(failures, Vec::<String>::new());
}

const MULTIPLY_LONG: &[Row] = &[
    // rdlo, rdhi, rm, rs.
    (0xE081_0392, "umull r0, r1, r2, r3"),
    (0xE0A1_0392, "umlal r0, r1, r2, r3"),
    (0xE0C1_0392, "smull r0, r1, r2, r3"),
    (0xE0E1_0392, "smlal r0, r1, r2, r3"),
    (0xE091_0392, "umulls r0, r1, r2, r3"),
    (0xE0B1_0392, "umlals r0, r1, r2, r3"),
    (0xE0D1_0392, "smulls r0, r1, r2, r3"),
    (0xE0F1_0392, "smlals r0, r1, r2, r3"),
    (0x00A1_0392, "umlaleq r0, r1, r2, r3"),
    (0x10F1_0392, "smlalnes r0, r1, r2, r3"),
    (0xE087_6598, "umull r6, r7, r8, r5"),
];

#[test]
fn given_long_multiply_words_when_disassembled_then_rdlo_comes_before_rdhi() {
    let failures = run_both(Kind::MultiplyLong, MULTIPLY_LONG);
    assert_eq!(failures, Vec::<String>::new());
}

const SWAP: &[Row] = &[
    (0xE102_0091, "swp r0, r1, [r2]"),
    (0xE142_0091, "swpb r0, r1, [r2]"),
    (0x0145_3094, "swpeqb r3, r4, [r5]"),
    (0xE10D_0091, "swp r0, r1, [sp]"),
];

#[test]
fn given_swap_words_when_disassembled_then_the_b_letter_follows_the_condition() {
    let failures = run_both(Kind::Swap, SWAP);
    assert_eq!(failures, Vec::<String>::new());
}

// ------------------------------------------------------------------ single transfers

const SINGLE_TRANSFER_IMMEDIATE: &[Row] = &[
    (0xE591_0000, "ldr r0, [r1]"),
    (0xE591_0004, "ldr r0, [r1, #0x4]"),
    (0xE511_0004, "ldr r0, [r1, #-0x4]"),
    (0xE5B1_0004, "ldr r0, [r1, #0x4]!"),
    (0xE491_0004, "ldr r0, [r1], #0x4"),
    (0xE411_0004, "ldr r0, [r1], #-0x4"),
    (0xE591_0FFF, "ldr r0, [r1, #0xfff]"),
    // A zero offset with writeback keeps the offset: `[rn, #0x0]!`.
    (0xE5B1_0000, "ldr r0, [r1, #0x0]!"),
    // Offset -0 is written #-0x0.
    (0xE501_0000, "str r0, [r1, #-0x0]"),
    (0xE411_0000, "ldr r0, [r1], #-0x0"),
    // b and t letters; P = 0 with W = 1 is the T form.
    (0xE5D1_0000, "ldrb r0, [r1]"),
    (0xE4B1_0004, "ldrt r0, [r1], #0x4"),
    (0xE4F1_0004, "ldrbt r0, [r1], #0x4"),
    (0xE581_0000, "str r0, [r1]"),
    (0xE5C1_0000, "strb r0, [r1]"),
    (0xE4A1_0004, "strt r0, [r1], #0x4"),
    (0xE4E1_0004, "strbt r0, [r1], #0x4"),
    // Stack forms.
    (0xE52D_0004, "str r0, [sp, #-0x4]!"),
    (0xE49D_F004, "ldr pc, [sp], #0x4"),
    // Condition before the letters.
    (0x0591_0000, "ldreq r0, [r1]"),
    (0x05D1_0000, "ldreqb r0, [r1]"),
    (0x14F1_0004, "ldrnebt r0, [r1], #0x4"),
];

#[test]
fn given_immediate_offset_transfers_when_disassembled_then_each_prints_its_addressing() {
    let failures = run_both(Kind::SingleTransferImmediate, SINGLE_TRANSFER_IMMEDIATE);
    assert_eq!(failures, Vec::<String>::new());
}

const SINGLE_TRANSFER_REGISTER: &[Row] = &[
    (0xE791_0002, "ldr r0, [r1, r2]"),
    (0xE711_0002, "ldr r0, [r1, -r2]"),
    (0xE791_0102, "ldr r0, [r1, r2, lsl #2]"),
    (0xE7B1_0002, "ldr r0, [r1, r2]!"),
    (0xE691_0002, "ldr r0, [r1], r2"),
    (0xE611_0182, "ldr r0, [r1], -r2, lsl #3"),
    (0xE6F1_0002, "ldrbt r0, [r1], r2"),
    (0xE7C1_0002, "strb r0, [r1, r2]"),
    (0xE721_0242, "str r0, [r1, -r2, asr #4]!"),
    (0x0791_0002, "ldreq r0, [r1, r2]"),
    // #0 shifts keep their special meaning in the offset register too.
    (0xE791_0022, "ldr r0, [r1, r2, lsr #32]"),
    (0xE791_0042, "ldr r0, [r1, r2, asr #32]"),
    (0xE791_0062, "ldr r0, [r1, r2, rrx]"),
    (0xE791_00E2, "ldr r0, [r1, r2, ror #1]"),
    (0xE711_0022, "ldr r0, [r1, -r2, lsr #32]"),
    (0xE691_0062, "ldr r0, [r1], r2, rrx"),
    // A pc base with a register offset gets no comment.
    (0xE79F_0001, "ldr r0, [pc, r1]"),
];

#[test]
fn given_register_offset_transfers_when_disassembled_then_each_prints_its_addressing() {
    let failures = run_both(Kind::SingleTransferRegister, SINGLE_TRANSFER_REGISTER);
    assert_eq!(failures, Vec::<String>::new());
}

const HALFWORD_TRANSFER: &[Row] = &[
    (0xE1D1_00B0, "ldrh r0, [r1]"),
    (0xE1D1_00B2, "ldrh r0, [r1, #0x2]"),
    (0xE1D1_01B2, "ldrh r0, [r1, #0x12]"),
    (0xE1D1_0FBF, "ldrh r0, [r1, #0xff]"),
    (0xE151_00B2, "ldrh r0, [r1, #-0x2]"),
    (0xE1C1_00B0, "strh r0, [r1]"),
    (0xE1CD_00B0, "strh r0, [sp]"),
    (0xE1D1_00D0, "ldrsb r0, [r1]"),
    (0xE1D1_00F0, "ldrsh r0, [r1]"),
    // The condition goes before the letters.
    (0x01D1_00B0, "ldreqh r0, [r1]"),
    (0x01D1_00D0, "ldreqsb r0, [r1]"),
    (0x11D1_00F0, "ldrnesh r0, [r1]"),
    (0x01C1_00B0, "streqh r0, [r1]"),
    // Register offsets.
    (0xE191_00B2, "ldrh r0, [r1, r2]"),
    (0xE111_00B2, "ldrh r0, [r1, -r2]"),
    (0xE111_00D2, "ldrsb r0, [r1, -r2]"),
    // Writeback and post-indexing.
    (0xE1F1_00B2, "ldrh r0, [r1, #0x2]!"),
    (0xE1E1_00B2, "strh r0, [r1, #0x2]!"),
    (0xE1F1_00B0, "ldrh r0, [r1, #0x0]!"),
    (0xE0D1_00B2, "ldrh r0, [r1], #0x2"),
    (0xE051_00B2, "ldrh r0, [r1], #-0x2"),
    (0xE091_00B2, "ldrh r0, [r1], r2"),
    (0xE011_00B2, "ldrh r0, [r1], -r2"),
    // P = 0 with W = 1 has no T form: written as an ordinary post-indexed access.
    (0xE0F1_00B2, "ldrh r0, [r1], #0x2"),
];

#[test]
fn given_halfword_transfers_when_disassembled_then_the_letters_follow_the_condition() {
    let failures = run_both(Kind::HalfwordTransfer, HALFWORD_TRANSFER);
    assert_eq!(failures, Vec::<String>::new());
}

const DOUBLE_TRANSFER: &[Row] = &[
    (0xE1C1_00D0, "ldrd r0, [r1]"),
    (0xE1C1_00F0, "strd r0, [r1]"),
    (0xE1C1_20D8, "ldrd r2, [r1, #0x8]"),
    (0xE16D_20F8, "strd r2, [sp, #-0x8]!"),
    (0xE181_00D2, "ldrd r0, [r1, r2]"),
    (0xE0C1_40F8, "strd r4, [r1], #0x8"),
    (0x01C1_00D0, "ldreqd r0, [r1]"),
    (0x11C1_00F0, "strned r0, [r1]"),
];

#[test]
fn given_doubleword_transfers_on_the_arm9_when_disassembled_then_each_prints_ldrd_or_strd() {
    let failures = run_arm9(Kind::DoubleTransfer, DOUBLE_TRANSFER);
    assert_eq!(failures, Vec::<String>::new());
}

// ------------------------------------------------------------------ block transfers

const BLOCK_MODES: &[Row] = &[
    (0xE890_0002, "ldmia r0, {r1}"),
    (0xE990_0002, "ldmib r0, {r1}"),
    (0xE810_0002, "ldmda r0, {r1}"),
    (0xE910_0002, "ldmdb r0, {r1}"),
    (0xE880_0002, "stmia r0, {r1}"),
    (0xE980_0002, "stmib r0, {r1}"),
    (0xE800_0002, "stmda r0, {r1}"),
    (0xE900_0002, "stmdb r0, {r1}"),
    // W writes `!` after the base, S writes `^` after the list.
    (0xE92D_4FF0, "stmdb sp!, {r4-r11, lr}"),
    (0xE8BD_8FF0, "ldmia sp!, {r4-r11, pc}"),
    (0xE8F0_800E, "ldmia r0!, {r1-r3, pc}^"),
    (0xE8C0_0002, "stmia r0, {r1}^"),
    (0xE8FD_800F, "ldmia sp!, {r0-r3, pc}^"),
    // The condition goes before the mode.
    (0x0890_0002, "ldmeqia r0, {r1}"),
];

#[test]
fn given_block_transfers_when_disassembled_then_mode_writeback_and_caret_are_shown() {
    let failures = run_both(Kind::BlockTransfer, BLOCK_MODES);
    assert_eq!(failures, Vec::<String>::new());
}

const BLOCK_LISTS: &[Row] = &[
    // A run of three or more is a range; one or two stay single.
    (0xE885_400F, "stmia r5, {r0-r3, lr}"),
    (0xE885_0003, "stmia r5, {r0, r1}"),
    (0xE885_0007, "stmia r5, {r0-r2}"),
    (0xE885_0001, "stmia r5, {r0}"),
    (0xE885_0015, "stmia r5, {r0, r2, r4}"),
    (0xE885_0037, "stmia r5, {r0-r2, r4, r5}"),
    (0xE880_6000, "stmia r0, {sp, lr}"),
    // An empty list.
    (0xE885_0000, "stmia r5, {}"),
    // Ranges that end in the named registers.
    (0xE880_7FF0, "stmia r0, {r4-lr}"),
    (0xE880_F800, "stmia r0, {r11-pc}"),
    (0xE880_FFFF, "stmia r0, {r0-pc}"),
];

#[test]
fn given_register_lists_when_disassembled_then_runs_of_three_become_ranges() {
    let failures = run_both(Kind::BlockTransfer, BLOCK_LISTS);
    assert_eq!(failures, Vec::<String>::new());
}

// ------------------------------------------------------------------ branches

const BRANCH: &[RowAt] = &[
    // Target = address + 8 + offset24 * 4 (`docs/hw/arm-branch-pipeline.md`).
    (0xEA00_0002, BASE, Cpu::Both, "b 0x02000110"),
    (0xEA00_0000, BASE, Cpu::Both, "b 0x02000108"),
    (0xEAFF_FFFE, BASE, Cpu::Both, "b 0x02000100"),
    (0xEB00_0002, BASE, Cpu::Both, "bl 0x02000110"),
    (0x0A00_0002, BASE, Cpu::Both, "beq 0x02000110"),
    (0x1A00_0002, BASE, Cpu::Both, "bne 0x02000110"),
    (0x2A00_0002, BASE, Cpu::Both, "bcs 0x02000110"),
    (0x3A00_0002, BASE, Cpu::Both, "bcc 0x02000110"),
    (0x4A00_0002, BASE, Cpu::Both, "bmi 0x02000110"),
    (0x5A00_0002, BASE, Cpu::Both, "bpl 0x02000110"),
    (0x6A00_0002, BASE, Cpu::Both, "bvs 0x02000110"),
    (0x7A00_0002, BASE, Cpu::Both, "bvc 0x02000110"),
    (0x8A00_0002, BASE, Cpu::Both, "bhi 0x02000110"),
    (0x9A00_0002, BASE, Cpu::Both, "bls 0x02000110"),
    (0xAA00_0002, BASE, Cpu::Both, "bge 0x02000110"),
    (0xBA00_0002, BASE, Cpu::Both, "blt 0x02000110"),
    (0xCA00_0002, BASE, Cpu::Both, "bgt 0x02000110"),
    (0xDA00_0002, BASE, Cpu::Both, "ble 0x02000110"),
    (0x1B00_0002, BASE, Cpu::Both, "blne 0x02000110"),
    (0x9B00_0002, BASE, Cpu::Both, "blls 0x02000110"),
    // Wrapping at both ends of the address space.
    (0xEA00_0000, 0xFFFF_FFF8, Cpu::Both, "b 0x00000000"),
    (0xEA00_0002, 0xFFFF_FFF8, Cpu::Both, "b 0x00000008"),
    (0xEB00_0000, 0xFFFF_FFFC, Cpu::Both, "bl 0x00000004"),
    (0xEAFF_FFFD, 0x0000_0000, Cpu::Both, "b 0xfffffffc"),
    // Largest forward and backward offsets.
    (0xEA7F_FFFF, 0x0000_0000, Cpu::Both, "b 0x02000004"),
    (0xEA80_0000, 0x0200_0000, Cpu::Both, "b 0x00000008"),
    // On the ARM7 the 1111 condition is the ordinary table with the condition written nv.
    (0xFA00_0002, BASE, Cpu::Arm7, "bnv 0x02000110"),
    (0xFB00_0002, BASE, Cpu::Arm7, "blnv 0x02000110"),
];

#[test]
fn given_branch_words_when_disassembled_then_the_target_includes_the_pipeline_offset() {
    let failures = run_at(Some(Kind::Branch), BRANCH);
    assert_eq!(failures, Vec::<String>::new());
}

const BRANCH_EXCHANGE: &[Row] = &[
    (0xE12F_FF11, "bx r1"),
    (0xE12F_FF1E, "bx lr"),
    (0xE12F_FF1F, "bx pc"),
    (0x012F_FF1E, "bxeq lr"),
    (0x112F_FF10, "bxne r0"),
];

#[test]
fn given_bx_words_when_disassembled_then_the_register_is_named() {
    let failures = run_both(Kind::BranchExchange, BRANCH_EXCHANGE);
    assert_eq!(failures, Vec::<String>::new());
}

#[test]
fn given_an_arm7_bx_with_cond_1111_when_disassembled_then_the_condition_reads_nv() {
    let rows = [(0xF12F_FF11, BASE, Cpu::Arm7, "bxnv r1")];
    let failures = run_at(Some(Kind::BranchExchange), &rows);
    assert_eq!(failures, Vec::<String>::new());
}

const BRANCH_LINK_EXCHANGE: &[Row] = &[
    (0xE12F_FF31, "blx r1"),
    (0xE12F_FF3E, "blx lr"),
    (0x112F_FF32, "blxne r2"),
];

#[test]
fn given_blx_register_words_on_the_arm9_when_disassembled_then_each_prints_blx_rm() {
    let failures = run_arm9(Kind::BranchLinkExchange, BRANCH_LINK_EXCHANGE);
    assert_eq!(failures, Vec::<String>::new());
}

const BRANCH_LINK_EXCHANGE_IMMEDIATE: &[RowAt] = &[
    // H = bit 24 adds 2 to the target; no condition is written.
    (0xFA00_0002, BASE, Cpu::Arm9, "blx 0x02000110"),
    (0xFB00_0002, BASE, Cpu::Arm9, "blx 0x02000112"),
    (0xFA00_0000, BASE, Cpu::Arm9, "blx 0x02000108"),
    (0xFAFF_FFFE, BASE, Cpu::Arm9, "blx 0x02000100"),
    (0xFBFF_FFFE, BASE, Cpu::Arm9, "blx 0x02000102"),
    // Wrapping, with and without H.
    (0xFB00_0000, 0xFFFF_FFF8, Cpu::Arm9, "blx 0x00000002"),
    (0xFAFF_FFFD, 0x0000_0000, Cpu::Arm9, "blx 0xfffffffc"),
    (0xFBFF_FFFD, 0x0000_0000, Cpu::Arm9, "blx 0xfffffffe"),
];

#[test]
fn given_blx_immediate_words_on_the_arm9_when_disassembled_then_the_h_bit_adds_two() {
    let failures = run_at(
        Some(Kind::BranchLinkExchangeImmediate),
        BRANCH_LINK_EXCHANGE_IMMEDIATE,
    );
    assert_eq!(failures, Vec::<String>::new());
}

// ------------------------------------------------------------------ ARMv5TE arithmetic

const COUNT_LEADING_ZEROS: &[Row] = &[
    (0xE16F_2F11, "clz r2, r1"),
    (0x116F_3F14, "clzne r3, r4"),
    (0xE16F_0F1D, "clz r0, sp"),
];

#[test]
fn given_clz_words_on_the_arm9_when_disassembled_then_each_prints_clz_rd_rm() {
    let failures = run_arm9(Kind::CountLeadingZeros, COUNT_LEADING_ZEROS);
    assert_eq!(failures, Vec::<String>::new());
}

const SATURATING: &[Row] = &[
    // qadd rd, rm, rn: Rd bits 15-12, Rm bits 3-0, Rn bits 19-16.
    (0xE102_0051, "qadd r0, r1, r2"),
    (0xE122_0051, "qsub r0, r1, r2"),
    (0xE142_0051, "qdadd r0, r1, r2"),
    (0xE162_0051, "qdsub r0, r1, r2"),
    (0x0102_0051, "qaddeq r0, r1, r2"),
    (0x1165_3054, "qdsubne r3, r4, r5"),
];

#[test]
fn given_saturating_words_on_the_arm9_when_disassembled_then_rn_comes_last() {
    let failures = run_arm9(Kind::SaturatingArithmetic, SATURATING);
    assert_eq!(failures, Vec::<String>::new());
}

const SIGNED_HALFWORD_MULTIPLY: &[Row] = &[
    // x is bit 5 (Rm half), y is bit 6 (Rs half); the letters read x then y.
    (0xE100_3281, "smlabb r0, r1, r2, r3"),
    (0xE100_32C1, "smlabt r0, r1, r2, r3"),
    (0xE100_32A1, "smlatb r0, r1, r2, r3"),
    (0xE100_32E1, "smlatt r0, r1, r2, r3"),
    (0xE160_0281, "smulbb r0, r1, r2"),
    (0xE160_02C1, "smulbt r0, r1, r2"),
    (0xE160_02A1, "smultb r0, r1, r2"),
    (0xE160_02E1, "smultt r0, r1, r2"),
    // rdlo, rdhi, rm, rs.
    (0xE141_0382, "smlalbb r0, r1, r2, r3"),
    (0xE141_03C2, "smlalbt r0, r1, r2, r3"),
    (0xE141_03A2, "smlaltb r0, r1, r2, r3"),
    (0xE141_03E2, "smlaltt r0, r1, r2, r3"),
    (0xE120_3281, "smlawb r0, r1, r2, r3"),
    (0xE120_32C1, "smlawt r0, r1, r2, r3"),
    (0xE120_02A1, "smulwb r0, r1, r2"),
    (0xE120_02E1, "smulwt r0, r1, r2"),
    // The condition goes after the x/y letters.
    (0x0100_3281, "smlabbeq r0, r1, r2, r3"),
    (0x1120_02E1, "smulwtne r0, r1, r2"),
    (0x1141_03E2, "smlalttne r0, r1, r2, r3"),
    (0xC160_02E1, "smulttgt r0, r1, r2"),
];

#[test]
fn given_signed_halfword_multiplies_on_the_arm9_when_disassembled_then_xy_letters_precede_the_condition()
 {
    let failures = run_arm9(Kind::SignedHalfwordMultiply, SIGNED_HALFWORD_MULTIPLY);
    assert_eq!(failures, Vec::<String>::new());
}

// ------------------------------------------------------------------ BKPT, PLD, SWI

const BREAKPOINT: &[Row] = &[
    // imm12 in bits 19-8, imm4 in bits 3-0.
    (0xE120_0070, "bkpt 0x0"),
    (0xE120_0071, "bkpt 0x1"),
    (0xE120_0170, "bkpt 0x10"),
    (0xE121_2374, "bkpt 0x1234"),
    (0xE12F_FF7F, "bkpt 0xffff"),
    // A condition other than AL is printed (`docs/hw/disassembly.md`, "Unverified").
    (0x0120_0070, "bkpteq 0x0"),
    (0x1121_2374, "bkptne 0x1234"),
];

#[test]
fn given_bkpt_words_on_the_arm9_when_disassembled_then_the_immediate_is_bare_hex() {
    let failures = run_arm9(Kind::Breakpoint, BREAKPOINT);
    assert_eq!(failures, Vec::<String>::new());
}

const PRELOAD: &[Row] = &[
    (0xF5D1_F020, "pld [r1, #0x20]"),
    (0xF551_F020, "pld [r1, #-0x20]"),
    (0xF5D1_F000, "pld [r1]"),
    (0xF5DD_F004, "pld [sp, #0x4]"),
    (0xF7D1_F002, "pld [r1, r2]"),
    (0xF751_F102, "pld [r1, -r2, lsl #2]"),
    (0xF7D1_F182, "pld [r1, r2, lsl #3]"),
    (0xF7D1_F0E2, "pld [r1, r2, ror #1]"),
];

#[test]
fn given_pld_words_on_the_arm9_when_disassembled_then_no_condition_is_written() {
    let failures = run_arm9(Kind::Preload, PRELOAD);
    assert_eq!(failures, Vec::<String>::new());
}

const SWI: &[Row] = &[
    (0xEF00_0001, "swi 0x1"),
    (0xEF00_0000, "swi 0x0"),
    (0xEFFF_FFFF, "swi 0xffffff"),
    (0xEF12_3456, "swi 0x123456"),
    (0x0F00_0000, "swieq 0x0"),
    (0x1F00_0005, "swine 0x5"),
];

#[test]
fn given_swi_words_when_disassembled_then_the_comment_field_is_bare_hex() {
    let failures = run_both(Kind::Swi, SWI);
    assert_eq!(failures, Vec::<String>::new());
}

// ------------------------------------------------------------------ coprocessor

const CDP: &[Row] = &[
    // cdp pN, opcode1, crd, crn, crm, opcode2.
    (0xEE13_2FA4, "cdp p15, 1, c2, c3, c4, 5"),
    (0xEE00_0000, "cdp p0, 0, c0, c0, c0, 0"),
    (0xEEFF_F1EF, "cdp p1, 15, c15, c15, c15, 7"),
    (0x0E13_2FA4, "cdpeq p15, 1, c2, c3, c4, 5"),
];

#[test]
fn given_cdp_words_when_disassembled_then_fields_are_decimal_in_assembler_order() {
    let failures = run_both(Kind::Coprocessor, CDP);
    assert_eq!(failures, Vec::<String>::new());
}

const MCR: &[Row] = &[
    (0xEE01_0F10, "mcr p15, 0, r0, c1, c0, 0"),
    (0xEE07_3F9A, "mcr p15, 0, r3, c7, c10, 4"),
    (0x0E43_1ED4, "mcreq p14, 2, r1, c3, c4, 6"),
    (0xEEEF_EFFF, "mcr p15, 7, lr, c15, c15, 7"),
];

#[test]
fn given_mcr_words_when_disassembled_then_the_core_register_follows_the_opcode() {
    let failures = run_both(Kind::Coprocessor, MCR);
    assert_eq!(failures, Vec::<String>::new());
}

const MRC: &[Row] = &[
    (0xEE11_0F10, "mrc p15, 0, r0, c1, c0, 0"),
    (0xEE3D_2FF1, "mrc p15, 1, r2, c13, c1, 7"),
    (0xEE10_FF10, "mrc p15, 0, pc, c0, c0, 0"),
    (0x1E11_0F10, "mrcne p15, 0, r0, c1, c0, 0"),
];

#[test]
fn given_mrc_words_when_disassembled_then_pc_as_the_core_register_is_named_pc() {
    let failures = run_both(Kind::Coprocessor, MRC);
    assert_eq!(failures, Vec::<String>::new());
}

const LDC: &[Row] = &[
    // The offset is the word offset times four.
    (0xED92_1F00, "ldc p15, c1, [r2]"),
    (0xED92_1F02, "ldc p15, c1, [r2, #0x8]"),
    (0xED92_1FFF, "ldc p15, c1, [r2, #0x3fc]"),
    (0xED12_1F02, "ldc p15, c1, [r2, #-0x8]"),
    (0xED12_1F00, "ldc p15, c1, [r2, #-0x0]"),
    (0xEDB2_1F02, "ldc p15, c1, [r2, #0x8]!"),
    (0xEDD2_1F02, "ldcl p15, c1, [r2, #0x8]"),
    (0xECB2_1F02, "ldc p15, c1, [r2], #0x8"),
    (0xEC32_1F02, "ldc p15, c1, [r2], #-0x8"),
    (0xECF2_1F02, "ldcl p15, c1, [r2], #0x8"),
    // Unindexed: P = 0, U = 1, W = 0; the option is decimal.
    (0xEC92_1F05, "ldc p15, c1, [r2], {5}"),
    (0xED90_0000, "ldc p0, c0, [r0]"),
    (0xED9D_1F00, "ldc p15, c1, [sp]"),
    (0x0D92_1F00, "ldceq p15, c1, [r2]"),
    (0x1DD2_1F02, "ldcnel p15, c1, [r2, #0x8]"),
];

#[test]
fn given_ldc_words_when_disassembled_then_the_offset_is_scaled_and_l_follows_the_condition() {
    let failures = run_both(Kind::Coprocessor, LDC);
    assert_eq!(failures, Vec::<String>::new());
}

const STC: &[Row] = &[
    (0xED82_1F02, "stc p15, c1, [r2, #0x8]"),
    (0xEDC2_1F02, "stcl p15, c1, [r2, #0x8]"),
    (0xECA2_1F02, "stc p15, c1, [r2], #0x8"),
    (0xECC2_1F05, "stcl p15, c1, [r2], {5}"),
    (0xEC82_1F05, "stc p15, c1, [r2], {5}"),
    (0x0DC2_1F02, "stceql p15, c1, [r2, #0x8]"),
];

#[test]
fn given_stc_words_when_disassembled_then_each_prints_its_addressing() {
    let failures = run_both(Kind::Coprocessor, STC);
    assert_eq!(failures, Vec::<String>::new());
}

const MCRR: &[Row] = &[
    // mcrr pN, opcode, rd, rn, crm; MCRR/MRRC win over the LDC/STC words they share.
    (0xEC42_1F03, "mcrr p15, 0, r1, r2, c3"),
    (0x0C42_1F03, "mcrreq p15, 0, r1, r2, c3"),
    (0xEC41_00FF, "mcrr p0, 15, r0, r1, c15"),
];

#[test]
fn given_mcrr_words_when_disassembled_then_they_are_not_read_as_stc() {
    let failures = run_both(Kind::Coprocessor, MCRR);
    assert_eq!(failures, Vec::<String>::new());
}

const MRRC: &[Row] = &[
    (0xEC54_3F56, "mrrc p15, 5, r3, r4, c6"),
    (0x1C54_3F56, "mrrcne p15, 5, r3, r4, c6"),
    (0xEC51_00FF, "mrrc p0, 15, r0, r1, c15"),
];

#[test]
fn given_mrrc_words_when_disassembled_then_they_are_not_read_as_ldc() {
    let failures = run_both(Kind::Coprocessor, MRRC);
    assert_eq!(failures, Vec::<String>::new());
}

// ------------------------------------------------------------------ boundary cases

const PC_RELATIVE: &[RowAt] = &[
    // PC reads as the instruction address + 8; the comment is the address read.
    (
        0xE59F_0010,
        BASE,
        Cpu::Both,
        "ldr r0, [pc, #0x10] ; 0x02000118",
    ),
    (
        0x059F_0010,
        BASE,
        Cpu::Both,
        "ldreq r0, [pc, #0x10] ; 0x02000118",
    ),
    (
        0xE51F_0008,
        BASE,
        Cpu::Both,
        "ldr r0, [pc, #-0x8] ; 0x02000100",
    ),
    (
        0xE51F_F004,
        BASE,
        Cpu::Both,
        "ldr pc, [pc, #-0x4] ; 0x02000104",
    ),
    (0xE59F_0000, BASE, Cpu::Both, "ldr r0, [pc] ; 0x02000108"),
    (
        0xE59F_0FFF,
        BASE,
        Cpu::Both,
        "ldr r0, [pc, #0xfff] ; 0x02001107",
    ),
    (
        0xE5DF_0004,
        BASE,
        Cpu::Both,
        "ldrb r0, [pc, #0x4] ; 0x0200010c",
    ),
    (
        0xE1DF_00B4,
        BASE,
        Cpu::Both,
        "ldrh r0, [pc, #0x4] ; 0x0200010c",
    ),
    (
        0xE15F_00B4,
        BASE,
        Cpu::Both,
        "ldrh r0, [pc, #-0x4] ; 0x02000104",
    ),
    (
        0xE1DF_00D2,
        BASE,
        Cpu::Both,
        "ldrsb r0, [pc, #0x2] ; 0x0200010a",
    ),
    (
        0xE1DF_00F2,
        BASE,
        Cpu::Both,
        "ldrsh r0, [pc, #0x2] ; 0x0200010a",
    ),
    // The comment follows the wrapped address.
    (
        0xE59F_0010,
        0xFFFF_FFF8,
        Cpu::Both,
        "ldr r0, [pc, #0x10] ; 0x00000010",
    ),
    (
        0xE51F_0010,
        0x0000_0000,
        Cpu::Both,
        "ldr r0, [pc, #-0x10] ; 0xfffffff8",
    ),
    // PLD with a pc base.
    (0xF5DF_F020, BASE, Cpu::Arm9, "pld [pc, #0x20] ; 0x02000128"),
    (0xF55F_F008, BASE, Cpu::Arm9, "pld [pc, #-0x8] ; 0x02000100"),
    (0xF5DF_F000, BASE, Cpu::Arm9, "pld [pc] ; 0x02000108"),
    // Post-indexed and register-offset pc accesses read no address: no comment.
    (0xE49F_0004, BASE, Cpu::Both, "ldr r0, [pc], #0x4"),
    (0xE0DF_00B2, BASE, Cpu::Both, "ldrh r0, [pc], #0x2"),
    (0xE79F_0001, BASE, Cpu::Both, "ldr r0, [pc, r1]"),
    (0xE19F_00B1, BASE, Cpu::Both, "ldrh r0, [pc, r1]"),
    (0xF7DF_F002, BASE, Cpu::Arm9, "pld [pc, r2]"),
    // Stores with a pc base get no comment; only loads and PLD do.
    (0xE58F_0004, BASE, Cpu::Both, "str r0, [pc, #0x4]"),
    (0xE5CF_0004, BASE, Cpu::Both, "strb r0, [pc, #0x4]"),
    (0xE1CF_00B4, BASE, Cpu::Both, "strh r0, [pc, #0x4]"),
    (
        0xE1CF_00D8,
        BASE,
        Cpu::Arm9,
        "ldrd r0, [pc, #0x8] ; 0x02000110",
    ),
    (0xE1CF_00F8, BASE, Cpu::Arm9, "strd r0, [pc, #0x8]"),
    (0xE0CF_00D8, BASE, Cpu::Arm9, "ldrd r0, [pc], #0x8"),
    // A base other than pc never gets a comment.
    (0xE591_0010, BASE, Cpu::Both, "ldr r0, [r1, #0x10]"),
];

#[test]
fn given_pc_relative_accesses_when_disassembled_then_only_pre_indexed_immediates_get_the_address_comment()
 {
    let failures = run_at(None, PC_RELATIVE);
    assert_eq!(failures, Vec::<String>::new());
}

const DESTINATION_PC: &[Row] = &[
    (0xE1A0_F00E, "mov pc, lr"),
    (0xE1B0_F00E, "movs pc, lr"),
    (0xE25E_F004, "subs pc, lr, #0x4"),
    (0xE1A0_F000, "mov pc, r0"),
    (0xE08F_F100, "add pc, pc, r0, lsl #2"),
    (0xE49D_F004, "ldr pc, [sp], #0x4"),
    (0xE8BD_8001, "ldmia sp!, {r0, pc}"),
    (0xE8FD_800F, "ldmia sp!, {r0-r3, pc}^"),
    (0xE12F_FF1F, "bx pc"),
];

#[test]
fn given_words_that_write_pc_when_disassembled_then_pc_is_written_like_any_register() {
    let rows: Vec<RowAt> = DESTINATION_PC
        .iter()
        .map(|&(word, text)| (word, BASE, Cpu::Both, text))
        .collect();
    let failures = run_at(None, &rows);
    assert_eq!(failures, Vec::<String>::new());
}

const ZERO_SHIFTS: &[RowAt] = &[
    (0xE1A0_0001, BASE, Cpu::Both, "mov r0, r1"),
    (0xE1A0_0021, BASE, Cpu::Both, "mov r0, r1, lsr #32"),
    (0xE1A0_0041, BASE, Cpu::Both, "mov r0, r1, asr #32"),
    (0xE1A0_0061, BASE, Cpu::Both, "mov r0, r1, rrx"),
    (0xE791_0002, BASE, Cpu::Both, "ldr r0, [r1, r2]"),
    (0xE791_0022, BASE, Cpu::Both, "ldr r0, [r1, r2, lsr #32]"),
    (0xE791_0042, BASE, Cpu::Both, "ldr r0, [r1, r2, asr #32]"),
    (0xE791_0062, BASE, Cpu::Both, "ldr r0, [r1, r2, rrx]"),
    (0xF7D1_F022, BASE, Cpu::Arm9, "pld [r1, r2, lsr #32]"),
    (0xF7D1_F062, BASE, Cpu::Arm9, "pld [r1, r2, rrx]"),
];

#[test]
fn given_zero_shift_amounts_when_disassembled_then_lsr_and_asr_read_32_ror_reads_rrx_and_lsl_vanishes()
 {
    let failures = run_at(None, ZERO_SHIFTS);
    assert_eq!(failures, Vec::<String>::new());
}

const BRANCH_TARGETS: &[(u32, u32, Cpu, Option<u32>)] = &[
    (0xEA00_0002, BASE, Cpu::Both, Some(0x0200_0110)),
    (0xEB00_0002, BASE, Cpu::Both, Some(0x0200_0110)),
    (0x0A00_0002, BASE, Cpu::Both, Some(0x0200_0110)),
    (0xEAFF_FFFE, BASE, Cpu::Both, Some(0x0200_0100)),
    (0xEA00_0000, 0xFFFF_FFF8, Cpu::Both, Some(0x0000_0000)),
    (0xEAFF_FFFD, 0x0000_0000, Cpu::Both, Some(0xFFFF_FFFC)),
    (0xEA7F_FFFF, 0x0000_0000, Cpu::Both, Some(0x0200_0004)),
    (0xEA80_0000, 0x0200_0000, Cpu::Both, Some(0x0000_0008)),
    // BLX immediate: H adds 2.
    (0xFA00_0002, BASE, Cpu::Arm9, Some(0x0200_0110)),
    (0xFB00_0002, BASE, Cpu::Arm9, Some(0x0200_0112)),
    (0xFB00_0000, 0xFFFF_FFF8, Cpu::Arm9, Some(0x0000_0002)),
    // On the ARM7 the same word is BL with condition nv: no H adjustment.
    (0xFB00_0002, BASE, Cpu::Arm7, Some(0x0200_0110)),
    // Targets held in a register, and every non-branch, have no static target.
    (0xE12F_FF11, BASE, Cpu::Both, None),
    (0xE12F_FF31, BASE, Cpu::Arm9, None),
    (0xE1A0_F00E, BASE, Cpu::Both, None),
    (0xE59F_F010, BASE, Cpu::Both, None),
    (0xE8BD_8001, BASE, Cpu::Both, None),
    (0xEF00_0001, BASE, Cpu::Both, None),
    (0xE7F0_00F0, BASE, Cpu::Both, None),
    (0xE3A0_0000, BASE, Cpu::Both, None),
    (0xF5D1_F020, BASE, Cpu::Arm9, None),
    (0xF3A0_0000, BASE, Cpu::Arm9, None),
];

#[test]
fn given_branch_and_non_branch_words_when_asked_for_the_target_then_only_b_bl_and_blx_immediate_have_one()
 {
    let failures = run_targets(BRANCH_TARGETS);
    assert_eq!(failures, Vec::<String>::new());
}

// ------------------------------------------------------------------ ARM7 versus ARM9

const ARMV5_VERSUS_ARMV4: &[PairRow] = &[
    (0xE16F_2F11, "clz r2, r1", "undefined 0xe16f2f11"),
    (0x116F_3F14, "clzne r3, r4", "undefined 0x116f3f14"),
    (0xE12F_FF31, "blx r1", "undefined 0xe12fff31"),
    (0xE102_0051, "qadd r0, r1, r2", "undefined 0xe1020051"),
    (0xE122_0051, "qsub r0, r1, r2", "undefined 0xe1220051"),
    (0xE142_0051, "qdadd r0, r1, r2", "undefined 0xe1420051"),
    (0xE162_0051, "qdsub r0, r1, r2", "undefined 0xe1620051"),
    (0xE100_3281, "smlabb r0, r1, r2, r3", "undefined 0xe1003281"),
    (0xE160_0281, "smulbb r0, r1, r2", "undefined 0xe1600281"),
    (
        0xE141_0382,
        "smlalbb r0, r1, r2, r3",
        "undefined 0xe1410382",
    ),
    (0xE120_3281, "smlawb r0, r1, r2, r3", "undefined 0xe1203281"),
    (0xE120_02A1, "smulwb r0, r1, r2", "undefined 0xe12002a1"),
    (0xE1C1_00D0, "ldrd r0, [r1]", "undefined 0xe1c100d0"),
    (0xE1C1_00F0, "strd r0, [r1]", "undefined 0xe1c100f0"),
    (0xE120_0070, "bkpt 0x0", "undefined 0xe1200070"),
    (0xF5D1_F020, "pld [r1, #0x20]", "ldrnvb pc, [r1, #0x20]"),
    (0xFA00_0002, "blx 0x02000110", "bnv 0x02000110"),
    (0xFB00_0002, "blx 0x02000112", "blnv 0x02000110"),
];

#[test]
fn given_armv5te_words_when_disassembled_on_each_processor_then_only_the_arm9_names_the_instruction()
 {
    let failures = run_pairs(ARMV5_VERSUS_ARMV4);
    assert_eq!(failures, Vec::<String>::new());
}

const CONDITION_1111: &[PairRow] = &[
    (0xF7D1_F002, "pld [r1, r2]", "ldrnvb pc, [r1, r2]"),
    (0xF3A0_0000, "undefined 0xf3a00000", "movnv r0, #0x0"),
    (0xFF00_0001, "undefined 0xff000001", "swinv 0x1"),
    (0xF791_0002, "undefined 0xf7910002", "ldrnv r0, [r1, r2]"),
    (0xF12F_FF11, "undefined 0xf12fff11", "bxnv r1"),
    (0xF890_0002, "undefined 0xf8900002", "ldmnvia r0, {r1}"),
    (0xF7F0_00F0, "undefined 0xf7f000f0", "undefined 0xf7f000f0"),
    (0xF120_0070, "undefined 0xf1200070", "undefined 0xf1200070"),
    (0xF7D1_F012, "undefined 0xf7d1f012", "undefined 0xf7d1f012"),
];

#[test]
fn given_cond_1111_words_when_disassembled_on_each_processor_then_the_arm9_keeps_only_blx_and_pld()
{
    let failures = run_pairs(CONDITION_1111);
    assert_eq!(failures, Vec::<String>::new());
}

// ------------------------------------------------------------------ properties

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(4096))]

    #[test]
    fn given_any_word_and_address_when_disassembled_on_the_arm9_then_the_mnemonic_matches_the_decoder_kind(
        word in proptest::prelude::any::<u32>(),
        address in proptest::prelude::any::<u32>(),
    ) {
        proptest::prop_assert_eq!(mnemonic_failure::<true>(word, address), Option::<String>::None);
    }

    #[test]
    fn given_any_word_and_address_when_disassembled_on_the_arm7_then_the_mnemonic_matches_the_decoder_kind(
        word in proptest::prelude::any::<u32>(),
        address in proptest::prelude::any::<u32>(),
    ) {
        proptest::prop_assert_eq!(mnemonic_failure::<false>(word, address), Option::<String>::None);
    }

    #[test]
    fn given_any_word_and_address_when_disassembled_on_the_arm9_then_branch_target_follows_the_offset_maths(
        word in proptest::prelude::any::<u32>(),
        address in proptest::prelude::any::<u32>(),
    ) {
        proptest::prop_assert_eq!(
            branch_target_failure::<true>(word, address),
            Option::<String>::None
        );
    }

    #[test]
    fn given_any_word_and_address_when_disassembled_on_the_arm7_then_branch_target_follows_the_offset_maths(
        word in proptest::prelude::any::<u32>(),
        address in proptest::prelude::any::<u32>(),
    ) {
        proptest::prop_assert_eq!(
            branch_target_failure::<false>(word, address),
            Option::<String>::None
        );
    }
}
