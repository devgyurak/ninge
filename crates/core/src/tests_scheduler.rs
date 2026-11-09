// SPDX-License-Identifier: Apache-2.0
//! The scheduler driving both CPUs through `Machine`. Expected values come from
//! `docs/hw/scheduler.md` ("Cost per instruction": ARM9 1 master cycle, ARM7 2; "Frame timing":
//! 1,120,380 master cycles per frame; "Interleaving"; "Debugger single-step"; "No program
//! loaded"; "ARM7 memory") and `docs/hw/direct-boot.md` ("ARM7 state at entry"),
//! never from running the code. Opcodes are encoded by hand from the ARM ARM, "A3 The ARM
//! Instruction Set" (data-processing and single-data-transfer formats, condition AL).

use super::*;
use crate::cart::test_image::{Image, words};
use crate::cpu::Bank;
use crate::scheduler::{EventKind, FRAME_CYCLES, HBLANK_START_CYCLES, LINE_CYCLES};

/// Where the ARM9 counting loop keeps its counter, and the ARM7's.
const ARM9_COUNTER: u32 = 0x0200_1000;
const ARM7_COUNTER: u32 = 0x0200_2000;
/// Where the ARM7 memory program records what it read (one word per slot).
const RECORD: u32 = 0x0200_3000;

/// `B .`.
const B_SELF: u32 = 0xEAFF_FFFE;
/// `mov r2, r2` (a no-op, to make the ARM7 loop 4 instructions long).
const MOV_R2_R2: u32 = 0xE1A0_2002;
/// `add r0, r0, #1`.
const ADD_R0_1: u32 = 0xE280_0001;
/// `str r0, [r1]`.
const STR_R0_R1: u32 = 0xE581_0000;

/// `mov rd, #imm8 ror (2 * rot)`.
const fn mov_imm(rd: u32, imm8: u32, rot: u32) -> u32 {
    0xE3A0_0000 | (rd << 12) | (rot << 8) | imm8
}
/// `orr rd, rd, #imm8 ror (2 * rot)`.
const fn orr_imm(rd: u32, imm8: u32, rot: u32) -> u32 {
    0xE380_0000 | (rd << 16) | (rd << 12) | (rot << 8) | imm8
}
/// `add rd, rn, #imm8 ror (2 * rot)`.
const fn add_imm(rd: u32, rn: u32, imm8: u32, rot: u32) -> u32 {
    0xE280_0000 | (rn << 16) | (rd << 12) | (rot << 8) | imm8
}
/// `ldr rd, [rn, #offset]` (offset up to 0xFFF, added).
const fn ldr(rd: u32, rn: u32, offset: u32) -> u32 {
    0xE590_0000 | (rn << 16) | (rd << 12) | offset
}
/// `str rd, [rn, #offset]` (offset up to 0xFFF, added).
const fn str(rd: u32, rn: u32, offset: u32) -> u32 {
    0xE580_0000 | (rn << 16) | (rd << 12) | offset
}

/// ARM9: `r1 = 0x0200_1000; r0 = 0; loop { r0 += 1; [r1] = r0 }`. Prologue 3, loop 3
/// (the branch back is word 5: R15 reads 28, the target is word 3 at 12, offset -4 words).
fn arm9_counter_code() -> Vec<u32> {
    vec![
        mov_imm(1, 0x02, 4),
        orr_imm(1, 0x01, 10),
        mov_imm(0, 0, 0),
        ADD_R0_1,
        STR_R0_R1,
        0xEAFF_FFFC,
    ]
}

/// ARM7: the same with `r1 = 0x0200_2000` and a `mov r2, r2` in the loop. Prologue 3, loop 4
/// (the branch is word 6: R15 reads 32, target 12, offset -5 words).
fn arm7_counter_code() -> Vec<u32> {
    vec![
        mov_imm(1, 0x02, 4),
        orr_imm(1, 0x02, 10),
        mov_imm(0, 0, 0),
        ADD_R0_1,
        STR_R0_R1,
        MOV_R2_R2,
        0xEAFF_FFFB,
    ]
}

