// SPDX-License-Identifier: Apache-2.0
//! Savestate round trips over our own `smoke`, `boot` and `memmap` test ROMs (#19): a run
//! resumed from a savestate matches an uninterrupted run in every frame hash and in the final savestate bytes.

use ninge_runner::golden::{assert_deterministic, assert_savestate_round_trip};

#[test]
fn given_smoke_rom_when_resumed_from_a_savestate_after_2_frames_then_it_matches_an_uninterrupted_run()
 {
    assert_savestate_round_trip("smoke", 2, 3);
}

#[test]
fn given_boot_rom_when_resumed_from_a_savestate_after_1_frame_then_it_matches_an_uninterrupted_run()
{
    assert_savestate_round_trip("boot", 1, 2);
}

#[test]
fn given_boot_rom_when_resumed_from_a_savestate_taken_before_any_frame_then_it_matches_an_uninterrupted_run()
 {
    assert_savestate_round_trip("boot", 0, 2);
}

#[test]
fn given_boot_rom_when_run_twice_then_the_final_savestate_bytes_match() {
    assert_deterministic("boot", 2);
}

#[test]
fn given_memmap_rom_when_resumed_from_a_savestate_after_1_frame_then_it_matches_an_uninterrupted_run()
 {
    assert_savestate_round_trip("memmap", 1, 2);
}
