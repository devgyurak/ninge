// SPDX-License-Identifier: Apache-2.0
//! The 3D engine stub through `Machine` (`docs/hw/gx-stub.md`): the ARM9's bus reaching it, the ARM7
//! still reaching the sound registers at the same addresses, POWCNT1 bit 3 gating the geometry
//! block, the level-driven IF bit 21, and a savestate taken in the middle of a command.

use super::dma::{
    B_SELF, DST, ENABLE, SPIN, SRC, WORD, dma_events, machine, peek_words, poke_words, rom_with,
    run_frames, set7, set9,
};
use super::*;
use crate::cpu::Bus;
use crate::io::dma::Trigger;
use crate::scheduler::Participants;

const GXFIFO: u32 = 0x0400_0400;
const GXSTAT: u32 = 0x0400_0600;
const POWCNT1: u32 = 0x0400_0304;
const IF: u32 = 0x0400_0214;
const SOUND0CNT: u32 = 0x0400_0400;
const MTX_MODE: u32 = 0x10;
const MTX_PUSH: u32 = 0x11;
/// FIFO empty and less than half full.
const RESET: u32 = 0x0600_0000;
const ERROR: u32 = 1 << 15;
/// GXSTAT bit 13: the projection pointer.
const PROJECTION_LEVEL: u32 = 1 << 13;
const GXFIFO_IRQ: u32 = 1 << 21;
/// POWCNT1 as direct boot leaves it (`820Fh`) with the geometry engine bit (3) cleared.
const POWCNT1_GEOMETRY_OFF: u16 = 0x820F & !(1 << 3);

#[test]
fn given_a_booted_machine_when_the_arm9_reads_gxstat_then_the_fifo_reads_empty() {
    let mut m = machine();
    assert_eq!(m.bus9().read32(GXSTAT), RESET);
}

#[test]
fn given_the_arm7_when_it_reads_the_geometry_addresses_then_it_sees_the_sound_registers_not_gxstat()
{
    let mut m = machine();
    let at = m.inspect().arm7_clock() + 1;
    m.bus7(at).write32(SOUND0CNT, 0x7F);
    let arm7 = (
        m.bus7(at + 1).read32(SOUND0CNT),
        m.bus7(at + 1).read32(GXSTAT),
    );
    assert_eq!(arm7, (0x7F, 0));
}

#[test]
fn given_the_arm7_writes_a_command_word_when_the_arm9_reads_gxstat_then_nothing_changed() {
    let mut m = machine();
    let at = m.inspect().arm7_clock() + 1;
    m.bus7(at).write32(GXFIFO, MTX_PUSH);
    assert_eq!(m.bus9().read32(GXSTAT), RESET);
}

#[test]
fn given_a_push_in_projection_mode_when_the_arm9_reads_gxstat_then_the_projection_level_is_one() {
    let mut m = machine();
    m.bus9().write32(GXFIFO, MTX_PUSH);
    run_frames(&mut m, 1);
    assert_eq!(m.bus9().read32(GXSTAT), RESET | PROJECTION_LEVEL);
}

#[test]
fn given_a_push_when_inspected_then_the_pointer_is_visible() {
    let mut m = machine();
    m.bus9().write32(GXFIFO, MTX_PUSH);
    run_frames(&mut m, 1);
    assert_eq!(m.inspect().gpu3d().projection_pointer(), 1);
}

#[test]
fn given_the_geometry_engine_is_off_when_a_command_is_written_then_it_is_dropped() {
    let mut m = machine();
    m.bus9().write16(POWCNT1, POWCNT1_GEOMETRY_OFF);
    m.bus9().write32(GXFIFO, MTX_PUSH);
    assert_eq!(m.bus9().read32(GXSTAT), RESET);
}

#[test]
fn given_the_geometry_engine_is_off_when_a_port_is_written_then_it_is_dropped() {
    let mut m = machine();
    m.bus9().write16(POWCNT1, POWCNT1_GEOMETRY_OFF);
    m.bus9().write32(0x0400_0444, 0);
    assert_eq!(m.bus9().read32(GXSTAT), RESET);
}

