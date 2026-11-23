// SPDX-License-Identifier: Apache-2.0
//! Tests of the audio sizes: the target is three frames, the capacity a quarter second.

use super::*;

#[test]
fn given_a_device_rate_when_the_target_is_asked_then_it_is_three_frame_periods_of_pairs() {
    // 3 x 16.715113 ms at 48 kHz = 2,406.97 pairs.
    assert_eq!(target_pairs(48_000), 2_406);
}

#[test]
fn given_a_device_rate_when_the_capacity_is_asked_then_it_is_a_quarter_second() {
    assert_eq!(ring_capacity(48_000), 12_000);
}
