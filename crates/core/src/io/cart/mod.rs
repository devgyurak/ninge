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
//!
//! The slot's SPI side is here too: AUXSPICNT's baud rate, hold and busy bits and AUXSPIDATA talk to
//! the save chip (`cart::save`, note `docs/hw/auxspi-saves.md`), which the device owns. A byte takes
//! a time that follows from the clock, so busy is computed from the access time, with no event.

use crate::cart::protocol::{self, BLOCK_BYTES};
use crate::cart::save::{SaveChip, SaveError, SaveType};
use crate::io::device::{Checked, Cx, Device, Effect, Fired, Width, read_bytes, write_bytes};
use crate::io::dma::Trigger;
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
/// AUXSPICNT bits 0-1: the SPI baud rate, 4 MHz, 2 MHz, 1 MHz, 512 kHz.
const AUXSPICNT_BAUD_MASK: u16 = 0b11;
/// AUXSPICNT bit 6: keep the chip selected after the transfer.
const AUXSPICNT_HOLD: u16 = 1 << 6;
/// AUXSPICNT bit 7: an SPI byte is on the wire. Read-only, computed from the time.
const AUXSPICNT_BUSY: u16 = 1 << 7;
/// AUXSPICNT bit 13: slot mode, 1 = SPI backup.
const AUXSPICNT_SPI_MODE: u16 = 1 << 13;
/// AUXSPICNT bit 15: slot enable.
const AUXSPICNT_ENABLE: u16 = 1 << 15;
/// The byte offset of AUXSPIDATA's low byte, the one whose write starts a transfer.
const AUXSPIDATA_OFFSET: u32 = 2;
/// Master cycles of an SPI byte at baud 0: 8 bits of 8 bus clocks, two master cycles a bus clock;
/// each baud step doubles it (`auxspi-saves.md`, "A transfer").
// TODO(hw-unverified): the divisor behind "4 MHz" and the rest is the bus clock over 8, 16, 32, 64
// (`auxspi-saves.md`, "Unverified").
const SPI_BYTE_CYCLES: u64 = 128;

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
// "Unverified").
const EXMEMCNT_SHARED_MASK: u16 = 0xE880;

/// The `B7` command: read data (`cart-protocol.md`, "Commands").
const CMD_READ: u8 = 0xB7;
/// The `B8` command: chip ID.
const CMD_CHIP_ID: u8 = 0xB8;
/// What a word of an unknown command reads.
// TODO(hw-unverified): GBATEK describes an endless stream of encrypted zero bytes; with no
// encryption to apply the model returns `FFFF_FFFFh` (`cart-protocol.md`, "Unverified").
const UNKNOWN_WORD: u32 = 0xFFFF_FFFF;

