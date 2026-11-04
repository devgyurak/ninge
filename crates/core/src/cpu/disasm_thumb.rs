// SPDX-License-Identifier: Apache-2.0
//! The Thumb disassembler: the text of one halfword, written in pre-UAL Thumb syntax
//! (`docs/hw/disassembly.md`, "Thumb"). The halfword is classified by the table the CPU runs
//! (`thumb_decode.rs`), and a BL prefix looks at the halfword after it to print the whole
//! instruction (`docs/hw/disassembly.md`, "The BL pair").
//!
//! Display writes straight to the formatter and allocates nothing.

use super::disasm::{CONDITIONS, Reg, RegList, bits, flag, sign_extend};
use super::thumb_decode::{ThumbKind, decode};
use core::fmt;

/// Format 4's operations by bits 9–6 (`docs/hw/thumb.md`, "Formats").
const ALU_OPERATIONS: [&str; 16] = [
    "and", "eor", "lsl", "lsr", "asr", "adc", "sbc", "ror", "tst", "neg", "cmp", "cmn", "orr",
    "mul", "bic", "mvn",
];

/// Format 5's operations by bits 9–8; op 3 is BX or BLX.
const HI_REGISTER_OPERATIONS: [&str; 3] = ["add", "cmp", "mov"];

/// Format 1's shifts by bits 12–11.
const SHIFTS: [&str; 3] = ["lsl", "lsr", "asr"];

/// Format 3's operations by bits 12–11.
const IMMEDIATE_OPERATIONS: [&str; 4] = ["mov", "cmp", "add", "sub"];

/// Format 7's operations by bits 11–10 (L, B).
const REGISTER_TRANSFERS: [&str; 4] = ["str", "strb", "ldr", "ldrb"];

/// Format 8's operations by bits 11–10 (H, S).
const SIGNED_TRANSFERS: [&str; 4] = ["strh", "ldsb", "ldrh", "ldsh"];

const BIT_LOAD: u32 = 11;
const BIT_BYTE: u32 = 12;
const BIT_STACK_POINTER: u32 = 11;

/// What a BL or BLX prefix turned out to be part of, given the halfword after it
/// (`docs/hw/disassembly.md`, "The BL pair").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pair {
    /// Not a prefix, or a prefix with no suffix after it.
    None,
    /// A prefix and a BL suffix: the branch target.
    Bl(u32),
    /// A prefix and a BLX suffix (ARM9): the branch target, word aligned.
    Blx(u32),
}

/// The text and the branch target of one Thumb halfword, from [`disassemble_thumb`]. It is
/// plain data: `Display` writes the text without allocating.
///
/// See `docs/hw/disassembly.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThumbDisassembly {
    halfword: u16,
    address: u32,
    kind: ThumbKind,
    pair: Pair,
}

/// What a BL prefix leaves in LR: its address + 4 + (offset11 sign-extended << 12)
/// (`docs/hw/thumb.md`, "Long branch with link").
fn prefix_link(halfword: u16, address: u32) -> u32 {
    address
        .wrapping_add(4)
        .wrapping_add(sign_extend(bits(u32::from(halfword), 10, 0), 11) << 12)
}

/// Disassembles `halfword`, which sits at `address`, as the ARM9 (`ARM9`) or the ARM7 decodes
/// it (`docs/hw/disassembly.md`). `next` is the halfword after it, if known: a BL prefix needs
/// it to print the complete `bl` or `blx`; every other halfword ignores it. A halfword the
/// decoder gives no handler is `undefined 0x…`.
pub fn disassemble_thumb<const ARM9: bool>(
    halfword: u16,
    next: Option<u16>,
    address: u32,
) -> ThumbDisassembly {
    let kind = decode::<ARM9>(halfword);
    let pair = match (kind, next) {
        (ThumbKind::LongBranchPrefix, Some(next)) => {
            // The suffix adds its offset11 << 1 to LR.
            let target =
                prefix_link(halfword, address).wrapping_add(bits(u32::from(next), 10, 0) << 1);
            match decode::<ARM9>(next) {
                ThumbKind::LongBranchSuffix => Pair::Bl(target),
                // BLX clears bits 1–0 of the target (`docs/hw/armv5te.md`, "BLX").
                ThumbKind::LongBranchExchangeSuffix => Pair::Blx(target & !3),
                _ => Pair::None,
            }
        }
        _ => Pair::None,
    };
    ThumbDisassembly {
        halfword,
        address,
        kind,
        pair,
    }
}

