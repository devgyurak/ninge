// SPDX-License-Identifier: Apache-2.0
//! The memory the ARM9 sees: main RAM, the two TCMs, VRAM bank A as LCDC, and the page table
//! that finds them (`docs/hw/arm9-memory-map.md`, `docs/hw/vram-banks.md`).
//!
//! Access order: TCM, then the page table, then the slow path (I/O and unmapped). Guest
//! addresses never panic: an address that nothing answers reads 0 and ignores a write.

use super::TcmConfig;
use crate::cpu::Tcm;
use serde::{Deserialize, Serialize};

/// Page size of the fast-path table: 16 KiB (AGENTS.md §2 rule 9).
const PAGE_SHIFT: u32 = 14;
const PAGE_SIZE: usize = 1 << PAGE_SHIFT;
const PAGE_MASK: u32 = (1 << PAGE_SHIFT) - 1;
/// One entry per page of the 4 GiB address space.
const PAGE_COUNT: usize = 1 << (32 - PAGE_SHIFT);

/// Main RAM: 4 MiB at `0200_0000h` (`arm9-memory-map.md`, "Main RAM").
const MAIN_RAM_BASE: u32 = 0x0200_0000;
const MAIN_RAM_SIZE: usize = 4 << 20;
/// The 16 MiB block the main RAM repeats through.
const MAIN_RAM_BLOCK_SIZE: usize = 16 << 20;
/// Physical TCM sizes: 32 KiB ITCM, 16 KiB DTCM (`cp15.md`, "C0,C0,0 / 1 / 2").
const ITCM_SIZE: usize = 32 << 10;
const DTCM_SIZE: usize = 16 << 10;

/// VRAM bank A, mapped as LCDC at `0680_0000h`, 128 KiB (`vram-banks.md`, "Behavior").
const VRAM_A_BASE: u32 = 0x0680_0000;
const VRAM_A_SIZE: usize = 128 << 10;
/// VRAMCNT_A, at `0400_0240h` (`vram-banks.md`, "Registers").
const VRAMCNT_A_ADDR: u32 = 0x0400_0240;
/// VRAMCNT_A bits 0-1: MST. Bit 2 is not used by bank A.
const VRAMCNT_MST_MASK: u8 = 0x03;
/// VRAMCNT_A bits 3-4: OFS.
const VRAMCNT_OFS_SHIFT: u32 = 3;
const VRAMCNT_OFS_MASK: u8 = 0x03;
/// VRAMCNT_A bit 7: enable.
const VRAMCNT_ENABLE: u8 = 0x80;
/// The bits of VRAMCNT_A that are kept: MST (0-1), OFS (3-4), enable (7).
const VRAMCNT_USED: u8 =
    VRAMCNT_MST_MASK | (VRAMCNT_OFS_MASK << VRAMCNT_OFS_SHIFT) | VRAMCNT_ENABLE;
/// The MST value that maps bank A for plain CPU access (LCDC mode).
const MST_LCDC: u8 = 0;

/// Which memory a page of the address space is backed by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    /// Nothing answers on the fast path; I/O and unmapped addresses use the slow path.
    Unmapped,
    /// Main RAM.
    MainRam,
    /// VRAM bank A, mapped as LCDC.
    VramA,
}

/// VRAMCNT_A as decoded (`vram-banks.md`, "VRAMCNT_A bits").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VramBankState {
    /// Bit 7.
    pub enabled: bool,
    /// Bits 0-1.
    pub mst: u8,
    /// Bits 3-4.
    pub offset: u8,
}

/// One page-table entry: a region and an offset into that region's buffer, never a pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Page {
    region: Region,
    /// Offset in the region's buffer of the first byte of this page.
    offset: u32,
}

const UNMAPPED_PAGE: Page = Page {
    region: Region::Unmapped,
    offset: 0,
};

/// The ARM9-visible memory. Plain data: the buffers are allocated once in [`Memory::new`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Memory {
    main_ram: Vec<u8>,
    itcm: Vec<u8>,
    dtcm: Vec<u8>,
    vram_a: Vec<u8>,
    vramcnt_a: u8,
    // Derived from `vramcnt_a`, so it is not saved; `build_page_table` rebuilds it. Empty after
    // a load until then, which reads as all-unmapped rather than panicking. One entry per 16 KiB
    // page (the page-table `decision` issue).
    #[serde(skip)]
    pages: Vec<Page>,
}

