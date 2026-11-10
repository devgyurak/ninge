// SPDX-License-Identifier: Apache-2.0
//! The DMA channels on their own. Expected values come from `docs/hw/dma.md`: "Registers" (the
//! 12-byte channel blocks at `0400_00B0h`, DMAnFILL at `0400_00E0h` on the ARM9 only), "Address
//! masks", "Word count" (21 bits ARM9; 14 bits ARM7 channels 0-2, 16 bits channel 3; count 0 is
//! the field's top plus one), "DMAnCNT control bits", "Start timing", "Enabling" (the latch on a
//! 0 to 1 enable), "Starting", "Transferring" (read then write per unit, step by the unit's size,
//! aligned down), "The end of a transfer" and "Model" (one start slot per channel). Tests named
//! `..._unverified` pin a modeled choice from "Unverified", not a hardware fact.
//!
//! Offsets are into the device's register space, which starts at DMA0SAD: channel n's SAD, DAD
//! and CNT are at `12n`, `12n + 4` and `12n + 8`, and DMAnFILL is at `0x30 + 4n`.

use super::*;
use std::collections::BTreeMap;

const ENABLE: u32 = 1 << 31;
const IRQ: u32 = 1 << 30;
const WORD: u32 = 1 << 26;
const REPEAT: u32 = 1 << 25;
/// IF bit 8, DMA channel 0 (`dma.md`, "IF bits").
const IF_DMA0: u32 = 1 << 8;

/// Destination control field, bits 21-22.
const fn dest(field: u32) -> u32 {
    field << 21
}
/// Source control field, bits 23-24.
const fn source(field: u32) -> u32 {
    field << 23
}
/// The ARM9's start timing, bits 27-29.
const fn timing9(value: u32) -> u32 {
    value << 27
}
/// The ARM7's start timing, bits 28-29.
const fn timing7(value: u32) -> u32 {
    value << 28
}

const fn sad(n: u32) -> u32 {
    12 * n
}
const fn dad(n: u32) -> u32 {
    12 * n + 4
}
const fn cnt(n: u32) -> u32 {
    12 * n + 8
}
const fn fill(n: u32) -> u32 {
    0x30 + 4 * n
}

const MAIN: u32 = 0x0200_0000;
const OTHER: u32 = 0x0300_0000;

fn write32<const A: bool>(dma: &mut Dma<A>, offset: u32, value: u32) -> Effect {
    dma.write(offset, Width::Word, value, 0)
}

fn write16<const A: bool>(dma: &mut Dma<A>, offset: u32, value: u16) -> Effect {
    dma.write(offset, Width::Half, u32::from(value), 0)
}

fn write8<const A: bool>(dma: &mut Dma<A>, offset: u32, value: u8) -> Effect {
    dma.write(offset, Width::Byte, u32::from(value), 0)
}

/// Byte writes at `offset`, `offset + 1`, ... in that order.
fn write_bytes_in_turn<const A: bool>(dma: &mut Dma<A>, offset: u32, bytes: &[u8]) {
    bytes.iter().enumerate().for_each(|(i, &byte)| {
        write8(dma, offset + i as u32, byte);
    });
}

fn read32<const A: bool>(dma: &Dma<A>, offset: u32) -> u32 {
    dma.peek(offset, Width::Word, 0)
}

/// Writes channel `ch`'s source, destination and control, in that order; returns the effect of
/// the control write.
fn arm<const A: bool>(
    dma: &mut Dma<A>,
    ch: u32,
    source: u32,
    destination: u32,
    control: u32,
) -> Effect {
    write32(dma, sad(ch), source);
    write32(dma, dad(ch), destination);
    write32(dma, cnt(ch), control)
}

/// Arms an Immediate channel and takes its transfer.
fn started<const A: bool>(
    dma: &mut Dma<A>,
    ch: u32,
    source: u32,
    destination: u32,
    control: u32,
) -> Transfer {
    arm(dma, ch, source, destination, control);
    dma.begin(ch as usize)
        .expect("an Immediate enable queues a start")
}

/// Arms a channel that waits for `trigger`, raises it and takes the transfer.
fn started_by<const A: bool>(
    dma: &mut Dma<A>,
    ch: u32,
    source: u32,
    destination: u32,
    control: u32,
    trigger: Trigger,
) -> Transfer {
    arm(dma, ch, source, destination, control);
    dma.trigger(trigger);
    dma.begin(ch as usize).expect("the trigger queued a start")
}

fn slot_effect(ch: u32) -> Effect {
    Effect {
        requests: Requests::default(),
        reschedule: 1 << ch,
    }
}

// ------------------------------------------------------------------------------------ mock bus

/// One bus access a transfer made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    R16(u32),
    W16(u32, u16),
    R32(u32),
    W32(u32, u32),
}

/// A sparse byte memory that logs the 16-bit and 32-bit accesses a transfer makes.
#[derive(Default)]
struct MockBus {
    bytes: BTreeMap<u32, u8>,
    log: Vec<Op>,
}

impl MockBus {
    fn with_halfwords(at: u32, values: &[u16]) -> Self {
        let mut bus = Self::default();
        values.iter().enumerate().for_each(|(i, &v)| {
            bus.put(at + 2 * i as u32, 2, u32::from(v));
        });
        bus
    }

    fn with_words(at: u32, values: &[u32]) -> Self {
        let mut bus = Self::default();
        values.iter().enumerate().for_each(|(i, &v)| {
            bus.put(at + 4 * i as u32, 4, v);
        });
        bus
    }

    fn get(&self, addr: u32, size: u32) -> u32 {
        (0..size).fold(0, |value, i| {
            let byte = self.bytes.get(&addr.wrapping_add(i)).copied().unwrap_or(0);
            value | u32::from(byte) << (8 * i)
        })
    }

    fn put(&mut self, addr: u32, size: u32, value: u32) {
        (0..size).for_each(|i| {
            self.bytes
                .insert(addr.wrapping_add(i), (value >> (8 * i)) as u8);
        });
    }
}

impl Bus for MockBus {
    fn fetch32(&mut self, addr: u32) -> u32 {
        self.get(addr, 4)
    }
    fn fetch16(&mut self, addr: u32) -> u16 {
        self.get(addr, 2) as u16
    }
    fn read8(&mut self, addr: u32) -> u8 {
        self.get(addr, 1) as u8
    }
    fn read16(&mut self, addr: u32) -> u16 {
        self.log.push(Op::R16(addr));
        self.get(addr, 2) as u16
    }
    fn read32(&mut self, addr: u32) -> u32 {
        self.log.push(Op::R32(addr));
        self.get(addr, 4)
    }
    fn write8(&mut self, addr: u32, value: u8) {
        self.put(addr, 1, u32::from(value));
    }
    fn write16(&mut self, addr: u32, value: u16) {
        self.log.push(Op::W16(addr, value));
        self.put(addr, 2, u32::from(value));
    }
    fn write32(&mut self, addr: u32, value: u32) {
        self.log.push(Op::W32(addr, value));
        self.put(addr, 4, value);
    }
}

