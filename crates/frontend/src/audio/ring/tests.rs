// SPDX-License-Identifier: Apache-2.0
//! Tests of the audio ring: order, silence and the underrun count on an empty pop, the drop count
//! on a full push, wraparound, and a producer and a consumer on two threads.

use super::*;
use proptest::prelude::*;
use std::sync::Arc;
use std::thread;

/// `count` interleaved pairs: pair `k` (from `start`) is `(k, -k)`.
fn pairs(start: i16, count: i16) -> Vec<i16> {
    (start..start + count).flat_map(|k| [k, -k]).collect()
}

fn popped(ring: &AudioRing, pair_count: usize) -> Vec<i16> {
    let mut out = vec![99; pair_count * 2];
    ring.pop_into(&mut out);
    out
}

#[test]
fn given_pushed_pairs_when_popped_then_they_come_back_in_order() {
    let ring = AudioRing::new(16);
    ring.push(&pairs(1, 5));
    assert_eq!(popped(&ring, 5), pairs(1, 5));
}

#[test]
fn given_an_empty_ring_when_popped_then_the_output_is_silence_and_the_shortfall_is_counted() {
    let ring = AudioRing::new(16);
    assert_eq!(
        (popped(&ring, 3), ring.underruns()),
        (vec![0; 6], 3),
        "silence and one underrun per missing pair"
    );
}

#[test]
fn given_fewer_pairs_than_asked_when_popped_then_the_rest_is_silence_and_only_it_is_counted() {
    let ring = AudioRing::new(16);
    ring.push(&pairs(1, 2));
    assert_eq!(
        (popped(&ring, 4), ring.underruns()),
        (vec![1, -1, 2, -2, 0, 0, 0, 0], 2)
    );
}

#[test]
fn given_a_full_ring_when_more_is_pushed_then_only_what_fits_is_stored_and_the_rest_is_dropped() {
    let ring = AudioRing::new(4);
    let stored = ring.push(&pairs(1, 6));
    assert_eq!(
        (stored, ring.fill(), ring.drops(), popped(&ring, 4)),
        (4, 4, 2, pairs(1, 4))
    );
}

#[test]
fn given_a_capacity_that_is_not_a_power_of_two_when_the_ring_is_made_then_it_rounds_up() {
    assert_eq!(AudioRing::new(5).capacity(), 8);
}

#[test]
fn given_pushes_and_pops_across_the_capacity_boundary_when_read_then_order_is_kept() {
    let ring = AudioRing::new(8);
    ring.push(&pairs(1, 6));
    let first = popped(&ring, 6);
    ring.push(&pairs(7, 6));
    let second = popped(&ring, 6);
    assert_eq!([first, second].concat(), pairs(1, 12));
}

#[test]
fn given_mixed_pushes_and_pops_when_the_fill_is_asked_then_it_is_pushed_minus_popped() {
    let ring = AudioRing::new(16);
    ring.push(&pairs(1, 7));
    popped(&ring, 3);
    ring.push(&pairs(8, 2));
    assert_eq!(ring.fill(), 6);
}

#[test]
fn given_a_trailing_half_pair_when_pushed_then_it_is_ignored() {
    let ring = AudioRing::new(8);
    assert_eq!(ring.push(&[1, -1, 2]), 1);
}

#[test]
fn given_extreme_samples_when_pushed_and_popped_then_both_channels_round_trip() {
    let ring = AudioRing::new(4);
    ring.push(&[i16::MIN, i16::MAX, -1, 1]);
    assert_eq!(popped(&ring, 2), vec![i16::MIN, i16::MAX, -1, 1]);
}

/// Pairs the consumer received out of order, missing or extra, over a producer and a consumer
/// thread on a ring far smaller than the stream.
fn two_thread_failures(total: i16, capacity: usize) -> Vec<(usize, i16)> {
    let ring = Arc::new(AudioRing::new(capacity));
    let producer = {
        let ring = Arc::clone(&ring);
        thread::spawn(move || {
            let stream = pairs(1, total);
            let mut sent = 0;
            while sent < stream.len() {
                sent += 2 * ring.push(&stream[sent..stream.len().min(sent + 16)]);
                thread::yield_now();
            }
        })
    };
    let mut got = Vec::new();
    while got.len() < 2 * usize::try_from(total).unwrap() {
        let ready = ring.fill().min(16);
        let mut chunk = vec![0; ready * 2];
        ring.pop_into(&mut chunk);
        got.extend(chunk);
        thread::yield_now();
    }
    producer.join().unwrap();
    let want = pairs(1, total);
    got.iter()
        .zip(&want)
        .enumerate()
        .filter(|(_, (got, want))| got != want)
        .map(|(i, (got, _))| (i, *got))
        .chain((got.len() != want.len()).then_some((got.len(), 0)))
        .collect()
}

#[test]
fn given_a_producer_and_a_consumer_on_two_threads_when_a_stream_passes_then_nothing_is_lost_or_reordered()
 {
    assert_eq!(two_thread_failures(20_000, 64), vec![]);
}

#[test]
fn given_only_stored_pairs_asked_for_when_popped_then_no_underrun_is_counted() {
    let ring = AudioRing::new(8);
    ring.push(&pairs(1, 3));
    popped(&ring, 3);
    assert_eq!(ring.underruns(), 0);
}

/// What comes out of a ring after pushing a stream in `sizes`-pair pieces and draining after each,
/// and the stream that went in.
fn drained_stream(sizes: &[i16]) -> (Vec<i16>, Vec<i16>) {
    let ring = AudioRing::new(16);
    let (mut next, mut got) = (1_i16, Vec::new());
    for size in sizes {
        let stored = ring.push(&pairs(next, *size));
        next += i16::try_from(stored).unwrap();
        got.extend(popped(&ring, ring.fill()));
    }
    (got, pairs(1, next - 1))
}

proptest! {
    #[test]
    fn given_any_split_of_a_stream_when_pushed_and_popped_then_the_order_is_the_stream_order(
        sizes in proptest::collection::vec(1_i16..9, 1..40),
    ) {
        let (got, stream) = drained_stream(&sizes);
        prop_assert_eq!(got, stream);
    }
}

#[test]
fn given_pops_that_take_and_miss_pairs_when_pulled_is_asked_then_it_counts_both() {
    let ring = AudioRing::new(8);
    ring.push(&pairs(1, 2));
    popped(&ring, 5);
    assert_eq!((ring.pulled(), ring.underruns()), (5, 3));
}
