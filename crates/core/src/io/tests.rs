// SPDX-License-Identifier: Apache-2.0
//! Tests of `Io`, the address map and the device interface.

use super::device::{Effect, Width, read_bytes, write_bytes};
use super::map::{ARM7_PORTS, ARM9_PORTS, Owner, Port, PortEntry, decode};
use super::*;
use crate::bus::VRAMCNT_A_ADDR;
use crate::gpu2d::{DISPCNT_A_ADDR, POWCNT1_ADDR};
use crate::io::dma::{DMA0FILL_ADDR, DMA0SAD_ADDR};
use crate::io::ipc::{IPCFIFOCNT_ADDR, IPCFIFORECV_ADDR, IPCFIFOSEND_ADDR, IPCSYNC_ADDR};
use crate::io::irq::{IE_ADDR, IF_ADDR, IME_ADDR, InterruptController, Requests};
use crate::io::timers::TM0CNT_L_ADDR;
use crate::io::timing::{DISPSTAT_ADDR, VCOUNT_ADDR};

/// The WRAMCNT, WRAMSTAT and VRAMCNT_G addresses, which `Memory` keeps private.
const VRAMCNT_G: u32 = 0x0400_0246;
const WRAMCNT: u32 = 0x0400_0247;
const WRAMSTAT: u32 = 0x0400_0241;

/// The pairs of entries in `table` that share an address, as (first start, second start).
fn overlapping_pairs(table: &[PortEntry]) -> Vec<(u32, u32)> {
    table
        .iter()
        .enumerate()
        .flat_map(|(i, &(a, a_len, _))| {
            table[i + 1..]
                .iter()
                .filter(move |&&(b, b_len, _)| a < b + b_len && b < a + a_len)
                .map(move |&(b, ..)| (a, b))
        })
        .collect()
}

#[test]
fn given_the_io_port_tables_when_compared_pairwise_then_no_two_ports_overlap() {
    assert_eq!(
        (
            overlapping_pairs(&ARM9_PORTS),
            overlapping_pairs(&ARM7_PORTS)
        ),
        (Vec::<(u32, u32)>::new(), Vec::<(u32, u32)>::new())
    );
}

/// The entries of `table` that break the shape the decoder relies on: a start or a length
/// off the 4-byte grid, an empty block, or a start below its port's base.
fn malformed_entries(table: &[PortEntry]) -> Vec<(u32, u32)> {
    table
        .iter()
        .filter(|&&(start, len, port)| {
            start % 4 != 0
                || len % 4 != 0
                || len == 0
                || start < port.map_or(VRAMCNT_A_ADDR, Port::base)
        })
        .map(|&(start, len, _)| (start, len))
        .collect()
}

#[test]
fn given_the_io_port_tables_when_each_entry_is_checked_then_blocks_are_aligned_and_above_their_base()
 {
    assert_eq!(
        (
            malformed_entries(&ARM9_PORTS),
            malformed_entries(&ARM7_PORTS)
        ),
        (Vec::<(u32, u32)>::new(), Vec::<(u32, u32)>::new())
    );
}

/// The cases where `decode` answers something other than `expected`, as (address, got).
fn decode_failures<const ARM9: bool>(
    expected: &[(u32, Option<Owner>)],
) -> Vec<(u32, Option<Owner>)> {
    expected
        .iter()
        .filter(|&&(addr, want)| decode::<ARM9>(addr) != want)
        .map(|&(addr, _)| (addr, decode::<ARM9>(addr)))
        .collect()
}

#[test]
fn given_each_arm9_register_address_when_decoded_then_the_expected_port_and_offset_answer() {
    let expected = [
        (DISPCNT_A_ADDR, Some(Owner::Io(Port::Gpu2d, 0))),
        (DISPCNT_A_ADDR + 3, Some(Owner::Io(Port::Gpu2d, 3))),
        (
            POWCNT1_ADDR,
            Some(Owner::Io(Port::Gpu2d, POWCNT1_ADDR - DISPCNT_A_ADDR)),
        ),
        (DISPSTAT_ADDR, Some(Owner::Io(Port::Timing, 0))),
        (VCOUNT_ADDR, Some(Owner::Io(Port::Timing, 2))),
        (TM0CNT_L_ADDR, Some(Owner::Io(Port::Timers, 0))),
        (TM0CNT_L_ADDR + 15, Some(Owner::Io(Port::Timers, 15))),
        // DMA0SAD..DMA3CNT, 12 bytes a channel, then DMA0FILL..DMA3FILL (`dma.md`).
        (DMA0SAD_ADDR, Some(Owner::Io(Port::Dma, 0))),
        (0x0400_00B8, Some(Owner::Io(Port::Dma, 8))),
        (0x0400_00DF, Some(Owner::Io(Port::Dma, 0x2F))),
        (DMA0FILL_ADDR, Some(Owner::Io(Port::DmaFill, 0x30))),
        (0x0400_00EF, Some(Owner::Io(Port::DmaFill, 0x3F))),
        (IPCSYNC_ADDR, Some(Owner::Io(Port::Ipc, 0))),
        (IPCFIFOCNT_ADDR, Some(Owner::Io(Port::Ipc, 4))),
        (IPCFIFOSEND_ADDR, Some(Owner::Io(Port::Ipc, 8))),
        (
            IPCFIFORECV_ADDR,
            Some(Owner::Io(Port::IpcRecv, IPCFIFORECV_ADDR - IPCSYNC_ADDR)),
        ),
        (IME_ADDR, Some(Owner::Io(Port::Irq, 0))),
        (IE_ADDR, Some(Owner::Io(Port::Irq, 8))),
        (IF_ADDR, Some(Owner::Io(Port::Irq, 12))),
        (VRAMCNT_A_ADDR, Some(Owner::Memory(0))),
        (VRAMCNT_G, Some(Owner::Memory(6))),
        (WRAMCNT, Some(Owner::Memory(7))),
    ];
    assert_eq!(decode_failures::<true>(&expected), vec![]);
}

