// SPDX-License-Identifier: Apache-2.0
//! A lock-free single-producer single-consumer ring of stereo samples.
//!
//! The emu thread pushes resampled pairs and the device callback pops them (#47, plan §4.3). The
//! callback must neither block nor allocate, so the ring is built from atomics alone: no lock,
//! no `unsafe`, no allocation after [`AudioRing::new`].
//!
//! # Layout
//!
//! One slot is an `AtomicU32` holding one stereo pair: the left sample in the low 16 bits, the
//! right sample in the high 16 bits. `head` counts every pair ever pushed and `tail` every pair
//! ever popped, both wrapping `usize`; a slot's index is the count masked by `capacity - 1`, so
//! the capacity is a power of two. The producer owns `head` and the consumer owns `tail`; each
//! publishes with `Release` and reads the other's with `Acquire`, so a slot's content is visible
//! before the count that exposes it, and a slot is free before the count that reuses it. The slot
//! accesses themselves are `Relaxed`, ordered by those counts.
//!
//! One producer and one consumer only: two of either would race on the counts.

use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// The ring. Share it through an `Arc`: one thread pushes, one pops.
#[derive(Debug)]
pub struct AudioRing {
    slots: Vec<AtomicU32>,
    /// Pairs ever pushed; written by the producer only.
    head: AtomicUsize,
    /// Pairs ever popped; written by the consumer only.
    tail: AtomicUsize,
    /// Pairs the consumer needed and did not find.
    underruns: AtomicU64,
    /// Pairs the producer had no room for.
    drops: AtomicU64,
}

/// A pair in one slot: left in the low half, right in the high half.
fn pack(left: i16, right: i16) -> u32 {
    u32::from(left as u16) | (u32::from(right as u16) << 16)
}

/// The inverse of [`pack`].
fn unpack(slot: u32) -> [i16; 2] {
    // Truncation is the point: each half is one sample.
    [slot as u16 as i16, (slot >> 16) as u16 as i16]
}

impl AudioRing {
    /// A ring of `capacity` pairs, rounded up to a power of two (at least 1).
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1).next_power_of_two();
        Self {
            slots: (0..capacity).map(|_| AtomicU32::new(0)).collect(),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            underruns: AtomicU64::new(0),
            drops: AtomicU64::new(0),
        }
    }

    /// How many pairs the ring can hold.
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// The pairs stored now. Either side may ask; the value can be stale by the time it is used,
    /// but the producer only ever sees it grow stale downward and the consumer upward.
    pub fn fill(&self) -> usize {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        head.wrapping_sub(tail)
    }

    /// Producer: store interleaved `(left, right)` samples; returns the pairs stored. A full ring
    /// drops the rest and counts it ([`AudioRing::drops`]); the pacer keeps that from happening.
    /// A trailing unpaired sample is ignored.
    pub fn push(&self, samples: &[i16]) -> usize {
        let (pairs, _) = samples.as_chunks::<2>();
        let wanted = pairs.len();
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        let room = self.capacity() - head.wrapping_sub(tail);
        let stored = wanted.min(room);
        let mask = self.capacity() - 1;
        for (i, pair) in pairs.iter().take(stored).enumerate() {
            self.slots[head.wrapping_add(i) & mask]
                .store(pack(pair[0], pair[1]), Ordering::Relaxed);
        }
        self.head
            .store(head.wrapping_add(stored), Ordering::Release);
        self.drops
            .fetch_add((wanted - stored) as u64, Ordering::Relaxed);
        stored
    }

    /// Consumer: fill the whole of `out` with interleaved samples. What the ring does not hold is
    /// written as silence (0) and counted as underrun pairs ([`AudioRing::underruns`]). A
    /// trailing unpaired sample of `out` is silence.
    pub fn pop_into(&self, out: &mut [i16]) {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        let ready = head.wrapping_sub(tail);
        let mask = self.capacity() - 1;
        let mut taken = 0;
        let (pairs, rest) = out.as_chunks_mut::<2>();
        for (i, pair) in pairs.iter_mut().enumerate() {
            if i < ready {
                let [left, right] =
                    unpack(self.slots[tail.wrapping_add(i) & mask].load(Ordering::Relaxed));
                pair[0] = left;
                pair[1] = right;
                taken += 1;
            } else {
                pair.fill(0);
            }
        }
        rest.fill(0);
        self.tail.store(tail.wrapping_add(taken), Ordering::Release);
        let missed = pairs.len() - taken;
        self.underruns.fetch_add(missed as u64, Ordering::Relaxed);
    }

    /// Pairs the consumer found missing since the ring was made.
    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }

    /// Pairs the producer could not store since the ring was made.
    pub fn drops(&self) -> u64 {
        self.drops.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests;
