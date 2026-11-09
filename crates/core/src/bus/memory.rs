// SPDX-License-Identifier: Apache-2.0
//! The memory both CPUs see: main RAM, the two TCMs, VRAM bank A as LCDC, the 32 KiB of shared
//! WRAM that WRAMCNT hands out, and the ARM7's own WRAM. Each CPU has a page table of its own
//! (`docs/hw/arm9-memory-map.md`, `docs/hw/vram-banks.md`, `docs/hw/wramcnt.md`).
//!
//! Access order: (ARM9 only) TCM, then the page table, then the slow path (I/O and unmapped).
//! Guest addresses never panic: an address that nothing answers reads 0 and ignores a write.

use super::TcmConfig;
use crate::cpu::Tcm;
use crate::gpu2d::Gpu2d;
use crate::gpu2d::timing::Timing;
use crate::ipc::{IPCFIFORECV_ADDR, IPCFIFOSEND_ADDR, Ipc, Requests};
use crate::irq::InterruptController;
use crate::timers::Timers;
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
    /// The shared WRAM, as WRAMCNT allocates it to the CPU asking.
    SharedWram,
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
    // The ARM7's own WRAM (64 KiB); only the ARM7 view reaches it.
    wram7: Vec<u8>,
    // The shared WRAM (32 KiB) and the WRAMCNT mode (bits 0-1) that splits it (`wramcnt.md`).
    shared_wram: Vec<u8>,
    wramcnt: u8,
    // Engine A's display registers, reached through the I/O slow path.
    gpu2d: Gpu2d,
    // VCOUNT and each CPU's DISPSTAT (`display-timing.md`).
    timing: Timing,
    // Each CPU's interrupt controller: the same addresses, separate state (`interrupts.md`).
    irq9: InterruptController<true>,
    irq7: InterruptController<false>,
    // IPCSYNC and the two IPC FIFOs (`ipc.md`).
    ipc: Ipc,
    // Each CPU's four timers: the same addresses, separate state (`timers.md`).
    timers9: Timers<true>,
    timers7: Timers<false>,
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
    // `wramcnt.md`, "Unverified").
    pub fn new() -> Self {
        let mut memory = Self {
            main_ram: vec![0; MAIN_RAM_SIZE],
            itcm: vec![0; ITCM_SIZE],
            dtcm: vec![0; DTCM_SIZE],
            vram_a: vec![0; VRAM_A_SIZE],
            vramcnt_a: 0,
            wram7: vec![0; WRAM7_SIZE],
            shared_wram: vec![0; SHARED_WRAM_SIZE],
            wramcnt: 0,
            gpu2d: Gpu2d::new(),
            timing: Timing::new(),
            irq9: InterruptController::new(),
            irq7: InterruptController::new(),
            ipc: Ipc::new(),
            timers9: Timers::new(),
            timers7: Timers::new(),
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
    // TODO(phase3): the BIOS joins the tables (#27).
    // TODO(phase4): the other VRAM banks and MST values join them (#35).
    // TODO(hw-unverified): main RAM mirrors through its whole 16 MiB block, GBATEK's general
    // rule; a disabled bank or MST other than 0 is unmapped; the LCDC window has no mirrors
    // (`arm9-memory-map.md`, `vram-banks.md`, "Unverified").
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
        let bank = self.vram_a();
        if bank.enabled && bank.mst == MST_LCDC {
            map_window(
                &mut self.pages9,
                VRAM_A_BASE,
                VRAM_A_BASE + VRAM_A_SIZE as u32,
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

    /// Engine A's display registers.
    // The debugger's IO register panel will show them (#58 shows the VRAM view).
    pub fn gpu2d(&self) -> &Gpu2d {
        &self.gpu2d
    }

    /// The bytes of bank A while the CPU sees it as LCDC VRAM (enabled, MST 0), else `None`.
    pub(crate) fn vram_a_lcdc(&self) -> Option<&[u8]> {
        let bank = self.vram_a();
        (bank.enabled && bank.mst == MST_LCDC).then_some(self.vram_a.as_slice())
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

    /// The bytes of `region` at `offset`, or `None` for a region the page table does not back.
    // TODO(hw-unverified): an empty ARM9 window (mode 3) falls through to the slow path, reads 0
    // and drops writes (`wramcnt.md`, "Unverified").
    fn region_bytes<const N: usize>(&self, region: Region, offset: usize) -> Option<[u8; N]> {
        let buf = match region {
            Region::MainRam => &self.main_ram,
            Region::VramA => &self.vram_a,
            Region::SharedWram => &self.shared_wram,
            Region::Wram7 => &self.wram7,
            Region::Unmapped => return None,
        };
        Some(bytes_at(buf, offset))
    }

    /// The bytes behind the ARM9 page table for an access of `N` bytes at the aligned `addr`.
    fn read_page<const N: usize>(&self, now: u64, addr: u32) -> [u8; N] {
        self.page_bytes(&self.pages9, addr)
            .unwrap_or_else(|| self.read_slow(now, addr))
    }

    /// The bytes `pages` map for an access of `N` bytes at the aligned `addr`, or `None` when
    /// the page is I/O or unmapped and the slow path answers. The bus stage CPU reads share;
    /// the CPU-only TCM stage sits in front of it.
    fn page_bytes<const N: usize>(&self, pages: &[Page], addr: u32) -> Option<[u8; N]> {
        let page = page_of(pages, addr);
        let offset = page.offset as usize + (addr & PAGE_MASK) as usize;
        self.region_bytes(page.region, offset)
    }

    /// Which byte of IPCFIFORECV `addr` is, if it is one.
    fn recv_offset(addr: u32) -> Option<usize> {
        addr.checked_sub(IPCFIFORECV_ADDR)
            .filter(|&offset| offset < 4)
            .map(|offset| offset as usize)
    }

    /// The byte of IPCFIFORECV at `at` that a read would show, without taking the word.
    // TODO(hw-unverified): IPCFIFORECV is not mirrored (`ipc.md`, "Unverified").
    fn peek_recv_byte<const ARM9: bool>(&self, at: u32) -> Option<u8> {
        let offset = Self::recv_offset(at)?;
        Some(self.ipc.peek_recv::<ARM9>().to_le_bytes()[offset])
    }

    /// A CPU's read of `N` bytes at `addr` when it lands on IPCFIFORECV: one word is taken from
    /// the FIFO whatever the width, and the access sees the bytes at its own address.
    // TODO(hw-unverified): a byte or halfword read takes a whole word (`ipc.md`, "Unverified").
    fn receive_bytes<const ARM9: bool, const N: usize>(&mut self, addr: u32) -> Option<[u8; N]> {
        let offset = Self::recv_offset(addr)?;
        let (word, requests) = self.ipc.receive::<ARM9>();
        self.apply(requests);
        let bytes = word.to_le_bytes();
        // `addr` is aligned to `N` and the register is 4 bytes, so the access stays inside it.
        Some(std::array::from_fn(|i| bytes[(offset + i) & 3]))
    }

    /// A CPU's write of `N` bytes at the aligned `addr` when it lands on IPCFIFOSEND: a word
    /// write sends one word. Returns whether the address was IPCFIFOSEND.
    // TODO(hw-unverified): a byte or halfword write is ignored (`ipc.md`, "Unverified").
    fn send_word<const ARM9: bool, const N: usize>(&mut self, addr: u32, bytes: [u8; N]) -> bool {
        if addr & !3 != IPCFIFOSEND_ADDR {
            return false;
        }
        if let Ok(word) = <[u8; 4]>::try_from(bytes.as_slice()) {
            let requests = self.ipc.send::<ARM9>(u32::from_le_bytes(word));
            self.apply(requests);
        }
        true
    }

    /// Hands the IF bits an IPC event raised to the two controllers.
    fn apply(&mut self, requests: Requests) {
        self.irq9.request(requests.irq9);
        self.irq7.request(requests.irq7);
    }

    /// The slow path for a read: I/O ports and unmapped addresses. Each byte is read from the
    /// register at its own address, so a wider read spans neighbouring registers.
    // TODO(hw-unverified): an undefined I/O port reads 0 (GBATEK says so); unmapped memory is
    // "undefined" there and reads 0 here; VRAMCNT_A is write-only and reads 0
    // (`arm9-memory-map.md`, "Unverified").
    // TODO(hw-unverified): the I/O block is not mirrored (`arm9-memory-map.md`, "Unverified").
    // TODO(phase3): the I/O registers of the other blocks arrive with their phases.
    fn read_slow<const N: usize>(&self, now: u64, addr: u32) -> [u8; N] {
        std::array::from_fn(|i| {
            let at = addr.wrapping_add(i as u32);
            // WRAMCNT is documented R/W in one place and write-only in another; it reads back.
            // TODO(hw-unverified): `wramcnt.md`, "Unverified".
            match at {
                WRAMCNT_ADDR => self.wramcnt,
                _ => self
                    .irq9
                    .read_byte(at)
                    .or_else(|| self.ipc.read_byte::<true>(at))
                    .or_else(|| self.peek_recv_byte::<true>(at))
                    .or_else(|| self.gpu2d.read_byte(at))
                    .or_else(|| self.timing.read_byte::<true>(at))
                    .or_else(|| self.timers9.read_byte(at, now))
                    .unwrap_or(0),
            }
        })
    }

    /// The slow path for an ARM7 read: WRAMSTAT; every other I/O port and unmapped address
    /// reads 0 (`wramcnt.md`, "Unverified").
    // TODO(phase3): the ARM7's other I/O registers arrive with their issues (#26).
    fn read_slow7<const N: usize>(&self, now: u64, addr: u32) -> [u8; N] {
        std::array::from_fn(|i| match addr.wrapping_add(i as u32) {
            // TODO(hw-unverified): WRAMSTAT is the stored 2-bit mode (`wramcnt.md`, "Unverified").
            WRAMSTAT_ADDR => self.wramcnt,
            at => self
                .irq7
                .read_byte(at)
                .or_else(|| self.ipc.read_byte::<false>(at))
                .or_else(|| self.peek_recv_byte::<false>(at))
                .or_else(|| self.timing.read_byte::<false>(at))
                .or_else(|| self.timers7.read_byte(at, now))
                .unwrap_or(0),
        })
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

    /// The bytes an ARM9 data read of `N` bytes at `addr` sees, with no side effect. `addr` is
    /// aligned down to `N`, the way the memory system ignores the bits below the access width.
    fn load<const N: usize>(&self, tcm: &TcmConfig, addr: u32) -> [u8; N] {
        let addr = addr & !(N as u32 - 1);
        self.tcm_load(tcm, addr)
            .unwrap_or_else(|| self.read_page(tcm.now, addr))
    }

    /// [`Memory::load`] as the CPU does it: a read of IPCFIFORECV takes a word out of the FIFO.
    fn load_mut<const N: usize>(&mut self, tcm: &TcmConfig, addr: u32) -> [u8; N] {
        let addr = addr & !(N as u32 - 1);
        if let Some(bytes) = self.tcm_load(tcm, addr) {
            return bytes;
        }
        match self.page_bytes(&self.pages9, addr) {
            Some(bytes) => bytes,
            None => self
                .receive_bytes::<true, N>(addr)
                .unwrap_or_else(|| self.read_slow(tcm.now, addr)),
        }
    }

    /// The bytes an instruction fetch of `N` bytes at `addr` sees.
    // TODO(hw-unverified): a fetch inside the DTCM region, or from a load-mode ITCM, reads the
    // memory underneath (`arm9-memory-map.md`, "Unverified").
    fn fetch<const N: usize>(&self, tcm: &TcmConfig, addr: u32) -> [u8; N] {
        let addr = addr & !(N as u32 - 1);
        match tcm_offset(&tcm.itcm, ITCM_SIZE, addr) {
            Some(offset) if !tcm.itcm.load_mode => bytes_at(&self.itcm, offset),
            _ => self.read_page(tcm.now, addr),
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
        let page = page_of(&self.pages9, addr);
        let offset = page.offset as usize + (addr & PAGE_MASK) as usize;
        match page.region {
            // VRAM takes 16-bit and 32-bit writes only; a byte write is ignored
            // (`vram-banks.md`, "Behavior").
            Region::VramA if N == 1 => self.write_slow(tcm.now, addr, bytes),
            Region::Unmapped => self.write_slow(tcm.now, addr, bytes),
            region => self.put_region(region, offset, bytes),
        }
    }

    /// Writes `bytes` into the buffer behind `region`.
    fn put_region<const N: usize>(&mut self, region: Region, offset: usize, bytes: [u8; N]) {
        let buf = match region {
            Region::MainRam => &mut self.main_ram,
            Region::VramA => &mut self.vram_a,
            Region::SharedWram => &mut self.shared_wram,
            Region::Wram7 => &mut self.wram7,
            Region::Unmapped => return,
        };
        put_bytes(buf, offset, bytes);
    }

    /// The slow path for a write: I/O ports; an unmapped address ignores the write.
    // TODO(hw-unverified): a wider write is split into bytes and each goes to the register at
    // its own address; a byte on no register is dropped (`arm9-memory-map.md`, "Unverified").
    // TODO(hw-unverified): the I/O block is not mirrored (`arm9-memory-map.md`, "Unverified").
    // TODO(phase3): the I/O registers of the other blocks arrive with their phases.
    fn write_slow<const N: usize>(&mut self, now: u64, addr: u32, bytes: [u8; N]) {
        // IPCFIFOSEND takes the access as one word; it is not split into bytes.
        if self.send_word::<true, N>(addr, bytes) {
            return;
        }
        // Each byte goes to the register at its own address; an address no register holds
        // ignores it, as does a VRAM byte write that lands here.
        for (i, &value) in bytes.iter().enumerate() {
            let at = addr.wrapping_add(i as u32);
            match at {
                VRAMCNT_A_ADDR => self.write_vramcnt_a(value),
                WRAMCNT_ADDR => self.write_wramcnt(value),
                // TODO(phase4): VRAMCNT_G arrives with the other banks (#35).
                VRAMCNT_G_ADDR => {}
                _ => {
                    self.irq9.write_byte(at, value);
                    let requests = self.ipc.write_byte::<true>(at, value);
                    self.apply(requests);
                    self.gpu2d.write_byte(at, value);
                    self.timing.write_byte::<true>(at, value);
                    let requests = self.timers9.write_byte(at, value, now);
                    self.irq9.request(requests);
                }
            }
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
            && self.wram7.len() == WRAM7_SIZE
            && self.shared_wram.len() == SHARED_WRAM_SIZE
            && self.vramcnt_a & !VRAMCNT_USED == 0
            && self.wramcnt & !WRAMCNT_MODE_MASK == 0
            && self.irq9.is_well_formed()
            && self.irq7.is_well_formed()
            && self.ipc.is_well_formed()
            && self.timing.is_well_formed()
            && self.timers9.is_well_formed()
            && self.timers7.is_well_formed()
    }

    /// Stores VRAMCNT_A and rebuilds the page table, since the write can move the bank.
    // TODO(hw-unverified): bit 2 is dropped (`vram-banks.md`, "Unverified").
    fn write_vramcnt_a(&mut self, value: u8) {
        self.vramcnt_a = value & VRAMCNT_USED;
        self.build_page_table();
    }

    /// VCOUNT and each CPU's DISPSTAT.
    pub fn timing(&self) -> &Timing {
        &self.timing
    }

    pub(crate) fn timing_mut(&mut self) -> &mut Timing {
        &mut self.timing
    }

    /// The ARM9's interrupt controller.
    pub fn irq9(&self) -> &InterruptController<true> {
        &self.irq9
    }

    /// The ARM7's interrupt controller.
    pub fn irq7(&self) -> &InterruptController<false> {
        &self.irq7
    }

    /// The ARM9's timers.
    pub fn timers9(&self) -> &Timers<true> {
        &self.timers9
    }

    /// The ARM7's timers.
    pub fn timers7(&self) -> &Timers<false> {
        &self.timers7
    }

    /// The ARM9's timers, for the machine to settle an overflow or look at what changed.
    pub(crate) fn timers9_mut(&mut self) -> &mut Timers<true> {
        &mut self.timers9
    }

    /// The ARM7's timers, for the machine to settle an overflow or look at what changed.
    pub(crate) fn timers7_mut(&mut self) -> &mut Timers<false> {
        &mut self.timers7
    }

    /// The ARM9's interrupt controller, for a source to raise a request.
    pub(crate) fn irq9_mut(&mut self) -> &mut InterruptController<true> {
        &mut self.irq9
    }

    /// The ARM7's interrupt controller, for a source to raise a request.
    pub(crate) fn irq7_mut(&mut self) -> &mut InterruptController<false> {
        &mut self.irq7
    }

    /// Stores WRAMCNT (bits 2-7 dropped) and rebuilds the shared WRAM block of both page tables,
    /// since the write moves ownership. ARM9 only: the ARM7's slow path never reaches it.
    fn write_wramcnt(&mut self, value: u8) {
        self.wramcnt = value & WRAMCNT_MODE_MASK;
        self.build_wram_pages();
    }

    /// The bytes an ARM7 access of `N` bytes at `addr` sees (aligned down to `N`): main RAM,
    /// shared WRAM and its own WRAM through the ARM7 page table; WRAMSTAT and open bus (0)
    /// from the slow path.
    // TODO(hw-unverified): the rest of the ARM7 bus reads 0 (`wramcnt.md`, "Unverified").
    fn load7<const N: usize>(&self, now: u64, addr: u32) -> [u8; N] {
        let addr = addr & !(N as u32 - 1);
        self.page_bytes(&self.pages7, addr)
            .unwrap_or_else(|| self.read_slow7(now, addr))
    }

    /// [`Memory::load7`] as the CPU does it: a read of IPCFIFORECV takes a word out of the FIFO.
    fn load7_mut<const N: usize>(&mut self, now: u64, addr: u32) -> [u8; N] {
        let addr = addr & !(N as u32 - 1);
        match self.page_bytes(&self.pages7, addr) {
            Some(bytes) => bytes,
            None => self
                .receive_bytes::<false, N>(addr)
                .unwrap_or_else(|| self.read_slow7(now, addr)),
        }
    }

    /// Stores `N` bytes as the ARM7 (see [`Memory::load7`] for what it reaches); a store to
    /// open bus or an I/O port is dropped.
    // WRAMCNT is ARM9-only.
    fn store7<const N: usize>(&mut self, now: u64, addr: u32, bytes: [u8; N]) {
        let addr = addr & !(N as u32 - 1);
        let page = page_of(&self.pages7, addr);
        let offset = page.offset as usize + (addr & PAGE_MASK) as usize;
        match page.region {
            Region::Unmapped => self.write_slow7(now, addr, bytes),
            region => self.put_region(region, offset, bytes),
        }
    }

    /// The slow path for an ARM7 write: the interrupt controller; every other I/O port and
    /// unmapped address drops the write. Each byte goes to the register at its own address.
    // TODO(phase3): the ARM7's other I/O registers arrive with their issues (#26).
    fn write_slow7<const N: usize>(&mut self, now: u64, addr: u32, bytes: [u8; N]) {
        if self.send_word::<false, N>(addr, bytes) {
            return;
        }
        for (i, &value) in bytes.iter().enumerate() {
            let at = addr.wrapping_add(i as u32);
            self.irq7.write_byte(at, value);
            let requests = self.ipc.write_byte::<false>(at, value);
            self.apply(requests);
            self.timing.write_byte::<false>(at, value);
            let requests = self.timers7.write_byte(at, value, now);
            self.irq7.request(requests);
        }
    }

    /// Fetches an ARM opcode as the ARM7. Every ARM7 access takes `now`, the ARM7's clock in
    /// master cycles: a timer register reads the counter as of it (`timers.md`, "Model").
    pub fn fetch32_arm7(&self, now: u64, addr: u32) -> u32 {
        u32::from_le_bytes(self.load7(now, addr))
    }

    /// Fetches a Thumb opcode as the ARM7.
    pub fn fetch16_arm7(&self, now: u64, addr: u32) -> u16 {
        u16::from_le_bytes(self.load7(now, addr))
    }

    /// Reads a byte as the ARM7, without side effects (the peek; the CPU reads through
    /// [`Memory::read8_arm7_mut`]).
    pub fn read8_arm7(&self, now: u64, addr: u32) -> u8 {
        u8::from_le_bytes(self.load7(now, addr))
    }

    /// Reads a halfword as the ARM7, without side effects.
    pub fn read16_arm7(&self, now: u64, addr: u32) -> u16 {
        u16::from_le_bytes(self.load7(now, addr))
    }

    /// Reads a word as the ARM7, without side effects.
    pub fn read32_arm7(&self, now: u64, addr: u32) -> u32 {
        u32::from_le_bytes(self.load7(now, addr))
    }

    /// Writes a byte as the ARM7.
    pub fn write8_arm7(&mut self, now: u64, addr: u32, value: u8) {
        self.store7(now, addr, value.to_le_bytes());
    }

    /// Writes a halfword as the ARM7.
    pub fn write16_arm7(&mut self, now: u64, addr: u32, value: u16) {
        self.store7(now, addr, value.to_le_bytes());
    }

    /// Writes a word as the ARM7.
    pub fn write32_arm7(&mut self, now: u64, addr: u32, value: u32) {
        self.store7(now, addr, value.to_le_bytes());
    }

    /// Fetches an ARM opcode.
    pub fn fetch32(&self, tcm: &TcmConfig, addr: u32) -> u32 {
        u32::from_le_bytes(self.fetch(tcm, addr))
    }

    /// Fetches a Thumb opcode.
    pub fn fetch16(&self, tcm: &TcmConfig, addr: u32) -> u16 {
        u16::from_le_bytes(self.fetch(tcm, addr))
    }

    /// Reads a byte of data without side effects: the peek the debugger and the runner use. The
    /// CPUs read through [`Memory::read8_mut`] and its siblings.
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

    /// Reads a byte of data as the CPU does: a read of IPCFIFORECV takes a word out of the
    /// FIFO and can set the error flag. The plain [`Memory::read8`] is the peek.
    pub fn read8_mut(&mut self, tcm: &TcmConfig, addr: u32) -> u8 {
        u8::from_le_bytes(self.load_mut(tcm, addr))
    }

    /// Reads a halfword of data as the CPU does (see [`Memory::read8_mut`]).
    pub fn read16_mut(&mut self, tcm: &TcmConfig, addr: u32) -> u16 {
        u16::from_le_bytes(self.load_mut(tcm, addr))
    }

    /// Reads a word of data as the CPU does (see [`Memory::read8_mut`]).
    pub fn read32_mut(&mut self, tcm: &TcmConfig, addr: u32) -> u32 {
        u32::from_le_bytes(self.load_mut(tcm, addr))
    }

    /// Reads a byte as the ARM7 does (see [`Memory::read8_mut`]).
    pub fn read8_arm7_mut(&mut self, now: u64, addr: u32) -> u8 {
        u8::from_le_bytes(self.load7_mut(now, addr))
    }

    /// Reads a halfword as the ARM7 does (see [`Memory::read8_mut`]).
    pub fn read16_arm7_mut(&mut self, now: u64, addr: u32) -> u16 {
        u16::from_le_bytes(self.load7_mut(now, addr))
    }

    /// Reads a word as the ARM7 does (see [`Memory::read8_mut`]).
    pub fn read32_arm7_mut(&mut self, now: u64, addr: u32) -> u32 {
        u32::from_le_bytes(self.load7_mut(now, addr))
    }

    /// Replaces the IPC block, to build a corrupt state in a test.
    #[cfg(test)]
    pub(crate) fn replace_ipc_for_test(&mut self, ipc: Ipc) {
        self.ipc = ipc;
    }

    /// IPCSYNC and the two IPC FIFOs, for the debugger.
    pub fn ipc(&self) -> &Ipc {
        &self.ipc
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

    #[test]
    fn given_an_ie_bit_outside_the_arm9_mask_when_checked_then_it_is_not_well_formed() {
        let mut memory = Memory::new();
        memory.irq9 = InterruptController::with_unchecked_ie(1 << 7);
        assert!(!memory.is_well_formed());
    }

    #[test]
    fn given_wramcnt_with_bits_2_to_7_set_when_checked_then_it_is_not_well_formed() {
        let mut memory = Memory::new();
        memory.wramcnt = 0x04;
        assert!(!memory.is_well_formed());
    }

    #[test]
    fn given_wramcnt_3_when_checked_then_it_is_well_formed() {
        let mut memory = Memory::new();
        memory.wramcnt = 3;
        assert!(memory.is_well_formed());
    }

    #[test]
    fn given_a_short_shared_wram_when_checked_then_it_is_not_well_formed() {
        let mut memory = Memory::new();
        memory.shared_wram.pop();
        assert!(!memory.is_well_formed());
    }

    #[test]
    fn given_a_long_shared_wram_when_checked_then_it_is_not_well_formed() {
        let mut memory = Memory::new();
        memory.shared_wram.push(0);
        assert!(!memory.is_well_formed());
    }
}
