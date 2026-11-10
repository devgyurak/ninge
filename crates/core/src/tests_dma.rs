// SPDX-License-Identifier: Apache-2.0
//! DMA driving both CPUs through `Machine`. Expected values come from `docs/hw/dma.md`
//! ("Enabling": enable reads 1 until the transfer is done; "Starting": Immediate starts when the
//! enable write retires, VBlank at line 192, HBlank in lines 0-191, channels 0 to 3 in order;
//! "Transferring": through the owning CPU's bus with the TCM off; "Fill"; "The end of a
//! transfer": IF bit 8 + n; "Edge cases": a register or an IPC FIFO as source or destination;
//! "Model": one `DmaStart` event per channel, atomic transfer, an ARM7 start fires at the end of
//! the round), `docs/hw/display-timing.md` (line 192 begins at `192 * LINE_CYCLES`; "Hooks for
//! later features"), `docs/hw/scheduler.md` ("Cost per instruction": ARM9 1 master cycle;
//! "Interleaving"; "Savestate checks"), `docs/hw/ipc.md` ("Sending", "Error", "Disabled FIFO")
//! and `docs/hw/arm9-memory-map.md` (main RAM mirrors every 4 MiB), never from running the code.
//! Tests named `..._unverified` pin a modeled choice from "Unverified".
//!
//! The programs are `b .`-style spins, so nothing but the DMA touches the data. Data lives at
//! `SRC` and `DST` in main RAM, clear of both CPUs' code.

use super::*;
use crate::cart::test_image::{Image, words};
use crate::cpu::Bus;
use crate::io::ipc::{IPCFIFOCNT_ADDR, IPCFIFOSEND_ADDR};
use crate::scheduler::{LINE_CYCLES, run_round};

const SRC: u32 = 0x0220_0000;
const DST: u32 = 0x0230_0000;

const ENABLE: u32 = 1 << 31;
const IRQ: u32 = 1 << 30;
const WORD: u32 = 1 << 26;
const REPEAT: u32 = 1 << 25;
const DEST_FIXED: u32 = 2 << 21;
const DEST_RELOAD: u32 = 3 << 21;
const SOURCE_FIXED: u32 = 2 << 23;
/// ARM9 start timings: VBlank is 1, HBlank is 2 (bits 27-29).
const ARM9_VBLANK: u32 = 1 << 27;
const ARM9_HBLANK: u32 = 2 << 27;
/// ARM7 start timings: VBlank is 1, the cartridge slot is 2 (bits 28-29).
const ARM7_VBLANK: u32 = 1 << 28;
const ARM7_CART: u32 = 2 << 28;
/// IF bit 8 + n, DMA channel n (`dma.md`, "IF bits").
const fn if_dma(n: u32) -> u32 {
    1 << (8 + n)
}

const fn sad_addr(n: u32) -> u32 {
    0x0400_00B0 + 12 * n
}
const fn dad_addr(n: u32) -> u32 {
    0x0400_00B4 + 12 * n
}
const fn cnt_addr(n: u32) -> u32 {
    0x0400_00B8 + 12 * n
}
const fn fill_addr(n: u32) -> u32 {
    0x0400_00E0 + 4 * n
}

const B_SELF: u32 = 0xEAFF_FFFE;
/// `mov r0, #0x55`, `mov r1, #0x66`, `b .`.
const SPIN: [u32; 3] = [0xE3A0_0055, 0xE3A0_1066, B_SELF];
/// `MCR p15, 0, R0, C9, C1, 0` (DTCM region register) and `MCR p15, 0, R1, C1, C0, 0` (control).
const MCR_DTCM_REGION: u32 = 0xEE09_0F11;
const MCR_CONTROL_R1: u32 = 0xEE01_1F10;
/// A DTCM of 16 KiB at `DTCM`; control bit 16 enables it (`cp15.md`).
const DTCM: u32 = 0x0210_0000;
/// The same main-RAM byte one 4 MiB mirror up, outside the 16 KiB DTCM.
const DTCM_BEHIND: u32 = DTCM + 0x40_0000;

fn rom_with(arm9: &[u32], arm7: &[u32]) -> Vec<u8> {
    let mut image = Image::valid();
    image.arm9_code = words(arm9);
    image.arm7_code = words(arm7);
    image.build()
}

fn machine() -> Machine {
    let mut m = Machine::new();
    m.load_rom(rom_with(&SPIN, &SPIN)).expect("synthetic image");
    m
}

/// A machine whose ARM9 has run the two MCRs that switch a DTCM on at `DTCM`.
fn machine_with_dtcm() -> Machine {
    let mut m = Machine::new();
    m.load_rom(rom_with(&[MCR_DTCM_REGION, MCR_CONTROL_R1, B_SELF], &SPIN))
        .expect("synthetic image");
    m.arm9.regs_mut().set(0, DTCM | 5 << 1);
    m.arm9.regs_mut().set(1, 1 << 16);
    m.step_arm9();
    m.step_arm9();
    m
}