/// Arms channel 0 of a fresh device (Immediate, `control` carries enable, flags and count), takes
/// the transfer and runs it on `bus`; returns where the addresses ended.
fn run_immediate<const A: bool>(
    control: u32,
    source: u32,
    destination: u32,
    bus: &mut MockBus,
) -> End {
    let mut dma = Dma::<A>::new();
    started(&mut dma, 0, source, destination, control).run(bus)
}

// -------------------------------------------------------------------- registers: reset, masks

/// The word offsets of the register space (channels and fill block) that do not read 0.
fn nonzero_registers<const A: bool>(dma: &Dma<A>) -> Vec<u32> {
    (0..0x40)
        .step_by(4)
        .filter(|&offset| read32(dma, offset) != 0)
        .collect()
}

#[test]
fn given_a_new_arm9_device_when_every_register_is_read_then_all_read_zero() {
    assert_eq!(nonzero_registers(&Arm9Dma::new()), Vec::<u32>::new());
}

#[test]
fn given_a_new_arm7_device_when_every_register_is_read_then_all_read_zero() {
    assert_eq!(nonzero_registers(&Arm7Dma::new()), Vec::<u32>::new());
}

type Arm9Dma = Dma<true>;
type Arm7Dma = Dma<false>;

/// Writes `FFFF_FFFFh` to each offset of a fresh device and returns the registers that read back
/// something other than the wanted value, as `(offset, got, want)`.
fn readback_failures<const A: bool>(expected: &[(u32, u32)]) -> Vec<(u32, u32, u32)> {
    expected
        .iter()
        .filter_map(|&(offset, want)| {
            let mut dma = Dma::<A>::new();
            write32(&mut dma, offset, 0xFFFF_FFFF);
            let got = read32(&dma, offset);
            (got != want).then_some((offset, got, want))
        })
        .collect()
}

/// ARM9: every address is 28 bits, DMAnCNT keeps all 32 bits (21-bit count, control 21-31), and
/// the fill registers hold 32 bits (`dma.md`, "Address masks", "Word count").
fn arm9_readbacks() -> Vec<(u32, u32)> {
    (0..4)
        .flat_map(|n| {
            [
                (sad(n), 0x0FFF_FFFF),
                (dad(n), 0x0FFF_FFFF),
                (cnt(n), 0xFFFF_FFFF),
                (fill(n), 0xFFFF_FFFF),
            ]
        })
        .collect()
}

#[test]
fn given_an_arm9_when_all_ones_are_written_to_each_register_then_each_reads_back_its_width() {
    assert_eq!(readback_failures::<true>(&arm9_readbacks()), vec![]);
}

/// ARM7 (`dma.md`, "Address masks", "Word count"): channel 0 27-bit both; channels 1-2 28-bit
/// source and 27-bit destination; channel 3 28-bit both; count 14 bits on channels 0-2 and 16 on
/// channel 3; control bit 27 does not exist, so bits 21-26 and 28-31 remain.
const ARM7_READBACKS: [(u32, u32); 12] = [
    (sad(0), 0x07FF_FFFF),
    (dad(0), 0x07FF_FFFF),
    (cnt(0), 0xF7E0_3FFF),
    (sad(1), 0x0FFF_FFFF),
    (dad(1), 0x07FF_FFFF),
    (cnt(1), 0xF7E0_3FFF),
    (sad(2), 0x0FFF_FFFF),
    (dad(2), 0x07FF_FFFF),
    (cnt(2), 0xF7E0_3FFF),
    (sad(3), 0x0FFF_FFFF),
    (dad(3), 0x0FFF_FFFF),
    (cnt(3), 0xF7E0_FFFF),
];

#[test]
fn given_an_arm7_when_all_ones_are_written_to_each_register_then_each_reads_back_its_width() {
    assert_eq!(readback_failures::<false>(&ARM7_READBACKS), vec![]);
}

/// The control bits 21-31 that read back as written when written alone to channel 0.
fn control_bits_reading_back<const A: bool>() -> Vec<u32> {
    (21..32)
        .filter(|&bit| {
            let mut dma = Dma::<A>::new();
            write32(&mut dma, cnt(0), 1 << bit);
            read32(&dma, cnt(0)) == 1 << bit
        })
        .collect()
}

#[test]
fn given_an_arm9_when_each_control_bit_is_written_alone_then_every_bit_from_21_to_31_reads_back() {
    assert_eq!(
        control_bits_reading_back::<true>(),
        (21..32).collect::<Vec<u32>>()
    );
}

#[test]
fn given_an_arm7_when_each_control_bit_is_written_alone_then_every_bit_but_27_reads_back_unverified()
 {
    assert_eq!(
        control_bits_reading_back::<false>(),
        vec![21, 22, 23, 24, 25, 26, 28, 29, 30, 31]
    );
}

#[test]
fn given_an_arm7_when_control_bit_27_is_set_with_enable_then_the_start_is_immediate_unverified() {
    // Bit 27 is not part of the ARM7's timing field (`dma.md`, "Start timing"), so the timing
    // reads 0: Immediate.
    let mut dma = Arm7Dma::new();
    let effect = arm(&mut dma, 1, 0, 0, ENABLE | (1 << 27) | 1);
    assert_eq!((effect, dma.pending(1)), (slot_effect(1), true));
}

#[test]
fn given_an_arm9_fill_register_when_written_then_it_reads_back_and_no_other_register_changes() {
    let mut dma = Arm9Dma::new();
    write32(&mut dma, fill(2), 0x1234_5678);
    let others: Vec<u32> = nonzero_registers(&dma)
        .into_iter()
        .filter(|&offset| offset != fill(2))
        .collect();
    assert_eq!(
        (dma.fill(2), read32(&dma, fill(2)), others),
        (0x1234_5678, 0x1234_5678, vec![])
    );
}

#[test]
fn given_an_arm7_when_the_fill_block_is_written_then_nothing_is_stored_and_it_reads_zero() {
    let mut dma = Arm7Dma::new();
    let effects: Vec<Effect> = (0..4)
        .map(|n| write32(&mut dma, fill(n), 0xFFFF_FFFF))
        .collect();
    assert_eq!(
        (effects, dma.clone(), nonzero_registers(&dma)),
        (vec![Effect::default(); 4], Arm7Dma::new(), vec![])
    );
}

#[test]
fn given_an_offset_past_the_fill_block_when_written_and_read_then_it_is_ignored_and_reads_zero() {
    let mut dma = Arm9Dma::new();
    let effect = write32(&mut dma, 0x40, 0xFFFF_FFFF);
    assert_eq!(
        (effect, dma.clone(), read32(&dma, 0x40)),
        (Effect::default(), Arm9Dma::new(), 0)
    );
}

#[test]
fn given_a_byte_write_to_the_top_byte_of_a_source_when_read_then_the_address_is_masked_per_cpu() {
    let mut arm9 = Arm9Dma::new();
    let mut arm7 = Arm7Dma::new();
    write8(&mut arm9, sad(0) + 3, 0xFF);
    write8(&mut arm7, sad(0) + 3, 0xFF);
    assert_eq!((arm9.source(0), arm7.source(0)), (0x0F00_0000, 0x0700_0000));
}

#[test]
fn given_the_four_bytes_of_a_source_written_in_turn_when_read_then_they_join_in_little_endian() {
    let mut dma = Arm9Dma::new();
    write_bytes_in_turn(&mut dma, sad(1), &[0x67, 0x45, 0x23, 0x01]);
    assert_eq!(dma.source(1), 0x0123_4567);
}