#[test]
fn given_each_arm7_register_address_when_decoded_then_the_expected_port_and_offset_answer() {
    let expected = [
        (DISPSTAT_ADDR, Some(Owner::Io(Port::Timing, 0))),
        (VCOUNT_ADDR, Some(Owner::Io(Port::Timing, 2))),
        (TM0CNT_L_ADDR + 4, Some(Owner::Io(Port::Timers, 4))),
        // The ARM7 has the DMA channels but no fill registers (`dma.md`, "Registers").
        (DMA0SAD_ADDR, Some(Owner::Io(Port::Dma, 0))),
        (0x0400_00DF, Some(Owner::Io(Port::Dma, 0x2F))),
        (IPCSYNC_ADDR, Some(Owner::Io(Port::Ipc, 0))),
        (IPCFIFOSEND_ADDR, Some(Owner::Io(Port::Ipc, 8))),
        (
            IPCFIFORECV_ADDR,
            Some(Owner::Io(Port::IpcRecv, IPCFIFORECV_ADDR - IPCSYNC_ADDR)),
        ),
        (IME_ADDR, Some(Owner::Io(Port::Irq, 0))),
        (IF_ADDR, Some(Owner::Io(Port::Irq, 12))),
        (WRAMSTAT, Some(Owner::Memory(1))),
    ];
    assert_eq!(decode_failures::<false>(&expected), vec![]);
}

#[test]
fn given_arm9_only_registers_when_the_arm7_decodes_them_then_no_port_owns_them() {
    let expected = [
        (DISPCNT_A_ADDR, None),
        (POWCNT1_ADDR, None),
        (WRAMCNT, None),
        (VRAMCNT_G, None),
        (0x0400_00E0, None),
        (0x0400_00EF, None),
    ];
    assert_eq!(decode_failures::<false>(&expected), vec![]);
}

#[test]
fn given_addresses_next_to_the_blocks_when_decoded_then_no_port_owns_them() {
    // Just below and above IME/IE/IF (208-217), the timers (100-10F), IPCSYNC..SEND
    // (180-18B), the 2D registers and the cart-side neighbour of IPCFIFORECV.
    let expected = [
        (0x0400_0207, None),
        (0x0400_0218, None),
        (0x0400_021B, None),
        (0x0400_0200, None),
        (0x0400_0300, None),
        (TM0CNT_L_ADDR - 1, None),
        (TM0CNT_L_ADDR + 16, None),
        // Just below DMA0SAD and just above the fill block, which ends at 0400_00EFh.
        (0x0400_00AF, None),
        (0x0400_00F0, None),
        (IPCSYNC_ADDR - 1, None),
        (IPCFIFOSEND_ADDR + 4, None),
        (IPCFIFORECV_ADDR + 4, None),
        (0x0400_0008, None),
        (0x0000_0000, None),
    ];
    assert_eq!(
        (
            decode_failures::<true>(&expected),
            decode_failures::<false>(&expected)
        ),
        (vec![], vec![])
    );
}

#[test]
fn given_every_byte_of_ime_ie_and_if_when_decoded_then_the_irq_port_owns_it() {
    let unowned: Vec<u32> = (0..4)
        .flat_map(|i| [IME_ADDR + i, IE_ADDR + i, IF_ADDR + i])
        .filter(|&addr| !matches!(decode::<true>(addr), Some(Owner::Io(Port::Irq, _))))
        .collect();
    assert_eq!(unowned, Vec::<u32>::new());
}