#[test]
fn given_an_error_and_the_engine_off_when_bit_15_is_written_then_the_error_stays() {
    let mut m = machine();
    m.bus9().write32(GXFIFO, MTX_PUSH);
    m.bus9().write32(GXFIFO, MTX_PUSH);
    run_frames(&mut m, 1);
    m.bus9().write16(POWCNT1, POWCNT1_GEOMETRY_OFF);
    m.bus9().write32(GXSTAT, ERROR);
    assert_eq!(m.bus9().read32(GXSTAT), RESET | ERROR);
}

#[test]
fn given_an_error_and_the_engine_on_when_bit_15_is_written_then_the_error_clears() {
    let mut m = machine();
    m.bus9().write32(GXFIFO, MTX_PUSH);
    m.bus9().write32(GXFIFO, MTX_PUSH);
    run_frames(&mut m, 1);
    m.bus9().write32(GXSTAT, ERROR);
    assert_eq!(m.bus9().read32(GXSTAT), RESET);
}

#[test]
fn given_the_engine_was_off_when_it_is_powered_on_again_then_commands_run() {
    let mut m = machine();
    m.bus9().write16(POWCNT1, POWCNT1_GEOMETRY_OFF);
    m.bus9().write32(GXFIFO, MTX_PUSH);
    m.bus9().write16(POWCNT1, 0x820F);
    m.bus9().write32(GXFIFO, MTX_PUSH);
    run_frames(&mut m, 1);
    assert_eq!(m.bus9().read32(GXSTAT), RESET | PROJECTION_LEVEL);
}

#[test]
fn given_the_fifo_irq_mode_is_half_full_when_it_is_set_then_if_bit_21_is_raised() {
    let mut m = machine();
    m.bus9().write32(GXSTAT, 1 << 30);
    assert_eq!(m.inspect().irq9().if_() & GXFIFO_IRQ, GXFIFO_IRQ);
}

#[test]
fn given_the_fifo_irq_mode_is_empty_when_it_is_set_then_if_bit_21_is_raised() {
    let mut m = machine();
    m.bus9().write32(GXSTAT, 2 << 30);
    assert_eq!(m.inspect().irq9().if_() & GXFIFO_IRQ, GXFIFO_IRQ);
}

#[test]
fn given_the_fifo_irq_mode_is_off_when_gxstat_is_written_then_if_bit_21_is_not_raised() {
    let mut m = machine();
    m.bus9().write32(GXSTAT, 0);
    assert_eq!(m.inspect().irq9().if_() & GXFIFO_IRQ, 0);
}

#[test]
fn given_the_fifo_irq_is_on_when_if_bit_21_is_acknowledged_then_it_stays_set() {
    let mut m = machine();
    m.bus9().write32(GXSTAT, 1 << 30);
    m.bus9().write32(IF, GXFIFO_IRQ);
    assert_eq!(m.inspect().irq9().if_() & GXFIFO_IRQ, GXFIFO_IRQ);
}

#[test]
fn given_the_fifo_irq_is_on_when_other_if_bits_are_acknowledged_then_bit_21_stays_set() {
    let mut m = machine();
    m.bus9().write32(GXSTAT, 1 << 30);
    m.bus9().write32(IF, !0);
    assert_eq!(m.inspect().irq9().if_(), GXFIFO_IRQ);
}

#[test]
fn given_the_fifo_irq_mode_is_turned_off_when_if_bit_21_is_acknowledged_then_it_clears() {
    let mut m = machine();
    m.bus9().write32(GXSTAT, 1 << 30);
    m.bus9().write32(GXSTAT, 0);
    m.bus9().write32(IF, GXFIFO_IRQ);
    assert_eq!(m.inspect().irq9().if_() & GXFIFO_IRQ, 0);
}