#[test]
fn given_a_word_read_that_spans_two_channels_when_split_then_each_byte_comes_from_its_own_register()
{
    // CNT0 = 2ABBCCDDh at offsets 8-11, SAD1 = 11223344h at 12-15: a word at 10 reads
    // BBh 2Ah (CNT0's top half) then 44h 33h (SAD1's low half), little-endian.
    let mut dma = Arm9Dma::new();
    write32(&mut dma, cnt(0), 0x2ABB_CCDD);
    write32(&mut dma, sad(1), 0x1122_3344);
    assert_eq!(dma.peek(10, Width::Word, 0), 0x3344_2ABB);
}

#[test]
fn given_a_word_write_that_spans_two_channels_when_split_then_each_byte_goes_to_its_own_register() {
    // Bytes 44h 33h land in CNT0's top half (offsets 10-11), 22h 11h in SAD1's low half.
    let mut dma = Arm9Dma::new();
    write32(&mut dma, 10, 0x1122_3344);
    assert_eq!((dma.control(0), dma.source(1)), (0x3344_0000, 0x0000_1122));
}

#[test]
fn given_a_register_read_when_done_through_the_device_read_then_it_has_no_effect_and_no_state_change()
 {
    let mut dma = Arm9Dma::new();
    arm(&mut dma, 0, MAIN, OTHER, ENABLE | 4);
    let before = dma.clone();
    let (value, effect) = dma.read(cnt(0), Width::Word, 0);
    assert_eq!(
        (value, effect, dma),
        (ENABLE | 4, Effect::default(), before)
    );
}

// ---------------------------------------------------------------------- the enable edge

#[test]
fn given_an_immediate_channel_when_enable_is_written_then_a_start_is_queued_on_its_own_slot() {
    let mut dma = Arm9Dma::new();
    let effect = arm(&mut dma, 3, MAIN, OTHER, ENABLE | 4);
    assert_eq!(
        (effect, dma.pending(3), dma.pending(0)),
        (slot_effect(3), true, false)
    );
}

/// The slots each of channels 0-3 reschedules when armed Immediate, as `(channel, got)` for the
/// channels that do not reschedule exactly their own bit.
fn wrong_slots<const A: bool>() -> Vec<(u32, u8)> {
    (0..4)
        .filter_map(|ch| {
            let mut dma = Dma::<A>::new();
            let got = arm(&mut dma, ch, 0, 0, ENABLE | 1).reschedule;
            (got != 1 << ch).then_some((ch, got))
        })
        .collect()
}

#[test]
fn given_an_arm9_when_each_channel_is_armed_immediate_then_it_reschedules_exactly_its_own_slot() {
    assert_eq!(wrong_slots::<true>(), vec![]);
}

#[test]
fn given_an_arm7_when_each_channel_is_armed_immediate_then_it_reschedules_exactly_its_own_slot() {
    assert_eq!(wrong_slots::<false>(), vec![]);
}

#[test]
fn given_a_channel_with_a_start_timing_when_enable_is_written_then_it_waits_with_nothing_queued() {
    let mut dma = Arm9Dma::new();
    let effect = arm(&mut dma, 0, MAIN, OTHER, ENABLE | timing9(1) | 4);
    assert_eq!(
        (effect, dma.pending(0), read32(&dma, cnt(0)) >> 31),
        (Effect::default(), false, 1)
    );
}

#[test]
fn given_the_four_control_bytes_written_in_turn_when_the_top_byte_lands_then_only_it_queues_the_start()
 {
    let mut dma = Arm9Dma::new();
    let early = [
        write8(&mut dma, cnt(2), 0x03),
        write8(&mut dma, cnt(2) + 1, 0x00),
        write8(&mut dma, cnt(2) + 2, 0x00),
    ];
    let pending_before = dma.pending(2);
    let top = write8(&mut dma, cnt(2) + 3, 0x80);
    assert_eq!(
        (early, pending_before, top, dma.pending(2)),
        ([Effect::default(); 3], false, slot_effect(2), true)
    );
}

#[test]
fn given_halfword_writes_to_a_control_register_when_the_high_half_sets_enable_then_the_start_is_queued()
 {
    let mut dma = Arm9Dma::new();
    let low = write16(&mut dma, cnt(1), 0x0004);
    let high = write16(&mut dma, cnt(1) + 2, 0x8000);
    assert_eq!(
        (low, high, dma.pending(1)),
        (Effect::default(), slot_effect(1), true)
    );
}

#[test]
fn given_a_queued_immediate_start_when_enable_is_written_again_as_1_then_nothing_new_is_queued() {
    let mut dma = Arm9Dma::new();
    arm(&mut dma, 0, MAIN, OTHER, ENABLE | 4);
    let again = write32(&mut dma, cnt(0), ENABLE | 9);
    assert_eq!((again, dma.pending(0)), (Effect::default(), true));
}

#[test]
fn given_a_channel_whose_registers_change_while_enabled_when_it_starts_then_it_uses_the_latched_copies()
 {
    // Enable went 0 -> 1 once; a later write with enable still 1 changes only the registers
    // (`dma.md`, "Enabling").
    let mut dma = Arm9Dma::new();
    arm(&mut dma, 0, MAIN, OTHER, ENABLE | timing9(1) | 5);
    arm(
        &mut dma,
        0,
        MAIN + 0x10_0000,
        OTHER + 0x10_0000,
        ENABLE | timing9(1) | 9,
    );
    dma.trigger(Trigger::VBlank);
    let transfer = dma.begin(0).expect("triggered");
    assert_eq!(
        (transfer.source, transfer.destination, transfer.count),
        (MAIN, OTHER, 5)
    );
}

#[test]
fn given_a_channel_whose_registers_change_while_enabled_when_read_back_then_they_show_what_was_written()
 {
    let mut dma = Arm9Dma::new();
    arm(&mut dma, 0, MAIN, OTHER, ENABLE | timing9(1) | 5);
    arm(
        &mut dma,
        0,
        MAIN + 0x10_0000,
        OTHER + 0x10_0000,
        ENABLE | timing9(1) | 9,
    );
    assert_eq!(
        (
            dma.source(0),
            dma.destination(0),
            dma.control(0) & 0x1F_FFFF
        ),
        (MAIN + 0x10_0000, OTHER + 0x10_0000, 9)
    );
}

#[test]
fn given_an_immediate_start_waiting_when_the_registers_change_then_the_start_uses_the_latched_copies()
 {
    let mut dma = Arm9Dma::new();
    arm(&mut dma, 0, MAIN, OTHER, ENABLE | 5);
    write32(&mut dma, sad(0), MAIN + 0x20);
    write32(&mut dma, dad(0), OTHER + 0x20);
    let transfer = dma.begin(0).expect("queued");
    assert_eq!((transfer.source, transfer.destination), (MAIN, OTHER));
}