impl ThumbDisassembly {
    /// The address a B, a conditional B, or a complete BL or BLX pair (on its prefix line)
    /// branches to, wrapping at 32 bits (`docs/hw/disassembly.md`, "Thumb"); `None` for every
    /// other halfword, including a suffix and a prefix with no suffix after it.
    pub fn branch_target(&self) -> Option<u32> {
        // R15 is the halfword's address + 4 in Thumb state.
        let pc = self.address.wrapping_add(4);
        let word = u32::from(self.halfword);
        match (self.kind, self.pair) {
            (ThumbKind::ConditionalBranch, _) => {
                Some(pc.wrapping_add(sign_extend(bits(word, 7, 0), 8) << 1))
            }
            (ThumbKind::Branch, _) => {
                Some(pc.wrapping_add(sign_extend(bits(word, 10, 0), 11) << 1))
            }
            (_, Pair::Bl(target) | Pair::Blx(target)) => Some(target),
            _ => None,
        }
    }

    /// The address `ldr rd, [pc, #offset]` reads: the word-aligned PC plus the offset
    /// (`docs/hw/thumb.md`, "Edge cases").
    fn pc_relative(&self, offset: u32) -> u32 {
        (self.address.wrapping_add(4) & !3).wrapping_add(offset)
    }
}

/// A register field of three bits starting at `low`.
const fn low_register(word: u32, low: u32) -> Reg {
    Reg(bits(word, low + 2, low))
}

/// A register of format 5: the H bit at `high_bit` extends the 3-bit field at `low`.
const fn high_register(word: u32, high_bit: u32, low: u32) -> Reg {
    Reg(bits(word, high_bit, high_bit) << 3 | bits(word, low + 2, low))
}

/// `[base, #offset]`, with a zero offset written `[base]`.
struct BaseOffset {
    base: Reg,
    offset: u32,
}

impl fmt::Display for BaseOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.offset {
            0 => write!(f, "[{}]", self.base),
            offset => write!(f, "[{}, #{offset:#x}]", self.base),
        }
    }
}

