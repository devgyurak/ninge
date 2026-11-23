// SPDX-License-Identifier: Apache-2.0
//! Tests of `handoff`.

use super::*;
use std::sync::Arc;

/// A `Slots` with a number stored in each slot, so a test sees which frame the reader got.
struct Frames {
    slots: Slots,
    contents: [u32; 3],
}

impl Frames {
    fn new() -> Self {
        Self {
            slots: Slots::new(),
            contents: [0; 3],
        }
    }

    fn publish(&mut self, value: u32) {
        self.contents[self.slots.back()] = value;
        self.slots.publish();
    }

    fn take(&mut self) -> Option<u32> {
        self.slots.take().map(|i| self.contents[i])
    }
}

#[test]
fn given_a_published_frame_when_the_reader_takes_then_it_gets_that_frame() {
    let mut frames = Frames::new();
    frames.publish(7);
    assert_eq!(frames.take(), Some(7));
}

#[test]
fn given_two_publishes_before_a_take_when_the_reader_takes_then_it_gets_only_the_second() {
    let mut frames = Frames::new();
    frames.publish(1);
    frames.publish(2);
    assert_eq!((frames.take(), frames.take()), (Some(2), None));
}

#[test]
fn given_no_publish_when_the_reader_takes_then_there_is_nothing() {
    assert_eq!(Frames::new().take(), None);
}

#[test]
fn given_a_frame_already_taken_when_the_reader_takes_again_then_there_is_nothing_new() {
    let mut frames = Frames::new();
    frames.publish(3);
    let _ = frames.take();
    assert_eq!(frames.take(), None);
}

#[test]
fn given_a_take_between_publishes_when_the_reader_takes_again_then_it_gets_the_newer_frame() {
    let mut frames = Frames::new();
    frames.publish(1);
    let first = frames.take();
    frames.publish(2);
    assert_eq!((first, frames.take()), (Some(1), Some(2)));
}

/// One step of a sequence.
#[derive(Clone, Copy)]
enum Op {
    Publish,
    Take,
}

/// Every sequence of `len` operations.
fn sequences(len: u32) -> Vec<Vec<Op>> {
    (0..1u32 << len)
        .map(|bits| {
            (0..len)
                .map(|i| {
                    if bits >> i & 1 == 1 {
                        Op::Publish
                    } else {
                        Op::Take
                    }
                })
                .collect()
        })
        .collect()
}

/// The sequences after which two of the three indices are equal, or one is out of range, at any
/// step.
fn overlapping_sequences(len: u32) -> Vec<String> {
    sequences(len)
        .into_iter()
        .enumerate()
        .filter(|(_, ops)| {
            let mut slots = Slots::new();
            ops.iter().any(|op| {
                match op {
                    Op::Publish => slots.publish(),
                    Op::Take => {
                        let _ = slots.take();
                    }
                }
                let mut all = [slots.back, slots.middle, slots.front];
                all.sort_unstable();
                all != [0, 1, 2]
            })
        })
        .map(|(i, _)| format!("sequence {i}"))
        .collect()
}

#[test]
fn given_any_publish_and_take_sequence_when_indices_are_checked_then_writer_and_reader_never_share_a_slot()
 {
    assert_eq!(overlapping_sequences(10), Vec::<String>::new());
}

#[test]
fn given_a_published_frame_through_the_wrapper_when_the_reader_takes_then_it_reads_that_frame() {
    let handoff = Handoff::new(|| 0u32);
    handoff.publish(|slot| *slot = 41);
    assert_eq!(
        handoff.take(Duration::from_millis(10), |slot| *slot),
        Some(41)
    );
}

#[test]
fn given_no_publish_when_the_reader_waits_with_a_timeout_then_it_returns_nothing() {
    let handoff = Handoff::new(|| 0u32);
    assert_eq!(handoff.take(Duration::from_millis(5), |slot| *slot), None);
}

#[test]
fn given_a_wake_when_the_reader_waits_then_it_returns_at_once_and_the_next_wait_is_normal() {
    let handoff = Handoff::new(|| 0u32);
    handoff.wake();
    let woken = handoff.take(Duration::from_secs(30), |slot| *slot);
    handoff.publish(|slot| *slot = 5);
    assert_eq!(
        (woken, handoff.take(Duration::from_millis(10), |slot| *slot)),
        (None, Some(5))
    );
}

#[test]
fn given_a_reader_waiting_on_another_thread_when_a_frame_is_published_then_it_wakes_with_it() {
    let handoff = Arc::new(Handoff::new(|| 0u32));
    let reader = {
        let handoff = Arc::clone(&handoff);
        std::thread::spawn(move || handoff.take(Duration::from_secs(30), |slot| *slot))
    };
    // Publishing before the reader waits is fine too: the frame stays fresh until taken.
    handoff.publish(|slot| *slot = 9);
    assert_eq!(reader.join().expect("reader thread"), Some(9));
}