#[test]
fn given_a_channel_disabled_and_enabled_again_when_it_starts_then_the_new_registers_are_latched() {
    let mut dma = Arm9Dma::new();
    arm(&mut dma, 0, MAIN, OTHER, ENABLE | timing9(1) | 5);
    write32(&mut dma, cnt(0), timing9(1) | 5);
    let transfer = started_by(
        &mut dma,
        0,
        MAIN + 0x40,
        OTHER + 0x40,
        ENABLE | timing9(1) | 7,
        Trigger::VBlank,
    );
    assert_eq!(
        (transfer.source, transfer.destination, transfer.count),
        (MAIN + 0x40, OTHER + 0x40, 7)
    );
}

#[test]
fn given_a_queued_immediate_start_when_enable_is_cleared_then_the_start_is_cancelled() {
    let mut dma = Arm9Dma::new();
    arm(&mut dma, 1, MAIN, OTHER, ENABLE | 4);
    let effect = write32(&mut dma, cnt(1), 4);
    assert_eq!(
        (effect, dma.pending(1), dma.slot_due(1), dma.begin(1)),
        (slot_effect(1), false, None, None)
    );
}

#[test]
fn given_a_triggered_start_when_enable_is_cleared_through_the_top_byte_then_the_start_is_cancelled()
{
    let mut dma = Arm9Dma::new();
    arm(&mut dma, 0, MAIN, OTHER, ENABLE | timing9(2) | 4);
    dma.trigger(Trigger::HBlank);
    let effect = write8(&mut dma, cnt(0) + 3, 0x10);
    assert_eq!(
        (effect, dma.pending(0), dma.begin(0)),
        (slot_effect(0), false, None)
    );
}

#[test]
fn given_a_channel_waiting_for_its_trigger_when_enable_is_cleared_then_the_trigger_finds_nothing() {
    let mut dma = Arm9Dma::new();
    arm(&mut dma, 0, MAIN, OTHER, ENABLE | timing9(1) | 4);
    let effect = write32(&mut dma, cnt(0), timing9(1) | 4);
    assert_eq!(
        (effect, dma.trigger(Trigger::VBlank)),
        (Effect::default(), 0)
    );
}

// ------------------------------------------------------------------------------ triggers

/// The slot mask `trigger` marks on channel 0 armed with `control`.
fn trigger_mask<const A: bool>(control: u32, trigger: Trigger) -> u8 {
    let mut dma = Dma::<A>::new();
    arm(&mut dma, 0, MAIN, OTHER, control);
    dma.trigger(trigger)
}

/// `(timing, VBlank mask, HBlank mask)` for each timing value whose masks differ from `want`.
fn trigger_failures<const A: bool>(
    shift: fn(u32) -> u32,
    want: &[(u32, u8, u8)],
) -> Vec<(u32, u8, u8)> {
    want.iter()
        .filter_map(|&(timing, vblank, hblank)| {
            let control = ENABLE | shift(timing) | 4;
            let got = (
                timing,
                trigger_mask::<A>(control, Trigger::VBlank),
                trigger_mask::<A>(control, Trigger::HBlank),
            );
            (got != (timing, vblank, hblank)).then_some(got)
        })
        .collect()
}

#[test]
fn given_arm9_start_timings_when_vblank_and_hblank_are_raised_then_only_1_and_2_start_a_channel() {
    // Timing 1 is VBlank, 2 is HBlank; 3-7 (display start, main-memory display, cartridge
    // slots, geometry FIFO) are not implemented: the channel is enabled and never starts
    // (`dma.md`, "Start timing").
    let want = [
        (1, 1, 0),
        (2, 0, 1),
        (3, 0, 0),
        (4, 0, 0),
        (5, 0, 0),
        (6, 0, 0),
        (7, 0, 0),
    ];
    assert_eq!(trigger_failures::<true>(timing9, &want), vec![]);
}

#[test]
fn given_arm7_start_timings_when_vblank_and_hblank_are_raised_then_only_timing_1_starts_on_vblank()
{
    // The ARM7 has no HBlank start; 2 is the cartridge slot and 3 the wireless or GBA slot.
    let want = [(1, 1, 0), (2, 0, 0), (3, 0, 0)];
    assert_eq!(trigger_failures::<false>(timing7, &want), vec![]);
}

#[test]
fn given_an_arm9_channel_with_an_unimplemented_timing_when_read_then_it_is_enabled_and_busy() {
    let mut dma = Arm9Dma::new();
    arm(&mut dma, 0, MAIN, OTHER, ENABLE | timing9(7) | 4);
    assert_eq!(
        (read32(&dma, cnt(0)) >> 31, dma.pending(0), dma.slot_due(0)),
        (1, false, None)
    );
}

#[test]
fn given_a_disabled_channel_with_a_vblank_timing_when_vblank_is_raised_then_nothing_starts() {
    assert_eq!(trigger_mask::<true>(timing9(1) | 4, Trigger::VBlank), 0);
}

#[test]
fn given_an_immediate_channel_when_vblank_is_raised_then_it_is_not_marked_again() {
    assert_eq!(trigger_mask::<true>(ENABLE | 4, Trigger::VBlank), 0);
}

#[test]
fn given_channels_with_mixed_timings_when_vblank_is_raised_then_the_mask_names_the_waiting_ones() {
    let mut dma = Arm9Dma::new();
    arm(&mut dma, 0, MAIN, OTHER, ENABLE | timing9(1) | 4);
    arm(&mut dma, 1, MAIN, OTHER, ENABLE | timing9(2) | 4);
    arm(&mut dma, 2, MAIN, OTHER, ENABLE | timing9(1) | 4);
    arm(&mut dma, 3, MAIN, OTHER, timing9(1) | 4);
    let vblank = dma.trigger(Trigger::VBlank);
    let hblank = dma.trigger(Trigger::HBlank);
    assert_eq!(
        (
            vblank,
            hblank,
            dma.pending(0),
            dma.pending(1),
            dma.pending(2),
            dma.pending(3)
        ),
        (0b0101, 0b0010, true, true, true, false)
    );
}

#[test]
fn given_a_channel_already_marked_when_the_trigger_is_raised_again_then_it_is_not_marked_twice() {
    let mut dma = Arm9Dma::new();
    arm(&mut dma, 0, MAIN, OTHER, ENABLE | timing9(1) | 4);
    let first = dma.trigger(Trigger::VBlank);
    let second = dma.trigger(Trigger::VBlank);
    assert_eq!((first, second), (1, 0));
}

#[test]
fn given_a_triggered_channel_when_its_start_is_taken_then_a_second_trigger_marks_it_again() {
    let mut dma = Arm9Dma::new();
    arm(&mut dma, 0, MAIN, OTHER, ENABLE | timing9(1) | REPEAT | 4);
    dma.trigger(Trigger::VBlank);
    dma.begin(0);
    assert_eq!(dma.trigger(Trigger::VBlank), 1);
}

// ------------------------------------------------------------------------------- begin

#[test]
fn given_a_channel_with_no_start_waiting_when_begin_is_asked_then_it_returns_nothing() {
    let mut dma = Arm9Dma::new();
    arm(&mut dma, 0, MAIN, OTHER, ENABLE | timing9(1) | 4);
    assert_eq!(dma.begin(0), None);
}

