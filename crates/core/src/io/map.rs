// SPDX-License-Identifier: Apache-2.0
//! The address map: [`decode`] turns an address into the [`Owner`] that holds it and an offset into
//! that owner's own register space, once per access, from the per-CPU tables [`ARM9_PORTS`] and
//! [`ARM7_PORTS`]. The map lives here and nowhere else; a test pins that no two entries overlap.

use crate::bus::VRAMCNT_A_ADDR;
use crate::dma::{CHANNELS_BYTES, DMA0FILL_ADDR, DMA0SAD_ADDR, FILL_BYTES};
use crate::gpu2d::timing::DISPSTAT_ADDR;
use crate::gpu2d::{DISPCNT_A_ADDR, POWCNT1_ADDR};
use crate::ipc::{IPCFIFORECV_ADDR, IPCSYNC_ADDR};
use crate::irq::IME_ADDR;
use crate::timers::TM0CNT_L_ADDR;

/// Who owns an address, and where in its register space: the memory-control block (VRAMCNT_A..G,
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
    /// DISPCNT of engine A and POWCNT1 (two table entries, one device).
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
pub(crate) const ARM9_PORTS: [PortEntry; 10] = [
    (DISPCNT_A_ADDR, 4, Some(Port::Gpu2d)),
    (DISPSTAT_ADDR, 4, Some(Port::Timing)),
    // DMA0SAD..DMA3CNT, twelve bytes a channel, then the fill registers.
    (DMA0SAD_ADDR, CHANNELS_BYTES, Some(Port::Dma)),
    (DMA0FILL_ADDR, FILL_BYTES, Some(Port::DmaFill)),
    (TM0CNT_L_ADDR, 16, Some(Port::Timers)),
    // IPCSYNC, IPCFIFOCNT and IPCFIFOSEND, 4 bytes each.
    (IPCSYNC_ADDR, 12, Some(Port::Ipc)),
    // IME, a hole, IE and IF.
    (IME_ADDR, 16, Some(Port::Irq)),
    // VRAMCNT_A..D and VRAMCNT_E, F, G and WRAMCNT.
    (VRAMCNT_A_ADDR, 8, None),
    (POWCNT1_ADDR, 4, Some(Port::Gpu2d)),
    (IPCFIFORECV_ADDR, 4, Some(Port::IpcRecv)),
];

/// What the ARM7 sees: the same blocks, without engine A, and WRAMSTAT in place of the ARM9's
/// memory-control block.
pub(crate) const ARM7_PORTS: [PortEntry; 7] = [
    (DISPSTAT_ADDR, 4, Some(Port::Timing)),
    (DMA0SAD_ADDR, CHANNELS_BYTES, Some(Port::Dma)),
    (TM0CNT_L_ADDR, 16, Some(Port::Timers)),
    (IPCSYNC_ADDR, 12, Some(Port::Ipc)),
    (IME_ADDR, 16, Some(Port::Irq)),
    // WRAMSTAT sits in the second byte.
    (VRAMCNT_A_ADDR, 4, None),
    (IPCFIFORECV_ADDR, 4, Some(Port::IpcRecv)),
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
