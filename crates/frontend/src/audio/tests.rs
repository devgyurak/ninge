// SPDX-License-Identifier: Apache-2.0
//! Tests of the audio sizes: the target is three frames, the capacity a quarter second.

use super::*;
use crate::pacing::Pacer;
use std::time::Duration;

#[test]
fn given_a_device_rate_when_the_target_is_asked_then_it_is_three_frame_periods_of_pairs() {
    // 3 x 16.715113 ms at 48 kHz = 2,406.97 pairs.
    assert_eq!(target_pairs(48_000), 2_406);
}

#[test]
fn given_a_device_rate_when_the_capacity_is_asked_then_it_is_a_quarter_second() {
    assert_eq!(ring_capacity(48_000), 12_000);
}

fn device_output() -> Output {
    (
        Arc::new(AudioRing::new(ring_capacity(48_000))),
        Arc::new(AtomicBool::new(false)),
        48_000,
    )
}

#[test]
fn given_an_open_device_when_the_run_is_paced_then_it_follows_the_ring() {
    let (pacing, _sink) = paced(Some(device_output()), Instant::now());
    assert!(pacing.feeds_audio());
}

#[test]
fn given_a_ring_when_prefilled_then_it_holds_the_target_in_silence() {
    let ring = AudioRing::new(ring_capacity(48_000));
    prefill(&ring, target_pairs(48_000));
    let mut out = vec![1; 8];
    ring.pop_into(&mut out);
    assert_eq!(
        (out, ring.fill(), ring.underruns()),
        (vec![0; 8], target_pairs(48_000) - 4, 0)
    );
}

#[test]
fn given_no_device_when_the_run_is_paced_then_it_keeps_the_frame_period_and_feeds_no_audio() {
    let now = Instant::now();
    let (pacing, _sink) = paced(None, now);
    assert_eq!(
        (pacing.feeds_audio(), pacing.remaining(now)),
        (false, Duration::ZERO)
    );
}

#[test]
fn given_a_device_sink_when_a_frame_is_pushed_then_the_ring_gains_pairs() {
    let output = device_output();
    let ring = Arc::clone(&output.0);
    let (_pacing, mut sink) = paced(Some(output), Instant::now());
    prefill(&ring, target_pairs(48_000));
    sink.frame_started();
    sink.push(&[100; 547 * 2]);
    assert!(ring.fill() > target_pairs(48_000) + 790);
}

#[test]
fn given_no_device_sink_when_a_frame_is_pushed_then_nothing_happens() {
    let (_pacing, mut sink) = paced(None, Instant::now());
    sink.push(&[100; 547 * 2]);
    assert!(matches!(sink, Sink::None(_)));
}