fn poke_words(m: &mut Machine, addr: u32, values: &[u32]) {
    values
        .iter()
        .enumerate()
        .for_each(|(i, &v)| m.bus9().write32(addr + 4 * i as u32, v));
}

fn poke_halfwords(m: &mut Machine, addr: u32, values: &[u16]) {
    values
        .iter()
        .enumerate()
        .for_each(|(i, &v)| m.bus9().write16(addr + 2 * i as u32, v));
}

fn peek_words(m: &Machine, addr: u32, count: u32) -> Vec<u32> {
    (0..count).map(|i| m.arm9_peek32(addr + 4 * i)).collect()
}

fn peek_halfwords(m: &Machine, addr: u32, count: u32) -> Vec<u16> {
    (0..count).map(|i| m.arm9_peek16(addr + 2 * i)).collect()
}

/// Writes ARM9 channel `ch`'s three registers as a program would, at the current master time.
fn set9(m: &mut Machine, ch: u32, source: u32, destination: u32, control: u32) {
    m.bus9().write32(sad_addr(ch), source);
    m.bus9().write32(dad_addr(ch), destination);
    m.bus9().write32(cnt_addr(ch), control);
}

/// The same for the ARM7, at its clock `at`.
fn set7(m: &mut Machine, at: u64, ch: u32, source: u32, destination: u32, control: u32) {
    m.bus7(at).write32(sad_addr(ch), source);
    m.bus7(at).write32(dad_addr(ch), destination);
    m.bus7(at).write32(cnt_addr(ch), control);
}

fn rounds(m: &mut Machine, count: u32) {
    (0..count).for_each(|_| run_round(m));
}

/// Whole rounds until the machine is in `frame` at the start of `line`: the round that ends right
/// after that line's start event.
fn run_until(m: &mut Machine, frame: u64, line: u16) {
    (0..200_000).for_each(|_| {
        if m.frame_count() != frame || m.vcount() != line {
            run_round(m);
        }
    });
}

fn run_frames(m: &mut Machine, frames: u32) {
    (0..frames).for_each(|_| m.run_frame());
}

/// The pending DMA start events as `(time, arm9, channel)`, in firing order.
fn dma_events(m: &Machine) -> Vec<(u64, bool, u8)> {
    m.scheduler()
        .pending()
        .iter()
        .filter_map(|event| match event.kind {
            EventKind::DmaStart { arm9, channel } => Some((event.time, arm9, channel)),
            _ => None,
        })
        .collect()
}

/// DMAnCNT's enable bit as the ARM9 reads it, for channel `n`.
fn busy9(m: &Machine, n: u32) -> u32 {
    m.arm9_peek32(cnt_addr(n)) >> 31
}

// ---------------------------------------------------------------- Immediate start, ARM9

/// A machine with four words at `SRC` and an Immediate ARM9 channel 0 copying them to `DST`.
fn immediate9() -> Machine {
    let mut m = machine();
    poke_words(
        &mut m,
        SRC,
        &[0x1111_1111, 0x2222_2222, 0x3333_3333, 0x4444_4444],
    );
    set9(&mut m, 0, SRC, DST, ENABLE | WORD | 4);
    m
}

#[test]
fn given_an_immediate_write_to_dma0cnt_when_the_write_retires_then_a_start_event_is_queued_at_the_floor()
 {
    // The write runs at master time 0; an ARM9 instruction costs one master cycle
    // (`scheduler.md`, "Cost per instruction"), so the start is queued at 1 and never before.
    let m = immediate9();
    assert_eq!(dma_events(&m), vec![(1, true, 0)]);
}

#[test]
fn given_an_immediate_write_when_nothing_has_run_yet_then_the_destination_is_untouched_and_enable_reads_set()
 {
    let m = immediate9();
    assert_eq!(
        (peek_words(&m, DST, 4), busy9(&m, 0), m.dma9().pending(0)),
        (vec![0; 4], 1, true)
    );
}

#[test]
fn given_a_queued_immediate_start_when_one_round_runs_then_the_words_are_copied() {
    let mut m = immediate9();
    run_round(&mut m);
    assert_eq!(
        peek_words(&m, DST, 4),
        vec![0x1111_1111, 0x2222_2222, 0x3333_3333, 0x4444_4444]
    );
}

#[test]
fn given_a_queued_immediate_start_when_one_round_runs_then_enable_clears_and_no_start_remains() {
    let mut m = immediate9();
    run_round(&mut m);
    assert_eq!(
        (busy9(&m, 0), m.dma9().pending(0), dma_events(&m)),
        (0, false, vec![])
    );
}

