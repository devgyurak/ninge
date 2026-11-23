// SPDX-License-Identifier: Apache-2.0
//! Frame pacing: how long the emu side waits after a frame before it starts the next.
//!
//! [`FramePacer`] never reads a clock; its caller passes the `Instant`s, so a test controls time.
//! The emu loop is generic over [`Pacer`], so the audio-based pacing of #47 replaces the fixed
//! rate without touching the loop.

use std::time::{Duration, Instant};

/// Fixed output rate; adaptive rate is #63.
pub const FRAME_TIME: Duration = Duration::from_micros(16_667);

/// What decides when the next frame starts.
pub trait Pacer {
    /// A frame starts at `now`.
    fn frame_started(&mut self, now: Instant);

    /// How long to wait, at `now`, before the next frame; zero when it is already due.
    fn remaining(&self, now: Instant) -> Duration;
}

/// One frame every [`FRAME_TIME`]. A frame that overran is not made up for: the next one starts
/// when its predecessor ended, so the rate neither catches up in a burst nor drifts.
#[derive(Clone, Copy, Debug)]
pub struct FramePacer {
    deadline: Instant,
}

impl FramePacer {
    /// A pacer whose first frame is due at `now`.
    pub fn new(now: Instant) -> Self {
        Self { deadline: now }
    }
}

impl Pacer for FramePacer {
    fn frame_started(&mut self, now: Instant) {
        self.deadline = now + FRAME_TIME;
    }

    /// The time left until the deadline, or zero once it has passed. Zero still lets the caller
    /// poll, so pending keys and resizes are read every frame even when drawing overran.
    fn remaining(&self, now: Instant) -> Duration {
        self.deadline.saturating_duration_since(now)
    }
}

#[cfg(test)]
mod tests;