impl fmt::Display for ThumbDisassembly {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let word = u32::from(self.halfword);
        let (rd, rs) = (low_register(word, 0), low_register(word, 3));
        // Formats 3, 6, 11, 12 and 15 keep their register in bits 10–8.
        let rd8 = low_register(word, 8);
        let ro = low_register(word, 6);
        let load = flag(word, BIT_LOAD);
        match self.kind {
            ThumbKind::Unhandled => write!(f, "undefined {:#06x}", self.halfword),
            ThumbKind::ShiftImmediate => {
                // LSR and ASR by #0 shift by 32.
                let amount = match (bits(word, 12, 11), bits(word, 10, 6)) {
                    (1 | 2, 0) => 32,
                    (_, amount) => amount,
                };
                let name = SHIFTS[bits(word, 12, 11) as usize];
                write!(f, "{name} {rd}, {rs}, #{amount}")
            }
            ThumbKind::AddSub => {
                let name = match flag(word, 9) {
                    true => "sub",
                    false => "add",
                };
                match flag(word, 10) {
                    true => write!(f, "{name} {rd}, {rs}, #{:#x}", bits(word, 8, 6)),
                    false => write!(f, "{name} {rd}, {rs}, {ro}"),
                }
            }
            ThumbKind::Immediate => write!(
                f,
                "{} {rd8}, #{:#x}",
                IMMEDIATE_OPERATIONS[bits(word, 12, 11) as usize],
                bits(word, 7, 0)
            ),
            ThumbKind::Alu => write!(
                f,
                "{} {rd}, {rs}",
                ALU_OPERATIONS[bits(word, 9, 6) as usize]
            ),
            ThumbKind::HiRegister => write!(
                f,
                "{} {}, {}",
                HI_REGISTER_OPERATIONS[bits(word, 9, 8) as usize],
                high_register(word, 7, 0),
                high_register(word, 6, 3)
            ),
            ThumbKind::BranchExchange => write!(f, "bx {}", high_register(word, 6, 3)),
            ThumbKind::BranchLinkExchange => write!(f, "blx {}", high_register(word, 6, 3)),
            ThumbKind::LoadPc => {
                let offset = bits(word, 7, 0) << 2;
                let source = BaseOffset {
                    base: Reg(15),
                    offset,
                };
                write!(
                    f,
                    "ldr {rd8}, {source} ; {:#010x}",
                    self.pc_relative(offset)
                )
            }
            ThumbKind::TransferRegister => write!(
                f,
                "{} {rd}, [{rs}, {ro}]",
                REGISTER_TRANSFERS[bits(word, 11, 10) as usize]
            ),
            ThumbKind::TransferSigned => write!(
                f,
                "{} {rd}, [{rs}, {ro}]",
                SIGNED_TRANSFERS[bits(word, 11, 10) as usize]
            ),
            ThumbKind::TransferImmediate => {
                // A word offset counts words, a byte offset bytes.
                let (name, scale) = match (flag(word, BIT_BYTE), load) {
                    (false, false) => ("str", 2),
                    (false, true) => ("ldr", 2),
                    (true, false) => ("strb", 0),
                    (true, true) => ("ldrb", 0),
                };
                let offset = bits(word, 10, 6) << scale;
                write!(f, "{name} {rd}, {}", BaseOffset { base: rs, offset })
            }
            ThumbKind::TransferHalfword => {
                let name = match load {
                    true => "ldrh",
                    false => "strh",
                };
                let offset = bits(word, 10, 6) << 1;
                write!(f, "{name} {rd}, {}", BaseOffset { base: rs, offset })
            }
            ThumbKind::TransferSp => {
                let name = match load {
                    true => "ldr",
                    false => "str",
                };
                let source = BaseOffset {
                    base: Reg(13),
                    offset: bits(word, 7, 0) << 2,
                };
                write!(f, "{name} {rd8}, {source}")
            }
            ThumbKind::LoadAddress => {
                let base = match flag(word, BIT_STACK_POINTER) {
                    true => Reg(13),
                    false => Reg(15),
                };
                write!(f, "add {rd8}, {base}, #{:#x}", bits(word, 7, 0) << 2)
            }
            ThumbKind::AdjustSp => {
                let name = match flag(word, 7) {
                    true => "sub",
                    false => "add",
                };
                write!(f, "{name} sp, #{:#x}", bits(word, 6, 0) << 2)
            }
            ThumbKind::PushPop => {
                // R adds LR to a PUSH and PC to a POP.
                let extra = match (flag(word, 8), load) {
                    (false, _) => 0,
                    (true, false) => 1 << 14,
                    (true, true) => 1 << 15,
                };
                let name = match load {
                    true => "pop",
                    false => "push",
                };
                write!(f, "{name} {}", RegList((bits(word, 7, 0) | extra) as u16))
            }
            ThumbKind::BlockTransfer => {
                let name = match load {
                    true => "ldmia",
                    false => "stmia",
                };
                write!(f, "{name} {rd8}!, {}", RegList(bits(word, 7, 0) as u16))
            }
            ThumbKind::ConditionalBranch => {
                // TODO(hw-unverified): condition 1110 is documented as undefined; it is
                // written `al` to show the odd encoding (`docs/hw/disassembly.md`, "Unverified").
                let cond = match bits(word, 11, 8) {
                    14 => "al",
                    cond => CONDITIONS[cond as usize],
                };
                write!(f, "b{cond} {:#010x}", self.branch_target().unwrap_or(0))
            }
            ThumbKind::Swi => write!(f, "swi {:#x}", bits(word, 7, 0)),
            ThumbKind::Branch => write!(f, "b {:#010x}", self.branch_target().unwrap_or(0)),
            ThumbKind::LongBranchPrefix => match self.pair {
                Pair::Bl(target) => write!(f, "bl {target:#010x}"),
                Pair::Blx(target) => write!(f, "blx {target:#010x}"),
                Pair::None => write!(
                    f,
                    "(bl prefix lr={:#010x})",
                    prefix_link(self.halfword, self.address)
                ),
            },
            ThumbKind::LongBranchSuffix => f.write_str("(bl suffix)"),
            ThumbKind::LongBranchExchangeSuffix => f.write_str("(blx suffix)"),
            ThumbKind::Breakpoint => write!(f, "bkpt {:#x}", bits(word, 7, 0)),
        }
    }
}