#[test]
fn given_an_immediate_channel_when_the_registers_are_changed_before_the_round_then_the_latched_copies_move()
 {
    let mut m = immediate9();
    m.bus9().write32(sad_addr(0), SRC + 0x1000);
    m.bus9().write32(dad_addr(0), DST + 0x1000);
    rounds(&mut m, 1);
    assert_eq!(
        (peek_words(&m, DST, 1), peek_words(&m, DST + 0x1000, 1)),
        (vec![0x1111_1111], vec![0])
    );
}

#[test]
fn given_a_queued_immediate_start_when_enable_is_cleared_before_the_round_then_no_event_remains_and_nothing_moves()
 {
    let mut m = immediate9();
    m.bus9().write32(cnt_addr(0), WORD | 4);
    rounds(&mut m, 2);
    assert_eq!(
        (dma_events(&m), peek_words(&m, DST, 4), busy9(&m, 0)),
        (vec![], vec![0; 4], 0)
    );
}

#[test]
fn given_a_16_bit_immediate_channel_when_the_round_runs_then_only_halfwords_move() {
    let mut m = machine();
    poke_words(&mut m, SRC, &[0xAAAA_1111, 0xBBBB_2222]);
    set9(&mut m, 2, SRC, DST, ENABLE | 3);
    rounds(&mut m, 1);
    // Three halfwords move; the fourth, 0xBBBB, stays behind.
    assert_eq!(peek_halfwords(&m, DST, 4), vec![0x1111, 0xAAAA, 0x2222, 0]);
}

// ------------------------------------------------------------------ Immediate start, ARM7

#[test]
fn given_an_arm7_immediate_write_when_the_master_clock_is_at_the_write_then_the_event_is_queued_at_that_time()
 {
    // An ARM7 event is floored at the master clock, 0 here (`dma.md`, "Model").
    let mut m = machine();
    set7(&mut m, 0, 0, SRC, DST, ENABLE | WORD | 1);
    assert_eq!(dma_events(&m), vec![(0, false, 0)]);
}

#[test]
fn given_an_arm7_start_written_while_it_lags_the_clock_when_the_round_ends_then_it_fires_at_the_master_clock()
 {
    // After one round both clocks are at 64 (one slice). The ARM7 writes at its clock 10, which
    // is behind the master clock, so its start fires at 64, the end of the round, not at 10.
    let mut m = machine();
    run_round(&mut m);
    poke_words(&mut m, SRC, &[0x0BAD_CAFE]);
    set7(&mut m, 10, 0, SRC, DST, ENABLE | WORD | 1);
    assert_eq!(
        (
            m.scheduler().now(),
            dma_events(&m),
            m.arm7_peek32(10, cnt_addr(0)) >> 31
        ),
        (64, vec![(64, false, 0)], 1)
    );
}

#[test]
fn given_an_arm7_start_queued_late_when_the_next_round_runs_then_the_words_are_copied_and_enable_clears()
 {
    let mut m = machine();
    run_round(&mut m);
    poke_words(&mut m, SRC, &[0x0BAD_CAFE]);
    set7(&mut m, 10, 0, SRC, DST, ENABLE | WORD | 1);
    let before = (peek_words(&m, DST, 1), m.dma7().pending(0));
    run_round(&mut m);
    assert_eq!(
        (
            before,
            peek_words(&m, DST, 1),
            m.arm7_peek32(64, cnt_addr(0)) >> 31,
            dma_events(&m)
        ),
        ((vec![0], true), vec![0x0BAD_CAFE], 0, vec![])
    );
}

// ------------------------------------------------------------------------------- IRQ

/// `(channel, IF)` for each ARM9 channel that does not raise exactly IF bit 8 + n after an
/// Immediate transfer with the IRQ bit set.
fn arm9_irq_failures() -> Vec<(u32, u32)> {
    (0..4)
        .filter_map(|ch| {
            let mut m = machine();
            set9(&mut m, ch, SRC, DST, ENABLE | IRQ | WORD | 1);
            rounds(&mut m, 1);
            let got = m.irq9().if_();
            (got != if_dma(ch)).then_some((ch, got))
        })
        .collect()
}

#[test]
fn given_each_arm9_channel_with_the_irq_bit_when_its_transfer_ends_then_if_bit_8_plus_n_is_set() {
    assert_eq!(arm9_irq_failures(), vec![]);
}

/// `(channel, IF)` for each ARM7 channel that does not raise exactly IF bit 8 + n on the ARM7.
fn arm7_irq_failures() -> Vec<(u32, u32)> {
    (0..4)
        .filter_map(|ch| {
            let mut m = machine();
            set7(&mut m, 0, ch, SRC, DST, ENABLE | IRQ | WORD | 1);
            rounds(&mut m, 1);
            let got = m.irq7().if_();
            (got != if_dma(ch) || m.irq9().if_() != 0).then_some((ch, got))
        })
        .collect()
}

