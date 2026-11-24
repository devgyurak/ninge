// SPDX-License-Identifier: Apache-2.0
//! The geometry command queue: the 256-entry FIFO and the 4-entry PIPE in front of the engine
//! (`docs/hw/geometry-engine.md`, "The FIFO and the PIPE").
//!
//! Both are fixed-size ring buffers of plain data: nothing here allocates after construction and
//! nothing is a pointer, so the queue is part of a savestate as it stands.

use serde::{Deserialize, Serialize};

/// Entries the FIFO holds (`geometry-engine.md`, "The FIFO and the PIPE").
pub(super) const FIFO_CAPACITY: usize = 256;
/// Entries the PIPE holds.
pub(super) const PIPE_CAPACITY: usize = 4;
/// The FIFO is "less than half full" below this many entries (GXSTAT bit 25).
pub(super) const HALF_FULL: usize = 128;
/// The PIPE refills from the FIFO when it holds fewer entries than this.
const PIPE_REFILL_BELOW: usize = 3;
/// How many entries one refill moves.
const PIPE_REFILL_BY: usize = 2;

/// One queued entry: a command ID with one 32-bit parameter, the 40 bits GBATEK counts. A command
/// with no parameter queues one entry whose `param` is 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Entry {
    pub(super) command: u8,
    pub(super) param: u32,
}

impl Entry {
    pub(super) const fn new(command: u8, param: u32) -> Self {
        Self { command, param }
    }
}

/// The FIFO and the PIPE. Entries enter at the back of the PIPE while the FIFO is empty and the
/// PIPE has room, and at the back of the FIFO otherwise; the engine takes them from the front of
/// the PIPE, which refills from the FIFO.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Queue {
    /// A ring of `FIFO_CAPACITY` entries, allocated once. A `Vec` because serde has no impl for an
    /// array this long; its length never changes.
    fifo: Vec<Entry>,
    fifo_head: u16,
    fifo_len: u16,
    pipe: [Entry; PIPE_CAPACITY],
    pipe_len: u8,
}

impl Queue {
    pub(super) fn new() -> Self {
        Self {
            fifo: vec![Entry::default(); FIFO_CAPACITY],
            fifo_head: 0,
            fifo_len: 0,
            pipe: [Entry::default(); PIPE_CAPACITY],
            pipe_len: 0,
        }
    }

    /// Entries in the FIFO proper: what GXSTAT counts.
    pub(in crate::gpu3d) fn fifo_len(&self) -> usize {
        usize::from(self.fifo_len)
    }

    /// Entries in the PIPE.
    pub(super) fn pipe_len(&self) -> usize {
        usize::from(self.pipe_len)
    }

    /// Entries in both.
    pub(super) fn len(&self) -> usize {
        self.fifo_len() + self.pipe_len()
    }

    /// Whether the FIFO cannot take another entry: the writer waits (`geometry-engine.md`, "The
    /// FIFO IRQ, the DMA start and the stall").
    pub(super) fn fifo_full(&self) -> bool {
        self.fifo_len() == FIFO_CAPACITY
    }

    /// The entry the engine would take next.
    pub(super) fn front(&self) -> Option<Entry> {
        (self.pipe_len > 0).then(|| self.pipe[0])
    }

    /// The `index`th entry from the front, PIPE first, for the debugger.
    pub(super) fn get(&self, index: usize) -> Option<Entry> {
        match index.checked_sub(self.pipe_len()) {
            None => self.pipe.get(index).copied(),
            Some(in_fifo) if in_fifo < self.fifo_len() => {
                self.fifo.get(self.fifo_slot(in_fifo)).copied()
            }
            Some(_) => None,
        }
    }

    /// Appends `entry`; `false` when both the PIPE route and the FIFO are closed (the FIFO is full).
    pub(super) fn push(&mut self, entry: Entry) -> bool {
        if self.fifo_len == 0 && self.pipe_len() < PIPE_CAPACITY {
            self.pipe[self.pipe_len()] = entry;
            self.pipe_len += 1;
            return true;
        }
        if self.fifo_full() {
            return false;
        }
        let slot = self.fifo_slot(self.fifo_len());
        self.fifo[slot] = entry;
        self.fifo_len += 1;
        true
    }

    /// Takes the front entry. When that leaves the PIPE below three entries, two move up from the
    /// FIFO at once (`geometry-engine.md`, "The FIFO and the PIPE").
    // TODO(hw-unverified): the refill is instant (`geometry-engine.md`, "Unverified").
    pub(super) fn pop(&mut self) -> Option<Entry> {
        let entry = self.front()?;
        self.pipe.copy_within(1.., 0);
        self.pipe_len -= 1;
        if self.pipe_len() < PIPE_REFILL_BELOW {
            (0..PIPE_REFILL_BY).for_each(|_| self.refill_one());
        }
        Some(entry)
    }

    /// Moves the front of the FIFO to the back of the PIPE, if there is one and room for it.
    fn refill_one(&mut self) {
        if self.fifo_len == 0 || self.pipe_len() == PIPE_CAPACITY {
            return;
        }
        self.pipe[self.pipe_len()] = self.fifo[usize::from(self.fifo_head)];
        self.pipe_len += 1;
        self.fifo_head = ((usize::from(self.fifo_head) + 1) % FIFO_CAPACITY) as u16;
        self.fifo_len -= 1;
    }

    /// The ring slot of the FIFO's `index`th entry from its head.
    fn fifo_slot(&self, index: usize) -> usize {
        (usize::from(self.fifo_head) + index) % FIFO_CAPACITY
    }

    /// Whether the queue holds only counts and positions that exist, for a savestate load.
    pub(super) fn well_formed(&self) -> bool {
        self.fifo.len() == FIFO_CAPACITY
            && usize::from(self.fifo_head) < FIFO_CAPACITY
            && self.fifo_len() <= FIFO_CAPACITY
            && self.pipe_len() <= PIPE_CAPACITY
            // Entries go to the FIFO only past a full PIPE, and the PIPE refills below three, so
            // a FIFO with entries has a PIPE of at least three.
            && (self.fifo_len == 0 || self.pipe_len() >= PIPE_REFILL_BELOW)
    }
}

impl Default for Queue {
    fn default() -> Self {
        Self::new()
    }
}
