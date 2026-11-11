// SPDX-License-Identifier: Apache-2.0
//! The memory both CPUs see: main RAM, the two TCMs, VRAM bank A (as LCDC or as engine A BG), the
//! palette RAM, the 32 KiB of shared WRAM that WRAMCNT hands out, and the ARM7's own WRAM. Each
//! CPU has a page table of its own (`docs/hw/arm9-memory-map.md`, `docs/hw/vram-banks.md`,
//! `docs/hw/palette-ram.md`, `docs/hw/wramcnt.md`).
//!
//! Access order: (ARM9 only) TCM, then the page table. An address neither answers returns
//! `None` and the bus view sends it to the register devices (`Io`), which read 0 for an address
//! nothing holds and drop a write. Guest addresses never panic. WRAMCNT and VRAMCNT_A stay here
//! because the page tables derive from them: a write to either rebuilds the tables.

use super::TcmConfig;
use crate::cpu::Tcm;
use crate::io::device::{Width, read_bytes};
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

/// ARM7 WRAM: 64 KiB at `0380_0000h` (`memory-map.md`).
const WRAM7_BASE: u32 = 0x0380_0000;
const WRAM7_SIZE: usize = 64 << 10;
/// Shared WRAM: 32 KiB in two 16 KiB halves (`wramcnt.md`, "Registers").
const SHARED_WRAM_SIZE: usize = 32 << 10;
const WRAM_HALF_SIZE: usize = 16 << 10;
/// The window shared WRAM repeats through: `0300_0000h`-`03FF_FFFFh` for the ARM9; the ARM7's
/// shared part ends at `WRAM7_BASE` (`wramcnt.md`, "Behavior").
const WRAM_WINDOW_BASE: u32 = 0x0300_0000;
const WRAM_WINDOW_END: u32 = 0x0400_0000;
/// WRAMCNT at `0400_0247h` (ARM9) and its ARM7 read-only copy WRAMSTAT at `0400_0241h`.
const WRAMCNT_ADDR: u32 = 0x0400_0247;
const WRAMSTAT_ADDR: u32 = 0x0400_0241;
/// VRAMCNT_G at `0400_0246h`, between VRAMCNT_F and WRAMCNT; Phase 4 (#35) owns it, so a write
/// is discarded for now.
const VRAMCNT_G_ADDR: u32 = 0x0400_0246;
/// WRAMCNT bits 0-1: the allocation mode. Bits 2-7 are not used.
const WRAMCNT_MODE_MASK: u8 = 0x03;

/// The BIOS areas: 32 KiB at `FFFF_0000h` for the ARM9, 16 KiB at 0 for the ARM7 (`hle-swi.md`,
/// "The BIOS area").
pub(crate) const BIOS9_BASE: u32 = 0xFFFF_0000;
const BIOS9_SIZE: u32 = 32 << 10;
pub(crate) const BIOS7_BASE: u32 = 0;
const BIOS7_SIZE: u32 = 16 << 10;

/// VRAM bank A, mapped as LCDC at `0680_0000h`, 128 KiB (`vram-banks.md`, "Behavior").
const VRAM_A_BASE: u32 = 0x0680_0000;
const VRAM_A_SIZE: usize = 128 << 10;
/// Engine A's BG VRAM window: 2 MiB at `0600_0000h`; bank A with MST 1 sits `OFS * 20000h` into
/// it (`vram-banks.md`, "Engine A BG VRAM (MST 1)").
const BG_A_BASE: u32 = 0x0600_0000;
/// Palette RAM: 2 KiB at `0500_0000h`, repeated through its 16 MiB block (`palette-ram.md`,
/// "Layout" and "Behavior").
const PALETTE_BASE: u32 = 0x0500_0000;
const PALETTE_SIZE: usize = 2 << 10;
const PALETTE_BLOCK_SIZE: usize = 16 << 20;
/// VRAMCNT_A, at `0400_0240h` (`vram-banks.md`, "Registers").
pub(crate) const VRAMCNT_A_ADDR: u32 = 0x0400_0240;
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
/// The MST value that maps bank A into engine A's BG VRAM.
const MST_BG_A: u8 = 1;