#[test]
fn given_the_fifo_irq_mode_is_turned_off_when_nothing_is_acknowledged_then_the_bit_already_set_stays()
 {
    let mut m = machine();
    m.bus9().write32(GXSTAT, 1 << 30);
    m.bus9().write32(GXSTAT, 0);
    assert_eq!(m.inspect().irq9().if_() & GXFIFO_IRQ, GXFIFO_IRQ);
}

#[test]
fn given_the_fifo_irq_is_on_when_the_arm7_acknowledges_its_own_if_then_the_arm9_bit_is_unaffected()
{
    let mut m = machine();
    m.bus9().write32(GXSTAT, 1 << 30);
    let at = m.inspect().arm7_clock() + 1;
    m.bus7(at).write32(IF, GXFIFO_IRQ);
    assert_eq!(m.inspect().irq9().if_() & GXFIFO_IRQ, GXFIFO_IRQ);
}

/// A machine whose ARM9 sent `MTX_MODE` as a packed word with two pushes after it, and not yet the
/// mode's parameter.
fn mid_command() -> Machine {
    let mut m = machine();
    m.bus9()
        .write32(GXFIFO, MTX_MODE | MTX_PUSH << 8 | MTX_PUSH << 16);
    m
}

#[test]
fn given_a_savestate_in_the_middle_of_a_packed_word_when_loaded_then_the_parameter_completes_it() {
    let m = mid_command();
    let state = m.save_state();
    let mut loaded = machine();
    let result = loaded.load_state(&state);
    loaded.bus9().write32(GXFIFO, 1);
    run_frames(&mut loaded, 1);
    // Mode 1 and two pushes: the coordinate level is 2, GXSTAT bits 8-12.
    assert_eq!(
        (result, loaded.bus9().read32(GXSTAT)),
        (Ok(()), RESET | 2 << 8)
    );
}

#[test]
fn given_the_same_geometry_writes_when_two_machines_save_then_the_states_are_identical() {
    let (a, b) = (mid_command(), mid_command());
    assert_eq!(a.save_state(), b.save_state());
}

// ------------------------------------------------------------------- DMA start mode 7

/// DMA control: the transfer-complete IRQ (bit 30), a fixed destination (bits 21-22 = 2) and ARM9
/// start timing 7, the geometry command FIFO (bits 27-29).
const IRQ: u32 = 1 << 30;
const DEST_FIXED: u32 = 2 << 21;
const GEOMETRY_FIFO: u32 = 7 << 27;
/// IF bit 8, DMA channel 0.
const DMA0_IRQ: u32 = 1 << 8;

/// An unpacked `MTX_MODE` 1 and then `pushes` unpacked `MTX_PUSH` commands, one word each: the
/// coordinate pointer ends at `pushes` if every word reaches the engine in order.
fn display_list(pushes: u32) -> Vec<u32> {
    [MTX_MODE, 1]
        .into_iter()
        .chain((0..pushes).map(|_| MTX_PUSH))
        .collect()
}

/// A machine whose ARM9 enabled channel 0 in start mode 7 for a 300-word display list.
fn gx_dma_enabled() -> Machine {
    let mut m = machine();
    poke_words(&mut m, SRC, &display_list(298));
    set9(
        &mut m,
        0,
        SRC,
        GXFIFO,
        ENABLE | IRQ | WORD | DEST_FIXED | GEOMETRY_FIFO | 300,
    );
    m
}

/// The same, after one frame has run.
fn gx_dma_done() -> Machine {
    let mut m = gx_dma_enabled();
    run_frames(&mut m, 1);
    m
}

#[test]
fn given_a_mode_7_channel_of_300_words_when_it_runs_then_the_engine_took_every_word_in_order() {
    // MTX_MODE 1 first, then 298 pushes: the 6-bit coordinate pointer is 298 mod 64 = 42. A lost
    // word, or a push before the mode, would change it.
    let m = gx_dma_done();
    assert_eq!(m.inspect().gpu3d().coordinate_pointer(), 42);
}