#[test]
fn given_a_word_at_the_second_memory_control_block_when_decoded_then_memory_owns_all_four_bytes() {
    // VRAMCNT_E/F/G and WRAMCNT share one aligned word, so one access never splits.
    let owners: Vec<Option<Owner>> = (0..4).map(|i| decode::<true>(0x0400_0244 + i)).collect();
    assert_eq!(
        owners,
        (4..8).map(|i| Some(Owner::Memory(i))).collect::<Vec<_>>()
    );
}

#[test]
fn given_a_word_the_width_helpers_split_when_it_is_joined_again_then_it_round_trips() {
    let split = write_bytes(0, Width::Word, 0xDDCC_BBAA, |_, _| Effect::default());
    let joined = read_bytes(0, Width::Word, |at| [0xAA, 0xBB, 0xCC, 0xDD][at as usize]);
    assert_eq!((split, joined), (Effect::default(), 0xDDCC_BBAA));
}

#[test]
fn given_a_halfword_write_when_split_then_each_byte_goes_to_its_own_offset_in_order() {
    let mut seen = Vec::new();
    write_bytes(2, Width::Half, 0xBEEF, |at, byte| {
        seen.push((at, byte));
        Effect::default()
    });
    assert_eq!(seen, vec![(2, 0xEF), (3, 0xBE)]);
}

#[test]
fn given_two_effects_when_merged_then_requests_and_reschedule_are_ored() {
    let merged = Effect {
        requests: Requests { arm9: 1, arm7: 0 },
        reschedule: 0b01,
    }
    .merge(Effect {
        requests: Requests { arm9: 2, arm7: 4 },
        reschedule: 0b10,
    });
    assert_eq!(
        merged,
        Effect {
            requests: Requests { arm9: 3, arm7: 4 },
            reschedule: 0b11,
        }
    );
}

#[test]
fn given_power_on_devices_when_checked_then_they_are_well_formed() {
    assert!(Io::new().is_well_formed(0));
}

#[test]
fn given_an_ie_bit_outside_the_arm9_mask_when_checked_then_the_devices_are_not_well_formed() {
    let mut io = Io::new();
    io.irq9 = InterruptController::with_unchecked_ie(1 << 7);
    assert!(!io.is_well_formed(0));
}

#[test]
fn given_line_and_hblank_events_when_fired_through_io_then_no_transfer_comes_back() {
    let mut io = Io::new();
    let mut scheduler = Scheduler::new();
    assert_eq!(
        (
            io.fire(&mut scheduler, EventKind::LineStart),
            io.fire(&mut scheduler, EventKind::HBlankStart)
        ),
        (None, None)
    );
}

#[test]
fn given_dma_start_events_for_channels_with_no_start_waiting_when_fired_then_no_transfer_comes_back()
 {
    // A start that was cancelled leaves nothing to run (`dma.md`, "Enabling").
    let mut io = Io::new();
    let mut scheduler = Scheduler::new();
    let kind = |arm9, channel| EventKind::DmaStart { arm9, channel };
    assert_eq!(
        (
            io.fire(&mut scheduler, kind(true, 0)),
            io.fire(&mut scheduler, kind(false, 3))
        ),
        (None, None)
    );
}

/// The times of the ARM9 channel 0 start event after the channel was set to wait for VBlank
/// and VBlank was raised at `time`, with the master clock at `now`.
fn vblank_start_times(now: u64, time: u64) -> Vec<u64> {
    const ENABLE: u32 = 1 << 31;
    const ARM9_VBLANK: u32 = 1 << 27;
    let mut io = Io::new();
    let mut scheduler = Scheduler::new();
    scheduler.advance(now);
    let at = At {
        now,
        floor: now + 1,
    };
    io.write::<true>(
        &mut scheduler,
        at,
        Port::Dma,
        8,
        Width::Word,
        ENABLE | ARM9_VBLANK | 1,
    );
    io.trigger_dma(&mut scheduler, Trigger::VBlank, time);
    let kind = EventKind::DmaStart {
        arm9: true,
        channel: 0,
    };
    scheduler
        .pending()
        .iter()
        .filter(|event| event.kind == kind)
        .map(|event| event.time)
        .collect()
}

#[test]
fn given_a_vblank_raised_before_the_master_clock_when_triggered_then_the_start_is_queued_at_the_clock()
 {
    // An event is never before `now` (`scheduler.md`, "Savestate checks").
    assert_eq!(vblank_start_times(100, 40), vec![100]);
}

#[test]
fn given_a_vblank_raised_after_the_master_clock_when_triggered_then_the_start_is_queued_at_that_time()
 {
    assert_eq!(vblank_start_times(100, 150), vec![150]);
}

#[test]
fn given_requests_for_each_cpu_when_raised_then_each_controller_latches_only_its_own() {
    let mut io = Io::new();
    io.request_irq(Requests {
        arm9: 0b01,
        arm7: 0b10,
    });
    assert_eq!((io.irq9().if_(), io.irq7().if_()), (0b01, 0b10));
}