#[test]
fn given_each_arm7_channel_with_the_irq_bit_when_its_transfer_ends_then_if_bit_8_plus_n_is_set_on_the_arm7_only()
 {
    assert_eq!(arm7_irq_failures(), vec![]);
}

#[test]
fn given_an_arm9_channel_with_the_irq_bit_when_the_start_is_queued_but_not_run_then_if_is_still_clear()
 {
    let mut m = machine();
    set9(&mut m, 1, SRC, DST, ENABLE | IRQ | WORD | 1);
    assert_eq!((m.irq9().if_(), m.irq7().if_()), (0, 0));
}

#[test]
fn given_an_arm9_channel_without_the_irq_bit_when_its_transfer_ends_then_if_stays_clear() {
    let mut m = machine();
    set9(&mut m, 0, SRC, DST, ENABLE | WORD | 1);
    rounds(&mut m, 1);
    assert_eq!((m.irq9().if_(), m.irq7().if_()), (0, 0));
}

// ------------------------------------------------------------------------------ VBlank

/// ARM9 channel 0 waits for VBlank to copy four words from `SRC` to `DST`.
fn vblank9() -> Machine {
    let mut m = machine();
    poke_words(&mut m, SRC, &[1, 2, 3, 4]);
    set9(&mut m, 0, SRC, DST, ENABLE | WORD | ARM9_VBLANK | 4);
    m
}

#[test]
fn given_a_vblank_channel_when_line_191_has_started_then_nothing_is_queued_and_enable_is_set() {
    let mut m = vblank9();
    run_until(&mut m, 0, 191);
    assert_eq!(
        (
            m.vcount(),
            dma_events(&m),
            m.dma9().pending(0),
            busy9(&m, 0),
            peek_words(&m, DST, 4)
        ),
        (191, vec![], false, 1, vec![0; 4])
    );
}

#[test]
fn given_a_vblank_channel_when_line_192_starts_then_its_start_is_queued_at_that_line_start() {
    let mut m = vblank9();
    run_until(&mut m, 0, 192);
    assert_eq!(
        (m.vcount(), dma_events(&m), peek_words(&m, DST, 4)),
        (192, vec![(192 * LINE_CYCLES, true, 0)], vec![0; 4])
    );
}

#[test]
fn given_a_vblank_channel_when_the_round_after_line_192_runs_then_the_words_are_copied_and_enable_clears()
 {
    let mut m = vblank9();
    run_until(&mut m, 0, 192);
    run_round(&mut m);
    assert_eq!(
        (peek_words(&m, DST, 4), busy9(&m, 0), dma_events(&m).len()),
        (vec![1, 2, 3, 4], 0, 0)
    );
}

#[test]
fn given_an_arm7_vblank_channel_when_line_192_starts_and_a_round_runs_then_the_words_are_copied_with_the_arm7_irq()
 {
    let mut m = machine();
    poke_words(&mut m, SRC, &[7, 8]);
    set7(
        &mut m,
        0,
        1,
        SRC,
        DST,
        ENABLE | IRQ | WORD | ARM7_VBLANK | 2,
    );
    run_until(&mut m, 0, 192);
    let queued = dma_events(&m);
    run_round(&mut m);
    assert_eq!(
        (
            queued,
            peek_words(&m, DST, 2),
            m.irq7().if_(),
            m.irq9().if_()
        ),
        (
            vec![(192 * LINE_CYCLES, false, 1)],
            vec![7, 8],
            if_dma(1),
            0
        )
    );
}

#[test]
fn given_vblank_channels_on_both_cpus_when_line_192_starts_then_each_cpus_channel_runs() {
    let mut m = machine();
    poke_words(&mut m, SRC, &[0x0909_0909]);
    set9(&mut m, 0, SRC, DST, ENABLE | WORD | ARM9_VBLANK | 1);
    set7(
        &mut m,
        0,
        0,
        SRC,
        DST + 0x100,
        ENABLE | WORD | ARM7_VBLANK | 1,
    );
    run_until(&mut m, 0, 192);
    run_round(&mut m);
    assert_eq!(
        (peek_words(&m, DST, 1), peek_words(&m, DST + 0x100, 1)),
        (vec![0x0909_0909], vec![0x0909_0909])
    );
}

#[test]
fn given_a_vblank_channel_enabled_during_vblank_when_the_frame_wraps_then_nothing_has_run_before_the_next_line_192()
 {
    // Enabling during VBlank waits for the next one (`dma.md`, "Starting").
    let mut m = machine();
    poke_words(&mut m, SRC, &[1, 2, 3, 4]);
    run_until(&mut m, 0, 200);
    set9(&mut m, 0, SRC, DST, ENABLE | WORD | ARM9_VBLANK | 4);
    let waiting = (m.dma9().pending(0), dma_events(&m));
    run_until(&mut m, 1, 191);
    assert_eq!(
        (
            waiting,
            peek_words(&m, DST, 4),
            m.dma9().pending(0),
            busy9(&m, 0)
        ),
        ((false, vec![]), vec![0; 4], false, 1)
    );
}

