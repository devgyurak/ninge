// SPDX-License-Identifier: Apache-2.0
//! The timers, observed through our own `timers` test ROM (`testroms/timers/`). Each CPU records
//! what it reads in main RAM (ARM9 at 0x0200_4000, ARM7 at 0x0200_5000; the slot layouts are at
//! the top of `arm9.s` and `arm7.s`). Expected values come from `docs/hw/timers.md` ("Registers",
//! "Behavior", "Edge cases", "Model", "Unverified"), never from running the ROM.
//!
//! The frame measurements start and stop on the edge into VCOUNT 192, so a count covers one
//! frame of 263 x 355 x 6 = 560,190 bus cycles = 1,120,380 master cycles (`scheduler.md`, "Frame
//! timing") up to a few instructions of polling, which is far below one tick of the slow
//! prescalers. The expected counts are worked out here:
//!
//! - F/1024 (2,048 master cycles a tick): 1,120,380 / 2,048 = 547.06, so 547.
//! - F/256 (512 a tick): 1,120,380 / 512 = 2,188.2, so 2,188.
//! - F/1 with reload 0 (131,072 master cycles a period): 1,120,380 / 131,072 = 8.5, so the
//!   count-up timer behind it reads 8, and the first timer is 71,804 cycles = 35,902 ticks into
//!   its ninth period.
//!
//! Slots the note marks Unverified are in tests whose names say so (`TODO(hw-unverified)`).

use ninge_core::Machine;
use ninge_runner::{load_rom_file, testrom_path};

const ARM9_RECORD: u32 = 0x0200_4000;
const ARM7_RECORD: u32 = 0x0200_5000;

/// Frames to run: the ARM9 waits for up to two frame edges and then runs a third of a frame of
/// IRQs; the ARM7 needs about three.
const FRAMES: u32 = 6;

