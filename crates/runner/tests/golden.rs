// SPDX-License-Identifier: Apache-2.0
//! Golden-frame tests over our own `smoke` test ROM. Review flow: `tests/golden/README.md`.

use ninge_runner::golden::{assert_deterministic, assert_frame_hash};

#[test]
fn given_smoke_rom_when_run_for_3_frames_then_both_screens_match_golden() {
    assert_frame_hash("smoke", 3);
}

#[test]
fn given_smoke_rom_when_run_twice_for_3_frames_then_every_frame_hash_matches() {
    assert_deterministic("smoke", 3);
}
