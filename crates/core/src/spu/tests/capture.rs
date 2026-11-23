// SPDX-License-Identifier: Apache-2.0
//! What a capture writes and when, from `docs/hw/sound-capture.md`, worked out by hand.
//!
//! With TMR `FFF0h` a capture sample takes `4 * 16 = 64` master cycles. A channel at volume 127
//! (read as 128) with the divider 0 gives a PCM16 sample `S` the 16.8 value `S << 8`, and at panning
//! 0 puts all of it on the left mixer, at panning 127 (read as 128) all of it on the right.

use super::*;

const LEFT: u32 = 0x7F;
const RIGHT: u32 = 0x7F | 127 << 16;
const SOURCE_CHANNEL: u32 = 1 << 1;

/// A device whose channels `volumes` carry `(channel, panning bits)` at full volume.
fn with_channels(channels: &[(u32, u32)]) -> Spu {
    let mut spu = Spu::new();
    channels
        .iter()
        .for_each(|&(x, cnt)| write(&mut spu, ch(x, CNT_OFFSET), Width::Word, cnt, 0));
    spu
}

fn samples(values: &[(usize, i32)]) -> [i32; CHANNELS] {
    let mut all = [0; CHANNELS];
    values.iter().for_each(|&(x, v)| all[x] = v);
    all
}

/// Capture `x` started at `now` with `cnt`, `len` words, `dad`, at TMR `FFF0h` on its channel.
fn capturing(spu: &mut Spu, x: u32, cnt: u8, len: u32, dad: u32, now: u64) {
    let (timer, base) = (ch(2 * x + 1, TMR_OFFSET), SNDCAP0DAD_OFFSET + 8 * x);
    write(spu, timer, Width::Half, TMR, now);
    write(spu, base, Width::Word, dad, now);
    write(spu, base + 4, Width::Half, len, now);
    write(
        spu,
        SNDCAP0CNT_OFFSET + x,
        Width::Byte,
        u32::from(cnt | CAP_START),
        now,
    );
}

/// The plan of capture `x` for the output sample at `time`, with `all` as the channels' samples.
fn plan(spu: &Spu, all: &[i32; CHANNELS], time: u64, x: usize) -> Option<CaptureWrites> {
    spu.capture_writes(&spu.mix(all), all, time)[x]
}

/// The value (PCM16 of the 16.8 source) capture `x` writes at `time`.
fn value(spu: &Spu, all: &[i32; CHANNELS], x: usize) -> Option<i32> {
    plan(spu, all, 1_000_000, x).map(|p| p.value)
}

/// A capture of the given source with a long buffer, started at time 0.
fn running(spu: &mut Spu, x: u32, cnt: u8) {
    capturing(spu, x, cnt, 100, 0x0200_0000, 0);
}

#[test]
fn given_the_mixer_source_when_capture_0_writes_then_the_value_is_the_left_mixer_sum() {
    // Channels 0 and 4 on the left (1000 + 300), channel 2 on the right.
    let mut spu = with_channels(&[(0, LEFT), (4, LEFT), (2, RIGHT)]);
    running(&mut spu, 0, 0);
    let all = samples(&[(0, 1000), (4, 300), (2, -500)]);
    assert_eq!(value(&spu, &all, 0), Some(1300));
}

#[test]
fn given_the_mixer_source_when_capture_1_writes_then_the_value_is_the_right_mixer_sum() {
    let mut spu = with_channels(&[(0, LEFT), (2, RIGHT), (5, RIGHT)]);
    running(&mut spu, 1, 0);
    let all = samples(&[(0, 1000), (2, -500), (5, 200)]);
    assert_eq!(value(&spu, &all, 1), Some(-300));
}

#[test]
fn given_the_channel_source_when_each_capture_writes_then_the_value_is_channel_a_alone() {
    // Capture 0 reads channel 0 and capture 1 channel 2, not the mixer (which also holds channel 4).
    let mut spu = with_channels(&[(0, LEFT), (2, RIGHT), (4, LEFT)]);
    running(&mut spu, 0, SOURCE_CHANNEL as u8);
    running(&mut spu, 1, SOURCE_CHANNEL as u8);
    let all = samples(&[(0, 1000), (2, -500), (4, 300)]);
    assert_eq!(
        (value(&spu, &all, 0), value(&spu, &all, 1)),
        (Some(1000), Some(-500))
    );
}

