// SPDX-License-Identifier: Apache-2.0
//! The 3D engine stub as a device. Expected values come from `docs/hw/gx-stub.md` ("The command
//! set", "GXFIFO: packed and unpacked words", "Matrix mode and the stack pointers", "GXSTAT bits",
//! "GXSTAT and the FIFO IRQ"), worked out by hand, never from running the code.
//!
//! Offsets are the device's own: `0400_0060h` is offset 0, so the geometry block (`0400_0400h`) is
//! `0x3A0`, a port is `0x3A0 + 4 * id` and GXSTAT is `0x5A0`.

use super::*;
use crate::io::device::cx_at;

const GXFIFO: u32 = GEOMETRY_OFFSET;
const GXSTAT: u32 = GEOMETRY_OFFSET + 0x200;
const fn port(id: u8) -> u32 {
    GEOMETRY_OFFSET + 4 * id as u32
}

const MTX_MODE: u8 = 0x10;
const MTX_PUSH: u8 = 0x11;
const MTX_POP: u8 = 0x12;
const MTX_STORE: u8 = 0x13;
const MTX_RESTORE: u8 = 0x14;
const COLOR: u8 = 0x20;
const SHININESS: u8 = 0x34;
const BOX_TEST: u8 = 0x70;
const POS_TEST: u8 = 0x71;
const VEC_TEST: u8 = 0x72;
const SWAP_BUFFERS: u8 = 0x50;
const INVALID: u8 = 0x05;

/// GXSTAT bits 8-13: both stack levels.
const LEVEL_BITS: u32 = 0x3F << 8;
const ERROR_BIT: u32 = 1 << 15;
const BOX_BIT: u32 = 1 << 1;
/// The reset value: FIFO less than half full and empty.
const RESET: u32 = 0x0600_0000;

/// Four command IDs in one word, byte 0 first.
const fn pack(ids: [u8; 4]) -> u32 {
    u32::from_le_bytes(ids)
}

/// Runs the engine until it is idle, firing its event slot at each due time as the scheduler would.
/// Most tests are about what a command does, not when, so `write` drains after each write;
/// the timing tests below call `gpu.write` themselves and step the engine by hand.
fn drain(gpu: &mut Gpu3d) {
    while let Some(ends) = gpu.slot_due(0) {
        gpu.slot_fire(0, ends);
    }
}

fn write(gpu: &mut Gpu3d, offset: u32, width: Width, value: u32) {
    gpu.write(offset, width, value, cx_at(0));
    drain(gpu);
}

fn read(gpu: &Gpu3d, offset: u32, width: Width) -> u32 {
    gpu.peek(offset, width, cx_at(0))
}

/// Words written to GXFIFO one after another.
fn send(gpu: &mut Gpu3d, words: &[u32]) {
    words
        .iter()
        .for_each(|&w| write(gpu, GXFIFO, Width::Word, w));
}

/// `count` writes of `value` to the port of command `id`.
fn port_writes(gpu: &mut Gpu3d, id: u8, count: u32, value: u32) {
    (0..count).for_each(|_| write(gpu, port(id), Width::Word, value));
}

fn gxstat(gpu: &Gpu3d) -> u32 {
    read(gpu, GXSTAT, Width::Word)
}

/// The two stack levels of GXSTAT (bits 8-13).
fn levels(gpu: &Gpu3d) -> u32 {
    (gxstat(gpu) & LEVEL_BITS) >> 8
}

fn error(gpu: &Gpu3d) -> bool {
    gxstat(gpu) & ERROR_BIT != 0
}

/// A device set to `mode` with MTX_MODE sent through GXFIFO as an unpacked command.
fn in_mode(mode: u32) -> Gpu3d {
    let mut gpu = Gpu3d::new();
    send(&mut gpu, &[MTX_MODE.into(), mode]);
    gpu
}

