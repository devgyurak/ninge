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

/// A fake ring: what the pacer reads of it.
type Ring = Rc<Cell<Level>>;

fn ring(fill: usize, pulled: u64) -> Ring {
    Rc::new(Cell::new(Level { fill, pulled }))
}

/// An audio pacer whose ring is whatever `ring` holds.
fn audio_pacer(ring: &Ring) -> AudioPacer<impl Fn() -> Level + use<>> {
    let ring = Rc::clone(ring);
    AudioPacer::new(move || ring.get(), TARGET, DEVICE_HZ)
}

fn wait_at_fill(fill: usize) -> Duration {
    audio_pacer(&ring(fill, 0)).remaining(Instant::now())
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
    assert_eq!(
        (
            audio_pacer(&ring(0, 0)).feeds_audio(),
            FramePacer::new(Instant::now()).feeds_audio()
        ),
        (true, false)
    );
}

/// Whether a [`Pacing`] still feeds audio after frames start at the `(milliseconds, pulled)`
/// steps, the ring holding a steady `fill` throughout.
fn feeds_after(fill: usize, steps: &[(u64, u64)]) -> bool {
    let ring = ring(fill, 0);
    let start = Instant::now();
    let mut pacing = Pacing::audio(audio_pacer(&ring), Arc::new(AtomicBool::new(false)), start);
    steps.iter().for_each(|&(ms, pulled)| {
        ring.set(Level { fill, pulled });
        pacing.frame_started(start + Duration::from_millis(ms));
    });
    pacing.feeds_audio()
}

/// One frame start every 17 ms for `frames` frames; the device has pulled `pulled(i)` by frame `i`.
fn frame_starts(frames: u64, pulled: impl Fn(u64) -> u64) -> Vec<(u64, u64)> {
    (0..frames).map(|i| (i * 17, pulled(i))).collect()
}

#[test]
fn given_a_device_that_pulls_nothing_when_frames_run_past_the_stall_limit_then_pacing_falls_back() {
    // 20 frames, 323 ms.
    assert!(!feeds_after(12_000, &frame_starts(20, |_| 5)));
}

#[test]
fn given_a_device_that_pulls_nothing_when_the_stall_limit_has_not_passed_then_pacing_stays_on_audio()
 {
    // 14 frames, 221 ms.
    assert!(feeds_after(12_000, &frame_starts(14, |_| 5)));
}

#[test]
fn given_a_device_that_keeps_pulling_when_a_long_run_goes_by_then_pacing_stays_on_audio() {
    assert!(feeds_after(2_400, &frame_starts(200, |i| 800 * i)));
}

#[test]
fn given_an_emulator_slower_than_real_time_when_the_fill_stays_put_but_the_device_pulls_then_pacing_stays_on_audio()
 {
    // The fill is the same at every frame start, as in a debug build; the device keeps calling.
    assert!(feeds_after(2_400, &frame_starts(100, |i| 900 * i)));
}

#[test]
fn given_one_long_pause_between_two_frame_starts_when_the_device_pulled_meanwhile_then_pacing_stays_on_audio()
 {
    // A laptop waking from sleep: 20 s between two frame starts, and the device ran on.
    assert!(feeds_after(
        2_400,
        &[(0, 0), (17, 800), (20_017, 960_800), (20_034, 961_600)]
    ));
}

#[test]
fn given_one_long_pause_when_the_device_has_pulled_nothing_since_then_a_single_frame_start_does_not_condemn_it()
 {
    // Two frame starts without a pull are fewer than STALL_FRAMES, however far apart.
    assert!(feeds_after(2_400, &[(0, 5), (20_000, 5), (20_017, 5)]));
}

#[test]
fn given_a_device_that_stopped_after_running_when_enough_frame_starts_pass_then_pacing_falls_back()
{
    let mut steps = frame_starts(10, |i| 800 * i);
    steps.extend((10..30).map(|i| (i * 17, 7_200)));
    assert!(!feeds_after(2_400, &steps));
}

/// A pacing that follows a fake ring, and the failed flag.
fn failed_pacing() -> (Pacing<impl Fn() -> Level>, Arc<AtomicBool>, Instant) {
    let ring = ring(TARGET + 480, 0);
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
    let mut pacing = Pacing::<fn() -> Level>::frame(start);
    pacing.frame_started(start);
    assert_eq!(
        (pacing.feeds_audio(), pacing.remaining(start)),
        (false, FRAME_TIME)
    );
}
