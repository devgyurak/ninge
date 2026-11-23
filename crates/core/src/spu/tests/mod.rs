// SPDX-License-Identifier: Apache-2.0
//! The sound registers as a device. Expected values come from `docs/hw/sound-registers.md`
//! ("Registers", "The status bit of a channel", "Model"), worked out by hand, never from running
//! the code.
//!
//! With TMR `FFF0h` a sample takes `4 * 16 = 64` master cycles, so a one-shot of PNT + LEN = 4
//! words lasts 16 PCM8 samples (1,024 cycles), 8 PCM16 samples (512), and, with the header word, 4
//! words of ADPCM are `8 * 3 = 24` samples (1,536).

use super::*;
use crate::io::device::cx_at;

mod capture;

/// TMR `FFF0h`: 16 steps of 4 master cycles.
const TMR: u32 = 0xFFF0;
/// Channel `x`'s register `reg` in the device's offsets.
const fn ch(x: u32, reg: u32) -> u32 {
    (x << 4) + reg
}
/// SOUNDxCNT with the given repeat mode and format and the start bit set.
const fn start(repeat: u32, format: u32) -> u32 {
    CNT_START | repeat << 27 | format << 29
}
const LOOP: u32 = 1;
const ONE_SHOT: u32 = 2;
const PCM8: u32 = 0;
const PCM16: u32 = 1;
const ADPCM: u32 = 2;
const PSG: u32 = 3;

fn write(spu: &mut Spu, offset: u32, width: Width, value: u32, now: u64) {
    spu.write(offset, width, value, cx_at(now));
}

/// `value` written at each of `offsets` as a `width` access at time 0.
fn write_each(spu: &mut Spu, offsets: &[u32], width: Width, value: u32) {
    offsets.iter().for_each(|&o| write(spu, o, width, value, 0));
}

fn read(spu: &Spu, offset: u32, width: Width, now: u64) -> u32 {
    spu.peek(offset, width, cx_at(now))
}

/// A device with channel `x` set to `tmr`, `pnt`, `len` and then started with `cnt` at `now`.
fn started(x: u32, cnt: u32, pnt: u32, len: u32, now: u64) -> Spu {
    let mut spu = Spu::new();
    write(&mut spu, ch(x, TMR_OFFSET), Width::Half, TMR, now);
    write(&mut spu, ch(x, PNT_OFFSET), Width::Half, pnt, now);
    write(&mut spu, ch(x, LEN_OFFSET), Width::Word, len, now);
    write(&mut spu, ch(x, CNT_OFFSET), Width::Word, cnt, now);
    spu
}

/// Whether channel `x`'s busy bit reads set at `now`.
fn busy(spu: &Spu, x: u32, now: u64) -> bool {
    read(spu, ch(x, CNT_OFFSET), Width::Word, now) & CNT_START != 0
}

/// Whether capture `x`'s busy bit reads set at `now`.
fn cap_busy(spu: &Spu, x: u32, now: u64) -> bool {
    let offset = SNDCAP0CNT_OFFSET + x;
    read(spu, offset, Width::Byte, now) & u32::from(CAP_START) != 0
}

/// Whether a started channel reads (busy just before `end`, busy at `end`).
fn busy_around(spu: &Spu, x: u32, end: u64) -> (bool, bool) {
    (busy(spu, x, end - 1), busy(spu, x, end))
}

/// The formats (name, format, PNT + LEN) whose one-shot busy bit does not read (set, clear) around
/// the end the note gives.
fn one_shot_failures() -> Vec<(&'static str, (bool, bool))> {
    [
        ("pcm8", PCM8, 1_024),
        ("pcm16", PCM16, 512),
        ("adpcm", ADPCM, 1_536),
    ]
    .iter()
    .map(|&(name, format, ticks)| {
        let spu = started(0, start(ONE_SHOT, format), 0, 4, 1_000);
        (name, busy_around(&spu, 0, 1_000 + ticks))
    })
    .filter(|&(_, around)| around != (true, false))
    .collect()
}

#[test]
fn given_a_one_shot_of_each_format_when_the_end_time_is_reached_then_the_busy_bit_clears_exactly_there()
 {
    assert_eq!(one_shot_failures(), Vec::new());
}

#[test]
fn given_a_slower_timer_when_a_pcm8_one_shot_runs_then_a_sample_takes_four_cycles_per_step() {
    // TMR 0: 4 * 10000h = 262,144 cycles per sample; 4 words of PCM8 are 16 samples.
    let mut spu = Spu::new();
    write(&mut spu, ch(2, LEN_OFFSET), Width::Word, 4, 0);
    write(
        &mut spu,
        ch(2, CNT_OFFSET),
        Width::Word,
        start(ONE_SHOT, PCM8),
        0,
    );
    assert_eq!(busy_around(&spu, 2, 16 * 262_144), (true, false));
}

#[test]
fn given_a_one_shot_with_pnt_and_len_when_started_then_the_sum_sets_the_length() {
    // PNT 1 + LEN 3 = 4 words of PCM8: the same 16 samples as LEN 4 alone.
    let spu = started(0, start(ONE_SHOT, PCM8), 1, 3, 0);
    assert_eq!(busy_around(&spu, 0, 1_024), (true, false));
}

#[test]
fn given_a_one_shot_shorter_than_four_words_when_much_time_passes_then_it_stays_busy() {
    let spu = started(0, start(ONE_SHOT, PCM8), 1, 2, 0);
    assert!(busy(&spu, 0, 1 << 60));
}

#[test]
fn given_a_loop_channel_when_much_time_passes_then_it_stays_busy() {
    let spu = started(0, start(LOOP, PCM8), 0, 4, 0);
    assert!(busy(&spu, 0, 1 << 60));
}

#[test]
fn given_a_psg_channel_when_much_time_passes_then_it_stays_busy() {
    let spu = started(8, start(ONE_SHOT, PSG), 0, 4, 0);
    assert!(busy(&spu, 8, 1 << 60));
}

