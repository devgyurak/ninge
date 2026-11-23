// SPDX-License-Identifier: Apache-2.0
//! Pacing: how long the emu side waits after a frame before it starts the next.
//!
//! The pacers never read a clock; their caller passes the `Instant`s, so a test controls time.
//! The emu loop is generic over [`Pacer`], so the way the machine is kept to its speed changes
//! without touching the loop (#47, plan §4.3):
//!
//! - [`AudioPacer`] waits while the audio ring holds more than its target, so the device's clock
//!   sets the speed and sound never underruns or lags.
//! - [`FramePacer`] waits out a frame period, the machine's real one ([`FRAME_TIME`]), when
//!   there is no sound to follow.
//! - [`Pacing`] is the one the run uses: audio while the device keeps up, and frame pacing for
//!   good from the first sign that it does not.
//!
//! Pacing changes wall-clock time only. What the machine computes, when input is applied (at
//! frame boundaries) and what a savestate holds do not depend on it.

use ninge_core::{FRAME_CYCLES, MASTER_CLOCK_HZ};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// One frame of the machine: [`FRAME_CYCLES`] master cycles at [`MASTER_CLOCK_HZ`], 16.715113 ms
/// (about 59.826 Hz; `docs/hw/scheduler.md`, "Clocks"), so a game runs at the same speed with
/// and without a sound device. The terminal's own output rate is #63, not this.
pub const FRAME_TIME: Duration =
    Duration::from_nanos(FRAME_CYCLES * 1_000_000_000 / MASTER_CLOCK_HZ);

/// The longest one audio wait lasts: two frame periods, so a key press or a stop is still seen
/// when the ring is far above its target.
pub const MAX_AUDIO_WAIT: Duration = FRAME_TIME.saturating_mul(2);

/// How long the device may go without pulling a single pair before it counts as stopped, and the
/// run falls back to frame pacing. A running device calls back every few milliseconds; one that
/// does not would otherwise stall the emu thread.
pub const STALL_LIMIT: Duration = Duration::from_millis(250);

/// How many frame starts in a row must see no pull before the device counts as stopped, besides
/// [`STALL_LIMIT`] passing. One long pause of the whole process (a laptop waking from sleep, a
/// debugger) is one frame start, so it never alone condemns a device.
pub const STALL_FRAMES: u32 = 3;

/// What the pacer reads of the audio ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Level {
    /// Pairs stored now.
    pub fill: usize,
    /// Pairs the device has asked for so far, stored or missing (`AudioRing::pulled`).
    pub pulled: u64,
}

/// What decides when the next frame starts.
pub trait Pacer {
    /// A frame starts at `now`.
    fn frame_started(&mut self, now: Instant);

    /// How long to wait, at `now`, before the next frame; zero when it is already due.
    fn remaining(&self, now: Instant) -> Duration;

    /// Whether the audio ring is still being fed: false for a pacer that does not follow sound,
    /// and for [`Pacing`] once it has fallen back.
    fn feeds_audio(&self) -> bool {
        false
    }
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

/// Follows the audio ring: when it holds more than `target` pairs the device has that much sound
/// left to play, so the machine waits for it to drain to the target; at or below, the next frame
/// is due at once. The wait is capped at [`MAX_AUDIO_WAIT`].
///
/// `level` reads the ring, so a test gives it a fake.
#[derive(Clone, Debug)]
pub struct AudioPacer<F> {
    level: F,
    target: usize,
    device_hz: u32,
}

impl<F: Fn() -> Level> AudioPacer<F> {
    /// A pacer for a ring read through `level`, kept near `target` pairs, drained at `device_hz`
    /// pairs a second.
    ///
    /// # Panics
    ///
    /// When `device_hz` is zero.
    pub fn new(level: F, target: usize, device_hz: u32) -> Self {
        assert!(device_hz > 0, "a device rate of zero never drains");
        Self {
            level,
            target,
            device_hz,
        }
    }

    /// The ring now.
    fn level(&self) -> Level {
        (self.level)()
    }
}

impl<F: Fn() -> Level> Pacer for AudioPacer<F> {
    /// Nothing to note: the ring, not a deadline, decides.
    fn frame_started(&mut self, _now: Instant) {}

