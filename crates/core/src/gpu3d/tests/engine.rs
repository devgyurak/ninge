// SPDX-License-Identifier: Apache-2.0
//! The geometry engine's timing as a device: when a command starts and ends, what GXSTAT shows
//! meanwhile, the stall of a write to a full FIFO, SWAP_BUFFERS' wait for VBlank and the levels the
//! FIFO feeds. Expected values come from `docs/hw/geometry-engine.md` ("Executing a command",
//! "The FIFO IRQ, the DMA start and the stall", "SWAP_BUFFERS"), worked out by hand. Times are
//! master cycles: twice GBATEK's bus cycles. A line is 4,260 and VBlank begins at line 192, which
//! is 817,920.
//!
//! The helpers here write without draining, unlike `write` in the parent module, so the engine
//! stays mid-command.

use super::*;
use crate::io::irq::GXFIFO_IRQ;
use crate::io::timing::next_vblank_start;

/// GXSTAT bit 27: busy.
const BUSY: u32 = 1 << 27;
/// GXSTAT bit 14: a push, pop, store or restore is executing.
const STACK_BUSY: u32 = 1 << 14;
/// The start of line 192 and the end of SWAP_BUFFERS' own 392 bus cycles after it.
const VBLANK: u64 = 192 * 4_260;
const SWAP_ENDS: u64 = VBLANK + 784;

/// One write at master time `now`, with no draining.
fn raw_at(gpu: &mut Gpu3d, now: u64, offset: u32, value: u32) -> Effect {
    gpu.write(offset, Width::Word, value, cx_at(now))
}

/// A GXFIFO word written at time 0, with no draining.
fn raw(gpu: &mut Gpu3d, value: u32) -> Effect {
    raw_at(gpu, 0, GXFIFO, value)
}

/// `count` writes of 0 to the port of command `id` at time 0, with no draining.
fn raw_port(gpu: &mut Gpu3d, id: u8, count: u32) {
    (0..count.max(1)).for_each(|_| {
        raw_at(gpu, 0, port(id), 0);
    });
}

/// A device whose engine waits in SWAP_BUFFERS, with `entries` pushes queued behind it.
fn waiting_with(entries: u32) -> Gpu3d {
    let mut gpu = Gpu3d::new();
    raw(&mut gpu, SWAP_BUFFERS.into());
    raw(&mut gpu, 0);
    (0..entries).for_each(|_| {
        raw(&mut gpu, MTX_PUSH.into());
    });
    gpu
}

#[test]
fn given_an_idle_engine_when_a_push_is_written_then_it_is_busy_for_34_master_cycles() {
    let mut gpu = Gpu3d::new();
    raw(&mut gpu, MTX_PUSH.into());
    assert_eq!(
        (gxstat(&gpu), gpu.slot_due(0)),
        (RESET | BUSY | STACK_BUSY, Some(34))
    );
}

#[test]
fn given_a_push_in_flight_when_its_event_fires_then_the_pointer_has_moved_and_the_engine_is_idle() {
    let mut gpu = Gpu3d::new();
    raw(&mut gpu, MTX_PUSH.into());
    gpu.slot_fire(0, 34);
    assert_eq!((gxstat(&gpu), gpu.slot_due(0)), (RESET | 1 << 13, None));
}

#[test]
fn given_a_push_in_flight_when_gxstat_is_read_then_the_pointer_has_not_moved_yet() {
    let mut gpu = in_mode(1);
    raw(&mut gpu, MTX_PUSH.into());
    assert_eq!(levels(&gpu), 0);
}

/// Each command's cycles with the matrix mode as it starts: `(id, parameters, mode 0 time)` and
/// the cases whose time differs from the table, as `(id, got)`.
fn cycle_failures(mode: u32, cases: &[(u8, u32, u64)]) -> Vec<(u8, Option<u64>)> {
    cases
        .iter()
        .filter_map(|&(id, params, want)| {
            let mut gpu = in_mode(mode);
            raw_port(&mut gpu, id, params);
            let got = gpu.slot_due(0);
            (got != Some(want)).then_some((id, got))
        })
        .collect()
}

#[test]
fn given_each_command_when_it_is_written_in_full_then_it_takes_the_cycles_gbatek_gives() {
    // GBATEK's bus cycles, doubled.
    let cases = [
        (0x10, 1, 2),
        (0x11, 0, 34),
        (0x12, 1, 72),
        (0x13, 1, 34),
        (0x14, 1, 72),
        (0x15, 0, 38),
        (0x16, 16, 68),
        (0x17, 12, 60),
        (0x18, 16, 70),
        (0x19, 12, 62),
        (0x1A, 9, 56),
        (0x1B, 3, 44),
        (0x1C, 3, 44),
        (0x20, 1, 2),
        (0x21, 1, 18),
        (0x22, 1, 2),
        (0x23, 2, 18),
        (0x24, 1, 16),
        (0x25, 1, 16),
        (0x26, 1, 16),
        (0x27, 1, 16),
        (0x28, 1, 16),
        (0x29, 1, 2),
        (0x2A, 1, 2),
        (0x2B, 1, 2),
        (0x30, 1, 8),
        (0x31, 1, 8),
        (0x32, 1, 12),
        (0x33, 1, 2),
        (0x34, 32, 64),
        (0x40, 1, 2),
        (0x41, 0, 2),
        (0x60, 1, 2),
        (0x70, 3, 206),
        (0x71, 2, 18),
        (0x72, 1, 10),
    ];
    assert_eq!(cycle_failures(0, &cases), vec![]);
}

