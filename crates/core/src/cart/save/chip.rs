// SPDX-License-Identifier: Apache-2.0
//! The byte state machine of a save chip. Note: `docs/hw/auxspi-saves.md`.

use super::{SaveError, SaveType};
use serde::{Deserialize, Serialize};

const CMD_WRSR: u8 = 0x01;
const CMD_PP_OR_WRITE: u8 = 0x02;
const CMD_READ: u8 = 0x03;
const CMD_WRDI: u8 = 0x04;
const CMD_RDSR: u8 = 0x05;
const CMD_WREN: u8 = 0x06;
const CMD_WRITE_HI: u8 = 0x0A;
const CMD_READ_HI: u8 = 0x0B;
const CMD_RDID: u8 = 0x9F;
const CMD_PAGE_ERASE: u8 = 0xDB;
const CMD_SECTOR_ERASE: u8 = 0xD8;
const CMD_POWER_DOWN: u8 = 0xB9;
const CMD_RELEASE: u8 = 0xAB;

/// What the chip drives while it has nothing to say: the output line floats high.
// TODO(hw-unverified): what a chip answers outside read data and status (`auxspi-saves.md`,
// "Unverified").
const IDLE: u8 = 0xFF;

/// Status bit 1: the write enable latch.
const STATUS_WEL: u8 = 1 << 1;
/// Status bits 2-3: the write-protect level.
const STATUS_WP_SHIFT: u32 = 2;
const STATUS_WP_MASK: u8 = 0b11;
/// Status bit 7 of the 8 KiB and larger EEPROMs and the FRAMs: the status register write disable.
const STATUS_SRWD: u8 = 1 << 7;
/// Status bits 4-7 of the 512-byte EEPROM, which always read 1.
const STATUS_EEPROM512_ONES: u8 = 0xF0;

/// A FLASH page and sector, in bytes.
const FLASH_PAGE: usize = 256;
const FLASH_SECTOR: usize = 0x1_0000;

/// What a session's command does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
enum Op {
    /// A command the chip does not have: the rest of the session is ignored. This includes WRSR on a
    /// FLASH.
    // TODO(hw-unverified): WRSR on a FLASH is ignored (`auxspi-saves.md`, "Unverified").
    #[default]
    Ignore,
    Wren,
    Wrdi,
    Rdsr,
    Wrsr,
    Rdid,
    Read,
    FastRead,
    /// Overwrites bytes inside a page (the EEPROM and FRAM write, the FLASH page write). With no data
    /// byte it changes nothing.
    // TODO(hw-unverified): a page write with no data byte erases nothing, and bytes after a page or
    // sector erase address are ignored (`auxspi-saves.md`, "Unverified").
    Write,
    /// Clears bits inside a page (the FLASH page program).
    Program,
    PageErase,
    SectorErase,
    /// Accepted, no effect (FLASH deep power-down and its release).
    // TODO(hw-unverified): the chip does not ignore instructions while powered down
    // (`auxspi-saves.md`, "Unverified").
    Nothing,
}

/// The chip's view of the command a session carries: which command, and where it is in it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Session {
    /// The command byte.
    cmd: u8,
    op: Op,
    /// The 512-byte EEPROM's ninth address bit, which the command carries.
    hi: bool,
    /// Bytes received after the command.
    got: u32,
    /// The address counter.
    addr: u32,
    /// The byte a WRSR carries.
    value: u8,
    /// The command got far enough to take effect when the session ends (it has its data byte, or
    /// for an erase its whole address).
    touched: bool,
}

/// A save chip: its type, its memory and the session on the bus. Plain data; the memory is the
/// save file's bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveChip {
    kind: SaveType,
    memory: Vec<u8>,
    session: Option<Session>,
    /// The write enable latch.
    wel: bool,
    /// The write-protect level (status bits 2-3).
    wp: u8,
    /// The status register write disable bit.
    srwd: bool,
    /// Counts the changes to the memory, for the host. Host bookkeeping: not in a savestate.
    #[serde(skip)]
    generation: Generation,
}

/// The change counter. It is no part of the chip's state, so two chips are equal whatever their
/// counters say (a loaded savestate equals the machine it was saved from).
#[derive(Clone, Copy, Debug, Default)]
struct Generation(u64);