#[test]
fn given_the_channel_source_when_channel_a_has_volume_and_divider_then_they_apply_before_panning() {
    // Divider 1 (half) and volume 64 (half): a quarter, whatever the panning.
    let cnt = 64 | 1 << 8 | 30 << 16;
    let mut spu = with_channels(&[(0, cnt)]);
    running(&mut spu, 0, SOURCE_CHANNEL as u8);
    assert_eq!(value(&spu, &samples(&[(0, 4000)]), 0), Some(1000));
}

#[test]
fn given_a_mixer_sum_beyond_16_bits_when_captured_then_it_clips_at_both_ends() {
    let mut spu = with_channels(&[(0, LEFT), (4, LEFT), (6, RIGHT), (7, RIGHT)]);
    running(&mut spu, 0, 0);
    running(&mut spu, 1, 0);
    let all = samples(&[(0, 32767), (4, 32767), (6, -32768), (7, -32768)]);
    assert_eq!(
        (value(&spu, &all, 0), value(&spu, &all, 1)),
        (Some(0x7FFF), Some(-0x8000))
    );
}

#[test]
fn given_a_pcm8_capture_when_the_mixer_value_has_a_set_fraction_msb_then_the_byte_rounds_toward_zero()
 {
    // Left sum 5.5 PCM8 steps: 5.5 * 256 PCM16 steps = 1408 -> 8.16 `5_8000h`.
    let mut spu = with_channels(&[(0, LEFT)]);
    running(&mut spu, 0, CAP_PCM8);
    assert_eq!(value(&spu, &samples(&[(0, 1408)]), 0), Some(5));
}

#[test]
fn given_a_pcm8_capture_when_the_mixer_value_is_negative_with_a_set_fraction_msb_then_it_rounds_up()
{
    let mut spu = with_channels(&[(0, LEFT)]);
    running(&mut spu, 0, CAP_PCM8);
    assert_eq!(value(&spu, &samples(&[(0, -1408)]), 0), Some(-5));
}

#[test]
fn given_a_pcm8_capture_when_the_mixer_value_is_negative_with_a_clear_fraction_msb_then_it_floors()
{
    // -5.75 steps: -1472.
    let mut spu = with_channels(&[(0, LEFT)]);
    running(&mut spu, 0, CAP_PCM8);
    assert_eq!(value(&spu, &samples(&[(0, -1472)]), 0), Some(-6));
}

/// The cases `(channel a, channel b, wanted)` whose captured value is not the wanted one, with
/// addition off: the both-negative bug writes `-8000h`, anything else plain `ch(a)`.
fn bug_failures() -> Vec<(i32, i32, Option<i32>)> {
    let mut spu = with_channels(&[(0, LEFT), (1, LEFT)]);
    running(&mut spu, 0, SOURCE_CHANNEL as u8);
    [
        (-100, -50, Some(-0x8000)),
        (-100, 50, Some(-100)),
        (100, -50, Some(100)),
        (100, 50, Some(100)),
        (-100, 0, Some(-100)),
        (0, -50, Some(0)),
    ]
    .into_iter()
    .filter(|&(a, b, want)| value(&spu, &samples(&[(0, a), (1, b)]), 0) != want)
    .collect()
}

#[test]
fn given_the_channel_source_without_addition_when_both_channels_are_negative_then_it_writes_minus_8000h()
 {
    assert_eq!(bug_failures(), Vec::new());
}

#[test]
fn given_the_mixer_source_when_both_channels_are_negative_then_the_bug_does_not_apply() {
    let mut spu = with_channels(&[(0, LEFT), (1, LEFT)]);
    running(&mut spu, 0, 0);
    assert_eq!(value(&spu, &samples(&[(0, -100), (1, -50)]), 0), Some(-150));
}

