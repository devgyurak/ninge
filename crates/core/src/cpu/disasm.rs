// SPDX-License-Identifier: Apache-2.0
//! The ARM disassembler: the text of one instruction word, written in GBATEK's pre-UAL syntax
//! (`docs/hw/disassembly.md`, "ARM"). The word is classified by the same table the CPU runs
//! (`decode.rs`), so the text can never name an instruction the processor would not execute as
//! that instruction.
//!
//! Display is not on the hot path, but it still allocates nothing: it writes straight to the
//! formatter.

use super::decode::{Kind, decode};
use core::fmt;

/// The condition names by field value; AL (1110) is not written, and 1111 is `nv`
/// (`docs/hw/disassembly.md`, "ARM").
pub(super) const CONDITIONS: [&str; 16] = [
    "eq", "ne", "cs", "cc", "mi", "pl", "vs", "vc", "hi", "ls", "ge", "lt", "gt", "le", "", "nv",
];

/// The data-processing mnemonics by opcode (`docs/hw/arm-data-processing.md`, "Encoding").
const DATA_OPCODES: [&str; 16] = [
    "and", "eor", "sub", "rsb", "add", "adc", "sbc", "rsc", "tst", "teq", "cmp", "cmn", "orr",
    "mov", "bic", "mvn",
];

/// The shift names by the type field in bits 6–5 (`docs/hw/arm-barrel-shifter.md`).
pub(super) const SHIFTS: [&str; 4] = ["lsl", "lsr", "asr", "ror"];

/// MUL-family mnemonics by bits 22–21 (`docs/hw/arm-multiply.md`, "Encoding").
const LONG_MULTIPLIES: [&str; 4] = ["umull", "umlal", "smull", "smlal"];

/// QADD and its relatives by bits 22–21 (`docs/hw/armv5te.md`, "Saturating arithmetic").
const SATURATING: [&str; 4] = ["qadd", "qsub", "qdadd", "qdsub"];

// Bit positions of the flag bits the formats share.
const BIT_SHIFT_BY_REGISTER: u32 = 4;
const BIT_S: u32 = 20;
const BIT_L: u32 = 20;
const BIT_W: u32 = 21;
const BIT_B: u32 = 22;
const BIT_U: u32 = 23;
const BIT_P: u32 = 24;
const BIT_IMMEDIATE: u32 = 25;

/// Bits `low..=high` of `word`, right-aligned.
pub(super) const fn bits(word: u32, high: u32, low: u32) -> u32 {
    (word >> low) & ((1 << (high - low + 1)) - 1)
}

pub(super) const fn flag(word: u32, bit: u32) -> bool {
    word >> bit & 1 == 1
}

/// `value`'s low `width` bits as a signed number, sign-extended to 32 bits.
pub(super) const fn sign_extend(value: u32, width: u32) -> u32 {
    ((value << (32 - width)) as i32 >> (32 - width)) as u32
}

const fn rn(word: u32) -> Reg {
    Reg(bits(word, 19, 16))
}

const fn rd(word: u32) -> Reg {
    Reg(bits(word, 15, 12))
}

const fn rs(word: u32) -> Reg {
    Reg(bits(word, 11, 8))
}

const fn rm(word: u32) -> Reg {
    Reg(bits(word, 3, 0))
}

/// A register by number: `r0`–`r12`, `sp`, `lr`, `pc`.
#[derive(Clone, Copy)]
pub(super) struct Reg(pub(super) u32);

impl fmt::Display for Reg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            13 => f.write_str("sp"),
            14 => f.write_str("lr"),
            15 => f.write_str("pc"),
            n => write!(f, "r{n}"),
        }
    }
}

/// A register list as `{r0-r3, lr}`: runs of three or more registers become ranges
/// (`docs/hw/disassembly.md`, "Common notation").
#[derive(Clone, Copy)]
pub(super) struct RegList(pub(super) u16);

impl RegList {
    const fn has(self, index: u32) -> bool {
        self.0 >> index & 1 == 1
    }
}