impl PartialEq for Generation {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for Generation {}

/// The command at `cmd` on a chip of `kind`, and whether it is the 512-byte EEPROM's upper half.
fn decode(kind: SaveType, cmd: u8) -> (Op, bool) {
    let common = match cmd {
        CMD_WREN => Op::Wren,
        CMD_WRDI => Op::Wrdi,
        CMD_RDSR => Op::Rdsr,
        CMD_RDID => Op::Rdid,
        _ => Op::Ignore,
    };
    match kind {
        SaveType::None => (Op::Ignore, false),
        _ if common != Op::Ignore => (common, false),
        SaveType::Eeprom512 => match cmd {
            CMD_READ => (Op::Read, false),
            CMD_READ_HI => (Op::Read, true),
            CMD_PP_OR_WRITE => (Op::Write, false),
            CMD_WRITE_HI => (Op::Write, true),
            CMD_WRSR => (Op::Wrsr, false),
            _ => (Op::Ignore, false),
        },
        SaveType::Eeprom8K
        | SaveType::Eeprom64K
        | SaveType::Eeprom128K
        | SaveType::Fram8K
        | SaveType::Fram32K => match cmd {
            CMD_READ => (Op::Read, false),
            CMD_PP_OR_WRITE => (Op::Write, false),
            CMD_WRSR => (Op::Wrsr, false),
            _ => (Op::Ignore, false),
        },
        SaveType::Flash256K | SaveType::Flash512K | SaveType::Flash1M | SaveType::Flash8M => {
            match cmd {
                CMD_READ => (Op::Read, false),
                CMD_READ_HI => (Op::FastRead, false),
                CMD_WRITE_HI => (Op::Write, false),
                CMD_PP_OR_WRITE => (Op::Program, false),
                CMD_PAGE_ERASE => (Op::PageErase, false),
                CMD_SECTOR_ERASE => (Op::SectorErase, false),
                CMD_POWER_DOWN | CMD_RELEASE => (Op::Nothing, false),
                _ => (Op::Ignore, false),
            }
        }
    }
}

/// Address bytes after the command (`auxspi-saves.md`, "EEPROM and FRAM", "FLASH").
// TODO(hw-unverified): the 128K EEPROM's address width (`auxspi-saves.md`, "Unverified").
fn address_bytes(kind: SaveType) -> u32 {
    match kind {
        SaveType::None => 0,
        SaveType::Eeprom512 => 1,
        SaveType::Eeprom8K | SaveType::Eeprom64K | SaveType::Fram8K | SaveType::Fram32K => 2,
        _ => 3,
    }
}

/// Bytes in a write's page, which the address wraps inside. A FRAM has no page, so its page is the
/// whole memory.
// TODO(hw-unverified): the 128K EEPROM's page size, and a write past a page's end
// (`auxspi-saves.md`, "Unverified").
fn page_bytes(kind: SaveType) -> usize {
    match kind {
        SaveType::Eeprom512 => 16,
        SaveType::Eeprom8K => 32,
        SaveType::Eeprom64K => 128,
        SaveType::Eeprom128K => 256,
        SaveType::Flash256K | SaveType::Flash512K | SaveType::Flash1M | SaveType::Flash8M => {
            FLASH_PAGE
        }
        SaveType::Fram8K | SaveType::Fram32K => kind.len(),
        SaveType::None => 1,
    }
}

impl SaveChip {
    /// A chip of `kind` holding `data`, which must be as long as the chip's memory.
    pub fn new(kind: SaveType, data: Vec<u8>) -> Result<Self, SaveError> {
        if data.len() != kind.len() {
            return Err(SaveError::BadLength {
                kind,
                expected: kind.len(),
                found: data.len(),
            });
        }
        Ok(Self {
            kind,
            memory: data,
            ..Self::default()
        })
    }

    /// A chip of `kind` with erased memory (every byte `FFh`).
    pub fn blank(kind: SaveType) -> Self {
        Self {
            kind,
            memory: vec![0xFF; kind.len()],
            ..Self::default()
        }
    }

    /// The chip's type.
    pub fn kind(&self) -> SaveType {
        self.kind
    }

