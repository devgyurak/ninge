// SPDX-License-Identifier: Apache-2.0
//! The scheduler running both CPUs through our own `sched` test ROM (`testroms/sched/`). The
//! ARM9 and the ARM7 each count forever and store the counter in main RAM after every
//! increment (ARM9 at 0x0200_1000, ARM7 at 0x0200_2000). The expected values come from the
//! cost model in `docs/hw/scheduler.md` ("Cost per instruction": ARM9 1 master cycle per
//! instruction, ARM7 2; "Frame timing": 1,120,380 master cycles per frame), never from running
//! the ROM:
//!
//! - A frame is 1,120,380 ARM9 instructions and 560,190 ARM7 instructions.
//! - ARM9: 3 prologue instructions, then a 3-instruction loop (`add`, `str`, `b`). After N
//!   frames the loop has run (N * 1,120,380 - 3) / 3 times, exactly: 1 frame gives 373,459,
//!   2 frames give 746,919.
//! - ARM7: 3 prologue instructions, then a 4-instruction loop (`add`, `str`, `mov`, `b`).
//!   After 1 frame, 560,187 loop instructions = 4 * 140,046 + 3: the partial 140,047th
//!   iteration got through `add`, `str` and `mov`, so the stored counter is 140,047. After
//!   2 frames, 1,120,377 = 4 * 280,094 + 1: the partial iteration stopped after its `add`,
//!   before the `str`, so memory still holds 280,094.
//!
//! The ROM draws nothing, so it has no golden frame hash.

use ninge_core::{FRAME_CYCLES, Machine, Screen};
use ninge_runner::golden::{assert_deterministic, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

const ARM9_COUNTER: u32 = 0x0200_1000;
const ARM7_COUNTER: u32 = 0x0200_2000;

/// A mismatch: (name, expected, got).
type Mismatch = (&'static str, u32, u32);

fn rom() -> Vec<u8> {
    load_rom_file(&testrom_path("sched")).expect("sched ROM is built")
}

fn booted() -> Machine {
    let mut machine = Machine::new();
    machine
        .load_rom(rom())
        .expect("sched ROM has a valid header");
    machine
}

fn run_frames(machine: &mut Machine, frames: u32) {
    (0..frames).for_each(|_| machine.run_frame());
}

/// Expected counters: (name, address, expected value after the given number of frames).
const AFTER_ONE_FRAME: &[(&str, u32, u32)] = &[
    ("ARM9 counter", ARM9_COUNTER, 373_459),
    ("ARM7 counter", ARM7_COUNTER, 140_047),
];
const AFTER_TWO_FRAMES: &[(&str, u32, u32)] = &[
    ("ARM9 counter", ARM9_COUNTER, 746_919),
    ("ARM7 counter", ARM7_COUNTER, 280_094),
];

/// Mismatches between `table` and what `machine` holds.
fn mismatches(machine: &Machine, table: &[(&'static str, u32, u32)]) -> Vec<Mismatch> {
    table
        .iter()
        .map(|&(name, address, expected)| (name, expected, machine.inspect().arm9_peek32(address)))
        .filter(|&(_, expected, got)| expected != got)
        .collect()
}

#[test]
fn given_sched_rom_when_run_one_frame_then_the_counters_show_the_model_instruction_counts() {
    let mut machine = booted();
    machine.run_frame();
    assert_eq!(
        mismatches(&machine, AFTER_ONE_FRAME),
        Vec::<Mismatch>::new()
    );
}

#[test]
fn given_sched_rom_when_run_two_frames_then_the_counters_show_the_model_instruction_counts() {
    let mut machine = booted();
    run_frames(&mut machine, 2);
    assert_eq!(
        mismatches(&machine, AFTER_TWO_FRAMES),
        Vec::<Mismatch>::new()
    );
}

#[test]
fn given_sched_rom_when_run_one_frame_then_the_arm7_clock_is_half_the_arm9_clock_count() {
    let mut machine = booted();
    machine.run_frame();
    assert_eq!(
        (
            machine.inspect().scheduler().now(),
            machine.inspect().arm7_clock() / 2
        ),
        (FRAME_CYCLES, FRAME_CYCLES / 2)
    );
}

#[test]
fn given_sched_rom_when_run_twice_then_the_final_savestate_bytes_match() {
    assert_deterministic("sched", 2);
}

#[test]
fn given_sched_rom_when_resumed_from_a_savestate_after_1_frame_then_it_matches_an_uninterrupted_run()
 {
    assert_savestate_round_trip("sched", 1, 2);
}

/// Differences between a run that saved in the middle of a frame (after `steps` debug steps)
/// and resumed in a fresh machine, and the run that simply continued: the savestate bytes at the
/// moment of the load, both screens after each of `frames` frames, the final savestate bytes,
/// and the final counters.
fn mid_frame_round_trip_differences(steps: u32, frames: u32) -> Vec<String> {
    let mut original = booted();
    (0..steps).for_each(|_| original.step_arm9());
    let saved = original.save_state();

    let mut resumed = booted();
    let mut differences: Vec<String> = Vec::new();
    match resumed.load_state(&saved) {
        Ok(()) => {}
        Err(error) => differences.push(format!("the savestate was refused: {error}")),
    }
    if resumed.save_state() != saved {
        differences.push("saving right after the load gives other bytes".to_string());
    }
    for frame in 1..=frames {
        original.run_frame();
        resumed.run_frame();
        for screen in [Screen::Top, Screen::Bottom] {
            if original.frame(screen) != resumed.frame(screen) {
                differences.push(format!("{screen:?} differs after frame {frame}"));
            }
        }
    }
    if original.save_state() != resumed.save_state() {
        differences.push("the final savestate bytes differ".to_string());
    }
    differences.extend(
        mismatches(&resumed, AFTER_TWO_FRAMES)
            .into_iter()
            .map(|(name, expected, got)| format!("{name}: expected {expected}, got {got}")),
    );
    differences
}

#[test]
fn given_sched_rom_when_saved_500000_debug_steps_in_and_resumed_then_it_matches_the_continuing_run()
{
    assert_eq!(
        mid_frame_round_trip_differences(500_000, 2),
        Vec::<String>::new()
    );
}
