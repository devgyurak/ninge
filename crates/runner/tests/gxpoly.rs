// SPDX-License-Identifier: Apache-2.0
//! Polygon assembly, clipping and Polygon RAM, observed through our own `gxpoly` test ROM
//! (`testroms/gxpoly/`). The ARM9 sends triangles with the matrices at their reset (the identity),
//! waits for the engine and records RAM_COUNT and DISP3DCNT in main RAM at `0x0200_8000`. Expected
//! words are worked out by hand from `docs/hw/geometry-engine.md`, never from running the ROM:
//!
//! - Slot 0: one triangle (3 vertices) and a four-vertex triangle strip (two triangles that share
//!   two vertices: 4 vertices) are 3 polygons and 7 vertices, `0007_0003h`. Slot 1: no overflow, 0.
//! - Slot 2: a triangle whose second corner is at x = 2.0 is cut by the right plane into four
//!   corners: one more polygon and four more vertices, 4 polygons and 11 vertices, `000B_0004h`.
//! - Slot 3: 2045 more triangles. Polygon RAM holds 2048, so 2044 fit and the last is dropped:
//!   2048 polygons and 11 + 3 x 2044 = 6143 vertices (`17FFh`), `17FF_0800h`. Slot 4: DISP3DCNT
//!   bit 13, `2000h`. Slot 5: a write of 1 to that bit clears it, 0.
//! - Slot 6: after SWAP_BUFFERS the set the geometry engine fills is empty: 0.

use ninge_core::Machine;
use ninge_runner::golden::{assert_deterministic, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

const REC: u32 = 0x0200_8000;
const FRAMES: u64 = 4;
/// The record slots the ROM writes: 0-6, the progress and the sentinel.
const SLOTS: [u32; 9] = [0, 1, 2, 3, 4, 5, 6, 30, 31];

fn boot_for(frames: u64) -> Machine {
    let mut machine = Machine::new();
    machine
        .load_rom(load_rom_file(&testrom_path("gxpoly")).expect("gxpoly ROM is built"))
        .expect("gxpoly ROM has a valid header");
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
fn given_the_gxpoly_rom_when_4_frames_run_then_the_record_holds_the_note_values() {
    assert_eq!(
        record(&boot_for(FRAMES)),
        [
            0x0007_0003, // a triangle and a strip of four: 3 polygons, 7 vertices
            0,           // no overflow
            0x000B_0004, // a clipped triangle: +1 polygon, +4 vertices
            // TODO(hw-unverified): the polygon that does not fit is dropped (`geometry-engine.md`,
            // "Unverified").
            0x17FF_0800, // Polygon RAM full: 2048 polygons, 6143 vertices
            0x2000,      // DISP3DCNT bit 13: overflow
            0,           // acknowledged
            0,           // after SWAP_BUFFERS the set is empty
            1,           // progress
            0x4F50_5847  // "GXPO"
        ]
    );
}

#[test]
fn given_the_gxpoly_rom_when_run_twice_then_the_frames_and_states_agree() {
    assert_deterministic("gxpoly", FRAMES);
}

#[test]
fn given_a_state_saved_mid_run_when_loaded_and_run_on_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("gxpoly", 1, 3);
}