#[test]
fn given_the_manual_and_prohibited_repeat_modes_when_much_time_passes_then_both_stay_busy() {
    let modes = [0, 3].map(|mode| {
        let spu = started(0, start(mode, PCM8), 0, 4, 0);
        busy(&spu, 0, 1 << 60)
    });
    assert_eq!(modes, [true, true]);
}

#[test]
fn given_a_busy_channel_when_a_zero_is_written_to_bit_31_then_it_stops_at_once() {
    let mut spu = started(0, start(LOOP, PCM8), 0, 4, 0);
    write(&mut spu, ch(0, CNT_OFFSET + 3), Width::Byte, 0, 10);
    assert_eq!((busy(&spu, 0, 10), spu.channel(0).end()), (false, 0));
}

#[test]
fn given_a_busy_channel_when_start_is_written_again_then_the_end_does_not_move() {
    let mut spu = started(0, start(ONE_SHOT, PCM8), 0, 4, 0);
    write(
        &mut spu,
        ch(0, CNT_OFFSET),
        Width::Word,
        start(ONE_SHOT, PCM8),
        500,
    );
    assert_eq!(spu.channel(0).end(), 1_024);
}

#[test]
fn given_an_ended_one_shot_when_start_is_written_again_then_it_runs_again_from_that_time() {
    let mut spu = started(0, start(ONE_SHOT, PCM8), 0, 4, 0);
    write(
        &mut spu,
        ch(0, CNT_OFFSET),
        Width::Word,
        start(ONE_SHOT, PCM8),
        2_000,
    );
    assert_eq!(busy_around(&spu, 0, 2_000 + 1_024), (true, false));
}

#[test]
fn given_an_ended_one_shot_when_a_low_byte_is_written_then_the_stale_start_bit_is_folded_away() {
    let mut spu = started(0, start(ONE_SHOT, PCM8), 0, 4, 0);
    write(&mut spu, ch(0, CNT_OFFSET), Width::Byte, 0x40, 5_000);
    assert_eq!(spu.channel(0).cnt() & CNT_START, 0);
}

#[test]
fn given_a_busy_channel_when_tmr_is_written_then_the_end_is_not_recomputed() {
    let mut spu = started(0, start(ONE_SHOT, PCM8), 0, 4, 0);
    write(&mut spu, ch(0, TMR_OFFSET), Width::Half, 0, 1);
    assert_eq!(spu.channel(0).end(), 1_024);
}

/// The registers written with all ones, as (name, address in the device, width, what a read
/// returns, according to the note) rows that read differently.
fn read_back_failures() -> Vec<(&'static str, u32)> {
    let rows: [(&str, u32, Width, u32); 8] = [
        ("SOUNDxCNT", ch(5, CNT_OFFSET), Width::Word, 0xFF7F_837F),
        ("SOUNDCNT", SOUNDCNT_OFFSET, Width::Word, 0x0000_BF7F),
        ("SOUNDBIAS", SOUNDBIAS_OFFSET, Width::Word, 0x0000_03FF),
        ("SNDCAP0CNT", SNDCAP0CNT_OFFSET, Width::Byte, 0x8F),
        ("SNDCAP1CNT", SNDCAP1CNT_OFFSET, Width::Byte, 0x8F),
        ("SNDCAP0DAD", SNDCAP0DAD_OFFSET, Width::Word, 0x07FF_FFFC),
        ("SNDCAP1DAD", SNDCAP1DAD_OFFSET, Width::Word, 0x07FF_FFFC),
        ("SOUNDCNT bytes", SOUNDCNT_OFFSET, Width::Half, 0xBF7F),
    ];
    rows.iter()
        .filter_map(|&(name, offset, width, expected)| {
            let mut spu = Spu::new();
            write(&mut spu, offset, width, u32::MAX, 0);
            let actual = read(&spu, offset, width, 0);
            (actual != expected).then_some((name, actual))
        })
        .collect()
}

#[test]
fn given_all_ones_written_to_each_readable_register_when_read_back_then_only_its_bits_remain() {
    assert_eq!(read_back_failures(), Vec::new());
}

#[test]
fn given_all_ones_written_to_the_write_only_registers_when_read_then_each_reads_zero() {
    let offsets = [
        ch(3, SAD_OFFSET),
        ch(3, TMR_OFFSET),
        ch(3, PNT_OFFSET),
        ch(3, LEN_OFFSET),
        SNDCAP0LEN_OFFSET,
        SNDCAP1LEN_OFFSET,
    ];
    let mut spu = Spu::new();
    write_each(&mut spu, &offsets, Width::Word, u32::MAX);
    let values = offsets.map(|o| read(&spu, o, Width::Word, 0));
    assert_eq!(values, [0; 6]);
}

#[test]
fn given_all_ones_written_to_the_write_only_registers_when_inspected_then_the_masked_values_are_kept()
 {
    let mut spu = Spu::new();
    write(&mut spu, ch(3, SAD_OFFSET), Width::Word, u32::MAX, 0);
    write(&mut spu, ch(3, TMR_OFFSET), Width::Word, u32::MAX, 0);
    write(&mut spu, ch(3, LEN_OFFSET), Width::Word, u32::MAX, 0);
    write(&mut spu, SNDCAP1LEN_OFFSET, Width::Word, u32::MAX, 0);
    let c = spu.channel(3);
    assert_eq!(
        (
            c.sad(),
            c.tmr(),
            c.pnt(),
            c.length(),
            spu.capture(1).length()
        ),
        (0x07FF_FFFC, 0xFFFF, 0xFFFF, 0x003F_FFFF, 0xFFFF)
    );
}

#[test]
fn given_a_byte_written_into_a_wide_register_when_read_then_only_that_byte_changes() {
    let mut spu = Spu::new();
    write(&mut spu, SOUNDBIAS_OFFSET, Width::Half, 0x0200, 0);
    write(&mut spu, SOUNDBIAS_OFFSET, Width::Byte, 0x01, 0);
    assert_eq!(read(&spu, SOUNDBIAS_OFFSET, Width::Word, 0), 0x0201);
}

