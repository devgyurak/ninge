// SPDX-License-Identifier: Apache-2.0
//! Tests of the ring sink: the start-up silence, and a trim that follows the ring's fill.

use super::*;

const DEVICE_HZ: u32 = 48_000;

/// A frame of 547 pairs of a constant.
fn frame() -> Vec<i16> {
    vec![100; 547 * 2]
}

#[test]
fn given_a_new_sink_when_the_ring_is_read_then_it_holds_the_target_in_silence() {
    let ring = Arc::new(AudioRing::new(8_192));
    let _sink = RingSink::new(Arc::clone(&ring), DEVICE_HZ, 2_400);
    let mut out = vec![1; 2_400 * 2];
    ring.pop_into(&mut out);
    assert_eq!((out, ring.fill(), ring.underruns()), (vec![0; 4_800], 0, 0));
}

#[test]
fn given_a_frame_when_pushed_then_the_ring_gains_about_a_frames_worth_at_the_device_rate() {
    let ring = Arc::new(AudioRing::new(8_192));
    let mut sink = RingSink::new(Arc::clone(&ring), DEVICE_HZ, 2_400);
    sink.push(&frame());
    // 547 pairs at 32,728.5 Hz is 16.7 ms: about 802 pairs at 48 kHz, plus the target.
    let gained = ring.fill() - 2_400;
    assert!((800..=804).contains(&gained), "gained {gained} pairs");
}

/// Pairs one frame adds to a ring that holds `extra` more than its target (negative: fewer).
fn gained_with_fill_offset(extra: isize) -> usize {
    let ring = Arc::new(AudioRing::new(65_536));
    let mut sink = RingSink::new(Arc::clone(&ring), DEVICE_HZ, 2_400);
    let mut sunk = vec![0; 2_400 * 2];
    // Start from an empty ring, then lay down the offset from the target.
    ring.pop_into(&mut sunk);
    ring.push(&vec![0; (2_400 + extra).max(0) as usize * 2]);
    let before = ring.fill();
    (0..20).for_each(|_| sink.push(&frame()));
    ring.fill() - before
}

#[test]
fn given_a_ring_above_its_target_when_frames_are_pushed_then_fewer_pairs_are_made_than_from_one_below()
 {
    assert!(gained_with_fill_offset(2_400) < gained_with_fill_offset(-2_400));
}

#[test]
fn given_a_sink_that_drops_everything_when_pushed_then_nothing_happens() {
    let mut sink = NoAudio;
    sink.push(&frame());
    assert_eq!(format!("{sink:?}"), "NoAudio");
}
