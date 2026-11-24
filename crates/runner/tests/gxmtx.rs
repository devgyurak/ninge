// SPDX-License-Identifier: Apache-2.0
//! The geometry matrices, observed through our own `gxmtx` test ROM (`testroms/gxmtx/`). The ARM9
//! sends two display lists and copies CLIPMTX_RESULT and VECMTX_RESULT into main RAM at
//! `0x0200_8000` once the engine is idle. Expected words are worked out by hand from
//! `docs/hw/geometry-engine.md` ("Matrices"), never from running the ROM. `1.0` is `1000h`, a
//! multiply computes `C = M x C`, the clip matrix is position times projection:
//!
//! - Mode 2: identity, then a quarter turn R (rows `0 1 0`, `-1 0 0`, `0 0 1`) multiplied in, then a
//!   scale by 2. The scale acts on the position matrix only, so the vector matrix stays R.
//! - Mode 1: identity, translation (2, 1, -1), scale by 2: the position matrix S x T has `2 0 0 0`,
//!   `0 2 0 0`, `0 0 2 0` and the bottom row `2 1 -1 1` (S's bottom row is `0 0 0 1`).
//! - Mode 0: the projection P is the identity with 0.5 in the bottom row's first place.
//! - After list 1 the position is the identity (mode 1 push, then identity), so the clip matrix is P.
//! - After list 2 the pop restores S x T, and the clip matrix is S x T x P: the bottom row is
//!   `2 x 1.0 + 1 x 0.5 = 2.5`, `1`, `-1`, `1`, the rest as S x T.

use ninge_core::Machine;
use ninge_runner::golden::{assert_deterministic, assert_savestate_round_trip};
use ninge_runner::{load_rom_file, testrom_path};

const REC: u32 = 0x0200_8000;
const FRAMES: u64 = 2;
const MINUS_ONE: u32 = 0xFFFF_F000;

fn boot_for(frames: u64) -> Machine {
    let mut machine = Machine::new();
    machine
        .load_rom(load_rom_file(&testrom_path("gxmtx")).expect("gxmtx ROM is built"))
        .expect("gxmtx ROM has a valid header");
    (0..frames).for_each(|_| machine.run_frame(Default::default()));
    machine
}

fn slots(machine: &Machine, first: u32, count: u32) -> Vec<u32> {
    (first..first + count)
        .map(|slot| machine.inspect().arm9_peek32(REC + 4 * slot))
        .collect()
}

#[test]
fn given_the_gxmtx_rom_when_it_runs_then_the_clip_matrix_after_list_1_is_the_projection() {
    assert_eq!(
        slots(&boot_for(FRAMES), 0, 16),
        [
            0x1000, 0, 0, 0, //
            0, 0x1000, 0, 0, //
            0, 0, 0x1000, 0, //
            0x0800, 0, 0, 0x1000
        ]
    );
}

#[test]
fn given_the_gxmtx_rom_when_it_runs_then_the_clip_matrix_after_the_pop_is_position_times_projection()
 {
    assert_eq!(
        slots(&boot_for(FRAMES), 16, 16),
        [
            0x2000, 0, 0, 0, //
            0, 0x2000, 0, 0, //
            0, 0, 0x2000, 0, //
            0x2800, 0x1000, MINUS_ONE, 0x1000
        ]
    );
}

#[test]
fn given_the_gxmtx_rom_when_it_runs_then_the_vector_matrix_is_the_unscaled_quarter_turn() {
    assert_eq!(
        slots(&boot_for(FRAMES), 32, 9),
        [0, 0x1000, 0, MINUS_ONE, 0, 0, 0, 0, 0x1000]
    );
}

#[test]
fn given_the_gxmtx_rom_when_it_runs_then_it_reports_done_with_its_sentinel() {
    assert_eq!(slots(&boot_for(FRAMES), 62, 2), [1, 0x544D_5847]);
}

#[test]
fn given_the_gxmtx_rom_when_run_twice_then_the_frames_and_states_agree() {
    assert_deterministic("gxmtx", FRAMES);
}

#[test]
fn given_a_state_saved_mid_run_when_loaded_and_run_on_then_it_matches_an_uninterrupted_run() {
    assert_savestate_round_trip("gxmtx", 1, 1);
}