#[test]
fn given_a_channel_index_past_the_last_when_begin_is_asked_then_it_returns_nothing() {
    let mut dma = Arm9Dma::new();
    arm(&mut dma, 3, MAIN, OTHER, ENABLE | 4);
    assert_eq!((dma.begin(4), dma.begin(usize::MAX)), (None, None));
}

#[test]
fn given_a_taken_start_when_begin_is_asked_again_then_it_returns_nothing_and_enable_stays_set() {
    let mut dma = Arm9Dma::new();
    arm(&mut dma, 0, MAIN, OTHER, ENABLE | 4);
    dma.begin(0);
    assert_eq!(
        (dma.begin(0), dma.pending(0), read32(&dma, cnt(0)) >> 31),
        (None, false, 1)
    );
}

#[test]
fn given_a_16_bit_channel_with_unaligned_addresses_when_begun_then_they_are_aligned_down_to_2() {
    let mut dma = Arm9Dma::new();
    let t = started(&mut dma, 0, MAIN + 3, OTHER + 5, ENABLE | 2);
    assert_eq!(
        (t.source, t.destination, t.word),
        (MAIN + 2, OTHER + 4, false)
    );
}

#[test]
fn given_a_32_bit_channel_with_unaligned_addresses_when_begun_then_they_are_aligned_down_to_4() {
    let mut dma = Arm9Dma::new();
    let t = started(&mut dma, 0, MAIN + 7, OTHER + 2, ENABLE | WORD | 2);
    assert_eq!((t.source, t.destination, t.word), (MAIN + 4, OTHER, true));
}

#[test]
fn given_an_arm7_channel_0_with_all_ones_addresses_when_begun_then_they_are_27_bit_and_word_aligned()
 {
    let mut dma = Arm7Dma::new();
    let t = started(&mut dma, 0, 0xFFFF_FFFF, 0xFFFF_FFFF, ENABLE | WORD | 1);
    assert_eq!((t.source, t.destination), (0x07FF_FFFC, 0x07FF_FFFC));
}

#[test]
fn given_a_transfer_when_begun_then_it_names_its_cpu_and_channel() {
    let mut arm9 = Arm9Dma::new();
    let mut arm7 = Arm7Dma::new();
    let nine = started(&mut arm9, 2, 0, 0, ENABLE | 1);
    let seven = started(&mut arm7, 3, 0, 0, ENABLE | 1);
    assert_eq!(
        (nine.arm9, nine.channel, seven.arm9, seven.channel),
        (true, 2, false, 3)
    );
}

#[test]
fn given_a_word_count_of_zero_when_begun_then_the_count_is_the_fields_top_plus_one() {
    // `dma.md`, "Word count": 200000h on the ARM9, 4000h on ARM7 channels 0-2, 10000h on
    // ARM7 channel 3.
    let mut arm9 = Arm9Dma::new();
    let mut arm7 = Arm7Dma::new();
    let counts = [
        started(&mut arm9, 0, 0, 0, ENABLE).count,
        started(&mut arm7, 0, 0, 0, ENABLE).count,
        started(&mut arm7, 2, 0, 0, ENABLE).count,
        started(&mut arm7, 3, 0, 0, ENABLE).count,
    ];
    assert_eq!(counts, [0x20_0000, 0x4000, 0x4000, 0x1_0000]);
}

#[test]
fn given_an_arm7_count_above_14_bits_when_begun_then_only_the_low_14_bits_count() {
    let mut dma = Arm7Dma::new();
    let t = started(&mut dma, 0, 0, 0, ENABLE | 0x4001);
    assert_eq!(t.count, 1);
}

#[test]
fn given_an_arm9_21_bit_count_when_begun_then_the_whole_count_is_kept() {
    let mut dma = Arm9Dma::new();
    let t = started(&mut dma, 0, 0, 0, ENABLE | 0x1F_FFFF);
    assert_eq!(t.count, 0x1F_FFFF);
}

// ---------------------------------------------------------------------------------- run

#[test]
fn given_a_16_bit_increment_transfer_when_run_over_three_units_then_each_unit_is_read_then_written()
{
    let mut bus = MockBus::with_halfwords(MAIN, &[0x1111, 0x2222, 0x3333]);
    run_immediate::<true>(ENABLE | 3, MAIN, OTHER, &mut bus);
    assert_eq!(
        bus.log,
        vec![
            Op::R16(MAIN),
            Op::W16(OTHER, 0x1111),
            Op::R16(MAIN + 2),
            Op::W16(OTHER + 2, 0x2222),
            Op::R16(MAIN + 4),
            Op::W16(OTHER + 4, 0x3333),
        ]
    );
}

#[test]
fn given_a_16_bit_increment_transfer_when_run_over_three_units_then_the_addresses_end_6_bytes_on() {
    let mut bus = MockBus::default();
    let end = run_immediate::<true>(ENABLE | 3, MAIN, OTHER, &mut bus);
    assert_eq!(
        end,
        End {
            source: MAIN + 6,
            destination: OTHER + 6
        }
    );
}

#[test]
fn given_a_32_bit_increment_transfer_when_run_over_two_units_then_words_are_read_then_written() {
    let mut bus = MockBus::with_words(MAIN, &[0xAAAA_0001, 0xBBBB_0002]);
    let end = run_immediate::<true>(ENABLE | WORD | 2, MAIN, OTHER, &mut bus);
    assert_eq!(
        (bus.log, end),
        (
            vec![
                Op::R32(MAIN),
                Op::W32(OTHER, 0xAAAA_0001),
                Op::R32(MAIN + 4),
                Op::W32(OTHER + 4, 0xBBBB_0002),
            ],
            End {
                source: MAIN + 8,
                destination: OTHER + 8
            }
        )
    );
}

#[test]
fn given_a_transfer_when_run_then_the_destination_memory_holds_the_source_units() {
    let mut bus = MockBus::with_words(MAIN, &[0x1111_2222, 0x3333_4444, 0x5555_6666]);
    run_immediate::<true>(ENABLE | WORD | 3, MAIN, OTHER, &mut bus);
    assert_eq!(
        [
            bus.get(OTHER, 4),
            bus.get(OTHER + 4, 4),
            bus.get(OTHER + 8, 4)
        ],
        [0x1111_2222, 0x3333_4444, 0x5555_6666]
    );
}

#[test]
fn given_decrementing_source_and_destination_when_run_then_both_walk_down_by_the_unit() {
    // Halfwords at MAIN+2, +4, +6 hold AAAAh, BBBBh, CCCCh; the walk starts at +6.
    let mut bus = MockBus::with_halfwords(MAIN, &[0, 0xAAAA, 0xBBBB, 0xCCCC]);
    let end = run_immediate::<true>(
        ENABLE | source(1) | dest(1) | 3,
        MAIN + 6,
        OTHER + 6,
        &mut bus,
    );
    assert_eq!(
        (bus.log, end),
        (
            vec![
                Op::R16(MAIN + 6),
                Op::W16(OTHER + 6, 0xCCCC),
                Op::R16(MAIN + 4),
                Op::W16(OTHER + 4, 0xBBBB),
                Op::R16(MAIN + 2),
                Op::W16(OTHER + 2, 0xAAAA),
            ],
            End {
                source: MAIN,
                destination: OTHER
            }
        )
    );
}

