// SPDX-License-Identifier: Apache-2.0
//! Tests of `pacing`.

use super::*;

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