    fn remaining(&self, _now: Instant) -> Duration {
        let excess = self.level().fill.saturating_sub(self.target) as u64;
        Duration::from_nanos(excess.saturating_mul(1_000_000_000) / u64::from(self.device_hz))
            .min(MAX_AUDIO_WAIT)
    }

    fn feeds_audio(&self) -> bool {
        true
    }
}

/// Notices a device that stopped pulling: seen once a frame, the count of pairs it has asked for
/// has not grown for [`STALL_FRAMES`] frame starts in a row and longer than [`STALL_LIMIT`].
///
/// It watches what the device does, not how full the ring is: an emulator that runs slower than
/// real time leaves the fill where it was while the device keeps calling back.
#[derive(Clone, Copy, Debug)]
struct StallWatch {
    last_pulled: Option<u64>,
    last_progress: Instant,
    idle_frames: u32,
}

impl StallWatch {
    fn new(now: Instant) -> Self {
        Self {
            last_pulled: None,
            last_progress: now,
            idle_frames: 0,
        }
    }

    /// Notes that the device had asked for `pulled` pairs at `now`; true once it has stopped.
    fn stalled(&mut self, pulled: u64, now: Instant) -> bool {
        if self.last_pulled == Some(pulled) {
            self.idle_frames += 1;
        } else {
            self.idle_frames = 0;
            self.last_progress = now;
        }
        self.last_pulled = Some(pulled);
        self.idle_frames >= STALL_FRAMES
            && now.saturating_duration_since(self.last_progress) > STALL_LIMIT
    }
}

/// The pacing a run uses: [`AudioPacer`] while the device keeps up, [`FramePacer`] after it does
/// not, for good (as the shared-memory to zlib switch is, AGENTS.md §2 rule 12).
///
/// It leaves audio at the first frame start that sees `failed` set (the device's error
/// callback sets it) or a stall ([`STALL_LIMIT`], [`STALL_FRAMES`]). It never goes back, so the
/// speed does not wander between two clocks, and [`Pacer::feeds_audio`] turns false so nothing
/// more is pushed.
#[derive(Debug)]
pub struct Pacing<F>(Mode<F>);

#[derive(Debug)]
enum Mode<F> {
    /// Following the device.
    Audio {
        pacer: AudioPacer<F>,
        /// Set by the device's error callback.
        failed: Arc<AtomicBool>,
        watch: StallWatch,
    },
    /// Frame pacing, for good.
    Frame(FramePacer),
}

impl<F: Fn() -> Level> Pacing<F> {
    /// Audio pacing from `now`, falling back when `failed` is set or the device stalls.
    pub fn audio(pacer: AudioPacer<F>, failed: Arc<AtomicBool>, now: Instant) -> Self {
        Self(Mode::Audio {
            pacer,
            failed,
            watch: StallWatch::new(now),
        })
    }

    /// Frame pacing from `now`, for a run with no sound device.
    pub fn frame(now: Instant) -> Self {
        Self(Mode::Frame(FramePacer::new(now)))
    }
}

impl<F: Fn() -> Level> Pacer for Pacing<F> {
    fn frame_started(&mut self, now: Instant) {
        if let Mode::Audio {
            pacer,
            failed,
            watch,
        } = &mut self.0
        {
            // `stalled` runs every frame, even after `failed`, so its record stays current.
            let stalled = watch.stalled(pacer.level().pulled, now);
            if failed.load(Ordering::Acquire) || stalled {
                self.0 = Mode::Frame(FramePacer::new(now));
            }
        }
        if let Mode::Frame(pacer) = &mut self.0 {
            pacer.frame_started(now);
        }
    }

    fn remaining(&self, now: Instant) -> Duration {
        match &self.0 {
            Mode::Audio { pacer, .. } => pacer.remaining(now),
            Mode::Frame(pacer) => pacer.remaining(now),
        }
    }

    fn feeds_audio(&self) -> bool {
        matches!(self.0, Mode::Audio { .. })
    }
}

#[cfg(test)]
mod tests;