/// `count` MTX_PUSH commands, each as its own unpacked GXFIFO word.
fn push(gpu: &mut Gpu3d, count: u32) {
    (0..count).for_each(|_| send(gpu, &[MTX_PUSH.into()]));
}

/// MTX_POP with `offset` as its parameter.
fn pop(gpu: &mut Gpu3d, offset: u32) {
    send(gpu, &[MTX_POP.into(), offset]);
}

/// Whether the pointer of each mode's stack moves on a push, as `(mode, levels, error)` cases
/// that came out different from the note's.
fn push_in_each_mode() -> Vec<(u32, u32, bool)> {
    // GXSTAT bit 13 is the projection level, bits 8-12 the coordinate level: bit 13 = 0x20.
    let cases = [
        (0, 0x20, false),
        (1, 1, false),
        (2, 1, false),
        (3, 0, false),
    ];
    cases
        .into_iter()
        .filter_map(|(mode, want_levels, want_error)| {
            let mut gpu = in_mode(mode);
            push(&mut gpu, 1);
            let got = (mode, levels(&gpu), error(&gpu));
            (got != (mode, want_levels, want_error)).then_some(got)
        })
        .collect()
}

#[test]
fn given_each_matrix_mode_when_one_push_runs_then_only_the_selected_stack_pointer_moves() {
    assert_eq!(push_in_each_mode(), vec![]);
}

#[test]
fn given_a_new_device_when_gxstat_is_read_then_the_fifo_reads_empty_and_nothing_else_is_set() {
    assert_eq!(gxstat(&Gpu3d::new()), RESET);
}

#[test]
fn given_a_packed_word_of_four_pushes_when_written_then_the_coordinate_level_is_four() {
    let mut gpu = in_mode(1);
    send(&mut gpu, &[pack([MTX_PUSH; 4])]);
    assert_eq!(levels(&gpu), 4);
}

#[test]
fn given_a_packed_word_with_bytes_two_to_four_zero_when_written_then_one_command_runs() {
    let mut gpu = in_mode(1);
    send(&mut gpu, &[pack([MTX_PUSH, 0, 0, 0])]);
    assert_eq!(levels(&gpu), 1);
}

#[test]
fn given_a_packed_word_of_four_nops_when_written_then_gxstat_is_unchanged() {
    let mut gpu = Gpu3d::new();
    send(&mut gpu, &[0]);
    assert_eq!(gxstat(&gpu), RESET);
}

#[test]
fn given_an_invalid_id_in_a_packed_word_when_the_next_word_comes_then_it_is_a_command_word() {
    let mut gpu = in_mode(1);
    send(
        &mut gpu,
        &[pack([INVALID, MTX_PUSH, 0, 0]), pack([MTX_PUSH, 0, 0, 0])],
    );
    assert_eq!(levels(&gpu), 2);
}

#[test]
fn given_a_packed_word_with_a_parameter_command_first_when_its_parameter_follows_then_the_rest_runs()
 {
    let mut gpu = Gpu3d::new();
    send(&mut gpu, &[pack([MTX_MODE, MTX_PUSH, MTX_PUSH, 0]), 1]);
    assert_eq!(levels(&gpu), 2);
}

#[test]
fn given_a_packed_word_with_a_parameter_command_first_when_no_parameter_follows_then_nothing_after_it_runs()
 {
    let mut gpu = in_mode(1);
    send(&mut gpu, &[pack([MTX_MODE, MTX_PUSH, 0, 0])]);
    assert_eq!(levels(&gpu), 0);
}

#[test]
fn given_a_parameter_that_looks_like_a_push_when_it_is_taken_then_it_is_not_run_as_a_command() {
    let mut gpu = Gpu3d::new();
    send(&mut gpu, &[MTX_MODE.into(), MTX_PUSH.into()]);
    assert_eq!((gpu.matrix_mode(), levels(&gpu)), (1, 0));
}

