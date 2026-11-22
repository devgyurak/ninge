// SPDX-License-Identifier: Apache-2.0
//! The address map: [`decode`] turns an address into the [`Owner`] that holds it and an offset into
//! that owner's own register space, once per access, from the per-CPU tables [`ARM9_PORTS`] and
//! [`ARM7_PORTS`]. The map lives here and nowhere else; a test pins that no two entries overlap.

use crate::bus::VRAMCNT_A_ADDR;
use crate::gpu2d::{
    AFFINE_REGS_ADDR, AFFINE_REGS_B_ADDR, AFFINE_REGS_BYTES, BG_REGS_ADDR, BG_REGS_B_ADDR,
    BG_REGS_BYTES, BLD_REGS_ADDR, BLD_REGS_B_ADDR, BLD_REGS_BYTES, DISP_MMEM_FIFO_ADDR,
    DISP_MMEM_FIFO_BYTES, DISPCAPCNT_ADDR, DISPCAPCNT_BYTES, DISPCNT_A_ADDR, DISPCNT_B_ADDR,
    MASTER_BRIGHT_ADDR, MASTER_BRIGHT_B_ADDR, MASTER_BRIGHT_BYTES, MOSAIC_ADDR, MOSAIC_B_ADDR,
    MOSAIC_BYTES, POWCNT1_ADDR, WINDOW_REGS_ADDR, WINDOW_REGS_B_ADDR, WINDOW_REGS_BYTES,
};
use crate::gpu3d::{
    DISP3DCNT_ADDR, DISP3DCNT_BYTES, GEOMETRY_ADDR, GEOMETRY_BYTES, RENDER_ADDR, RENDER_BYTES,
};
use crate::io::cart::{
    AUXSPICNT_ADDR, BLOCK_LEN as CART_BYTES, DATA_ADDR as CART_DATA_ADDR, EXMEMCNT_ADDR,
};
use crate::io::divsqrt::{BLOCK_BYTES as DIVSQRT_BYTES, DIVCNT_ADDR};
use crate::io::dma::{CHANNELS_BYTES, DMA0FILL_ADDR, DMA0SAD_ADDR, FILL_BYTES};
use crate::io::ipc::{IPCFIFORECV_ADDR, IPCSYNC_ADDR};
use crate::io::irq::IME_ADDR;
use crate::io::keypad::{ARM7_BYTES as KEYPAD7_BYTES, ARM9_BYTES as KEYPAD9_BYTES, KEYINPUT_ADDR};
use crate::io::power::{BLOCK_BYTES as POWER_BYTES, POSTFLG_ADDR};
use crate::io::rtc::{BLOCK_BYTES as RTC_BYTES, RTCCNT_ADDR};
use crate::io::spi::{BLOCK_BYTES as SPI_BYTES, SPICNT_ADDR};
use crate::io::timers::TM0CNT_L_ADDR;
use crate::io::timing::DISPSTAT_ADDR;
use crate::spu::{BLOCK_BYTES as SPU_BYTES, SOUND_REGS_ADDR};

/// Who owns an address, and where in its register space: the memory-control block (VRAMCNT_A..I,
/// WRAMCNT, WRAMSTAT) stays in `Memory` because the page tables derive from it (#109), every
/// other register belongs to a device behind a [`Port`]. [`decode`] says which, so no `Io` access
/// ever sees an address `Memory` owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    /// `Memory`, at this many bytes past VRAMCNT_A.
    Memory(u32),
    /// A device, at this offset into its register space ([`Port::base`] is offset 0).
    Io(Port, u32),
}

/// Which device holds a register. [`Port::base`] is where its register space starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Port {
    /// DISPCNT, the BG control and scroll registers of both engines, and POWCNT1 (nine table
    /// entries, one device).
    Gpu2d,
    /// DISPSTAT and VCOUNT.
    Timing,
    /// IME, IE and IF.
    Irq,
    /// IPCSYNC, IPCFIFOCNT and IPCFIFOSEND.
    Ipc,
    /// IPCFIFORECV: its own entry, because it lives in a different block, but the same device.
    IpcRecv,
    /// The four timers.
    Timers,
    /// The four DMA channels' address, count and control registers.
    Dma,
    /// The ARM9's DMA fill registers: their own entry, because they live in a different block,
    /// but the same device as [`Port::Dma`].
    DmaFill,
    /// KEYINPUT and KEYCNT, and on the ARM7 EXTKEYIN.
    Keypad,
    /// DIVCNT to SQRT_PARAM, ARM9 only.
    DivSqrt,
    /// DISP3DCNT, the 3D rendering block and the geometry block, ARM9 only (three table entries,
    /// one device).
    Gpu3d,
    /// HALTCNT and POWCNT2, ARM7 only.
    Power,
    /// The sound channels, SOUNDCNT, SOUNDBIAS and the capture registers, ARM7 only.
    Spu,
    /// SPICNT and SPIDATA, ARM7 only.
    Spi,
    /// RTCCNT, ARM7 only.
    Rtc,
    /// AUXSPICNT, AUXSPIDATA, ROMCTRL and the command registers of the cartridge slot.
    Cart,
    /// EXMEMCNT / EXMEMSTAT: its own entry, because it lives in a different block, but the same
    /// device as [`Port::Cart`].
    CartExmem,
    /// The cartridge data port: its own entry, in a different block, the same device as
    /// [`Port::Cart`].
    CartData,
}