/// The bytes of `buf` at `offset`, or zeros when the range is outside it (an invariant we
/// own, kept total so a guest address cannot panic).
fn bytes_at<const N: usize>(buf: &[u8], offset: usize) -> [u8; N] {
    buf.get(offset..offset.saturating_add(N))
        .and_then(|bytes| bytes.try_into().ok())
        .unwrap_or([0; N])
}

/// Copies `bytes` into `buf` at `offset`; a range outside the buffer is dropped.
fn put_bytes<const N: usize>(buf: &mut [u8], offset: usize, bytes: [u8; N]) {
    if let Some(slot) = buf.get_mut(offset..offset.saturating_add(N)) {
        slot.copy_from_slice(&bytes);
    }
}

/// Where a TCM holds `addr`: the offset into its buffer, if the TCM is enabled and the address
/// is inside its region. The TCM repeats through the region, so the offset is taken modulo the
/// physical size (`arm9-memory-map.md`, "Tightly coupled memory").
// TODO(hw-unverified): a size field outside 3-23 and an unaligned base follow `cp15.md` as
// written (`arm9-memory-map.md`, "Unverified").
fn tcm_offset(tcm: &Tcm, physical: usize, addr: u32) -> Option<usize> {
    if !tcm.enabled {
        return None;
    }
    let delta = u64::from(addr).checked_sub(u64::from(tcm.base))?;
    if delta >= tcm.size {
        return None;
    }
    usize::try_from(delta % physical as u64).ok()
}

impl Memory {
    /// Memory in its power-on state.
    // TODO(hw-unverified): all RAM starts at zero and VRAMCNT_A starts at 0, which GBATEK does
    // not state (`arm9-memory-map.md`, "Unverified"; `vram-banks.md`, "Unverified").
    pub fn new() -> Self {
        let mut memory = Self {
            main_ram: vec![0; MAIN_RAM_SIZE],
            itcm: vec![0; ITCM_SIZE],
            dtcm: vec![0; DTCM_SIZE],
            vram_a: vec![0; VRAM_A_SIZE],
            vramcnt_a: 0,
            pages: Vec::new(),
        };
        memory.build_page_table();
        memory
    }

    /// Rebuilds the page table from the mapping state. Called at construction, after every
    /// mapping change (a VRAMCNT_A write), and after a savestate load (`savestate`).
    // TODO(phase3): shared WRAM and the BIOS join the table (#22, #27).
    // TODO(phase4): the other VRAM banks and MST values join it (#35).
    // TODO(hw-unverified): main RAM mirrors through its whole 16 MiB block, GBATEK's general
    // rule; a disabled bank or MST other than 0 is unmapped; the LCDC window has no mirrors
    // (`arm9-memory-map.md`, `vram-banks.md`, "Unverified").
    pub(crate) fn build_page_table(&mut self) {
        self.pages.clear();
        self.pages.resize(PAGE_COUNT, UNMAPPED_PAGE);
        let ram_pages = MAIN_RAM_SIZE / PAGE_SIZE;
        let first = (MAIN_RAM_BASE >> PAGE_SHIFT) as usize;
        for i in 0..MAIN_RAM_BLOCK_SIZE / PAGE_SIZE {
            let offset = (i % ram_pages) * PAGE_SIZE;
            self.pages[first + i] = Page {
                region: Region::MainRam,
                offset: offset as u32,
            };
        }
        let bank = self.vram_a();
        if bank.enabled && bank.mst == MST_LCDC {
            let first = (VRAM_A_BASE >> PAGE_SHIFT) as usize;
            for i in 0..VRAM_A_SIZE / PAGE_SIZE {
                self.pages[first + i] = Page {
                    region: Region::VramA,
                    offset: (i * PAGE_SIZE) as u32,
                };
            }
        }
    }

    /// The region backing the page that holds `addr`.
    pub fn region(&self, addr: u32) -> Region {
        self.page(addr).region
    }

    /// VRAMCNT_A as decoded.
    pub fn vram_a(&self) -> VramBankState {
        VramBankState {
            enabled: self.vramcnt_a & VRAMCNT_ENABLE != 0,
            mst: self.vramcnt_a & VRAMCNT_MST_MASK,
            offset: (self.vramcnt_a >> VRAMCNT_OFS_SHIFT) & VRAMCNT_OFS_MASK,
        }
    }

    fn page(&self, addr: u32) -> Page {
        self.pages
            .get((addr >> PAGE_SHIFT) as usize)
            .copied()
            .unwrap_or(UNMAPPED_PAGE)
    }