/// A ROM whose two CPUs count forever.
fn counting_rom() -> Vec<u8> {
    let mut image = Image::valid();
    image.arm9_code = words(&arm9_counter_code());
    image.arm7_code = words(&arm7_counter_code());
    image.build()
}

/// A ROM whose ARM7 records what it reads from its own map and whose ARM9 spins.
///
/// Slots at `RECORD`: 0 read through the WRAM mirror at +0x10000, 1 read of the WRAM word,
/// 2 read of 0x0400_0000, 3 read through the main RAM mirror at +0x400000, 4 read of
/// 0x0600_0000, 5 progress. Slots 2 and 4 hold the read value OR 0x100, so that a read of 0
/// is told apart from a program that never ran.
fn arm7_memory_rom() -> Vec<u8> {
    let mut image = Image::valid();
    image.arm9_code = words(&[B_SELF]);
    image.arm7.size = 0x100;
    image.arm7_code = words(&[
        mov_imm(0, 0x02, 4),     // r0 = 0x0200_0000
        add_imm(0, 0, 0x03, 10), // r0 = 0x0200_3000, the record
        mov_imm(1, 0x38, 6),     // r1 = 0x0380_0000, ARM7 WRAM
        mov_imm(2, 0xA5, 0),     // r2 = 0xA5
        str(2, 1, 0),            // WRAM[0] = 0xA5
        add_imm(3, 1, 0x01, 8),  // r3 = 0x0381_0000, the mirror
        ldr(4, 3, 0),
        str(4, 0, 0), // slot 0
        ldr(4, 1, 0),
        str(4, 0, 4),        // slot 1
        mov_imm(5, 0x04, 4), // r5 = 0x0400_0000
        ldr(4, 5, 0),
        orr_imm(4, 0x01, 12),    // mark: bit 8 says the load ran
        str(4, 0, 8),            // slot 2
        str(2, 0, 0x100),        // main RAM word at RECORD + 0x100
        mov_imm(6, 0x24, 6),     // r6 = 0x0240_0000
        add_imm(6, 6, 0x03, 10), // + 0x3000
        add_imm(6, 6, 0x01, 12), // + 0x100
        ldr(4, 6, 0),
        str(4, 0, 0xC),      // slot 3
        mov_imm(5, 0x06, 4), // r5 = 0x0600_0000
        ldr(4, 5, 0),
        orr_imm(4, 0x01, 12), // mark
        str(4, 0, 0x10),      // slot 4
        mov_imm(6, 7, 0),
        str(6, 0, 0x14), // slot 5: progress
        B_SELF,
    ]);
    image.build()
}

fn booted(rom: &[u8]) -> Machine {
    let mut machine = Machine::new();
    machine.load_rom(rom.to_vec()).expect("synthetic image");
    machine
}

fn run_frames(machine: &mut Machine, frames: u32) {
    (0..frames).for_each(|_| machine.run_frame());
}

fn step(machine: &mut Machine, steps: u64) {
    (0..steps).for_each(|_| machine.step_arm9());
}

fn counters(machine: &Machine) -> (u32, u32) {
    (
        machine.arm9_peek32(ARM9_COUNTER),
        machine.arm9_peek32(ARM7_COUNTER),
    )
}

fn record(machine: &Machine, slot: u32) -> u32 {
    machine.arm9_peek32(RECORD + slot * 4)
}

// ------------------------------------------------------------- one frame

#[test]
fn given_one_frame_when_run_then_arm7_clock_is_exactly_half_of_arm9() {
    let mut machine = booted(&counting_rom());
    machine.run_frame();
    // ARM9 cycles = scheduler.now = 1,120,380; ARM7 cycles = arm7_clock / 2 = 560,190.
    assert_eq!(
        (machine.scheduler().now(), machine.arm7_clock() / 2),
        (1_120_380, 560_190)
    );
}

