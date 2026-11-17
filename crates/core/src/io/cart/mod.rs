// SPDX-License-Identifier: Apache-2.0
//! The cartridge slot's ROM interface: EXMEMCNT, AUXSPICNT (the ROM-mode bits), ROMCTRL, the
//! command registers and the data port. Note: `docs/hw/cart-protocol.md`.
//!
//! Seen by both CPUs: one instance, with the CPU as the const generic of the accessors
//! (`AGENTS.md` §4, "Per-CPU devices"). EXMEMCNT bit 11 gives the slot to one CPU at a time; the
//! other CPU reads zero from the slot registers and its writes are dropped.
//!
//! The device holds the protocol's state and nothing of the ROM: the image arrives with each
//! access in the context (`device::Cx::rom`), and the bytes of a word are read from it when the
//! word is taken, so a savestate carries no ROM.
// TODO(#38): the SPI side of AUXSPICNT (baud rate, hold, busy) and AUXSPIDATA arrive with the
// save chip; they are stored or read as zero here.

use crate::cart::protocol::{self, BLOCK_BYTES};
use crate::io::device::{Checked, Cx, Device, Effect, Width, read_bytes, write_bytes};
use crate::io::irq::{CARD_TRANSFER_IRQ, Requests};
use serde::{Deserialize, Serialize};

/// AUXSPICNT, at `0400_01A0h`; the device's register space starts here.
pub(crate) const AUXSPICNT_ADDR: u32 = 0x0400_01A0;
/// Bytes of the register block at `AUXSPICNT_ADDR`: AUXSPICNT, AUXSPIDATA, ROMCTRL and the
/// command registers.
pub(crate) const BLOCK_LEN: u32 = 16;
/// EXMEMCNT (ARM9) and EXMEMSTAT (ARM7), at `0400_0204h`: a different block, the same device.
pub(crate) const EXMEMCNT_ADDR: u32 = 0x0400_0204;
/// The data port, at `0410_0010h`: a different block, the same device.
pub(crate) const DATA_ADDR: u32 = 0x0410_0010;

const ROMCTRL_OFFSET: u32 = 4;
const COMMAND_OFFSET: u32 = 8;
const EXMEMCNT_OFFSET: u32 = EXMEMCNT_ADDR - AUXSPICNT_ADDR;
const EXMEMCNT_HIGH: u32 = EXMEMCNT_OFFSET + 1;
const DATA_OFFSET: u32 = DATA_ADDR - AUXSPICNT_ADDR;
/// The data port's last byte.
const DATA_LAST: u32 = DATA_OFFSET + 3;

/// The AUXSPICNT bits that are kept: SPI baud rate (0-1) and hold (6), slot mode (13), the
/// transfer-ready IRQ (14) and the enable (15) (`cart-protocol.md`, "AUXSPICNT bits").
const AUXSPICNT_MASK: u16 = 0xE043;
/// AUXSPICNT bit 14: the transfer-ready IRQ.
const AUXSPICNT_IRQ: u16 = 1 << 14;

/// ROMCTRL bit 15: apply the KEY2 seed. Write-only; reads 0.
const ROMCTRL_SEED: u32 = 1 << 15;
/// ROMCTRL bit 23: data word status, 1 = a word is ready. Read-only.
const ROMCTRL_READY: u32 = 1 << 23;
/// ROMCTRL bit 29: RESB, release reset. Once set it stays set.
const ROMCTRL_RESB: u32 = 1 << 29;
/// ROMCTRL bit 31: block start on write, busy on read.
const ROMCTRL_BUSY: u32 = 1 << 31;
/// The ROMCTRL bits a write stores as given; the others are not state.
const ROMCTRL_STORED: u32 = !(ROMCTRL_SEED | ROMCTRL_READY | ROMCTRL_BUSY);
/// ROMCTRL's data block size field, bits 24-26.
const BLOCK_SIZE_SHIFT: u32 = 24;
const BLOCK_SIZE_MASK: u32 = 0b111;

/// EXMEMCNT bit 11: NDS slot access rights, 0 = ARM9, 1 = ARM7.
const EXMEMCNT_SLOT_ARM7: u16 = 1 << 11;
/// EXMEMCNT bits 0-6: kept per CPU.
const EXMEMCNT_OWN_MASK: u8 = 0x7F;
/// EXMEMCNT bits 7-15 that read back, one setting for both CPUs that only the ARM9 changes:
/// bit 7, 11, 13, 14 and 15; bits 8-10 and 12 read zero (`cart-protocol.md`, "EXMEMCNT bits").
// TODO(hw-unverified): EXMEMCNT's reset value and its bits 13-15 are only stored (`cart-protocol.md`,
// "Unverified"). IREQ_MC (IF bit 20) is not modeled.
const EXMEMCNT_SHARED_MASK: u16 = 0xE880;