    /// The bytes behind the page table for an access of `N` bytes at the aligned `addr`.
    fn read_page<const N: usize>(&self, addr: u32) -> [u8; N] {
        let page = self.page(addr);
        let offset = page.offset as usize + (addr & PAGE_MASK) as usize;
        match page.region {
            Region::MainRam => bytes_at(&self.main_ram, offset),
            Region::VramA => bytes_at(&self.vram_a, offset),
            Region::Unmapped => Self::read_slow(),
        }
    }

    /// The slow path for a read: I/O ports and unmapped addresses.
    // TODO(hw-unverified): an undefined I/O port reads 0 (GBATEK says so); unmapped memory is
    // "undefined" there and reads 0 here; VRAMCNT_A is write-only and reads 0
    // (`arm9-memory-map.md`, "Unverified").
    // TODO(hw-unverified): the I/O block is not mirrored (`arm9-memory-map.md`, "Unverified").
    // TODO(phase3): the I/O registers of the other blocks arrive with their phases.
    fn read_slow<const N: usize>() -> [u8; N] {
        [0; N]
    }

    /// The bytes an ARM9 data read of `N` bytes at `addr` sees. `addr` is aligned down to `N`,
    /// the way the memory system ignores the bits below the access width.
    fn load<const N: usize>(&self, tcm: &TcmConfig, addr: u32) -> [u8; N] {
        let addr = addr & !(N as u32 - 1);
        // The ITCM has priority over the DTCM; in load mode it claims the address but only
        // writes reach it, so the read goes underneath.
        // TODO(hw-unverified): GBATEK says the ITCM "appears to have priority"; taken as certain.
        // TODO(hw-unverified): a load-mode ITCM over an enabled DTCM reads underneath, not the
        // DTCM (`arm9-memory-map.md`, "Unverified").
        match tcm_offset(&tcm.itcm, ITCM_SIZE, addr) {
            Some(offset) if !tcm.itcm.load_mode => bytes_at(&self.itcm, offset),
            Some(_) => self.read_page(addr),
            None => match tcm_offset(&tcm.dtcm, DTCM_SIZE, addr) {
                Some(offset) if !tcm.dtcm.load_mode => bytes_at(&self.dtcm, offset),
                _ => self.read_page(addr),
            },
        }
    }

    /// The bytes an instruction fetch of `N` bytes at `addr` sees.
    // TODO(hw-unverified): a fetch inside the DTCM region, or from a load-mode ITCM, reads the
    // memory underneath (`arm9-memory-map.md`, "Unverified").
    fn fetch<const N: usize>(&self, tcm: &TcmConfig, addr: u32) -> [u8; N] {
        let addr = addr & !(N as u32 - 1);
        match tcm_offset(&tcm.itcm, ITCM_SIZE, addr) {
            Some(offset) if !tcm.itcm.load_mode => bytes_at(&self.itcm, offset),
            _ => self.read_page(addr),
        }
    }

    /// Stores `N` bytes at `addr` (aligned down to `N`). A TCM takes the store whether or not
    /// it is in load mode.
    // TODO(hw-unverified): the ITCM wins over an overlapping DTCM (see `Memory::load`).
    fn store<const N: usize>(&mut self, tcm: &TcmConfig, addr: u32, bytes: [u8; N]) {
        let addr = addr & !(N as u32 - 1);
        if let Some(offset) = tcm_offset(&tcm.itcm, ITCM_SIZE, addr) {
            return put_bytes(&mut self.itcm, offset, bytes);
        }
        if let Some(offset) = tcm_offset(&tcm.dtcm, DTCM_SIZE, addr) {
            return put_bytes(&mut self.dtcm, offset, bytes);
        }
        let page = self.page(addr);
        let offset = page.offset as usize + (addr & PAGE_MASK) as usize;
        match page.region {
            Region::MainRam => put_bytes(&mut self.main_ram, offset, bytes),
            // VRAM takes 16-bit and 32-bit writes only; a byte write is ignored
            // (`vram-banks.md`, "Behavior").
            Region::VramA if N > 1 => put_bytes(&mut self.vram_a, offset, bytes),
            Region::VramA | Region::Unmapped => self.write_slow(addr, bytes),
        }
    }