impl fmt::Display for RegList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("{")?;
        let mut index = 0;
        let mut first = true;
        while index < 16 {
            if !self.has(index) {
                index += 1;
                continue;
            }
            let start = index;
            while index < 16 && self.has(index) {
                index += 1;
            }
            let last = index - 1;
            if !first {
                f.write_str(", ")?;
            }
            first = false;
            match last - start {
                0 => write!(f, "{}", Reg(start))?,
                1 => write!(f, "{}, {}", Reg(start), Reg(last))?,
                _ => write!(f, "{}-{}", Reg(start), Reg(last))?,
            }
        }
        f.write_str("}")
    }
}

/// A shift applied to a register: by an immediate amount, whose #0 has special meanings, or
/// by a register (`docs/hw/disassembly.md`, "Notes on the forms").
#[derive(Clone, Copy)]
enum Shift {
    Immediate { kind: u32, amount: u32 },
    Register { kind: u32, rs: Reg },
}

impl Shift {
    /// The shift in bits 11–4 of a data-processing operand or a single-transfer offset.
    const fn of(word: u32) -> Self {
        let kind = bits(word, 6, 5);
        match flag(word, BIT_SHIFT_BY_REGISTER) {
            true => Self::Register { kind, rs: rs(word) },
            false => Self::Immediate {
                kind,
                amount: bits(word, 11, 7),
            },
        }
    }
}

impl fmt::Display for Shift {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Register { kind, rs } => write!(f, "{} {rs}", SHIFTS[kind as usize]),
            // LSL #0 is no shift; LSR and ASR #0 mean #32; ROR #0 is RRX.
            Self::Immediate { kind: 0, amount: 0 } => Ok(()),
            Self::Immediate { kind: 3, amount: 0 } => f.write_str("rrx"),
            Self::Immediate { kind, amount } => {
                let amount = match (kind, amount) {
                    (1 | 2, 0) => 32,
                    (_, amount) => amount,
                };
                write!(f, "{} #{amount}", SHIFTS[kind as usize])
            }
        }
    }
}

/// A shifted register operand, `rm` or `rm, lsl #2`.
#[derive(Clone, Copy)]
struct ShiftedRegister {
    rm: Reg,
    shift: Shift,
}

impl fmt::Display for ShiftedRegister {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.shift {
            // LSL #0 writes no shift at all.
            Shift::Immediate { kind: 0, amount: 0 } => write!(f, "{}", self.rm),
            shift => write!(f, "{}, {shift}", self.rm),
        }
    }
}

/// The offset of a load or store address.
#[derive(Clone, Copy)]
enum Offset {
    Immediate(u32),
    Register(ShiftedRegister),
}

/// The addressing of a load or store: base, offset sign, and pre/post-indexing.
#[derive(Clone, Copy)]
struct Address {
    base: Reg,
    offset: Offset,
    up: bool,
    pre: bool,
    writeback: bool,
}

impl Address {
    /// The `[rn, …]` or `[rn], …` text. An offset of +0 is omitted in the pre-indexed form
    /// without writeback.
    fn write(self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = match self.up {
            true => "",
            false => "-",
        };
        match (self.pre, self.offset, self.up, self.writeback) {
            (true, Offset::Immediate(0), true, false) => write!(f, "[{}]", self.base),
            (true, _, _, writeback) => {
                write!(f, "[{}, ", self.base)?;
                self.write_offset(f, sign)?;
                f.write_str("]")?;
                f.write_str(letter(writeback, "!"))
            }
            (false, ..) => {
                write!(f, "[{}], ", self.base)?;
                self.write_offset(f, sign)
            }
        }
    }

    fn write_offset(self, f: &mut fmt::Formatter<'_>, sign: &str) -> fmt::Result {
        match self.offset {
            Offset::Immediate(value) => write!(f, "#{sign}{value:#x}"),
            Offset::Register(register) => write!(f, "{sign}{register}"),
        }
    }

    /// The address a pc-based, pre-indexed, immediate-offset access reads, for the trailing
    /// comment (`docs/hw/disassembly.md`, "Common notation"): R15 is the instruction's
    /// address + 8 in ARM state.
    fn pc_relative(self, instruction_address: u32) -> Option<u32> {
        match (self.base.0, self.offset, self.pre) {
            (15, Offset::Immediate(offset), true) => {
                let pc = instruction_address.wrapping_add(8);
                Some(match self.up {
                    true => pc.wrapping_add(offset),
                    false => pc.wrapping_sub(offset),
                })
            }
            _ => None,
        }
    }
}