#[test]
fn given_a_stopped_capture_when_an_output_sample_comes_then_it_has_nothing_to_write() {
    let spu = Spu::new();
    assert!(plan(&spu, &samples(&[]), 1_000_000, 0).is_none());
}

#[test]
fn given_a_busy_capture_when_a_zero_is_written_to_bit_7_then_it_writes_nothing_more() {
    let mut spu = Spu::new();
    running(&mut spu, 0, 0);
    write(&mut spu, SNDCAP0CNT_OFFSET, Width::Byte, 0, 500);
    assert!(plan(&spu, &samples(&[]), 1_000_000, 0).is_none());
}

#[test]
fn given_a_capture_started_at_the_start_when_an_event_comes_then_the_completed_samples_are_written()
{
    // 2048 cycles after a start at 0 is 32 samples of 64 cycles: slots 0..32.
    let mut spu = Spu::new();
    running(&mut spu, 0, 0);
    let p = plan(&spu, &samples(&[]), 2048, 0).map(|p| (p.first, p.count));
    assert_eq!(p, Some((0, 32)));
}

#[test]
fn given_two_events_in_a_row_when_each_writes_then_the_slots_continue_without_a_gap() {
    let mut spu = Spu::new();
    running(&mut spu, 0, 0);
    let all = samples(&[]);
    let ranges = [2048, 4096].map(|t| plan(&spu, &all, t, 0).map(|p| (p.first, p.count)));
    assert_eq!(ranges, [Some((0, 32)), Some((32, 32))]);
}

#[test]
fn given_a_capture_started_between_events_when_the_event_comes_then_only_the_slots_since_start_count()
 {
    // Started at 1,024: 1,024 cycles of the 2,048 are 16 samples.
    let mut spu = Spu::new();
    capturing(&mut spu, 0, 0, 100, 0x0200_0000, 1_024);
    let p = plan(&spu, &samples(&[]), 2048, 0).map(|p| (p.first, p.count));
    assert_eq!(p, Some((0, 16)));
}

#[test]
fn given_a_capture_started_after_the_event_time_when_the_event_comes_then_nothing_is_written() {
    // The ARM7 can write at a time past the master clock.
    let mut spu = Spu::new();
    capturing(&mut spu, 0, 0, 100, 0x0200_0000, 5_000);
    assert!(plan(&spu, &samples(&[]), 2048, 0).is_none());
}

#[test]
fn given_a_loop_capture_when_the_slots_pass_the_buffer_then_the_addresses_wrap_to_dad() {
    // 2 words of PCM16: 4 samples at DAD, +2, +4, +6, and slot 4 back at DAD.
    let mut spu = Spu::new();
    capturing(&mut spu, 0, 0, 2, 0x0200_0100, 0);
    let p = plan(&spu, &samples(&[]), 2048, 0).expect("running");
    let addresses = [0, 1, 2, 3, 4, 5].map(|slot| p.address(slot));
    assert_eq!(
        addresses,
        [
            0x0200_0100,
            0x0200_0102,
            0x0200_0104,
            0x0200_0106,
            0x0200_0100,
            0x0200_0102
        ]
    );
}

#[test]
fn given_a_pcm8_loop_capture_when_the_slots_pass_the_buffer_then_each_slot_is_a_byte_and_wraps() {
    // 1 word of PCM8: 4 bytes.
    let mut spu = Spu::new();
    capturing(&mut spu, 0, CAP_PCM8, 1, 0x0200_0100, 0);
    let p = plan(&spu, &samples(&[]), 2048, 0).expect("running");
    let addresses = [0, 3, 4, 5].map(|slot| p.address(slot));
    assert_eq!(
        addresses,
        [0x0200_0100, 0x0200_0103, 0x0200_0100, 0x0200_0101]
    );
}

#[test]
fn given_a_one_shot_capture_when_it_ends_then_the_last_event_writes_the_rest_and_busy_clears() {
    // 1 word of PCM16 is 2 samples, 128 cycles: the event at 2048 writes slots 0..2 only.
    let mut spu = Spu::new();
    capturing(&mut spu, 0, CAP_ONE_SHOT, 1, 0x0200_0000, 0);
    let p = plan(&spu, &samples(&[]), 2048, 0).map(|p| (p.first, p.count));
    assert_eq!(
        (p, cap_busy(&spu, 0, 127), cap_busy(&spu, 0, 128)),
        (Some((0, 2)), true, false)
    );
}