#[test]
fn given_an_unpacked_command_when_sent_through_gxfifo_then_it_takes_effect() {
    let mut gpu = Gpu3d::new();
    send(&mut gpu, &[MTX_MODE.into(), 2]);
    assert_eq!(gpu.matrix_mode(), 2);
}

#[test]
fn given_a_mirror_of_gxfifo_when_a_command_is_written_there_then_it_runs() {
    let mut gpu = in_mode(1);
    write(&mut gpu, GXFIFO + 0x3C, Width::Word, MTX_PUSH.into());
    assert_eq!(levels(&gpu), 1);
}

#[test]
fn given_an_stm_style_run_of_mirrors_when_one_unpacked_command_is_sent_then_its_parameter_follows()
{
    let mut gpu = Gpu3d::new();
    write(&mut gpu, GXFIFO, Width::Word, MTX_MODE.into());
    write(&mut gpu, GXFIFO + 4, Width::Word, 3);
    assert_eq!(gpu.matrix_mode(), 3);
}

#[test]
fn given_a_command_with_a_parameter_when_sent_to_its_port_then_one_write_runs_it() {
    let mut gpu = Gpu3d::new();
    write(&mut gpu, port(MTX_MODE), Width::Word, 1);
    assert_eq!(gpu.matrix_mode(), 1);
}

#[test]
fn given_a_command_without_parameters_when_its_port_gets_a_dummy_write_then_it_runs() {
    let mut gpu = in_mode(1);
    write(&mut gpu, port(MTX_PUSH), Width::Word, 0xDEAD_BEEF);
    assert_eq!(levels(&gpu), 1);
}

#[test]
fn given_a_command_with_three_parameters_when_the_port_is_written_twice_then_the_third_write_is_still_owed()
 {
    let mut gpu = in_mode(1);
    port_writes(&mut gpu, 0x1B, 2, 0);
    write(&mut gpu, GXFIFO, Width::Word, MTX_PUSH.into());
    // The word was the third parameter of MTX_SCALE, not a command.
    assert_eq!(levels(&gpu), 0);
}

#[test]
fn given_shininess_when_its_32_parameters_arrive_then_none_of_them_runs_as_a_command() {
    let mut gpu = in_mode(1);
    send(&mut gpu, &[SHININESS.into()]);
    send(&mut gpu, &[u32::from(MTX_PUSH); 32]);
    assert_eq!(levels(&gpu), 0);
}

#[test]
fn given_shininess_when_a_33rd_word_comes_then_it_is_a_command_word() {
    let mut gpu = in_mode(1);
    send(&mut gpu, &[SHININESS.into()]);
    send(&mut gpu, &[u32::from(MTX_PUSH); 32]);
    send(&mut gpu, &[MTX_PUSH.into()]);
    assert_eq!(levels(&gpu), 1);
}

#[test]
fn given_shininess_when_32_words_go_to_its_port_then_it_completes() {
    let mut gpu = in_mode(1);
    port_writes(&mut gpu, SHININESS, 32, u32::from(MTX_PUSH));
    send(&mut gpu, &[MTX_PUSH.into()]);
    assert_eq!(levels(&gpu), 1);
}

#[test]
fn given_a_port_no_command_uses_when_written_then_the_decoder_is_unchanged() {
    let mut gpu = in_mode(1);
    write(
        &mut gpu,
        GEOMETRY_OFFSET + 0x74,
        Width::Word,
        MTX_PUSH.into(),
    );
    send(&mut gpu, &[MTX_PUSH.into()]);
    assert_eq!(levels(&gpu), 1);
}

#[test]
fn given_a_command_missing_a_parameter_when_another_port_is_written_then_the_old_command_is_dropped()
 {
    let mut gpu = Gpu3d::new();
    send(&mut gpu, &[MTX_MODE.into()]);
    write(&mut gpu, port(MTX_PUSH), Width::Word, 0);
    // The projection pointer is 1 after the push; the word below is a command word, so the second
    // push runs and finds entry 1.
    send(&mut gpu, &[MTX_PUSH.into()]);
    assert!(error(&gpu));
}