#[test]
fn given_a_fixed_source_when_run_then_every_unit_reads_the_same_address() {
    let mut bus = MockBus::with_words(MAIN, &[0xCAFE_F00D]);
    let end = run_immediate::<true>(ENABLE | WORD | source(2) | 3, MAIN, OTHER, &mut bus);
    assert_eq!(
        (bus.log, end),
        (
            vec![
                Op::R32(MAIN),
                Op::W32(OTHER, 0xCAFE_F00D),
                Op::R32(MAIN),
                Op::W32(OTHER + 4, 0xCAFE_F00D),
                Op::R32(MAIN),
                Op::W32(OTHER + 8, 0xCAFE_F00D),
            ],
            End {
                source: MAIN,
                destination: OTHER + 12
            }
        )
    );
}

#[test]
fn given_a_fixed_destination_when_run_then_every_unit_is_written_to_the_same_address() {
    let mut bus = MockBus::with_words(MAIN, &[1, 2, 3]);
    let end = run_immediate::<true>(ENABLE | WORD | dest(2) | 3, MAIN, OTHER, &mut bus);
    assert_eq!(
        (bus.log, end),
        (
            vec![
                Op::R32(MAIN),
                Op::W32(OTHER, 1),
                Op::R32(MAIN + 4),
                Op::W32(OTHER, 2),
                Op::R32(MAIN + 8),
                Op::W32(OTHER, 3),
            ],
            End {
                source: MAIN + 12,
                destination: OTHER
            }
        )
    );
}

#[test]
fn given_destination_control_3_when_run_then_the_destination_increments_during_the_transfer() {
    let mut bus = MockBus::with_halfwords(MAIN, &[7, 8, 9]);
    run_immediate::<true>(ENABLE | dest(3) | 3, MAIN, OTHER, &mut bus);
    assert_eq!(
        bus.log,
        vec![
            Op::R16(MAIN),
            Op::W16(OTHER, 7),
            Op::R16(MAIN + 2),
            Op::W16(OTHER + 2, 8),
            Op::R16(MAIN + 4),
            Op::W16(OTHER + 4, 9),
        ]
    );
}

#[test]
fn given_source_control_3_when_run_then_the_source_increments_like_control_0_unverified() {
    // TODO(hw-unverified): `dma.md` marks source control 3 prohibited; the model treats it as
    // increment.
    let mut bus = MockBus::with_halfwords(MAIN, &[7, 8]);
    run_immediate::<true>(ENABLE | source(3) | 2, MAIN, OTHER, &mut bus);
    assert_eq!(
        bus.log,
        vec![
            Op::R16(MAIN),
            Op::W16(OTHER, 7),
            Op::R16(MAIN + 2),
            Op::W16(OTHER + 2, 8),
        ]
    );
}

#[test]
fn given_unaligned_16_bit_addresses_when_run_then_the_accesses_use_the_aligned_down_addresses() {
    let mut bus = MockBus::with_halfwords(MAIN, &[0x0A0A, 0x0B0B]);
    run_immediate::<true>(ENABLE | 2, MAIN + 1, OTHER + 3, &mut bus);
    assert_eq!(
        bus.log,
        vec![
            Op::R16(MAIN),
            Op::W16(OTHER + 2, 0x0A0A),
            Op::R16(MAIN + 2),
            Op::W16(OTHER + 4, 0x0B0B),
        ]
    );
}

#[test]
fn given_unaligned_32_bit_addresses_when_run_then_the_accesses_use_the_aligned_down_addresses() {
    let mut bus = MockBus::with_words(MAIN, &[0x0A0A_0A0A]);
    run_immediate::<true>(ENABLE | WORD | 1, MAIN + 2, OTHER + 7, &mut bus);
    assert_eq!(
        bus.log,
        vec![Op::R32(MAIN), Op::W32(OTHER + 4, 0x0A0A_0A0A)]
    );
}

#[test]
fn given_a_count_of_one_when_run_then_exactly_one_unit_moves() {
    let mut bus = MockBus::with_halfwords(MAIN, &[0x1234, 0x5678]);
    run_immediate::<true>(ENABLE | 1, MAIN, OTHER, &mut bus);
    assert_eq!(bus.log, vec![Op::R16(MAIN), Op::W16(OTHER, 0x1234)]);
}

#[test]
fn given_an_arm7_channel_0_with_count_zero_when_run_then_4000h_units_move() {
    // 4000h units of 16 bits: 8000h accesses, the last pair at the 3FFFh-th halfword.
    let mut bus = MockBus::default();
    let end = run_immediate::<false>(ENABLE, MAIN, OTHER, &mut bus);
    assert_eq!(
        (bus.log.len(), bus.log[0x7FFE], bus.log[0x7FFF], end),
        (
            0x8000,
            Op::R16(MAIN + 0x7FFE),
            Op::W16(OTHER + 0x7FFE, 0),
            End {
                source: MAIN + 0x8000,
                destination: OTHER + 0x8000
            }
        )
    );
}

// ------------------------------------------------------------------------------- finish

/// Where a finished transfer left its addresses in the tests below.
const END: End = End {
    source: MAIN + 0x10,
    destination: OTHER + 0x10,
};

/// The IF bits `finish` requests on CPU `A` for each channel armed with `control`, as
/// `(channel, arm9 requests, arm7 requests)` for the channels that differ from `want(channel)`.
fn finish_failures<const A: bool>(control: u32, want: fn(u32) -> u32) -> Vec<(u32, u32, u32)> {
    (0..4)
        .filter_map(|ch| {
            let mut dma = Dma::<A>::new();
            let t = started(&mut dma, ch, MAIN, OTHER, control);
            let effect = dma.finish(&t, END);
            let got = (ch, effect.requests.arm9, effect.requests.arm7);
            let (want9, want7) = match A {
                true => (want(ch), 0),
                false => (0, want(ch)),
            };
            (got != (ch, want9, want7) || effect.reschedule != 0).then_some(got)
        })
        .collect()
}

#[test]
fn given_arm9_channels_with_the_irq_bit_when_they_finish_then_if_bit_8_plus_n_is_requested_on_the_arm9()
 {
    assert_eq!(
        finish_failures::<true>(ENABLE | IRQ | 1, |ch| IF_DMA0 << ch),
        vec![]
    );
}

#[test]
fn given_arm7_channels_with_the_irq_bit_when_they_finish_then_if_bit_8_plus_n_is_requested_on_the_arm7()
 {
    assert_eq!(
        finish_failures::<false>(ENABLE | IRQ | 1, |ch| IF_DMA0 << ch),
        vec![]
    );
}

#[test]
fn given_channels_without_the_irq_bit_when_they_finish_then_nothing_is_requested() {
    assert_eq!(
        (
            finish_failures::<true>(ENABLE | 1, |_| 0),
            finish_failures::<false>(ENABLE | 1, |_| 0)
        ),
        (vec![], vec![])
    );
}

#[test]
fn given_a_one_shot_channel_when_it_finishes_then_enable_clears_and_the_other_bits_stay() {
    let mut dma = Arm9Dma::new();
    let control = ENABLE | IRQ | WORD | dest(1) | source(2) | timing9(1) | 6;
    let t = started_by(&mut dma, 0, MAIN, OTHER, control, Trigger::VBlank);
    dma.finish(&t, END);
    assert_eq!(
        (dma.control(0), dma.pending(0), dma.trigger(Trigger::VBlank)),
        (control & !ENABLE, false, 0)
    );
}