/// Which memory a page of the address space is backed by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    /// Nothing answers on the fast path; I/O and unmapped addresses use the slow path.
    Unmapped,
    /// Main RAM.
    MainRam,
    /// VRAM bank A, mapped as LCDC or as engine A BG VRAM; the same buffer either way.
    VramA,
    /// The palette RAM, 2 KiB repeated through its block (`palette-ram.md`).
    Palette,
    /// The shared WRAM, as WRAMCNT allocates it to the CPU asking.
    SharedWram,
    /// The BIOS area: synthetic, reads zero, writes are ignored (`hle-swi.md`, "The BIOS area").
    Bios,
    /// The ARM7's own 64 KiB WRAM.
    Wram7,
}

/// What share of the 32 KiB shared WRAM a CPU owns (`wramcnt.md`, "WRAMCNT bits").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WramShare {
    /// Nothing.
    None,
    /// Bytes 0-16383 of the shared WRAM.
    First16,
    /// Bytes 16384-32767.
    Second16,
    /// All 32 KiB.
    All32,
}

impl WramShare {
    /// `(start, length)` inside the shared WRAM, or `None` when the CPU owns nothing.
    fn span(self) -> Option<(usize, usize)> {
        match self {
            Self::None => None,
            Self::First16 => Some((0, WRAM_HALF_SIZE)),
            Self::Second16 => Some((WRAM_HALF_SIZE, WRAM_HALF_SIZE)),
            Self::All32 => Some((0, SHARED_WRAM_SIZE)),
        }
    }
}

/// WRAMCNT as decoded (`wramcnt.md`, "WRAMCNT bits"). A debugger's memory view (#57) shows each
/// CPU's window from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WramState {
    /// Bits 0-1.
    pub mode: u8,
    /// What the ARM9 owns.
    pub arm9: WramShare,
    /// What the ARM7 owns.
    pub arm7: WramShare,
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

/// Engine A's BG VRAM as the renderer reads it: the bytes of bank A and where they sit in the BG
/// window, or nothing when no bank is mapped there (`vram-banks.md`, "Engine A BG VRAM (MST 1)").
/// Offsets count from the start of the window (`0600_0000h`); an offset outside the bank reads 0.
#[derive(Clone, Copy, Debug)]
pub struct BgVram<'a> {
    bank: Option<&'a [u8]>,
    /// Where the bank starts in the window: `OFS * 20000h`.
    base: u32,
}

impl BgVram<'_> {
    /// The byte at `offset` in the window, or 0 where no bank holds it.
    // TODO(hw-unverified): an address no bank holds reads 0 (`vram-banks.md`, "Unverified").
    pub(crate) fn byte(&self, offset: u32) -> u8 {
        offset
            .checked_sub(self.base)
            .and_then(|at| self.bank?.get(at as usize))
            .copied()
            .unwrap_or(0)
    }

    /// The little-endian halfword at `offset`.
    pub(crate) fn half(&self, offset: u32) -> u16 {
        u16::from_le_bytes([self.byte(offset), self.byte(offset.wrapping_add(1))])
    }
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

/// The RAM buffers and the two page tables. Plain data: the buffers are allocated once in
/// [`Memory::new`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Memory {
    main_ram: Vec<u8>,
    itcm: Vec<u8>,
    dtcm: Vec<u8>,
    vram_a: Vec<u8>,
    vramcnt_a: u8,
    // The palette RAM (`palette-ram.md`); the ARM9 alone sees it.
    palette: Vec<u8>,
    // The ARM7's own WRAM (64 KiB); only the ARM7 view reaches it.
    wram7: Vec<u8>,
    // The shared WRAM (32 KiB) and the WRAMCNT mode (bits 0-1) that splits it (`wramcnt.md`).
    shared_wram: Vec<u8>,
    wramcnt: u8,
    // Derived from `vramcnt_a` and `wramcnt`, so the tables are not saved; `build_page_table`
    // rebuilds both. Empty after a load until then, which reads as all-unmapped rather than
    // panicking. One entry per 16 KiB page, one table per CPU (the page-table decision is
    // recorded in the PR that added #22).
    #[serde(skip)]
    pages9: Vec<Page>,
    #[serde(skip)]
    pages7: Vec<Page>,
}