#[test]
fn given_a_halfword_write_to_gxfifo_when_a_command_is_written_then_it_is_one_entry() {
    let mut gpu = in_mode(1);
    write(&mut gpu, GXFIFO, Width::Half, MTX_PUSH.into());
    assert_eq!(levels(&gpu), 1);
}

#[test]
fn given_the_test_commands_when_their_parameters_arrive_then_each_consumes_exactly_its_count() {
    let mut gpu = in_mode(1);
    send(&mut gpu, &[BOX_TEST.into(), 0, 0, 0]);
    send(&mut gpu, &[POS_TEST.into(), 0, 0]);
    send(&mut gpu, &[VEC_TEST.into(), 0]);
    send(&mut gpu, &[MTX_PUSH.into()]);
    assert_eq!(levels(&gpu), 1);
}

#[test]
fn given_a_draw_command_when_its_parameters_arrive_then_it_is_consumed_and_has_no_effect() {
    let mut gpu = in_mode(1);
    send(&mut gpu, &[COLOR.into(), 0x7FFF, SWAP_BUFFERS.into(), 0]);
    send(&mut gpu, &[MTX_PUSH.into()]);
    assert_eq!(levels(&gpu), 1);
}

#[test]
fn given_no_box_test_when_gxstat_is_read_then_the_result_bit_is_clear() {
    assert_eq!(gxstat(&Gpu3d::new()) & BOX_BIT, 0);
}

#[test]
fn given_a_box_test_when_it_runs_then_the_result_bit_reads_inside() {
    let mut gpu = Gpu3d::new();
    send(&mut gpu, &[BOX_TEST.into(), 0, 0, 0]);
    assert_eq!(gxstat(&gpu), RESET | BOX_BIT);
}

/// Cases where the level after the operation differs from the note: `(name, got, want)`.
fn pop_offsets() -> Vec<(&'static str, u32, u32)> {
    // Three pushes put the coordinate pointer at 3.
    let cases = [
        ("one", 1, 2),
        ("two", 2, 1),
        ("zero", 0, 3),
        ("minus one", 0x3F, 4),
        ("minus two", 0x3E, 5),
        // Only bits 0-5 count: 0xFFFF_FFC1 has the offset bits 000001, so +1.
        ("high bits ignored", 0xFFFF_FFC1, 2),
    ];
    cases
        .into_iter()
        .filter_map(|(name, offset, want)| {
            let mut gpu = in_mode(1);
            push(&mut gpu, 3);
            pop(&mut gpu, offset);
            let got = levels(&gpu);
            (got != want).then_some((name, got, want))
        })
        .collect()
}

#[test]
fn given_a_pointer_at_three_when_pop_runs_with_each_offset_then_the_pointer_moves_by_the_signed_offset()
 {
    assert_eq!(pop_offsets(), vec![]);
}

#[test]
fn given_mode_two_when_push_and_pop_run_then_they_act_as_in_mode_one() {
    let mut gpu = in_mode(2);
    push(&mut gpu, 2);
    pop(&mut gpu, 1);
    assert_eq!(levels(&gpu), 1);
}

#[test]
fn given_a_pointer_at_31_when_it_is_reached_by_pushes_then_no_error_is_set() {
    let mut gpu = in_mode(1);
    push(&mut gpu, 31);
    assert_eq!((levels(&gpu), error(&gpu)), (31, false));
}

#[test]
fn given_a_pointer_at_31_when_one_more_push_runs_then_the_error_is_set_and_the_level_wraps_to_zero()
{
    let mut gpu = in_mode(1);
    push(&mut gpu, 32);
    assert_eq!((levels(&gpu), error(&gpu)), (0, true));
    assert_eq!(gpu.coordinate_pointer(), 32);
}

