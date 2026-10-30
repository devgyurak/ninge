// SPDX-License-Identifier: Apache-2.0
//! Whole-machine smoke test through the runner on our own `smoke` test ROM.

use ninge_core::{SCREEN_HEIGHT, SCREEN_WIDTH};
use ninge_runner::{frame_hash, load_rom_file, run, testrom_path};

fn black_hash() -> u64 {
    frame_hash(&vec![0u32; SCREEN_WIDTH * SCREEN_HEIGHT])
}

fn run_smoke(frames: u64) -> ninge_runner::RunReport {
    let rom = load_rom_file(&testrom_path("smoke")).expect("smoke ROM is built");
    run(rom, frames)
}

#[test]
fn given_smoke_rom_when_run_for_3_frames_then_report_counts_3_frames() {
    assert_eq!(run_smoke(3).frames, 3);
}

#[test]
fn given_smoke_rom_when_run_for_3_frames_then_top_screen_hash_is_all_black() {
    assert_eq!(run_smoke(3).top_hash, black_hash());
}

#[test]
fn given_smoke_rom_when_run_for_3_frames_then_bottom_screen_hash_is_all_black() {
    assert_eq!(run_smoke(3).bottom_hash, black_hash());
}

#[test]
fn given_smoke_rom_when_run_for_1_frame_then_report_counts_1_frame() {
    assert_eq!(run_smoke(1).frames, 1);
}
