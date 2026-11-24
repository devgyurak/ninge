// SPDX-License-Identifier: Apache-2.0
//! POS_TEST, VEC_TEST and BOX_TEST, observed through our own `gxtest` test ROM
//! (`testroms/gxtest/`). The ARM9 sets a translation of (0x100, 0x200, 0x300) in the position
//! matrix, runs the three tests and records the results at `0x0200_8000`. Expected words are worked
//! out by hand from `docs/hw/geometry-engine.md`, never from running the ROM:
//!
//! - Slots 0-3: POS_TEST of (1, 2, 3) through the translation: `(0x101, 0x202, 0x303, 0x1000)`, the
//!   w of a point being 1.0.
//! - Slots 4-5: VEC_TEST of (256, -256, 0), which are 0.5 and -0.5 with 9 fraction bits, through the
//!   identity vector matrix: 12 fraction bits and a copied sign give `0800h`, `F800h` and `0`, so the
//!   word at `0400_0630h` is `F800_0800h` and the halfword at `0400_0634h` is 0.
//! - Slot 6: a box of size `0x200` at the corner `-0x100` is, translated, wholly inside the view
//!   volume: GXSTAT bit 1 is 2. Slot 7: a box at x = `0x3000` is beyond the right plane: 0.
//! - Slot 8: GXSTAT bit 0 read right after the last BOX_TEST word is 1: the test takes 103 bus
//!   cycles and is still busy.

use ninge_core::Machine;
use ninge_runner::golden::{assert_deterministic, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

const REC: u32 = 0x0200_8000;
const FRAMES: u64 = 2;
/// The record slots the ROM writes: 0-8, the progress and the sentinel.
const SLOTS: [u32; 11] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 62, 63];

fn boot_for(frames: u64) -> Machine {
    let mut machine = Machine::new();
    machine
        .load_rom(load_rom_file(&testrom_path("gxtest")).expect("gxtest ROM is built"))
        .expect("gxtest ROM has a valid header");
    (0..frames).for_each(|_| machine.run_frame(Default::default()));
    machine
}

fn record(machine: &Machine) -> Vec<u32> {
    SLOTS
        .iter()
        .map(|&slot| machine.inspect().arm9_peek32(REC + 4 * slot))
        .collect()
}

#[test]
fn given_the_gxtest_rom_when_it_runs_then_the_record_holds_the_note_values() {
    assert_eq!(
        record(&boot_for(FRAMES)),
        [
            0x101,       // POS_RESULT x
            0x202,       // y
            0x303,       // z
            0x1000,      // w = 1.0
            0xF800_0800, // VEC_RESULT x and y
            0,           // VEC_RESULT z
            // TODO(hw-unverified): the box test follows the model of the note
            // (`geometry-engine.md`, "Unverified").
            2,           // the inside box: GXSTAT bit 1
            0,           // the box beyond the right plane
            1,           // GXSTAT bit 0 right after sending: busy
            1,           // progress
            0x4554_5847, // "GXTE"
        ]
    );
}

#[test]
fn given_the_gxtest_rom_when_run_twice_then_the_frames_and_states_agree() {
    assert_deterministic("gxtest", FRAMES);
}

#[test]
fn given_a_state_saved_mid_run_when_loaded_and_run_on_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("gxtest", 1, 1);
}
