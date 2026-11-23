// SPDX-License-Identifier: Apache-2.0
//! Tests of `pacing`.

use super::*;
use std::cell::Cell;
use std::rc::Rc;

#[test]
fn given_time_left_when_the_pacer_is_asked_then_it_is_the_remainder() {
    let now = Instant::now();
    let mut pacer = FramePacer::new(now);
    pacer.frame_started(now);
    assert_eq!(
        pacer.remaining(now + Duration::from_millis(6)),
        FRAME_TIME - Duration::from_millis(6)
    );
}

#[test]
fn given_the_deadline_passed_when_the_pacer_is_asked_then_it_is_zero() {
    let now = Instant::now();
    let mut pacer = FramePacer::new(now);
    pacer.frame_started(now);
    assert_eq!(pacer.remaining(now + FRAME_TIME * 2), Duration::ZERO);
}

#[test]
fn given_a_new_pacer_when_it_is_asked_before_any_frame_then_the_first_frame_is_due() {
    let now = Instant::now();
    assert_eq!(FramePacer::new(now).remaining(now), Duration::ZERO);
}

#[test]
fn given_an_overrun_frame_when_the_next_starts_then_it_gets_a_whole_frame_time() {
    let now = Instant::now();
    let mut pacer = FramePacer::new(now);
    pacer.frame_started(now);
    let late = now + FRAME_TIME * 3;
    pacer.frame_started(late);
    assert_eq!(pacer.remaining(late), FRAME_TIME);
}

// ------------------------------------------------------------------------------------ audio

const DEVICE_HZ: u32 = 48_000;
const TARGET: usize = 2_400;

/// An audio pacer whose ring holds whatever `fill` says.
fn audio_pacer(fill: &Rc<Cell<usize>>) -> AudioPacer<impl Fn() -> usize + use<>> {
    let fill = Rc::clone(fill);
    AudioPacer::new(move || fill.get(), TARGET, DEVICE_HZ)
}

fn wait_at_fill(fill: usize) -> Duration {
    let ring = Rc::new(Cell::new(fill));
    audio_pacer(&ring).remaining(Instant::now())
}

#[test]
fn given_the_hardware_clock_when_the_frame_time_is_asked_then_it_is_16715113_nanoseconds() {
    assert_eq!(FRAME_TIME, Duration::from_nanos(16_715_113));
}

#[test]
fn given_the_frame_time_when_the_longest_audio_wait_is_asked_then_it_is_two_frames() {
    assert_eq!(MAX_AUDIO_WAIT, FRAME_TIME * 2);
}

#[test]
fn given_a_ring_above_its_target_when_the_pacer_is_asked_then_it_waits_for_the_excess_to_play() {
    // 480 pairs at 48 kHz is 10 ms.
    assert_eq!(wait_at_fill(TARGET + 480), Duration::from_millis(10));
}

#[test]
fn given_a_ring_at_or_below_its_target_when_the_pacer_is_asked_then_it_waits_zero() {
    assert_eq!(
        (
            wait_at_fill(TARGET),
            wait_at_fill(TARGET - 1),
            wait_at_fill(0)
        ),
        (Duration::ZERO, Duration::ZERO, Duration::ZERO)
    );
}

#[test]
fn given_a_ring_far_above_its_target_when_the_pacer_is_asked_then_the_wait_is_capped_at_two_frames()
{
    assert_eq!(wait_at_fill(TARGET + 48_000), MAX_AUDIO_WAIT);
}

#[test]
fn given_an_audio_pacer_when_asked_then_it_feeds_audio_and_a_frame_pacer_does_not() {
    let ring = Rc::new(Cell::new(0));
    assert_eq!(
        (
            audio_pacer(&ring).feeds_audio(),
            FramePacer::new(Instant::now()).feeds_audio()
        ),
        (true, false)
    );
}