    /// The chip's memory, as a save file holds it.
    pub fn data(&self) -> &[u8] {
        &self.memory
    }

    /// Changes to the memory so far. Not part of a savestate.
    pub fn generation(&self) -> u64 {
        self.generation.0
    }

    /// Sets the change counter, which the machine keeps rising across a savestate load.
    pub(crate) fn set_generation(&mut self, generation: u64) {
        self.generation = Generation(generation);
    }

    /// Whether a session is open (the chip is selected).
    pub fn selected(&self) -> bool {
        self.session.is_some()
    }

    /// The command byte of the open session.
    pub fn command(&self) -> Option<u8> {
        self.session.as_ref().map(|s| s.cmd)
    }

    /// The address counter of the open session; 0 when none is open.
    pub fn address(&self) -> u32 {
        self.session.as_ref().map_or(0, |s| s.addr)
    }

    /// The status register as RDSR reads it.
    pub fn status(&self) -> u8 {
        let wel = if self.wel { STATUS_WEL } else { 0 };
        let wp = self.wp << STATUS_WP_SHIFT;
        match self.kind {
            SaveType::None => IDLE,
            SaveType::Eeprom512 => STATUS_EEPROM512_ONES | wel | wp,
            SaveType::Flash256K | SaveType::Flash512K | SaveType::Flash1M | SaveType::Flash8M => {
                wel
            }
            _ => wel | wp | if self.srwd { STATUS_SRWD } else { 0 },
        }
    }

    /// Whether the memory and the registers hold only values the chip can reach.
    pub(crate) fn well_formed(&self) -> bool {
        self.memory.len() == self.kind.len()
            && self.wp <= STATUS_WP_MASK
            // An open session's operation is what its command means on this chip.
            && self
                .session
                .as_ref()
                .is_none_or(|s| (s.op, s.hi) == decode(self.kind, s.cmd))
    }

    /// One byte over the bus: `byte` goes in, the reply comes out. With `hold` clear the chip is
    /// deselected after the byte, which ends the session and lets its command take effect.
    pub fn transfer(&mut self, byte: u8, hold: bool) -> u8 {
        let reply = self.exchange(byte);
        if !hold {
            self.deselect();
        }
        reply
    }

    fn exchange(&mut self, byte: u8) -> u8 {
        let Some(mut session) = self.session.take() else {
            let (op, hi) = decode(self.kind, byte);
            self.session = Some(Session {
                cmd: byte,
                op,
                hi,
                ..Session::default()
            });
            return IDLE;
        };
        let reply = self.step(&mut session, byte);
        self.session = Some(session);
        reply
    }

    /// The chip's ID bytes, which RDID returns in order; EEPROM and FRAM have none.
    // TODO(hw-unverified): the FLASH ID codes, and the bytes after the third (`auxspi-saves.md`,
    // "Unverified").
    fn id(&self) -> &'static [u8] {
        match self.kind {
            SaveType::Flash256K => &[0x20, 0x40, 0x12],
            SaveType::Flash512K => &[0x20, 0x40, 0x13],
            SaveType::Flash1M => &[0x20, 0x40, 0x14],
            SaveType::Flash8M => &[0xC2, 0x20, 0x17],
            _ => &[],
        }
    }

    /// The memory's address mask: its size is a power of two, so an address past the end drops
    /// its high bits (`auxspi-saves.md`, "EEPROM and FRAM").
    // TODO(hw-unverified): mirroring past the chip's size (`auxspi-saves.md`, "Unverified").
    fn mask(&self) -> usize {
        self.memory.len().saturating_sub(1)
    }