/// ROMCTRL bit 27: the transfer clock, 0 = 6.7 MHz and 1 = 4.2 MHz.
const ROMCTRL_SLOW_CLOCK: u32 = 1 << 27;
/// Master cycles of one card clock: the 33.51 MHz bus clock divided by 5 or 8, in master cycles
/// that count at twice the bus clock (`cart-protocol.md`, "Timing").
const FAST_CLOCK_CYCLES: u64 = 10;
const SLOW_CLOCK_CYCLES: u64 = 16;
/// Card clocks to send the 8-byte command, and for each 4-byte word (a byte a clock).
const COMMAND_CLOCKS: u64 = 8;
const WORD_CLOCKS: u64 = 4;
/// ROMCTRL's gap 1 (bits 0-12) and gap 2 (bits 16-21), in card clocks.
const GAP1_MASK: u32 = 0x1FFF;
const GAP2_SHIFT: u32 = 16;
const GAP2_MASK: u32 = 0x3F;
/// Gap 2 follows each 200h bytes of data (`cart-protocol.md`, "Timing").
const GAP2_EVERY: u32 = 0x200;

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
    /// The save chip on the SPI bus.
    save: SaveChip,
    /// The byte the last SPI transfer received: what AUXSPIDATA reads.
    // TODO(hw-unverified): AUXSPIDATA's reset value and its read while busy (`auxspi-saves.md`,
    // "Unverified").
    spi_in: u8,
    /// The master time the SPI byte on the wire is done; busy until then.
    spi_done: u64,
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
    /// Bytes of the transfer in all, from the block size when it started.
    total: u32,
    /// Bytes of the transfer not yet taken.
    left: u32,
    /// The master time the next word is ready (or, for a block of size 0, the transfer is done): the
    /// time of the device's one event slot. `None` while nothing is on its way.
    due: Option<u64>,
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

    /// AUXSPICNT as stored: without the busy bit, which depends on the time.
    pub fn auxspicnt(&self) -> u16 {
        self.auxspicnt
    }

    /// AUXSPICNT as a CPU reads it at master time `now`.
    pub fn auxspicnt_at(&self, now: u64) -> u16 {
        self.auxspicnt
            | if self.spi_busy(now) {
                AUXSPICNT_BUSY
            } else {
                0
            }
    }

    /// Whether an SPI byte is still on the wire at master time `now`.
    pub fn spi_busy(&self, now: u64) -> bool {
        now < self.spi_done
    }

    /// The save chip, for the debugger.
    pub fn save(&self) -> &SaveChip {
        &self.save
    }

    /// Fits a chip of `kind` holding `data` (`Machine::insert_save`).
    pub(crate) fn insert_save(&mut self, kind: SaveType, data: Vec<u8>) -> Result<(), SaveError> {
        let generation = self.save.generation();
        self.save = SaveChip::new(kind, data)?;
        self.save.set_generation(generation);
        self.spi_done = 0;
        Ok(())
    }

    /// Sets the save memory's change counter (`Machine::load_state`).
    pub(crate) fn set_save_generation(&mut self, generation: u64) {
        self.save.set_generation(generation);
    }

    /// An SPI byte written at `now`: the chip answers at once, and the byte takes its time on the
    /// wire. Nothing happens unless the slot is enabled in SPI mode, and a byte written while one is
    /// still on the wire is dropped.
    // TODO(hw-unverified): AUXSPIDATA written with the mode or the enable off, or while busy
    // (`auxspi-saves.md`, "Unverified").
    fn spi_write(&mut self, byte: u8, now: u64) {
        let want = AUXSPICNT_SPI_MODE | AUXSPICNT_ENABLE;
        if self.auxspicnt & want != want || self.spi_busy(now) {
            return;
        }
        let hold = self.auxspicnt & AUXSPICNT_HOLD != 0;
        self.spi_in = self.save.transfer(byte, hold);
        let baud = u32::from(self.auxspicnt & AUXSPICNT_BAUD_MASK);
        self.spi_done = now.saturating_add(SPI_BYTE_CYCLES << baud);
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

    /// The command of the transfer running, or the last one run.
    pub fn current_command(&self) -> u8 {
        self.cmd
    }

    /// The master time the next word is due, if one is on its way: the model's time, which the
    /// scheduler's event can trail when an ARM7 access queued it while the ARM7's clock lagged.
    pub fn next_word_due(&self) -> Option<u64> {
        self.due
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

    /// Whether a word is waiting for CPU `ARM9`: it owns the slot and the word is ready. The DMA
    /// channels that start on the slot take it as a request (`cart-protocol.md`, "DMA start").
    pub(crate) fn word_waiting<const ARM9: bool>(&self) -> bool {
        self.ready && self.owned_by::<ARM9>()
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
    // TODO(hw-unverified): with no word ready the data port reads the last word taken, zero before any
    // (`cart-protocol.md`, "Unverified").
    fn word(&self, rom: &[u8]) -> u32 {
        if !self.ready {
            return self.last_word;
        }
        match self.cmd {
            CMD_READ => u32::from_le_bytes(core::array::from_fn(|i| {
                let at = self.base.wrapping_add((self.pos + i as u32) % BLOCK_BYTES);
                protocol::byte_at(rom, at)
            })),
            CMD_CHIP_ID => protocol::chip_id(rom),
            _ => UNKNOWN_WORD,
        }
    }

    /// A CPU takes the ready word: it becomes the last word, and the transfer moves on. The last
    /// word ends the transfer.
    // TODO(hw-unverified): the transfer ends, and the IRQ is raised, when the last word is taken
    // (`cart-protocol.md`, "Unverified").
    // TODO(hw-unverified): the next word is scheduled when this one is taken, with gap 2 after each
    // 200h bytes; the constants and the wait for a slow reader are in `cart-protocol.md`,
    // "Unverified".
    fn take(&mut self, rom: &[u8], now: u64) -> Effect {
        self.last_word = self.word(rom);
        self.left -= 4;
        self.pos = (self.pos + 4) % BLOCK_BYTES;
        self.ready = false;
        if self.left == 0 {
            return self.finish();
        }
        // The next word comes a word's time after this one was taken, and the gap 2 of the whole
        // 200h bytes later when this one completed them.
        let taken = self.total - self.left;
        let gap = if taken.is_multiple_of(GAP2_EVERY) {
            self.gap2()
        } else {
            0
        };
        self.schedule(now, WORD_CLOCKS + gap)
    }

    /// The card clock in master cycles, as ROMCTRL bit 27 has it now.
    fn clock_cycles(&self) -> u64 {
        if self.romctrl & ROMCTRL_SLOW_CLOCK != 0 {
            SLOW_CLOCK_CYCLES
        } else {
            FAST_CLOCK_CYCLES
        }
    }

    /// ROMCTRL's gap 1 in card clocks.
    fn gap1(&self) -> u64 {
        u64::from(self.romctrl & GAP1_MASK)
    }

    /// ROMCTRL's gap 2 in card clocks.
    fn gap2(&self) -> u64 {
        u64::from((self.romctrl >> GAP2_SHIFT) & GAP2_MASK)
    }

    /// Queues the device's event `clocks` card clocks after `now`; the effect names the slot so `Io`
    /// replaces the scheduler's event.
    fn schedule(&mut self, now: u64, clocks: u64) -> Effect {
        self.due = Some(now.saturating_add(clocks * self.clock_cycles()));
        Effect {
            reschedule: 1,
            ..Effect::default()
        }
    }

    /// The transfer is done: not busy, no word ready, and IF bit 19 on the owner when AUXSPICNT
    /// bit 14 asks for it.
    // TODO(hw-unverified): the IRQ goes to the CPU that owns the slot when the transfer ends
    // (`cart-protocol.md`, "Unverified").
    // TODO(hw-unverified): IREQ_MC (IF bit 20) is not modeled (`cart-protocol.md`, "Unverified").
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
    // TODO(hw-unverified): the timing constants (an 8-clock command, the gap placement, a byte a
    // clock) are our reading of GBATEK (`cart-protocol.md`, "Timing", "Unverified").
    // TODO(hw-unverified): the transfer does not wait for a slow reader, and the owner at the end
    // of a transfer gets the IRQ even if the slot changed hands (`cart-protocol.md`, "Unverified").
    fn start(&mut self, rom: &[u8], now: u64) -> Effect {
        self.cmd = self.command[0];
        let size = (self.romctrl >> BLOCK_SIZE_SHIFT) & BLOCK_SIZE_MASK;
        self.total = block_bytes(size);
        self.left = self.total;
        if self.cmd == CMD_READ {
            let address = u32::from_be_bytes([
                self.command[1],
                self.command[2],
                self.command[3],
                self.command[4],
            ]);
            let first = protocol::start_of(address, rom);
            self.base = first - first % BLOCK_BYTES;
            self.pos = first % BLOCK_BYTES;
        } else {
            self.base = 0;
            self.pos = 0;
        }
        self.busy = true;
        self.ready = false;
        // The first word is ready after the command, the leading gap and one word's time; a block
        // of size 0 has no word, so the transfer is done after the command and the gap.
        let word = if self.left > 0 { WORD_CLOCKS } else { 0 };
        self.schedule(now, COMMAND_CLOCKS + self.gap1() + word)
    }

    /// The byte of the register space at `offset` for CPU `ARM9`.
    fn byte<const ARM9: bool>(&self, offset: u32, cx: Cx<'_>) -> u8 {
        match offset {
            EXMEMCNT_OFFSET | EXMEMCNT_HIGH => {
                self.exmem::<ARM9>().to_le_bytes()[(offset & 1) as usize]
            }
            // TODO(hw-unverified): the CPU that does not own the slot reads zero from its
            // registers and its writes are dropped (`cart-protocol.md`, "Ownership").
            _ if !self.owned_by::<ARM9>() => 0,
            0 | 1 => self.auxspicnt_at(cx.now).to_le_bytes()[offset as usize],
            AUXSPIDATA_OFFSET => self.spi_in,
            ROMCTRL_OFFSET..=7 => self.romctrl().to_le_bytes()[(offset - ROMCTRL_OFFSET) as usize],
            DATA_OFFSET..=DATA_LAST => {
                self.word(cx.rom).to_le_bytes()[(offset - DATA_OFFSET) as usize]
            }
            _ => 0,
        }
    }

    /// A write of one byte at `offset` by CPU `ARM9`.
    fn write_byte<const ARM9: bool>(&mut self, offset: u32, byte: u8, cx: Cx<'_>) -> Effect {
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
            AUXSPIDATA_OFFSET => {
                self.spi_write(byte, cx.now);
                Effect::default()
            }
            ROMCTRL_OFFSET..=7 => self.write_romctrl_byte(offset - ROMCTRL_OFFSET, byte, cx),
            COMMAND_OFFSET..=15 => {
                self.command[(offset - COMMAND_OFFSET) as usize] = byte;
                Effect::default()
            }
            _ => Effect::default(),
        }
    }

    /// A write of byte `index` (0-3) of ROMCTRL. Byte 3 carries the start bit, and RESB can be
    /// set but not cleared.
    fn write_romctrl_byte(&mut self, index: u32, byte: u8, cx: Cx<'_>) -> Effect {
        let shift = 8 * index;
        let kept = self.romctrl & !(0xFF << shift);
        let mut next = kept | (u32::from(byte) << shift);
        next &= ROMCTRL_STORED;
        next |= self.romctrl & ROMCTRL_RESB;
        self.romctrl = next;
        if index == 3 && byte & 0x80 != 0 && !self.busy {
            return self.start(cx.rom, cx.now);
        }
        Effect::default()
    }
}

impl Checked for Cart {
    fn well_formed(&self, now: u64) -> bool {
        self.pos < BLOCK_BYTES
            && self.base.is_multiple_of(BLOCK_BYTES)
            && self.left <= block_bytes(6)
            && self.left.is_multiple_of(4)
            && (!self.ready || (self.busy && self.left > 0 && self.due.is_none()))
            && (self.due.is_none() || (self.busy && !self.ready))
            && (!self.busy || self.ready || self.due.is_some())
            && self.total <= block_bytes(6)
            && self.total.is_multiple_of(4)
            && (self.busy || self.left == 0)
            && self.left <= self.total
            && self.save.well_formed()
            && self.spi_done <= now.saturating_add(SPI_BYTE_CYCLES << AUXSPICNT_BAUD_MASK)
            && self.auxspicnt & !AUXSPICNT_MASK == 0
            && self.romctrl & !ROMCTRL_STORED == 0
            && self.exmem9 & !EXMEMCNT_OWN_MASK == 0
            && self.exmem7 & !EXMEMCNT_OWN_MASK == 0
            && self.exmem_shared & !EXMEMCNT_SHARED_MASK == 0
    }
}

impl<const ARM9: bool> Device<ARM9> for Cart {
    fn peek(&self, offset: u32, width: Width, cx: Cx<'_>) -> u32 {
        read_bytes(offset, width, |at| self.byte::<ARM9>(at, cx))
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
            self.take(cx.rom, cx.now)
        } else {
            Effect::default()
        };
        (value, effect)
    }

    fn write(&mut self, offset: u32, width: Width, value: u32, cx: Cx<'_>) -> Effect {
        write_bytes(offset, width, value, |at, byte| {
            self.write_byte::<ARM9>(at, byte, cx)
        })
    }

    /// The one slot: the next word, or the end of a block of size 0.
    const SLOTS: u8 = 1;

    fn slot_due(&self, _slot: u8) -> Option<u64> {
        self.due
    }

    /// The word is ready: the owner's DMA channels waiting for the slot get a request. A block of
    /// size 0 ends here instead.
    // TODO(hw-unverified): a channel enabled while a word waits takes it at once, as if the request
    // were a level (`Io::write`; `dma.md`, "Unverified").
    // TODO(hw-unverified): the card's event sorts before the timers and DMA and is the ARM9's
    // whichever CPU queued it, our choice of tie order (`scheduler.md`, "Unverified").
    fn slot_fire(&mut self, _slot: u8, _now: u64) -> Fired {
        if self.due.take().is_none() {
            return Fired::default();
        }
        if self.left == 0 {
            return Fired {
                effect: self.finish(),
                ..Fired::default()
            };
        }
        self.ready = true;
        Fired {
            trigger: Some(Trigger::CardWord {
                arm9: self.owner_is_arm9(),
            }),
            ..Fired::default()
        }
    }
}

#[cfg(test)]
mod tests;