#[test]
fn given_unused_and_gap_addresses_when_written_and_read_then_they_read_zero() {
    let offsets = [0x10A, 0x10B, 0x10F, 0x120, 0x13F];
    let mut spu = Spu::new();
    write_each(&mut spu, &offsets, Width::Byte, 0xFF);
    assert_eq!(offsets.map(|o| read(&spu, o, Width::Byte, 0)), [0; 5]);
}

#[test]
fn given_a_capture_of_zero_words_when_one_shot_pcm16_then_it_lasts_one_word_at_channel_1s_rate() {
    // 1 word = 2 samples of 64 cycles.
    let mut spu = Spu::new();
    write(&mut spu, ch(1, TMR_OFFSET), Width::Half, TMR, 0);
    write(
        &mut spu,
        SNDCAP0CNT_OFFSET,
        Width::Byte,
        u32::from(CAP_START | CAP_ONE_SHOT),
        100,
    );
    assert_eq!(
        (cap_busy(&spu, 0, 227), cap_busy(&spu, 0, 228)),
        (true, false)
    );
}

#[test]
fn given_a_pcm8_capture_of_two_words_when_one_shot_then_it_lasts_eight_samples() {
    let mut spu = Spu::new();
    write(&mut spu, ch(1, TMR_OFFSET), Width::Half, TMR, 0);
    write(&mut spu, SNDCAP0LEN_OFFSET, Width::Half, 2, 0);
    let cnt = u32::from(CAP_START | CAP_ONE_SHOT | CAP_PCM8);
    write(&mut spu, SNDCAP0CNT_OFFSET, Width::Byte, cnt, 0);
    assert_eq!(
        (cap_busy(&spu, 0, 511), cap_busy(&spu, 0, 512)),
        (true, false)
    );
}

#[test]
fn given_capture_1_when_one_shot_then_it_runs_at_channel_3s_rate_not_channel_1s() {
    let mut spu = Spu::new();
    write(&mut spu, ch(1, TMR_OFFSET), Width::Half, 0, 0);
    write(&mut spu, ch(3, TMR_OFFSET), Width::Half, TMR, 0);
    write(
        &mut spu,
        SNDCAP1CNT_OFFSET,
        Width::Byte,
        u32::from(CAP_START | CAP_ONE_SHOT),
        0,
    );
    assert_eq!(
        (cap_busy(&spu, 1, 127), cap_busy(&spu, 1, 128)),
        (true, false)
    );
}

#[test]
fn given_a_loop_capture_when_much_time_passes_then_it_stays_busy() {
    let mut spu = Spu::new();
    write(
        &mut spu,
        SNDCAP0CNT_OFFSET,
        Width::Byte,
        u32::from(CAP_START),
        0,
    );
    assert!(cap_busy(&spu, 0, 1 << 60));
}

#[test]
fn given_a_busy_capture_when_a_zero_is_written_to_bit_7_then_it_stops_at_once() {
    let mut spu = Spu::new();
    write(
        &mut spu,
        SNDCAP0CNT_OFFSET,
        Width::Byte,
        u32::from(CAP_START),
        0,
    );
    write(&mut spu, SNDCAP0CNT_OFFSET, Width::Byte, 0, 10);
    assert_eq!((cap_busy(&spu, 0, 10), spu.capture(0).end()), (false, 0));
}

#[test]
fn given_a_slowest_timer_and_the_longest_length_when_started_then_the_end_does_not_wrap() {
    let mut spu = Spu::new();
    write(&mut spu, ch(0, PNT_OFFSET), Width::Half, 0xFFFF, 0);
    write(&mut spu, ch(0, LEN_OFFSET), Width::Word, LEN_MASK, 0);
    write(
        &mut spu,
        ch(0, CNT_OFFSET),
        Width::Word,
        start(ONE_SHOT, ADPCM),
        1 << 61,
    );
    assert!(spu.channel(0).end() > 1 << 61);
}

#[test]
fn given_a_new_device_when_checked_then_it_is_well_formed() {
    assert!(Spu::new().well_formed(0));
}

#[test]
fn given_registers_written_with_all_ones_when_checked_then_the_device_is_well_formed() {
    let mut spu = Spu::new();
    let words: Vec<u32> = (0..BLOCK_BYTES).step_by(4).collect();
    write_each(&mut spu, &words, Width::Word, u32::MAX);
    assert!(spu.well_formed(0));
}

#[test]
fn given_a_state_with_an_unused_bit_set_when_checked_then_it_is_not_well_formed() {
    let mut bad_channel = Spu::new();
    bad_channel.channels[4].cnt = 1 << 7;
    let mut bad_bias = Spu::new();
    bad_bias.bias = 1 << 10;
    let mut bad_capture = Spu::new();
    bad_capture.captures[1].cnt = 1 << 4;
    let verdicts = [bad_channel, bad_bias, bad_capture].map(|spu| spu.well_formed(0));
    assert_eq!(verdicts, [false, false, false]);
}

#[test]
fn given_a_stopped_channel_with_an_end_time_when_checked_then_it_is_not_well_formed() {
    let mut spu = Spu::new();
    spu.channels[0].end = 5;
    assert!(!spu.well_formed(0));
}

#[test]
fn given_a_one_shot_past_its_end_when_checked_then_it_is_still_well_formed() {
    let spu = started(0, start(ONE_SHOT, PCM8), 0, 4, 0);
    assert!(spu.well_formed(1 << 40));
}

// ----------------------------------------------------------------------------------- playback
//
// Expected samples come from `docs/hw/spu.md` ("Position"): a sample is `4 * (10000h - TMR)` master
// cycles, here 64, counted from the write that started the sound, at 1,000 below. PCM8 `NN` is the
// PCM16 value `NN00h`; PCM16 is little-endian.