    // TODO(hw-unverified): the read address counter wraps to 0 at the end of the chip, and a FRAM
    // write has no page (`auxspi-saves.md`, "Unverified").
    fn step(&mut self, s: &mut Session, byte: u8) -> u8 {
        let index = s.got;
        s.got = s.got.saturating_add(1);
        let n = address_bytes(self.kind);
        match s.op {
            Op::Ignore | Op::Wren | Op::Wrdi | Op::Nothing => IDLE,
            Op::Rdsr => self.status(),
            Op::Rdid => self.id().get(index as usize).copied().unwrap_or(IDLE),
            Op::Wrsr => {
                if index == 0 {
                    s.value = byte;
                    s.touched = true;
                }
                IDLE
            }
            _ if index < n => {
                s.addr = if self.kind == SaveType::Eeprom512 {
                    u32::from(byte) | u32::from(s.hi) << 8
                } else {
                    s.addr << 8 | u32::from(byte)
                };
                s.touched = index + 1 == n && matches!(s.op, Op::PageErase | Op::SectorErase);
                IDLE
            }
            // The fast read's dummy byte sits between the address and the data.
            Op::FastRead if index == n => IDLE,
            // TODO(hw-unverified): the 512-byte EEPROM's read counter keeps going past its 256-byte
            // half (`auxspi-saves.md`, "Unverified").
            Op::Read | Op::FastRead => {
                let at = s.addr as usize & self.mask();
                s.addr = (at as u32 + 1) & self.mask() as u32;
                self.memory.get(at).copied().unwrap_or(IDLE)
            }
            Op::Write | Op::Program => {
                s.touched = true;
                let at = s.addr as usize & self.mask();
                if self.wel && !self.protected(at) {
                    let old = self.memory.get(at).copied().unwrap_or(IDLE);
                    let new = if s.op == Op::Program {
                        old & byte
                    } else {
                        byte
                    };
                    self.store(at, new);
                }
                // The address wraps inside the page: the bits above it stay.
                let page = page_bytes(self.kind);
                s.addr = ((at & !(page - 1)) | ((at + 1) & (page - 1))) as u32;
                IDLE
            }
            Op::PageErase | Op::SectorErase => IDLE,
        }
    }

    /// Whether the write-protect level covers memory address `at`: EEPROM and FRAM only.
    // TODO(hw-unverified): a write into the protected region (`auxspi-saves.md`, "Unverified").
    fn protected(&self, at: usize) -> bool {
        let len = self.memory.len();
        let is_flash = matches!(
            self.kind,
            SaveType::Flash256K | SaveType::Flash512K | SaveType::Flash1M | SaveType::Flash8M
        );
        match self.wp {
            _ if is_flash => false,
            0 => false,
            1 => at >= len - len / 4,
            2 => at >= len / 2,
            _ => true,
        }
    }

    fn store(&mut self, at: usize, value: u8) {
        if let Some(byte) = self.memory.get_mut(at)
            && *byte != value
        {
            *byte = value;
            self.generation = Generation(self.generation.0.wrapping_add(1));
        }
    }

    /// Sets `size` bytes from the one holding `at`, aligned down to `size`, to `FFh`.
    fn erase(&mut self, at: usize, size: usize) {
        let first = at & !(size - 1);
        (first..first + size).for_each(|i| self.store(i, 0xFF));
    }

    /// Chip select goes high: the open session's command takes effect.
    // TODO(hw-unverified): WEL and the write-protect bits change when the session ends, WRSR needs
    // WEL, a session cut short changes nothing and an unknown command ignores its session
    // (`auxspi-saves.md`, "Unverified").
    fn deselect(&mut self) {
        let Some(s) = self.session.take() else {
            return;
        };
        match s.op {
            Op::Wren => self.wel = true,
            Op::Wrdi => self.wel = false,
            Op::Wrsr if s.touched => {
                if self.wel {
                    self.wp = (s.value >> STATUS_WP_SHIFT) & STATUS_WP_MASK;
                    self.srwd = self.kind != SaveType::Eeprom512 && s.value & STATUS_SRWD != 0;
                }
                self.wel = false;
            }
            Op::Write | Op::Program if s.touched => self.wel = false,
            Op::PageErase | Op::SectorErase if s.touched => {
                if self.wel {
                    let size = if s.op == Op::PageErase {
                        FLASH_PAGE
                    } else {
                        FLASH_SECTOR
                    };
                    self.erase(s.addr as usize & self.mask(), size);
                }
                self.wel = false;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
impl SaveChip {
    /// Makes the open session's operation one its command does not mean, for the `well_formed` test.
    pub(super) fn corrupt_session_op(&mut self) {
        if let Some(session) = &mut self.session {
            session.op = Op::PageErase;
        }
    }
}