/// The `B7` command: read data (`cart-protocol.md`, "Commands").
const CMD_READ: u8 = 0xB7;
/// The `B8` command: chip ID.
const CMD_CHIP_ID: u8 = 0xB8;
/// What a word of an unknown command reads.
// TODO(hw-unverified): GBATEK describes an endless stream of encrypted zero bytes; with no
// encryption to apply the model returns `FFFF_FFFFh` (`cart-protocol.md`, "Unverified").
const UNKNOWN_WORD: u32 = 0xFFFF_FFFF;

/// A transfer's data block size in bytes, from ROMCTRL bits 24-26: 0 none, 1 to 6 `100h << n`,
/// 7 four bytes (`cart-protocol.md`, "ROMCTRL bits").
const fn block_bytes(size: u32) -> u32 {
    match size {
        0 => 0,
        7 => 4,
        n => 0x100 << n,
    }
}

/// The slot's registers and the transfer in progress. Plain data: the ROM is not part of it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cart {
    /// EXMEMCNT bits 0-6 of the ARM9's copy.
    exmem9: u8,
    /// EXMEMSTAT bits 0-6 of the ARM7's copy.
    exmem7: u8,
    /// EXMEMCNT bits 7-15, the one setting both CPUs read.
    exmem_shared: u16,
    /// AUXSPICNT, the kept bits.
    auxspicnt: u16,
    /// ROMCTRL without the status bits (23 and 31), which are read from the transfer state.
    romctrl: u32,
    /// The command registers, first command byte first.
    command: [u8; 8],
    /// The command of the transfer running, or last run.
    cmd: u8,
    /// Where the transfer's 4 KiB block starts in the ROM.
    base: u32,
    /// The next byte's offset inside that block; the data wraps there (`cart-protocol.md`).
    pos: u32,
    /// Bytes of the transfer not yet taken.
    left: u32,
    /// A transfer is running (ROMCTRL bit 31).
    busy: bool,
    /// A word is ready to be taken (ROMCTRL bit 23).
    ready: bool,
    /// The last word taken: what the data port reads when none is ready.
    last_word: u32,
}

impl Cart {
    /// The slot at power-on: every register 0, no transfer.
    // TODO(hw-unverified): GBATEK states no reset values for these registers (`cart-protocol.md`,
    // "Unverified").
    pub fn new() -> Self {
        Self::default()
    }

    /// The slot as direct boot leaves it: the ARM9 owns it, nothing is enabled, RESB is released.
    // TODO(hw-unverified): direct boot skips the card's reset phases; these values are our
    // choice (`cart-protocol.md`, "The card after direct boot").
    pub(crate) fn direct_boot(&mut self) {
        *self = Self::new();
        self.romctrl = ROMCTRL_RESB;
    }

    /// ROMCTRL as a CPU reads it.
    pub fn romctrl(&self) -> u32 {
        self.romctrl
            | if self.ready { ROMCTRL_READY } else { 0 }
            | if self.busy { ROMCTRL_BUSY } else { 0 }
    }

    /// AUXSPICNT as a CPU reads it.
    pub fn auxspicnt(&self) -> u16 {
        self.auxspicnt
    }

    /// EXMEMCNT as the ARM9 reads it.
    pub fn exmemcnt9(&self) -> u16 {
        self.exmem::<true>()
    }

    /// EXMEMSTAT as the ARM7 reads it.
    pub fn exmemcnt7(&self) -> u16 {
        self.exmem::<false>()
    }

    /// The eight command bytes, first command byte first.
    pub fn command(&self) -> [u8; 8] {
        self.command
    }

    /// The ROM offset of the next byte the transfer hands out.
    pub fn address(&self) -> u32 {
        self.base.wrapping_add(self.pos)
    }

    /// Bytes of the transfer still to be taken.
    pub fn remaining(&self) -> u32 {
        self.left
    }

    /// Whether the ARM9 owns the slot (EXMEMCNT bit 11 clear).
    pub fn owner_is_arm9(&self) -> bool {
        self.exmem_shared & EXMEMCNT_SLOT_ARM7 == 0
    }

    fn owned_by<const ARM9: bool>(&self) -> bool {
        self.owner_is_arm9() == ARM9
    }