/// The source data: sixteen bytes, at address 0.
const DATA: [u8; 16] = [
    0x10, 0x20, 0x30, 0x40, 0x80, 0x7F, 0xFF, 0x01, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C,
];
/// The byte at `addr`, `0xEE` past the data so a read out of range shows.
fn memory(addr: u32) -> u8 {
    DATA.get(addr as usize).copied().unwrap_or(0xEE)
}
const STARTED_AT: u64 = 1_000;
/// Samples of a PCM8 channel started at [`STARTED_AT`], `n` samples in.
const fn at_sample(n: u64) -> u64 {
    STARTED_AT + 64 * n
}

/// The (time, value) pairs where channel `x` does not play what `expected` says.
fn sample_failures(spu: &Spu, x: usize, expected: &[(u64, i32)]) -> Vec<(u64, i32)> {
    expected
        .iter()
        .map(|&(time, _)| (time, spu.channel(x).sample(time, memory)))
        .zip(expected)
        .filter(|(got, want)| got != *want)
        .map(|(got, _)| got)
        .collect()
}

#[test]
fn given_a_pcm8_one_shot_when_the_sample_clock_runs_then_each_byte_plays_as_pcm16_in_order() {
    let spu = started(0, start(ONE_SHOT, PCM8), 0, 4, STARTED_AT);
    let expected = [
        (at_sample(0), 0x1000),
        (at_sample(1), 0x2000),
        (at_sample(3), 0x4000),
        (at_sample(4), -0x8000),
        (at_sample(5), 0x7F00),
        (at_sample(6), -0x100),
        (at_sample(15), 0x0C00),
    ];
    assert_eq!(sample_failures(&spu, 0, &expected), Vec::new());
}

#[test]
fn given_a_time_inside_a_sample_when_read_then_the_sample_does_not_change_until_the_next_one() {
    let spu = started(0, start(ONE_SHOT, PCM8), 0, 4, STARTED_AT);
    let expected = [
        (at_sample(0), 0x1000),
        (at_sample(1) - 1, 0x1000),
        (at_sample(1), 0x2000),
    ];
    assert_eq!(sample_failures(&spu, 0, &expected), Vec::new());
}

#[test]
fn given_a_pcm16_one_shot_when_the_sample_clock_runs_then_each_halfword_plays_little_endian() {
    // 64-cycle samples; 4 words are 8 samples.
    let spu = started(0, start(ONE_SHOT, PCM16), 0, 4, STARTED_AT);
    let expected = [
        (at_sample(0), 0x2010),
        (at_sample(1), 0x4030),
        (at_sample(2), 0x7F80),
        (at_sample(3), 0x01FF),
        (at_sample(7), 0x0C0B),
    ];
    assert_eq!(sample_failures(&spu, 0, &expected), Vec::new());
}

#[test]
fn given_a_one_shot_when_it_ends_then_the_output_is_zero_and_the_busy_bit_clears_together() {
    let spu = started(0, start(ONE_SHOT, PCM8), 0, 4, STARTED_AT);
    let end = at_sample(16);
    assert_eq!(
        (
            spu.channel(0).sample(end - 1, memory),
            busy(&spu, 0, end - 1),
            spu.channel(0).sample(end, memory),
            busy(&spu, 0, end)
        ),
        (0x0C00, true, 0, false)
    );
}

#[test]
fn given_a_one_shot_with_hold_when_it_ends_then_the_last_sample_stays_for_ever() {
    let spu = started(0, start(ONE_SHOT, PCM8) | CNT_HOLD, 0, 4, STARTED_AT);
    let expected = [(at_sample(16), 0x0C00), (at_sample(1_000_000), 0x0C00)];
    assert_eq!(sample_failures(&spu, 0, &expected), Vec::new());
}

#[test]
fn given_a_loop_with_a_pnt_part_when_it_wraps_then_only_the_len_part_repeats() {
    // PNT 1 word (samples 0-3) once, then LEN 3 words (samples 4-15) for ever.
    let spu = started(0, start(LOOP, PCM8), 1, 3, STARTED_AT);
    let expected = [
        (at_sample(3), 0x4000),
        (at_sample(15), 0x0C00),
        (at_sample(16), -0x8000),
        (at_sample(27), 0x0C00),
        (at_sample(28), -0x8000),
    ];
    assert_eq!(sample_failures(&spu, 0, &expected), Vec::new());
}

#[test]
fn given_a_loop_with_pnt_zero_when_it_wraps_then_it_restarts_at_the_first_sample() {
    let spu = started(0, start(LOOP, PCM8), 0, 4, STARTED_AT);
    let expected = [(at_sample(15), 0x0C00), (at_sample(16), 0x1000)];
    assert_eq!(sample_failures(&spu, 0, &expected), Vec::new());
}

#[test]
fn given_the_manual_and_prohibited_repeat_modes_when_played_then_they_loop() {
    let manual = started(0, start(0, PCM8), 0, 4, STARTED_AT);
    let prohibited = started(0, start(3, PCM8), 0, 4, STARTED_AT);
    let expected = [(at_sample(16), 0x1000)];
    assert_eq!(
        (
            sample_failures(&manual, 0, &expected),
            sample_failures(&prohibited, 0, &expected)
        ),
        (Vec::new(), Vec::new())
    );
}

#[test]
fn given_a_sound_shorter_than_the_minimum_when_played_then_it_is_silent_and_stays_busy() {
    let spu = started(0, start(ONE_SHOT, PCM8), 1, 2, STARTED_AT);
    let looped = started(0, start(LOOP, PCM8), 0, 3, STARTED_AT);
    assert_eq!(
        (
            spu.channel(0).sample(at_sample(0), memory),
            looped.channel(0).sample(at_sample(0), memory),
            busy(&spu, 0, at_sample(10_000))
        ),
        (0, 0, true)
    );
}

#[test]
fn given_a_loop_with_an_empty_loop_part_when_the_pnt_part_ends_then_it_is_silent() {
    let spu = started(0, start(LOOP, PCM8), 4, 0, STARTED_AT);
    let expected = [
        (at_sample(15), 0x0C00),
        (at_sample(16), 0),
        (at_sample(99), 0),
    ];
    assert_eq!(sample_failures(&spu, 0, &expected), Vec::new());
}