#[test]
fn given_a_vblank_channel_enabled_during_vblank_when_the_next_line_192_starts_then_it_runs() {
    let mut m = machine();
    poke_words(&mut m, SRC, &[1, 2, 3, 4]);
    run_until(&mut m, 0, 200);
    set9(&mut m, 0, SRC, DST, ENABLE | WORD | ARM9_VBLANK | 4);
    run_until(&mut m, 1, 192);
    run_round(&mut m);
    assert_eq!(
        (peek_words(&m, DST, 4), busy9(&m, 0)),
        (vec![1, 2, 3, 4], 0)
    );
}

// ------------------------------------------------------------------------------ HBlank

/// ARM9 channel 0 moves one halfword per HBlank from a fixed `SRC` to an incrementing `DST`,
/// repeating.
fn hblank9(extra: u32) -> Machine {
    let mut m = machine();
    poke_halfwords(&mut m, SRC, &[0xBEEF]);
    set9(
        &mut m,
        0,
        SRC,
        DST,
        ENABLE | REPEAT | SOURCE_FIXED | ARM9_HBLANK | extra | 1,
    );
    m
}

#[test]
fn given_a_repeating_hblank_channel_when_line_192_starts_then_one_unit_moved_per_visible_line() {
    // HBlank lines 0-191 give 192 transfers of one unit; the halfword after them is untouched.
    let mut m = hblank9(0);
    run_until(&mut m, 0, 192);
    run_round(&mut m);
    let expected = [vec![0xBEEF; 192], vec![0; 8]].concat();
    assert_eq!(peek_halfwords(&m, DST, 200), expected);
}

#[test]
fn given_a_repeating_hblank_channel_when_it_has_run_it_then_enable_stays_set_and_the_next_start_waits()
 {
    let mut m = hblank9(0);
    run_until(&mut m, 0, 100);
    assert_eq!((busy9(&m, 0), m.dma9().pending(0)), (1, false));
}

#[test]
fn given_a_repeating_hblank_channel_when_a_whole_frame_has_run_then_no_unit_moved_in_the_blank_lines_unverified()
 {
    // TODO(hw-unverified): `dma.md` "Unverified": no HBlank start in lines 192-262.
    let mut m = hblank9(0);
    run_until(&mut m, 1, 0);
    let expected = [vec![0xBEEF; 192], vec![0; 8]].concat();
    assert_eq!(peek_halfwords(&m, DST, 200), expected);
}

#[test]
fn given_a_repeating_hblank_channel_with_destination_control_3_when_it_runs_then_the_destination_reloads_and_the_source_continues()
 {
    // Source increments one halfword per line (not reloaded), the destination reloads to `DST`
    // each time (`dma.md`, "The end of a transfer"): the last value written is line 191's, 192.
    let mut m = machine();
    let source: Vec<u16> = (1..=192).collect();
    poke_halfwords(&mut m, SRC, &source);
    set9(
        &mut m,
        0,
        SRC,
        DST,
        ENABLE | REPEAT | DEST_RELOAD | ARM9_HBLANK | 1,
    );
    run_until(&mut m, 0, 192);
    run_round(&mut m);
    assert_eq!(peek_halfwords(&m, DST, 3), vec![192, 0, 0]);
}

#[test]
fn given_an_arm7_channel_with_the_cartridge_start_when_a_frame_runs_then_it_never_starts() {
    // The ARM7 has no HBlank start; timing 2 is the cartridge slot (`dma.md`, "Start timing").
    let mut m = machine();
    poke_words(&mut m, SRC, &[5]);
    set7(&mut m, 0, 0, SRC, DST, ENABLE | WORD | ARM7_CART | 1);
    run_until(&mut m, 1, 0);
    assert_eq!(
        (
            peek_words(&m, DST, 1),
            m.dma7().control(0) >> 31,
            dma_events(&m)
        ),
        (vec![0], 1, vec![])
    );
}

// --------------------------------------------------------------------- channel order

/// Four ARM9 VBlank channels chained through `DST`: channel 0 copies `SRC` to `DST`, channel 1
/// `DST` to `DST + 4`, and so on, enabled in the order 3, 2, 1, 0. They only all end up holding the
/// word if they run in the order 0, 1, 2, 3.
fn chained_vblank9() -> Machine {
    let mut m = machine();
    poke_words(&mut m, SRC, &[0x1234_5678]);
    let control = ENABLE | WORD | ARM9_VBLANK | 1;
    set9(&mut m, 3, DST + 8, DST + 12, control);
    set9(&mut m, 2, DST + 4, DST + 8, control);
    set9(&mut m, 1, DST, DST + 4, control);
    set9(&mut m, 0, SRC, DST, control);
    m
}