#[test]
fn given_a_mode_7_channel_when_it_runs_then_the_enable_bit_clears() {
    let m = gx_dma_done();
    assert_eq!(m.inspect().dma9().control(0) >> 31, 0);
}

#[test]
fn given_a_mode_7_channel_with_the_irq_bit_when_it_runs_then_if_bit_8_is_raised() {
    let m = gx_dma_done();
    assert_eq!(m.inspect().irq9().if_() & DMA0_IRQ, DMA0_IRQ);
}

#[test]
fn given_a_mode_7_channel_when_it_is_enabled_but_has_not_run_then_the_engine_saw_nothing() {
    let m = gx_dma_enabled();
    assert_eq!(m.inspect().gpu3d().coordinate_pointer(), 0);
}

#[test]
fn given_a_mode_7_channel_when_the_transfer_ends_then_the_error_flag_shows_the_pointer_passed_31() {
    // 298 pushes take the pointer through entry 31 and past it.
    let m = gx_dma_done();
    assert!(m.inspect().gpu3d().stack_error());
}

#[test]
fn given_an_arm7_channel_with_the_same_start_bits_when_a_frame_runs_then_nothing_is_moved() {
    // Timing bits 27-29 = 7 leave 3 in the ARM7's bits 28-29: the wireless / GBA-slot start.
    let mut m = machine();
    poke_words(&mut m, SRC, &[5]);
    set7(&mut m, 0, 0, SRC, DST, ENABLE | WORD | GEOMETRY_FIFO | 1);
    run_frames(&mut m, 1);
    assert_eq!(
        (peek_words(&m, DST, 1), m.inspect().dma7().control(0) >> 31),
        (vec![0], 1)
    );
}

#[test]
fn given_a_savestate_with_a_mode_7_start_queued_when_loaded_and_run_then_it_equals_an_uninterrupted_run()
 {
    let mut uninterrupted = gx_dma_enabled();
    let saved = uninterrupted.save_state();
    run_frames(&mut uninterrupted, 1);

    let mut resumed = machine();
    let loaded = resumed.load_state(&saved);
    run_frames(&mut resumed, 1);
    assert_eq!(
        (loaded, resumed.save_state() == uninterrupted.save_state()),
        (Ok(()), true)
    );
}

#[test]
fn given_a_mode_7_channel_when_the_machine_runs_twice_then_the_savestate_bytes_are_identical() {
    assert_eq!(gx_dma_done().save_state(), gx_dma_done().save_state());
}

/// SWAP_BUFFERS (command word and parameter) then `pushes` unpacked pushes, through the ARM9's bus.
fn hold_the_engine_with(m: &mut Machine, pushes: u32) {
    m.bus9().write32(GXFIFO, 0x50);
    m.bus9().write32(GXFIFO, 0);
    (0..pushes).for_each(|_| m.bus9().write32(GXFIFO, MTX_PUSH));
}

#[test]
fn given_a_full_geometry_fifo_when_the_arm9_writes_once_more_then_the_view_reports_the_wait() {
    let mut m = machine();
    hold_the_engine_with(&mut m, 260);
    let mut bus = m.bus9();
    bus.write32(GXFIFO, MTX_PUSH);
    // SWAP_BUFFERS ends 392 bus cycles after line 192 starts, and the first push after it takes 17
    // bus cycles more (`geometry-engine.md`, "Executing a command").
    assert_eq!(bus.take_stall(), 192 * 4_260 + 784 + 34);
}

#[test]
fn given_a_stall_when_it_is_taken_then_a_second_take_reports_none() {
    let mut m = machine();
    hold_the_engine_with(&mut m, 260);
    let mut bus = m.bus9();
    bus.write32(GXFIFO, MTX_PUSH);
    bus.take_stall();
    assert_eq!(bus.take_stall(), 0);
}

#[test]
fn given_the_fifo_has_room_when_the_arm9_writes_then_the_view_reports_no_wait() {
    let mut m = machine();
    hold_the_engine_with(&mut m, 10);
    assert_eq!(m.bus9().take_stall(), 0);
}