#[test]
fn given_one_frame_when_run_then_both_clocks_are_one_frame_in_master_cycles() {
    let mut machine = booted(&counting_rom());
    machine.run_frame();
    assert_eq!(
        (machine.scheduler().now(), machine.arm7_clock()),
        (FRAME_CYCLES, FRAME_CYCLES)
    );
}

#[test]
fn given_two_frames_when_run_then_both_clocks_are_two_frames_in_master_cycles() {
    let mut machine = booted(&counting_rom());
    run_frames(&mut machine, 2);
    assert_eq!(
        (machine.scheduler().now(), machine.arm7_clock()),
        (2 * FRAME_CYCLES, 2 * FRAME_CYCLES)
    );
}

#[test]
fn given_counting_rom_when_one_frame_runs_then_the_counters_show_373459_arm9_and_140047_arm7_iterations()
 {
    // ARM9: 1,120,380 instructions = 3 prologue + 3 * 373,459 loop instructions.
    // ARM7: 560,190 instructions = 3 prologue + 4 * 140,046 + 3; the partial iteration
    // reached its `str`, so the counter is 140,047.
    let mut machine = booted(&counting_rom());
    machine.run_frame();
    assert_eq!(counters(&machine), (373_459, 140_047));
}

#[test]
fn given_counting_rom_when_two_frames_run_then_the_counters_show_746919_arm9_and_280094_arm7_iterations()
 {
    // ARM9: 2,240,760 = 3 + 3 * 746,919. ARM7: 1,120,380 = 3 + 4 * 280,094 + 1; the partial
    // iteration stopped after its `add`, before the `str`, so memory holds 280,094.
    let mut machine = booted(&counting_rom());
    run_frames(&mut machine, 2);
    assert_eq!(counters(&machine), (746_919, 280_094));
}

#[test]
fn given_booted_machine_when_nothing_runs_then_both_clocks_are_zero_and_no_counter_moved() {
    let machine = booted(&counting_rom());
    assert_eq!(
        (
            machine.scheduler().now(),
            machine.arm7_clock(),
            counters(&machine)
        ),
        (0, 0, (0, 0))
    );
}

// ------------------------------------------------------------ no program

#[test]
fn given_machine_without_rom_when_running_a_frame_then_the_clock_stays_zero_and_the_frame_is_counted()
 {
    let mut machine = Machine::new();
    machine.run_frame();
    assert_eq!(
        (
            machine.scheduler().now(),
            machine.arm7_clock(),
            machine.frame_count()
        ),
        (0, 0, 1)
    );
}

#[test]
fn given_machine_without_rom_when_running_three_frames_then_the_clock_stays_zero_and_three_frames_are_counted()
 {
    let mut machine = Machine::new();
    run_frames(&mut machine, 3);
    assert_eq!((machine.scheduler().now(), machine.frame_count()), (0, 3));
}

// ----------------------------------------------------------- line events

#[test]
fn given_a_booted_machine_when_nothing_ran_then_hblank_and_the_next_line_start_are_pending_in_line_0()
 {
    let machine = booted(&counting_rom());
    let pending: Vec<(u64, EventKind)> = machine
        .scheduler()
        .pending()
        .iter()
        .map(|event| (event.time, event.kind))
        .collect();
    assert_eq!(
        pending,
        vec![
            (HBLANK_START_CYCLES, EventKind::HBlankStart),
            (LINE_CYCLES, EventKind::LineStart)
        ]
    );
}

#[test]
fn given_one_frame_run_when_listing_pending_then_the_next_line_events_follow_the_frame_end_time() {
    let mut machine = booted(&counting_rom());
    machine.run_frame();
    let pending: Vec<(u64, EventKind)> = machine
        .scheduler()
        .pending()
        .iter()
        .map(|event| (event.time, event.kind))
        .collect();
    assert_eq!(
        pending,
        vec![
            (FRAME_CYCLES + HBLANK_START_CYCLES, EventKind::HBlankStart),
            (FRAME_CYCLES + LINE_CYCLES, EventKind::LineStart)
        ]
    );
}

