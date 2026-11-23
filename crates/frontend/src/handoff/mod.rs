// SPDX-License-Identifier: Apache-2.0
//! The triple-buffered frame hand-off from the emu side to the render side (plan §4.3).
//!
//! Three slots: the writer fills its *back* slot and publishes it, the reader takes the newest
//! published slot as its *front*. Neither waits for the other to finish with a slot, because they
//! never hold the same one. A frame the reader has not taken yet is replaced by a newer publish, so
//! a slow reader skips frames instead of slowing the writer: every emulated frame is unchanged, and
//! only displayed frames may be dropped.
//!
//! [`Slots`] is the pure index logic; [`Handoff`] puts a mutex around each slot and a condition
//! variable around the indices, with no `unsafe`. A slot's mutex is never contended: the indices
//! the two sides hold are disjoint.

use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// Which slot is whose, and whether the middle one holds a frame the reader has not taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slots {
    /// The writer's slot.
    back: usize,
    /// The newest published slot, `fresh` while the reader has not taken it.
    middle: usize,
    /// The reader's slot.
    front: usize,
    fresh: bool,
}

impl Default for Slots {
    fn default() -> Self {
        Self::new()
    }
}

impl Slots {
    /// Nothing published: the three slots are the back, the middle and the front, in that order.
    pub const fn new() -> Self {
        Self {
            back: 0,
            middle: 1,
            front: 2,
            fresh: false,
        }
    }

    /// The slot the writer fills.
    pub const fn back(&self) -> usize {
        self.back
    }

    /// The slot the reader holds.
    pub const fn front(&self) -> usize {
        self.front
    }

    /// Whether a published frame waits to be taken.
    pub const fn fresh(&self) -> bool {
        self.fresh
    }

    /// The writer is done with its back slot: it becomes the newest frame and the writer gets the
    /// previous middle slot, replacing an untaken frame.
    pub fn publish(&mut self) {
        std::mem::swap(&mut self.back, &mut self.middle);
        self.fresh = true;
    }

    /// The reader takes the newest frame: its slot index, or `None` when nothing new was
    /// published since the last take.
    pub fn take(&mut self) -> Option<usize> {
        self.fresh.then(|| {
            std::mem::swap(&mut self.front, &mut self.middle);
            self.fresh = false;
            self.front
        })
    }
}

/// The indices and the wake request, under one lock.
#[derive(Debug, Default)]
struct State {
    slots: Slots,
    /// Set by [`Handoff::wake`]; the next wait ends early and clears it.
    wake: bool,
}

/// Three slots of `T` between one writer and one reader.
#[derive(Debug)]
pub struct Handoff<T> {
    slots: [Mutex<T>; 3],
    state: Mutex<State>,
    ready: Condvar,
}

/// Locks `mutex`; a panic on the other side leaves plain data behind, and the stop flag ends the
/// run, so a poisoned lock is not an error here.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<T> Handoff<T> {
    /// A hand-off whose three slots are made by `make`.
    pub fn new(mut make: impl FnMut() -> T) -> Self {
        Self {
            slots: [Mutex::new(make()), Mutex::new(make()), Mutex::new(make())],
            state: Mutex::new(State::default()),
            ready: Condvar::new(),
        }
    }

    /// Fills the writer's slot with `fill`, publishes it and wakes the reader. Only one thread may
    /// publish: the back slot belongs to it until the publish swaps it.
    pub fn publish(&self, fill: impl FnOnce(&mut T)) {
        let back = lock(&self.state).slots.back();
        fill(&mut lock(&self.slots[back]));
        lock(&self.state).slots.publish();
        self.ready.notify_all();
    }

    /// Waits up to `timeout` for a frame nobody took yet and reads it with `read`; `None` when the
    /// time passed with nothing new, or [`Handoff::wake`] ended the wait. The timeout lets the
    /// reader look at its stop flag.
    pub fn take<R>(&self, timeout: Duration, read: impl FnOnce(&T) -> R) -> Option<R> {
        let index = {
            let (mut state, _) = self
                .ready
                .wait_timeout_while(lock(&self.state), timeout, |s| !s.slots.fresh() && !s.wake)
                .unwrap_or_else(PoisonError::into_inner);
            state.wake = false;
            state.slots.take()
        };
        index.map(|i| read(&lock(&self.slots[i])))
    }

    /// Ends the reader's current or next wait early, so it can see a stop request at once.
    pub fn wake(&self) {
        lock(&self.state).wake = true;
        self.ready.notify_all();
    }
}

#[cfg(test)]
mod tests;