impl Port {
    /// The address that is offset 0 of the device behind this port. A device with two table
    /// entries ([`Port::Gpu2d`]) or two ports ([`Port::Ipc`], [`Port::IpcRecv`]) has one offset
    /// space across them.
    pub(crate) const fn base(self) -> u32 {
        match self {
            Self::Gpu2d => DISPCNT_A_ADDR,
            Self::Timing => DISPSTAT_ADDR,
            Self::Irq => IME_ADDR,
            Self::Ipc | Self::IpcRecv => IPCSYNC_ADDR,
            Self::Timers => TM0CNT_L_ADDR,
            Self::Dma | Self::DmaFill => DMA0SAD_ADDR,
            Self::DivSqrt => DIVCNT_ADDR,
            Self::Gpu3d => DISP3DCNT_ADDR,
            Self::Keypad => KEYINPUT_ADDR,
            Self::Power => POSTFLG_ADDR,
            Self::Spu => SOUND_REGS_ADDR,
            Self::Spi => SPICNT_ADDR,
            Self::Rtc => RTCCNT_ADDR,
            Self::Cart | Self::CartExmem | Self::CartData => AUXSPICNT_ADDR,
        }
    }
}

/// One entry of the address map: the first address, the length in bytes, and the device that owns
/// it, or `None` for the memory-control block that `Memory` owns.
pub(crate) type PortEntry = (u32, u32, Option<Port>);

/// What the ARM9 sees, in ascending address order. Every entry is a 4-byte-aligned block whose
/// length is a multiple of 4, so one aligned access never spans two entries. Holes inside an
/// entry belong to its device, which reads them 0 and drops writes to them.
// TODO(hw-unverified): the I/O block is not mirrored (`arm9-memory-map.md`, "Unverified").
// TODO(phase3): the I/O registers of the other blocks arrive with their issues.
pub(crate) const ARM9_PORTS: [PortEntry; 33] = [
    (DISPCNT_A_ADDR, 4, Some(Port::Gpu2d)),
    (DISPSTAT_ADDR, 4, Some(Port::Timing)),
    // BG0CNT-BG3CNT and the scroll registers of BG0-BG3, in the same device as DISPCNT.
    (BG_REGS_ADDR, BG_REGS_BYTES, Some(Port::Gpu2d)),
    // BG2PA..BG3Y, the transform registers of the two affine layers.
    (AFFINE_REGS_ADDR, AFFINE_REGS_BYTES, Some(Port::Gpu2d)),
    // WIN0H..WINOUT, the window registers.
    (WINDOW_REGS_ADDR, WINDOW_REGS_BYTES, Some(Port::Gpu2d)),
    // MOSAIC, a halfword at the start of its block.
    (MOSAIC_ADDR, MOSAIC_BYTES, Some(Port::Gpu2d)),
    // BLDCNT, BLDALPHA and BLDY, the colour special effects.
    (BLD_REGS_ADDR, BLD_REGS_BYTES, Some(Port::Gpu2d)),
    // DISP3DCNT, a halfword and two unused bytes.
    (DISP3DCNT_ADDR, DISP3DCNT_BYTES, Some(Port::Gpu3d)),
    // DISPCAPCNT, the display capture control register.
    (DISPCAPCNT_ADDR, DISPCAPCNT_BYTES, Some(Port::Gpu2d)),
    // DISP_MMEM_FIFO, the main-memory display FIFO.
    (DISP_MMEM_FIFO_ADDR, DISP_MMEM_FIFO_BYTES, Some(Port::Gpu2d)),
    // MASTER_BRIGHT, a halfword at the start of its block.
    (MASTER_BRIGHT_ADDR, MASTER_BRIGHT_BYTES, Some(Port::Gpu2d)),
    // DMA0SAD..DMA3CNT, twelve bytes a channel, then the fill registers.
    (DMA0SAD_ADDR, CHANNELS_BYTES, Some(Port::Dma)),
    (DMA0FILL_ADDR, FILL_BYTES, Some(Port::DmaFill)),
    (TM0CNT_L_ADDR, 16, Some(Port::Timers)),
    // DIVCNT, the divider's registers, SQRTCNT and the square root's, with holes between.
    (DIVCNT_ADDR, DIVSQRT_BYTES, Some(Port::DivSqrt)),
    // KEYINPUT and KEYCNT.
    (KEYINPUT_ADDR, KEYPAD9_BYTES, Some(Port::Keypad)),
    // IPCSYNC, IPCFIFOCNT and IPCFIFOSEND, 4 bytes each.
    (IPCSYNC_ADDR, 12, Some(Port::Ipc)),
    // IME, a hole, IE and IF.
    (IME_ADDR, 16, Some(Port::Irq)),
    // VRAMCNT_A..D, VRAMCNT_E, F, G, WRAMCNT, then VRAMCNT_H and I and two bytes no register
    // holds (the block stays on the 4-byte grid).
    (VRAMCNT_A_ADDR, 12, None),
    (POWCNT1_ADDR, 4, Some(Port::Gpu2d)),
    // The 3D rendering registers, RDLINES_COUNT to TOON_TABLE, and the geometry engine's GXFIFO,
    // command ports, GXSTAT and result registers.
    (RENDER_ADDR, RENDER_BYTES, Some(Port::Gpu3d)),
    (GEOMETRY_ADDR, GEOMETRY_BYTES, Some(Port::Gpu3d)),
    // Engine B's block, the same layout 1000h on: DISPCNT, then the BG control and scroll registers.
    (DISPCNT_B_ADDR, 4, Some(Port::Gpu2d)),
    (BG_REGS_B_ADDR, BG_REGS_BYTES, Some(Port::Gpu2d)),
    (AFFINE_REGS_B_ADDR, AFFINE_REGS_BYTES, Some(Port::Gpu2d)),
    (WINDOW_REGS_B_ADDR, WINDOW_REGS_BYTES, Some(Port::Gpu2d)),
    (MOSAIC_B_ADDR, MOSAIC_BYTES, Some(Port::Gpu2d)),
    (BLD_REGS_B_ADDR, BLD_REGS_BYTES, Some(Port::Gpu2d)),
    (MASTER_BRIGHT_B_ADDR, MASTER_BRIGHT_BYTES, Some(Port::Gpu2d)),
    (IPCFIFORECV_ADDR, 4, Some(Port::IpcRecv)),
    // AUXSPICNT, AUXSPIDATA, ROMCTRL and the command registers; EXMEMCNT; the data port.
    (AUXSPICNT_ADDR, CART_BYTES, Some(Port::Cart)),
    (EXMEMCNT_ADDR, 4, Some(Port::CartExmem)),
    (CART_DATA_ADDR, 4, Some(Port::CartData)),
];