/// The text and the branch target of one ARM instruction word, from
/// [`disassemble_arm`]. It is plain data: `Display` writes the text without allocating.
///
/// See `docs/hw/disassembly.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArmDisassembly {
    word: u32,
    address: u32,
    kind: Kind,
}

/// Disassembles `word`, which sits at `address`, as the ARM9 (`ARM9`) or the ARM7 decodes it
/// (`docs/hw/disassembly.md`). A word the decoder gives no handler is `undefined 0x…`.
pub fn disassemble_arm<const ARM9: bool>(word: u32, address: u32) -> ArmDisassembly {
    ArmDisassembly {
        word,
        address,
        kind: decode::<ARM9>(word),
    }
}

impl ArmDisassembly {
    /// The address a B, BL or BLX with an immediate branches to, wrapping at 32 bits
    /// (`docs/hw/disassembly.md`, "Branch targets"); `None` for every other instruction,
    /// including the register forms of BX and BLX.
    pub fn branch_target(&self) -> Option<u32> {
        // The offset is signed, in words, relative to the address + 8 (R15 in ARM state;
        // `docs/hw/arm-branch-pipeline.md`).
        let target = self
            .address
            .wrapping_add(8)
            .wrapping_add(sign_extend(bits(self.word, 23, 0), 24) << 2);
        match self.kind {
            Kind::Branch => Some(target),
            // H (bit 24) adds a halfword (`docs/hw/armv5te.md`, "BLX").
            Kind::BranchLinkExchangeImmediate => {
                Some(target.wrapping_add(bits(self.word, 24, 24) << 1))
            }
            _ => None,
        }
    }

    fn condition(&self) -> &'static str {
        CONDITIONS[bits(self.word, 31, 28) as usize]
    }
}

/// `s` when the S bit (bit 20) is set.
const fn s_suffix(word: u32) -> &'static str {
    letter(flag(word, BIT_S), "s")
}

const fn letter(set: bool, text: &'static str) -> &'static str {
    match set {
        true => text,
        false => "",
    }
}

/// The rotated 8-bit immediate of a data-processing or MSR word, after the rotation.
const fn rotated_immediate(word: u32) -> u32 {
    bits(word, 7, 0).rotate_right(bits(word, 11, 8) * 2)
}

const fn psr_name(word: u32) -> &'static str {
    match flag(word, BIT_B) {
        true => "spsr",
        false => "cpsr",
    }
}

/// The MSR field mask letters in the order `f s x c` (bits 19–16).
struct PsrFields(u32);

impl fmt::Display for PsrFields {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(letter(flag(self.0, 19), "f"))?;
        f.write_str(letter(flag(self.0, 18), "s"))?;
        f.write_str(letter(flag(self.0, 17), "x"))?;
        f.write_str(letter(flag(self.0, 16), "c"))
    }
}

/// The address of a single data transfer (`docs/hw/arm-load-store.md`, "Encodings"); `t` is
/// set for the post-indexed form with W (LDRT, STRT).
fn single_address(word: u32, register_offset: bool) -> (Address, bool) {
    let pre = flag(word, BIT_P);
    let offset = match register_offset {
        true => Offset::Register(ShiftedRegister {
            rm: rm(word),
            shift: Shift::of(word),
        }),
        false => Offset::Immediate(bits(word, 11, 0)),
    };
    let address = Address {
        base: rn(word),
        offset,
        up: flag(word, BIT_U),
        pre,
        writeback: pre && flag(word, BIT_W),
    };
    (address, !pre && flag(word, BIT_W))
}