#[test]
fn given_two_frames_run_when_listing_pending_then_the_next_line_events_follow_two_frames() {
    let mut machine = booted(&counting_rom());
    run_frames(&mut machine, 2);
    let times: Vec<u64> = machine
        .scheduler()
        .pending()
        .iter()
        .map(|event| event.time)
        .collect();
    assert_eq!(
        times,
        vec![
            2 * FRAME_CYCLES + HBLANK_START_CYCLES,
            2 * FRAME_CYCLES + LINE_CYCLES
        ]
    );
}

// -------------------------------------------------------------- single step

#[test]
fn given_booted_machine_when_stepping_the_arm9_three_times_then_now_advances_by_exactly_three() {
    let mut machine = booted(&counting_rom());
    step(&mut machine, 3);
    assert_eq!(machine.scheduler().now(), 3);
}

#[test]
fn given_booted_machine_when_stepping_the_arm9_once_then_now_is_one() {
    let mut machine = booted(&counting_rom());
    step(&mut machine, 1);
    assert_eq!(machine.scheduler().now(), 1);
}

#[test]
fn given_booted_machine_when_stepping_the_arm9_once_then_the_arm7_ran_one_instruction_past_now() {
    // The ARM7 starts at 0 < 1, runs one instruction (2 master cycles) and ends 1 past now.
    let mut machine = booted(&counting_rom());
    step(&mut machine, 1);
    assert_eq!(machine.arm7_clock(), 2);
}

#[test]
fn given_booted_machine_when_stepping_the_arm9_four_times_then_the_arm7_clock_equals_now() {
    let mut machine = booted(&counting_rom());
    step(&mut machine, 4);
    assert_eq!((machine.scheduler().now(), machine.arm7_clock()), (4, 4));
}

#[test]
fn given_booted_machine_when_stepping_the_arm9_three_times_then_the_arm7_is_one_cycle_ahead() {
    // ARM7 instructions start at 0 and 2 (both < 3); the second ends at 4.
    let mut machine = booted(&counting_rom());
    step(&mut machine, 3);
    assert_eq!(machine.arm7_clock(), 4);
}

#[test]
fn given_booted_machine_when_stepping_the_arm9_six_times_then_the_arm7_has_run_three_instructions()
{
    // After 6 ARM9 cycles the ARM7 has run 3 instructions: mov, orr, mov. r1 = 0x0200_2000
    // and r0 = 0 are the visible proof (three prologue instructions).
    let mut machine = booted(&counting_rom());
    step(&mut machine, 6);
    let regs = machine.arm7().regs();
    assert_eq!((regs.get(1), regs.get(0)), (0x0200_2000, 0));
}

#[test]
fn given_step_one_short_of_a_frame_when_stepping_then_the_frame_end_has_not_fired() {
    let mut machine = booted(&counting_rom());
    step(&mut machine, FRAME_CYCLES - 1);
    assert_eq!(machine.frame_count(), 0);
}

#[test]
fn given_debug_steps_crossing_the_frame_end_time_when_stepping_then_the_frame_end_has_fired() {
    let mut machine = booted(&counting_rom());
    step(&mut machine, FRAME_CYCLES);
    assert_eq!(machine.frame_count(), 1);
}

#[test]
fn given_debug_steps_crossing_the_frame_end_time_when_listing_pending_then_the_next_line_events_follow_the_frame()
 {
    let mut machine = booted(&counting_rom());
    step(&mut machine, FRAME_CYCLES);
    let times: Vec<u64> = machine
        .scheduler()
        .pending()
        .iter()
        .map(|event| event.time)
        .collect();
    assert_eq!(
        times,
        vec![
            FRAME_CYCLES + HBLANK_START_CYCLES,
            FRAME_CYCLES + LINE_CYCLES
        ]
    );
}