/// What the ARM7 sees: the same blocks, without the 2D engines, and WRAMSTAT in place of the ARM9's
/// memory-control block.
pub(crate) const ARM7_PORTS: [PortEntry; 15] = [
    (DISPSTAT_ADDR, 4, Some(Port::Timing)),
    (DMA0SAD_ADDR, CHANNELS_BYTES, Some(Port::Dma)),
    (TM0CNT_L_ADDR, 16, Some(Port::Timers)),
    // KEYINPUT, KEYCNT, a hole (RCNT) and EXTKEYIN.
    (KEYINPUT_ADDR, KEYPAD7_BYTES, Some(Port::Keypad)),
    // RTCCNT, right after EXTKEYIN.
    (RTCCNT_ADDR, RTC_BYTES, Some(Port::Rtc)),
    (IPCSYNC_ADDR, 12, Some(Port::Ipc)),
    (IME_ADDR, 16, Some(Port::Irq)),
    // POSTFLG, HALTCNT and POWCNT2.
    (POSTFLG_ADDR, POWER_BYTES, Some(Port::Power)),
    // The sound registers. The list is not strictly ascending: the entries keep the places of
    // the blocks they belong to.
    (SOUND_REGS_ADDR, SPU_BYTES, Some(Port::Spu)),
    // WRAMSTAT sits in the second byte.
    (VRAMCNT_A_ADDR, 4, None),
    (IPCFIFORECV_ADDR, 4, Some(Port::IpcRecv)),
    (AUXSPICNT_ADDR, CART_BYTES, Some(Port::Cart)),
    // SPICNT and SPIDATA, after the cartridge block.
    (SPICNT_ADDR, SPI_BYTES, Some(Port::Spi)),
    (EXMEMCNT_ADDR, 4, Some(Port::CartExmem)),
    (CART_DATA_ADDR, 4, Some(Port::CartData)),
];

/// The owner of `addr` for CPU `ARM9` and its offset there, or `None` when nothing holds it (it
/// reads 0 and ignores writes).
// TODO(hw-unverified): an undefined I/O port reads 0 (GBATEK says so); unmapped memory is
// "undefined" there and reads 0 here (`arm9-memory-map.md`, "Unverified").
pub(crate) fn decode<const ARM9: bool>(addr: u32) -> Option<Owner> {
    let ports: &[PortEntry] = if ARM9 { &ARM9_PORTS } else { &ARM7_PORTS };
    ports.iter().find_map(|&(start, len, port)| {
        let within = addr.checked_sub(start).filter(|&d| d < len)?;
        Some(match port {
            Some(port) => Owner::Io(port, start.wrapping_sub(port.base()).wrapping_add(within)),
            None => Owner::Memory(start.wrapping_sub(VRAMCNT_A_ADDR).wrapping_add(within)),
        })
    })
}