#[test]
fn given_a_timer_of_zero_when_played_then_a_sample_lasts_four_times_ten_thousand_hex_cycles() {
    let mut spu = Spu::new();
    write(&mut spu, ch(0, LEN_OFFSET), Width::Word, 4, STARTED_AT);
    write(
        &mut spu,
        ch(0, CNT_OFFSET),
        Width::Word,
        start(LOOP, PCM8),
        STARTED_AT,
    );
    let slow = 4 * 0x1_0000;
    let expected = [(STARTED_AT + slow - 1, 0x1000), (STARTED_AT + slow, 0x2000)];
    assert_eq!(sample_failures(&spu, 0, &expected), Vec::new());
}

#[test]
fn given_a_stopped_channel_when_sampled_then_it_is_silent() {
    let spu = Spu::new();
    assert_eq!(
        (
            spu.channel(0).sample(STARTED_AT, memory),
            spu.channel(0).elapsed(STARTED_AT)
        ),
        (0, None)
    );
}

#[test]
fn given_a_started_channel_when_stopped_then_its_output_is_zero_at_once() {
    let mut spu = started(0, start(LOOP, PCM8), 0, 4, STARTED_AT);
    write(
        &mut spu,
        ch(0, CNT_OFFSET) + 3,
        Width::Byte,
        0,
        at_sample(2),
    );
    assert_eq!(
        (
            spu.channel(0).sample(at_sample(2), memory),
            spu.channel(0).start()
        ),
        (0, 0)
    );
}

#[test]
fn given_a_playing_channel_when_sad_len_and_tmr_are_written_then_the_sound_does_not_move() {
    let mut spu = started(0, start(LOOP, PCM8), 0, 4, STARTED_AT);
    let later = at_sample(2);
    write(&mut spu, ch(0, SAD_OFFSET), Width::Word, 4, later);
    write(&mut spu, ch(0, LEN_OFFSET), Width::Word, 1, later);
    write(&mut spu, ch(0, TMR_OFFSET), Width::Half, 0, later);
    let expected = [
        (at_sample(2), 0x3000),
        (at_sample(3), 0x4000),
        (at_sample(16), 0x1000),
    ];
    assert_eq!(sample_failures(&spu, 0, &expected), Vec::new());
}

#[test]
fn given_a_playing_channel_when_the_volume_is_written_then_the_registers_are_read_live() {
    let mut spu = started(0, start(LOOP, PCM8), 0, 4, STARTED_AT);
    write(&mut spu, ch(0, CNT_OFFSET), Width::Byte, 0x2A, at_sample(1));
    let channel = spu.channel(0);
    assert_eq!(
        (
            channel.volume(),
            channel.format(),
            channel.repeat_mode(),
            channel.elapsed(at_sample(5))
        ),
        (0x2A, PCM8, LOOP, Some(5))
    );
}

#[test]
fn given_a_channel_read_at_each_field_when_cnt_is_set_then_the_accessors_return_the_fields() {
    let mut spu = Spu::new();
    let cnt = 0x45 | 2 << 8 | 1 << 15 | 0x33 << 16 | 5 << 24;
    write(&mut spu, ch(0, CNT_OFFSET), Width::Word, cnt, 0);
    let channel = spu.channel(0);
    assert_eq!(
        (
            channel.volume(),
            channel.divider(),
            channel.hold(),
            channel.panning(),
            channel.duty()
        ),
        (0x45, 2, true, 0x33, 5)
    );
}

#[test]
fn given_a_time_before_the_start_when_sampled_then_the_channel_is_silent_and_has_no_position() {
    let spu = started(0, start(LOOP, PCM8), 0, 4, STARTED_AT);
    assert_eq!(
        (
            spu.channel(0).sample(STARTED_AT - 1, memory),
            spu.channel(0).elapsed(STARTED_AT - 1)
        ),
        (0, None)
    );
}

// ------------------------------------------------------------------------------------ IMA-ADPCM
//
// Four words (a header and three of data): 24 samples. The header is `0` (value 0, index 0); the
// data bytes are `21h 43h` (nibbles 1, 2, 3, 4), two zero bytes (four nibbles of 0), `55h` four
// times (eight nibbles of 5) and four zero bytes. Worked out by hand from the formula of
// `docs/hw/spu.md` ("IMA-ADPCM") and the step table:
//
// | sample | nibble | step | diff | value | index |
// |---|---|---|---|---|---|
// | 0-3 | 1 2 3 4 | 7 | 1 3 4 7 | 1 4 8 15 | 0 0 0 2 |
// | 4-7 | 0 0 0 0 | 9 8 7 7 | 1 1 0 0 | 16 17 17 17 | 1 0 0 0 |
// | 8-15 | 5 x 8 | 7 11 16 23 34 50 73 107 | 8 14 22 30 46 68 100 146 | 25 39 61 91 137 205 305 451 | 4 8 12 16 20 24 28 32 |
// | 16-23 | 0 x 8 | 157 143 130 118 107 97 88 80 | 19 17 16 14 13 12 11 10 | 470 487 503 517 530 542 553 563 | 31 30 29 28 27 26 25 24 |

const ADPCM_DATA: [u8; 16] = [
    0, 0, 0, 0, 0x21, 0x43, 0, 0, 0x55, 0x55, 0x55, 0x55, 0, 0, 0, 0,
];
fn adpcm_memory(addr: u32) -> u8 {
    ADPCM_DATA.get(addr as usize).copied().unwrap_or(0xEE)
}

/// The channel's sample at each of `times`, with the decoder advanced to each time in turn, as the
/// machine does at every output sample.
fn adpcm_run(spu: &Spu, times: &[u64]) -> Vec<i32> {
    let mut channel = *spu.channel(0);
    times
        .iter()
        .map(|&time| {
            channel.decoders = channel.advanced(time, adpcm_memory);
            channel.sample(time, adpcm_memory)
        })
        .collect()
}