    fn exmem<const ARM9: bool>(&self) -> u16 {
        let own = if ARM9 { self.exmem9 } else { self.exmem7 };
        u16::from(own) | self.exmem_shared
    }

    /// A write of one byte of EXMEMCNT (`at` 0 or 1): the low 7 bits are per CPU, the rest only
    /// the ARM9 changes.
    fn write_exmem<const ARM9: bool>(&mut self, at: u32, byte: u8) {
        let own = byte & EXMEMCNT_OWN_MASK;
        match (ARM9, at) {
            (true, 0) => {
                self.exmem9 = own;
                self.exmem_shared = (self.exmem_shared & 0xFF00) | u16::from(byte & 0x80);
            }
            (true, _) => {
                let high = u16::from(byte) << 8;
                self.exmem_shared = ((self.exmem_shared & 0x00FF) | high) & EXMEMCNT_SHARED_MASK;
            }
            (false, 0) => self.exmem7 = own,
            (false, _) => {}
        }
    }

    /// The word the data port holds: the next word of the transfer when one is ready, else the
    /// last one taken.
    fn word(&self, rom: &[u8]) -> u32 {
        if !self.ready {
            return self.last_word;
        }
        match self.cmd {
            CMD_READ => u32::from_le_bytes(core::array::from_fn(|i| {
                let at = self.base.wrapping_add((self.pos + i as u32) % BLOCK_BYTES);
                protocol::byte_at(rom, at)
            })),
            CMD_CHIP_ID => protocol::chip_id(rom.len()),
            _ => UNKNOWN_WORD,
        }
    }

    /// A CPU takes the ready word: it becomes the last word, and the transfer moves on. The last
    /// word ends the transfer.
    // TODO(hw-unverified): the transfer ends, and the IRQ is raised, when the last word is taken
    // (`cart-protocol.md`, "Unverified").
    fn take(&mut self, rom: &[u8]) -> Effect {
        self.last_word = self.word(rom);
        self.left -= 4;
        self.pos = (self.pos + 4) % BLOCK_BYTES;
        if self.left == 0 {
            return self.finish();
        }
        Effect::default()
    }

    /// The transfer is done: not busy, no word ready, and IF bit 19 on the owner when AUXSPICNT
    /// bit 14 asks for it.
    // TODO(hw-unverified): the IRQ goes to the CPU that owns the slot when the transfer ends
    // (`cart-protocol.md`, "Unverified").
    fn finish(&mut self) -> Effect {
        self.busy = false;
        self.ready = false;
        if self.auxspicnt & AUXSPICNT_IRQ == 0 {
            return Effect::default();
        }
        Effect::raising(match self.owner_is_arm9() {
            true => Requests::for_cpu::<true>(CARD_TRANSFER_IRQ),
            false => Requests::for_cpu::<false>(CARD_TRANSFER_IRQ),
        })
    }

    /// A write that sets ROMCTRL bit 31 while idle: latches the command and gets the first word
    /// ready. A block size of 0 has no words and finishes at once.
    // TODO(hw-unverified): AUXSPICNT's enable bit and RESB do not gate a start, and the write
    // direction (ROMCTRL bit 30) is stored and ignored; a start while busy is ignored (`cart-protocol.md`, "Unverified").
    // TODO(phase5): the first word is ready at once and each word follows the one before with no
    // delay; the per-word timing of `cart-protocol.md`, "Timing", comes with the next PR of #37.
    // TODO(hw-unverified): the transfer does not wait for a slow reader, and the owner at the end
    // of a transfer gets the IRQ even if the slot changed hands (`cart-protocol.md`, "Unverified").
    fn start(&mut self, rom: &[u8]) -> Effect {
        self.cmd = self.command[0];
        let size = (self.romctrl >> BLOCK_SIZE_SHIFT) & BLOCK_SIZE_MASK;
        self.left = block_bytes(size);
        if self.cmd == CMD_READ {
            let address = u32::from_be_bytes([
                self.command[1],
                self.command[2],
                self.command[3],
                self.command[4],
            ]);
            let first = protocol::start_of(address, rom.len());
            self.base = first - first % BLOCK_BYTES;
            self.pos = first % BLOCK_BYTES;
        } else {
            self.base = 0;
            self.pos = 0;
        }
        self.busy = true;
        self.ready = self.left > 0;
        if self.left == 0 {
            return self.finish();
        }
        Effect::default()
    }

