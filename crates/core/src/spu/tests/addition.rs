// SPDX-License-Identifier: Apache-2.0
//! The addition of channel 1 or 3 into channel 0 or 2 (SNDCAPxCNT bits 0 and 7), from
//! `docs/hw/sound-capture.md` ("The addition mode", "The two bugs"), worked out by hand.
//!
//! A channel at volume 127 (read as 128) and divider 0 gives a PCM16 sample `S` the 16.8 value
//! `S << 8`; at panning 0 all of it is on the left mixer, at panning 127 (read as 128) all on the
//! right. A capture value is that, as PCM16, so `S` again.

use super::capture::{
    LEFT, RIGHT, SOURCE_CHANNEL, capturing, plan, running, samples, value, with_channels,
};
use super::*;

const ADD: u8 = CAP_ADD;
/// SOUNDCNT bits 12 and 13: channel 1 and channel 3 are not sent to the mixer.
const CH1_NOT_MIXED: u32 = 1 << 12;
const CH3_NOT_MIXED: u32 = 1 << 13;
/// A time at which every capture of these tests is running.
const NOW: u64 = 1_000;

/// A device whose channel 0 is started (a loop, busy for ever) at the panning `a`, whose channel 1
/// has the registers `b`, and whose capture 0 runs with `cnt`.
fn adding(a: u32, b: u32, cnt: u8) -> Spu {
    let mut spu = started(0, start(LOOP, PCM8) | a, 0, 4, 0);
    write(&mut spu, ch(1, CNT_OFFSET), Width::Word, b, 0);
    running(&mut spu, 0, cnt);
    spu
}

/// The left and right mixer sums at [`NOW`] for the channel samples `all`.
fn sums(spu: &Spu, all: &[i32; CHANNELS]) -> [i64; 2] {
    spu.mix(all, spu.adding(NOW)).sum
}

#[test]
fn given_addition_off_when_channel_b_is_on_the_right_then_it_stays_on_the_right_of_the_mixer() {
    let spu = adding(LEFT, RIGHT, 0);
    assert_eq!(
        sums(&spu, &samples(&[(0, 1000), (1, 500)])),
        [1000 << 8, 500 << 8]
    );
}

#[test]
fn given_addition_on_when_channel_a_is_busy_then_channel_b_joins_channel_a_on_channel_as_side() {
    let spu = adding(LEFT, RIGHT, ADD);
    assert_eq!(sums(&spu, &samples(&[(0, 1000), (1, 500)])), [1500 << 8, 0]);
}

#[test]
fn given_addition_on_when_channel_a_is_not_busy_then_channel_b_is_muted() {
    // Channel 0 is not started: ch(b) is muted for the mixer.
    let mut spu = with_channels(&[(0, LEFT), (1, LEFT)]);
    running(&mut spu, 0, ADD);
    assert_eq!(sums(&spu, &samples(&[(0, 0), (1, 500)])), [0, 0]);
}

#[test]
fn given_addition_on_when_soundcnt_says_channel_b_is_not_to_the_mixer_then_it_is_muted() {
    let mut spu = adding(LEFT, LEFT, ADD);
    write(&mut spu, SOUNDCNT_OFFSET, Width::Half, CH1_NOT_MIXED, 0);
    assert_eq!(sums(&spu, &samples(&[(0, 1000), (1, 500)])), [1000 << 8, 0]);
}

#[test]
fn given_addition_on_for_capture_1_when_soundcnt_bit_13_is_set_then_channel_3_is_muted() {
    let mut spu = started(2, start(LOOP, PCM8) | RIGHT, 0, 4, 0);
    write(&mut spu, ch(3, CNT_OFFSET), Width::Word, RIGHT, 0);
    write(&mut spu, SOUNDCNT_OFFSET, Width::Half, CH3_NOT_MIXED, 0);
    running(&mut spu, 1, ADD);
    assert_eq!(sums(&spu, &samples(&[(2, 1000), (3, 500)])), [0, 1000 << 8]);
}

#[test]
fn given_the_add_bit_with_bit_7_clear_when_mixed_then_addition_is_ignored() {
    // The capture is stopped (bit 7 clear): ch(b) stays on its own side.
    let mut spu = started(0, start(LOOP, PCM8) | LEFT, 0, 4, 0);
    write(&mut spu, ch(1, CNT_OFFSET), Width::Word, RIGHT, 0);
    write(&mut spu, SNDCAP0CNT_OFFSET, Width::Byte, u32::from(ADD), 0);
    assert_eq!(
        sums(&spu, &samples(&[(0, 1000), (1, 500)])),
        [1000 << 8, 500 << 8]
    );
}