/// The address of a halfword or doubleword transfer: the split 8-bit immediate (bit 22) or Rm.
fn split_address(word: u32) -> Address {
    let pre = flag(word, BIT_P);
    let offset = match flag(word, BIT_B) {
        true => Offset::Immediate(bits(word, 11, 8) << 4 | bits(word, 3, 0)),
        false => Offset::Register(ShiftedRegister {
            rm: rm(word),
            shift: Shift::Immediate { kind: 0, amount: 0 },
        }),
    };
    Address {
        base: rn(word),
        offset,
        up: flag(word, BIT_U),
        pre,
        writeback: pre && flag(word, BIT_W),
    }
}

/// The address of PLD: pre-indexed, no writeback; bit 25 selects the register offset
/// (`docs/hw/armv5te.md`, "PLD").
fn preload_address(word: u32) -> Address {
    let offset = match flag(word, BIT_IMMEDIATE) {
        true => Offset::Register(ShiftedRegister {
            rm: rm(word),
            shift: Shift::of(word),
        }),
        false => Offset::Immediate(bits(word, 11, 0)),
    };
    Address {
        base: rn(word),
        offset,
        up: flag(word, BIT_U),
        pre: true,
        writeback: false,
    }
}

/// The address of LDC and STC: the 8-bit offset counts words (`docs/hw/arm-exceptions-modes.md`,
/// "Coprocessor instructions").
fn coprocessor_address(word: u32) -> Address {
    let pre = flag(word, BIT_P);
    Address {
        base: rn(word),
        offset: Offset::Immediate(bits(word, 7, 0) << 2),
        up: flag(word, BIT_U),
        pre,
        writeback: pre && flag(word, BIT_W),
    }
}

impl ArmDisassembly {
    fn data_processing(
        &self,
        f: &mut fmt::Formatter<'_>,
        operand: impl fmt::Display,
    ) -> fmt::Result {
        let word = self.word;
        let name = DATA_OPCODES[bits(word, 24, 21) as usize];
        let cond = self.condition();
        match bits(word, 24, 21) {
            // TST, TEQ, CMP and CMN set flags only, so the S bit is implied.
            8..=11 => write!(f, "{name}{cond} {}, {operand}", rn(word)),
            // MOV and MVN have no first operand.
            13 | 15 => write!(f, "{name}{cond}{} {}, {operand}", s_suffix(word), rd(word)),
            _ => write!(
                f,
                "{name}{cond}{} {}, {}, {operand}",
                s_suffix(word),
                rd(word),
                rn(word)
            ),
        }
    }

    /// `<mnemonic> rd, <address>`, with the pc-relative comment on a load.
    fn load_store(
        &self,
        f: &mut fmt::Formatter<'_>,
        mnemonic: impl fmt::Display,
        address: Address,
        load: bool,
    ) -> fmt::Result {
        write!(f, "{mnemonic} {}, ", rd(self.word))?;
        address.write(f)?;
        match (load, address.pc_relative(self.address)) {
            (true, Some(target)) => write!(f, " ; {target:#010x}"),
            _ => Ok(()),
        }
    }

    fn single_transfer(&self, f: &mut fmt::Formatter<'_>, register_offset: bool) -> fmt::Result {
        let word = self.word;
        let (address, user) = single_address(word, register_offset);
        let load = flag(word, BIT_L);
        let name = match load {
            true => "ldr",
            false => "str",
        };
        let mnemonic = format_args!(
            "{name}{}{}{}",
            self.condition(),
            letter(flag(word, BIT_B), "b"),
            letter(user, "t")
        );
        self.load_store(f, mnemonic, address, load)
    }

    fn halfword_transfer(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let word = self.word;
        let load = flag(word, BIT_L);
        let (name, size) = match (load, bits(word, 6, 5)) {
            (true, 2) => ("ldr", "sb"),
            (true, 3) => ("ldr", "sh"),
            (true, _) => ("ldr", "h"),
            (false, _) => ("str", "h"),
        };
        let mnemonic = format_args!("{name}{}{size}", self.condition());
        self.load_store(f, mnemonic, split_address(word), load)
    }

    fn double_transfer(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let word = self.word;
        // SH = 10 is LDRD and SH = 11 is STRD (`docs/hw/armv5te.md`, "LDRD and STRD").
        let load = bits(word, 6, 5) == 2;
        let name = match load {
            true => "ldr",
            false => "str",
        };
        let mnemonic = format_args!("{name}{}d", self.condition());
        self.load_store(f, mnemonic, split_address(word), load)
    }