/// The times of samples `first..first + count` of a channel started at [`STARTED_AT`].
fn sample_times(first: u64, count: u64) -> Vec<u64> {
    (first..first + count).map(at_sample).collect()
}

#[test]
fn given_an_adpcm_one_shot_when_the_sample_clock_runs_then_the_values_follow_the_hand_table() {
    let spu = started(0, start(ONE_SHOT, ADPCM), 0, 4, STARTED_AT);
    assert_eq!(
        adpcm_run(&spu, &sample_times(0, 16)),
        vec![
            1, 4, 8, 15, 16, 17, 17, 17, 25, 39, 61, 91, 137, 205, 305, 451
        ]
    );
}

#[test]
fn given_an_adpcm_one_shot_when_the_decoder_catches_up_in_one_jump_then_it_matches_stepping() {
    let spu = started(0, start(ONE_SHOT, ADPCM), 0, 4, STARTED_AT);
    assert_eq!(adpcm_run(&spu, &[at_sample(23)]), vec![563]);
}

#[test]
fn given_an_adpcm_one_shot_when_it_ends_then_the_output_is_zero_or_the_last_value_with_hold() {
    let plain = started(0, start(ONE_SHOT, ADPCM), 0, 4, STARTED_AT);
    let held = started(0, start(ONE_SHOT, ADPCM) | CNT_HOLD, 0, 4, STARTED_AT);
    let times = [at_sample(23), at_sample(24), at_sample(500)];
    assert_eq!(
        (adpcm_run(&plain, &times), adpcm_run(&held, &times)),
        (vec![563, 0, 0], vec![563, 563, 563])
    );
}

#[test]
fn given_an_adpcm_loop_when_it_wraps_then_the_state_at_the_loop_start_is_put_back() {
    // PNT 2 words puts the loop start at data sample 8, where the state is 17, index 0. Sample 24
    // wraps to it and decodes nibble 5 again (25, 39, ...) instead of going on from 563.
    let spu = started(0, start(LOOP, ADPCM), 2, 2, STARTED_AT);
    assert_eq!(
        adpcm_run(&spu, &sample_times(22, 6)),
        vec![553, 563, 25, 39, 61, 91]
    );
}

#[test]
fn given_an_adpcm_loop_with_pnt_one_when_it_wraps_then_the_header_state_is_put_back() {
    let spu = started(0, start(LOOP, ADPCM), 1, 3, STARTED_AT);
    assert_eq!(adpcm_run(&spu, &sample_times(23, 3)), vec![563, 1, 4]);
}

#[test]
fn given_an_adpcm_sound_shorter_than_the_minimum_when_played_then_it_is_silent() {
    let spu = started(0, start(ONE_SHOT, ADPCM), 0, 3, STARTED_AT);
    assert_eq!(adpcm_run(&spu, &sample_times(0, 3)), vec![0, 0, 0]);
}

#[test]
fn given_a_pcm_channel_when_advanced_then_the_decoder_state_does_not_change() {
    let spu = started(0, start(LOOP, PCM8), 0, 4, STARTED_AT);
    assert_eq!(
        spu.channel(0).advanced(at_sample(9), memory),
        Decoders::default()
    );
}

#[test]
fn given_an_adpcm_channel_when_stopped_then_the_decoder_state_is_cleared() {
    let mut spu = started(0, start(LOOP, ADPCM), 1, 3, STARTED_AT);
    let state = spu.channel(0).advanced(at_sample(5), adpcm_memory);
    spu.set_decoders(std::array::from_fn(|_| state));
    write(
        &mut spu,
        ch(0, CNT_OFFSET) + 3,
        Width::Byte,
        0,
        at_sample(6),
    );
    assert_eq!(*spu.channel(0).adpcm(), Adpcm::default());
}

#[test]
fn given_a_decoder_state_with_a_wide_index_when_checked_then_the_device_is_not_well_formed() {
    let mut spu = Spu::new();
    spu.channels[0].decoders.adpcm = Adpcm {
        index: 200,
        ..Adpcm::default()
    };
    assert!(!spu.well_formed(0));
}

// ------------------------------------------------------------------------------ PSG and noise
//
// A PSG channel samples 8 positions per wave at 64 master cycles each (TMR `FFF0h`); the wave starts
// LOW (`spu.md`, "PSG"). Noise runs the formula of `spu.md` ("Noise") from `7FFFh`: its first four
// outputs are LOW with the register at `5FFFh`, `4FFFh`, `47FFh`, `43FFh` (see `psg/tests.rs`).

const HIGH: i32 = 0x7FFF;
const LOW: i32 = -0x7FFF;

/// A device with channel `x` started as PSG or noise with `duty` and `cnt` extras at `STARTED_AT`.
fn generator(x: u32, duty: u32) -> Spu {
    started(x, start(LOOP, PSG) | duty << 24, 0, 0, STARTED_AT)
}

/// `channel` with its generators advanced to each of `times` in turn.
fn advance_each(mut channel: Channel, times: &[u64]) -> Channel {
    times
        .iter()
        .for_each(|&time| channel.decoders = channel.advanced(time, memory));
    channel
}

/// The channel `x` sample at each of `times`, the generators advanced to each in turn.
fn generated(spu: &Spu, x: usize, times: &[u64]) -> Vec<i32> {
    let mut channel = *spu.channel(x);
    times
        .iter()
        .map(|&time| {
            channel.decoders = channel.advanced(time, memory);
            channel.sample(time, memory)
        })
        .collect()
}

#[test]
fn given_a_psg_channel_with_duty_3_when_played_then_it_is_four_low_and_four_high_each_wave() {
    let spu = generator(8, 3);
    assert_eq!(
        generated(&spu, 8, &sample_times(0, 9)),
        vec![LOW, LOW, LOW, LOW, HIGH, HIGH, HIGH, HIGH, LOW]
    );
}