#[test]
fn given_a_pointer_at_32_when_a_pop_returns_to_31_then_the_error_is_set() {
    let mut gpu = in_mode(1);
    push(&mut gpu, 31);
    pop(&mut gpu, 0);
    assert!(error(&gpu));
}

#[test]
fn given_a_pointer_at_zero_when_pop_runs_then_it_wraps_to_63_and_the_error_is_set() {
    let mut gpu = in_mode(1);
    pop(&mut gpu, 1);
    assert_eq!((gpu.coordinate_pointer(), error(&gpu)), (63, true));
}

/// `(store or restore, address)` cases whose error flag differs from the note.
fn store_addresses() -> Vec<(u8, u32, bool)> {
    // 30 is the last valid entry; 31 raises the flag; bits above 4 are ignored, so 0x20 is entry 0.
    let cases = [(30, false), (31, true), (0x20, false), (0x3F, true)];
    [MTX_STORE, MTX_RESTORE]
        .into_iter()
        .flat_map(|command| cases.map(|(address, want)| (command, address, want)))
        .filter_map(|(command, address, want)| {
            let mut gpu = in_mode(1);
            push(&mut gpu, 2);
            send(&mut gpu, &[command.into(), address]);
            let moved = gpu.coordinate_pointer() != 2;
            let got = error(&gpu);
            (got != want || moved).then_some((command, address, got))
        })
        .collect()
}

#[test]
fn given_store_and_restore_when_run_with_each_address_then_only_entry_31_sets_the_error_and_the_pointer_stays()
 {
    assert_eq!(store_addresses(), vec![]);
}

#[test]
fn given_projection_mode_when_push_runs_twice_then_the_second_sets_the_error() {
    let mut gpu = Gpu3d::new();
    push(&mut gpu, 1);
    assert!(!error(&gpu));
    push(&mut gpu, 1);
    assert!(error(&gpu));
    assert_eq!(gpu.projection_pointer(), 0);
}

#[test]
fn given_projection_mode_when_pop_runs_at_zero_then_the_error_is_set() {
    let mut gpu = Gpu3d::new();
    pop(&mut gpu, 1);
    assert_eq!((gpu.projection_pointer(), error(&gpu)), (1, true));
}

#[test]
fn given_projection_mode_when_store_and_pop_get_other_parameters_then_they_are_ignored() {
    let mut gpu = Gpu3d::new();
    send(&mut gpu, &[MTX_STORE.into(), 31]);
    push(&mut gpu, 1);
    pop(&mut gpu, 5);
    assert_eq!((gpu.projection_pointer(), error(&gpu)), (0, false));
}

#[test]
fn given_texture_mode_when_push_runs_twice_then_the_second_sets_the_error() {
    let mut gpu = in_mode(3);
    push(&mut gpu, 2);
    assert!(error(&gpu));
}

/// A device with the error set and the projection pointer at 1, and the coordinate pointer at 5.
fn errored() -> Gpu3d {
    let mut gpu = Gpu3d::new();
    // Three projection pushes: ok (pointer 1), error (pointer 0), ok (pointer 1).
    push(&mut gpu, 3);
    send(&mut gpu, &[MTX_MODE.into(), 1]);
    push(&mut gpu, 5);
    gpu
}

#[test]
fn given_an_error_and_a_projection_pointer_when_bit_15_is_written_with_one_then_both_clear() {
    let mut gpu = errored();
    write(&mut gpu, GXSTAT, Width::Word, ERROR_BIT);
    assert_eq!((gpu.projection_pointer(), error(&gpu)), (0, false));
}

#[test]
fn given_an_error_when_bit_15_is_written_with_one_then_the_coordinate_pointer_is_unchanged() {
    let mut gpu = errored();
    write(&mut gpu, GXSTAT, Width::Word, ERROR_BIT);
    assert_eq!(gpu.coordinate_pointer(), 5);
}