#[test]
fn given_matrix_mode_2_when_a_multiply_or_translate_is_written_then_it_takes_30_bus_cycles_more() {
    let cases = [
        (0x18, 16, 130),
        (0x19, 12, 122),
        (0x1A, 9, 116),
        (0x1B, 3, 44),
        (0x1C, 3, 104),
    ];
    assert_eq!(cycle_failures(2, &cases), vec![]);
}

#[test]
fn given_a_load_missing_its_last_parameter_when_gxstat_is_read_then_the_entries_wait_in_the_queue()
{
    let mut gpu = Gpu3d::new();
    raw_port(&mut gpu, 0x16, 15);
    // Four entries in the PIPE, eleven in the FIFO: less than half full, busy, nothing runs.
    assert_eq!((gxstat(&gpu), gpu.slot_due(0)), (0x0A0B_0000, None));
}

#[test]
fn given_a_load_missing_its_last_parameter_when_it_arrives_then_the_queue_empties_and_it_runs() {
    let mut gpu = Gpu3d::new();
    raw_port(&mut gpu, 0x16, 16);
    assert_eq!((gxstat(&gpu), gpu.slot_due(0)), (RESET | BUSY, Some(68)));
}

#[test]
fn given_a_packed_word_of_pushes_when_written_then_the_engine_runs_them_one_after_another() {
    let mut gpu = Gpu3d::new();
    raw(&mut gpu, pack([MTX_PUSH; 4]));
    gpu.slot_fire(0, 34);
    assert_eq!(gpu.slot_due(0), Some(68));
}

#[test]
fn given_a_command_when_the_event_is_late_then_the_chain_of_commands_ending_by_then_runs() {
    let mut gpu = Gpu3d::new();
    raw(&mut gpu, pack([MTX_PUSH; 4]));
    gpu.slot_fire(0, 100);
    // Pushes end at 34, 68 and 102: two have taken effect by 100, the third is running.
    assert_eq!((gpu.slot_due(0), gpu.projection_pointer()), (Some(102), 0));
}

#[test]
fn given_a_write_when_it_returns_then_it_replaces_the_engine_event_and_asks_for_the_irq_recheck() {
    let mut gpu = Gpu3d::new();
    let effect = raw(&mut gpu, MTX_PUSH.into());
    assert_eq!(
        (
            effect.reschedule,
            effect.recheck_held_requests,
            effect.stall
        ),
        (1, true, 0)
    );
}

#[test]
fn given_the_engine_event_when_it_fires_then_the_effect_signals_the_dma_and_rechecks_the_irq() {
    let mut gpu = Gpu3d::new();
    raw(&mut gpu, MTX_PUSH.into());
    let fired = gpu.slot_fire(0, 34);
    assert_eq!(
        fired.effect,
        Effect {
            reschedule: 1,
            signals: Signals::of(Signal::GeometryFifoHalf),
            recheck_held_requests: true,
            ..Effect::default()
        }
    );
}

// ------------------------------------------------------------------------------- SWAP_BUFFERS

#[test]
fn given_swap_buffers_written_in_a_visible_line_when_it_starts_then_it_ends_392_bus_cycles_after_vblank()
 {
    let mut gpu = Gpu3d::new();
    raw_at(&mut gpu, 100, GXFIFO, SWAP_BUFFERS.into());
    raw_at(&mut gpu, 100, GXFIFO, 0);
    assert_eq!(gpu.slot_due(0), Some(SWAP_ENDS));
}

#[test]
fn given_swap_buffers_written_exactly_at_vblank_when_it_starts_then_it_does_not_wait() {
    let mut gpu = Gpu3d::new();
    raw_at(&mut gpu, VBLANK, GXFIFO, SWAP_BUFFERS.into());
    raw_at(&mut gpu, VBLANK, GXFIFO, 0);
    assert_eq!(gpu.slot_due(0), Some(SWAP_ENDS));
}

#[test]
fn given_swap_buffers_written_inside_vblank_when_it_starts_then_it_waits_for_the_next_frame() {
    let mut gpu = Gpu3d::new();
    raw_at(&mut gpu, VBLANK + 1, GXFIFO, SWAP_BUFFERS.into());
    raw_at(&mut gpu, VBLANK + 1, GXFIFO, 0);
    assert_eq!(gpu.slot_due(0), Some(263 * 4_260 + SWAP_ENDS));
}

#[test]
fn given_swap_buffers_in_flight_when_the_cpu_goes_on_then_only_the_engine_is_busy() {
    let gpu = waiting_with(0);
    assert_eq!(gxstat(&gpu), RESET | BUSY);
}