    /// The slow path for a write: I/O ports; an unmapped address ignores the write.
    // TODO(hw-unverified): a wider write over a byte register uses only the byte at the
    // register's address (`arm9-memory-map.md`, "Unverified").
    // TODO(hw-unverified): the I/O block is not mirrored (`arm9-memory-map.md`, "Unverified").
    // TODO(phase3): the I/O registers of the other blocks arrive with their phases.
    fn write_slow<const N: usize>(&mut self, addr: u32, bytes: [u8; N]) {
        // The index of VRAMCNT_A's byte inside this write. An address past the register wraps
        // to a huge index and a VRAM byte write lands here too; both miss `get` and do nothing.
        let touched = VRAMCNT_A_ADDR.wrapping_sub(addr) as usize;
        if let Some(&value) = bytes.get(touched) {
            self.write_vramcnt_a(value);
        }
    }

    /// Whether the buffers have the sizes `new` allocates and VRAMCNT_A holds only used bits.
    /// A savestate that decodes to anything else is corrupt (`savestate`); the accessors stay
    /// total either way, so this guards behavior, not memory safety.
    pub(crate) fn is_well_formed(&self) -> bool {
        self.main_ram.len() == MAIN_RAM_SIZE
            && self.itcm.len() == ITCM_SIZE
            && self.dtcm.len() == DTCM_SIZE
            && self.vram_a.len() == VRAM_A_SIZE
            && self.vramcnt_a & !VRAMCNT_USED == 0
    }

    /// Stores VRAMCNT_A and rebuilds the page table, since the write can move the bank.
    // TODO(hw-unverified): bit 2 is dropped (`vram-banks.md`, "Unverified").
    fn write_vramcnt_a(&mut self, value: u8) {
        self.vramcnt_a = value & VRAMCNT_USED;
        self.build_page_table();
    }

    /// Fetches an ARM opcode.
    pub fn fetch32(&self, tcm: &TcmConfig, addr: u32) -> u32 {
        u32::from_le_bytes(self.fetch(tcm, addr))
    }

    /// Fetches a Thumb opcode.
    pub fn fetch16(&self, tcm: &TcmConfig, addr: u32) -> u16 {
        u16::from_le_bytes(self.fetch(tcm, addr))
    }

    /// Reads a byte of data. Reads have no side effects, so the debugger uses them too.
    pub fn read8(&self, tcm: &TcmConfig, addr: u32) -> u8 {
        u8::from_le_bytes(self.load(tcm, addr))
    }

    /// Reads a halfword of data.
    pub fn read16(&self, tcm: &TcmConfig, addr: u32) -> u16 {
        u16::from_le_bytes(self.load(tcm, addr))
    }

    /// Reads a word of data.
    pub fn read32(&self, tcm: &TcmConfig, addr: u32) -> u32 {
        u32::from_le_bytes(self.load(tcm, addr))
    }

    /// Writes a byte of data.
    pub fn write8(&mut self, tcm: &TcmConfig, addr: u32, value: u8) {
        self.store(tcm, addr, value.to_le_bytes());
    }

    /// Writes a halfword of data.
    pub fn write16(&mut self, tcm: &TcmConfig, addr: u32, value: u16) {
        self.store(tcm, addr, value.to_le_bytes());
    }

    /// Writes a word of data.
    pub fn write32(&mut self, tcm: &TcmConfig, addr: u32, value: u32) {
        self.store(tcm, addr, value.to_le_bytes());
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests_well_formed {
    use super::*;

    #[test]
    fn given_power_on_memory_when_checked_then_it_is_well_formed() {
        assert!(Memory::new().is_well_formed());
    }

    #[test]
    fn given_a_short_main_ram_when_checked_then_it_is_not_well_formed() {
        let mut memory = Memory::new();
        memory.main_ram.pop();
        assert!(!memory.is_well_formed());
    }

    #[test]
    fn given_a_short_itcm_when_checked_then_it_is_not_well_formed() {
        let mut memory = Memory::new();
        memory.itcm.pop();
        assert!(!memory.is_well_formed());
    }

    #[test]
    fn given_a_short_dtcm_when_checked_then_it_is_not_well_formed() {
        let mut memory = Memory::new();
        memory.dtcm.pop();
        assert!(!memory.is_well_formed());
    }

    #[test]
    fn given_a_long_vram_a_when_checked_then_it_is_not_well_formed() {
        let mut memory = Memory::new();
        memory.vram_a.push(0);
        assert!(!memory.is_well_formed());
    }

    #[test]
    fn given_an_unused_vramcnt_a_bit_when_checked_then_it_is_not_well_formed() {
        let mut memory = Memory::new();
        memory.vramcnt_a = !VRAMCNT_USED;
        assert!(!memory.is_well_formed());
    }
}