#[test]
fn given_a_texture_pointer_when_bit_15_is_written_with_one_then_it_resets() {
    let mut gpu = in_mode(3);
    push(&mut gpu, 1);
    write(&mut gpu, GXSTAT, Width::Word, ERROR_BIT);
    assert_eq!(gpu.texture_pointer(), 0);
}

#[test]
fn given_an_error_when_bit_15_is_written_with_zero_then_nothing_changes() {
    let mut gpu = errored();
    let before = gpu.clone();
    write(&mut gpu, GXSTAT, Width::Word, !ERROR_BIT & 0x3FFF_FFFF);
    assert_eq!(gpu, before);
}

#[test]
fn given_gxstat_when_every_read_only_bit_is_written_then_nothing_changes() {
    let mut gpu = errored();
    let before = gpu.clone();
    // Everything except bit 15 and bits 30-31.
    write(&mut gpu, GXSTAT, Width::Word, 0x3FFF_7FFF);
    assert_eq!(gpu, before);
}

/// `(mode, readback)` cases whose GXSTAT bits 30-31 read back differently from what was written.
fn irq_mode_readback() -> Vec<(u32, u32)> {
    (0..4)
        .filter_map(|mode| {
            let mut gpu = Gpu3d::new();
            write(&mut gpu, GXSTAT, Width::Word, mode << 30);
            let got = gxstat(&gpu) >> 30;
            (got != mode).then_some((mode, got))
        })
        .collect()
}

#[test]
fn given_each_fifo_irq_mode_when_written_to_gxstat_then_it_reads_back() {
    assert_eq!(irq_mode_readback(), vec![]);
}

#[test]
fn given_a_byte_write_to_the_top_byte_of_gxstat_when_mode_two_is_written_then_the_word_reads_it() {
    let mut gpu = Gpu3d::new();
    write(&mut gpu, GXSTAT + 3, Width::Byte, 0x80);
    assert_eq!(gxstat(&gpu), 0x8000_0000 | RESET);
}

#[test]
fn given_a_byte_write_to_the_second_byte_of_gxstat_when_bit_15_is_one_then_the_error_clears() {
    let mut gpu = errored();
    write(&mut gpu, GXSTAT + 1, Width::Byte, 0x80);
    assert!(!error(&gpu));
}

/// `(mode, requests)` cases whose IRQ query differs from the note: only modes 1 and 2 raise bit 21
/// for the ARM9.
fn irq_queries() -> Vec<(u32, Requests)> {
    let raised = Requests::for_cpu::<true>(1 << 21);
    [
        (0, Requests::default()),
        (1, raised),
        (2, raised),
        (3, Requests::default()),
    ]
    .into_iter()
    .filter_map(|(mode, want)| {
        let mut gpu = Gpu3d::new();
        write(&mut gpu, GXSTAT, Width::Word, mode << 30);
        let got = Device::<true>::held_requests(&gpu);
        (got != want).then_some((mode, got))
    })
    .collect()
}

#[test]
fn given_each_fifo_irq_mode_when_the_condition_is_queried_then_only_modes_one_and_two_hold_bit_21()
{
    assert_eq!(irq_queries(), vec![]);
}

/// The effect of a write at `offset`.
fn write_effect(gpu: &mut Gpu3d, offset: u32, value: u32) -> Effect {
    gpu.write(offset, Width::Word, value, cx_at(0))
}

#[test]
fn given_a_gxstat_write_when_it_returns_then_it_asks_for_the_held_requests_to_be_rechecked() {
    let mut gpu = Gpu3d::new();
    assert!(write_effect(&mut gpu, GXSTAT, 1 << 30).recheck_held_requests);
}

#[test]
fn given_a_gxfifo_write_when_it_returns_then_it_asks_for_the_held_requests_to_be_rechecked() {
    let mut gpu = Gpu3d::new();
    assert!(write_effect(&mut gpu, GXFIFO, 0).recheck_held_requests);
}

