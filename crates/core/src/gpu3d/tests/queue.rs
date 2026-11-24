// SPDX-License-Identifier: Apache-2.0
//! The FIFO and the PIPE as a queue. Expected values come from `docs/hw/geometry-engine.md`
//! ("The FIFO and the PIPE"): 4 PIPE entries and 256 FIFO entries; an entry goes straight into the
//! PIPE while the FIFO is empty and the PIPE has room, and into the FIFO otherwise; when the PIPE
//! holds fewer than 3 entries, 2 entries move up from the FIFO.

use crate::gpu3d::queue::{Entry, Queue};

/// A queue after `count` pushes of entries whose parameters are 0, 1, 2, ...
fn pushed(count: u32) -> Queue {
    let mut queue = Queue::new();
    (0..count).for_each(|param| {
        queue.push(Entry::new(0x11, param));
    });
    queue
}

/// The parameters of the entries taken until the queue is empty.
fn taken(queue: &mut Queue) -> Vec<u32> {
    std::iter::from_fn(|| queue.pop().map(|entry| entry.param)).collect()
}

#[test]
fn given_an_empty_queue_when_an_entry_is_pushed_then_it_goes_into_the_pipe() {
    let queue = pushed(1);
    assert_eq!((queue.pipe_len(), queue.fifo_len()), (1, 0));
}

#[test]
fn given_a_full_pipe_when_another_entry_is_pushed_then_it_goes_into_the_fifo() {
    let queue = pushed(5);
    assert_eq!((queue.pipe_len(), queue.fifo_len()), (4, 1));
}

#[test]
fn given_four_in_the_pipe_and_one_in_the_fifo_when_one_is_taken_then_the_pipe_does_not_refill() {
    let mut queue = pushed(5);
    queue.pop();
    assert_eq!((queue.pipe_len(), queue.fifo_len()), (3, 1));
}

#[test]
fn given_three_in_the_pipe_and_one_in_the_fifo_when_one_is_taken_then_the_fifo_entry_moves_up() {
    let mut queue = pushed(5);
    queue.pop();
    queue.pop();
    assert_eq!((queue.pipe_len(), queue.fifo_len()), (3, 0));
}

#[test]
fn given_a_long_fifo_when_the_pipe_runs_below_three_then_two_entries_move_up() {
    let mut queue = pushed(14);
    queue.pop();
    queue.pop();
    assert_eq!((queue.pipe_len(), queue.fifo_len()), (4, 8));
}

#[test]
fn given_260_entries_when_a_261st_is_pushed_then_it_is_refused_and_the_fifo_is_full() {
    let mut queue = pushed(260);
    let accepted = queue.push(Entry::new(0x11, 260));
    assert_eq!(
        (accepted, queue.fifo_full(), queue.len()),
        (false, true, 260)
    );
}

#[test]
fn given_entries_pushed_in_order_when_taken_then_they_come_out_in_the_same_order() {
    let mut queue = pushed(10);
    assert_eq!(taken(&mut queue), (0..10).collect::<Vec<_>>());
}

/// A queue filled to 260, drained by 100 and refilled with 100 more, so the FIFO's ring has wrapped.
fn wrapped_ring() -> Queue {
    let mut queue = pushed(260);
    (0..100).for_each(|_| {
        queue.pop();
    });
    (260..360).for_each(|param| {
        queue.push(Entry::new(0x11, param));
    });
    queue
}

#[test]
fn given_a_wrapped_ring_when_entries_are_taken_then_the_order_is_kept() {
    assert_eq!(taken(&mut wrapped_ring()), (100..360).collect::<Vec<_>>());
}

#[test]
fn given_a_queue_when_an_entry_is_looked_up_by_position_then_the_pipe_comes_first() {
    let queue = pushed(6);
    let params: Vec<_> = (0..7)
        .map(|index| queue.get(index).map(|entry| entry.param))
        .collect();
    assert_eq!(
        params,
        vec![Some(0), Some(1), Some(2), Some(3), Some(4), Some(5), None]
    );
}

#[test]
fn given_a_queue_with_entries_when_checked_then_it_is_well_formed() {
    assert!(pushed(200).well_formed());
}