#[test]
fn given_any_time_when_the_next_vblank_is_asked_then_it_is_the_first_line_192_start_at_or_after() {
    let frame = 263 * 4_260;
    let asked = [0, VBLANK - 1, VBLANK, VBLANK + 1, frame, frame + VBLANK];
    let got: Vec<u64> = asked.iter().map(|&t| next_vblank_start(t)).collect();
    assert_eq!(
        got,
        vec![
            VBLANK,
            VBLANK,
            VBLANK,
            frame + VBLANK,
            VBLANK + frame,
            VBLANK + frame
        ]
    );
}

// --------------------------------------------------------------------------- the full FIFO stall

#[test]
fn given_a_waiting_engine_when_260_entries_are_queued_then_the_fifo_is_full() {
    let gpu = waiting_with(260);
    // The PIPE holds four and the FIFO 256 (count low byte 0, bit 24): busy, not half full.
    assert_eq!(gxstat(&gpu), 0x0900_0000);
}

#[test]
fn given_a_full_fifo_behind_swap_buffers_when_one_more_entry_is_written_then_the_write_stalls_until_two_pushes_ran()
 {
    let mut gpu = waiting_with(260);
    let effect = raw(&mut gpu, MTX_PUSH.into());
    // SWAP_BUFFERS ends at 818,704. The next push takes the PIPE's head, which leaves the FIFO full
    // (the PIPE is still at three), so the write waits for that push too: 34 more.
    assert_eq!(effect.stall, SWAP_ENDS + 34);
}

#[test]
fn given_a_full_fifo_when_a_write_has_waited_then_its_entry_is_queued_and_the_fifo_is_full_again() {
    let mut gpu = waiting_with(260);
    raw(&mut gpu, MTX_PUSH.into());
    assert_eq!(gpu.fifo_len(), 255);
}

#[test]
fn given_a_fifo_with_room_when_an_entry_is_written_then_nothing_stalls() {
    let mut gpu = waiting_with(259);
    let effect = raw(&mut gpu, MTX_PUSH.into());
    assert_eq!(effect.stall, 0);
}

// ---------------------------------------------------------------------------- the FIFO's levels

#[test]
fn given_127_and_128_entries_in_the_fifo_when_the_dma_level_is_asked_then_it_holds_only_below_half()
{
    let below = waiting_with(4 + 127).held_signals();
    let at_half = waiting_with(4 + 128).held_signals();
    assert_eq!(
        (
            below.contains(Signal::GeometryFifoHalf),
            at_half.contains(Signal::GeometryFifoHalf)
        ),
        (true, false)
    );
}

/// The FIFO IRQ the device holds with `mode` set and `entries` pushes queued behind SWAP_BUFFERS.
fn held_irq(mode: u32, entries: u32) -> u32 {
    let mut gpu = waiting_with(entries);
    raw_at(&mut gpu, 0, GXSTAT, mode << 30);
    gpu.held_requests().arm9
}

#[test]
fn given_irq_mode_1_when_the_fifo_has_127_and_128_entries_then_it_is_held_only_below_half() {
    assert_eq!(
        (held_irq(1, 4 + 127), held_irq(1, 4 + 128)),
        (GXFIFO_IRQ, 0)
    );
}

#[test]
fn given_irq_mode_2_when_the_fifo_is_empty_and_when_it_holds_one_entry_then_it_is_held_only_empty()
{
    assert_eq!((held_irq(2, 4), held_irq(2, 5)), (GXFIFO_IRQ, 0));
}

#[test]
fn given_irq_mode_2_when_the_pipe_holds_entries_but_the_fifo_is_empty_then_the_fifo_counts_as_empty()
 {
    assert_eq!(held_irq(2, 3), GXFIFO_IRQ);
}

// ------------------------------------------------------------------------------------ savestate

#[test]
fn given_a_command_in_flight_when_saved_and_loaded_then_it_ends_at_the_same_time() {
    let mut original = waiting_with(10);
    let bytes = postcard::to_allocvec(&original).unwrap();
    let mut restored: Gpu3d = postcard::from_bytes(&bytes).unwrap();
    original.slot_fire(0, SWAP_ENDS);
    restored.slot_fire(0, SWAP_ENDS);
    assert_eq!(
        (
            restored == original,
            restored.slot_due(0),
            restored.fifo_len()
        ),
        (true, Some(SWAP_ENDS + 34), 6)
    );
}

#[test]
fn given_a_machine_state_with_commands_queued_when_checked_then_it_is_well_formed() {
    assert!(waiting_with(200).well_formed(0));
}

#[test]
fn given_queued_entries_when_inspected_then_the_debugger_sees_the_pipe_first() {
    let mut gpu = Gpu3d::new();
    raw_port(&mut gpu, 0x16, 3);
    assert_eq!(
        (
            gpu.queued(0),
            gpu.queued(2),
            gpu.queued(3),
            gpu.pipe_len(),
            gpu.running()
        ),
        (Some((0x16, 0)), Some((0x16, 0)), None, 3, None)
    );
}