/// The entry for the page holding `addr`; an address outside a (not yet built) table is unmapped.
fn page_of(pages: &[Page], addr: u32) -> Page {
    pages
        .get((addr >> PAGE_SHIFT) as usize)
        .copied()
        .unwrap_or(UNMAPPED_PAGE)
}

/// Points every page of `[base, end)` at `region`, repeating `span` (start, length in the
/// region's buffer) through the window one page at a time; no span leaves them unmapped.
fn map_window(
    pages: &mut [Page],
    base: u32,
    end: u32,
    region: Region,
    span: Option<(usize, usize)>,
) {
    let first = (base >> PAGE_SHIFT) as usize;
    let count = ((end - base) >> PAGE_SHIFT) as usize;
    let Some(window) = pages.get_mut(first..first + count) else {
        return;
    };
    for (i, page) in window.iter_mut().enumerate() {
        *page = match span {
            Some((start, len)) => Page {
                region,
                offset: (start + (i * PAGE_SIZE) % len) as u32,
            },
            None => UNMAPPED_PAGE,
        };
    }
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
    // TODO(hw-unverified): all RAM starts at zero, and VRAMCNT_A and WRAMCNT start at 0, which
    // GBATEK does not state (`arm9-memory-map.md`, "Unverified"; `vram-banks.md`, "Unverified";
    // `wramcnt.md`, "Unverified"; `palette-ram.md`, "Unverified").
    pub fn new() -> Self {
        let mut memory = Self {
            main_ram: vec![0; MAIN_RAM_SIZE],
            itcm: vec![0; ITCM_SIZE],
            dtcm: vec![0; DTCM_SIZE],
            vram_a: vec![0; VRAM_A_SIZE],
            vramcnt_a: 0,
            palette: vec![0; PALETTE_SIZE],
            wram7: vec![0; WRAM7_SIZE],
            shared_wram: vec![0; SHARED_WRAM_SIZE],
            wramcnt: 0,
            pages9: Vec::new(),
            pages7: Vec::new(),
        };
        memory.build_page_table();
        memory
    }

    /// Rebuilds both page tables from the mapping state. Called at construction, after a
    /// VRAMCNT_A write, and after a savestate load (`savestate`); a WRAMCNT write rebuilds only
    /// the shared WRAM block (`build_wram_pages`).
    // Per-CPU tables: the decision is recorded in the PR that added #22.
    // TODO(phase4): the other VRAM banks and the OBJ mapping join them (#35); texture slots are
    // `TODO(phase7)`.
    // TODO(hw-unverified): main RAM mirrors through its whole 16 MiB block, GBATEK's general
    // rule; a disabled bank or an MST that is not 0 or 1 is unmapped; the LCDC and BG windows
    // have no mirrors (`arm9-memory-map.md`, `vram-banks.md`, "Unverified").
    // TODO(hw-unverified): the palette RAM repeats through its 16 MiB block, and the ARM7 has
    // none (`palette-ram.md`, "Unverified").
    pub(crate) fn build_page_table(&mut self) {
        self.pages9.clear();
        self.pages9.resize(PAGE_COUNT, UNMAPPED_PAGE);
        self.pages7.clear();
        self.pages7.resize(PAGE_COUNT, UNMAPPED_PAGE);
        // Both CPUs see main RAM repeated through its 16 MiB block.
        for pages in [&mut self.pages9, &mut self.pages7] {
            map_window(
                pages,
                MAIN_RAM_BASE,
                MAIN_RAM_BASE + MAIN_RAM_BLOCK_SIZE as u32,
                Region::MainRam,
                Some((0, MAIN_RAM_SIZE)),
            );
        }
        // The synthetic BIOS, one per CPU (`hle-swi.md`); it has no backing buffer.
        map_window(
            &mut self.pages9,
            BIOS9_BASE,
            BIOS9_BASE + BIOS9_SIZE,
            Region::Bios,
            Some((0, PAGE_SIZE)),
        );
        map_window(
            &mut self.pages7,
            BIOS7_BASE,
            BIOS7_BASE + BIOS7_SIZE,
            Region::Bios,
            Some((0, PAGE_SIZE)),
        );
        // The palette RAM is the ARM9's alone; each 16 KiB page of the block points at offset 0 and
        // `region_bytes` takes the offset modulo its 2 KiB (a page holds eight repeats).
        map_window(
            &mut self.pages9,
            PALETTE_BASE,
            PALETTE_BASE + PALETTE_BLOCK_SIZE as u32,
            Region::Palette,
            Some((0, PALETTE_SIZE)),
        );
        // Bank A is seen at one place at a time: where MST says, and only while enabled.
        let bank = self.vram_a();
        let base = match (bank.enabled, bank.mst) {
            (true, MST_LCDC) => Some(VRAM_A_BASE),
            (true, MST_BG_A) => Some(BG_A_BASE + u32::from(bank.offset) * VRAM_A_SIZE as u32),
            _ => None,
        };
        if let Some(base) = base {
            map_window(
                &mut self.pages9,
                base,
                base + VRAM_A_SIZE as u32,
                Region::VramA,
                Some((0, VRAM_A_SIZE)),
            );
        }
        self.build_wram_pages();
    }

    /// Rebuilds the `0300_0000h` block of both tables: the invalidation a WRAMCNT write needs
    /// (`wramcnt.md`, "Changing the mode"). Everything outside that block is left alone.
    // TODO(hw-unverified): the ARM7's contiguous 96 KiB at `037F_8000h` follows from the
    // repetition rule, not from a GBATEK statement (`wramcnt.md`, "Unverified").
    // TODO(hw-unverified): the repetition of the shared half and of the ARM7's WRAM through
    // their windows is GBATEK's general rule (`wramcnt.md`, "Unverified").
    fn build_wram_pages(&mut self) {
        let state = self.wramcnt();
        map_window(
            &mut self.pages9,
            WRAM_WINDOW_BASE,
            WRAM_WINDOW_END,
            Region::SharedWram,
            state.arm9.span(),
        );
        // The ARM7 sees what it owns of the shared WRAM below `0380_0000h`, or its own WRAM when
        // it owns none; above that it always sees its own WRAM.
        let (region, span) = match state.arm7.span() {
            Some(span) => (Region::SharedWram, Some(span)),
            None => (Region::Wram7, Some((0, WRAM7_SIZE))),
        };
        map_window(&mut self.pages7, WRAM_WINDOW_BASE, WRAM7_BASE, region, span);
        map_window(
            &mut self.pages7,
            WRAM7_BASE,
            WRAM_WINDOW_END,
            Region::Wram7,
            Some((0, WRAM7_SIZE)),
        );
    }

    /// WRAMCNT as decoded. The debugger's memory viewer (#57) shows each CPU's window from it.
    pub fn wramcnt(&self) -> WramState {
        let mode = self.wramcnt & WRAMCNT_MODE_MASK;
        let (arm9, arm7) = match mode {
            0 => (WramShare::All32, WramShare::None),
            1 => (WramShare::Second16, WramShare::First16),
            2 => (WramShare::First16, WramShare::Second16),
            _ => (WramShare::None, WramShare::All32),
        };
        WramState { mode, arm9, arm7 }
    }

    /// Engine A's BG VRAM: bank A when it is enabled with MST 1, else nothing
    /// (`vram-banks.md`, "Engine A BG VRAM (MST 1)").
    pub(crate) fn bg_vram_a(&self) -> BgVram<'_> {
        let bank = self.vram_a();
        BgVram {
            bank: (bank.enabled && bank.mst == MST_BG_A).then_some(self.vram_a.as_slice()),
            base: u32::from(bank.offset) * VRAM_A_SIZE as u32,
        }
    }

    /// The bytes of bank A while the CPU sees it as LCDC VRAM (enabled, MST 0), else `None`.
    pub(crate) fn vram_a_lcdc(&self) -> Option<&[u8]> {
        let bank = self.vram_a();
        (bank.enabled && bank.mst == MST_LCDC).then_some(self.vram_a.as_slice())
    }

    /// The 2 KiB of palette RAM: engine A's BG and OBJ tables, then engine B's (`palette-ram.md`,
    /// "Layout"). A debugger's palette panel (#57) shows it.
    pub fn palette(&self) -> &[u8] {
        &self.palette
    }

    /// The region backing the page that holds `addr`, as the ARM9 sees it.
    pub fn region(&self, addr: u32) -> Region {
        page_of(&self.pages9, addr).region
    }

    /// The region backing the page that holds `addr`, as the ARM7 sees it.
    pub fn region7(&self, addr: u32) -> Region {
        page_of(&self.pages7, addr).region
    }

    /// VRAMCNT_A as decoded.
    pub fn vram_a(&self) -> VramBankState {
        VramBankState {
            enabled: self.vramcnt_a & VRAMCNT_ENABLE != 0,
            mst: self.vramcnt_a & VRAMCNT_MST_MASK,
            offset: (self.vramcnt_a >> VRAMCNT_OFS_SHIFT) & VRAMCNT_OFS_MASK,
        }
    }

    /// The page table of CPU `ARM9`.
    fn pages<const ARM9: bool>(&self) -> &[Page] {
        if ARM9 { &self.pages9 } else { &self.pages7 }
    }

    /// The bytes of `region` at `offset`, or `None` for a region the page table does not back.
    // TODO(hw-unverified): an empty ARM9 window (mode 3) falls through to the slow path, reads 0
    // and drops writes (`wramcnt.md`, "Unverified").
    fn region_bytes<const N: usize>(&self, region: Region, offset: usize) -> Option<[u8; N]> {
        let buf = match region {
            Region::MainRam => &self.main_ram,
            Region::VramA => &self.vram_a,
            // The palette repeats every 2 KiB inside a page too, so the offset wraps here.
            Region::Palette => return Some(bytes_at(&self.palette, offset % PALETTE_SIZE)),
            Region::SharedWram => &self.shared_wram,
            Region::Wram7 => &self.wram7,
            // The BIOS is synthetic: it reads zero (`hle-swi.md`, "The BIOS area").
            // TODO(hw-unverified): the ARM7 BIOS is read-protected on hardware.
            Region::Bios => return Some([0; N]),
            Region::Unmapped => return None,
        };
        Some(bytes_at(buf, offset))
    }

    /// The bytes `pages` map for an access of `N` bytes at the aligned `addr`, or `None` when
    /// the page is I/O or unmapped and the register devices answer. The bus stage CPU reads share;
    /// the CPU-only TCM stage sits in front of it.
    fn page_bytes<const ARM9: bool, const N: usize>(&self, addr: u32) -> Option<[u8; N]> {
        let page = page_of(self.pages::<ARM9>(), addr);
        let offset = page.offset as usize + (addr & PAGE_MASK) as usize;
        self.region_bytes(page.region, offset)
    }

    /// What an ARM9 data read of `N` bytes at the aligned `addr` finds in a TCM, if a TCM answers.
    ///
    /// The ITCM has priority over the DTCM; in load mode it claims the address but only
    /// writes reach it, so the read goes underneath (`None`: the page table answers).
    // TODO(hw-unverified): GBATEK says the ITCM "appears to have priority"; taken as certain.
    // TODO(hw-unverified): a load-mode ITCM over an enabled DTCM reads underneath, not the
    // DTCM (`arm9-memory-map.md`, "Unverified").
    fn tcm_load<const N: usize>(&self, tcm: &TcmConfig, addr: u32) -> Option<[u8; N]> {
        match tcm_offset(&tcm.itcm, ITCM_SIZE, addr) {
            Some(offset) if !tcm.itcm.load_mode => Some(bytes_at(&self.itcm, offset)),
            Some(_) => None,
            None => match tcm_offset(&tcm.dtcm, DTCM_SIZE, addr) {
                Some(offset) if !tcm.dtcm.load_mode => Some(bytes_at(&self.dtcm, offset)),
                _ => None,
            },
        }
    }

    /// The bytes a data read of `N` bytes at `addr` finds in memory, with no side effect, or
    /// `None` when the register devices answer. `addr` is aligned down to `N`, the way the memory
    /// system ignores the bits below the access width. The ARM7 has no TCM: its `tcm` is
    /// [`TcmConfig::NONE`].
    // TODO(hw-unverified): the rest of the ARM7 bus reads 0 (`wramcnt.md`, "Unverified").
    pub(crate) fn load<const ARM9: bool, const N: usize>(
        &self,
        tcm: &TcmConfig,
        addr: u32,
    ) -> Option<[u8; N]> {
        let addr = addr & !(N as u32 - 1);
        self.tcm_load(tcm, addr)
            .or_else(|| self.page_bytes::<ARM9, N>(addr))
    }

    /// The bytes an instruction fetch of `N` bytes at `addr` finds in memory, or `None` when the
    /// register devices answer.
    // TODO(hw-unverified): a fetch inside the DTCM region, or from a load-mode ITCM, reads the
    // memory underneath (`arm9-memory-map.md`, "Unverified").
    pub(crate) fn fetch<const ARM9: bool, const N: usize>(
        &self,
        tcm: &TcmConfig,
        addr: u32,
    ) -> Option<[u8; N]> {
        let addr = addr & !(N as u32 - 1);
        match tcm_offset(&tcm.itcm, ITCM_SIZE, addr) {
            Some(offset) if !tcm.itcm.load_mode => Some(bytes_at(&self.itcm, offset)),
            _ => self.page_bytes::<ARM9, N>(addr),
        }
    }

    /// Stores `N` bytes at `addr` (aligned down to `N`). A TCM takes the store whether or not
    /// it is in load mode. Returns whether memory took it: `false` sends the store to the
    /// register devices (an I/O port or an unmapped address, and a VRAM byte write, which
    /// the devices drop).
    // TODO(hw-unverified): the ITCM wins over an overlapping DTCM (see `Memory::load`).
    pub(crate) fn store<const ARM9: bool, const N: usize>(
        &mut self,
        tcm: &TcmConfig,
        addr: u32,
        bytes: [u8; N],
    ) -> bool {
        let addr = addr & !(N as u32 - 1);
        if let Some(offset) = tcm_offset(&tcm.itcm, ITCM_SIZE, addr) {
            put_bytes(&mut self.itcm, offset, bytes);
            return true;
        }
        if let Some(offset) = tcm_offset(&tcm.dtcm, DTCM_SIZE, addr) {
            put_bytes(&mut self.dtcm, offset, bytes);
            return true;
        }
        let page = page_of(self.pages::<ARM9>(), addr);
        let offset = page.offset as usize + (addr & PAGE_MASK) as usize;
        match page.region {
            // VRAM and the palette take 16-bit and 32-bit writes only; a byte write is ignored
            // (`vram-banks.md`, "Behavior"; `palette-ram.md`, "Behavior").
            // TODO(hw-unverified): the rule is taken to cover the BG window too
            // (`vram-banks.md`, "Unverified").
            Region::VramA | Region::Palette if N == 1 => false,
            Region::Unmapped => false,
            // ROM: the write is taken and dropped, it is not an I/O access.
            Region::Bios => true,
            region => {
                self.put_region(region, offset, bytes);
                true
            }
        }
    }

    /// Writes `bytes` into the buffer behind `region`.
    fn put_region<const N: usize>(&mut self, region: Region, offset: usize, bytes: [u8; N]) {
        let buf = match region {
            Region::MainRam => &mut self.main_ram,
            Region::VramA => &mut self.vram_a,
            Region::Palette => {
                put_bytes(&mut self.palette, offset % PALETTE_SIZE, bytes);
                return;
            }
            Region::SharedWram => &mut self.shared_wram,
            Region::Wram7 => &mut self.wram7,
            Region::Bios | Region::Unmapped => return,
        };
        put_bytes(buf, offset, bytes);
    }

    /// What CPU `ARM9` reads from the memory-control block at `offset` bytes past VRAMCNT_A:
    /// WRAMCNT for the ARM9, WRAMSTAT for the ARM7. VRAMCNT_A is write-only and reads 0. This is
    /// the memory-control block of the I/O map (`Owner::Memory`), kept here because the page tables derive from
    /// these registers (#109); it has no side effect, so a peek and a read are the same.
    // WRAMCNT is documented R/W in one place and write-only in another; it reads back.
    // TODO(hw-unverified): WRAMSTAT is the stored 2-bit mode (`wramcnt.md`, "Unverified").
    // TODO(hw-unverified): VRAMCNT_A reads 0 (`arm9-memory-map.md`, "Unverified").
    pub(crate) fn read_control<const ARM9: bool>(&self, offset: u32, width: Width) -> u32 {
        read_bytes(offset, width, |at| {
            match (ARM9, VRAMCNT_A_ADDR.wrapping_add(at)) {
                (true, WRAMCNT_ADDR) | (false, WRAMSTAT_ADDR) => self.wramcnt,
                _ => 0,
            }
        })
    }

    /// Writes the memory-control block at `offset` bytes past VRAMCNT_A, each byte to the
    /// register at its own address: VRAMCNT_A, WRAMCNT, or VRAMCNT_G (discarded for now). The
    /// ARM7 has none (WRAMCNT is ARM9-only), and a byte on no register is dropped.
    // TODO(hw-unverified): a byte on no register is dropped (`arm9-memory-map.md`, "Unverified").
    pub(crate) fn write_control<const ARM9: bool>(
        &mut self,
        offset: u32,
        width: Width,
        value: u32,
    ) {
        if !ARM9 {
            return;
        }
        (0..width.bytes()).for_each(|i| {
            let byte = (value >> (8 * i)) as u8;
            match VRAMCNT_A_ADDR.wrapping_add(offset).wrapping_add(i) {
                VRAMCNT_A_ADDR => self.write_vramcnt_a(byte),
                WRAMCNT_ADDR => self.write_wramcnt(byte),
                // TODO(phase4): VRAMCNT_G arrives with the other banks (#35).
                VRAMCNT_G_ADDR => {}
                _ => {}
            }
        });
    }

    /// Whether the buffers have the sizes `new` allocates and VRAMCNT_A holds only used bits.
    /// A savestate that decodes to anything else is corrupt (`savestate`); the accessors stay
    /// total either way, so this guards behavior, not memory safety.
    pub(crate) fn is_well_formed(&self) -> bool {
        self.main_ram.len() == MAIN_RAM_SIZE
            && self.itcm.len() == ITCM_SIZE
            && self.dtcm.len() == DTCM_SIZE
            && self.vram_a.len() == VRAM_A_SIZE
            && self.palette.len() == PALETTE_SIZE
            && self.wram7.len() == WRAM7_SIZE
            && self.shared_wram.len() == SHARED_WRAM_SIZE
            && self.vramcnt_a & !VRAMCNT_USED == 0
            && self.wramcnt & !WRAMCNT_MODE_MASK == 0
    }

    /// Stores VRAMCNT_A and rebuilds the page table, since the write can move the bank.
    // TODO(hw-unverified): bit 2 is dropped (`vram-banks.md`, "Unverified").
    fn write_vramcnt_a(&mut self, value: u8) {
        self.vramcnt_a = value & VRAMCNT_USED;
        self.build_page_table();
    }

    /// Stores WRAMCNT (bits 2-7 dropped) and rebuilds the shared WRAM block of both page tables,
    /// since the write moves ownership.
    fn write_wramcnt(&mut self, value: u8) {
        self.wramcnt = value & WRAMCNT_MODE_MASK;
        self.build_wram_pages();
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
