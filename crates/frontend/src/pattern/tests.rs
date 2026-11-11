// SPDX-License-Identifier: Apache-2.0
//! Tests of `pattern`.

use super::*;

#[test]
fn given_same_frame_number_when_generating_twice_then_buffers_are_equal() {
    assert_eq!(dummy_frame(5, 0), dummy_frame(5, 0));
}

#[test]
fn given_next_frame_number_when_generating_then_buffer_changes() {
    assert_ne!(dummy_frame(5, 0), dummy_frame(6, 0));
}

#[test]
fn given_different_phase_when_generating_then_screens_differ() {
    assert_ne!(dummy_frame(5, 0), dummy_frame(5, 1));
}

#[test]
fn given_frame_when_generating_then_it_has_one_pixel_per_screen_position() {
    assert_eq!(dummy_frame(0, 0).len(), 256 * 192);
}

#[test]
fn given_frame_when_generating_then_high_byte_is_zero() {
    assert!(dummy_frame(9, 1).iter().all(|p| p >> 24 == 0));
}