    /// The byte of the register space at `offset` for CPU `ARM9`.
    fn byte<const ARM9: bool>(&self, offset: u32, rom: &[u8]) -> u8 {
        match offset {
            EXMEMCNT_OFFSET | EXMEMCNT_HIGH => {
                self.exmem::<ARM9>().to_le_bytes()[(offset & 1) as usize]
            }
            // TODO(hw-unverified): the CPU that does not own the slot reads zero from its
            // registers and its writes are dropped (`cart-protocol.md`, "Ownership").
            _ if !self.owned_by::<ARM9>() => 0,
            0 | 1 => self.auxspicnt.to_le_bytes()[offset as usize],
            ROMCTRL_OFFSET..=7 => self.romctrl().to_le_bytes()[(offset - ROMCTRL_OFFSET) as usize],
            DATA_OFFSET..=DATA_LAST => {
                self.word(rom).to_le_bytes()[(offset - DATA_OFFSET) as usize]
            }
            _ => 0,
        }
    }

    /// A write of one byte at `offset` by CPU `ARM9`.
    fn write_byte<const ARM9: bool>(&mut self, offset: u32, byte: u8, rom: &[u8]) -> Effect {
        match offset {
            EXMEMCNT_OFFSET | EXMEMCNT_HIGH => {
                self.write_exmem::<ARM9>(offset & 1, byte);
                Effect::default()
            }
            _ if !self.owned_by::<ARM9>() => Effect::default(),
            0 => {
                self.auxspicnt =
                    (self.auxspicnt & 0xFF00) | u16::from(byte) & (AUXSPICNT_MASK & 0xFF);
                Effect::default()
            }
            1 => {
                self.auxspicnt =
                    (self.auxspicnt & 0x00FF) | (u16::from(byte) << 8 & AUXSPICNT_MASK);
                Effect::default()
            }
            ROMCTRL_OFFSET..=7 => self.write_romctrl_byte(offset - ROMCTRL_OFFSET, byte, rom),
            COMMAND_OFFSET..=15 => {
                self.command[(offset - COMMAND_OFFSET) as usize] = byte;
                Effect::default()
            }
            _ => Effect::default(),
        }
    }

    /// A write of byte `index` (0-3) of ROMCTRL. Byte 3 carries the start bit, and RESB can be
    /// set but not cleared.
    fn write_romctrl_byte(&mut self, index: u32, byte: u8, rom: &[u8]) -> Effect {
        let shift = 8 * index;
        let kept = self.romctrl & !(0xFF << shift);
        let mut next = kept | (u32::from(byte) << shift);
        next &= ROMCTRL_STORED;
        next |= self.romctrl & ROMCTRL_RESB;
        self.romctrl = next;
        if index == 3 && byte & 0x80 != 0 && !self.busy {
            return self.start(rom);
        }
        Effect::default()
    }
}

impl Checked for Cart {
    fn well_formed(&self, _now: u64) -> bool {
        self.pos < BLOCK_BYTES
            && self.base.is_multiple_of(BLOCK_BYTES)
            && self.left <= block_bytes(6)
            && self.left.is_multiple_of(4)
            && (!self.ready || (self.busy && self.left > 0))
            && self.auxspicnt & !AUXSPICNT_MASK == 0
            && self.romctrl & !ROMCTRL_STORED == 0
            && self.exmem9 & !EXMEMCNT_OWN_MASK == 0
            && self.exmem7 & !EXMEMCNT_OWN_MASK == 0
            && self.exmem_shared & !EXMEMCNT_SHARED_MASK == 0
    }
}

impl<const ARM9: bool> Device<ARM9> for Cart {
    fn peek(&self, offset: u32, width: Width, cx: Cx<'_>) -> u32 {
        read_bytes(offset, width, |at| self.byte::<ARM9>(at, cx.rom))
    }

    /// A read that includes the data port's last byte takes the ready word.
    fn read(&mut self, offset: u32, width: Width, cx: Cx<'_>) -> (u32, Effect) {
        let value = Device::<ARM9>::peek(self, offset, width, cx);
        let in_port = (DATA_OFFSET..=DATA_LAST).contains(&offset);
        // TODO(hw-unverified): the word is taken by a read that includes byte 3 of the port, and
        // by no other (`cart-protocol.md`, "Unverified").
        let takes =
            in_port && offset + width.bytes() > DATA_LAST && self.ready && self.owned_by::<ARM9>();
        let effect = if takes {
            self.take(cx.rom)
        } else {
            Effect::default()
        };
        (value, effect)
    }

    fn write(&mut self, offset: u32, width: Width, value: u32, cx: Cx<'_>) -> Effect {
        write_bytes(offset, width, value, |at, byte| {
            self.write_byte::<ARM9>(at, byte, cx.rom)
        })
    }
}

#[cfg(test)]
mod tests;