#[test]
fn given_a_finished_one_shot_when_a_later_event_comes_then_nothing_more_is_written() {
    let mut spu = Spu::new();
    capturing(&mut spu, 0, CAP_ONE_SHOT, 1, 0x0200_0000, 0);
    assert!(plan(&spu, &samples(&[]), 4096, 0).is_none());
}

#[test]
fn given_a_capture_of_zero_words_when_it_runs_then_it_holds_one_word_of_two_samples() {
    let mut spu = Spu::new();
    capturing(&mut spu, 0, 0, 0, 0x0200_0000, 0);
    let p = plan(&spu, &samples(&[]), 2048, 0).expect("running");
    assert_eq!((p.address(2), p.address(1)), (0x0200_0000, 0x0200_0002));
}

#[test]
fn given_capture_0_and_capture_1_when_their_channels_have_different_timers_then_each_uses_its_own()
{
    // Channel 1 at TMR 0xFFF0 (64 cycles), channel 3 at TMR 0xFFE0 (128 cycles).
    let mut spu = Spu::new();
    running(&mut spu, 0, 0);
    write(&mut spu, ch(3, TMR_OFFSET), Width::Half, 0xFFE0, 0);
    write(&mut spu, SNDCAP1DAD_OFFSET, Width::Word, 0x0200_0000, 0);
    write(&mut spu, SNDCAP1LEN_OFFSET, Width::Half, 100, 0);
    write(
        &mut spu,
        SNDCAP1CNT_OFFSET,
        Width::Byte,
        u32::from(CAP_START),
        0,
    );
    let all = samples(&[]);
    let counts = [0, 1].map(|x| plan(&spu, &all, 2048, x).map(|p| p.count));
    assert_eq!(counts, [Some(32), Some(16)]);
}

#[test]
fn given_a_busy_capture_when_its_timer_or_length_is_written_then_the_running_capture_keeps_them() {
    let mut spu = Spu::new();
    capturing(&mut spu, 0, 0, 2, 0x0200_0100, 0);
    write(&mut spu, ch(1, TMR_OFFSET), Width::Half, 0, 10);
    write(&mut spu, SNDCAP0LEN_OFFSET, Width::Half, 50, 10);
    write(&mut spu, SNDCAP0DAD_OFFSET, Width::Word, 0x0200_9000, 10);
    let p = plan(&spu, &samples(&[]), 2048, 0).expect("running");
    assert_eq!((p.count, p.address(4)), (32, 0x0200_0100));
}

#[test]
fn given_a_running_capture_when_inspected_then_the_position_and_next_address_follow_the_clock() {
    // 2 words of PCM16 at 64 cycles a sample: at 192 cycles three samples are written.
    let mut spu = Spu::new();
    capturing(&mut spu, 0, 0, 2, 0x0200_0100, 0);
    let capture = spu.capture(0);
    assert_eq!(
        (
            capture.position(192),
            capture.next_address(192),
            capture.position(256)
        ),
        (3, 0x0200_0106, 0)
    );
}

#[test]
fn given_a_stopped_capture_when_inspected_then_the_position_is_zero_and_the_address_is_dad() {
    let mut spu = Spu::new();
    write(&mut spu, SNDCAP0DAD_OFFSET, Width::Word, 0x0200_0100, 0);
    let capture = spu.capture(0);
    assert_eq!(
        (capture.position(500), capture.next_address(500)),
        (0, 0x0200_0100)
    );
}

#[test]
fn given_a_started_capture_when_the_device_round_trips_through_postcard_then_it_decodes_equal() {
    let mut spu = Spu::new();
    capturing(&mut spu, 0, CAP_ONE_SHOT, 3, 0x0200_0100, 700);
    let bytes = postcard::to_allocvec(&spu).expect("plain data encodes");
    let back = postcard::from_bytes::<Spu>(&bytes);
    assert_eq!(back, Ok(spu));
}