#[test]
fn given_four_vblank_channels_when_line_192_starts_then_the_events_are_queued_0_to_3_at_one_time() {
    let mut m = chained_vblank9();
    run_until(&mut m, 0, 192);
    let t = 192 * LINE_CYCLES;
    assert_eq!(
        dma_events(&m),
        vec![(t, true, 0), (t, true, 1), (t, true, 2), (t, true, 3)]
    );
}

#[test]
fn given_four_vblank_channels_enabled_in_reverse_when_line_192_runs_then_they_run_in_channel_order()
{
    let mut m = chained_vblank9();
    run_until(&mut m, 0, 192);
    run_round(&mut m);
    assert_eq!(peek_words(&m, DST, 4), vec![0x1234_5678; 4]);
}

// ---------------------------------------------------------------------------- TCM

#[test]
fn given_a_mapped_dtcm_when_a_dma_writes_into_its_window_then_the_word_lands_in_main_ram_not_the_tcm()
 {
    let mut m = machine_with_dtcm();
    poke_words(&mut m, SRC, &[0xFEED_F00D]);
    set9(&mut m, 0, SRC, DTCM, ENABLE | WORD | 1);
    rounds(&mut m, 1);
    // The CPU's view of the window is the TCM, which the DMA did not reach; the same RAM byte
    // one mirror up lies outside the window (`arm9-memory-map.md`, "Main RAM").
    assert_eq!(
        (m.arm9_peek32(DTCM), m.arm9_peek32(DTCM_BEHIND)),
        (0, 0xFEED_F00D)
    );
}

#[test]
fn given_a_mapped_dtcm_when_a_dma_reads_from_its_window_then_it_reads_the_ram_behind_not_the_tcm() {
    let mut m = machine_with_dtcm();
    m.bus9().write32(DTCM, 0x7C7C_7C7C);
    m.bus9().write32(DTCM_BEHIND, 0x0AAB_0AAB);
    set9(&mut m, 0, DTCM, DST, ENABLE | WORD | 1);
    rounds(&mut m, 1);
    assert_eq!(
        (m.arm9_peek32(DTCM), peek_words(&m, DST, 1)),
        (0x7C7C_7C7C, vec![0x0AAB_0AAB])
    );
}

// ---------------------------------------------------- registers and FIFOs as endpoints

#[test]
fn given_a_dma_writing_another_channels_control_when_it_ends_then_that_channel_is_armed_and_queued()
{
    // Channel 0 stores the word `ENABLE | 1` into DMA1CNT: a register write like any other
    // (`dma.md`, "Edge cases"), so channel 1 starts as if a CPU had enabled it. The write lands
    // at the master time the transfer ran (1), and an ARM9 write's floor is one cycle later.
    let mut m = machine();
    poke_words(&mut m, SRC, &[ENABLE | 1]);
    poke_halfwords(&mut m, SRC + 0x100, &[0xABCD]);
    set9(&mut m, 1, SRC + 0x100, DST, 1);
    set9(&mut m, 0, SRC, cnt_addr(1), ENABLE | WORD | 1);
    rounds(&mut m, 1);
    assert_eq!(
        (
            m.dma9().pending(1),
            dma_events(&m),
            peek_halfwords(&m, DST, 1),
            busy9(&m, 0)
        ),
        (true, vec![(2, true, 1)], vec![0], 0)
    );
}

#[test]
fn given_a_dma_that_armed_another_channel_when_the_next_round_runs_then_that_channel_moves_its_data()
 {
    let mut m = machine();
    poke_words(&mut m, SRC, &[ENABLE | 1]);
    poke_halfwords(&mut m, SRC + 0x100, &[0xABCD]);
    set9(&mut m, 1, SRC + 0x100, DST, 1);
    set9(&mut m, 0, SRC, cnt_addr(1), ENABLE | WORD | 1);
    rounds(&mut m, 2);
    assert_eq!(
        (
            peek_halfwords(&m, DST, 1),
            busy9(&m, 0),
            busy9(&m, 1),
            dma_events(&m)
        ),
        (vec![0xABCD], 0, 0, vec![])
    );
}

#[test]
fn given_a_dma_reading_another_channels_source_register_when_it_runs_then_the_destination_gets_its_value()
 {
    let mut m = machine();
    m.bus9().write32(sad_addr(1), 0x0212_3456);
    set9(&mut m, 0, sad_addr(1), DST, ENABLE | WORD | 1);
    rounds(&mut m, 1);
    assert_eq!(peek_words(&m, DST, 1), vec![0x0212_3456]);
}