#[test]
fn given_addition_on_when_the_other_capture_reads_the_mixer_then_it_sees_the_changed_mix() {
    // Capture 0 adds channel 1 (on the right) into channel 0 (left); capture 1 reads the right
    // mixer, which loses channel 1.
    let mut spu = adding(LEFT, RIGHT, ADD);
    running(&mut spu, 1, 0);
    let all = samples(&[(0, 1000), (1, 500)]);
    assert_eq!(
        (value(&spu, &all, 1), value(&spu, &all, 0)),
        (Some(0), Some(1500))
    );
}

#[test]
fn given_addition_off_when_the_other_capture_reads_the_mixer_then_it_keeps_channel_b() {
    let mut spu = adding(LEFT, RIGHT, 0);
    running(&mut spu, 1, 0);
    let all = samples(&[(0, 1000), (1, 500)]);
    assert_eq!(value(&spu, &all, 1), Some(500));
}

#[test]
fn given_the_channel_source_with_addition_when_both_are_busy_then_the_capture_is_a_plus_b() {
    let spu = adding(LEFT, LEFT, ADD | SOURCE_CHANNEL as u8);
    assert_eq!(value(&spu, &samples(&[(0, 1000), (1, 500)]), 0), Some(1500));
}

#[test]
fn given_the_channel_source_with_addition_when_channel_a_is_off_then_the_capture_is_plain_channel_b()
 {
    // Channel 0 is not started and SOUNDCNT bit 12 says "not to the mixer": the capture adds anyway.
    let mut spu = with_channels(&[(0, LEFT), (1, LEFT)]);
    write(&mut spu, SOUNDCNT_OFFSET, Width::Half, CH1_NOT_MIXED, 0);
    running(&mut spu, 0, ADD | SOURCE_CHANNEL as u8);
    assert_eq!(value(&spu, &samples(&[(0, 0), (1, 700)]), 0), Some(700));
}

/// The cases `(a, b, wanted)` whose PCM16 capture value, with addition on and the channel source,
/// is not the wanted one: the sum is ANDed with `FFFFh` instead of clipped.
fn overflow_failures() -> Vec<(i32, i32, Option<i32>)> {
    let spu = adding(LEFT, LEFT, ADD | SOURCE_CHANNEL as u8);
    [
        (30000, 10000, Some(-25536)),
        (-30000, -10000, Some(25536)),
        (32767, 1, Some(-32768)),
        (-32768, -1, Some(32767)),
        (-100, -50, Some(-150)),
        (20000, 10000, Some(30000)),
    ]
    .into_iter()
    .filter(|&(a, b, want)| value(&spu, &samples(&[(0, a), (1, b)]), 0) != want)
    .collect()
}

#[test]
fn given_the_channel_source_with_addition_when_the_sum_leaves_16_bits_then_it_wraps_and_loses_its_sign()
 {
    assert_eq!(overflow_failures(), Vec::new());
}

#[test]
fn given_the_channel_source_with_addition_when_both_are_negative_then_the_both_negative_bug_does_not_apply()
 {
    let spu = adding(LEFT, LEFT, ADD | SOURCE_CHANNEL as u8);
    assert_eq!(value(&spu, &samples(&[(0, -100), (1, -50)]), 0), Some(-150));
}

#[test]
fn given_a_pcm8_channel_capture_with_addition_when_the_sum_overflows_then_the_integer_part_wraps() {
    // 40000 wraps to -25536 PCM16 steps, which is -99.75 PCM8 steps: the floor is -100 and the
    // fraction's MSB is clear, so no rounding up.
    let spu = adding(LEFT, LEFT, ADD | SOURCE_CHANNEL as u8 | CAP_PCM8);
    assert_eq!(
        value(&spu, &samples(&[(0, 30000), (1, 10000)]), 0),
        Some(-100)
    );
}

#[test]
fn given_addition_on_when_the_capture_ends_then_the_mixer_goes_back_to_the_plain_channels() {
    // A one-shot of one word at 64 cycles a sample ends at 128: after it, ch(b) is on its own side.
    let mut spu = started(0, start(LOOP, PCM8) | LEFT, 0, 4, 0);
    write(&mut spu, ch(1, CNT_OFFSET), Width::Word, RIGHT, 0);
    capturing(&mut spu, 0, ADD | CAP_ONE_SHOT, 1, 0x0200_0000, 0);
    let all = samples(&[(0, 1000), (1, 500)]);
    let during = spu.mix(&all, spu.adding(127)).sum;
    let after = spu.mix(&all, spu.adding(128)).sum;
    assert_eq!((during, after), ([1500 << 8, 0], [1000 << 8, 500 << 8]));
}

#[test]
fn given_a_capture_with_addition_when_written_then_the_capture_still_writes_its_slots() {
    let spu = adding(LEFT, LEFT, ADD | SOURCE_CHANNEL as u8);
    let all = samples(&[(0, 1000), (1, 500)]);
    assert_eq!(plan(&spu, &all, 2048, 0).map(|p| p.count), Some(32));
}