    fn block_transfer(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let word = self.word;
        let name = match flag(word, BIT_L) {
            true => "ldm",
            false => "stm",
        };
        // P and U pick the mode (`docs/hw/arm-load-store.md`, "Block data transfer").
        let mode = match (flag(word, BIT_P), flag(word, BIT_U)) {
            (true, true) => "ib",
            (false, true) => "ia",
            (true, false) => "db",
            (false, false) => "da",
        };
        write!(
            f,
            "{name}{}{mode} {}{}, {}{}",
            self.condition(),
            rn(word),
            letter(flag(word, BIT_W), "!"),
            RegList(bits(word, 15, 0) as u16),
            // The S bit (bit 22) is the user-bank / SPSR-restore marker.
            letter(flag(word, BIT_B), "^")
        )
    }

    fn signed_multiply(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let word = self.word;
        let cond = self.condition();
        let x = match flag(word, 5) {
            true => "t",
            false => "b",
        };
        let y = match flag(word, 6) {
            true => "t",
            false => "b",
        };
        // Rd is bits 19–16, the accumulator (or RdLo) bits 15–12.
        let (d, acc) = (rn(word), rd(word));
        let (m, s) = (rm(word), rs(word));
        match (bits(word, 22, 21), flag(word, 5)) {
            (0, _) => write!(f, "smla{x}{y}{cond} {d}, {m}, {s}, {acc}"),
            (1, false) => write!(f, "smlaw{y}{cond} {d}, {m}, {s}, {acc}"),
            (1, true) => write!(f, "smulw{y}{cond} {d}, {m}, {s}"),
            (2, _) => write!(f, "smlal{x}{y}{cond} {acc}, {d}, {m}, {s}"),
            _ => write!(f, "smul{x}{y}{cond} {d}, {m}, {s}"),
        }
    }

    fn coprocessor(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let word = self.word;
        let cond = self.condition();
        let cp = bits(word, 11, 8);
        let (c_n, c_m) = (bits(word, 19, 16), bits(word, 3, 0));
        let c_d = bits(word, 15, 12);
        let load = flag(word, BIT_L);
        match (bits(word, 27, 20), bits(word, 27, 25), flag(word, 4)) {
            // MCRR and MRRC take the words LDC and STC would otherwise have.
            (0xC4 | 0xC5, ..) => {
                let name = match load {
                    true => "mrrc",
                    false => "mcrr",
                };
                write!(
                    f,
                    "{name}{cond} p{cp}, {}, {}, {}, c{c_m}",
                    bits(word, 7, 4),
                    rd(word),
                    rn(word)
                )
            }
            (_, 0b110, _) => {
                let name = match load {
                    true => "ldc",
                    false => "stc",
                };
                write!(
                    f,
                    "{name}{cond}{} p{cp}, c{c_d}, ",
                    letter(flag(word, BIT_B), "l")
                )?;
                match (flag(word, BIT_P), flag(word, BIT_U), flag(word, BIT_W)) {
                    // The unindexed form hands the coprocessor an 8-bit option.
                    (false, true, false) => write!(f, "[{}], {{{}}}", rn(word), bits(word, 7, 0)),
                    _ => coprocessor_address(word).write(f),
                }
            }
            (_, _, false) => write!(
                f,
                "cdp{cond} p{cp}, {}, c{c_d}, c{c_n}, c{c_m}, {}",
                bits(word, 23, 20),
                bits(word, 7, 5)
            ),
            (_, _, true) => {
                let name = match load {
                    true => "mrc",
                    false => "mcr",
                };
                write!(
                    f,
                    "{name}{cond} p{cp}, {}, {}, c{c_n}, c{c_m}, {}",
                    bits(word, 23, 21),
                    rd(word),
                    bits(word, 7, 5)
                )
            }
        }
    }
}