#[test]
fn given_100_debug_steps_when_running_a_frame_then_it_ends_at_one_frame_of_master_cycles() {
    let mut machine = booted(&counting_rom());
    step(&mut machine, 100);
    machine.run_frame();
    assert_eq!(
        (
            machine.scheduler().now(),
            machine.arm7_clock(),
            machine.frame_count()
        ),
        (FRAME_CYCLES, FRAME_CYCLES, 1)
    );
}

#[test]
fn given_a_mid_frame_debug_step_state_when_two_frames_run_then_the_counters_match_a_plain_run() {
    // Whatever path got there, 2 frames of master cycles give the same instruction counts.
    let mut machine = booted(&counting_rom());
    step(&mut machine, 500_000);
    run_frames(&mut machine, 2);
    assert_eq!(counters(&machine), (746_919, 280_094));
}

// -------------------------------------------------------- ARM7 entry state

#[test]
fn given_valid_image_when_loading_then_arm7_r12_and_r14_are_the_entry() {
    let machine = booted(&Image::valid().build());
    let regs = machine.arm7().regs();
    assert_eq!((regs.get(12), regs.get(14)), (0x0238_0000, 0x0238_0000));
}

#[test]
fn given_valid_image_when_loading_then_arm7_r15_is_the_entry_plus_8() {
    let machine = booted(&Image::valid().build());
    assert_eq!(machine.arm7().regs().get(15), 0x0238_0008);
}

#[test]
fn given_valid_image_when_loading_then_arm7_stack_pointers_are_0380fd80_0380ff80_0380ffc0() {
    let machine = booted(&Image::valid().build());
    let regs = machine.arm7().regs();
    assert_eq!(
        (
            regs.get(13),
            regs.banked(Bank::Irq, 13),
            regs.banked(Bank::Supervisor, 13)
        ),
        (0x0380_FD80, 0x0380_FF80, 0x0380_FFC0)
    );
}

#[test]
fn given_valid_image_when_loading_then_arm7_cpsr_is_system_arm_with_i_and_f_set() {
    let machine = booted(&Image::valid().build());
    assert_eq!(machine.arm7().regs().cpsr(), 0x0000_00DF);
}

#[test]
fn given_valid_image_when_loading_then_arm7_r0_to_r11_are_zero() {
    let machine = booted(&Image::valid().build());
    let regs = machine.arm7().regs();
    let values: Vec<u32> = (0..12).map(|index| regs.get(index)).collect();
    // R12 is the entry, so it is not in the zero list; checking it ties this to a booted ARM7.
    assert_eq!((values, regs.get(12)), (vec![0; 12], 0x0238_0000));
}

#[test]
fn given_valid_image_when_loading_then_arm7_banked_lr_and_spsr_of_irq_and_svc_are_zero() {
    let machine = booted(&Image::valid().build());
    let regs = machine.arm7().regs();
    assert_eq!(
        (
            regs.banked(Bank::Irq, 14),
            regs.banked(Bank::Supervisor, 14),
            regs.spsr(Bank::Irq),
            regs.spsr(Bank::Supervisor),
            regs.banked(Bank::Irq, 13)
        ),
        (0, 0, Some(0), Some(0), 0x0380_FF80)
    );
}

#[test]
fn given_code_at_the_arm7_entry_when_loading_then_its_pipeline_holds_the_first_two_opcodes() {
    let mut image = Image::valid();
    image.arm7_code = words(&[0xE1A0_0000, 0xE1A0_1001]);
    let machine = booted(&image.build());
    assert_eq!(machine.arm7().pipeline(), [0xE1A0_0000, 0xE1A0_1001]);
}

#[test]
fn given_valid_image_when_loading_then_the_arm7_clock_and_now_are_zero() {
    let machine = booted(&Image::valid().build());
    assert_eq!((machine.arm7_clock(), machine.scheduler().now()), (0, 0));
}