#[test]
fn given_each_psg_channel_when_started_then_channels_8_to_13_play_the_wave() {
    let failures: Vec<u32> = (8..=13)
        .filter(|&x| generated(&generator(x, 0), x as usize, &[at_sample(7)]) != vec![HIGH])
        .collect();
    assert_eq!(failures, Vec::<u32>::new());
}

#[test]
fn given_a_psg_channel_when_the_length_and_repeat_mode_are_anything_then_it_never_ends() {
    let mut spu = Spu::new();
    write(&mut spu, ch(9, TMR_OFFSET), Width::Half, TMR, STARTED_AT);
    write(
        &mut spu,
        ch(9, CNT_OFFSET),
        Width::Word,
        start(ONE_SHOT, PSG) | 7 << 24,
        STARTED_AT,
    );
    let late = at_sample(10_000_000);
    assert_eq!(
        (busy(&spu, 9, late), generated(&spu, 9, &[late])),
        (true, vec![LOW])
    );
}

#[test]
fn given_a_psg_channel_restarted_when_started_again_then_the_wave_begins_with_its_low_part() {
    let mut spu = generator(8, 3);
    write(
        &mut spu,
        ch(8, CNT_OFFSET) + 3,
        Width::Byte,
        0,
        at_sample(5),
    );
    write(
        &mut spu,
        ch(8, CNT_OFFSET),
        Width::Word,
        start(LOOP, PSG) | 3 << 24,
        at_sample(5),
    );
    let restart = at_sample(5);
    let times: Vec<u64> = (0..5).map(|n| restart + 64 * n).collect();
    assert_eq!(generated(&spu, 8, &times), vec![LOW, LOW, LOW, LOW, HIGH]);
}

#[test]
fn given_a_psg_duty_written_while_playing_when_read_then_the_new_duty_applies_at_once() {
    let mut spu = generator(8, 0);
    write(
        &mut spu,
        ch(8, CNT_OFFSET) + 3,
        Width::Byte,
        0x80 | 1 << 3 | 3 << 5 | 3,
        at_sample(2),
    );
    // CNT byte 3 is bits 24-31: start, PSG (3 << 29), loop (1 << 27), duty 3 here.
    assert_eq!(spu.channel(8).duty(), 3);
}

#[test]
fn given_format_3_on_channels_0_to_7_when_started_then_it_is_silent_and_busy() {
    let spu = generator(3, 3);
    assert_eq!(
        (
            generated(&spu, 3, &[at_sample(7)]),
            busy(&spu, 3, at_sample(1_000_000))
        ),
        (vec![0], true)
    );
}

#[test]
fn given_a_noise_channel_when_played_then_the_outputs_follow_the_formula() {
    let spu = generator(14, 0);
    assert_eq!(
        generated(&spu, 14, &sample_times(0, 4)),
        vec![LOW, LOW, LOW, LOW]
    );
}

#[test]
fn given_noise_on_channel_15_when_caught_up_in_one_jump_then_the_register_matches_stepping() {
    let spu = generator(15, 0);
    let jump = spu.channel(15).advanced(at_sample(3), memory);
    let stepped = advance_each(*spu.channel(15), &sample_times(0, 4));
    assert_eq!(
        (jump.noise.register(), jump == stepped.decoders),
        (0x43FF, true)
    );
}

#[test]
fn given_a_noise_channel_when_restarted_then_the_register_starts_again_at_7fff() {
    let mut spu = generator(14, 0);
    let state = spu.channel(14).advanced(at_sample(9), memory);
    let mut all = [Decoders::default(); CHANNELS];
    all[14] = state;
    spu.set_decoders(all);
    write(
        &mut spu,
        ch(14, CNT_OFFSET) + 3,
        Width::Byte,
        0,
        at_sample(10),
    );
    write(
        &mut spu,
        ch(14, CNT_OFFSET),
        Width::Word,
        start(LOOP, PSG),
        at_sample(10),
    );
    let first = spu.channel(14).advanced(at_sample(10), memory);
    assert_eq!(first.noise.register(), 0x5FFF);
}

#[test]
fn given_a_noise_register_wider_than_15_bits_when_checked_then_the_device_is_not_well_formed() {
    let mut spu = Spu::new();
    spu.channels[14].decoders.noise.x = 0x8000;
    assert!(!spu.well_formed(0));
}

// ------------------------------------------------------------------------------------- the mixer
//
// `Spu::output` from the per-field arithmetic of `spu/mixer/tests.rs`. With volume 127, panning 0,
// master 127 and bias 200h, a PCM16 sample `s` on the left comes out as `(s >> 6) * 64`: the 10-bit
// level is `s >> 6` and the `i16` scales it by 64 (`spu.md`, "Output samples").

/// SOUNDCNT with the master enable and master volume 127.
const MASTER_ON: u32 = 1 << 15 | 0x7F;

/// A device with SOUNDCNT `soundcnt`, SOUNDBIAS `bias` and channel `x`'s SOUNDxCNT (volume, divider
/// and panning bits only) set from `cnts`.
fn mixer(soundcnt: u32, bias: u32, cnts: &[(u32, u32)]) -> Spu {
    let mut spu = Spu::new();
    write(&mut spu, SOUNDCNT_OFFSET, Width::Half, soundcnt, 0);
    write(&mut spu, SOUNDBIAS_OFFSET, Width::Half, bias, 0);
    cnts.iter()
        .for_each(|&(x, cnt)| write(&mut spu, ch(x, CNT_OFFSET), Width::Word, cnt, 0));
    spu
}

/// Full volume, panning 0 (all left).
const LEFT: u32 = 0x7F;
/// Full volume, panning 127 (all right).
const RIGHT: u32 = 0x7F | 127 << 16;

/// The channels' samples with `values` at the listed channels and 0 elsewhere.
fn samples(values: &[(usize, i32)]) -> [i32; CHANNELS] {
    let mut all = [0; CHANNELS];
    values.iter().for_each(|&(x, v)| all[x] = v);
    all
}