impl fmt::Display for ArmDisassembly {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let word = self.word;
        let cond = self.condition();
        match self.kind {
            Kind::Unhandled | Kind::Undefined => write!(f, "undefined {word:#010x}"),
            Kind::DataImmediate => {
                self.data_processing(f, format_args!("#{:#x}", rotated_immediate(word)))
            }
            Kind::DataImmediateShift | Kind::DataRegisterShift => self.data_processing(
                f,
                ShiftedRegister {
                    rm: rm(word),
                    shift: Shift::of(word),
                },
            ),
            Kind::Mrs => write!(f, "mrs{cond} {}, {}", rd(word), psr_name(word)),
            Kind::MsrImmediate => write!(
                f,
                "msr{cond} {}_{}, #{:#x}",
                psr_name(word),
                PsrFields(word),
                rotated_immediate(word)
            ),
            Kind::MsrRegister => write!(
                f,
                "msr{cond} {}_{}, {}",
                psr_name(word),
                PsrFields(word),
                rm(word)
            ),
            Kind::Multiply => {
                // Rd is bits 19–16 here, and Rn (the accumulator) bits 15–12.
                let (d, acc, m, s) = (rn(word), rd(word), rm(word), rs(word));
                match flag(word, BIT_W) {
                    true => write!(f, "mla{cond}{} {d}, {m}, {s}, {acc}", s_suffix(word)),
                    false => write!(f, "mul{cond}{} {d}, {m}, {s}", s_suffix(word)),
                }
            }
            Kind::MultiplyLong => write!(
                f,
                "{}{cond}{} {}, {}, {}, {}",
                LONG_MULTIPLIES[bits(word, 22, 21) as usize],
                s_suffix(word),
                rd(word),
                rn(word),
                rm(word),
                rs(word)
            ),
            Kind::Swap => write!(
                f,
                "swp{cond}{} {}, {}, [{}]",
                letter(flag(word, BIT_B), "b"),
                rd(word),
                rm(word),
                rn(word)
            ),
            Kind::SingleTransferImmediate => self.single_transfer(f, false),
            Kind::SingleTransferRegister => self.single_transfer(f, true),
            Kind::HalfwordTransfer => self.halfword_transfer(f),
            Kind::DoubleTransfer => self.double_transfer(f),
            Kind::BlockTransfer => self.block_transfer(f),
            Kind::Branch => {
                let name = match flag(word, BIT_P) {
                    true => "bl",
                    false => "b",
                };
                write!(
                    f,
                    "{name}{cond} {:#010x}",
                    self.branch_target().unwrap_or(0)
                )
            }
            Kind::BranchExchange => write!(f, "bx{cond} {}", rm(word)),
            Kind::BranchLinkExchange => write!(f, "blx{cond} {}", rm(word)),
            Kind::BranchLinkExchangeImmediate => {
                write!(f, "blx {:#010x}", self.branch_target().unwrap_or(0))
            }
            Kind::CountLeadingZeros => write!(f, "clz{cond} {}, {}", rd(word), rm(word)),
            Kind::SaturatingArithmetic => write!(
                f,
                "{}{cond} {}, {}, {}",
                SATURATING[bits(word, 22, 21) as usize],
                rd(word),
                rm(word),
                rn(word)
            ),
            Kind::SignedHalfwordMultiply => self.signed_multiply(f),
            // TODO(hw-unverified): the condition is shown because the decoder runs BKPT
            // conditionally (`docs/hw/disassembly.md`, "Unverified").
            Kind::Breakpoint => write!(
                f,
                "bkpt{cond} {:#x}",
                bits(word, 19, 8) << 4 | bits(word, 3, 0)
            ),
            Kind::Preload => {
                let address = preload_address(word);
                f.write_str("pld ")?;
                address.write(f)?;
                match address.pc_relative(self.address) {
                    Some(target) => write!(f, " ; {target:#010x}"),
                    None => Ok(()),
                }
            }
            Kind::Swi => write!(f, "swi{cond} {:#x}", bits(word, 23, 0)),
            // TODO(hw-unverified): the ARM7 traps these as undefined, but the decoder says
            // Coprocessor, so the general form is printed (`docs/hw/disassembly.md`,
            // "Unverified").
            Kind::Coprocessor => self.coprocessor(f),
        }
    }
}