/// Whether a [`Pacing`] still feeds audio after frames start at the `(milliseconds, fill)` steps.
fn feeds_after(steps: &[(u64, usize)]) -> bool {
    let ring = Rc::new(Cell::new(0));
    let start = Instant::now();
    let mut pacing = Pacing::audio(audio_pacer(&ring), Arc::new(AtomicBool::new(false)), start);
    steps.iter().for_each(|&(ms, fill)| {
        ring.set(fill);
        pacing.frame_started(start + Duration::from_millis(ms));
    });
    pacing.feeds_audio()
}

/// One frame start every 17 ms for `frames` frames; the fill is `fill(i)` at frame `i`.
fn frame_starts(frames: u64, fill: impl Fn(u64) -> usize) -> Vec<(u64, usize)> {
    (0..frames).map(|i| (i * 17, fill(i))).collect()
}

#[test]
fn given_a_fill_that_never_drops_when_frames_run_past_the_stall_limit_then_pacing_falls_back() {
    // 20 frames, 323 ms: a device that stopped pulling while the emu thread keeps pushing.
    assert!(!feeds_after(&frame_starts(20, |i| 2_400 + 800 * i as usize)));
}

#[test]
fn given_a_fill_that_never_drops_when_the_stall_limit_has_not_passed_then_pacing_stays_on_audio() {
    // 14 frames, 221 ms.
    assert!(feeds_after(&frame_starts(14, |i| 2_400 + 800 * i as usize)));
}

#[test]
fn given_a_fill_that_keeps_dropping_when_a_long_run_goes_by_then_pacing_stays_on_audio() {
    assert!(feeds_after(
        &frame_starts(200, |i| 200_000 - 500 * i as usize)
    ));
}

#[test]
fn given_a_fill_that_dips_every_other_frame_when_a_long_run_goes_by_then_pacing_stays_on_audio() {
    // The steady state: the device drains a little between pushes.
    assert!(feeds_after(
        &frame_starts(200, |i| 2_400 + 800 * (i % 2) as usize)
    ));
}

#[test]
fn given_a_full_ring_that_nobody_drains_when_the_limit_passes_then_pacing_falls_back() {
    assert!(!feeds_after(&frame_starts(20, |_| 12_000)));
}

/// A pacing that has fallen back because the failed flag was set, and the flag.
fn failed_pacing() -> (Pacing<impl Fn() -> usize>, Arc<AtomicBool>, Instant) {
    let ring = Rc::new(Cell::new(TARGET + 480));
    let failed = Arc::new(AtomicBool::new(false));
    let start = Instant::now();
    let pacing = Pacing::audio(audio_pacer(&ring), Arc::clone(&failed), start);
    (pacing, failed, start)
}

#[test]
fn given_the_failed_flag_when_the_next_frame_starts_then_pacing_falls_back_to_frame_time() {
    let (mut pacing, failed, start) = failed_pacing();
    pacing.frame_started(start);
    let before = (pacing.feeds_audio(), pacing.remaining(start));
    failed.store(true, Ordering::Release);
    pacing.frame_started(start);
    assert_eq!(
        (before, pacing.feeds_audio(), pacing.remaining(start)),
        ((true, Duration::from_millis(10)), false, FRAME_TIME)
    );
}

#[test]
fn given_a_fallback_when_the_failed_flag_clears_then_pacing_stays_on_frame_time() {
    let (mut pacing, failed, start) = failed_pacing();
    failed.store(true, Ordering::Release);
    pacing.frame_started(start);
    failed.store(false, Ordering::Release);
    pacing.frame_started(start);
    assert!(!pacing.feeds_audio());
}

#[test]
fn given_frame_pacing_from_the_start_when_asked_then_it_waits_a_frame_and_feeds_no_audio() {
    let start = Instant::now();
    let mut pacing = Pacing::<fn() -> usize>::frame(start);
    pacing.frame_started(start);
    assert_eq!(
        (pacing.feeds_audio(), pacing.remaining(start)),
        (false, FRAME_TIME)
    );
}