#[test]
fn given_a_repeating_vblank_channel_when_it_finishes_then_enable_stays_and_the_next_trigger_restarts_it()
 {
    let mut dma = Arm9Dma::new();
    let control = ENABLE | REPEAT | timing9(1) | 5;
    let t = started_by(&mut dma, 0, MAIN, OTHER, control, Trigger::VBlank);
    dma.finish(&t, END);
    assert_eq!(
        (
            read32(&dma, cnt(0)) >> 31,
            dma.pending(0),
            dma.trigger(Trigger::VBlank)
        ),
        (1, false, 1)
    );
}

#[test]
fn given_a_repeating_channel_when_it_finishes_then_the_count_reloads_and_the_source_continues() {
    // The register count changes to 9 while the transfer of 5 is latched; the reload takes 9.
    // The source continues where the transfer ended; destination control 0 does the same.
    let mut dma = Arm9Dma::new();
    let control = ENABLE | REPEAT | timing9(1) | 5;
    let t = started_by(&mut dma, 0, MAIN, OTHER, control, Trigger::VBlank);
    write32(&mut dma, cnt(0), ENABLE | REPEAT | timing9(1) | 9);
    dma.finish(&t, END);
    dma.trigger(Trigger::VBlank);
    let next = dma.begin(0).expect("restarted by the trigger");
    assert_eq!(
        (next.count, next.source, next.destination),
        (9, END.source, END.destination)
    );
}

#[test]
fn given_a_repeating_channel_with_destination_control_3_when_it_finishes_then_the_destination_reloads()
 {
    let mut dma = Arm9Dma::new();
    let control = ENABLE | REPEAT | dest(3) | timing9(1) | 5;
    let t = started_by(&mut dma, 0, MAIN, OTHER, control, Trigger::VBlank);
    dma.finish(&t, END);
    dma.trigger(Trigger::VBlank);
    let next = dma.begin(0).expect("restarted by the trigger");
    assert_eq!((next.source, next.destination), (END.source, OTHER));
}

#[test]
fn given_a_repeating_arm7_channel_with_count_zero_when_it_finishes_then_the_count_reloads_to_the_top()
 {
    let mut dma = Arm7Dma::new();
    let control = ENABLE | REPEAT | timing7(1);
    let t = started_by(&mut dma, 0, MAIN, OTHER, control, Trigger::VBlank);
    dma.finish(&t, END);
    dma.trigger(Trigger::VBlank);
    assert_eq!(dma.begin(0).expect("restarted").count, 0x4000);
}

#[test]
fn given_an_immediate_channel_with_repeat_when_it_finishes_then_enable_clears_unverified() {
    // TODO(hw-unverified): `dma.md` "Unverified": repeat is ignored for Immediate.
    let mut dma = Arm9Dma::new();
    let t = started(&mut dma, 0, MAIN, OTHER, ENABLE | REPEAT | 5);
    dma.finish(&t, END);
    assert_eq!(
        (read32(&dma, cnt(0)) >> 31, dma.trigger(Trigger::VBlank)),
        (0, 0)
    );
}

#[test]
fn given_a_repeating_channel_disabled_during_its_transfer_when_it_finishes_then_it_stays_disabled()
{
    let mut dma = Arm9Dma::new();
    let control = ENABLE | REPEAT | timing9(1) | 5;
    let t = started_by(&mut dma, 0, MAIN, OTHER, control, Trigger::VBlank);
    write32(&mut dma, cnt(0), REPEAT | timing9(1) | 5);
    dma.finish(&t, END);
    assert_eq!(
        (read32(&dma, cnt(0)) >> 31, dma.trigger(Trigger::VBlank)),
        (0, 0)
    );
}

#[test]
fn given_a_channel_disabled_during_its_transfer_when_it_finishes_then_the_irq_is_still_requested_unverified()
 {
    // `dma.md` "Unverified": the IRQ bit is read at the end, from whatever the register holds.
    let mut dma = Arm9Dma::new();
    let t = started(&mut dma, 0, MAIN, OTHER, ENABLE | IRQ | 5);
    write32(&mut dma, cnt(0), IRQ | 5);
    let effect = dma.finish(&t, END);
    assert_eq!(effect.requests, Requests::for_cpu::<true>(IF_DMA0));
}

#[test]
fn given_a_transfer_for_a_channel_past_the_last_when_it_finishes_then_nothing_happens() {
    let mut dma = Arm9Dma::new();
    let mut t = started(&mut dma, 0, MAIN, OTHER, ENABLE | IRQ | 1);
    t.channel = 4;
    let before = dma.clone();
    let effect = dma.finish(&t, END);
    assert_eq!((effect, dma), (Effect::default(), before));
}

// ------------------------------------------------------------------- accessors and slots

#[test]
fn given_a_channel_index_past_the_last_when_the_accessors_are_read_then_they_return_zero() {
    let mut dma = Arm9Dma::new();
    arm(&mut dma, 3, MAIN, OTHER, ENABLE | 4);
    write32(&mut dma, fill(3), 5);
    assert_eq!(
        (
            dma.source(4),
            dma.destination(4),
            dma.control(4),
            dma.fill(4),
            dma.pending(4),
            dma.source(usize::MAX),
            dma.pending(usize::MAX)
        ),
        (0, 0, 0, 0, false, 0, false)
    );
}

#[test]
fn given_the_accessors_when_a_channel_is_armed_then_they_return_its_registers() {
    let mut dma = Arm9Dma::new();
    arm(&mut dma, 2, MAIN + 8, OTHER + 8, ENABLE | WORD | 6);
    assert_eq!(
        (
            dma.source(2),
            dma.destination(2),
            dma.control(2),
            dma.pending(2)
        ),
        (MAIN + 8, OTHER + 8, ENABLE | WORD | 6, true)
    );
}

#[test]
fn given_each_slot_of_a_device_when_asked_for_its_event_then_it_is_that_cpus_dma_start() {
    let arm9: Vec<Option<EventKind>> = (0..4).map(|s| Arm9Dma::new().slot_kind(s)).collect();
    let arm7: Vec<Option<EventKind>> = (0..4).map(|s| Arm7Dma::new().slot_kind(s)).collect();
    let kind = |arm9, channel| Some(EventKind::DmaStart { arm9, channel });
    assert_eq!(
        (arm9, arm7),
        (
            vec![kind(true, 0), kind(true, 1), kind(true, 2), kind(true, 3)],
            vec![
                kind(false, 0),
                kind(false, 1),
                kind(false, 2),
                kind(false, 3)
            ]
        )
    );
}

#[test]
fn given_a_slot_past_the_fourth_when_asked_for_its_event_then_there_is_none() {
    let dma = Arm9Dma::new();
    assert_eq!(
        (
            dma.slot_kind(4),
            dma.slot_kind(255),
            dma.slot_due(4),
            dma.slot_due(255)
        ),
        (None, None, None, None)
    );
}