#[test]
fn given_a_full_left_channel_when_mixed_then_the_left_carries_the_sample_to_a_multiple_of_64() {
    let spu = mixer(MASTER_ON, 0x200, &[(0, LEFT)]);
    assert_eq!(spu.output(&samples(&[(0, 0x4000)]), true), [0x4000, 0]);
}

#[test]
fn given_a_full_right_channel_when_mixed_then_the_right_carries_the_sample() {
    let spu = mixer(MASTER_ON, 0x200, &[(0, RIGHT)]);
    assert_eq!(spu.output(&samples(&[(0, 0x4000)]), true), [0, 0x4000]);
}

#[test]
fn given_a_centered_channel_when_mixed_then_each_side_has_half() {
    let spu = mixer(MASTER_ON, 0x200, &[(0, 0x7F | 64 << 16)]);
    assert_eq!(spu.output(&samples(&[(0, 0x4000)]), true), [0x2000, 0x2000]);
}

#[test]
fn given_a_channel_divider_of_16_when_mixed_then_the_level_is_a_sixteenth() {
    let spu = mixer(MASTER_ON, 0x200, &[(0, LEFT | 3 << 8)]);
    assert_eq!(spu.output(&samples(&[(0, 0x4000)]), true), [0x400, 0]);
}

#[test]
fn given_a_channel_volume_of_half_when_mixed_then_the_level_is_half() {
    let spu = mixer(MASTER_ON, 0x200, &[(0, 64)]);
    assert_eq!(spu.output(&samples(&[(0, 0x4000)]), true), [0x2000, 0]);
}

#[test]
fn given_a_master_volume_of_half_when_mixed_then_the_level_is_half() {
    let spu = mixer(1 << 15 | 64, 0x200, &[(0, LEFT)]);
    assert_eq!(spu.output(&samples(&[(0, 0x4000)]), true), [0x2000, 0]);
}

#[test]
fn given_two_channels_when_mixed_then_they_add_before_the_master_volume() {
    let spu = mixer(MASTER_ON, 0x200, &[(0, LEFT), (2, LEFT)]);
    assert_eq!(
        spu.output(&samples(&[(0, 0x2000), (2, 0x1000)]), true),
        [0x3000, 0]
    );
}

#[test]
fn given_a_sum_beyond_the_range_when_mixed_then_the_level_clips_at_both_ends() {
    let spu = mixer(MASTER_ON, 0x200, &[(0, LEFT), (2, LEFT)]);
    let high = spu.output(&samples(&[(0, 0x7FFF), (2, 0x7FFF)]), true);
    let low = spu.output(&samples(&[(0, -0x8000), (2, -0x8000)]), true);
    assert_eq!((high[0], low[0]), (0x7FC0, -0x8000));
}

#[test]
fn given_a_bias_off_the_medium_when_mixed_then_the_level_shifts_by_it() {
    let spu = mixer(MASTER_ON, 0x280, &[(0, LEFT)]);
    // 80h above the medium is 128 * 64 = 2000h on both sides, plus the signal on the left.
    assert_eq!(spu.output(&samples(&[(0, 0x4000)]), true), [0x6000, 0x2000]);
}

#[test]
fn given_the_master_enable_off_when_mixed_then_the_output_is_the_bias_alone() {
    let spu = mixer(0x7F, 0x280, &[(0, LEFT)]);
    assert_eq!(spu.output(&samples(&[(0, 0x4000)]), true), [0x2000, 0x2000]);
}

#[test]
fn given_a_gate_closed_when_mixed_then_the_output_is_zero_even_with_the_master_enable_off() {
    let on = mixer(MASTER_ON, 0x280, &[(0, LEFT)]);
    let off = mixer(0, 0x280, &[]);
    assert_eq!(
        (
            on.output(&samples(&[(0, 0x4000)]), false),
            off.output(&samples(&[]), false)
        ),
        ([0, 0], [0, 0])
    );
}

#[test]
fn given_the_left_source_set_to_channel_1_when_mixed_then_only_channel_1_reaches_the_left() {
    let spu = mixer(MASTER_ON | 1 << 8, 0x200, &[(0, LEFT), (1, LEFT)]);
    assert_eq!(
        spu.output(&samples(&[(0, 0x2000), (1, 0x1000)]), true),
        [0x1000, 0]
    );
}

#[test]
fn given_the_sources_channel_3_and_both_when_mixed_then_left_and_right_pick_independently() {
    // Left from channel 3, right from channels 1 and 3 (both at full right).
    let spu = mixer(
        MASTER_ON | 2 << 8 | 3 << 10,
        0x200,
        &[(1, RIGHT), (3, RIGHT)],
    );
    // Channel 3 at panning 127 is all right: its left is 0 and its right 0x1000; channel 1 right
    // 0x2000. Left source ch3 -> 0; right source ch1 + ch3 -> 0x3000.
    assert_eq!(
        spu.output(&samples(&[(1, 0x2000), (3, 0x1000)]), true),
        [0, 0x3000]
    );
}

#[test]
fn given_channel_1_not_sent_to_the_mixer_when_mixed_then_the_mixer_omits_it() {
    let spu = mixer(MASTER_ON | 1 << 12, 0x200, &[(0, LEFT), (1, LEFT)]);
    assert_eq!(
        spu.output(&samples(&[(0, 0x2000), (1, 0x1000)]), true),
        [0x2000, 0]
    );
}

#[test]
fn given_channel_3_not_sent_but_selected_when_mixed_then_the_output_still_has_it() {
    let spu = mixer(MASTER_ON | 1 << 13 | 2 << 8, 0x200, &[(0, LEFT), (3, LEFT)]);
    assert_eq!(
        spu.output(&samples(&[(0, 0x2000), (3, 0x1000)]), true),
        [0x1000, 0]
    );
}

#[test]
fn given_a_channel_that_is_silent_when_mixed_then_it_adds_nothing() {
    let spu = mixer(MASTER_ON, 0x200, &[(0, LEFT), (5, LEFT)]);
    assert_eq!(spu.output(&samples(&[(0, 0x2000)]), true), [0x2000, 0]);
}