#[test]
fn given_the_geometry_block_unpowered_when_gxstat_is_written_then_it_still_asks_for_a_recheck() {
    let mut gpu = Gpu3d::new();
    let mut cx = cx_at(0);
    cx.lines.geometry_powered = false;
    let effect = gpu.write(GXSTAT, Width::Word, 1 << 30, cx);
    assert_eq!(
        (
            effect.recheck_held_requests,
            Device::<true>::held_requests(&gpu)
        ),
        (true, Requests::default())
    );
}

#[test]
fn given_a_new_device_when_the_irq_condition_is_queried_then_nothing_is_raised() {
    assert_eq!(
        Device::<true>::held_requests(&Gpu3d::new()),
        Requests::default()
    );
}

#[test]
fn given_disp3dcnt_when_all_ones_are_written_then_only_bits_0_to_11_and_14_read_back() {
    let mut gpu = Gpu3d::new();
    write(&mut gpu, 0, Width::Half, 0xFFFF);
    assert_eq!(read(&gpu, 0, Width::Word), 0x4FFF);
}

#[test]
fn given_disp3dcnt_when_the_acknowledge_bits_are_written_then_they_stay_clear() {
    let mut gpu = Gpu3d::new();
    write(&mut gpu, 0, Width::Half, 0x3000);
    assert_eq!(read(&gpu, 0, Width::Half), 0);
}

#[test]
fn given_disp3dcnt_when_the_two_bytes_are_written_separately_then_each_changes_only_itself() {
    let mut gpu = Gpu3d::new();
    write(&mut gpu, 0, Width::Byte, 0xAB);
    write(&mut gpu, 1, Width::Byte, 0x45);
    assert_eq!(read(&gpu, 0, Width::Half), 0x45AB);
}

#[test]
fn given_a_word_write_to_disp3dcnt_when_the_upper_half_is_set_then_it_is_not_stored() {
    let mut gpu = Gpu3d::new();
    write(&mut gpu, 0, Width::Word, 0xFFFF_0001);
    assert_eq!(read(&gpu, 0, Width::Word), 1);
}

/// `(address low bits, value read)` of the registers that must read 0 whatever was written to them.
fn zero_registers() -> Vec<(u32, u32)> {
    let result_offsets = [
        0x604, 0x620, 0x62C, 0x630, 0x634, 0x610, 0x330, 0x3B8, 0x350,
    ];
    result_offsets
        .into_iter()
        .filter_map(|addr| {
            let offset = addr - 0x60;
            let mut gpu = Gpu3d::new();
            write(&mut gpu, offset, Width::Word, 0xFFFF_FFFF);
            let got = read(&gpu, offset, Width::Word);
            (got != 0).then_some((addr, got))
        })
        .collect()
}

#[test]
fn given_the_result_and_write_only_registers_when_written_and_read_then_they_read_zero() {
    assert_eq!(zero_registers(), vec![]);
}

#[test]
fn given_the_rendering_block_when_rdlines_count_is_read_then_it_reads_46() {
    assert_eq!(read(&Gpu3d::new(), RENDER_OFFSET, Width::Byte), 46);
}

#[test]
fn given_gxfifo_when_read_then_it_reads_zero() {
    let mut gpu = Gpu3d::new();
    write(&mut gpu, GXFIFO, Width::Word, 0xFFFF_FFFF);
    assert_eq!(read(&gpu, GXFIFO, Width::Word), 0);
}

#[test]
fn given_a_state_in_the_middle_of_a_packed_word_when_saved_and_loaded_then_it_continues_identically()
 {
    let mut original = Gpu3d::new();
    send(&mut original, &[pack([MTX_MODE, MTX_PUSH, MTX_PUSH, 0])]);
    let bytes = postcard::to_allocvec(&original).unwrap();
    let mut restored: Gpu3d = postcard::from_bytes(&bytes).unwrap();
    send(&mut original, &[1]);
    send(&mut restored, &[1]);
    assert_eq!((restored == original, levels(&restored)), (true, 2));
}