#[test]
fn given_a_waiting_start_when_its_slot_is_asked_for_a_due_time_then_it_is_due_at_once() {
    let mut dma = Arm9Dma::new();
    arm(&mut dma, 1, MAIN, OTHER, ENABLE | 4);
    arm(&mut dma, 2, MAIN, OTHER, ENABLE | timing9(1) | 4);
    assert_eq!(
        (dma.slot_due(0), dma.slot_due(1), dma.slot_due(2)),
        (None, Some(0), None)
    );
}

#[test]
fn given_a_waiting_start_when_its_slot_fires_then_the_transfer_comes_back_once() {
    let mut dma = Arm9Dma::new();
    arm(&mut dma, 1, MAIN, OTHER, ENABLE | 4);
    let first = dma.slot_fire(1, 0);
    let second = dma.slot_fire(1, 0);
    assert_eq!(
        (
            first.effect,
            first.transfer.map(|t| (t.arm9, t.channel, t.count)),
            second.transfer
        ),
        (Effect::default(), Some((true, 1, 4)), None)
    );
}

#[test]
fn given_a_slot_with_no_start_when_it_fires_then_nothing_comes_back() {
    let mut dma = Arm7Dma::new();
    assert_eq!(dma.slot_fire(0, 0), Fired::default());
}

#[test]
fn given_a_channel_when_its_event_kind_is_asked_then_it_names_the_cpu_and_channel() {
    assert_eq!(
        (Arm9Dma::event_kind(2), Arm7Dma::event_kind(3)),
        (
            EventKind::DmaStart {
                arm9: true,
                channel: 2
            },
            EventKind::DmaStart {
                arm9: false,
                channel: 3
            }
        )
    );
}

// -------------------------------------------------------------------------- well-formed

/// A named edit of a device, for the checks below.
type Case<const A: bool> = (&'static str, fn(&mut Dma<A>));

/// The corruptions in `cases` that `is_well_formed` still accepts.
fn accepted_corruptions<const A: bool>(cases: &[Case<A>]) -> Vec<&'static str> {
    cases
        .iter()
        .filter(|(_, corrupt)| {
            let mut dma = Dma::<A>::new();
            corrupt(&mut dma);
            dma.is_well_formed()
        })
        .map(|&(name, _)| name)
        .collect()
}

/// The valid edge values in `cases` that `is_well_formed` refuses.
fn refused_valid_values<const A: bool>(cases: &[Case<A>]) -> Vec<&'static str> {
    cases
        .iter()
        .filter(|(_, set)| {
            let mut dma = Dma::<A>::new();
            set(&mut dma);
            !dma.is_well_formed()
        })
        .map(|&(name, _)| name)
        .collect()
}

#[test]
fn given_power_on_and_armed_devices_when_checked_then_they_are_well_formed() {
    let mut arm9 = Arm9Dma::new();
    let mut arm7 = Arm7Dma::new();
    arm(&mut arm9, 0, 0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF);
    arm(&mut arm7, 3, 0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF);
    assert_eq!(
        (
            Arm9Dma::new().is_well_formed(),
            arm9.is_well_formed(),
            arm7.is_well_formed()
        ),
        (true, true, true)
    );
}

#[test]
fn given_an_arm9_state_with_a_bit_that_does_not_exist_when_checked_then_it_is_not_well_formed() {
    let cases: [Case<true>; 6] = [
        ("source bit 28", |d| d.channels[0].source = 1 << 28),
        ("destination bit 31", |d| {
            d.channels[1].destination = 1 << 31
        }),
        ("latched source bit 28", |d| {
            d.channels[2].source_now = 1 << 28
        }),
        ("latched destination bit 28", |d| {
            d.channels[3].destination_now = 1 << 28;
        }),
        ("latched count above 2^21", |d| {
            d.channels[0].count_now = 0x20_0001
        }),
        ("pending without enable", |d| d.channels[0].pending = true),
    ];
    assert_eq!(accepted_corruptions(&cases), Vec::<&str>::new());
}

#[test]
fn given_an_arm7_state_with_a_bit_that_does_not_exist_when_checked_then_it_is_not_well_formed() {
    let cases: [Case<false>; 10] = [
        ("control bit 27", |d| d.channels[0].control = 1 << 27),
        ("channel 0 count bit 14", |d| d.channels[0].control = 0x4000),
        ("channel 3 count bit 16", |d| {
            d.channels[3].control = 0x1_0000
        }),
        ("channel 0 source bit 27", |d| {
            d.channels[0].source = 1 << 27
        }),
        ("channel 1 destination bit 27", |d| {
            d.channels[1].destination = 1 << 27;
        }),
        ("channel 0 latched source bit 27", |d| {
            d.channels[0].source_now = 1 << 27;
        }),
        ("channel 2 latched destination bit 27", |d| {
            d.channels[2].destination_now = 1 << 27;
        }),
        ("channel 0 latched count above 4000h", |d| {
            d.channels[0].count_now = 0x4001;
        }),
        ("channel 3 latched count above 10000h", |d| {
            d.channels[3].count_now = 0x1_0001;
        }),
        ("a fill register", |d| d.fill[0] = 1),
    ];
    assert_eq!(accepted_corruptions(&cases), Vec::<&str>::new());
}

#[test]
fn given_the_largest_valid_values_when_checked_then_the_device_is_well_formed() {
    let arm9: [Case<true>; 3] = [
        ("count 200000h", |d| d.channels[0].count_now = 0x20_0000),
        ("source 28 bits", |d| d.channels[0].source = 0x0FFF_FFFF),
        ("fill", |d| d.fill[3] = u32::MAX),
    ];
    let arm7: [Case<false>; 4] = [
        ("channel 0 count 4000h", |d| {
            d.channels[0].count_now = 0x4000
        }),
        ("channel 3 count 10000h", |d| {
            d.channels[3].count_now = 0x1_0000
        }),
        ("channel 1 source 28 bits", |d| {
            d.channels[1].source = 0x0FFF_FFFF
        }),
        ("channel 3 destination 28 bits", |d| {
            d.channels[3].destination = 0x0FFF_FFFF;
        }),
    ];
    assert_eq!(
        (refused_valid_values(&arm9), refused_valid_values(&arm7)),
        (vec![], vec![])
    );
}

#[test]
fn given_an_enabled_pending_channel_when_checked_then_it_is_well_formed() {
    let mut dma = Arm9Dma::new();
    arm(&mut dma, 0, MAIN, OTHER, ENABLE | 4);
    assert!(dma.is_well_formed());
}

#[test]
fn given_a_default_device_when_compared_to_a_new_one_then_they_are_equal() {
    assert_eq!(Arm9Dma::default(), Arm9Dma::new());
}

#[test]
fn given_a_transfer_that_clears_and_sets_its_own_enable_when_it_finishes_then_the_new_start_is_kept_unverified()
 {
    let mut dma = Arm9Dma::new();
    let t = started(&mut dma, 0, MAIN, OTHER, ENABLE | 4);
    write32(&mut dma, cnt(0), 0);
    write32(&mut dma, cnt(0), ENABLE | 4);
    let effect = dma.finish(&t, END);
    assert_eq!(
        (
            effect,
            dma.pending(0),
            dma.control(0) >> 31,
            dma.is_well_formed()
        ),
        (Effect::default(), true, 1, true)
    );
}