/// A mismatch: (slot, name, lowest accepted value, highest accepted value, got).
type Mismatch = (u32, &'static str, u32, u32, u32);

/// Expected record: (slot, name, lowest accepted value, highest accepted value). An exact value
/// has the same bound twice.
type Row = (u32, &'static str, u32, u32);

fn rom_machine() -> Machine {
    let rom = load_rom_file(&testrom_path("timers")).expect("timers ROM is built");
    let mut machine = Machine::new();
    machine
        .load_rom(rom)
        .expect("timers ROM has a valid header");
    machine
}

fn run(machine: &mut Machine, frames: u32) {
    (0..frames).for_each(|_| machine.run_frame());
}

/// A machine that booted the `timers` ROM and ran long enough for both CPUs to finish.
fn boot() -> Machine {
    let mut machine = rom_machine();
    run(&mut machine, FRAMES);
    machine
}

fn mismatches(machine: &Machine, base: u32, rows: &[Row]) -> Vec<Mismatch> {
    rows.iter()
        .map(|&(slot, name, low, high)| {
            (
                slot,
                name,
                low,
                high,
                machine.inspect().arm9_peek32(base + slot * 4),
            )
        })
        .filter(|&(_, _, low, high, got)| !(low..=high).contains(&got))
        .collect()
}

fn arm9_mismatches(rows: &[Row]) -> Vec<Mismatch> {
    mismatches(&boot(), ARM9_RECORD, rows)
}

fn arm7_mismatches(rows: &[Row]) -> Vec<Mismatch> {
    mismatches(&boot(), ARM7_RECORD, rows)
}

// ------------------------------------------------------------------ completion

const DONE: &[Row] = &[
    (30, "ARM9 progress", 1, 1),
    (31, "ARM9 sentinel", 0x544D_5239, 0x544D_5239),
];

#[test]
fn given_timers_rom_when_run_then_the_arm9_finishes_and_writes_its_sentinel() {
    assert_eq!(arm9_mismatches(DONE), Vec::<Mismatch>::new());
}

#[test]
fn given_timers_rom_when_run_then_the_arm7_finishes_and_writes_its_sentinel() {
    let rows: &[Row] = &[
        (30, "ARM7 progress", 1, 1),
        (31, "ARM7 sentinel", 0x544D_5237, 0x544D_5237),
    ];
    assert_eq!(arm7_mismatches(rows), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ overflow timing (ARM9)

#[test]
fn given_a_started_f1024_timer_when_one_frame_passes_then_its_counter_is_547() {
    // `timers.md`, "Clock" and "Counting": 1,120,380 master cycles / 2,048 per tick.
    let rows: &[Row] = &[(0, "timer 2 counter after a frame", 547, 547)];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_a_count_up_timer_behind_an_f1_timer_when_one_frame_passes_then_it_counted_8_overflows() {
    // `timers.md`, "Count-up": the F/1 timer with reload 0 overflows every 131,072 master cycles.
    let rows: &[Row] = &[(1, "timer 1 counter after a frame", 8, 8)];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_an_f1_timer_feeding_a_count_up_timer_when_one_frame_passes_then_it_is_35902_ticks_into_its_ninth_period()
 {
    // 1,120,380 - 8 x 131,072 = 71,804 master cycles = 35,902 ticks; polling skews a few ticks.
    let rows: &[Row] = &[(2, "timer 0 counter after a frame", 35_880, 35_925)];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_started_timers_when_their_control_is_read_back_then_it_is_what_was_written() {
    // `timers.md`, "TMnCNT_H bits": bits 0-2, 6 and 7 are kept.
    let rows: &[Row] = &[
        (3, "timer 1 control", 0x84, 0x84),
        (4, "timer 2 control", 0x83, 0x83),
    ];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ IRQ (ARM9)

#[test]
fn given_timer_3_with_its_irq_on_when_it_overflows_three_times_then_the_itcm_handler_ran_three_times()
 {
    // `timers.md`, "IF bits": an overflow of timer 3 requests IF bit 6; the handler saw only it.
    let rows: &[Row] = &[
        (5, "handler count", 3, 3),
        (6, "IF values the handler saw", 0x40, 0x40),
        (7, "IF after the acknowledge and the stop", 0, 0),
    ];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_a_stopped_timer_when_its_counter_is_read_twice_then_it_is_frozen_just_past_the_reload() {
    // `timers.md`, "Counting" (Stop): it keeps its value. It stopped a few instructions after an
    // overflow, so it is at most a couple of F/64 ticks past the reload FC00h.
    let rows: &[Row] = &[
        (8, "timer 3 counter just after the stop", 0xFC00, 0xFC02),
        (9, "timer 3 counter after a delay", 0xFC00, 0xFC02),
    ];
    let machine = boot();
    let frozen = (
        machine.inspect().arm9_peek32(ARM9_RECORD + 8 * 4),
        machine.inspect().arm9_peek32(ARM9_RECORD + 9 * 4),
    );
    assert_eq!(
        (
            mismatches(&machine, ARM9_RECORD, rows),
            frozen.0 == frozen.1
        ),
        (Vec::<Mismatch>::new(), true)
    );
}

#[test]
fn given_a_stopped_timer_when_it_is_started_again_then_the_reload_is_loaded() {
    // `timers.md`, "Counting" (Start): 0 -> 1 loads the reload value.
    let rows: &[Row] = &[(16, "timer 3 counter after a restart", 0xFC00, 0xFC01)];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_a_running_timer_when_its_irq_enable_is_clear_then_if_is_not_latched_until_it_is_set() {
    // `timers.md`, "Edge cases" (IRQ disabled) and "IF bits": bit 4 for timer 1.
    let rows: &[Row] = &[
        (10, "IF with the IRQ enable clear", 0, 0),
        (11, "IF after the IRQ enable was set", 0x10, 0x10),
    ];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_timer_0_with_the_count_up_bit_when_it_runs_then_it_still_overflows_unverified() {
    // TODO(hw-unverified): `timers.md`, "Unverified": timer 0's bit 2 is ignored.
    let rows: &[Row] = &[(12, "IF bit 3", 0x08, 0x08)];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ reload (ARM9)

#[test]
fn given_a_word_write_of_reload_and_start_when_the_counter_is_read_then_it_starts_from_the_reload()
{
    // `timers.md`, "Writing a 32-bit word": the reload lands before the start.
    let rows: &[Row] = &[
        (13, "timer 2 counter after the word write", 0x1000, 0x1010),
        (15, "timer 2 control", 0x80, 0x80),
    ];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_a_running_timer_when_its_reload_is_written_then_the_counter_does_not_jump() {
    // `timers.md`, "Counting" (Reload): the new reload applies from the next overflow.
    let rows: &[Row] = &[(14, "timer 2 counter after the reload write", 0x1000, 0x1020)];
    assert_eq!(arm9_mismatches(rows), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ the ARM7's own timers

#[test]
fn given_an_arm7_f256_timer_when_one_frame_passes_then_its_counter_is_2188() {
    // `timers.md`, "Clock": the ARM7's timers run from the same bus clock, 512 master cycles a
    // tick at F/256: 1,120,380 / 512 = 2,188.2.
    let rows: &[Row] = &[(0, "ARM7 timer 1 counter after a frame", 2_188, 2_188)];
    assert_eq!(arm7_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_an_arm7_count_up_chain_when_one_frame_passes_then_it_counted_8_overflows() {
    let rows: &[Row] = &[
        (1, "ARM7 timer 3 counter", 8, 8),
        (2, "ARM7 timer 2 counter", 35_880, 35_925),
        (3, "ARM7 timer 3 control", 0x84, 0x84),
        (4, "ARM7 timer 1 control", 0x82, 0x82),
    ];
    assert_eq!(arm7_mismatches(rows), Vec::<Mismatch>::new());
}

#[test]
fn given_an_arm7_timer_with_its_irq_on_when_it_overflows_then_if_latches_bit_3_with_ime_clear() {
    // `timers.md`, "IF bits"; the ARM7 takes no IRQ here (its vector is BIOS, #27).
    let rows: &[Row] = &[
        (5, "ARM7 IF while the timer runs", 0x08, 0x08),
        (6, "ARM7 IF after the acknowledge", 0, 0),
        (7, "ARM7 timer 0 control after the stop", 0, 0),
    ];
    assert_eq!(arm7_mismatches(rows), Vec::<Mismatch>::new());
}

// ------------------------------------------------------------------ determinism

#[test]
fn given_the_timers_rom_when_run_twice_then_the_savestate_bytes_are_identical() {
    assert_eq!(boot().save_state(), boot().save_state());
}

#[test]
fn given_a_savestate_taken_mid_run_when_loaded_and_continued_then_it_matches_an_uninterrupted_run()
{
    let mut first = rom_machine();
    run(&mut first, 2);
    let state = first.save_state();
    let mut resumed = rom_machine();
    let loaded = resumed.load_state(&state);
    run(&mut resumed, FRAMES - 2);
    assert_eq!(
        (loaded, resumed.save_state()),
        (Ok(()), boot().save_state())
    );
}