/// `STR r1, [r0]`.
const STR_R1_R0: u32 = 0xE580_1000;

/// The master cycles one `STR` of a push to GXFIFO takes on a machine whose engine waits in
/// SWAP_BUFFERS with `queued` pushes behind it.
fn str_cost_with(queued: u32) -> u64 {
    let mut m = Machine::new();
    m.load_rom(rom_with(&[STR_R1_R0, B_SELF], &SPIN))
        .expect("synthetic image");
    hold_the_engine_with(&mut m, queued);
    m.arm9.regs_mut().set(0, GXFIFO);
    m.arm9.regs_mut().set(1, MTX_PUSH);
    let before = m.scheduler.now();
    m.step_arm9_instruction();
    m.scheduler.now() - before
}

#[test]
fn given_an_str_to_a_full_geometry_fifo_when_the_arm9_steps_then_the_wait_is_part_of_its_cost() {
    // The same instruction with room costs the instruction's own cycles; with a full FIFO it costs
    // those plus the wait for SWAP_BUFFERS and the first push after it.
    assert_eq!(
        str_cost_with(260) - str_cost_with(10),
        192 * 4_260 + 784 + 34
    );
}

// ------------------------------------------------------------------------ the ARM9 hold

#[test]
fn given_a_held_arm9_when_the_machine_runs_then_no_instruction_runs_until_the_hold_ends() {
    let mut m = machine();
    let before = m.arm9.clone();
    let start = m.scheduler.now();
    m.arm9_hold_until = start + 10_000;
    Participants::run_arm9_until(&mut m, start + 500);
    assert_eq!((m.scheduler.now(), m.arm9 == before), (start + 500, true));
}

#[test]
fn given_a_hold_when_the_target_is_past_its_end_then_the_clock_stops_at_the_hold_and_the_cpu_goes_on()
 {
    let mut m = machine();
    let start = m.scheduler.now();
    m.arm9_hold_until = start + 100;
    Participants::run_arm9_until(&mut m, start + 200);
    assert_eq!(m.scheduler.now(), start + 200);
}

#[test]
fn given_a_held_arm9_when_an_event_is_due_before_the_hold_ends_then_the_clock_stops_at_the_event() {
    let mut m = machine();
    let start = m.scheduler.now();
    m.arm9_hold_until = start + 1_000_000;
    let event = m.scheduler.next_time().expect("the line events are queued");
    Participants::run_arm9_until(&mut m, start + 1_000_000);
    assert_eq!(m.scheduler.now(), event);
}

#[test]
fn given_a_dma_burst_into_a_full_fifo_when_it_runs_then_the_arm9_is_held_for_the_wait() {
    // A mode-0 DMA of 300 words to GXFIFO behind SWAP_BUFFERS: the FIFO fills and the burst waits.
    let mut m = machine();
    hold_the_engine_with(&mut m, 0);
    poke_words(&mut m, SRC, &[MTX_PUSH; 300]);
    set9(&mut m, 0, SRC, GXFIFO, ENABLE | WORD | DEST_FIXED | 300);
    run_frames(&mut m, 1);
    assert!(m.arm9_hold_until > 0);
}

#[test]
fn given_a_geometry_burst_pause_when_saved_and_loaded_then_the_next_start_is_due_at_the_same_time()
{
    // A channel whose next start was queued for later (the pause between two bursts): the due time
    // is state, so a load rebuilds the event where it was.
    let mut m = machine();
    set9(&mut m, 0, SRC, DST, ENABLE | WORD | 1 << 27 | 4);
    m.io.trigger_dma(&mut m.scheduler, Trigger::VBlank, 5_000);
    let state = m.save_state();
    let mut loaded = machine();
    loaded.load_state(&state).expect("a state of this machine");
    assert_eq!(dma_events(&loaded), vec![(5_000, true, 0)]);
}