#[test]
fn given_a_dma_whose_destination_is_its_own_registers_when_it_runs_then_the_transfer_works_on_its_latched_copies()
 {
    // Two words go to DMA0DAD then DMA0CNT, which hold the register values while the transfer
    // runs. The destination moved on by its latched copy, so the second word still reaches DMA0CNT
    // (clearing it) even though DMA0DAD now holds another address (`dma.md`, "Edge cases").
    let mut m = machine();
    poke_words(&mut m, SRC, &[DST + 0x40, 0]);
    set9(&mut m, 0, SRC, dad_addr(0), ENABLE | WORD | 2);
    rounds(&mut m, 1);
    assert_eq!(
        (
            m.dma9().destination(0),
            m.dma9().control(0),
            m.dma9().pending(0),
            dma_events(&m)
        ),
        (DST + 0x40, 0, false, vec![])
    );
}

#[test]
fn given_a_fixed_destination_at_ipcfifosend_when_three_words_move_then_the_send_fifo_holds_them_in_order()
 {
    let mut m = machine();
    m.bus9().write16(IPCFIFOCNT_ADDR, 1 << 15);
    poke_words(&mut m, SRC, &[0x11, 0x22, 0x33]);
    set9(
        &mut m,
        0,
        SRC,
        IPCFIFOSEND_ADDR,
        ENABLE | WORD | DEST_FIXED | 3,
    );
    rounds(&mut m, 1);
    assert_eq!(
        m.ipc().send_words::<true>().collect::<Vec<u32>>(),
        vec![0x11, 0x22, 0x33]
    );
}

#[test]
fn given_seventeen_words_to_an_enabled_send_fifo_when_the_dma_ends_then_16_are_held_and_the_error_bit_is_set()
 {
    // A full FIFO sets the sender's error bit (IPCFIFOCNT bit 14) and keeps its 16 words
    // (`ipc.md`, "Sending", "Edge cases").
    let mut m = machine();
    m.bus9().write16(IPCFIFOCNT_ADDR, 1 << 15);
    let source: Vec<u32> = (1..=17).collect();
    poke_words(&mut m, SRC, &source);
    set9(
        &mut m,
        0,
        SRC,
        IPCFIFOSEND_ADDR,
        ENABLE | WORD | DEST_FIXED | 17,
    );
    rounds(&mut m, 1);
    assert_eq!(
        (
            m.ipc().send_words::<true>().collect::<Vec<u32>>(),
            m.ipc().control::<true>() & (1 << 14)
        ),
        ((1..=16).collect::<Vec<u32>>(), 1 << 14)
    );
}

#[test]
fn given_a_disabled_send_fifo_when_a_dma_writes_it_then_no_word_is_stored_and_no_error_is_set() {
    let mut m = machine();
    poke_words(&mut m, SRC, &[0x11, 0x22]);
    set9(
        &mut m,
        0,
        SRC,
        IPCFIFOSEND_ADDR,
        ENABLE | WORD | DEST_FIXED | 2,
    );
    rounds(&mut m, 1);
    assert_eq!(
        (
            m.ipc().send_words::<true>().count(),
            m.ipc().control::<true>() & (1 << 14)
        ),
        (0, 0)
    );
}

#[test]
fn given_an_arm7_dma_to_its_send_fifo_when_it_runs_then_the_arm9_reads_the_words_in_order() {
    let mut m = machine();
    m.bus7(0).write16(IPCFIFOCNT_ADDR, 1 << 15);
    poke_words(&mut m, SRC, &[0xA1, 0xA2]);
    set7(
        &mut m,
        0,
        0,
        SRC,
        IPCFIFOSEND_ADDR,
        ENABLE | WORD | DEST_FIXED | 2,
    );
    rounds(&mut m, 1);
    assert_eq!(
        m.ipc().send_words::<false>().collect::<Vec<u32>>(),
        vec![0xA1, 0xA2]
    );
}

// ---------------------------------------------------------------------------- fill

#[test]
fn given_a_fill_register_when_a_dma_uses_it_as_a_fixed_source_then_the_fill_word_is_written_over_and_over()
 {
    let mut m = machine();
    m.bus9().write32(fill_addr(0), 0xDEAD_BEEF);
    set9(
        &mut m,
        0,
        fill_addr(0),
        DST,
        ENABLE | WORD | SOURCE_FIXED | 4,
    );
    rounds(&mut m, 1);
    assert_eq!(
        peek_words(&m, DST, 5),
        vec![0xDEAD_BEEF, 0xDEAD_BEEF, 0xDEAD_BEEF, 0xDEAD_BEEF, 0]
    );
}