#[test]
fn given_a_new_device_when_checked_then_it_is_well_formed() {
    assert!(Gpu3d::new().well_formed(0));
}

/// Mutations of a new device that no program can reach, each of which `well_formed` must refuse.
fn malformed_states() -> Vec<usize> {
    let mutations: [fn(&mut Gpu3d); 7] = [
        |g| g.disp3dcnt = 0x8000,
        |g| g.mtx_mode = 4,
        |g| g.projection_sp = 2,
        |g| g.texture_sp = 2,
        |g| g.coordinate_sp = 64,
        |g| g.irq_mode = 4,
        |g| {
            g.command = INVALID;
            g.owed = 1;
        },
    ];
    mutations
        .into_iter()
        .enumerate()
        .filter_map(|(index, mutate)| {
            let mut gpu = Gpu3d::new();
            mutate(&mut gpu);
            gpu.well_formed(0).then_some(index)
        })
        .collect()
}

#[test]
fn given_each_unreachable_state_when_checked_then_it_is_refused() {
    assert_eq!(malformed_states(), Vec::<usize>::new());
}

#[test]
fn given_a_command_owing_more_than_it_takes_when_checked_then_it_is_refused() {
    let mut gpu = Gpu3d::new();
    gpu.command = MTX_PUSH;
    gpu.owed = 1;
    assert!(!gpu.well_formed(0));
}

#[test]
fn given_a_command_owing_its_parameters_when_checked_then_it_is_well_formed() {
    let mut gpu = Gpu3d::new();
    send(&mut gpu, &[SHININESS.into()]);
    assert!(gpu.well_formed(0));
}

/// The ID of each command in the note's table with its parameter count, and every other byte.
fn parameter_count_failures() -> Vec<(u8, Option<u8>)> {
    let table: [(u8, u8); 38] = [
        (0x10, 1),
        (0x11, 0),
        (0x12, 1),
        (0x13, 1),
        (0x14, 1),
        (0x15, 0),
        (0x16, 16),
        (0x17, 12),
        (0x18, 16),
        (0x19, 12),
        (0x1A, 9),
        (0x1B, 3),
        (0x1C, 3),
        (0x20, 1),
        (0x21, 1),
        (0x22, 1),
        (0x23, 2),
        (0x24, 1),
        (0x25, 1),
        (0x26, 1),
        (0x27, 1),
        (0x28, 1),
        (0x29, 1),
        (0x2A, 1),
        (0x2B, 1),
        (0x30, 1),
        (0x31, 1),
        (0x32, 1),
        (0x33, 1),
        (0x34, 32),
        (0x40, 1),
        (0x41, 0),
        (0x50, 1),
        (0x60, 1),
        (0x70, 3),
        (0x71, 2),
        (0x72, 1),
        (0x00, 0),
    ];
    (0..=255u8)
        .filter_map(|id| {
            let want = table
                .iter()
                .find(|&&(known, _)| known == id && id != 0)
                .map(|&(_, count)| count);
            let got = parameter_count(id);
            (got != want).then_some((id, got))
        })
        .collect()
}

#[test]
fn given_every_byte_when_its_parameter_count_is_looked_up_then_it_matches_the_command_table() {
    assert_eq!(parameter_count_failures(), vec![]);
}

#[test]
fn given_a_gxstat_write_when_it_returns_then_it_does_not_ask_for_the_signals() {
    let mut gpu = Gpu3d::new();
    assert!(!write_effect(&mut gpu, GXSTAT, 1 << 30).recheck_held_signals);
}

mod engine;
mod lighting;
mod matrices;
mod matrix;
mod polygons;
mod queue;