#[test]
fn given_machine_that_ran_a_frame_when_loading_a_rom_then_both_clocks_restart_at_zero() {
    let mut machine = booted(&counting_rom());
    machine.run_frame();
    machine.load_rom(counting_rom()).expect("valid image");
    assert_eq!((machine.arm7_clock(), machine.scheduler().now()), (0, 0));
}

// ------------------------------------------------------------ ARM7 memory

#[test]
fn given_arm7_program_when_run_one_frame_then_it_finished_every_step() {
    let mut machine = booted(&arm7_memory_rom());
    machine.run_frame();
    assert_eq!(record(&machine, 5), 7);
}

#[test]
fn given_arm7_stores_to_its_wram_when_it_reads_the_word_back_then_it_reads_the_stored_value() {
    let mut machine = booted(&arm7_memory_rom());
    machine.run_frame();
    assert_eq!(record(&machine, 1), 0xA5);
}

// TODO(hw-unverified): the 64 KiB WRAM repeating through 0x0380_0000-0x03FF_FFFF is GBATEK's
// general mirroring rule (scheduler.md, "Unverified").
#[test]
fn given_arm7_stores_to_its_wram_when_it_reads_at_plus_64_kib_then_it_reads_the_mirror_as_the_note_models_unverified()
 {
    let mut machine = booted(&arm7_memory_rom());
    machine.run_frame();
    assert_eq!(record(&machine, 0), 0xA5);
}

// TODO(hw-unverified): open bus reads 0 on the ARM7 (scheduler.md, "Unverified").
#[test]
fn given_arm7_reads_the_io_area_when_it_is_unmapped_then_it_reads_zero_as_the_note_models_unverified()
 {
    let mut machine = booted(&arm7_memory_rom());
    machine.run_frame();
    assert_eq!(record(&machine, 2), 0x100);
}

// TODO(hw-unverified): as above, for the VRAM window that the ARM7 does not see yet (#35).
#[test]
fn given_arm7_reads_the_vram_window_when_it_is_unmapped_then_it_reads_zero_as_the_note_models_unverified()
 {
    let mut machine = booted(&arm7_memory_rom());
    machine.run_frame();
    assert_eq!(record(&machine, 4), 0x100);
}

// TODO(hw-unverified): main RAM repeating every 4 MiB inside its 16 MiB block
// (arm9-memory-map.md "Unverified"; scheduler.md "ARM7 memory").
#[test]
fn given_arm7_stores_to_main_ram_when_it_reads_at_plus_4_mib_then_it_reads_the_mirror_as_the_note_models_unverified()
 {
    let mut machine = booted(&arm7_memory_rom());
    machine.run_frame();
    assert_eq!(record(&machine, 3), 0xA5);
}

#[test]
fn given_arm7_stores_to_main_ram_when_the_arm9_peeks_it_then_both_cpus_see_the_same_word() {
    let mut machine = booted(&arm7_memory_rom());
    machine.run_frame();
    assert_eq!(machine.arm9_peek32(RECORD + 0x100), 0xA5);
}

/// Runs two frames of a machine whose clock sits at the very edge of `u64`, as only a
/// hand-made machine can; returns the frames counted.
fn frames_counted_at_the_edge_of_time() -> u64 {
    let mut machine = booted(&counting_rom());
    machine.scheduler.pop_due(u64::MAX);
    machine.scheduler.now = u64::MAX - 2 * FRAME_CYCLES;
    machine.arm7_clock = machine.scheduler.now;
    machine
        .scheduler
        .schedule(u64::MAX - FRAME_CYCLES, EventKind::LineStart);
    machine.run_frame();
    machine.run_frame();
    machine.frame_count()
}

#[test]
fn given_a_clock_at_the_edge_of_u64_when_two_frames_run_then_both_return() {
    // The second frame end saturates at u64::MAX and is due again at once; each `run_frame`
    // must still return instead of popping it forever.
    assert_eq!(frames_counted_at_the_edge_of_time(), 2);
}
