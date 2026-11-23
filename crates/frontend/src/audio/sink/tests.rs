// SPDX-License-Identifier: Apache-2.0
//! Tests of the ring sink: the start-up silence, and a trim that follows the ring's fill.

use super::*;

const DEVICE_HZ: u32 = 48_000;

/// A frame of 547 pairs of a constant.
fn frame() -> Vec<i16> {
    vec![100; 547 * 2]
}

#[test]
fn given_a_frame_when_pushed_then_the_ring_gains_about_a_frames_worth_at_the_device_rate() {
    let ring = Arc::new(AudioRing::new(8_192));
    let mut sink = RingSink::new(Arc::clone(&ring), DEVICE_HZ, 2_400);
    ring.push(&vec![0; 2_400 * 2]);
    sink.frame_started();
    sink.push(&frame());
    // 547 pairs at 32,728.5 Hz is 16.7 ms: about 802 pairs at 48 kHz.
    let gained = ring.fill() - 2_400;
    assert!((800..=804).contains(&gained), "gained {gained} pairs");
}

/// Pairs one frame adds when the ring held `fill` at its start, against a target of 2,400, after
/// the sink has had 20 such frames to settle: the trim follows the level at the start.
fn gained_with_fill(fill: usize) -> usize {
    let ring = Arc::new(AudioRing::new(65_536));
    let mut sink = RingSink::new(Arc::clone(&ring), DEVICE_HZ, 2_400);
    let mut gained = 0;
    (0..20).for_each(|_| {
        ring.pop_into(&mut vec![0; ring.fill() * 2]);
        ring.push(&vec![0; fill * 2]);
        sink.frame_started();
        sink.push(&frame());
        gained = ring.fill() - fill;
    });
    gained
}

#[test]
fn given_a_ring_above_its_target_at_frame_start_when_frames_are_pushed_then_fewer_pairs_are_made_than_from_one_below()
 {
    assert!(gained_with_fill(4_800) < gained_with_fill(0));
}

#[test]
fn given_a_ring_that_emptied_during_the_frame_when_pushed_then_the_trim_follows_the_level_at_the_start()
 {
    // At the start the ring was at its target; the device then took 2,000 pairs while the frame
    // ran. Read at the push, the ring looks empty and the trim would speed production up.
    let ring = Arc::new(AudioRing::new(65_536));
    let mut sink = RingSink::new(Arc::clone(&ring), DEVICE_HZ, 2_400);
    ring.push(&vec![0; 2_400 * 2]);
    sink.frame_started();
    ring.pop_into(&mut vec![0; 2_000 * 2]);
    let before = ring.fill();
    sink.push(&vec![100; 20_000 * 2]);
    // No trim: 20,000 pairs at 32,728.498 Hz make 29,333 pairs at 48 kHz.
    let made = ring.fill() - before;
    assert!((29_330..=29_336).contains(&made), "made {made} pairs");
}

#[test]
fn given_a_sink_that_drops_everything_when_pushed_then_nothing_happens() {
    let mut sink = NoAudio;
    sink.push(&frame());
    assert_eq!(format!("{sink:?}"), "NoAudio");
}