#[test]
fn given_a_fill_word_written_by_the_arm9_when_the_arm7_dma_reads_that_address_then_it_reads_zero_unverified()
 {
    // The fill block is not on the ARM7's bus (`dma.md`, "Registers"); an unmapped I/O address
    // reads 0 in this model (`io/map.rs`, `decode`).
    let mut m = machine();
    m.bus9().write32(fill_addr(0), 0xDEAD_BEEF);
    poke_words(&mut m, DST, &[0xFFFF_FFFF]);
    set7(&mut m, 0, 0, fill_addr(0), DST, ENABLE | WORD | 1);
    rounds(&mut m, 1);
    assert_eq!(peek_words(&m, DST, 1), vec![0]);
}

// ------------------------------------------------------- accessors and independence

#[test]
fn given_registers_written_by_each_cpu_when_read_through_the_accessors_then_each_shows_only_its_own()
 {
    let mut m = machine();
    m.bus9().write32(sad_addr(1), 0x0212_0000);
    m.bus7(0).write32(sad_addr(1), 0x0234_0000);
    m.bus9().write32(fill_addr(2), 0x55);
    assert_eq!(
        (
            m.dma9().source(1),
            m.dma7().source(1),
            m.dma9().fill(2),
            m.dma7().fill(2)
        ),
        (0x0212_0000, 0x0234_0000, 0x55, 0)
    );
}

#[test]
fn given_an_arm9_register_write_when_the_arm7_reads_the_same_address_then_it_reads_its_own_register()
 {
    let mut m = machine();
    m.bus9().write32(sad_addr(0), 0x0212_0000);
    assert_eq!(m.arm7_peek32(0, sad_addr(0)), 0);
}

#[test]
fn given_an_arm9_channel_when_it_runs_then_the_arm7_channels_are_untouched() {
    let mut m = immediate9();
    rounds(&mut m, 1);
    assert_eq!(*m.dma7(), Dma::<false>::new());
}

// -------------------------------------------------------------------------- determinism

/// A machine with a mix of pending and waiting channels on both CPUs.
fn busy_machine() -> Machine {
    let mut m = machine();
    poke_words(&mut m, SRC, &[0xA0, 0xA1, 0xA2, 0xA3]);
    poke_halfwords(&mut m, SRC + 0x100, &[0x5A5A]);
    set9(&mut m, 0, SRC, DST, ENABLE | IRQ | WORD | 4);
    set9(
        &mut m,
        1,
        SRC + 0x100,
        DST + 0x100,
        ENABLE | REPEAT | SOURCE_FIXED | ARM9_HBLANK | 1,
    );
    set7(
        &mut m,
        0,
        0,
        SRC,
        DST + 0x200,
        ENABLE | WORD | ARM7_VBLANK | 2,
    );
    set7(&mut m, 0, 1, SRC, DST + 0x300, ENABLE | IRQ | WORD | 1);
    m
}

#[test]
fn given_the_same_dma_setup_when_run_twice_then_the_savestate_bytes_are_identical() {
    let run = || {
        let mut m = busy_machine();
        run_frames(&mut m, 1);
        m.save_state()
    };
    assert!(run() == run());
}

#[test]
fn given_a_machine_with_pending_channels_when_saved_loaded_and_run_a_frame_then_it_equals_an_uninterrupted_run()
 {
    let mut uninterrupted = busy_machine();
    let saved = uninterrupted.save_state();
    run_frames(&mut uninterrupted, 1);

    let mut resumed = machine();
    resumed
        .load_state(&saved)
        .expect("a state saved by the same build loads");
    run_frames(&mut resumed, 1);
    assert!(resumed.save_state() == uninterrupted.save_state());
}

#[test]
fn given_a_machine_with_waiting_channels_when_saved_mid_frame_and_continued_then_it_equals_an_uninterrupted_run()
 {
    let mut uninterrupted = busy_machine();
    run_until(&mut uninterrupted, 0, 100);
    let saved = uninterrupted.save_state();
    run_frames(&mut uninterrupted, 1);

    let mut resumed = machine();
    resumed
        .load_state(&saved)
        .expect("a state saved by the same build loads");
    run_frames(&mut resumed, 1);
    assert!(resumed.save_state() == uninterrupted.save_state());
}

#[test]
fn given_a_saved_machine_with_pending_channels_when_loaded_then_the_channels_and_events_match() {
    let original = busy_machine();
    let mut loaded = machine();
    loaded.load_state(&original.save_state()).expect("loads");
    assert_eq!(
        (
            loaded.dma9().clone(),
            loaded.dma7().clone(),
            dma_events(&loaded)
        ),
        (
            original.dma9().clone(),
            original.dma7().clone(),
            dma_events(&original)
        )
    );
}

#[test]
fn given_a_saved_machine_with_pending_channels_when_loaded_and_saved_again_then_the_bytes_are_identical()
 {
    let original = busy_machine();
    let saved = original.save_state();
    let mut loaded = machine();
    loaded.load_state(&saved).expect("loads");
    assert!(loaded.save_state() == saved);
}
