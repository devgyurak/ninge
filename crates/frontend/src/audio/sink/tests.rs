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

/// The first `pairs` pairs of `ring`, popped.
fn popped(ring: &AudioRing, pairs: usize) -> Vec<i16> {
    let mut out = vec![1; pairs * 2];
    ring.pop_into(&mut out);
    out
}

#[test]
fn given_an_empty_ring_when_resumed_then_it_holds_exactly_the_target_in_silence() {
    let ring = Arc::new(AudioRing::new(8_192));
    let mut sink = RingSink::new(Arc::clone(&ring), DEVICE_HZ, 2_400);
    sink.resumed();
    let fill = ring.fill();
    assert_eq!((fill, popped(&ring, 2_400)), (2_400, vec![0; 4_800]));
}

#[test]
fn given_a_ring_below_the_target_when_resumed_then_only_the_difference_is_added_after_what_was_there()
 {
    let ring = Arc::new(AudioRing::new(8_192));
    let mut sink = RingSink::new(Arc::clone(&ring), DEVICE_HZ, 2_400);
    ring.push(&[7, 8, 9, 10]);
    sink.resumed();
    let fill = ring.fill();
    let mut expected = vec![7, 8, 9, 10];
    expected.resize(4_800, 0);
    assert_eq!((fill, popped(&ring, 2_400)), (2_400, expected));
}

#[test]
fn given_a_ring_at_or_above_the_target_when_resumed_then_nothing_is_added() {
    let at = Arc::new(AudioRing::new(8_192));
    let above = Arc::new(AudioRing::new(8_192));
    at.push(&vec![5; 2_400 * 2]);
    above.push(&vec![5; 3_000 * 2]);
    RingSink::new(Arc::clone(&at), DEVICE_HZ, 2_400).resumed();
    RingSink::new(Arc::clone(&above), DEVICE_HZ, 2_400).resumed();
    assert_eq!((at.fill(), above.fill()), (2_400, 3_000));
}

/// Pairs one push of 20,000 input pairs adds to an empty ring after `setup` has run.
fn gained_after(setup: impl FnOnce(&AudioRing, &mut RingSink)) -> usize {
    let ring = Arc::new(AudioRing::new(65_536));
    let mut sink = RingSink::new(Arc::clone(&ring), DEVICE_HZ, 2_400);
    setup(&ring, &mut sink);
    let before = ring.fill();
    sink.push(&vec![100; 20_000 * 2]);
    ring.fill() - before
}

#[test]
fn given_a_pause_that_emptied_the_ring_when_resumed_then_the_next_push_is_made_without_a_trim() {
    // A fresh sink's level is its target, so its push is untrimmed; after a frame started on an
    // empty ring the level would be 0 and speed production up, unless `resumed` resets it.
    let fresh = gained_after(|_, _| {});
    let resumed = gained_after(|_, sink| {
        sink.frame_started();
        sink.resumed();
    });
    assert_eq!(resumed, fresh);
}

#[test]
fn given_a_frame_started_on_an_empty_ring_without_resumed_when_pushed_then_it_makes_more_pairs() {
    // The control for the test above: the trim is there to be reset.
    let fresh = gained_after(|_, _| {});
    let trimmed = gained_after(|_, sink| sink.frame_started());
    assert!(trimmed > fresh, "trimmed {trimmed}, fresh {fresh}");
}

#[test]
fn given_a_sink_that_drops_everything_when_resumed_then_a_push_still_does_nothing() {
    let mut sink = NoAudio;
    sink.resumed();
    sink.push(&frame());
    assert_eq!(format!("{sink:?}"), "NoAudio");
}
